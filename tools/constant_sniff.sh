#!/bin/sh
# constant_sniff.sh - advisory scan for suspicious provenance hints.
#
# Unlike the other gates this one never fails the build; it prints
# anything that looks like an unexplained magic constant or a
# provenance note ("copied from ...", "value taken from ...") so a
# human can review whether the number has a documented source.

set -u
cd "$(dirname "$0")/.."

echo "== constant sniff (advisory) =="

grep -rinE '(copied|taken|borrowed) from' crates/ tools/ || true
grep -rinE '0x[0-9A-Fa-f]{4,}[[:space:]]*//.*(magic|unknown|why)' \
  crates/ || true

echo "advisory scan done"
exit 0
