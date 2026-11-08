"""MHC2 タグの読み書きと sRGB→Gamma 2.2 LUT の計算。"""
import struct

# --- ST 2084 (PQ) ---
M1, M2 = 2610 / 16384, 2523 / 4096 * 128
C1, C2, C3 = 3424 / 4096, 2413 / 4096 * 32, 2392 / 4096 * 32


def pq_eotf(e: float) -> float:
    """PQ 信号 (0-1) → 輝度 (nits)"""
    p = e ** (1 / M2)
    return 10000 * (max(p - C1, 0) / (C2 - C3 * p)) ** (1 / M1)


def pq_inv_eotf(nits: float) -> float:
    y = (max(nits, 0) / 10000) ** M1
    return ((C1 + C2 * y) / (1 + C3 * y)) ** M2


def srgb_inv_eotf(l: float) -> float:
    """線形 (0-1) → sRGB 信号"""
    return 12.92 * l if l <= 0.0031308 else 1.055 * l ** (1 / 2.4) - 0.055


def gamma22_lut(white: float, n: int = 1024, gamma: float = 2.2, black: float = 0.0) -> list[float]:
    """dylanraga/gen-srgb-to-gamma-lut の NVIDIA 方式 (black=0)。PQ 入力 → PQ 出力。"""
    out = []
    for i in range(n):
        x = i / (n - 1)
        lum = pq_eotf(x)
        if lum > white:
            out.append(x)
            continue
        s = srgb_inv_eotf(lum / white)
        out.append(pq_inv_eotf((white - black) * s ** gamma + black))
    return out


# --- MHC2 タグ ---
def s15(v: float) -> bytes:
    return struct.pack(">i", round(v * 65536))


def parse_mhc2(tag: bytes) -> dict:
    assert tag[:4] == b"MHC2"
    n, mn, mx, om, o0, o1, o2 = struct.unpack_from(">IiiIIII", tag, 8)
    matrix = [v / 65536 for v in struct.unpack_from(">12i", tag, om)]
    luts = []
    for o in (o0, o1, o2):
        assert tag[o : o + 4] == b"sf32"
        luts.append([v / 65536 for v in struct.unpack_from(f">{n}i", tag, o + 8)])
    return {"n": n, "min": mn / 65536, "max": mx / 65536, "matrix": matrix, "luts": luts}


def build_mhc2(lut: list[float], min_nits: float = 0.0, max_nits: float = 800.0,
               matrix: list[float] | None = None) -> bytes:
    n = len(lut)
    matrix = matrix or [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]
    head = 36
    om = head
    o0 = om + 48
    lut_size = 8 + 4 * n
    o1, o2 = o0 + lut_size, o0 + 2 * lut_size
    b = b"MHC2" + bytes(4) + struct.pack(">I", n) + s15(min_nits) + s15(max_nits)
    b += struct.pack(">IIII", om, o0, o1, o2)
    b += b"".join(s15(v) for v in matrix)
    lut_blob = b"sf32" + bytes(4) + b"".join(s15(v) for v in lut)
    return b + lut_blob * 3
