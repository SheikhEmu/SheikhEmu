#!/bin/sh
# check_files.sh - file content gate.
#
# Fails when:
#   - a .nes image exists outside tests/roms/ (our own self-made images),
#   - a ROM archive (.zip/.7z/.rar) exists anywhere except .git/target,
#   - a BIOS-looking file exists (disksys, *.fds, *bios*),
#   - code files still contain TODO/FIXME/XXX/HACK markers (no
#     placeholder structures are allowed in the project).
#
# Exit status: 0 = pass, 1 = violations found.

set -u
cd "$(dirname "$0")/.."

status=0

echo "== files gate =="

# 1. ROM images only under tests/roms/.
outside=$(find . -name '*.nes' \
  -not -path './tests/roms/*' -not -path './.git/*' -not -path './target/*' \
  -not -path './tests/roms/build/*' 2>/dev/null)
if [ -n "$outside" ]; then
  echo "FAIL: .nes file(s) outside tests/roms/:"
  echo "$outside"
  status=1
fi

# 2. No ROM archives.
archives=$(find . \( -name '*.zip' -o -name '*.7z' -o -name '*.rar' \) \
  -not -path './.git/*' -not -path './target/*' 2>/dev/null)
if [ -n "$archives" ]; then
  echo "FAIL: ROM archive file(s) found:"
  echo "$archives"
  status=1
fi

# 3. No BIOS-looking files.
bios=$(find . \( -iname '*bios*' -o -iname '*disksys*' -o -iname '*.fds' \) \
  -not -path './.git/*' -not -path './target/*' 2>/dev/null)
if [ -n "$bios" ]; then
  echo "FAIL: BIOS-like file(s) found:"
  echo "$bios"
  status=1
fi

# 4. No placeholder markers in code.
markers=$(grep -rnE 'TODO|FIXME|XXX|HACK' \
  crates/ tools/ tests/ Makefile 2>/dev/null)
if [ -n "$markers" ]; then
  echo "FAIL: placeholder marker(s) in code:"
  echo "$markers"
  status=1
fi

if [ "$status" -eq 0 ]; then
  echo "PASS: no copyrighted-looking files, no placeholder markers"
fi
exit "$status"
