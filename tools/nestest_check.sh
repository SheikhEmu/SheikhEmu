#!/bin/sh
# nestest_check.sh - M1.5 calibration (ب) against the reference trace.
#
# The nestest files live OUTSIDE this repository by design: nestest is a
# category-B test ROM (run it, never commit it) fetched from its
# documented source (christopherpow/nes-test-roms, other/nestest.nes and
# other/nestest.log). Look for them in $SHEIKHEMU_NESTEST_DIR or
# ~/SheikhEmuRoms. This is a session/owner verification tool, NOT a CI
# gate: the CI runners do not have the files.
#
# Gate: our trace must match the reference trace byte for byte over the
# whole 8991-line automated window, and the result bytes at $02/$03 must
# be 00/00 ("tests completed successfully" per the nestest docs).

set -u
cd "$(dirname "$0")/.."

DIR="${SHEIKHEMU_NESTEST_DIR:-$HOME/SheikhEmuRoms}"
NES="$DIR/nestest.nes"
LOG="$DIR/nestest.log"

for file in "$NES" "$LOG"; do
    if [ ! -f "$file" ]; then
        echo "nestest file missing: $file"
        echo "place nestest.nes and nestest.log from the documented source"
        echo "into $DIR (or point SHEIKHEMU_NESTEST_DIR at them)"
        exit 3
    fi
done

cargo build --release -p se-cli >/dev/null 2>&1 || {
    echo "FAIL: release build failed"
    exit 1
}

OUT=$(mktemp) || exit 1
trap 'rm -f "$OUT"' EXIT

if ! ./target/release/se-cli run "$NES" --instructions 8991 \
        --start-pc C000 --trace-cpu "$OUT" >/dev/null; then
    echo "FAIL: nestest run did not complete cleanly"
    exit 1
fi

if cmp -s "$OUT" "$LOG"; then
    lines=$(wc -l < "$LOG")
    echo "PASS: nestest trace byte-identical (${lines}/${lines} lines)"
    exit 0
fi

echo "FAIL: nestest trace diverges from the reference:"
diff "$LOG" "$OUT" | head -8
echo "divergent lines: $(diff "$LOG" "$OUT" | grep -c '^<')"
exit 1
