"""任意の SDR 白レベル (nits) とガンマ用の sRGB→Gamma HDR プロファイルを生成する。

使い方:
    uv run gen_profile.py 236            # → profiles/gamma22_auto_236nits.icm
    uv run gen_profile.py 236 --gamma 2.4
"""
import argparse
import os

from mhc2 import build_mhc2, gamma22_lut
from relabel_icc import mluc, rebuild

HERE = os.path.dirname(os.path.abspath(__file__))
TEMPLATE = os.path.join(HERE, "template.icm")  # dylanraga 版 (MHC2 以外のタグを流用)
OUT_DIR = os.path.join(HERE, "profiles")
GAMMAS = (2.2, 2.4, 2.6)


def is_auto_profile(name: str | None) -> bool:
    return bool(name) and name.startswith("gamma") and "_auto_" in name


def profile_name(nits: int, gamma: float = 2.2) -> str:
    return f"gamma{round(gamma * 10)}_auto_{nits}nits.icm"


def make_profile(nits: int, gamma: float = 2.2) -> bytes:
    with open(TEMPLATE, "rb") as f:
        data = f.read()
    # 元プロファイルと同じ min 0.005 / max 800 nits
    mhc2 = build_mhc2(gamma22_lut(nits, gamma=gamma), min_nits=0.0050048828125, max_nits=800.0)
    desc = f"sRGB to Gamma {gamma:.1f} - SDR {nits} nits (auto)"
    return rebuild(data, {b"desc": mluc(desc), b"MHC2": mhc2})


def write_profile(nits: int, gamma: float = 2.2) -> str:
    os.makedirs(OUT_DIR, exist_ok=True)
    path = os.path.join(OUT_DIR, profile_name(nits, gamma))
    with open(path, "wb") as f:
        f.write(make_profile(nits, gamma))
    return path


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("nits", type=int, nargs="+")
    ap.add_argument("--gamma", type=float, default=2.2)
    a = ap.parse_args()
    for n in a.nits:
        print(write_profile(n, a.gamma))
