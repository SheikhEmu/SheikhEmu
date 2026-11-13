#!/usr/bin/env python3
"""hud_row_check.py - the D-35 HUD-row tile-map gate (stdlib only).

The owner's evidence standard for the HUD: no VLM or ASCII readings of
small text - the proof is the tile map. This gate reconstructs row 0 of
the final frame of each deterministic scenario from the CLI's dumped
PPM, matches every 8x8 column block against the game's own tile
bitmaps (the GLYPHS table of mkfont_brick.py plus the life icon), and
asserts the full expected row - the complete, non-overlapping
"SCORE xxxx LEVEL n LIVES ***" line.

Run by tools/check_brick.sh as part of the playability gate.
Usage: hud_row_check.py <frame.ppm> <state>
Exit 0 = verified; 1 = mismatch (report on stdout).
"""
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__))))
from mkfont_brick import GLYPHS, glyph_rows  # noqa: E402

# Background palette 0 of the ROM ($0F, $10, $30, $28) as this
# emulator's calibrated 2C02 table renders it (bg 0 = the HUD row).
PALETTE = [
    ((0, 0, 0), 0),        # $0F black
    ((156, 156, 156), 1),  # $10 gray
    ((255, 171, 171), 2),  # $30 pink (unused)
    ((255, 255, 255), 3),  # $28 white: the two-plane text ink
]

# The life-icon tile (chr_tiles entry 7): ink rows 2-4 at columns 3-5.
ICON_ROWS = [0x00, 0x00, 0x18, 0x18, 0x18, 0x00, 0x00, 0x00]

EXPECTED = {
    #          01234567890123456789012345678901
    "title": "  SCORE 0000 LEVEL 1 LIVES ***  ",
    "early": "  SCORE 0000 LEVEL 1 LIVES ***  ",
    "mid":   "  SCORE 0030 LEVEL 1 LIVES ***  ",
    "win":   "  SCORE 0320 LEVEL 3 LIVES ***  ",
    "over":  "  SCORE 0030 LEVEL 1 LIVES      ",
}


def read_ppm(path):
    """A P6 PPM -> a flat list of (r, g, b) rows (256x240)."""
    with open(path, "rb") as handle:
        data = handle.read()
    marker, rest = data.split(b"\n", 1)
    if marker != b"P6":
        raise SystemExit(f"not a P6 PPM: {path}")
    dims, rest = rest.split(b"\n", 1)
    width, height = (int(x) for x in dims.split())
    _maxval, pixels = rest.split(b"\n", 1)
    if (width, height) != (256, 240):
        raise SystemExit(f"unexpected PPM size {width}x{height}")
    return [tuple(pixels[3 * i:3 * i + 3]) for i in range(256 * 240)]


def tile_grid(pixels, col):
    """The 8x8 color-index grid of one row-0 tile column."""
    grid = []
    for dy in range(8):
        row = []
        for dx in range(8):
            p = pixels[(dy * 256) + (col * 8 + dx)]
            best, index = min(PALETTE, key=lambda kv: sum(abs(a - b) for a, b in zip(kv[0], p)))
            row.append(index)
        grid.append(row)
    return grid


def candidates():
    out = {" ": [[0] * 8 for _ in range(8)]}
    for ch in GLYPHS:
        if ch == " ":
            continue
        # plane 0 == plane 1 -> the ink renders as color 3
        bytes_ = glyph_rows(ch)
        out[ch] = [[3 if (byte >> (7 - dx)) & 1 else 0 for dx in range(8)] for byte in bytes_]
    out["*"] = [[3 if (byte >> (7 - dx)) & 1 else 0 for dx in range(8)] for byte in ICON_ROWS]
    return out


def read_row(pixels):
    names = []
    unknown = []
    table = candidates()
    for col in range(32):
        grid = tile_grid(pixels, col)
        found = [name for name, cand in table.items() if cand == grid]
        if not found:
            found = ["?"]
            unknown.append(col)
        names.append(found)
    return names, unknown


def main():
    if len(sys.argv) != 3:
        print("usage: hud_row_check.py <frame.ppm> <state>")
        return 2
    path, state = sys.argv[1], sys.argv[2]
    if state not in EXPECTED:
        print(f"unknown state {state}; known: {sorted(EXPECTED)}")
        return 2
    pixels = read_ppm(path)
    names, unknown = read_row(pixels)
    decoded = "".join(n[0] for n in names)
    expected = EXPECTED[state]
    print(f"HUD row tile map ({state}, {os.path.basename(path)})")
    print(f"  decoded : '{decoded}'")
    print(f"  expected: '{expected}'")

    problems = []
    if unknown:
        problems.append(f"unidentified tiles at columns {unknown}")
    for i, want in enumerate(expected):
        if want not in names[i]:
            problems.append(f"column {i}: got {names[i]} want '{want}'")
    if problems:
        print("FAIL: the HUD row tile map does not match the design")
        for p in problems:
            print(" -", p)
        return 1
    print("PASS: the HUD row tile map matches the design exactly")
    return 0


if __name__ == "__main__":
    sys.exit(main())
