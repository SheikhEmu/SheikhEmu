#!/bin/sh
# check_apu_demo.sh - the M4.4 APU demo gate (state + PCM goldens).
#
# Runs the release binary over tests/roms/apu_demo.nes (100% self-made
# from tests/roms/src/apu_demo.asm) for 1040 frames - the eight
# 128-frame phases (silence, pulse 1 A4, pulse 2 A5, triangle A3,
# short-mode noise, looped DMC, the five-channel ensemble, silence)
# plus the first 16 frames of the replayed silence - and compares BOTH
# the state hash and the PCM fingerprint against tests/golden/
# apu-demo.txt. The same two values are asserted by crates/se-core/
# tests/apu_demo_golden.rs inside `cargo test --workspace` on all three
# CI platforms, and that test cross-checks its embedded image against
# this assembled .nes whenever the file is present, so the .asm source
# and the embedded copy cannot drift apart silently.
#
# Regenerating the values on purpose after an engine change is a
# decision that must be recorded in DECISIONS.md first (the golden
# file header carries the history).

set -u
cd "$(dirname "$0")/.."

echo "== apu demo gate (M4.4) =="

cargo build --release -p se-cli >/dev/null 2>&1 || {
    echo "FAIL: release build failed"
    exit 1
}

test -f tests/roms/apu_demo.nes || make test-roms >/dev/null 2>&1 || {
    echo "FAIL: test ROMs not built"
    exit 1
}

expected_hash=$(sed -n 's/^hash=\([0-9a-f]*\)$/\1/p' tests/golden/apu-demo.txt)
expected_pcm=$(sed -n 's/^pcm=\([0-9a-f]*\)$/\1/p' tests/golden/apu-demo.txt)
if [ -z "$expected_hash" ] || [ -z "$expected_pcm" ]; then
    echo "FAIL: tests/golden/apu-demo.txt has no hash=/pcm= lines"
    exit 1
fi

report=$(./target/release/se-cli run tests/roms/apu_demo.nes --frames 1040 --hash)
actual_hash=$(printf '%s\n' "$report" | sed -n 's/^بصمة الحالة: \([0-9a-f]*\)$/\1/p')
actual_pcm=$(printf '%s\n' "$report" | sed -n 's/^بصمة PCM: \([0-9a-f]*\)$/\1/p')
if [ -z "$actual_hash" ] || [ -z "$actual_pcm" ]; then
    echo "FAIL: run produced no fingerprints"
    exit 1
fi

status=0

if [ "$actual_hash" != "$expected_hash" ]; then
    echo "FAIL: apu_demo 1040-frame state hash drift"
    echo "  expected: $expected_hash"
    echo "  actual:   $actual_hash"
    echo "  if the change is intentional, record the decision and regenerate"
    status=1
fi

if [ "$actual_pcm" != "$expected_pcm" ]; then
    echo "FAIL: apu_demo 1040-frame PCM fingerprint drift"
    echo "  expected: $expected_pcm"
    echo "  actual:   $actual_pcm"
    echo "  if the change is intentional, record the decision and regenerate"
    status=1
fi

if [ "$status" -eq 0 ]; then
    echo "PASS: apu_demo 1040-frame state $actual_hash + PCM $actual_pcm"
fi
exit "$status"
