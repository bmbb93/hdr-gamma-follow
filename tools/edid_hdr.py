"""レジストリに残っているモニターの EDID から、HDR 静的メタデータ（輝度）を読み出す。

    uv run tools/edid_hdr.py

CTA-861 拡張ブロックの HDR Static Metadata Data Block (拡張タグ 6) を解析する。
レジストリには過去に接続したモニターの EDID も残るので、全部表示する。
"""
import winreg

ROOT = r"SYSTEM\CurrentControlSet\Enum\DISPLAY"


def subkeys(k):
    i = 0
    while True:
        try:
            yield winreg.EnumKey(k, i)
        except OSError:
            return
        i += 1


def name_of(edid: bytes) -> str:
    for off in (54, 72, 90, 108):
        d = edid[off : off + 18]
        if d[:3] == b"\0\0\0" and d[3] == 0xFC:
            return d[5:].split(b"\n")[0].decode("ascii", "replace").strip()
    return "?"


def hdr_block(edid: bytes):
    for ext in range(1, len(edid) // 128):
        b = edid[ext * 128 : ext * 128 + 128]
        if b[0] != 0x02:
            continue
        end = b[2]
        i = 4
        while i < end:
            tag, ln = b[i] >> 5, b[i] & 0x1F
            if tag == 7 and ln >= 1 and b[i + 1] == 6:
                return b[i + 2 : i + 1 + ln]
            i += 1 + ln
    return None


def describe(h: bytes) -> str:
    eotf = h[0]
    names = [n for bit, n in enumerate(["SDR", "HDR(trad)", "PQ", "HLG"]) if eotf & (1 << bit)]
    out = [f"EOTF={'/'.join(names)}"]
    if len(h) >= 3:
        mx = 50 * 2 ** (h[2] / 32)
        out.append(f"max={mx:.0f} nits")
        if len(h) >= 4:
            out.append(f"max frame-avg={50 * 2 ** (h[3] / 32):.0f} nits")
        if len(h) >= 5:
            out.append(f"min={mx * (h[4] / 255) ** 2 / 100:.4f} nits")
    return ", ".join(out)


with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE, ROOT) as root:
    for model in subkeys(root):
        with winreg.OpenKey(root, model) as mk:
            for inst in subkeys(mk):
                try:
                    with winreg.OpenKey(mk, inst + r"\Device Parameters") as dp:
                        edid, _ = winreg.QueryValueEx(dp, "EDID")
                except OSError:
                    continue
                h = hdr_block(edid)
                print(f"{model}\\{inst}: {name_of(edid)} | {describe(h) if h else 'HDR block なし'}")
