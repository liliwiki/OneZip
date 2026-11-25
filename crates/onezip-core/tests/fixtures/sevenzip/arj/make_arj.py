"""生成测试用的 ARJ（存储方式，method 0）：python make_arj.py

格式按 ARJ 技术说明：主头之后每个条目一个本地头（0x60 0xEA、头长度、头、头的 CRC32、
扩展头长度 0），紧跟数据；最后是长度为 0 的头。
"""
import struct
import zlib

DOS_TIME = (46 << 25) | (11 << 21) | (25 << 16) | (12 << 11) | (0 << 5) | 0  # 2026-11-25 12:00


def header(basic: bytes) -> bytes:
    return b"\x60\xEA" + struct.pack("<H", len(basic)) + basic + struct.pack("<I", zlib.crc32(basic)) + b"\0\0"


def main_header(name: str) -> bytes:
    fixed = struct.pack("<BBBBBBBBIIIIHHH", 30, 11, 1, 0, 0x10, 0, 2, 0, DOS_TIME, DOS_TIME, 0, 0, 0, 0, 0)
    return header(fixed + name.encode() + b"\0" + b"\0")


def entry(path: str, data: bytes | None) -> bytes:
    is_dir = data is None
    data = data or b""
    crc = zlib.crc32(data)
    fixed = struct.pack(
        "<BBBBBBBBIIIIHHH",
        30, 11, 1, 0, 0x10, 0, 3 if is_dir else 0, 0, DOS_TIME, len(data), len(data), crc,
        path.rfind("/") + 1, 0x10 if is_dir else 0x20, 0,
    )
    return header(fixed + path.encode() + b"\0" + b"\0") + data


files = [
    ("readme.txt", b"OneZip ARJ fixture\r\n" * 100),
    ("sub", None),
    ("sub/data.bin", bytes((i * 7 + i // 13) & 0xFF for i in range(5000))),
    ("sub/empty.txt", b""),
    ("last.txt", b"the last file\n"),
]
with open("stored.arj", "wb") as f:
    f.write(main_header("stored.arj"))
    for path, data in files:
        f.write(entry(path, data))
    f.write(b"\x60\xEA\0\0")
print("stored.arj written")
