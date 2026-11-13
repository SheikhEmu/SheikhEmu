#!/usr/bin/env python3
"""Generate the 8x8 tile .db blocks for the brick game ROM.

Original bitmaps written for this project (classic 5x7 dot-matrix
letterforms redrawn by hand): 10 digits + the letters the game's own
texts need. Emits `mini_asm` `.db` lines ready to paste into
brick.asm (tile data lives in the ROM and is uploaded to CHR-RAM at
boot through $2007, exactly like pattern.asm does).

D-31: every glyph is emitted as a full two-plane 16-byte tile. The
pattern is duplicated into plane 1 so text pixels are color 3 of the
background palette (a bright ink), while one-plane tiles (walls,
bricks) keep color 1. Before D-31 the glyphs were 8-byte single-plane
"tiles": the PPU always decodes 16 bytes per tile, so each glyph's
second plane was the next glyph's data - the on-screen garble the
owner's videos exposed.

Usage: python3 mkfont_brick.py            # print the .db lines
       python3 mkfont_brick.py --check    # verify 8 rows per glyph
"""
import sys

# 5x7 glyphs, each row a 5-char string ('#' = ink). Row 8 (bottom) is
# blank spacing except for descender-free letters; digits get an
# underline-free classic calculator look.
GLYPHS = {
    "0": [" ### ", "#   #", "#   #", "#   #", "#   #", "#   #", " ### "],
    "1": ["  #  ", " ##  ", "  #  ", "  #  ", "  #  ", "  #  ", " ### "],
    "2": [" ### ", "#   #", "    #", "   # ", "  #  ", " #   ", "#####"],
    "3": [" ### ", "#   #", "    #", "  ## ", "    #", "#   #", " ### "],
    "4": ["   # ", "  ## ", " # # ", "#  # ", "#####", "   # ", "   # "],
    "5": ["#####", "#    ", "#### ", "    #", "    #", "#   #", " ### "],
    "6": [" ### ", "#   #", "#    ", "#### ", "#   #", "#   #", " ### "],
    "7": ["#####", "    #", "   # ", "  #  ", "  #  ", "  #  ", "  #  "],
    "8": [" ### ", "#   #", "#   #", " ### ", "#   #", "#   #", " ### "],
    "9": [" ### ", "#   #", "#   #", " ####", "    #", "#   #", " ### "],
    "A": [" ### ", "#   #", "#   #", "#####", "#   #", "#   #", "#   #"],
    "B": ["#### ", "#   #", "#   #", "#### ", "#   #", "#   #", "#### "],
    "C": [" ### ", "#   #", "#    ", "#    ", "#    ", "#   #", " ### "],
    "E": ["#####", "#    ", "#    ", "#### ", "#    ", "#    ", "#####"],
    "G": [" ### ", "#   #", "#    ", "#  ##", "#   #", "#   #", " ### "],
    "H": ["#   #", "#   #", "#   #", "#####", "#   #", "#   #", "#   #"],
    "I": [" ### ", "  #  ", "  #  ", "  #  ", "  #  ", "  #  ", " ### "],
    "K": ["#   #", "#  # ", "# #  ", "##   ", "# #  ", "#  # ", "#   #"],
    "L": ["#    ", "#    ", "#    ", "#    ", "#    ", "#    ", "#####"],
    "M": ["#   #", "## ##", "# # #", "#   #", "#   #", "#   #", "#   #"],
    "N": ["#   #", "##  #", "# # #", "# # #", "#  ##", "#   #", "#   #"],
    "O": [" ### ", "#   #", "#   #", "#   #", "#   #", "#   #", " ### "],
    "P": ["#### ", "#   #", "#   #", "#### ", "#    ", "#    ", "#    "],
    "R": ["#### ", "#   #", "#   #", "#### ", "# #  ", "#  # ", "#   #"],
    "S": [" ####", "#    ", "#    ", " ### ", "    #", "    #", "#### "],
    "T": ["#####", "  #  ", "  #  ", "  #  ", "  #  ", "  #  ", "  #  "],
    "U": ["#   #", "#   #", "#   #", "#   #", "#   #", "#   #", " ### "],
    "V": ["#   #", "#   #", "#   #", "#   #", "#   #", " # # ", "  #  "],
    "Y": ["#   #", "#   #", "#   #", " # # ", "  #  ", "  #  ", "  #  "],
    "W": ["#   #", "#   #", "#   #", "# # #", "# # #", "## ##", "#   #"],
    " ": ["     ", "     ", "     ", "     ", "     ", "     ", "     "],
}

# Non-font tiles the game ROM defines directly in asm (wall, bricks,
# paddle, ball, life icon): these are drawn as full 8x8 bitmaps here
# only for the --check pass; brick.asm carries its own copies.
FONT_ORDER = (
    list("0123456789")
    + list("ABCEGHIKLMNOPRSTUVWY")
    + list(" ")
)


def glyph_rows(ch: str) -> list[int]:
    """One glyph -> 8 tile rows (5x7 centered in 8x8, row 8 blank)."""
    rows5 = GLYPHS[ch]
    out = []
    for r in rows5:
        bits = "0" + r + "00"  # 1px left padding, 2px right
        out.append(int(bits.replace("#", "1").replace(" ", "0"), 2))
    out.append(0)
    return out


def main() -> None:
    if "--check" in sys.argv:
        for ch in FONT_ORDER:
            rows = glyph_rows(ch)
            assert len(rows) == 8, ch
            for v in rows:
                assert 0 <= v <= 0xFF, (ch, v)
        print(f"check ok: {len(FONT_ORDER)} glyphs, 8 rows each")
        return
    for ch in FONT_ORDER:
        rows = ", ".join(f"${v:02X}" for v in glyph_rows(ch))
        comment = "space" if ch == " " else ch
        print(f"    .db {rows}          ; '{comment}' plane 0")
        print(f"    .db {rows}          ; '{comment}' plane 1 (D-31: color 3)")


if __name__ == "__main__":
    main()
