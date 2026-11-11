#!/bin/sh
# size_check.sh - binary size budget gate.
#
# The release binary must stay within the M0 budget of 4 MiB. The
# current baseline is recorded in docs/SIZE_BASELINE.md; a regression
# beyond the budget fails the gate, and growth above the baseline is
# reported (advisory) so the owner can re-baseline it deliberately.
#
# Exit status: 0 = pass, 1 = over budget.

set -u
cd "$(dirname "$0")/.."

BUDGET=4194304 # 4 MiB
BIN=target/release/se-cli

echo "== size gate =="

if [ ! -x "$BIN" ]; then
  cargo build --release -p se-cli || exit 1
fi

size=$(wc -c < "$BIN" | tr -d '[:space:]')
echo "release binary: $size bytes (budget: $BUDGET)"

if [ "$size" -gt "$BUDGET" ]; then
  echo "FAIL: binary exceeds the 4 MiB budget"
  exit 1
fi

if [ -f docs/SIZE_BASELINE.md ]; then
  baseline=$(grep -oE '[0-9,]{6,}' docs/SIZE_BASELINE.md | head -n 1 | tr -d ',')
  if [ -n "$baseline" ] && [ "$size" -gt "$baseline" ]; then
    echo "note: binary grew above the recorded baseline ($baseline bytes)"
  fi
fi

echo "PASS: within budget"
exit 0
