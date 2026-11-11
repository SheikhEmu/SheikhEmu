# SheikhEmu makefile (POSIX make).
#
# Everything runs offline and locally; no network, no accounts.

PYTHON ?= python3

ASM := tools/asm/mini_asm.py
MKROM := tools/mkrom.py

ROM_BUILD := tests/roms/build
HELLO_BIN := $(ROM_BUILD)/hello.bin
HELLO_NES := tests/roms/hello.nes
PATTERN_BIN := $(ROM_BUILD)/pattern.bin
PATTERN_NES := tests/roms/pattern.nes

.PHONY: default test-roms gates fmt clippy test size golden ci-local clean help

default: help

help:
	@echo "SheikhEmu targets:"
	@echo "  make test-roms   assemble and package the self-made test ROMs"
	@echo "  make gates       run the compliance gates (provenance/cost/files)"
	@echo "  make fmt          cargo fmt --check"
	@echo "  make clippy      cargo clippy with warnings denied"
	@echo "  make test        cargo test --workspace"
	@echo "  make size        release build + 4 MiB size gate"
	@echo "  make golden      hello determinism + pattern frame gates"
	@echo "  make ci-local    everything CI runs, locally"
	@echo "  make clean       remove build outputs (not target/)"

$(ROM_BUILD):
	mkdir -p $(ROM_BUILD)

$(HELLO_BIN): tests/roms/src/hello.asm tools/asm/mini_asm.py | $(ROM_BUILD)
	$(PYTHON) $(ASM) --selftest
	$(PYTHON) $(ASM) $< -o $@ --sym $@.sym

$(HELLO_NES): $(HELLO_BIN) tools/mkrom.py
	$(PYTHON) $(MKROM) make --prg $(HELLO_BIN) --chr-size 8192 \
                --mapper 0 --mirroring v --output $@

$(PATTERN_BIN): tests/roms/src/pattern.asm tools/asm/mini_asm.py | $(ROM_BUILD)
	$(PYTHON) $(ASM) $< -o $@ --sym $@.sym

$(PATTERN_NES): $(PATTERN_BIN) tools/mkrom.py
	$(PYTHON) $(MKROM) make --prg $(PATTERN_BIN) --mapper 0 --mirroring v --output $@

test-roms: $(HELLO_NES) $(PATTERN_NES)
	$(PYTHON) $(ASM) --selftest

gates:
	tools/provenance_check.sh && tools/cost_check.sh && tools/check_files.sh

fmt:
	cargo fmt --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace

size:
	tools/size_check.sh

golden:
	tools/check_golden.sh
	tools/check_pattern.sh

ci-local: fmt clippy test test-roms gates size golden

clean:
	rm -rf $(ROM_BUILD) $(HELLO_NES)
