#!/usr/bin/env python3
"""mkbanktest - build PRG/CHR images for the SheikhEmu board-test ROMs.

Composes multi-bank cartridge images for the M3 mapper verification
ROMs (one family per ROM, the explicit §6.3 requirement: no test ROM
exists for these boards, so we wrote our own). Signatures are one
recognizable byte per bank at fixed offsets so a switch/read/compare
loop in the test engine can prove the window arithmetic.

PRG layouts:
  fixed   [data bank 0 .. data bank N-2] + [engine bank]   (UxROM/Camerica:
          the engine lives in the fixed last 16 KiB bank at $C000)
  linear  [engine bank 0] + [data bank 1 .. data bank N-1]  (AxROM/
          BNROM/ColorDreams/GxROM: 32 KiB units, engine in the
          power-on bank)

Data-bank signatures (i = bank index):
  type fixed  (16 KiB units): lo at offset 0,        hi at offset 0x3FFF
  type linear (32 KiB units): lo at offset 0,        hi at offset 0x3FFF
CHR banks (8 KiB units): lo at offset 0, hi at offset 0x1FFF.

Usage:
    mkbanktest.py build-prg --engine FILE --layout fixed|linear
        --banks N --unit BYTES --sig-lo $B0 --sig-hi $B4 --output FILE
    mkbanktest.py build-chr --banks N --sig-lo $C0 --sig-hi $C4
        --output FILE
"""

from __future__ import annotations

import argparse
import sys

UNIT_16K = 16 * 1024
UNIT_32K = 32 * 1024
UNIT_8K = 8 * 1024


def parse_value(text: str) -> int:
    text = text.strip()
    if text.startswith("$"):
        return int(text[1:], 16)
    return int(text, 0)


def build_prg(args: argparse.Namespace) -> int:
    with open(args.engine, "rb") as handle:
        engine = handle.read()
    if len(engine) != args.unit:
        raise SystemExit(
            f"mkbanktest: engine image is {len(engine)} bytes, "
            f"expected exactly {args.unit} (the .org/.fill layout)")
    if not 2 <= args.banks <= 16:
        raise SystemExit(f"mkbanktest: --banks must be 2..16 (got {args.banks})")

    image = bytearray()
    data_count = args.banks - 1
    if args.layout == "fixed":
        # Data banks 0..N-2, engine last (fixed at $C000).
        for bank in range(data_count):
            data = bytearray(args.unit)
            data[0] = (args.sig_lo + bank) & 0xFF
            data[0x3FFF] = (args.sig_hi + bank) & 0xFF
            image.extend(data)
        image.extend(engine)
    else:
        # Engine bank 0 (power-on), data banks 1..N-1.
        image.extend(engine)
        for bank in range(1, args.banks):
            data = bytearray(args.unit)
            data[0] = (args.sig_lo + bank) & 0xFF
            data[0x3FFF] = (args.sig_hi + bank) & 0xFF
            image.extend(data)

    with open(args.output, "wb") as handle:
        handle.write(image)
    layout_note = ("engine last (fixed $C000)" if args.layout == "fixed"
                   else "engine bank 0 (power-on)")
    print(f"mkbanktest: PRG {len(image)} bytes to {args.output} "
          f"({args.banks} x {args.unit} banks, {layout_note})")
    return 0


def build_chr(args: argparse.Namespace) -> int:
    if not 1 <= args.banks <= 16:
        raise SystemExit(f"mkbanktest: --banks must be 1..16 (got {args.banks})")
    image = bytearray()
    for bank in range(args.banks):
        data = bytearray(UNIT_8K)
        data[0] = (args.sig_lo + bank) & 0xFF
        data[UNIT_8K - 1] = (args.sig_hi + bank) & 0xFF
        image.extend(data)
    with open(args.output, "wb") as handle:
        handle.write(image)
    print(f"mkbanktest: CHR {len(image)} bytes to {args.output} "
          f"({args.banks} x 8 KiB banks)")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)

    prg = sub.add_parser("build-prg", help="compose a banked PRG image")
    prg.add_argument("--engine", required=True,
                     help="assembled engine binary (exactly --unit bytes)")
    prg.add_argument("--layout", choices=["fixed", "linear"], required=True)
    prg.add_argument("--banks", type=int, required=True)
    prg.add_argument("--unit", type=parse_value, required=True,
                     help="bank unit in bytes ($4000 or $8000)")
    prg.add_argument("--sig-lo", type=parse_value, default=0xB0)
    prg.add_argument("--sig-hi", type=parse_value, default=0xB4)
    prg.add_argument("--output", required=True)
    prg.set_defaults(func=build_prg)

    chr_p = sub.add_parser("build-chr", help="compose a banked CHR image")
    chr_p.add_argument("--banks", type=int, required=True)
    chr_p.add_argument("--sig-lo", type=parse_value, default=0xC0)
    chr_p.add_argument("--sig-hi", type=parse_value, default=0xC4)
    chr_p.add_argument("--output", required=True)
    chr_p.set_defaults(func=build_chr)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
