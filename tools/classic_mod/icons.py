"""Inventory icons: the cell rectangle an item names, cut out of an atlas, and whether two pictures differ."""
from __future__ import annotations

import io

import numpy as np
from PIL import Image

CELL = 50
DEFAULT_ATLAS = "ui\\ui_icon_equipment"


def decode(data: bytes) -> np.ndarray:
    return np.asarray(Image.open(io.BytesIO(data)).convert("RGBA")).astype(np.int16)


def coords(resolved: dict):
    """(texture, x, y, w, h) in cells, or None when the item names no whole rectangle."""
    def num(k, default=None):
        v = resolved.get(k)
        if v is None:
            return default
        try:
            return int(float(v.split(",")[0]))
        except ValueError:
            return None
    x, y = num("inv_grid_x"), num("inv_grid_y")
    w, h = num("inv_grid_width", 1), num("inv_grid_height", 1)
    if None in (x, y, w, h) or w <= 0 or h <= 0 or x < 0 or y < 0:
        return None
    tex = (resolved.get("icons_texture") or DEFAULT_ATLAS).lower()
    return tex, x, y, w, h


def cut(atlas: np.ndarray, x, y, w, h):
    y0, x0 = y * CELL, x * CELL
    y1, x1 = y0 + h * CELL, x0 + w * CELL
    if y1 > atlas.shape[0] or x1 > atlas.shape[1]:
        return None
    return atlas[y0:y1, x0:x1]


def differs(a, b) -> bool:
    """More than 5 % of the pixels off by more than 40 in a channel (a block compression's noise is less)."""
    if a is None or b is None or a.shape != b.shape:
        return True
    off = np.abs(a - b).max(axis=2) > 40
    return off.mean() > 0.05


def cells_differing(a: np.ndarray, b: np.ndarray) -> int:
    if a.shape != b.shape:
        return 1 << 30
    h, w = a.shape[0] // CELL, a.shape[1] // CELL
    off = (np.abs(a[:h * CELL, :w * CELL] - b[:h * CELL, :w * CELL]).max(axis=2) > 40)
    per = off.reshape(h, CELL, w, CELL).mean(axis=(1, 3))
    return int((per > 0.05).sum())


def save_png(pic: np.ndarray, path: str):
    Image.fromarray(pic.astype(np.uint8), "RGBA").save(path, optimize=True)
