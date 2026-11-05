#!/usr/bin/env python3
"""Draws the menu bar's busy frames and its needs-you tick. Black frames are template images."""
import math, struct, zlib
from pathlib import Path

SIZE, SAMPLES = 44, 4
HERE = Path(__file__).parent
TICK = [(7, 23), (19.5, 34), (36, 9)]
RED = (229, 72, 58)
FRAMES = 8

def segment_distance(x, y, a, b):
    ax, ay, bx, by = *a, *b
    t = max(0, min(1, ((x - ax) * (bx - ax) + (y - ay) * (by - ay)) / ((bx - ax) ** 2 + (by - ay) ** 2)))
    return math.hypot(x - ax - t * (bx - ax), y - ay - t * (by - ay))

def in_tick(x, y, points, width):
    return any(segment_distance(x, y, a, b) <= width / 2 for a, b in zip(points, points[1:]))

def in_ring(x, y, start):
    radius = math.hypot(x - 22, y - 22)
    if abs(radius - 19.5) > 1.6:
        return False
    return (math.degrees(math.atan2(y - 22, x - 22)) - start) % 360 < 280

def draw(inside, colour):
    rows = []
    for py in range(SIZE):
        row = bytearray([0])
        for px in range(SIZE):
            hits = sum(inside(px + (i + 0.5) / SAMPLES, py + (j + 0.5) / SAMPLES) for i in range(SAMPLES) for j in range(SAMPLES))
            row += bytes(colour) + bytes([round(255 * hits / SAMPLES ** 2)])
        rows.append(bytes(row))
    return b"".join(rows)

def png(path, pixels):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    header = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0)
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(pixels, 9)) + chunk(b"IEND", b""))

small = [(22 + (x - 21.5) * 0.56, 22 + (y - 21.5) * 0.56) for x, y in TICK]
for frame in range(FRAMES):
    start = frame * 360 / FRAMES
    png(HERE / f"tray-busy-{frame}.png", draw(lambda x, y: in_tick(x, y, small, 4.2) or in_ring(x, y, start), (0, 0, 0)))
png(HERE / "tray-needs.png", draw(lambda x, y: in_tick(x, y, TICK, 6.4), RED))
