"""Draws the two pictures the Windows installer shows: the tall one on the
welcome page and the small one in the page header. Needs only Python itself.

Run from the project folder:  python "Design/installer art.py"
"""

import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOGO = ROOT / "src-tauri" / "icons" / "icon.png"
OUT = ROOT / "src-tauri" / "installer"
SCALE = 4  # drawn larger, then shrunk, so edges come out smooth


def read_png(path):
    """Returns (width, height, rows) for an 8-bit RGBA PNG. rows[y][x] = (r, g, b, a)."""
    data = path.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
    pos, chunks, header = 8, [], None
    while pos < len(data):
        length, kind = struct.unpack(">I4s", data[pos : pos + 8])
        body = data[pos + 8 : pos + 8 + length]
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            chunks.append(body)
        pos += 12 + length
    width, height, depth, colour, _, _, interlace = header
    assert (depth, colour, interlace) == (8, 6, 0), "expected a plain 8-bit RGBA PNG"
    raw = zlib.decompress(b"".join(chunks))
    stride, rows, prev = width * 4, [], bytearray(width * 4)
    for y in range(height):
        start = y * (stride + 1)
        kind, line = raw[start], bytearray(raw[start + 1 : start + 1 + stride])
        for i in range(stride):
            left = line[i - 4] if i >= 4 else 0
            up = prev[i]
            corner = prev[i - 4] if i >= 4 else 0
            if kind == 1:
                line[i] = (line[i] + left) & 255
            elif kind == 2:
                line[i] = (line[i] + up) & 255
            elif kind == 3:
                line[i] = (line[i] + (left + up) // 2) & 255
            elif kind == 4:
                p = left + up - corner
                pa, pb, pc = abs(p - left), abs(p - up), abs(p - corner)
                best = left if pa <= pb and pa <= pc else up if pb <= pc else corner
                line[i] = (line[i] + best) & 255
        rows.append([tuple(line[x * 4 : x * 4 + 4]) for x in range(width)])
        prev = line
    return width, height, rows


def mix(a, b, t):
    return tuple(a[i] + (b[i] - a[i]) * t for i in range(3))


def ramp(stops, t):
    """Colour at position t (0..1) along a list of (position, colour) stops."""
    t = min(1.0, max(0.0, t))
    for (p0, c0), (p1, c1) in zip(stops, stops[1:]):
        if t <= p1:
            return mix(c0, c1, (t - p0) / (p1 - p0) if p1 > p0 else 0)
    return stops[-1][1]


def rgb(text):
    return tuple(int(text[i : i + 2], 16) for i in (1, 3, 5))


SKY = [(0, rgb("#130a29")), (0.42, rgb("#2a1160")), (0.68, rgb("#8a2390")), (0.86, rgb("#ff4fa3")), (1, rgb("#ff9a3c"))]
SUN = [(0, rgb("#ffe66d")), (0.5, rgb("#ff9a3c")), (1, rgb("#ff3d8b"))]


def paste(canvas, logo, left, top, size):
    """Draws the logo into a square of `size` pixels with its top-left corner at (left, top)."""
    w, h, rows = logo
    for y in range(size):
        for x in range(size):
            r, g, b, a = rows[min(h - 1, y * h // size)][min(w - 1, x * w // size)]
            if a and 0 <= top + y < len(canvas) and 0 <= left + x < len(canvas[0]):
                canvas[top + y][left + x] = mix(canvas[top + y][left + x], (r, g, b), a / 255)


def shrink(canvas):
    height, width = len(canvas) // SCALE, len(canvas[0]) // SCALE
    out = []
    for y in range(height):
        row = []
        for x in range(width):
            total = [0.0, 0.0, 0.0]
            for dy in range(SCALE):
                for dx in range(SCALE):
                    px = canvas[y * SCALE + dy][x * SCALE + dx]
                    total[0] += px[0]
                    total[1] += px[1]
                    total[2] += px[2]
            row.append(tuple(int(round(v / (SCALE * SCALE))) for v in total))
        out.append(row)
    return out


def write_bmp(path, rows):
    height, width = len(rows), len(rows[0])
    pad = (4 - width * 3 % 4) % 4
    body = bytearray()
    for row in reversed(rows):
        for r, g, b in row:
            body += bytes((b, g, r))
        body += b"\x00" * pad
    header = b"BM" + struct.pack("<IHHI", 54 + len(body), 0, 0, 54)
    info = struct.pack("<IiiHHIIiiII", 40, width, height, 1, 24, 0, len(body), 2835, 2835, 0, 0)
    path.write_bytes(header + info + body)


def sidebar(logo):
    """164 by 314: night sky fading to sunset, the striped sun going down, the logo above it."""
    width, height = 164 * SCALE, 314 * SCALE
    canvas = [[ramp(SKY, y / height)] * width for y in range(height)]
    canvas = [list(row) for row in canvas]
    cx, cy, radius = width / 2, 236 * SCALE, 64 * SCALE
    gaps = [(250, 3), (262, 5), (275, 7)]  # where the sun is cut by sky, in final pixels
    horizon = 290 * SCALE
    for y in range(int(cy - radius), min(horizon, int(cy + radius))):
        if any(top * SCALE <= y < (top + thick) * SCALE for top, thick in gaps):
            continue
        half = (radius * radius - (y - cy) ** 2) ** 0.5
        colour = ramp(SUN, (y - (cy - radius)) / (2 * radius))
        for x in range(int(cx - half), int(cx + half)):
            canvas[y][x] = colour
    for y in range(horizon, height):
        line = rgb("#2de2e6") if y < horizon + 2 * SCALE else rgb("#130a29")
        canvas[y] = [line] * width
    size = 104 * SCALE
    paste(canvas, logo, (width - size) // 2, 34 * SCALE, size)
    return shrink(canvas)


def header(logo):
    """150 by 57: the same sky left to right with the logo on the right."""
    width, height = 150 * SCALE, 57 * SCALE
    across = [(0, rgb("#130a29")), (0.45, rgb("#2a1160")), (0.8, rgb("#8a2390")), (1, rgb("#ff4fa3"))]
    row = [ramp(across, x / width) for x in range(width)]
    canvas = [list(row) for _ in range(height)]
    for y in range(height - 2 * SCALE, height):
        canvas[y] = [rgb("#2de2e6")] * width
    size = 45 * SCALE
    paste(canvas, logo, width - size - 6 * SCALE, 5 * SCALE, size)
    return shrink(canvas)


if __name__ == "__main__":
    OUT.mkdir(exist_ok=True)
    logo = read_png(LOGO)
    write_bmp(OUT / "sidebar.bmp", sidebar(logo))
    write_bmp(OUT / "header.bmp", header(logo))
    print("wrote", OUT / "sidebar.bmp", "and", OUT / "header.bmp")
