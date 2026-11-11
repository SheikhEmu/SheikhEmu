#!/bin/sh
# check_golden.sh - determinism gate on the self-made hello cartridge.
#
# Runs the release binary for 600 frames and compares the state hash
# with tests/golden/hello-600.txt. The ROM is 100% self-made
# (tests/roms/src/hello.asm), so this gate is safe to enforce in CI.
# Regenerating the hash on purpose after an engine change is a decision
# that must be recorded in DECISIONS.md first.

set -u
cd "$(dirname "$0")/.."

echo "== golden gate =="

cargo build --release -p se-cli >/dev/null 2>&1 || {
    echo "FAIL: release build failed"
    exit 1
}

test -f tests/roms/hello.nes || make test-roms >/dev/null 2>&1 || {
    echo "FAIL: test ROMs not built"
    exit 1
}

expected=$(sed -n 's/^hash=\([0-9a-f]*\)$/\1/p' tests/golden/hello-600.txt)
if [ -z "$expected" ]; then
    echo "FAIL: tests/golden/hello-600.txt has no hash= line"
    exit 1
fi

actual=$(./target/release/se-cli run tests/roms/hello.nes --frames 600 --hash \
    | sed -n 's/^بصمة الحالة: \([0-9a-f]*\)$/\1/p')
if [ -z "$actual" ]; then
    echo "FAIL: run produced no state hash"
    exit 1
fi

if [ "$actual" != "$expected" ]; then
    echo "FAIL: hello 600-frame hash drift"
    echo "  expected: $expected"
    echo "  actual:   $actual"
    echo "  if the change is intentional, record the decision and regenerate"
    exit 1
fi

echo "PASS: hello 600-frame determinism hash $actual"
