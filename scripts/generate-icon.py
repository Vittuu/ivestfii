"""Generate the code-defined application icon using only Python's standard library."""
from pathlib import Path
import struct
import zlib


def chunk(kind: bytes, data: bytes) -> bytes:
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))


size = 32
rows = bytearray()
for y in range(size):
    rows.append(0)
    for x in range(size):
        bar = (7 <= x < 11 and 19 <= y < 26) or (14 <= x < 18 and 13 <= y < 26) or (21 <= x < 25 and 6 <= y < 26)
        rows.extend((54, 211, 175, 255) if bar else (12, 24, 39, 255))
png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b'')
directory = Path(__file__).resolve().parent.parent / 'src-tauri' / 'icons'
directory.mkdir(parents=True, exist_ok=True)
(directory / 'icon.png').write_bytes(png)
(directory / 'icon.ico').write_bytes(struct.pack('<HHH', 0, 1, 1) + struct.pack('<BBBBHHII', size, size, 0, 0, 1, 32, len(png), 22) + png)
