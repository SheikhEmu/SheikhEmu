#!/bin/sh
# provenance_check.sh - provenance firewall gate.
#
# The reference emulators (Mesen2, puNES, ...) are used ONLY as output
# oracles, never as source. This gate fails when tracked files contain
# links to their source repositories, or to ROM download sites.
# Mentioning a reference emulator by name in the design docs (oracle
# policy) is allowed and is not flagged.
#
# Exit status: 0 = pass, 1 = violations found.

set -u
cd "$(dirname "$0")/.."

status=0

echo "== provenance gate =="

# 1. Links to reference emulator source repositories.
if git grep -inE '(github|gitlab|bitbucket)\.com/[^[:space:]]*(mesen|punes|fceux|nestopia|higan|nintaco|bizhawk|rnregex|emux)' \
    -- ':(exclude)tools/provenance_check.sh' >/dev/null 2>&1; then
  echo "FAIL: link(s) to reference emulator source repositories found:"
  git grep -inE '(github|gitlab|bitbucket)\.com/[^[:space:]]*(mesen|punes|fceux|nestopia|higan|nintaco|bizhawk|rnregex|emux)' \
    -- ':(exclude)tools/provenance_check.sh' || true
  status=1
fi

# 2. Links to ROM download sites.
if git grep -inE '(vimm\.net|cdromance|romhustler|loveroms|emuparadise|edgeemu|coolrom|wowroms|romsfun|doperoms|nicoblog|romulation)' \
    -- ':(exclude)tools/provenance_check.sh' >/dev/null 2>&1; then
  echo "FAIL: link(s) to ROM download sites found:"
  git grep -inE '(vimm\.net|cdromance|romhustler|loveroms|emuparadise|edgeemu|coolrom|wowroms|romsfun|doperoms|nicoblog|romulation)' \
    -- ':(exclude)tools/provenance_check.sh' || true
  status=1
fi

# 3. Phrases that indicate copied code.
if git grep -inE '(copied|stolen|ported) (from|out of) (mesen|punes|fceux|nestopia|higan|any other emulator)' \
    -- ':(exclude)tools/provenance_check.sh' >/dev/null 2>&1; then
  echo "FAIL: text indicates code copied from a reference emulator:"
  git grep -inE '(copied|stolen|ported) (from|out of) (mesen|punes|fceux|nestopia|higan|any other emulator)' \
    -- ':(exclude)tools/provenance_check.sh' || true
  status=1
fi

if [ "$status" -eq 0 ]; then
  echo "PASS: no forbidden source links or ROM site references"
fi
exit "$status"
