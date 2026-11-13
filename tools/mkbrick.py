#!/usr/bin/env python3
"""mkbrick - build the UxROM level banks for the SHEIKHEMU BRICK ROM.

The game engine (brick.asm, assembled by mini_asm) lives in the fixed
last 16 KiB bank; this tool generates the three switchable level banks
(they hold the brick layout, the palette and the speed parameters at
fixed offsets the engine reads through the $8000 window after every
bank register write) and packs the four banks into one 64 KiB PRG
image.

Level bank layout (offsets inside the 16 KiB bank):
  $8000: 84 layout bytes (6 rows x 14 cells, 1 = alive)
  $8054: 32 palette bytes ($3F00-$3F1F order)
  $8074: speed, points, brick tile, signature ($B0+bank)

D-31: bg palette 0 must match the engine's attract palette - walls
gray (color 1), two-plane text white (color 3); the old $00 entries
were black-on-black invisible ink.

Original content: the levels below are this project's own design.
"""
import argparse
import sys

BANK = 0x4000
LAYOUT_OFF = 0x0000
PALETTE_OFF = 0x0054
PARAMS_OFF = 0x0074

ROWS, COLS = 6, 14
TOWER_COL = 7


def palette(brick: int, brick_hi: int, brick_lo: int) -> bytes:
    """The 32 palette bytes for one level (bg + sprite palettes)."""
    # D-31: bg 0 color 3 is the text ink - $28, the white entry of this
    # emulator's calibrated 2C02 table (its $30 renders pink).
    return bytes([
        0x0F, 0x10, 0x30, 0x28,        # bg 0: field, walls gray, text white (D-31)
        0x0F, brick, brick_hi, brick_lo,  # bg 1: the level's bricks
        0x0F, 0x10, 0x30, 0x28,        # bg 2
        0x0F, 0x10, 0x30, 0x28,        # bg 3
        0x0F, 0x30, 0x21, 0x30,        # sprite 4: paddle
        0x0F, 0x30, brick, 0x30,       # sprite 5: ball
        0x0F, 0x30, 0x28, 0x30,        # sprite 6: life icons
        0x0F, 0x10, 0x30, 0x30,        # sprite 7
    ])


def tower(height: int) -> bytes:
    """A tower of `height` bricks in the center column."""
    grid = bytearray(ROWS * COLS)
    for row in range(min(height, ROWS)):
        grid[row * COLS + TOWER_COL] = 1
    return bytes(grid)


LEVELS = [
    # (tower height, speed px/frame, points, brick tile, colors)
    (4, 2, 10, 0x02, (0x16, 0x27, 0x06)),   # level 1: red,   4 bricks
    (5, 2, 20, 0x03, (0x2A, 0x1A, 0x0A)),   # level 2: green, 5 bricks
    (6, 3, 30, 0x04, (0x28, 0x38, 0x18)),   # level 3: gold,  6 bricks
]


def build_bank(index: int) -> bytes:
    height, speed, points, tile, colors = LEVELS[index]
    bank = bytearray(BANK)
    bank[LAYOUT_OFF:LAYOUT_OFF + ROWS * COLS] = tower(height)
    bank[PALETTE_OFF:PALETTE_OFF + 32] = palette(*colors)
    bank[PARAMS_OFF + 0] = speed
    bank[PARAMS_OFF + 1] = points
    bank[PARAMS_OFF + 2] = tile
    bank[PARAMS_OFF + 3] = 0xB0 + index      # signature byte
    return bytes(bank)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="cmd", required=True)
    build = sub.add_parser("build", help="build the level banks")
    build.add_argument("--engine", required=True,
                       help="the assembled fixed-bank engine image")
    build.add_argument("--output", required=True, help="the packed PRG image")
    args = parser.parse_args()

    if args.cmd == "build":
        engine = open(args.engine, "rb").read()
        if len(engine) != BANK:
            sys.exit(f"engine image must be exactly 16 KiB (got {len(engine)})")
        prg = b"".join(build_bank(i) for i in range(3)) + engine
        if len(prg) != 4 * BANK:
            sys.exit("internal packing error")
        open(args.output, "wb").write(prg)
        print(f"mkbrick: wrote {len(prg)} bytes to {args.output} "
              f"(3 level banks + fixed engine)")
        for i, (height, speed, points, tile, colors) in enumerate(LEVELS):
            print(f"  bank {i}: tower {height} bricks, speed {speed}, "
                  f"points {points}, tile ${tile:02X}, "
                  f"brick color ${colors[0]:02X}")


if __name__ == "__main__":
    main()
