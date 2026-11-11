//! M1.2 calibration (أ): execute the self-made hello cartridge on the
//! real memory bus and prove its cycle counts against an independent
//! arithmetic model of the program.
//!
//! The hello program (tests/roms/src/hello.asm, assembled by
//! tools/asm/mini_asm.py) clears RAM, then loops forever incrementing a
//! tick counter at `$10` and a lap counter at `$11`, publishing them at
//! `$0200` and `$0201`. Its whole timing is known instruction by
//! instruction, so a small model written from the documented cycle
//! table (independent of the CPU engine) can predict the exact visible
//! counters at any cycle boundary. Matching the machine to the model is
//! the M1.2 proof; the M1.5 proof (nestest, calibration ب) lives in the
//! session protocol, not in the repository.

use se_core::machine::Machine;
use se_core::timing::Region;

/// The hello.nes PRG image: the identical bytes `make test-roms`
/// assembles from tests/roms/src/hello.asm (100% self-made content).
/// `hello_prg_matches_the_assembled_file` below cross-checks the two
/// whenever the built ROM is present, so the copies cannot drift.
fn hello_prg() -> Vec<u8> {
    let mut prg = vec![0u8; 16 * 1024];
    let code: [u8; 38] = [
        // reset: SEI, CLD, LDX #$00
        0x78, 0xD8, 0xA2, 0x00,
        // clear_zero_page: LDA #$00 / STA $00,X / INX / BNE (loop)
        0xA9, 0x00, 0x95, 0x00, 0xE8, 0xD0, 0xF9,
        // counters to zero: LDA #$00 / STA $10 / STA $11
        0xA9, 0x00, 0x85, 0x10, 0x85, 0x11, // main: INC $10 / BNE no_lap / [INC $11]
        0xE6, 0x10, 0xD0, 0x02, 0xE6, 0x11,
        // LDA $10 / STA $0200 / LDA $11 / STA $0201 / JMP main
        0xA5, 0x10, 0x8D, 0x00, 0x02, 0xA5, 0x11, 0x8D, 0x01, 0x02, 0x4C, 0x11, 0xC0,
        // nmi: RTI, irq: RTI
        0x40, 0x40,
    ];
    prg[..code.len()].copy_from_slice(&code);
    // Vectors: NMI $C024, RESET $C000, IRQ $C025.
    prg[0x3FFA..0x4000].copy_from_slice(&[0x24, 0xC0, 0x00, 0xC0, 0x25, 0xC0]);
    prg
}

/// Independent timing model of the hello loop.
///
/// Written straight from the documented instruction table (NESdev Wiki
/// "Cycle reference chart"), not from the engine: reset 7; setup
/// 2+2+2 + (255×11 + 10) + 2+3+3; then per iteration INC 5 + branch 3
/// (2 when the tick wraps) + wrap INC 5 + LDA 3 + STA 4 + LDA 3 + STA 4
/// + JMP 3.
///
/// The counters become visible when the publishing stores complete.
fn model_visible_at(cycles: u64) -> (u8, u8) {
    // The first INC $10 starts at 7 + 2829.
    let mut t = 7 + 2829;
    let mut tick: u64 = 0;
    let mut lap: u64 = 0;
    let (mut published_tick, mut published_lap) = (0u8, 0u8);
    loop {
        // INC $10 (5): the tick advances.
        t += 5;
        tick += 1;
        let wrap = tick.is_multiple_of(256);
        // BNE (3 taken / 2 not) and the lap increment on wrap (5).
        t += if wrap { 2 + 5 } else { 3 };
        lap += u64::from(wrap);
        // LDA $10 + STA $0200: the tick becomes visible.
        t += 3 + 4;
        if cycles < t {
            break;
        }
        published_tick = (tick % 256) as u8;
        // LDA $11 + STA $0201: the lap becomes visible.
        t += 3 + 4;
        if cycles < t {
            break;
        }
        published_lap = (lap % 256) as u8;
        // JMP main (3).
        t += 3;
        if cycles < t {
            break;
        }
    }
    (published_tick, published_lap)
}

fn machine_with_hello() -> Machine {
    Machine::new(hello_prg(), Region::Ntsc).expect("valid hello PRG")
}

#[test]
fn hello_counters_match_the_independent_model_at_every_boundary() {
    // Boundaries include exact instruction completions, mid-loop points,
    // the first wrap (tick 256) and a deep run.
    for &budget in &[
        2840u64, // before the first INC completes
        2841,    // first INC done, nothing published yet
        2851,    // STA $0200 of tick 1 done
        2858,    // STA $0201 of tick 1 done
        2861,    // JMP of tick 1 done
        9230,    // STA $0200 of the wrapping tick 256
        9237,    // STA $0201 of the wrapping tick 256
        5_000, 12_345, 100_000, 1_000_000,
    ] {
        let mut machine = machine_with_hello();
        machine.reset();
        machine.run_cycles(budget).expect("hello runs forever");
        // run_cycles stops at the first whole-instruction end at or past
        // the budget: the model must be evaluated at that exact cycle.
        let visible = (machine.bus.peek(0x0200), machine.bus.peek(0x0201));
        let expected = model_visible_at(machine.cycle);
        assert_eq!(
            visible, expected,
            "tick/lap counters at cycle {} (budget {})",
            machine.cycle, budget
        );
    }
}

#[test]
fn hello_state_hash_is_deterministic_across_runs() {
    let run = || {
        let mut machine = machine_with_hello();
        machine.reset();
        machine.run_frames(2).expect("hello runs forever");
        machine.state_hash()
    };
    assert_eq!(run(), run(), "identical runs must hash identically");
}

#[test]
fn hello_interrupts_round_trip_through_the_real_vectors() {
    // M1.3 on the real cartridge: an NMI pushes the interrupted PC,
    // lands on the hello NMI handler (RTI) and returns exactly.
    let mut machine = machine_with_hello();
    machine.reset();
    machine.run_cycles(2900).expect("hello runs forever");
    let interrupted_pc = machine.cpu.pc;
    let cycles = u64::from(machine.cpu.service_interrupt(&mut machine.bus, 0xFFFA));
    assert_eq!(cycles, 7);
    assert_eq!(machine.cpu.pc, 0xC024);
    machine.cycle += cycles;
    // RTI of the handler: six documented cycles.
    assert_eq!(machine.step(), Ok(6));
    machine.cycle += 6;
    assert_eq!(machine.cpu.pc, interrupted_pc);
}

#[test]
fn hello_prg_matches_the_assembled_file_when_present() {
    // Cross-check with the real `make test-roms` output when it exists
    // (it does in the CI gates job and local builds). Absence simply
    // means this cross-check runs where the file is built; the engine
    // tests above are unconditional.
    let manifest = env!("CARGO_MANIFEST_DIR");
    let rom_path = std::path::Path::new(manifest).join("../../tests/roms/hello.nes");
    let Ok(data) = std::fs::read(&rom_path) else {
        return;
    };
    assert_eq!(data.len(), 16 + 16 * 1024 + 8 * 1024, "hello.nes layout");
    let assembled_prg = &data[16..16 + 16 * 1024];
    assert_eq!(
        assembled_prg,
        hello_prg().as_slice(),
        "hello.asm and the embedded test image drifted apart"
    );
}
