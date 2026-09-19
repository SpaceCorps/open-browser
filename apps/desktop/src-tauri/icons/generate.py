#!/usr/bin/env python3
"""Regenerate the app icons from the one drawing below.

The icons are committed, because a build must not depend on Python being present. This script is
here so that the mark is editable — changing a colour means editing `draw` and re-running it rather
than opening five binaries in an image editor.

    python3 apps/desktop/src-tauri/icons/generate.py

Needs Pillow. On macOS the `.icns` is assembled with `iconutil`, which ships with the OS.
"""

from __future__ import annotations

import pathlib
import shutil
import subprocess
import sys

from PIL import Image, ImageDraw

HERE = pathlib.Path(__file__).resolve().parent

# A browser window on a dark slate ground, in the accent the UI uses for a running agent.
GROUND = (17, 20, 28, 255)
CHROME = (39, 45, 60, 255)
GLASS = (245, 247, 252, 255)
ACCENT = (109, 140, 255, 255)
DOT = (86, 96, 122, 255)

SIZE = 1024


def draw() -> Image.Image:
    image = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    pen = ImageDraw.Draw(image)

    # macOS applies its own mask to nothing here, so the rounded square is part of the art.
    pen.rounded_rectangle((0, 0, SIZE - 1, SIZE - 1), radius=int(SIZE * 0.22), fill=GROUND)

    left, top, right, bottom = int(SIZE * 0.16), int(SIZE * 0.24), int(SIZE * 0.84), int(SIZE * 0.76)
    radius = int(SIZE * 0.05)
    pen.rounded_rectangle((left, top, right, bottom), radius=radius, fill=CHROME)

    # The title bar, and the page under it.
    bar = top + int(SIZE * 0.10)
    pen.rounded_rectangle((left, bar - radius, right, bottom), radius=radius, fill=GLASS)
    pen.rectangle((left, bar, right, bar + radius), fill=GLASS)

    dot_y = top + int(SIZE * 0.05)
    dot_r = int(SIZE * 0.017)
    for index in range(3):
        cx = left + int(SIZE * 0.045) + index * int(SIZE * 0.055)
        pen.ellipse((cx - dot_r, dot_y - dot_r, cx + dot_r, dot_y + dot_r), fill=DOT)

    # Two lines of content, then the pointer: the app is a browser something else is driving.
    line_x = left + int(SIZE * 0.07)
    for offset, width in ((0.055, 0.34), (0.115, 0.22)):
        y = bar + int(SIZE * offset)
        pen.rounded_rectangle(
            (line_x, y, line_x + int(SIZE * width), y + int(SIZE * 0.028)),
            radius=int(SIZE * 0.014),
            fill=(203, 210, 226, 255),
        )

    pointer = [
        (int(SIZE * 0.575), int(SIZE * 0.455)),
        (int(SIZE * 0.575), int(SIZE * 0.735)),
        (int(SIZE * 0.645), int(SIZE * 0.665)),
        (int(SIZE * 0.690), int(SIZE * 0.770)),
        (int(SIZE * 0.745), int(SIZE * 0.744)),
        (int(SIZE * 0.700), int(SIZE * 0.640)),
        (int(SIZE * 0.795), int(SIZE * 0.625)),
    ]
    pen.polygon(pointer, fill=ACCENT)
    return image


def main() -> int:
    master = draw()
    for name, size in (("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256), ("icon.png", 512)):
        master.resize((size, size), Image.LANCZOS).save(HERE / name)
    master.resize((256, 256), Image.LANCZOS).save(
        HERE / "icon.ico", sizes=[(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    )

    iconutil = shutil.which("iconutil")
    if not iconutil:
        print("no iconutil on this machine; icon.icns left as it was", file=sys.stderr)
        return 0
    iconset = HERE / "icon.iconset"
    shutil.rmtree(iconset, ignore_errors=True)
    iconset.mkdir()
    for size in (16, 32, 128, 256, 512):
        master.resize((size, size), Image.LANCZOS).save(iconset / f"icon_{size}x{size}.png")
        master.resize((size * 2, size * 2), Image.LANCZOS).save(iconset / f"icon_{size}x{size}@2x.png")
    subprocess.run([iconutil, "-c", "icns", str(iconset), "-o", str(HERE / "icon.icns")], check=True)
    shutil.rmtree(iconset)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
