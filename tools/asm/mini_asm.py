#!/usr/bin/env python3
"""mini_asm - a two-pass 6502 assembler for SheikhEmu test cartridges.

Self-contained: Python 3 only, no dependencies, no external code.
Assembly is deliberately small (labels, .org/.db/.dw/.fill, the documented
instruction set) because it exists to build the project's own test ROMs,
not to replace a general-purpose assembler.

Syntax:
    label:              bind a label at the current address
    .org $C000          set the assembly address
    .db $01, $02, "AB"  emit bytes (numbers or ASCII strings)
    .dw nmi, reset, irq emit 16-bit little-endian words (labels allowed)
    .fill 128, $FF      emit `count` copies of `value` (default $00)
    lda #$10            immediate
    lda $10             zero page (values <= $FF stay on page zero)
    lda $1000           absolute
    lda $1000,x         absolute indexed / `lda $10,x` zero-page indexed
    lda ($10,x)         zero-page indexed indirect
    lda ($10),y         zero-page indirect indexed
    jmp ($2FFF)         indirect (the address itself, wrap bug is runtime)
    beq label           relative branch, -128..+127
    asl a               accumulator mode

Mnemonic and register names are case-insensitive; labels are
case-sensitive. Exit code is non-zero on any error, with the offending
line and reason printed to stderr.

Usage:
    mini_asm.py input.asm -o output.bin [--sym output.sym] [--selftest]
"""

from __future__ import annotations

import argparse
import re
import sys

# ---------------------------------------------------------------------------
# Opcode tables (documented instruction set only).
# mnemonic -> {mode: opcode}
# ---------------------------------------------------------------------------

TABLE: dict[str, dict[str, int]] = {
    "lda": {"imm": 0xA9, "zp": 0xA5, "zpx": 0xB5, "abs": 0xAD, "abx": 0xBD,
            "aby": 0xB9, "izx": 0xA1, "izy": 0xB1},
    "ldx": {"imm": 0xA2, "zp": 0xA6, "zpy": 0xB6, "abs": 0xAE, "aby": 0xBE},
    "ldy": {"imm": 0xA0, "zp": 0xA4, "zpx": 0xB4, "abs": 0xAC, "abx": 0xBC},
    "sta": {"zp": 0x85, "zpx": 0x95, "abs": 0x8D, "abx": 0x9D, "aby": 0x99,
            "izx": 0x81, "izy": 0x91},
    "stx": {"zp": 0x86, "zpy": 0x96, "abs": 0x8E},
    "sty": {"zp": 0x84, "zpx": 0x94, "abs": 0x8C},
    "tax": {"imp": 0xAA}, "tay": {"imp": 0xA8},
    "txa": {"imp": 0x8A}, "tya": {"imp": 0x98},
    "tsx": {"imp": 0xBA}, "txs": {"imp": 0x9A},
    "adc": {"imm": 0x69, "zp": 0x65, "zpx": 0x75, "abs": 0x6D, "abx": 0x7D,
            "aby": 0x79, "izx": 0x61, "izy": 0x71},
    "sbc": {"imm": 0xE9, "zp": 0xE5, "zpx": 0xF5, "abs": 0xED, "abx": 0xFD,
            "aby": 0xF9, "izx": 0xE1, "izy": 0xF1},
    "and": {"imm": 0x29, "zp": 0x25, "zpx": 0x35, "abs": 0x2D, "abx": 0x3D,
            "aby": 0x39, "izx": 0x21, "izy": 0x31},
    "ora": {"imm": 0x09, "zp": 0x05, "zpx": 0x15, "abs": 0x0D, "abx": 0x1D,
            "aby": 0x19, "izx": 0x01, "izy": 0x11},
    "eor": {"imm": 0x49, "zp": 0x45, "zpx": 0x55, "abs": 0x4D, "abx": 0x5D,
            "aby": 0x59, "izx": 0x41, "izy": 0x51},
    "cmp": {"imm": 0xC9, "zp": 0xC5, "zpx": 0xD5, "abs": 0xCD, "abx": 0xDD,
            "aby": 0xD9, "izx": 0xC1, "izy": 0xD1},
    "cpx": {"imm": 0xE0, "zp": 0xE4, "abs": 0xEC},
    "cpy": {"imm": 0xC0, "zp": 0xC4, "abs": 0xCC},
    "inc": {"zp": 0xE6, "zpx": 0xF6, "abs": 0xEE, "abx": 0xFE},
    "dec": {"zp": 0xC6, "zpx": 0xD6, "abs": 0xCE, "abx": 0xDE},
    "inx": {"imp": 0xE8}, "iny": {"imp": 0xC8},
    "dex": {"imp": 0xCA}, "dey": {"imp": 0x88},
    "asl": {"acc": 0x0A, "zp": 0x06, "zpx": 0x16, "abs": 0x0E, "abx": 0x1E},
    "lsr": {"acc": 0x4A, "zp": 0x46, "zpx": 0x56, "abs": 0x4E, "abx": 0x5E},
    "rol": {"acc": 0x2A, "zp": 0x26, "zpx": 0x36, "abs": 0x2E, "abx": 0x3E},
    "ror": {"acc": 0x6A, "zp": 0x66, "zpx": 0x76, "abs": 0x6E, "abx": 0x7E},
    "bit": {"zp": 0x24, "abs": 0x2C},
    "jmp": {"abs": 0x4C, "ind": 0x6C},
    "jsr": {"abs": 0x20},
    "rts": {"imp": 0x60}, "rti": {"imp": 0x40}, "brk": {"imp": 0x00},
    "bcc": {"rel": 0x90}, "bcs": {"rel": 0xB0}, "beq": {"rel": 0xF0},
    "bmi": {"rel": 0x30}, "bne": {"rel": 0xD0}, "bpl": {"rel": 0x10},
    "bvc": {"rel": 0x50}, "bvs": {"rel": 0x70},
    "pha": {"imp": 0x48}, "php": {"imp": 0x08},
    "pla": {"imp": 0x68}, "plp": {"imp": 0x28},
    "clc": {"imp": 0x18}, "cld": {"imp": 0xD8}, "cli": {"imp": 0x58},
    "clv": {"imp": 0xB8}, "sec": {"imp": 0x38}, "sed": {"imp": 0xF8},
    "sei": {"imp": 0x78},
    "nop": {"imp": 0xEA},
}

# Number of operand bytes per addressing mode.
OPERAND_SIZE = {
    "imp": 0, "acc": 0, "imm": 1, "zp": 1, "zpx": 1, "zpy": 1,
    "izx": 1, "izy": 1, "rel": 1, "abs": 2, "abx": 2, "aby": 2, "ind": 2,
}

LABEL_RE = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)\s*:(.*)$")
NUMBER_RE = re.compile(r"^\$([0-9A-Fa-f]+)$|^([0-9]+)$")


class AsmError(Exception):
    """Assembly error with source line context."""


class Line:
    """One meaningful source line."""

    __slots__ = ("number", "text", "labels", "mnemonic", "operand")

    def __init__(self, number: int, text: str) -> None:
        self.number = number
        self.text = text
        self.labels: list[str] = []
        self.mnemonic: str | None = None
        self.operand: str | None = None


def parse_number(token: str) -> int | None:
    """Parse `$HH`, decimal, or a negative decimal; None if not numeric."""
    token = token.strip()
    if not token:
        return None
    if token.startswith("$"):
        try:
            return int(token[1:], 16)
        except ValueError:
            return None
    match = NUMBER_RE.match(token)
    if match:
        return int(token)
    if token.startswith("-") and token[1:].isdigit():
        return int(token)
    return None


def split_db_values(text: str) -> list[str]:
    """Split a .db/.dw operand list on commas outside quotes."""
    values: list[str] = []
    current = []
    quote = False
    for ch in text:
        if ch == '"':
            quote = not quote
            current.append(ch)
        elif ch == "," and not quote:
            values.append("".join(current).strip())
            current = []
        else:
            current.append(ch)
    tail = "".join(current).strip()
    if tail:
        values.append(tail)
    return values


def string_bytes(token: str) -> bytes:
    """Interpret a double-quoted ASCII string as bytes."""
    if len(token) < 2 or token[0] != '"' or token[-1] != '"':
        raise AsmError(f"bad string literal: {token}")
    inner = token[1:-1]
    return inner.encode("ascii")


def value_bytes(token: str) -> int:
    value = parse_number(token)
    if value is None:
        raise AsmError(f"expected a number, got: {token}")
    if value < -128 or value > 0xFF:
        raise AsmError(f"value out of byte range: {token}")
    return value & 0xFF


def strip_mode_syntax(operand: str) -> tuple[str, str | None, str | None]:
    """Classify operand syntax; returns (value_text, index, kind).

    kind is one of: imp, acc, imm, izx, izy, ind, indexed, plain.
    """
    text = operand.strip()
    if not text:
        return "", None, "imp"
    if text.lower() == "a":
        return "", None, "acc"
    if text.startswith("#"):
        return text[1:].strip(), None, "imm"

    # Parenthesised forms first: strip the parentheses before looking at
    # suffixes such as ",y".
    if text.startswith("(") and text.endswith(")"):
        inner = text[1:-1].strip()
        # ($FF,x) - indexed indirect: comma inside the parentheses.
        inner_match = re.match(r"^(.*?)\s*,\s*([xXyY])$", inner)
        if inner_match:
            value, index = inner_match.group(1).strip(), inner_match.group(2).lower()
            if index != "x":
                raise AsmError(f"only ,x is valid inside parentheses: {operand}")
            return value, None, "izx"
        return inner, None, "ind"

    # ($10),y - indirect indexed: comma after the closing parenthesis.
    paren_match = re.match(r"^(\(.*?\))\s*,\s*([xXyY])$", text)
    if paren_match:
        value = paren_match.group(1).strip()
        value = value[1:-1].strip()  # strip parentheses again
        index = paren_match.group(2).lower()
        if index != "y":
            raise AsmError(f"only ,y is valid after parentheses: {operand}")
        return value, None, "izy"

    # value,x / value,y
    plain_match = re.match(r"^(.*?)\s*,\s*([xXyY])$", text)
    if plain_match:
        value = plain_match.group(1).strip()
        index = plain_match.group(2).lower()
        return value, index, "indexed"

    return text, None, "plain"


def choose_mode(mnemonic: str, kind: str, value_text: str,
               index: str | None, numeric: int | None) -> str:
    """Pick the addressing mode for one instruction operand."""
    entry = TABLE[mnemonic]
    if kind == "plain":
        # Branch mnemonics have exactly one mode: rel.
        if "rel" in entry and len(entry) == 1:
            return "rel"
    if kind == "indexed":
        base_is_zp = numeric is not None and numeric <= 0xFF
        if index == "x":
            if base_is_zp and "zpx" in entry:
                return "zpx"
            if "abx" in entry:
                return "abx"
            raise AsmError(f"{mnemonic} does not support ,x indexing")
        if index == "y":
            if base_is_zp and "zpy" in entry:
                return "zpy"
            if "aby" in entry:
                return "aby"
            raise AsmError(f"{mnemonic} does not support zero-page ,y")
    if kind == "plain":
        if numeric is not None and numeric <= 0xFF and "zp" in entry:
            return "zp"
        if numeric is not None and numeric > 0xFFFF:
            raise AsmError(f"address out of 16-bit range: {value_text}")
        if "abs" in entry:
            return "abs"
        if numeric is not None and numeric <= 0xFF and "zp" in entry:
            return "zp"
        raise AsmError(f"{mnemonic} does not support absolute addressing")
    if kind == "izx" and "izx" not in entry:
        raise AsmError(f"{mnemonic} does not support (zp,x)")
    if kind == "izy" and "izy" not in entry:
        raise AsmError(f"{mnemonic} does not support (zp),y")
    if kind == "ind" and "ind" not in entry:
        raise AsmError(f"{mnemonic} does not support indirect addressing")
    if kind == "imm" and "imm" not in entry:
        raise AsmError(f"{mnemonic} does not support immediate values")
    mode = {"imp": "imp", "acc": "acc", "imm": "imm", "izx": "izx",
            "izy": "izy", "ind": "ind"}[kind]
    if mode not in entry:
        raise AsmError(f"{mnemonic} does not support {mode} mode")
    return mode


def resolve_value(value_text: str, labels: dict[str, int]) -> int:
    """Resolve an operand value: number or label."""
    numeric = parse_number(value_text)
    if numeric is not None:
        return numeric
    if value_text in labels:
        return labels[value_text]
    raise AsmError(f"undefined symbol: {value_text}")


class SourceFile:
    """Parsed source: statements with sizes, then emitted bytes."""

    def __init__(self) -> None:
        self.statements: list[dict] = []
        self.labels: dict[str, int] = {}
        self.base: int | None = None
        self.end: int = 0

    def assemble(self, text: str) -> None:
        lines = self._parse(text)
        self._pass1(lines)
        self._pass2(lines)

    # -- parsing ----------------------------------------------------------

    def _parse(self, text: str) -> list[Line]:
        out: list[Line] = []
        for number, raw in enumerate(text.splitlines(), start=1):
            stripped = raw.split(";", 1)[0].strip()
            if not stripped:
                continue
            line = Line(number, stripped)
            rest = stripped
            while True:
                match = LABEL_RE.match(rest)
                if not match:
                    break
                line.labels.append(match.group(1))
                rest = match.group(2).strip()
            if rest:
                if rest.startswith("."):
                    parts = rest.split(None, 1)
                    line.mnemonic = parts[0].lower()
                    line.operand = parts[1].strip() if len(parts) > 1 else ""
                else:
                    parts = rest.split(None, 1)
                    line.mnemonic = parts[0].lower()
                    if len(parts) > 1:
                        line.operand = parts[1].strip()
                    else:
                        line.operand = ""
            if line.labels or line.mnemonic:
                out.append(line)
        return out

    # -- pass 1: sizes and labels -----------------------------------------

    def _pass1(self, lines: list[Line]) -> None:
        address = 0
        for line in lines:
            # Bind labels BEFORE advancing the address.
            for label in line.labels:
                if label in self.labels:
                    raise AsmError(
                        f"line {line.number}: duplicate label: {label}")
                self.labels[label] = address
            if line.mnemonic is None:
                continue
            if line.mnemonic == ".org":
                address = self._require_number(line)
                if self.base is None:
                    self.base = address
                continue
            if line.mnemonic == ".db":
                address += sum(len(string_bytes(tok)) if tok.startswith('"')
                               else 1 for tok in split_db_values(line.operand))
                continue
            if line.mnemonic == ".dw":
                address += 2 * len(split_db_values(line.operand))
                continue
            if line.mnemonic == ".fill":
                parts = split_db_values(line.operand)
                count = self._require_number(line, parts[0] if parts else None)
                address += count
                continue
            if line.mnemonic in TABLE:
                size = 1 + OPERAND_SIZE[
                    self._classify(line, allow_forward_labels=True)]
                address += size
                continue
            raise AsmError(
                f"line {line.number}: unknown mnemonic: {line.mnemonic}")
        if self.base is None:
            raise AsmError("source has no .org directive; base address unknown")
        self.end = address
        if self.end <= self.base:
            raise AsmError("source assembled to an empty image")

    def _require_number(self, line: Line, token: str | None = None) -> int:
        text = token if token is not None else (line.operand or "")
        value = parse_number(text.strip())
        if value is None:
            raise AsmError(f"line {line.number}: expected a number: {text}")
        return value

    def _classify(self, line: Line, allow_forward_labels: bool) -> str:
        """Determine the addressing mode of one instruction line."""
        mnemonic = line.mnemonic or ""
        operand = line.operand or ""
        value_text, index, kind = strip_mode_syntax(operand)
        numeric = parse_number(value_text)
        if kind in ("plain", "indexed") and numeric is None \
                and not allow_forward_labels:
            # Label operand: resolved in pass 2, but mode choice for
            # labels is absolute/relative; re-classify here.
            pass
        mode = choose_mode(mnemonic, kind, value_text, index, numeric)
        if mode == "rel":
            if numeric is None and not allow_forward_labels and \
                    value_text not in self.labels:
                raise AsmError(
                    f"line {line.number}: undefined symbol: {value_text}")
            if "rel" not in TABLE[mnemonic]:
                raise AsmError(
                    f"line {line.number}: {mnemonic} is not a branch")
        if mode in ("imp", "acc") and operand.strip() and kind not in ("imp", "acc"):
            raise AsmError(
                f"line {line.number}: {mnemonic} takes no operand")
        return mode

    # -- pass 2: emit ------------------------------------------------------

    def _pass2(self, lines: list[Line]) -> None:
        image = bytearray(self.end - self.base)
        address = self.base
        for line in lines:
            if line.mnemonic is None:
                continue
            if line.mnemonic == ".org":
                address = self._require_number(line)
                continue
            if line.mnemonic == ".db":
                for token in split_db_values(line.operand or ""):
                    if token.startswith('"'):
                        data = string_bytes(token)
                    else:
                        data = bytes([value_bytes(token)])
                    self._emit(image, address, data)
                    address += len(data)
                continue
            if line.mnemonic == ".dw":
                for token in split_db_values(line.operand or ""):
                    value = resolve_value(token, self.labels)
                    if value < 0 or value > 0xFFFF:
                        raise AsmError(
                            f"line {line.number}: word out of range: {token}")
                    self._emit(image, address,
                               bytes([value & 0xFF, (value >> 8) & 0xFF]))
                    address += 2
                continue
            if line.mnemonic == ".fill":
                parts = split_db_values(line.operand or "")
                count = self._require_number(line, parts[0] if parts else None)
                fill_value = 0x00
                if len(parts) > 1:
                    fill_value = self._require_number(line, parts[1])
                self._emit(image, address, bytes([fill_value]) * count)
                address += count
                continue
            # Instruction.
            value_text, index, kind = strip_mode_syntax(line.operand or "")
            numeric = parse_number(value_text)
            mode = choose_mode(line.mnemonic or "", kind, value_text,
                               index, numeric)
            opcode = TABLE[line.mnemonic or ""][mode]
            size = OPERAND_SIZE[mode]
            self._emit(image, address, bytes([opcode]))
            address += 1
            if size == 0:
                continue
            value = resolve_value(value_text, self.labels)
            if mode == "rel":
                offset = value - (address + 1)
                if offset < -128 or offset > 127:
                    raise AsmError(
                        f"line {line.number}: branch out of range "
                        f"({offset}) to {value_text}")
                self._emit(image, address, bytes([offset & 0xFF]))
            elif size == 1:
                if mode in ("zp", "zpx", "zpy", "izx", "izy"):
                    if value > 0xFF:
                        raise AsmError(
                            f"line {line.number}: zero-page address out of "
                            f"range: {value_text}")
                if mode == "imm" and value > 0xFF:
                    raise AsmError(
                        f"line {line.number}: immediate out of range: "
                        f"{value_text}")
                self._emit(image, address, bytes([value & 0xFF]))
            else:
                if value > 0xFFFF:
                    raise AsmError(
                        f"line {line.number}: address out of range: "
                        f"{value_text}")
                self._emit(image, address,
                           bytes([value & 0xFF, (value >> 8) & 0xFF]))
            address += size
        self.image = image

    def _emit(self, image: bytearray, address: int, data: bytes) -> None:
        offset = address - self.base
        if offset < 0 or offset + len(data) > len(image):
            raise AsmError(
                f"emit outside image bounds at ${address:04X} "
                f"(missing .org before the code?)")
        image[offset:offset + len(data)] = data


def assemble_text(text: str) -> tuple[int, bytes, dict[str, int]]:
    """Assemble `text`; returns (base address, image bytes, labels)."""
    source = SourceFile()
    source.assemble(text)
    return source.base or 0, bytes(source.image), source.labels


# ---------------------------------------------------------------------------
# Self tests
# ---------------------------------------------------------------------------

def _expect_eq(actual: object, expected: object, what: str) -> None:
    if actual != expected:
        raise AssertionError(f"{what}: got {actual!r}, expected {expected!r}")


def selftest() -> None:
    checks = [
        # (source, base, expected image bytes)
        ("lda #$10", 0xC000, b"\xa9\x10"),
        ("lda $10", 0xC000, b"\xa5\x10"),
        ("lda $1000", 0xC000, b"\xad\x00\x10"),
        ("lda $1000,x", 0xC000, b"\xbd\x00\x10"),
        ("sta $1000,y", 0xC000, b"\x99\x00\x10"),
        ("lda ($10),y", 0xC000, b"\xb1\x10"),
        ("lda ($10,x)", 0xC000, b"\xa1\x10"),
        ("ldx $10,y", 0xC000, b"\xb6\x10"),
        ("asl a", 0xC000, b"\x0a"),
        ("jmp ($2FFF)", 0xC000, b"\x6c\xff\x2f"),
        ("inc $10,x", 0xC000, b"\xf6\x10"),
        (".org $C000\nstart: nop\n jmp start", 0xC000, b"\xea\x4c\x00\xc0"),
        (".org $C000\n jsr $1234", 0xC000, b"\x20\x34\x12"),
        (".db 1, 2, \"AB\"", 0xC000, b"\x01\x02\x41\x42"),
        (".dw $1234, $BE", 0xC000, b"\x34\x12\xbe\x00"),
        (".fill 3, $ff", 0xC000, b"\xff\xff\xff"),
        ("asl $10", 0xC000, b"\x06\x10"),
        ("bne $C005", 0xC000, b"\xd0\x03"),
        ("bne *+3", 0xC000, None),  # '*' unsupported -> error expected
    ]
    for source, base, expected in checks:
        try:
            _, image, _ = assemble_text(".org ${:04X}\n{}".format(base, source)
                                        if ".org" not in source else source)
        except AsmError:
            if expected is None:
                continue
            raise AssertionError(f"unexpected error for: {source}")
        if expected is None:
            raise AssertionError(f"expected an error for: {source}")
        _expect_eq(image, expected, f"assembling {source!r}")

    # Branch offset with labels, backward: -4 from the branch operand.
    _, image, _ = assemble_text(
        ".org $C000\nloop: inc $10\nbne loop\nbeq loop\n")
    _expect_eq(image, b"\xe6\x10\xd0\xfc\xf0\xfa",
               "branch offsets with labels")

    # .dw with labels resolves to addresses, little endian (2 words = 4 bytes).
    _, image, _ = assemble_text(
        ".org $C000\nnmi: rti\nirq: rti\n.org $FFFA\n.dw nmi, irq")
    _expect_eq(image[-4:], b"\x00\xc0\x01\xc0", ".dw labels")

    # Total image covers the gap with zeros and reaches $FFFF.
    base, image, _ = assemble_text(
        ".org $C000\nnop\n.org $FFFA\n.fill 6")
    _expect_eq(base, 0xC000, "base address")
    _expect_eq(len(image), 0x10000 - 0xC000, "image spans to $FFFF")

    # Labels bind before .db advances the address.
    _, _, labels = assemble_text(".org $C000\nhere: .db 1\nafter: nop")
    _expect_eq(labels["here"], 0xC000, "label bound before .db advance")
    _expect_eq(labels["after"], 0xC001, "label after .db")

    # Duplicate labels are rejected.
    try:
        assemble_text(".org $C000\ndup: nop\ndup: nop")
    except AsmError:
        pass
    else:
        raise AssertionError("duplicate label not detected")

    # Unknown mnemonic is rejected.
    try:
        assemble_text(".org $C000\nxyz #$10")
    except AsmError:
        pass
    else:
        raise AssertionError("unknown mnemonic not detected")

    print("mini_asm selftest: all checks passed")


# ---------------------------------------------------------------------------
# Command line
# ---------------------------------------------------------------------------

def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("input", nargs="?", help="assembly source file")
    parser.add_argument("-o", "--output", help="output .bin file")
    parser.add_argument("--sym", help="optional symbol file to write")
    parser.add_argument("--selftest", action="store_true",
                        help="run built-in self tests")
    args = parser.parse_args(argv)

    if args.selftest:
        selftest()
        return 0
    if not args.input or not args.output:
        parser.error("input and -o output are required (or --selftest)")

    with open(args.input, "r", encoding="utf-8") as handle:
        text = handle.read()
    try:
        base, image, labels = assemble_text(text)
    except AsmError as err:
        print(f"mini_asm: error: {err}", file=sys.stderr)
        return 1

    with open(args.output, "wb") as handle:
        handle.write(image)

    if args.sym:
        with open(args.sym, "w", encoding="utf-8") as handle:
            for name in sorted(labels):
                handle.write(f"{name:16s} = ${labels[name]:04X}\n")

    print(f"mini_asm: wrote {len(image)} bytes "
          f"(base ${base:04X}) to {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
