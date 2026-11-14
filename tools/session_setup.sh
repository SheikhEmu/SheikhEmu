#!/bin/sh
# tools/session_setup.sh - deterministic session bootstrap (D-43).
#
# The execution platform recycles the sandbox between sessions, which
# historically lost: the Rust toolchain PATH, the ALSA development
# headers (se-frontend's cpal needs them to build on Linux), and the
# category-B external test ROMs. This script re-provisions every one of
# those from its documented, PINNED source, idempotently, and verifies
# each with a hard check. It must be the FIRST command of every session
# (see docs/SESSION_PROTOCOL.md).
#
# What it provisions (all outside the repository, never committed):
#   1. Rust         - ~/.cargo via rustup: source ~/.cargo/env, install
#                     the pinned stable/minimal profile if missing, and
#                     ensure the clippy + rustfmt components.
#   2. ALSA headers - /usr/include/alsa when the system dev package is
#                     present; otherwise sudo apt when passwordless sudo
#                     is available; otherwise a PINNED debian .deb
#                     (SHA256-verified) extracted rootless to
#                     ~/.local/sheikhemu-alsa with pkg-config glue.
#   3. Category-B   - ~/SheikhEmuRoms clones held at pinned commits
#      ROMs           (christopherpow/nes-test-roms, bbbradsmith/
#                     nes-audio-tests) plus the three standalone
#                     verification vectors with pinned SHA256. Run-only
#                     material (SPEC 12.2): it NEVER enters any commit,
#                     and the repo's .gitignore must forbid it.
#
# The script also (re)writes ~/.sheikhemu-session-env.sh; source that
# file afterwards to get PATH / PKG_CONFIG_PATH exports in the shell.
#
# Exit status: 0 = every check green, 1 = at least one FAIL (fix it or
# report to the owner BEFORE starting any work).
#
# Pinned provenance (recorded 2026-11-15 by the D-43 session):
#   christopherpow/nes-test-roms.git @ 95d8f621ae55cee0d09b91519a8989ae0e64753b
#   bbbradsmith/nes-audio-tests.git @ 99ef107f3f3ffe19f72937507ca0820a4f5c34cd
#   libasound2-dev 1.2.14-1+deb13u1 amd64
#     sha256 dfa8e8a133ef8704093d03464d86a3e86d26232baf1e3827aa030cb68c437e1c

set -u

# ---------------- pinned constants ----------------
RUST_TOOLCHAIN=stable
RUSTUP_INIT_URL='https://sh.rustup.rs'

ROMS_DIR="${SHEIKHEMU_ROMS:-$HOME/SheikhEmuRoms}"
NTR_URL='https://github.com/christopherpow/nes-test-roms.git'
NTR_COMMIT='95d8f621ae55cee0d09b91519a8989ae0e64753b'
NAT_URL='https://github.com/bbbradsmith/nes-audio-tests.git'
NAT_COMMIT='99ef107f3f3ffe19f72937507ca0820a4f5c34cd'

NESTEST_NES_SHA256='f67d55fd6b3cf0bad1cc85f1df0d739c65b53e79cecb7fea8f77ec0eadab0004'
NESTEST_LOG_SHA256='442c4dd5539c7e88b3fd73c7b732a7eadbd22b47c2cd9e58397ef147f64f6f8f'
PPU_VBL_NMI_SHA256='8dbab1be785585c399cf055ef02147b788ab75fd80e81cf9568a2feafc03fb7d'

ALSA_PREFIX="$HOME/.local/sheikhemu-alsa"
ALSA_DEB_URL='https://deb.debian.org/debian/pool/main/a/alsa-lib/libasound2-dev_1.2.14-1+deb13u1_amd64.deb'
ALSA_DEB_SHA256='dfa8e8a133ef8704093d03464d86a3e86d26232baf1e3827aa030cb68c437e1c'
ALSA_VERSION='1.2.14'
SYSTEM_LIBASOUND='/usr/lib/x86_64-linux-gnu/libasound.so.2'

ENV_FILE="$HOME/.sheikhemu-session-env.sh"

FAILED=0
ok()   { printf '  ok   %s\n' "$*"; }
fail() { FAILED=$((FAILED + 1)); printf '  FAIL %s\n' "$*"; }
note() { printf '       %s\n' "$*"; }

verify_sha256() { # file expected_sha256
    want="$2"
    got=$(sha256sum "$1" 2>/dev/null | cut -d' ' -f1) || got=''
    [ "$got" = "$want" ]
}

# ---------------- resolve the repository ----------------
REPO_ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd) || {
    printf '  FAIL cannot resolve repo root\n'; exit 1; }

# =================================================================
# 0. preflight
# =================================================================
printf '== 0. preflight ==\n'
[ -f "$REPO_ROOT/tools/mkrom.py" ] \
    && ok "repo root: $REPO_ROOT" \
    || fail "repo root not recognized: $REPO_ROOT"
if ! git -C "$REPO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    fail "$REPO_ROOT is not a git work tree"
fi
for c in git curl sha256sum dpkg-deb pkg-config python3; do
    command -v "$c" >/dev/null 2>&1 \
        && ok "tool present: $c" \
        || fail "missing required tool: $c"
done
command -v ffmpeg >/dev/null 2>&1 \
    && ok "tool present: ffmpeg (WAV/MP3 for M4.6)" \
    || note 'advisory: ffmpeg missing (needed only at M4.6)'

# the category-B guard: no .nes image may ever be tracked
tracked_nes=$(git -C "$REPO_ROOT" ls-files -- '*.nes' 2>/dev/null)
if [ -n "$tracked_nes" ]; then
    fail 'tracked .nes files exist (category B must never be committed):'
    printf '%s\n' "$tracked_nes"
else
    ok 'no tracked .nes files in the repository'
fi
# the SPEC 12.2 .gitignore guard entries
for entry in 'SheikhEmuRoms/' '*.nes' '*.fds' '*.unf' '*.nsf'; do
    grep -qxF -- "$entry" "$REPO_ROOT/.gitignore" 2>/dev/null \
        && ok ".gitignore entry present: $entry" \
        || fail ".gitignore entry missing: $entry"
done

# =================================================================
# 1. Rust toolchain
# =================================================================
printf '== 1. Rust toolchain (stable, minimal profile) ==\n'
if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
    ok 'sourced ~/.cargo/env'
fi
if ! command -v cargo >/dev/null 2>&1; then
    note 'cargo missing - installing rustup (stable/minimal)'
    mkdir -p "$HOME/.cache/sheikhemu"
    if curl -sSfL -o "$HOME/.cache/sheikhemu/rustup-init.sh" "$RUSTUP_INIT_URL"; then
        sh "$HOME/.cache/sheikhemu/rustup-init.sh" -y \
            --profile minimal --default-toolchain "$RUST_TOOLCHAIN" \
            --no-modify-path >/dev/null || fail 'rustup-init failed'
        . "$HOME/.cargo/env"
    else
        fail 'could not download rustup-init'
    fi
fi
if command -v cargo >/dev/null 2>&1; then
    ok "cargo: $(cargo --version)"
    ok "rustc: $(rustc --version)"
else
    fail 'cargo still unavailable after provisioning'
fi
command -v cargo-clippy >/dev/null 2>&1 \
    || rustup component add clippy >/dev/null 2>&1 \
    || fail 'rustup component add clippy failed'
command -v cargo-clippy >/dev/null 2>&1 && ok 'component: clippy'
command -v cargo-fmt >/dev/null 2>&1 \
    || rustup component add rustfmt >/dev/null 2>&1 \
    || fail 'rustup component add rustfmt failed'
command -v cargo-fmt >/dev/null 2>&1 && ok 'component: rustfmt'

# =================================================================
# 2. ALSA development headers
# =================================================================
printf '== 2. ALSA development headers (se-frontend / cpal) ==\n'
ALSA_MODE=''
if [ -f /usr/include/alsa/asoundlib.h ]; then
    ALSA_MODE='system'
    ok 'system ALSA dev headers at /usr/include/alsa'
elif command -v sudo >/dev/null 2>&1 && sudo -n true >/dev/null 2>&1; then
    note 'passwordless sudo available - apt-get install libasound2-dev'
    if sudo apt-get update >/dev/null 2>&1 \
       && sudo apt-get install -y libasound2-dev >/dev/null 2>&1 \
       && [ -f /usr/include/alsa/asoundlib.h ]; then
        ALSA_MODE='system'
        ok 'system ALSA dev headers installed via apt'
    else
        fail 'apt libasound2-dev install failed; trying rootless pinned deb'
    fi
fi
if [ -z "$ALSA_MODE" ]; then
    # rootless path: pinned debian deb extracted to a local prefix with
    # pkg-config glue. Deterministic: pinned URL + pinned SHA256.
    [ "$(uname -m)" = 'x86_64' ] \
        || fail "rootless ALSA path pinned for x86_64 only (got $(uname -m))"
    [ -f "$SYSTEM_LIBASOUND" ] \
        || fail "system runtime libasound.so.2 missing: $SYSTEM_LIBASOUND"
    if [ ! -f "$ALSA_PREFIX/lib/pkgconfig/alsa.pc" ] \
       || [ ! -f "$ALSA_PREFIX/include/alsa/asoundlib.h" ]; then
        note "provisioning rootless prefix: $ALSA_PREFIX"
        mkdir -p "$ALSA_PREFIX/pool" "$ALSA_PREFIX/lib/pkgconfig" \
                 "$ALSA_PREFIX/include"
        curl -sSfL -o "$ALSA_PREFIX/pool/libasound2-dev.deb" "$ALSA_DEB_URL" \
            || fail "download failed: $ALSA_DEB_URL"
        if verify_sha256 "$ALSA_PREFIX/pool/libasound2-dev.deb" \
                         "$ALSA_DEB_SHA256"; then
            ok 'deb sha256 matches the pinned value'
        else
            fail 'deb sha256 MISMATCH - refusing to install'
        fi
        rm -rf "$ALSA_PREFIX/root"
        mkdir -p "$ALSA_PREFIX/root"
        dpkg-deb -x "$ALSA_PREFIX/pool/libasound2-dev.deb" \
                 "$ALSA_PREFIX/root" \
            || fail 'dpkg-deb extraction failed'
        rm -rf "$ALSA_PREFIX/include/alsa"
        cp -a "$ALSA_PREFIX/root/usr/include/alsa" "$ALSA_PREFIX/include/alsa"
        ln -sf "$SYSTEM_LIBASOUND" "$ALSA_PREFIX/lib/libasound.so"
        sed -e "s|^prefix=.*|prefix=$ALSA_PREFIX|" \
           -e 's|^exec_prefix=.*|exec_prefix=${prefix}|' \
           -e 's|^libdir=.*|libdir=${prefix}/lib|' \
           -e 's|^includedir=.*|includedir=${prefix}/include|' \
           "$ALSA_PREFIX/root/usr/lib/x86_64-linux-gnu/pkgconfig/alsa.pc" \
           > "$ALSA_PREFIX/lib/pkgconfig/alsa.pc" \
           || fail 'alsa.pc rewrite failed'
        rm -rf "$ALSA_PREFIX/root"
    fi
    export PKG_CONFIG_PATH="$ALSA_PREFIX/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
    got_ver=$(pkg-config --modversion alsa 2>/dev/null) || got_ver=''
    if [ "$got_ver" = "$ALSA_VERSION" ]; then
        ALSA_MODE='rootless-prefix'
        ok "pkg-config alsa $got_ver via $ALSA_PREFIX (rootless, pinned)"
    else
        fail "pkg-config alsa version '$got_ver' != pinned $ALSA_VERSION"
    fi
fi

# =================================================================
# 3. category-B external ROMs (outside the repository, pinned)
# =================================================================
printf '== 3. category-B ROMs (run-only, outside the repository) ==\n'
mkdir -p "$ROMS_DIR"
ROMS_REAL=$(CDPATH='' cd -- "$ROMS_DIR" && pwd -P)
REPO_REAL=$(CDPATH='' cd -- "$REPO_ROOT" && pwd -P)
case "$ROMS_REAL" in
    "$REPO_REAL"|"$REPO_REAL"/*)
        fail "ROM dir inside the repository: $ROMS_REAL" ;;
    *)
        ok "ROM dir outside the repository: $ROMS_REAL" ;;
esac

clone_pinned() { # url dir commit label
    _url="$1"; _dir="$2"; _commit="$3"; _label="$4"
    if [ ! -d "$_dir/.git" ]; then
        note "cloning $_label"
        git clone --quiet "$_url" "$_dir" || {
            fail "clone failed: $_url"; return 1; }
    fi
    _cur=$(git -C "$_dir" rev-parse HEAD 2>/dev/null) || _cur=''
    if [ "$_cur" != "$_commit" ]; then
        note "$_label drift: $_cur -> re-pinning $_commit"
        git -C "$_dir" fetch --quiet origin \
            || fail "$_label fetch failed"
        git -C "$_dir" checkout --quiet "$_commit" \
            || fail "$_label checkout $_commit failed"
    fi
    _cur=$(git -C "$_dir" rev-parse HEAD 2>/dev/null) || _cur=''
    if [ "$_cur" = "$_commit" ]; then
        ok "$_label @ ${_commit}"
    else
        fail "$_label not at pinned commit ($_cur)"
    fi
}

clone_pinned "$NTR_URL" "$ROMS_DIR/nes-test-roms" "$NTR_COMMIT" \
    'christopherpow/nes-test-roms'
clone_pinned "$NAT_URL" "$ROMS_DIR/nes-audio-tests" "$NAT_COMMIT" \
    'bbbradsmith/nes-audio-tests'

# the three standalone verification vectors (tools/nestest_check.sh and
# tools/ppu_vbl_check.sh read them from the ROM dir root)
install_pinned_file() { # src dst sha256 label
    _src="$1"; _dst="$2"; _sha="$3"; _label="$4"
    if [ ! -f "$_dst" ]; then
        cp "$_src" "$_dst" || fail "copy failed: $_label"
    fi
    if verify_sha256 "$_dst" "$_sha"; then
        ok "$_label sha256 matches the pinned value"
    else
        fail "$_label sha256 MISMATCH (expected $_sha)"
    fi
}
install_pinned_file "$ROMS_DIR/nes-test-roms/other/nestest.nes" \
    "$ROMS_DIR/nestest.nes" "$NESTEST_NES_SHA256" 'nestest.nes'
install_pinned_file "$ROMS_DIR/nes-test-roms/other/nestest.log" \
    "$ROMS_DIR/nestest.log" "$NESTEST_LOG_SHA256" 'nestest.log'
install_pinned_file "$ROMS_DIR/nes-test-roms/ppu_vbl_nmi/ppu_vbl_nmi.nes" \
    "$ROMS_DIR/ppu_vbl_nmi.nes" "$PPU_VBL_NMI_SHA256" 'ppu_vbl_nmi.nes'

# provenance stamp (rewritten each run; doubles as the category-B
# license/index pointer required by SPEC 12.2)
{
    printf '# SheikhEmu category-B external ROMs - provenance stamp\n'
    printf '# generated by tools/session_setup.sh (D-43)\n'
    printf 'generated: %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf 'policy: run-only material (SPEC 12.2) - NEVER committed\n'
    printf 'nes-test-roms: %s @ %s\n' "$NTR_URL" "$NTR_COMMIT"
    printf 'nes-audio-tests: %s @ %s\n' "$NAT_URL" "$NAT_COMMIT"
    printf 'nestest.nes sha256: %s\n' "$NESTEST_NES_SHA256"
    printf 'nestest.log sha256: %s\n' "$NESTEST_LOG_SHA256"
    printf 'ppu_vbl_nmi.nes sha256: %s\n' "$PPU_VBL_NMI_SHA256"
    printf 'licenses: per-file index in each upstream repository\n'
    printf '  (nes-test-roms: readme.txt files; nes-audio-tests: readme.md)\n'
} > "$ROMS_DIR/.provenance"
ok "provenance stamp: $ROMS_DIR/.provenance"

# =================================================================
# 4. session env file
# =================================================================
{
    printf '# Generated by tools/session_setup.sh (D-43) - do not commit.\n'
    printf '# Source this file in the working shell:\n'
    printf '#   . %s\n' "$ENV_FILE"
    if [ -f "$HOME/.cargo/env" ]; then
        printf '. "%s/.cargo/env"\n' "$HOME"
    fi
    if [ "$ALSA_MODE" = 'rootless-prefix' ]; then
        printf 'export PKG_CONFIG_PATH="%s/lib/pkgconfig%s"\n' \
            "$ALSA_PREFIX" "${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
    fi
    printf 'export SHEIKHEMU_ROMS="%s"\n' "$ROMS_DIR"
} > "$ENV_FILE"
ok "session env written: $ENV_FILE"

# =================================================================
# summary
# =================================================================
printf '== summary ==\n'
if [ "$FAILED" -eq 0 ]; then
    printf 'PASS - environment ready. Next (SESSION_PROTOCOL.md):\n'
    printf '  . %s\n' "$ENV_FILE"
    printf '  git fetch origin && git reset --hard origin/main\n'
    printf '  git log origin/main..main --oneline | wc -l   # must be 0\n'
    exit 0
else
    printf 'FAIL - %d check(s) failed. Fix or report before any work.\n' \
        "$FAILED"
    exit 1
fi
