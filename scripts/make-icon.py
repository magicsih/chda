#!/usr/bin/env python3
"""Render the chda icon as PNGs and an .icns.

Usage: scripts/make-icon.py <out-dir>

Pure Python plus the macOS `sips` and `iconutil` tools. The 1024 px source
follows the macOS icon grid: an 824 px rounded tile centred on a transparent
canvas, with a soft shadow in the margin. Shapes are drawn as signed
distances, so every edge is anti-aliased through alpha.
"""
import math
import os
import struct
import subprocess
import sys
import zlib

SIZE = 1024
TILE = 824
RADIUS = 185
BG = (0x1E, 0x1E, 0x2E)
ACCENT = (0x89, 0xB4, 0xFA)
FG = (0xCD, 0xD6, 0xF4)
SHADOW_OFFSET = 10
SHADOW_SOFTNESS = 24
SHADOW_ALPHA = 0.3


def png(path, size, pixels):
    """Write RGBA rows (lists of (r, g, b, a) tuples) as an 8-bit PNG."""
    raw = b"".join(b"\x00" + bytes(c for px in row for c in px) for row in pixels)

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(raw, 9)))
        f.write(chunk(b"IEND", b""))


def rounded_rect(cx, cy, half_w, half_h, r):
    """Signed distance to a rounded rectangle centred on (cx, cy)."""
    def dist(px, py):
        qx = abs(px - cx) - half_w + r
        qy = abs(py - cy) - half_h + r
        outside = math.hypot(max(qx, 0.0), max(qy, 0.0))
        return outside + min(max(qx, qy), 0.0) - r
    return dist


def segment(ax, ay, bx, by, half_width):
    """Signed distance to a thick line segment with round caps."""
    def dist(px, py):
        dx, dy = bx - ax, by - ay
        t = max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)))
        return math.hypot(px - ax - t * dx, py - ay - t * dy) - half_width
    return dist


def coverage(d):
    """Fraction of a pixel inside a shape whose edge is `d` px away."""
    return max(0.0, min(1.0, 0.5 - d))


def over(top, alpha, bottom):
    """`top` composited with `alpha` over an opaque `bottom` color."""
    return tuple(t * alpha + b * (1 - alpha) for t, b in zip(top, bottom))


def draw(size):
    s = size / SIZE
    c = size / 2
    half = TILE * s / 2
    tile = rounded_rect(c, c, half, half, RADIUS * s)
    shadow = rounded_rect(c, c + SHADOW_OFFSET * s, half, half, RADIUS * s)
    # A prompt chevron ">" and a cursor block: the terminal, at a glance.
    tip_x, tip_y, arm = size * 0.46, size * 0.5, size * 0.16
    stroke = size * 0.045
    upper = segment(tip_x - arm, tip_y - arm, tip_x, tip_y, stroke)
    lower = segment(tip_x - arm, tip_y + arm, tip_x, tip_y, stroke)
    block = rounded_rect(size * 0.67, size * 0.5, size * 0.11, size * 0.08, size * 0.02)

    rows = []
    for y in range(size):
        py = y + 0.5
        row = []
        for x in range(size):
            px = x + 0.5
            inside = coverage(tile(px, py))
            if inside == 0.0:
                d = shadow(px, py)
                fade = max(0.0, 1.0 - d / (SHADOW_SOFTNESS * s)) if d > 0 else 1.0
                row.append((0, 0, 0, round(255 * SHADOW_ALPHA * fade * fade)))
                continue
            color = BG
            color = over(ACCENT, coverage(min(upper(px, py), lower(px, py))), color)
            color = over(FG, coverage(block(px, py)), color)
            row.append(tuple(round(v) for v in color) + (round(255 * inside),))
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
