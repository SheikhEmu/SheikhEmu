#!/bin/sh
# check_pattern.sh - M2 frame-golden gate on the self-made pattern ROM.
#
# Runs the release binary over tests/roms/pattern.nes (100% self-made,
# category A: tests/roms/src/pattern.asm) and compares the palette-index
# plane of the last completed frame with tests/golden/pattern-frame.bin
# byte for byte — the approved M2 reading of "a 240p frame matching
# visually": byte-identical to our deposited reference plus the owner's
# visual confirmation of the PNG in the session report. The scene
# exercises the background pipeline, fine scroll, sprites through OAM
# DMA, the sprite-0 hit and a line-granular mid-frame scroll split, so
# a rendering regression anywhere in that chain drifts the plane.
#
# Regenerating the reference on purpose after engine changes is a
# decision that must be recorded in DECISIONS.md first.

set -u
cd "$(dirname "$0")/.."

echo "== pattern frame gate =="

cargo build --release -p se-cli >/dev/null 2>&1 || {
    echo "FAIL: release build failed"
    exit 1
}

test -f tests/roms/pattern.nes || make test-roms >/dev/null 2>&1 || {
    echo "FAIL: test ROMs not built"
    exit 1
}

OUT1=$(mktemp) || exit 1
OUT2=$(mktemp) || exit 1
trap 'rm -f "$OUT1" "$OUT2"' EXIT

./target/release/se-cli run tests/roms/pattern.nes --frames 100 \
    --dump-plane "$OUT1" >/dev/null || {
    echo "FAIL: pattern run did not complete cleanly"
    exit 1
}
./target/release/se-cli run tests/roms/pattern.nes --frames 100 \
    --dump-plane "$OUT2" >/dev/null || {
    echo "FAIL: second pattern run did not complete cleanly"
    exit 1
}

if ! cmp -s "$OUT1" "$OUT2"; then
    echo "FAIL: pattern render is not deterministic"
    exit 1
fi

if cmp -s "$OUT1" tests/golden/pattern-frame.bin; then
    echo "PASS: pattern frame byte-identical to the deposited reference"
    exit 0
fi

echo "FAIL: pattern frame drifted from tests/golden/pattern-frame.bin"
echo "  bytes differing: $(cmp -l "$OUT1" tests/golden/pattern-frame.bin | wc -l)"
echo "  if the change is intentional, record the decision and regenerate"
exit 1
