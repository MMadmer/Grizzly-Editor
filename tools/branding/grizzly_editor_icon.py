"""The editor's icon: the grizzly's head of the game's splash, purple lines on a black head, transparent around it.

The head is the game repo's tools/branding/grizzly_emblem.py (OUTLINE, LINES, NOSE), without the name and without the neon.
At small sizes the lines get relatively thicker and the fine ones go: a 16-pixel icon holds the silhouette, the eyes and the nose.

    python tools/branding/grizzly_editor_icon.py
"""
import os

import numpy as np
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "..", "assets")
COLOR = (0x9D, 0x7F, 0xF0)

OUTLINE = [(0.0, 0.66), (0.19, 0.69), (0.30, 0.70), (0.31, 0.79), (0.37, 0.865), (0.45, 0.87), (0.51, 0.80),
           (0.50, 0.67), (0.60, 0.55), (0.69, 0.33), (0.66, 0.14), (0.56, -0.03), (0.40, -0.15), (0.20, -0.25),
           (0.0, -0.27)]
EAR = [(0.34, 0.73), (0.38, 0.81), (0.45, 0.815), (0.475, 0.72)]
BROW = [(0.13, 0.505), (0.29, 0.535)]
EYE = [(0.14, 0.445), (0.27, 0.47), (0.22, 0.41), (0.14, 0.445)]
MUZZLE = [(0.10, 0.43), (0.17, 0.19), (0.20, 0.05), (0.12, -0.10), (0.0, -0.13)]
MOUTH = [(0.0, 0.05), (0.0, -0.03), (0.07, -0.065)]
NOSE = [(-0.12, 0.18), (0.12, 0.18), (0.10, 0.10), (0.03, 0.05), (-0.03, 0.05), (-0.10, 0.10), (-0.12, 0.18)]
TOP, BOTTOM, HALF_W = 0.87, -0.27, 0.69


def segments(size):
    fine = size >= 40
    lines = [OUTLINE, MUZZLE, EYE] + ([EAR, BROW, MOUTH] if fine else [])
    out = []
    for line in lines:
        out += list(zip(line, line[1:]))
        if any(x != 0.0 for x, _ in line):
            mirrored = [(-x, y) for x, y in line]
            out += list(zip(mirrored, mirrored[1:]))
    out += list(zip(NOSE, NOSE[1:]))
    return out


def inside_head(px, py):
    """Even-odd test against the whole silhouette: the outline and its mirror image."""
    ring = OUTLINE + [(-x, y) for x, y in reversed(OUTLINE)]
    inside = np.zeros(px.shape, dtype=bool)
    for (x0, y0), (x1, y1) in zip(ring, ring[1:] + ring[:1]):
        if y0 == y1:
            continue
        crosses = (y0 > py) != (y1 > py)
        inside ^= crosses & (px < x0 + (py - y0) * (x1 - x0) / (y1 - y0))
    return inside


def draw(size):
    margin = max(1.0, size * 0.04)
    unit = (size - 2 * margin) / max(TOP - BOTTOM, 2 * HALF_W)
    middle_y = (TOP + BOTTOM) / 2
    ys, xs = np.mgrid[0:size, 0:size]
    px = (xs + 0.5 - size / 2) / unit
    py = (size / 2 - ys - 0.5) / unit + middle_y
    d = np.full((size, size), 1e9)
    for (x0, y0), (x1, y1) in segments(size):
        bx, by = x1 - x0, y1 - y0
        ax, ay = px - x0, py - y0
        h = np.clip((ax * bx + ay * by) / (bx * bx + by * by), 0, 1)
        d = np.minimum(d, np.hypot(ax - bx * h, ay - by * h))
    half = max(0.62 / unit, 0.016)
    aa = 0.5 / unit
    alpha = np.clip((half + aa - d) / (2 * aa), 0, 1)
    # the edge of the fill lies under the outline's stroke, so it needs no smoothing of its own
    head = inside_head(px, py)
    rgba = np.zeros((size, size, 4), dtype=np.uint8)
    for channel, value in enumerate(COLOR):
        rgba[..., channel] = np.where(head, value * alpha + 0.5, value).astype(np.uint8)
    rgba[..., 3] = np.where(head, 255, alpha * 255 + 0.5).astype(np.uint8)
    return Image.fromarray(rgba, "RGBA")


def main():
    sizes = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
    images = [draw(s) for s in sizes]
    png = os.path.join(OUT, "grizzly-editor-icon.png")
    draw(512).save(png)
    ico = os.path.join(OUT, "grizzly-editor-icon.ico")
    images[-1].save(ico, format="ICO", sizes=[(s, s) for s in sizes], append_images=images[:-1])
    print("wrote", os.path.normpath(png), os.path.normpath(ico))


if __name__ == "__main__":
    main()
