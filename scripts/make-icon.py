#!/usr/bin/env python3
"""Render the chda wordmark icon as PNGs and an .icns.

Usage: scripts/make-icon.py <out-dir>
Needs only the macOS `sips` and `iconutil` tools; draws with Pillow if
available, otherwise with a tiny built-in PNG writer (flat shapes).
"""
import os
import struct
import subprocess
import sys
import zlib

SIZE = 1024
BG = (0x1E, 0x1E, 0x2E)
ACCENT = (0x89, 0xB4, 0xFA)
FG = (0xCD, 0xD6, 0xF4)


def png(path, size, pixels):
    raw = b"".join(b"\x00" + bytes(c for px in row for c in px) for row in pixels)

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(raw, 9)))
        f.write(chunk(b"IEND", b""))


def rounded_rect(x, y, w, h, r):
    def inside(px, py):
        if px < x or py < y or px >= x + w or py >= y + h:
            return False
        cx = min(max(px, x + r), x + w - r)
        cy = min(max(py, y + r), y + h - r)
        return (px - cx) ** 2 + (py - cy) ** 2 <= r * r
    return inside


def draw(size):
    tile = rounded_rect(size * 0.08, size * 0.08, size * 0.84, size * 0.84, size * 0.2)
    # A prompt chevron ">" and a cursor block: the terminal, at a glance.
    chev_w = size * 0.07

    def chevron(px, py):
        # ">" with its point at (cx, cy); arms run up-left and down-left.
        cx, cy = size * 0.46, size * 0.5
        dx, dy = px - cx, py - cy
        return abs(abs(dx) - abs(dy)) < chev_w and dx < chev_w and abs(dy) < size * 0.2

    block = rounded_rect(size * 0.56, size * 0.42, size * 0.22, size * 0.16, size * 0.02)
    rows = []
    for y in range(size):
        row = []
        for x in range(size):
            if not tile(x, y):
                row.append((0, 0, 0))
            elif chevron(x, y):
                row.append(ACCENT)
            elif block(x, y):
                row.append(FG)
            else:
                row.append(BG)
        rows.append(row)
    return rows


def main():
    out = sys.argv[1]
    iconset = os.path.join(out, "chda.iconset")
    os.makedirs(iconset, exist_ok=True)
    base = os.path.join(out, "chda-1024.png")
    png(base, SIZE, draw(SIZE))
    for px in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            name = f"icon_{px}x{px}{'@2x' if scale == 2 else ''}.png"
            subprocess.run(["sips", "-z", str(px * scale), str(px * scale), base, "--out", os.path.join(iconset, name)],
                           check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["iconutil", "-c", "icns", iconset, "-o", os.path.join(out, "chda.icns")], check=True)
    print(os.path.join(out, "chda.icns"))


if __name__ == "__main__":
    main()
