#!/bin/sh
# check_brick.sh - the M3.4 playability gate (D-28 adaptation).
#
# Runs the two committed deterministic input scenarios through the
# headless runner and asserts the game reaches its documented terminal
# states through REAL play: the paddle steering, the ball physics,
# brick destruction, the lives, the score, and the per-level bank
# writes all happen inside the ROM; the script only feeds the
# controller (the same --input-script protocol proven in M3.1).
#
#   WIN  scenario (tests/input/brick-win.txt): clears all three
#        towers, ends in YOU WIN: outcome=2, level reached = 3,
#        bank writes = 3, bricks destroyed = 15, lives = 3;
#   OVER scenario (tests/input/brick-over.txt): left-held steering
#        misses every serve, ends in GAME OVER: outcome=1, lives = 0.
#
# The full-run state hashes are compared against the deposited
# goldens (tests/golden/brick-win.txt, brick-over.txt).
#
# Exit status: 0 = pass, 1 = any failure.

set -u
cd "$(dirname "$0")/.."

echo "== brick gate (M3.4 play, D-28 adapted) =="

status=0

if [ ! -x target/release/se-cli ]; then
  cargo build --release -p se-cli || exit 1
fi

if [ ! -f tests/roms/brick.nes ]; then
  echo "FAIL: missing tests/roms/brick.nes (run: make brick)"
  exit 1
fi

dump=$(mktemp)

# One WRAM byte out of the RAM dump: $1 = file, $2 = WRAM offset.
wbyte() {
  od -An -tu1 -j $((0x800 + $2)) -N 1 "$1" | tr -d ' '
}

check() { # $1 = label, $2 = actual, $3 = expected
  if [ "$2" = "$3" ]; then
    printf 'PASS: %-28s %s\n' "$1" "$2"
  else
    printf 'FAIL: %-28s got %s, want %s\n' "$1" "$2" "$3"
    status=1
  fi
}

# ---- the WIN scenario ------------------------------------------------------
vid=$(mktemp -d)
report=$(target/release/se-cli run tests/roms/brick.nes --frames 2100 \
    --input-script tests/input/brick-win.txt --hash --dump-ram "$dump" --dump-video "$vid")
printf '%s\n' "$report" | grep -q 'نص النتيجة: YOU WIN' \
  || { echo "FAIL: win scenario never reached YOU WIN"; status=1; }

expected=$(sed -n 's/^hash=\([0-9a-f]*\)$/\1/p' tests/golden/brick-win.txt)
actual=$(printf '%s\n' "$report" | sed -n 's/^بصمة الحالة: \([0-9a-f]*\)$/\1/p')
check "win state hash" "$actual" "$expected"

check "win outcome (2)" "$(wbyte "$dump" 0x12)" "2"
check "win level reached (3rd)" "$(wbyte "$dump" 0x10)" "2"
check "win lives kept (3)" "$(wbyte "$dump" 0x11)" "3"
check "win bank writes (3)" "$(wbyte "$dump" 0x13)" "3"
check "win bricks destroyed (15)" "$(wbyte "$dump" 0x14)" "15"
check "win current bank (2)" "$(wbyte "$dump" 0x15)" "2"

# ---- the OVER scenario -----------------------------------------------------
report=$(target/release/se-cli run tests/roms/brick.nes --frames 1000 \
    --input-script tests/input/brick-over.txt --hash --dump-ram "$dump" --dump-video "$vid")
printf '%s\n' "$report" | grep -q 'نص النتيجة: GAME OVER' \
  || { echo "FAIL: over scenario never reached GAME OVER"; status=1; }

expected=$(sed -n 's/^hash=\([0-9a-f]*\)$/\1/p' tests/golden/brick-over.txt)
actual=$(printf '%s\n' "$report" | sed -n 's/^بصمة الحالة: \([0-9a-f]*\)$/\1/p')
check "over state hash" "$actual" "$expected"

check "over outcome (1)" "$(wbyte "$dump" 0x12)" "1"
check "over lives exhausted (0)" "$(wbyte "$dump" 0x11)" "0"

# ---- the HUD row tile-map gate (D-35, the owner's evidence standard) ------
# The final frame of each scenario, decoded as a tile map from the pixels:
# the complete non-overlapping SCORE/LEVEL/LIVES line.
python3 tools/hud_row_check.py "$vid/f02099.ppm" win || status=1
python3 tools/hud_row_check.py "$vid/f00999.ppm" over || status=1
python3 tools/hud_row_check.py "$vid/f00020.ppm" title || status=1

rm -f "$dump"
rm -rf "$vid"

if [ "$status" -eq 0 ]; then
  echo "PASS: SHEIKHEMU BRICK played to both terminal states (D-28)"
fi
exit "$status"
