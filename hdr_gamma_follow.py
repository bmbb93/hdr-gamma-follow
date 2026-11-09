# /// script
# requires-python = ">=3.11"
# dependencies = ["pystray", "pillow"]
# ///
"""Windows の「SDR コンテンツの明るさ」に合わせて sRGB→Gamma HDR プロファイルを自動で切り替える常駐ツール。

    uv run hdr_gamma_follow.py                 # 常駐（タスクトレイにアイコン。右クリックでガンマ切替）
    uvw run hdr_gamma_follow.py                # 常駐（コンソールなし。タスクスケジューラ用）
    uv run hdr_gamma_follow.py --gamma 2.4     # ガンマを設定（常駐中ならすぐ反映。未常駐なら 1 回適用）
    uv run hdr_gamma_follow.py --once          # 今の明るさとガンマで 1 回だけ適用
    uv run hdr_gamma_follow.py --flash         # 切替が画面に効くかの目視テスト（3 秒だけ極端に暗いカーブ）

ガンマは 1.8 / 2.0 / 2.2 / 2.4 / 2.6（既定 2.2）。選択は config.json に保存され、次回起動時も引き継ぐ。
HDR が有効なディスプレイだけを対象にする。必要なプロファイルはその場で生成・インストールして既定の
HDR プロファイルにし、前の自動プロファイルの関連付けは外す。
"""
import argparse
import ctypes as C
import json
import logging
import os
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import colorprof as cp  # noqa: E402
from gen_profile import GAMMAS, OUT_DIR, is_auto_profile, make_profile, profile_name  # noqa: E402
from mhc2 import build_mhc2, gamma22_lut  # noqa: E402
from probe_sdr import ADV_COLOR, GET_ADVANCED_COLOR_INFO, GET_SDR_WHITE_LEVEL, SDR_WHITE, paths, query  # noqa: E402
from relabel_icc import mluc, rebuild  # noqa: E402

POLL_SEC = 0.5
SETTLE_SEC = 1.0  # スライダーを動かし終えてからこの時間変化がなければ適用
CONFIG = os.path.join(HERE, "config.json")
DEFAULT_GAMMA = 2.2

log = logging.getLogger("hdr-gamma")


# ---------- 設定 ----------
def load_gamma() -> float:
    try:
        with open(CONFIG, encoding="utf-8") as f:
            g = float(json.load(f).get("gamma", DEFAULT_GAMMA))
        return g if g in GAMMAS else DEFAULT_GAMMA
    except (OSError, ValueError):
        return DEFAULT_GAMMA


def save_gamma(g: float) -> None:
    with open(CONFIG, "w", encoding="utf-8") as f:
        json.dump({"gamma": g}, f)


# ---------- ディスプレイ ----------
def displays():
    """[(key, adapterLUID, sourceId, hdr_enabled, sdr_white_nits)]"""
    out = []
    for p in paths():
        t, s = p.targetInfo, p.sourceInfo
        adv = query(GET_ADVANCED_COLOR_INFO, ADV_COLOR, t.adapterId, t.id).value
        raw = query(GET_SDR_WHITE_LEVEL, SDR_WHITE, t.adapterId, t.id).SDRWhiteLevel
        key = (t.adapterId.HighPart, t.adapterId.LowPart, s.id)
        out.append((key, s.adapterId, s.id, bool(adv & 2), round(raw * 80 / 1000)))
    return out


def ensure_installed(name: str, data_fn) -> None:
    if os.path.exists(os.path.join(cp.color_dir(), name)):
        return
    os.makedirs(OUT_DIR, exist_ok=True)
    src = os.path.join(OUT_DIR, name)
    with open(src, "wb") as f:
        f.write(data_fn())
    cp.install(src)
    log.info("installed %s", name)


# ColorProfileGetDisplayDefault はプロセス内でキャッシュされ、他プロセス（設定アプリ等）による変更が
# 見えない。そのため外部変更の検出はレジストリを直接読む。HDR の既定は ICMProfileAC の最後の要素。
ICM_DISPLAY = r"Software\Microsoft\Windows NT\CurrentVersion\ICM\ProfileAssociations\Display\{4d36e96e-e325-11ce-bfc1-08002be10318}"


def _reg_hdr_lists() -> dict[str, list[str]]:
    import winreg
    out = {}
    try:
        root = winreg.OpenKey(winreg.HKEY_CURRENT_USER, ICM_DISPLAY)
    except OSError:
        return out
    with root:
        i = 0
        while True:
            try:
                sub = winreg.EnumKey(root, i)
            except OSError:
                break
            i += 1
            try:
                with winreg.OpenKey(root, sub) as k:
                    v, _ = winreg.QueryValueEx(k, "ICMProfileAC")
                    out[sub] = list(v)
            except OSError:
                pass
    return out


def reg_find_subkey(name: str) -> str | None:
    """既定の HDR プロファイルが name になっているレジストリ上のディスプレイ subkey"""
    hits = [k for k, v in _reg_hdr_lists().items() if v and v[-1] == name]
    return hits[0] if len(hits) == 1 else None


def reg_hdr_default(subkey: str) -> str | None:
    v = _reg_hdr_lists().get(subkey)
    return v[-1] if v else None


def apply(adapter, source: int, nits: int, gamma: float, force: bool = False) -> str:
    name = profile_name(nits, gamma)
    ensure_installed(name, lambda: make_profile(nits, gamma))
    scope = cp.user_scope(adapter, source)
    prev = cp.get_default(scope, adapter, source, True)
    if prev != name or force:
        cp.associate_hdr(scope, name, adapter, source)
        cp.set_default_hdr(scope, name, adapter, source)
        if is_auto_profile(prev) and prev != name:
            try:
                cp.dissociate_hdr(scope, prev, adapter, source)
            except OSError:
                pass
    log.info("source %d: SDR white %d nits, gamma %.1f -> %s (was %s)", source, nits, gamma, name, prev)
    return name


def run_once(gamma: float):
    for key, adapter, source, hdr, nits in displays():
        if hdr:
            apply(adapter, source, nits, gamma)


# ---------- 常駐 ----------
class Follower:
    def __init__(self):
        self.gamma = load_gamma()
        self.status = "starting"
        self.wake = threading.Event()
        self.stop = threading.Event()
        self.on_change = lambda: None
        self._cfg_mtime = self._mtime()

    def _mtime(self):
        try:
            return os.path.getmtime(CONFIG)
        except OSError:
            return None

    def set_gamma(self, g: float):
        self.gamma = g
        save_gamma(g)
        self._cfg_mtime = self._mtime()
        self.wake.set()

    def loop(self):
        applied: dict = {}  # key -> (hdr, nits, gamma)
        subkeys: dict = {}  # key -> レジストリ subkey（外部変更の検出用）
        pending: dict = {}  # key -> ((hdr, nits), 最初に観測した時刻)
        log.info("started (gamma %.1f)", self.gamma)
        while not self.stop.is_set():
            try:
                m = self._mtime()
                if m != self._cfg_mtime:  # CLI の --gamma で書き換えられた
                    self._cfg_mtime, self.gamma = m, load_gamma()
                gamma = self.gamma
                status = []
                for key, adapter, source, hdr, nits in displays():
                    if hdr:
                        status.append(f"{nits} nits")
                    state = (hdr, nits, gamma)
                    prev = applied.get(key)
                    if prev == state:
                        pending.pop(key, None)
                        # 設定アプリや Windows に既定を戻されていたら掛け直す
                        if hdr and key in subkeys:
                            cur = reg_hdr_default(subkeys[key])
                            log.debug("check %s %s -> %s", key, subkeys[key], cur)
                            if cur != profile_name(nits, gamma):
                                log.info("source %d: default changed externally to %s; re-applying", source, cur)
                                apply(adapter, source, nits, gamma, force=True)
                        continue
                    # 明るさの変化はスライダーが止まるまで待つ。起動直後とガンマ変更は即時
                    if prev is not None and prev[:2] != state[:2]:
                        if pending.get(key, (None,))[0] != state[:2]:
                            pending[key] = (state[:2], time.monotonic())
                            continue
                        if time.monotonic() - pending[key][1] < SETTLE_SEC:
                            continue
                    if hdr:
                        name = apply(adapter, source, nits, gamma, force=True)
                        sk = reg_find_subkey(name)
                        log.debug("subkey for %s: %s (%s)", name, sk, _reg_hdr_lists())
                        if sk:
                            subkeys[key] = sk
                            # 一覧に残った古い自動プロファイルの関連付けを外す
                            scope = cp.user_scope(adapter, source)
                            for old in _reg_hdr_lists().get(sk, []):
                                if is_auto_profile(old) and old != name:
                                    try:
                                        cp.dissociate_hdr(scope, old, adapter, source)
                                        log.info("dissociated %s", old)
                                    except OSError:
                                        pass
                        else:
                            subkeys.pop(key, None)
                            log.info("source %d: registry subkey not found; external-change check disabled", source)
                    applied[key] = state
                    pending.pop(key, None)
                new_status = f"Gamma {gamma:.1f} / " + (", ".join(status) or "HDR off")
                if new_status != self.status:
                    self.status = new_status
                    self.on_change()
            except Exception:
                log.exception("poll failed")
                self.stop.wait(5)
            self.wake.wait(POLL_SEC)
            self.wake.clear()


def make_icon(gamma: float):
    from PIL import Image, ImageDraw, ImageFont

    img = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle((0, 0, 63, 63), radius=12, fill=(32, 32, 36, 255))
    text = f"{gamma:.1f}"
    try:
        font = ImageFont.truetype("segoeuib.ttf", 30)
    except OSError:
        font = ImageFont.load_default(size=30)
    d.text((32, 32), text, font=font, fill=(255, 210, 90, 255), anchor="mm")
    return img


def run_tray():
    import pystray

    f = Follower()

    def choose(g):
        def handler(icon, item):
            f.set_gamma(g)
            icon.icon = make_icon(g)
        return handler

    def is_checked(g):
        return lambda item: f.gamma == g

    def reapply(icon, item):
        f.wake.set()

    def quit_(icon, item):
        f.stop.set()
        f.wake.set()
        icon.stop()

    menu = pystray.Menu(
        pystray.MenuItem(lambda item: f.status, None, enabled=False),
        pystray.Menu.SEPARATOR,
        *[pystray.MenuItem(f"Gamma {g:.1f}", choose(g), checked=is_checked(g), radio=True) for g in GAMMAS],
        pystray.Menu.SEPARATOR,
        pystray.MenuItem("終了", quit_),
    )
    icon = pystray.Icon("hdr-gamma", make_icon(f.gamma), "HDR Gamma", menu)

    def on_change():
        icon.title = f"HDR {f.status}"
        icon.icon = make_icon(f.gamma)
        icon.update_menu()

    f.on_change = on_change
    threading.Thread(target=f.loop, daemon=True).start()
    icon.run()


def already_running() -> bool:
    C.windll.kernel32.CreateMutexW(None, False, "Local\\hdr-gamma22-follow")
    return C.windll.kernel32.GetLastError() == 183  # ERROR_ALREADY_EXISTS


def flash():
    """切替が実際に画面へ反映されるかの目視テスト。"""
    name = "gamma_test_flash.icm"
    ensure_installed(name, lambda: rebuild(
        open(os.path.join(HERE, "template.icm"), "rb").read(),
        {b"desc": mluc("TEST - very dark curve"),
         b"MHC2": build_mhc2(gamma22_lut(480, gamma=4.0), 0.0050048828125, 800.0)}))
    for key, adapter, source, hdr, nits in displays():
        if not hdr:
            continue
        scope = cp.user_scope(adapter, source)
        prev = cp.get_default(scope, adapter, source, True)
        print(f"source {source}: 3 秒間テストカーブを適用します（画面が暗くなれば成功）")
        cp.associate_hdr(scope, name, adapter, source)
        time.sleep(3)
        if prev:
            cp.set_default_hdr(scope, prev, adapter, source)
        cp.dissociate_hdr(scope, name, adapter, source)
        print(f"source {source}: 元に戻しました -> {cp.get_default(scope, adapter, source, True)}")


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--once", action="store_true")
    ap.add_argument("--flash", action="store_true")
    ap.add_argument("--gamma", type=float, choices=GAMMAS)
    args = ap.parse_args()
    handlers = [logging.FileHandler(os.path.join(HERE, "follow.log"), encoding="utf-8")]
    if sys.stdout:
        handlers.append(logging.StreamHandler())
    logging.basicConfig(level=logging.DEBUG if os.environ.get("HDRG_DEBUG") else logging.INFO, format="%(asctime)s %(message)s", handlers=handlers)
    if args.flash:
        flash()
    elif args.gamma is not None or args.once:
        if args.gamma is not None:
            save_gamma(args.gamma)
            print(f"gamma = {args.gamma:.1f}")
        if args.once or not already_running():
            run_once(load_gamma())
    elif already_running():
        log.info("already running")
    else:
        run_tray()
