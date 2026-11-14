//! APU demo golden determinism tests (M4.4).
//!
//! The embedded image is the exact PRG that `make test-roms` assembles
//! from tests/roms/src/apu_demo.asm (100% self-made content): the
//! NMI-driven eight-phase sound demo that exercises all five channels
//! in isolated windows with tones at published-formula frequencies
//! (pulse 1 A4 440.40 Hz, pulse 2 A5 873.91 Hz, triangle A3 220.20 Hz,
//! short-mode noise 300.70 Hz pattern, DMC 261.36 Hz delta triangles,
//! then the five-channel ensemble). The machine is built exactly the
//! way the CLI builds it from apu_demo.nes (iNES mapper 0, blank 8 KiB
//! CHR-ROM, vertical mirroring, no PRG RAM, NTSC), so the state hash
//! and the PCM fingerprint asserted here are the same values
//! tools/check_apu_demo.sh enforces on the assembled cartridge in the
//! gates job - the same ROM, the same goldens, on ubuntu, windows and
//! macOS.
//!
//! The golden window is 1040 frames: all eight 128-frame phases plus
//! the first 16 frames of the replayed silence. The silence phases are
//! part of the fingerprint on purpose: their PCM encodes the DC
//! blocker's settling behavior. The B-10/D-42 owner-approved migration
//! (2026-11-14) moved exactly this golden pair together with the four
//! concert values — the pole now rounds toward zero, so the trailing
//! silence decays to exactly 0 instead of latching on the old
//! -14323 constant: state 0c197976941b5b27 -> 3d15efeb5b82a7c8,
//! PCM 7548173990dcf41e -> 1082b39f30867115 (old -> new; the
//! decision, the values and the proof that nothing else moved are in
//! DECISIONS.md D-42 and EVIDENCE.md sections 7 and 10.5).

use se_core::machine::Machine;
use se_core::ppu::{Chr, Mirroring};
use se_core::timing::Region;

/// The demo window the goldens cover (frames).
const GOLDEN_FRAMES: u32 = 1040;
/// The committed NTSC goldens (see tests/golden/apu-demo.txt).
const GOLDEN_STATE: &str = "3d15efeb5b82a7c8";
const GOLDEN_PCM: &str = "1082b39f30867115";

/// The apu_demo.nes PRG image: the identical bytes `make test-roms`
/// assembles from tests/roms/src/apu_demo.asm. The cross-check test
/// below compares this copy with the built cartridge whenever it is
/// present, so the two cannot drift apart silently.
fn apu_demo_prg() -> Vec<u8> {
    let mut prg = vec![0u8; 16 * 1024];
    // $C000: reset setup, the bare main loop, the NMI handler, the
    // script walker, the eight phase scripts and the pointer table.
    #[rustfmt::skip]
    let code: [u8; 216] = [
        0x78, 0xD8, 0xA2, 0x00, 0xA9, 0x00, 0x95, 0x00, 0xE8, 0xD0, 0xFB, 0xA9,
        0x00, 0x85, 0x10, 0x85, 0x11, 0x8D, 0x15, 0x40, 0xA9, 0x40, 0x8D, 0x17,
        0x40, 0xA9, 0x80, 0x8D, 0x00, 0x20, 0xA2, 0x00, 0xBD, 0xC8, 0xC0, 0x95,
        0x30, 0xE8, 0xE0, 0x10, 0xD0, 0xF6, 0x4C, 0x2A, 0xC0, 0xE6, 0x11, 0xA5,
        0x11, 0xC9, 0x80, 0x90, 0x0C, 0xA9, 0x00, 0x85, 0x11, 0xE6, 0x10, 0xA5,
        0x10, 0x29, 0x07, 0x85, 0x10, 0xA5, 0x11, 0xD0, 0x11, 0xA5, 0x10, 0x0A,
        0xA8, 0xB9, 0x30, 0x00, 0x85, 0x20, 0xB9, 0x31, 0x00, 0x85, 0x21, 0x20,
        0x57, 0xC0, 0x40, 0xA0, 0x00, 0xB1, 0x20, 0xC9, 0xFF, 0xF0, 0x0B, 0xAA,
        0xC8, 0xB1, 0x20, 0x9D, 0x00, 0x40, 0xC8, 0x4C, 0x59, 0xC0, 0x60, 0x40,
        0x15, 0x00, 0xFF, 0x15, 0x01, 0x00, 0xBF, 0x01, 0x00, 0x02, 0xFD, 0x03,
        0x08, 0xFF, 0x15, 0x02, 0x04, 0xBF, 0x05, 0x00, 0x06, 0x7F, 0x07, 0x08,
        0xFF, 0x15, 0x04, 0x08, 0xFF, 0x0A, 0xFD, 0x0B, 0x08, 0xFF, 0x15, 0x08,
        0x0C, 0x3F, 0x0E, 0x84, 0x0F, 0x08, 0xFF, 0x12, 0x80, 0x13, 0x02, 0x10,
        0x40, 0x15, 0x10, 0xFF, 0x15, 0x1F, 0x00, 0xBF, 0x01, 0x00, 0x02, 0xFD,
        0x03, 0x08, 0x04, 0xBF, 0x05, 0x00, 0x06, 0x7F, 0x07, 0x08, 0x08, 0xFF,
        0x0A, 0xFD, 0x0B, 0x08, 0x0C, 0x3F, 0x0E, 0x84, 0x0F, 0x08, 0x12, 0x80,
        0x13, 0x02, 0x10, 0x40, 0xFF, 0x15, 0x00, 0xFF, 0x6C, 0xC0, 0x6F, 0xC0,
        0x7A, 0xC0, 0x85, 0xC0, 0x8E, 0xC0, 0x97, 0xC0, 0xA0, 0xC0, 0xC5, 0xC0,
    ];
    prg[..code.len()].copy_from_slice(&code);
    // $E000: the 33-byte looped DMC sample (16 bit-balanced $FF/$00
    // triangles + the $0F that keeps the loop drift-free).
    #[rustfmt::skip]
    let sample: [u8; 33] = [
        0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00,
        0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00,
        0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0x0F,
    ];
    prg[0x2000..0x2000 + sample.len()].copy_from_slice(&sample);
    // Vectors: NMI $C02D, RESET $C000, IRQ $C06B.
    prg[0x3FFA..0x4000].copy_from_slice(&[0x2D, 0xC0, 0x00, 0xC0, 0x6B, 0xC0]);
    prg
}

/// The machine exactly as the CLI builds it from apu_demo.nes.
fn demo_machine() -> Machine {
    Machine::from_rom(
        apu_demo_prg(),
        0,
        0,
        Chr::Rom(vec![0; 0x2000]),
        Mirroring::Vertical,
        false,
        Region::Ntsc,
    )
    .expect("valid apu_demo PRG")
}

#[test]
fn ntsc_demo_state_and_pcm_goldens() {
    let mut machine = demo_machine();
    machine.reset();
    machine.run_frames(GOLDEN_FRAMES).expect("demo runs");
    assert_eq!(
        format!("{:016x}", machine.state_hash()),
        GOLDEN_STATE,
        "NTSC apu_demo state golden"
    );
    assert_eq!(
        format!("{:016x}", machine.pcm_hash()),
        GOLDEN_PCM,
        "NTSC apu_demo PCM golden"
    );
    // The NMI-driven schedule must have reached the documented point:
    // the replayed phase 0 (silence), 16 frames into the second loop.
    assert_eq!(machine.bus.peek(0x10), 0, "the phase counter wrapped");
    assert_eq!(machine.bus.peek(0x11), 16, "16 frames into the replay");
}

#[test]
fn identical_demo_runs_are_byte_identical() {
    let run = || {
        let mut machine = demo_machine();
        machine.reset();
        machine.enable_pcm_capture(true);
        machine.run_frames(GOLDEN_FRAMES).expect("demo runs");
        (machine.take_pcm(), machine.state_hash(), machine.pcm_hash())
    };
    let (pcm_a, state_a, hash_a) = run();
    let (pcm_b, state_b, hash_b) = run();
    assert_eq!(pcm_a, pcm_b, "identical runs give identical PCM bytes");
    assert_eq!(state_a, state_b);
    assert_eq!(hash_a, hash_b);
    // One sample per CPU cycle, all of them counted.
    assert_eq!(pcm_a.len() as u64, 30_971_887, "the native-rate length");
}

#[test]
fn every_solo_phase_is_audible_in_its_window() {
    // The eight 128-frame phases map to eighths of the stream. Windows
    // 1..=6 are the solo/ensemble windows and must carry real signal;
    // the silence windows (0 and 7) are fingerprinted wholesale above
    // and carry the documented DC settling behavior instead.
    let mut machine = demo_machine();
    machine.reset();
    machine.enable_pcm_capture(true);
    machine.run_frames(GOLDEN_FRAMES).expect("demo runs");
    let pcm = machine.take_pcm();
    let len = pcm.len();
    for phase in 1..=6usize {
        let a = phase * len / 8;
        let b = (phase + 1) * len / 8;
        let peak = pcm[a..b]
            .iter()
            .map(|&s| i32::from(s).abs())
            .max()
            .unwrap_or(0);
        assert!(peak > 4000, "phase {phase} peak {peak} is far too quiet");
    }
}

#[test]
fn apu_demo_prg_matches_the_assembled_file_when_present() {
    // Cross-check with the real `make test-roms` output whenever the
    // cartridge exists (the CI gates job and local builds have it).
    let manifest = env!("CARGO_MANIFEST_DIR");
    let rom_path = std::path::Path::new(manifest).join("../../tests/roms/apu_demo.nes");
    let Ok(data) = std::fs::read(&rom_path) else {
        return;
    };
    assert_eq!(data.len(), 16 + 16 * 1024 + 8 * 1024, "apu_demo.nes layout");
    let assembled_prg = &data[16..16 + 16 * 1024];
    assert_eq!(
        assembled_prg,
        apu_demo_prg().as_slice(),
        "apu_demo.asm and the embedded test image drifted apart"
    );
}
