//! APU golden determinism tests (M4.2, the D-37 hard condition).
//!
//! An inline self-made "concert" program drives all five channels of
//! the real APU through the real bus — pulse 1 as a sustained
//! constant-volume tone, pulse 2 with an envelope and a downward
//! sweep, a free-running triangle, halted noise, and a 33-byte DMC
//! sample whose fetches stall the CPU — then the full-machine state
//! hash and the PCM stream fingerprint are compared against the
//! committed goldens. Pure integer arithmetic end to end means the
//! same values must come out on every platform (the CI matrix runs
//! these on ubuntu, windows and macOS).

use se_core::machine::Machine;
use se_core::timing::Region;

/// The inline APU concert program (16 KiB NROM, CHR-RAM convention of
/// `Machine::new`). The code is a straight register setup followed by
/// an infinite jump; the DMC sample bytes live at $8F00 and are
/// fetched through the $CF00 mirror of the 16 KiB window.
fn concert_prg() -> Vec<u8> {
    let mut prg = vec![0u8; 16 * 1024];
    #[rustfmt::skip]
    let code: [u8; 105] = [
        0x78,                   // SEI: keep the harness deterministic
        0xD8,                   // CLD
        0xA9, 0x0F, 0x8D, 0x15, 0x40, // $4015 = $0F: p1 p2 t n on
        // Pulse 1: duty 2, halt, constant volume 10, A4-ish tone.
        0xA9, 0xBA, 0x8D, 0x00, 0x40, // $4000
        0xA9, 0x00, 0x8D, 0x01, 0x40, // $4001: sweep off (no mute)
        0xA9, 0xFD, 0x8D, 0x02, 0x40, // $4002: timer low $FD
        0xA9, 0xF8, 0x8D, 0x03, 0x40, // $4003: length 30, high 0
        // Pulse 2: duty 1, halt, envelope, sweeping downward.
        0xA9, 0x6A, 0x8D, 0x04, 0x40, // $4004
        0xA9, 0xB9, 0x8D, 0x05, 0x40, // $4005: on, period 3, negate, shift 1
        0xA9, 0x54, 0x8D, 0x06, 0x40, // $4006: timer low $54
        0xA9, 0xC8, 0x8D, 0x07, 0x40, // $4007: length 192, high 0
        // Triangle: control on, reload 127, timer $1FD.
        0xA9, 0xFF, 0x8D, 0x08, 0x40, // $4008
        0xA9, 0xFD, 0x8D, 0x0A, 0x40, // $400A
        0xA9, 0xF8, 0x8D, 0x0B, 0x40, // $400B: length 30, timer high 0
        // Noise: halt, constant volume 8, period index 12.
        0xA9, 0x38, 0x8D, 0x0C, 0x40, // $400C
        0xA9, 0x0C, 0x8D, 0x0E, 0x40, // $400E
        0xA9, 0xF0, 0x8D, 0x0F, 0x40, // $400F: length 32
        // DMC: rate $F, direct level $20, sample at $CF00, 33 bytes.
        0xA9, 0x0F, 0x8D, 0x10, 0x40, // $4010
        0xA9, 0x20, 0x8D, 0x11, 0x40, // $4011
        0xA9, 0x3C, 0x8D, 0x12, 0x40, // $4012 -> $C000 + $3C*64 = $CF00
        0xA9, 0x02, 0x8D, 0x13, 0x40, // $4013 -> 33 bytes
        // All five channels on (starts the DMC sample).
        0xA9, 0x1F, 0x8D, 0x15, 0x40, // $4015 = $1F
        0x4C, 0x66, 0x80,             // JMP $8066 forever
    ];
    prg[..code.len()].copy_from_slice(&code);
    // The DMC sample bytes at $8F00 (a deterministic pattern).
    let sample: [u8; 33] = [
        0x55, 0xAA, 0x33, 0xCC, 0x0F, 0xF0, 0x5A, 0xA5, 0x3C, 0xC3, 0x96, 0x69, 0x0F, 0xF0, 0x81,
        0x7E, 0x24, 0xDB, 0x55, 0xAA, 0x33, 0xCC, 0x0F, 0xF0, 0x5A, 0xA5, 0x3C, 0xC3, 0x96, 0x69,
        0x0F, 0xF0, 0x7F,
    ];
    prg[0x0F00..0x0F00 + sample.len()].copy_from_slice(&sample);
    // Vectors: NMI, reset and IRQ all at $8000.
    prg[0x3FFA..0x4000].copy_from_slice(&[0x00, 0x80, 0x00, 0x80, 0x00, 0x80]);
    prg
}

/// Run the concert for `frames` frames and return the machine, the
/// state hash and the PCM fingerprint.
fn run_concert(region: Region, frames: u32, capture: bool) -> (Machine, u64, u64) {
    let mut machine = Machine::new(concert_prg(), region).expect("concert prg");
    machine.reset();
    machine.enable_pcm_capture(capture);
    machine.run_frames(frames).expect("concert runs");
    let state = machine.state_hash();
    let pcm = machine.pcm_hash();
    (machine, state, pcm)
}

#[test]
fn ntsc_concert_state_and_pcm_goldens() {
    let (_, state, pcm) = run_concert(Region::Ntsc, 45, false);
    assert_eq!(
        format!("{state:016x}"),
        "2bbd410b30f0f451",
        "NTSC concert state golden"
    );
    assert_eq!(
        format!("{pcm:016x}"),
        "afdc159028c540ec",
        "NTSC concert PCM golden"
    );
}

#[test]
fn pal_concert_state_and_pcm_goldens() {
    let (_, state, pcm) = run_concert(Region::Pal, 45, false);
    assert_eq!(
        format!("{state:016x}"),
        "4f58f320851574e0",
        "PAL concert state golden"
    );
    assert_eq!(
        format!("{pcm:016x}"),
        "d920be6eaf8a1f21",
        "PAL concert PCM golden"
    );
}

#[test]
fn identical_concert_runs_are_byte_identical() {
    // Two captured runs produce exactly the same sample bytes — the
    // determinism the D-37 hard condition demands, proven directly.
    let (mut first, state_a, pcm_a) = run_concert(Region::Ntsc, 10, true);
    let mut second = first.clone();
    let _ = second.run_frames(0);
    let (mut third, state_b, pcm_b) = run_concert(Region::Ntsc, 10, true);
    let a = first.take_pcm();
    let b = third.take_pcm();
    assert_eq!(a, b, "identical runs give identical PCM bytes");
    assert_eq!(state_a, state_b);
    assert_eq!(pcm_a, pcm_b);
    // Capture must not influence the machine state (the buffer is an
    // output artifact, not state).
    assert_eq!(first.state_hash(), second.state_hash());
}

#[test]
fn concert_actually_makes_sound() {
    // Sanity: the concert stream is not silence — the channels are
    // audible in the captured bytes.
    let (mut machine, _, _) = run_concert(Region::Ntsc, 5, true);
    let pcm = machine.take_pcm();
    let peak = pcm.iter().map(|&s| i32::from(s).abs()).max().unwrap_or(0);
    assert!(peak > 4_000, "the concert peak {peak} is far too quiet");
}
