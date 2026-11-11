#!/usr/bin/env python3
"""mkrom - build SheikhEmu test cartridge images (.nes).

Self-contained: Python 3 only. Produces iNES or NES 2.0 files with
clean headers (bytes 12-15 always zero for old-style headers, so the
images never look like DiskDude-era files). Exponent-multiplier sizes
are not emitted; test ROMs are always aligned to 16/8 KiB units.

Usage:
    mkrom.py make --prg FILE [--chr FILE | --chr-size N]
        [--mapper N] [--submapper N] [--nes2]
        [--mirroring v|h|4] [--battery]
        [--timing ntsc|pal|dendy|multiple]
        [--prg-ram-kib N] [--prg-nvram-kib N]
        [--chr-ram-kib N] [--chr-nvram-kib N]
        --output FILE
"""

from __future__ import annotations

import argparse
import sys

PRG_UNIT = 16 * 1024
CHR_UNIT = 8 * 1024
MAX_PRG_UNITS = 0xEFF
MAX_CHR_UNITS = 0xEFF

TIMING_BITS = {"ntsc": 0, "pal": 1, "multiple": 2, "dendy": 3}


def size_to_shift(kib: float, field: str) -> int:
    """Convert a KiB size into a NES 2.0 shift count (64 << n)."""
    size_bytes = int(round(kib * 1024))
    if size_bytes == 0:
        return 0
    if size_bytes < 64 or size_bytes & (size_bytes - 1) != 0:
        raise SystemExit(
            f"mkrom: {field} must be a power-of-two multiple of 64 bytes "
            f"(got {size_bytes} bytes)")
    shift = size_bytes.bit_length() - 1 - 6
    if shift < 1 or shift > 15:
        raise SystemExit(
            f"mkrom: {field} shift count out of range 1..15 (got {shift})")
    return shift


def build_header(args: argparse.Namespace, prg_len: int,
                 chr_len: int) -> bytes:
    prg_units = prg_len // PRG_UNIT
    chr_units = chr_len // CHR_UNIT
    if prg_units > MAX_PRG_UNITS or chr_units > MAX_CHR_UNITS:
        raise SystemExit(
            "mkrom: size exceeds the linear NES 2.0 notation; "
            "exponent-multiplier files are not produced by this tool")

    mapper = args.mapper
    if not 0 <= mapper <= 0xFFF:
        raise SystemExit(f"mkrom: mapper must be 0..4095 (got {mapper})")
    if not args.nes2 and mapper > 0xFF:
        raise SystemExit("mkrom: mappers above 255 require --nes2")

    flags6 = (mapper & 0x0F) << 4
    if args.mirroring == "h":
        flags6 |= 0x01
    elif args.mirroring == "4":
        flags6 |= 0x08
    if args.battery:
        flags6 |= 0x02
    if args.trainer:
        flags6 |= 0x04

    if args.nes2:
        flags7 = (mapper >> 4) & 0x0F
        flags7 = (flags7 << 4) | 0x08
        byte8 = (mapper >> 8) & 0x0F
        byte8 = (byte8 << 4) | (args.submapper & 0x0F)
    else:
        flags7 = (mapper >> 4) & 0x0F
        flags7 <<= 4
        byte8 = 0

    header = bytearray(16)
    header[0:4] = b"NES\x1A"
    header[4] = prg_units & 0xFF
    header[5] = chr_units & 0xFF
    header[6] = flags6
    header[7] = flags7
    header[8] = byte8
    if args.nes2:
        header[9] = (chr_units >> 8) << 4 | (prg_units >> 8)
        header[10] = size_to_shift(args.prg_ram_kib, "prg-ram") \
            | size_to_shift(args.prg_nvram_kib, "prg-nvram") << 4
        header[11] = size_to_shift(args.chr_ram_kib, "chr-ram") \
            | size_to_shift(args.chr_nvram_kib, "chr-nvram") << 4
        header[12] = TIMING_BITS[args.timing]
        # bytes 13-15 stay zero (regular console, no misc ROMs, no
        # default expansion device).
    else:
        # Old-style header: bytes 8-15 stay zero (clean header).
        pass
    return bytes(header)


def make(args: argparse.Namespace) -> int:
    with open(args.prg, "rb") as handle:
        prg = handle.read()
    if len(prg) % PRG_UNIT != 0 or len(prg) == 0:
        raise SystemExit(
            f"mkrom: PRG size must be a non-zero multiple of 16 KiB "
            f"(got {len(prg)} bytes)")
    if len(prg) // PRG_UNIT > MAX_PRG_UNITS:
        raise SystemExit("mkrom: PRG size exceeds the linear notation")

    chr_data = b""
    if args.chr:
        with open(args.chr, "rb") as handle:
            chr_data = handle.read()
        if len(chr_data) % CHR_UNIT != 0:
            raise SystemExit(
                f"mkrom: CHR size must be a multiple of 8 KiB "
                f"(got {len(chr_data)} bytes)")
    elif args.chr_size:
        if args.chr_size % CHR_UNIT != 0:
            raise SystemExit(
                f"mkrom: --chr-size must be a multiple of 8 KiB")
        chr_data = bytes(args.chr_size)

    trainer = b""
    if args.trainer:
        with open(args.trainer, "rb") as handle:
            trainer = handle.read()
        if len(trainer) != 512:
            raise SystemExit(
                f"mkrom: trainer must be exactly 512 bytes "
                f"(got {len(trainer)})")

    header = build_header(args, len(prg), len(chr_data))
    image = header + trainer + prg + chr_data
    with open(args.output, "wb") as handle:
        handle.write(image)
    print(f"mkrom: wrote {len(image)} bytes to {args.output} "
          f"(PRG {len(prg)}, CHR {len(chr_data)}, "
          f"{'NES 2.0' if args.nes2 else 'iNES'}, mapper {args.mapper})")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)

    mk = sub.add_parser("make", help="build a .nes image")
    mk.add_argument("--prg", required=True, help="PRG-ROM binary file")
    mk.add_argument("--chr", help="CHR-ROM binary file (mutually exclusive "
                                  "with --chr-size)")
    mk.add_argument("--chr-size", type=int, default=0,
                    help="emit this many bytes of zero CHR-ROM")
    mk.add_argument("--mapper", type=lambda s: int(s, 0), default=0)
    mk.add_argument("--submapper", type=lambda s: int(s, 0), default=0)
    mk.add_argument("--nes2", action="store_true",
                    help="write a NES 2.0 header instead of iNES")
    mk.add_argument("--mirroring", choices=["v", "h", "4"], default="v")
    mk.add_argument("--battery", action="store_true")
    mk.add_argument("--trainer", help="512-byte trainer file")
    mk.add_argument("--timing", choices=list(TIMING_BITS), default="ntsc",
                    help="NES 2.0 byte 12 timing field")
    mk.add_argument("--prg-ram-kib", type=float, default=0)
    mk.add_argument("--prg-nvram-kib", type=float, default=0)
    mk.add_argument("--chr-ram-kib", type=float, default=0)
    mk.add_argument("--chr-nvram-kib", type=float, default=0)
    mk.add_argument("--output", required=True)
    mk.set_defaults(func=make)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
