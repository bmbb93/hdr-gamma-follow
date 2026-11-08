"""ICC プロファイルのタグを差し替えて書き出す（プロファイル ID の MD5 も再計算）。

使い方（表示名だけ変える）:
    uv run relabel_icc.py <入力.icm> <出力.icm> "<新しい説明>"

差し替えないタグ (MHC2 など) は中身をそのまま保持する。共有オフセット (rTRC/gTRC/bTRC) も維持。
"""
import hashlib
import struct
import sys


def mluc(text: str) -> bytes:
    s = text.encode("utf-16-be")
    # 'mluc' + reserved + count=1 + recsize=12 + enUS + len + offset(=28)
    return b"mluc" + struct.pack(">III", 0, 1, 12) + b"enUS" + struct.pack(">II", len(s), 28) + s


def pad4(b: bytes) -> bytes:
    return b + b"\0" * (-len(b) % 4)


def rebuild(data: bytes, replace: dict[bytes, bytes]) -> bytes:
    """replace = {タグ署名: 新しいタグデータ}。共有されていないタグのみ差し替え可。"""
    (count,) = struct.unpack_from(">I", data, 128)
    entries = [struct.unpack_from(">4sII", data, 132 + 12 * i) for i in range(count)]
    by_loc = {}
    for sig, off, size in entries:
        by_loc.setdefault((off, size), []).append(sig)

    pos = 132 + 12 * count
    body = b""
    new_loc = {}
    for (off, size), sigs in sorted(by_loc.items()):
        hit = [s for s in sigs if s in replace]
        if hit and len(sigs) > 1:
            raise ValueError(f"shared tag {sigs} cannot be replaced individually")
        blob = replace[hit[0]] if hit else data[off : off + size]
        new_loc[(off, size)] = (pos, len(blob))
        blob = pad4(blob)
        body += blob
        pos += len(blob)

    table = struct.pack(">I", count)
    for sig, off, size in entries:
        noff, nsize = new_loc[(off, size)]
        table += struct.pack(">4sII", sig, noff, nsize)

    out = bytearray(data[:128] + table + body)
    struct.pack_into(">I", out, 0, len(out))  # profile size

    # Profile ID: flags(44-47), intent(64-67), ID(84-99) を 0 にして MD5
    tmp = bytearray(out)
    tmp[44:48] = b"\0" * 4
    tmp[64:68] = b"\0" * 4
    tmp[84:100] = b"\0" * 16
    out[84:100] = hashlib.md5(tmp).digest()
    return bytes(out)


def relabel(data: bytes, desc: str) -> bytes:
    return rebuild(data, {b"desc": mluc(desc)})


if __name__ == "__main__":
    src, dst, desc = sys.argv[1:4]
    with open(src, "rb") as f:
        data = f.read()
    with open(dst, "wb") as f:
        f.write(relabel(data, desc))
    print(f"{dst}: {desc}")
