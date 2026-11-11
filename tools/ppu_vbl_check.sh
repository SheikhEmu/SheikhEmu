#!/bin/sh
# ppu_vbl_check.sh - M2 exit-gate calibration (ب) for the PPU.
#
# The ppu_vbl_nmi files live OUTSIDE this repository by design: it is a
# category-B test ROM (run it, never commit it) fetched from its
# documented source (christopherpow/nes-test-roms, ppu_vbl_nmi/). Look
# for the ROM in $SHEIKHEMU_PPUVBL_DIR or ~/SheikhEmuRoms. This is a
# session/owner verification tool, NOT a CI gate: the CI runners do not
# have the file.
#
# The ROM is an MMC1 board (128 KiB PRG, CHR RAM): it runs through the
# minimal mapper-1 banking of decision D-22 with the $6000 WRAM the
# blargg protocol requires.
#
# Gate (spec §15, M2 row): the harness reports the $6000 status $00
# ("Passed") and prints "All 10 tests passed", with the even/odd
# timing measurement matching the documented "08 08 09 07".

set -u
cd "$(dirname "$0")/.."

DIR="${SHEIKHEMU_PPUVBL_DIR:-$HOME/SheikhEmuRoms}"
NES="$DIR/ppu_vbl_nmi.nes"

if [ ! -f "$NES" ]; then
    echo "ppu_vbl_nmi.nes missing: $NES"
    echo "place it from the documented source into $DIR"
    echo "(or point SHEIKHEMU_PPUVBL_DIR at it)"
    exit 3
fi

cargo build --release -p se-cli >/dev/null 2>&1 || {
    echo "FAIL: release build failed"
    exit 1
}

# 2000 frames: the harness finishes its 10 sub-tests well within that
# window (a few seconds headless).
REPORT=$(mktemp) || exit 1
trap 'rm -f "$REPORT"' EXIT

./target/release/se-cli run "$NES" --frames 2000 > "$REPORT" || {
    echo "FAIL: ppu_vbl_nmi run did not complete cleanly"
    cat "$REPORT"
    exit 1
}

# The status line must read "success".
if ! grep -q "حالة الاختبار: نجاح كامل (00)" "$REPORT"; then
    echo "FAIL: ppu_vbl_nmi did not report success:"
    grep -E "حالة الاختبار|نص النتيجة" "$REPORT"
    exit 1
fi

if ! grep -q "All 10 tests passed" "$REPORT"; then
    echo "FAIL: ppu_vbl_nmi harness did not pass all tests:"
    grep -E "نص النتيجة" "$REPORT"
    exit 1
fi

# The even/odd timing measurement must match the documented values.
if ! grep -q "08 08 09 07" "$REPORT"; then
    echo "FAIL: even/odd timing measurement drifted from 08 08 09 07:"
    grep -E "نص النتيجة" "$REPORT"
    exit 1
fi

echo "PASS: ppu_vbl_nmi all 10 tests (status 00, timing 08 08 09 07)"
exit 0
