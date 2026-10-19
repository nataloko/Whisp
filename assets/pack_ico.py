# /// script
# requires-python = ">=3.8"
# ///
"""Packs PNG images into a .ico file (PNG frames, supported since Vista)."""
import struct, sys

out, *pngs = sys.argv[1:]
images = [open(p, "rb").read() for p in pngs]
header = struct.pack("<HHH", 0, 1, len(images))
offset = 6 + 16 * len(images)
entries, data = b"", b""
for img in images:
    w, h = struct.unpack(">II", img[16:24])
    entries += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(img), offset + len(data))
    data += img
open(out, "wb").write(header + entries + data)
