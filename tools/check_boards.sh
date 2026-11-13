#!/bin/sh
# check_boards.sh - T1 board verification gate (M3).
#
# Runs every self-made board-verification ROM (one per T1 family,
# §6.3: no test ROM existed for these boards, so we wrote our own)
# through the headless runner and fails unless the blargg-style
# protocol reports full success (status $00) for each.
#
# Exit status: 0 = all boards pass, 1 = any board fails.

set -u
cd "$(dirname "$0")/.."

status=0

echo "== boards gate =="

if [ ! -x target/release/se-cli ]; then
  cargo build --release -p se-cli || exit 1
fi

for rom in \
  tests/roms/banktest_uxrom.nes \
  tests/roms/banktest_camerica.nes \
  tests/roms/banktest_axrom.nes \
  tests/roms/banktest_bnrom.nes \
  tests/roms/banktest_colordreams.nes \
  tests/roms/banktest_gxrom.nes \
  tests/roms/banktest_cnrom.nes \
  tests/roms/banktest_j87.nes \
  tests/roms/banktest_m101.nes
do
  [ -f "$rom" ] || {
    echo "FAIL: missing $rom (run: make boards)"
    status=1
    continue
  }
  report=$(target/release/se-cli run "$rom" --frames 60 2>&1)
  if printf '%s\n' "$report" | grep -q 'حالة الاختبار: نجاح كامل'; then
    text=$(printf '%s\n' "$report" | sed -n 's/^نص النتيجة: //p')
    printf 'PASS: %-32s %s\n' "$(basename "$rom" .nes)" "$text"
  else
    echo "FAIL: $rom"
    printf '%s\n' "$report" | grep -E 'حالة الاختبار|نص النتيجة|خطأ' || true
    status=1
  fi
done

if [ "$status" -eq 0 ]; then
  echo "PASS: all T1 boards verified (9/9)"
fi
exit "$status"
