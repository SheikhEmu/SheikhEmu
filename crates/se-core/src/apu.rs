//! `apu` — the RP2A03 audio processing unit (M4.2).
//!
//! Five channels (pulse x2, triangle, noise, DMC), the 4/5-step frame
//! sequencer, and the non-linear mixer, written against the public
//! NESdev Wiki pages only ("APU", "APU Pulse", "APU Triangle",
//! "APU Noise", "APU DMC", "APU Sweep", "APU Envelope", "APU Length
//! Counter", "APU Frame Counter", "APU Mixer") with the length-counter
//! reload rule pinned by the published category-B `apu_test` sources
//! (blargg: a fourth-register write reloads the length counter even
//! when the halt flag is set, but not while the channel is disabled
//! through `$4015`).
//!
//! ## Determinism (D-37 hard condition)
//!
//! The whole signal path is integer-only. The mixer evaluates the
//! published formulas in Q15 fixed point (`u128` for the two wide
//! divisions), so the same inputs produce the same PCM bytes on every
//! platform — no floating point anywhere in the deterministic path.
//! One output sample is produced per CPU cycle: the native stream rate
//! is the CPU clock itself, derived from the master clock alone
//! (NTSC 1789772.7 Hz, PAL 1662607.0 Hz, Dendy 1773447.5 Hz). A
//! first-order integer high-pass (the DC blocker, pole 65535/65536)
//! removes the DC component; the full NES analog filter chain
//! (90 Hz / 440 Hz HPF, 14 kHz LPF) belongs to the frontend stage.
//!
//! ## B1 documented choices (D-39)
//!
//! - `$4000-$4014` reads keep the D-18 reference `$FF` (write-only
//!   registers; the full open-bus treatment is B2). `$4015` reads are
//!   the live status (M4.2), `$4017` reads stay joypad 2 (M3).
//! - DMC fetches stall the CPU for 4 cycles: three idle cycles plus
//!   the fetch cycle (D-38 B1; the 3-cycle read-alignment case and the
//!   OAM-DMA interleave are B2). A fetch requested during OAM DMA is
//!   deferred to the next regular cycle.
//! - The frame sequencer thresholds are the wiki's APU-cycle tables
//!   doubled into CPU cycles; the frame IRQ flag is set at the first
//!   documented set point (29828/33252 CPU cycles after the sequence
//!   start).
//! - Dendy uses the PAL frame-counter, noise and DMC tables (the
//!   wiki documents no separate Dendy tables; the PAL-family chip is
//!   the documented assumption).
//! - The noise LFSR powers up as 1 (per "APU Noise"; the power-up page
//!   itself marks its row uncertain).
//! - Triangle/noise/DMC timers run on the APU half-cycles with a phase
//!   anchored at power-on; the `$4017` write reset honors the
//!   documented 3-or-4-cycle delay using that phase.

use crate::timing::Region;

/// FNV-1a 64-bit offset basis (public domain algorithm).
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The published length-counter reload table ("APU Length Counter";
/// identical to the table inside the category-B `apu_test` source).
const LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22,
    192, 24, 72, 26, 16, 28, 32, 30,
];

/// The four duty sequencer bit patterns ("APU Pulse"), position 0..7.
const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 0, 0, 0],
    [1, 0, 0, 1, 1, 1, 1, 1],
];

/// The 32-step triangle sequencer ("APU Triangle").
const TRIANGLE_SEQ: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    13, 14, 15,
];

/// Noise timer periods in CPU cycles, NTSC ("APU Noise"). The timer
/// itself counts APU half-cycles, so the reload value is half of the
/// table entry (entries are all even).
const NOISE_PERIODS_NTSC: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];
/// Noise timer periods in CPU cycles, PAL (also used for Dendy).
const NOISE_PERIODS_PAL: [u16; 16] = [
    4, 8, 14, 30, 60, 88, 118, 148, 188, 236, 354, 472, 708, 944, 1890, 3778,
];

/// DMC output-unit timer rates in CPU cycles, NTSC ("APU DMC").
const DMC_RATES_NTSC: [u16; 16] = [
    428, 380, 340, 320, 286, 254, 226, 214, 190, 160, 142, 128, 106, 84, 72, 54,
];
/// DMC output-unit timer rates in CPU cycles, PAL (also Dendy).
const DMC_RATES_PAL: [u16; 16] = [
    398, 354, 316, 298, 276, 236, 210, 198, 176, 148, 132, 118, 98, 78, 66, 50,
];

/// The frame-sequencer clock points in CPU cycles: the quarters
/// (envelopes and the triangle linear counter), the halves (length
/// counters and sweeps), the first IRQ set point and the wrap, for the
/// 4-step and 5-step sequences of both timing families ("APU Frame
/// Counter" tables, APU cycles doubled). Dendy uses the PAL family
/// (documented assumption, D-39).
struct FrameTable {
    quarters: [u32; 5],
    halfes: [u32; 2],
    /// CPU cycles of the first frame-IRQ set point (4-step only).
    irq_set: Option<u32>,
    /// CPU cycles after which the sequence wraps to zero.
    wrap: u32,
}

fn frame_table(region: Region, mode5: bool) -> FrameTable {
    match (region, mode5) {
        (Region::Ntsc, false) => FrameTable {
            quarters: [7457, 14913, 22371, 29829, 0],
            halfes: [14913, 29829],
            irq_set: Some(29828),
            wrap: 29830,
        },
        (Region::Ntsc, true) => FrameTable {
            quarters: [7457, 14913, 22371, 37281, 0],
            halfes: [14913, 37281],
            irq_set: None,
            wrap: 37282,
        },
        (Region::Pal | Region::Dendy, false) => FrameTable {
            quarters: [8313, 16627, 24939, 33253, 0],
            halfes: [16627, 33253],
            irq_set: Some(33252),
            wrap: 33254,
        },
        (Region::Pal | Region::Dendy, true) => FrameTable {
            quarters: [8313, 16627, 24939, 41565, 0],
            halfes: [16627, 41565],
            irq_set: None,
            wrap: 41566,
        },
    }
}

/// TND common-denominator coefficients: the wiki TND term is
/// `t/8227 + n/12241 + d/22638`, i.e. the numerator
/// `t*(12241*22638) + n*(8227*22638) + d*(8227*12241)` over
/// `8227*12241*22638`. `159.79 = 15979/100`.
const TND_T: u64 = 12_241 * 22_638;
const TND_N: u64 = 8227 * 22_638;
const TND_D: u64 = 8227 * 12_241;
const TND_DENOM: u64 = 8227 * 12_241 * 22_638;

/// The pulse mix table in Q15: `95.88 / (8128/p + 100)` for the summed
/// pulse output `p = 0..30`, zero when the sum is zero ("APU Mixer":
/// the zero group is treated as zero). Computed by a const fn with
/// integer arithmetic only — the table is identical on every platform.
const fn pulse_mix_table() -> [i32; 31] {
    let mut table = [0i32; 31];
    let mut p = 1u32;
    while p <= 30 {
        // pulse_out = 95.88 * p / (8128 + 100*p) = 9588*p / (100*(8128+100*p))
        let p128 = p as u128;
        let num = 9588u128 * p128 * 32_768u128;
        let den = 100u128 * (8128u128 + 100u128 * p128);
        table[p as usize] = ((num + den / 2) / den) as i32;
        p += 1;
    }
    table
}

/// The committed pulse mix table (Q15).
const PULSE_MIX: [i32; 31] = pulse_mix_table();

/// The shared envelope generator ("APU Envelope").
#[derive(Clone, Copy, Debug)]
struct Envelope {
    /// Length-counter halt flag / envelope loop flag (bit 5).
    halt: bool,
    /// Constant-volume flag (bit 4).
    constant: bool,
    /// Volume / envelope divider reload (bits 3-0).
    volume: u8,
    /// The envelope start flag (set by a fourth-register write).
    start: bool,
    /// The divider countdown.
    divider: u8,
    /// The decay level (the envelope output when not constant).
    decay: u8,
}

impl Envelope {
    const fn new() -> Self {
        Envelope {
            halt: false,
            constant: false,
            volume: 0,
            start: false,
            divider: 0,
            decay: 0,
        }
    }

    /// The volume the mixer receives.
    fn output(&self) -> u8 {
        if self.constant {
            self.volume
        } else {
            self.decay
        }
    }

    /// Quarter-frame clock ("APU Envelope", verbatim order):
    /// start set -> clear it, reload the decay with 15 and the divider
    /// with the volume; otherwise clock the divider, which reloads
    /// with the volume at zero and then clocks the decay level.
    fn clock(&mut self) {
        if self.start {
            self.start = false;
            self.decay = 15;
            self.divider = self.volume;
        } else if self.divider == 0 {
            self.divider = self.volume;
            if self.decay > 0 {
                self.decay -= 1;
            } else if self.halt {
                self.decay = 15;
            }
        } else {
            self.divider -= 1;
        }
    }

    fn state_bytes(&self, out: &mut Vec<u8>) {
        out.push(u8::from(self.halt) << 4 | u8::from(self.constant) << 3 | self.volume);
        out.push(u8::from(self.start) << 2 | self.divider);
        out.push(self.decay);
    }
}

/// One pulse channel ("APU Pulse" + "APU Sweep").
#[derive(Clone, Copy, Debug)]
struct Pulse {
    /// True for pulse 1 ($4000-$4003): its sweep negate uses the
    /// one's complement, pulse 2 the two's complement.
    first: bool,
    /// Duty index (2 bits).
    duty: u8,
    /// The envelope / length-halt unit.
    env: Envelope,
    /// Sweep enabled flag (bit 7 of $4001).
    sweep_enabled: bool,
    /// Sweep divider period field (bits 6-4): the divider counts
    /// P..0, so the period is P+1 half-frames.
    sweep_period: u8,
    /// Sweep negate flag (bit 3).
    sweep_negate: bool,
    /// Sweep shift count (bits 2-0).
    sweep_shift: u8,
    /// The sweep reload flag (set by $4001 writes).
    sweep_reload: bool,
    /// The sweep divider countdown.
    sweep_divider: u8,
    /// The 11-bit timer period (HHHLLLLLLLL, loaded as period+1 ticks).
    timer_period: u16,
    /// The timer countdown.
    timer: u16,
    /// The duty sequencer position (0..7).
    seq_pos: u8,
    /// The length counter.
    length: u8,
}

impl Pulse {
    const fn new(first: bool) -> Self {
        Pulse {
            first,
            duty: 0,
            env: Envelope::new(),
            sweep_enabled: false,
            sweep_period: 0,
            sweep_negate: false,
            sweep_shift: 0,
            sweep_reload: false,
            sweep_divider: 0,
            timer_period: 0,
            timer: 0,
            seq_pos: 0,
            length: 0,
        }
    }

    /// The sweep target period: `P + (P >> n)`, or with the negate
    /// flag the one's complement on pulse 1 (`P - (P >> n) - 1`) and
    /// the two's complement on pulse 2 (`P - (P >> n)`).
    fn sweep_target(&self) -> u16 {
        let period = self.timer_period;
        let delta = period >> self.sweep_shift;
        if self.sweep_negate {
            if self.is_pulse1() {
                period.wrapping_sub(delta).wrapping_sub(1)
            } else {
                period.wrapping_sub(delta)
            }
        } else {
            period.wrapping_add(delta)
        }
    }

    /// True when this is pulse 1 (the $4000-$4003 instance).
    fn is_pulse1(&self) -> bool {
        self.first
    }

    /// The sweep unit mutes the channel while the current period is
    /// below 8 or the target exceeds $7FF ("APU Sweep": muting happens
    /// regardless of whether the sweep unit is enabled).
    fn sweep_muted(&self) -> bool {
        self.timer_period < 8 || self.sweep_target() > 0x7FF
    }

    /// The value the mixer receives: the envelope volume except when
    /// the sequencer output is zero, the sweep unit is silencing the
    /// channel, the length counter is zero, or the timer period is
    /// below 8 ("APU Pulse").
    fn output(&self) -> u8 {
        if DUTY_TABLE[self.duty as usize][self.seq_pos as usize] == 0
            || self.sweep_muted()
            || self.length == 0
            || self.timer_period < 8
        {
            0
        } else {
            self.env.output()
        }
    }

    /// APU-cycle timer tick: counts the period down to zero and clocks
    /// the duty sequencer on the zero-to-period reload.
    fn tick_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            self.seq_pos = (self.seq_pos + 1) & 7;
        } else {
            self.timer -= 1;
        }
    }

    /// Half-frame clock: the sweep unit, then the length counter.
    fn clock_half(&mut self) {
        // "APU Sweep", verbatim order: with the divider at zero the
        // period follows the target (only when enabled, shifted and
        // not muting), then the divider reloads at zero or on the
        // reload flag, otherwise it decrements.
        let target = self.sweep_target();
        let muted = self.timer_period < 8 || target > 0x7FF;
        if self.sweep_divider == 0 && self.sweep_enabled && self.sweep_shift > 0 && !muted {
            self.timer_period = target;
        }
        if self.sweep_divider == 0 || self.sweep_reload {
            self.sweep_divider = self.sweep_period;
            self.sweep_reload = false;
        } else {
            self.sweep_divider -= 1;
        }
        if !self.env.halt && self.length > 0 {
            self.length -= 1;
        }
    }

    fn write_ctrl(&mut self, value: u8) {
        self.duty = (value >> 6) & 3;
        self.env.halt = value & 0x20 != 0;
        self.env.constant = value & 0x10 != 0;
        self.env.volume = value & 0x0F;
    }

    fn write_sweep(&mut self, value: u8) {
        self.sweep_enabled = value & 0x80 != 0;
        self.sweep_period = (value >> 4) & 7;
        self.sweep_negate = value & 0x08 != 0;
        self.sweep_shift = value & 7;
        // Side effect: the reload flag is set.
        self.sweep_reload = true;
    }

    fn write_timer_lo(&mut self, value: u8) {
        self.timer_period = (self.timer_period & 0x700) | u16::from(value);
    }

    /// The fourth-register write: timer high bits, the length reload
    /// (gated on the channel's `$4015` enable — the category-B
    /// `apu_test` rule), the duty sequencer restart and the envelope
    /// restart. The timer countdown itself is not reset ("APU
    /// Pulse").
    fn write_timer_hi(&mut self, value: u8, enabled: bool) {
        self.timer_period = (self.timer_period & 0xFF) | (u16::from(value & 7) << 8);
        if enabled {
            self.length = LENGTH_TABLE[usize::from(value >> 3 & 31)];
        }
        self.seq_pos = 0;
        self.env.start = true;
    }

    fn state_bytes(&self, out: &mut Vec<u8>) {
        out.push(u8::from(self.first) << 6 | self.duty << 5);
        self.env.state_bytes(out);
        let sweep = u8::from(self.sweep_enabled) << 7
            | self.sweep_period << 4
            | u8::from(self.sweep_negate) << 3
            | self.sweep_shift;
        out.push(sweep);
        out.push(u8::from(self.sweep_reload) << 7 | self.sweep_divider);
        out.extend_from_slice(&self.timer_period.to_le_bytes());
        out.extend_from_slice(&self.timer.to_le_bytes());
        out.push(self.seq_pos);
        out.push(self.length);
    }
}

/// The triangle channel ("APU Triangle").
#[derive(Clone, Copy, Debug)]
struct Triangle {
    /// Control flag ($4008 bit 7): the length halt and the linear
    /// counter reload-flag gate.
    control: bool,
    /// The linear counter reload value (bits 6-0).
    linear_reload_value: u8,
    /// The linear counter reload flag (set by $4008/$400B writes).
    linear_reload: bool,
    /// The linear counter.
    linear_counter: u8,
    /// The 11-bit timer period.
    timer_period: u16,
    /// The timer countdown (ticks at the CPU rate).
    timer: u16,
    /// The sequencer position (0..31).
    seq_pos: u8,
    /// The length counter.
    length: u8,
}

impl Triangle {
    const fn new() -> Self {
        Triangle {
            control: false,
            linear_reload_value: 0,
            linear_reload: false,
            linear_counter: 0,
            timer_period: 0,
            timer: 0,
            seq_pos: 0,
            length: 0,
        }
    }

    /// The sequencer value (the triangle has no volume control).
    fn output(&self) -> u8 {
        TRIANGLE_SEQ[self.seq_pos as usize]
    }

    /// CPU-cycle timer tick: the sequencer is clocked on reload only
    /// while both the linear counter and the length counter are
    /// nonzero.
    fn tick_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            if self.linear_counter > 0 && self.length > 0 {
                self.seq_pos = (self.seq_pos + 1) & 31;
            }
        } else {
            self.timer -= 1;
        }
    }

    /// Quarter-frame clock of the linear counter ("APU Triangle",
    /// verbatim order).
    fn clock_quarter(&mut self) {
        if self.linear_reload {
            self.linear_counter = self.linear_reload_value;
        } else if self.linear_counter > 0 {
            self.linear_counter -= 1;
        }
        if !self.control {
            self.linear_reload = false;
        }
    }

    /// Half-frame length clock.
    fn clock_half(&mut self) {
        if !self.control && self.length > 0 {
            self.length -= 1;
        }
    }

    fn write_linear(&mut self, value: u8) {
        self.control = value & 0x80 != 0;
        self.linear_reload_value = value & 0x7F;
        // Side effect: the reload flag is set.
        self.linear_reload = true;
    }

    fn write_timer_lo(&mut self, value: u8) {
        self.timer_period = (self.timer_period & 0x700) | u16::from(value);
    }

    fn write_timer_hi(&mut self, value: u8, enabled: bool) {
        self.timer_period = (self.timer_period & 0xFF) | (u16::from(value & 7) << 8);
        if enabled {
            self.length = LENGTH_TABLE[usize::from(value >> 3 & 31)];
        }
        // Side effect: the linear counter reload flag is set.
        self.linear_reload = true;
    }

    fn state_bytes(&self, out: &mut Vec<u8>) {
        out.push(u8::from(self.control) << 7 | self.linear_reload_value);
        out.push(u8::from(self.linear_reload) << 7 | self.linear_counter);
        out.extend_from_slice(&self.timer_period.to_le_bytes());
        out.extend_from_slice(&self.timer.to_le_bytes());
        out.push(self.seq_pos);
        out.push(self.length);
    }
}

/// The noise channel ("APU Noise").
#[derive(Clone, Copy, Debug)]
struct Noise {
    env: Envelope,
    /// The mode flag ($400E bit 7): bit 6 instead of bit 1 in the
    /// feedback.
    mode: bool,
    /// The timer reload in APU half-cycles (table entry / 2 - 1).
    timer_period: u16,
    /// The timer countdown.
    timer: u16,
    /// The 15-bit LFSR (powers up as 1).
    lfsr: u16,
    /// The length counter.
    length: u8,
}

impl Noise {
    const fn new() -> Self {
        Noise {
            env: Envelope::new(),
            mode: false,
            timer_period: 0,
            timer: 0,
            lfsr: 1,
            length: 0,
        }
    }

    /// The value the mixer receives: the envelope volume except when
    /// LFSR bit 0 is set or the length counter is zero.
    fn output(&self) -> u8 {
        if self.lfsr & 1 != 0 || self.length == 0 {
            0
        } else {
            self.env.output()
        }
    }

    /// APU-cycle timer tick: on reload the LFSR shifts right by one
    /// with the feedback (bit 0 xor bit 6 or bit 1) entering bit 14.
    fn tick_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            let feedback = (self.lfsr & 1)
                ^ if self.mode {
                    (self.lfsr >> 6) & 1
                } else {
                    (self.lfsr >> 1) & 1
                };
            self.lfsr = (self.lfsr >> 1) | (feedback << 14);
        } else {
            self.timer -= 1;
        }
    }

    /// Half-frame length clock.
    fn clock_half(&mut self) {
        if !self.env.halt && self.length > 0 {
            self.length -= 1;
        }
    }

    fn write_ctrl(&mut self, value: u8) {
        self.env.halt = value & 0x20 != 0;
        self.env.constant = value & 0x10 != 0;
        self.env.volume = value & 0x0F;
    }

    fn write_period(&mut self, value: u8, region: Region) {
        self.mode = value & 0x80 != 0;
        let table = match region {
            Region::Ntsc => &NOISE_PERIODS_NTSC,
            Region::Pal | Region::Dendy => &NOISE_PERIODS_PAL,
        };
        let entry = table[usize::from(value & 0x0F)];
        // The table is in CPU cycles; the timer counts APU half-cycles.
        self.timer_period = entry / 2 - 1;
    }

    fn write_length(&mut self, value: u8, enabled: bool) {
        if enabled {
            self.length = LENGTH_TABLE[usize::from(value >> 3 & 31)];
        }
        self.env.start = true;
    }

    fn state_bytes(&self, out: &mut Vec<u8>) {
        self.env.state_bytes(out);
        out.push(u8::from(self.mode) << 7);
        out.extend_from_slice(&self.timer_period.to_le_bytes());
        out.extend_from_slice(&self.timer.to_le_bytes());
        out.extend_from_slice(&self.lfsr.to_le_bytes());
        out.push(self.length);
    }
}

/// The delta modulation channel ("APU DMC").
#[derive(Clone, Copy, Debug)]
struct Dmc {
    /// IRQ enabled flag ($4010 bit 7; clearing it clears the flag).
    irq_enable: bool,
    /// Loop flag ($4010 bit 6).
    loop_sample: bool,
    /// Rate index ($4010 bits 3-0).
    rate_idx: u8,
    /// The output-unit timer reload in APU half-cycles (rate / 2 - 1).
    timer_period: u16,
    /// The timer countdown.
    timer: u16,
    /// The 7-bit output level (sent to the mixer whether the channel
    /// is enabled or not; powers up as 0).
    level: u8,
    /// The output-unit silence flag.
    silence: bool,
    /// The bits-remaining counter of the output unit.
    bits_remaining: u8,
    /// The output-unit right shift register.
    shift_reg: u8,
    /// The one-byte sample buffer.
    sample_buffer: u8,
    /// True when the sample buffer is empty.
    buffer_empty: bool,
    /// The memory reader's current address counter.
    addr: u16,
    /// The sample start address from $4012 ($C000 + A * 64).
    sample_addr: u16,
    /// The memory reader's bytes-remaining counter.
    bytes_remaining: u16,
    /// The sample length from $4013 ((L * 16) + 1 bytes).
    sample_len: u16,
    /// The DMC interrupt flag (asserts the CPU IRQ line until cleared).
    irq_flag: bool,
    /// True when the memory reader wants a fetch (the CPU stall).
    dma_pending: bool,
}

impl Dmc {
    const fn new() -> Self {
        Dmc {
            irq_enable: false,
            loop_sample: false,
            rate_idx: 0,
            timer_period: 0,
            timer: 0,
            level: 0,
            silence: true,
            bits_remaining: 8,
            shift_reg: 0,
            sample_buffer: 0,
            buffer_empty: true,
            addr: 0,
            sample_addr: 0,
            bytes_remaining: 0,
            sample_len: 0,
            irq_flag: false,
            dma_pending: false,
        }
    }

    /// APU-cycle timer tick: the output unit. Verbatim order — if not
    /// silenced, the level moves by delta 2 within 0..127 following
    /// shift-register bit 0; the shift register is clocked; then the
    /// bits-remaining counter is decremented and a new output cycle
    /// starts at zero (re-checking the buffer).
    fn tick_timer(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            if !self.silence {
                if self.shift_reg & 1 != 0 {
                    if self.level <= 125 {
                        self.level += 2;
                    }
                } else if self.level >= 2 {
                    self.level -= 2;
                }
            }
            self.shift_reg >>= 1;
            if self.bits_remaining > 0 {
                self.bits_remaining -= 1;
            }
            if self.bits_remaining == 0 {
                self.bits_remaining = 8;
                if self.buffer_empty {
                    self.silence = true;
                } else {
                    self.silence = false;
                    self.shift_reg = self.sample_buffer;
                    self.buffer_empty = true;
                }
            }
        } else {
            self.timer -= 1;
        }
    }

    /// The memory reader wants a fetch whenever the buffer is empty
    /// and bytes remain.
    fn needs_fetch(&self) -> bool {
        self.buffer_empty && self.bytes_remaining > 0
    }

    /// The fetch completed with `value`: the buffer is filled, the
    /// address advances (wrapping $FFFF to $8000), the bytes counter
    /// is decremented and at zero either the loop restarts the sample
    /// or the IRQ flag is set ("APU DMC").
    fn dma_complete(&mut self, value: u8) {
        self.sample_buffer = value;
        self.buffer_empty = false;
        self.dma_pending = false;
        // The address wraps around to $8000 past $FFFF.
        self.addr = self.addr.wrapping_add(1);
        if self.addr == 0 {
            self.addr = 0x8000;
        }
        self.bytes_remaining -= 1;
        if self.bytes_remaining == 0 {
            if self.loop_sample {
                self.addr = self.sample_addr;
                self.bytes_remaining = self.sample_len;
            } else if self.irq_enable {
                self.irq_flag = true;
            }
        }
    }

    fn write_flags(&mut self, value: u8, region: Region) {
        self.irq_enable = value & 0x80 != 0;
        if !self.irq_enable {
            self.irq_flag = false;
        }
        self.loop_sample = value & 0x40 != 0;
        self.rate_idx = value & 0x0F;
        let table = match region {
            Region::Ntsc => &DMC_RATES_NTSC,
            Region::Pal | Region::Dendy => &DMC_RATES_PAL,
        };
        let rate = table[usize::from(self.rate_idx)];
        // The rate table is in CPU cycles; the timer counts APU
        // half-cycles ("a rate of 428 means the output level changes
        // every 214 APU cycles").
        self.timer_period = rate / 2 - 1;
    }

    fn state_bytes(&self, out: &mut Vec<u8>) {
        out.push(u8::from(self.irq_enable) << 7 | u8::from(self.loop_sample) << 6 | self.rate_idx);
        out.extend_from_slice(&self.timer_period.to_le_bytes());
        out.extend_from_slice(&self.timer.to_le_bytes());
        out.push(self.level);
        out.push(
            u8::from(self.silence) << 7
                | u8::from(self.buffer_empty) << 6
                | u8::from(self.irq_flag) << 5
                | u8::from(self.dma_pending) << 4
                | self.bits_remaining & 0x0F,
        );
        out.push(self.shift_reg);
        out.push(self.sample_buffer);
        out.extend_from_slice(&self.addr.to_le_bytes());
        out.extend_from_slice(&self.sample_addr.to_le_bytes());
        out.extend_from_slice(&self.bytes_remaining.to_le_bytes());
        out.extend_from_slice(&self.sample_len.to_le_bytes());
    }
}

/// The complete APU.
#[derive(Clone, Debug)]
pub struct Apu {
    region: Region,
    /// The two pulse channels: [pulse 1, pulse 2].
    pulse: [Pulse; 2],
    tri: Triangle,
    noise: Noise,
    dmc: Dmc,
    /// `$4015` channel enable bits: pulse 1/2, triangle, noise, DMC.
    enable: [bool; 5],
    /// CPU cycles since the frame sequence started.
    frame_cycle: u32,
    /// `$4017` bit 7: the 5-step sequence.
    frame_mode5: bool,
    /// `$4017` bit 6: the IRQ inhibit flag.
    frame_irq_inhibit: bool,
    /// The frame interrupt flag.
    frame_irq_flag: bool,
    /// The pending sequencer reset of a `$4017` write (3 or 4 CPU
    /// cycles after the write).
    frame_reset_countdown: Option<u8>,
    /// The APU half-cycle phase (toggles every CPU cycle; the
    /// pulse/noise/DMC timers advance on the active half).
    half: bool,
    /// DC blocker state: the previous input and output (Q15).
    dc_x: i64,
    dc_y: i64,
    /// PCM capture (only when enabled; the hash is always kept).
    capturing: bool,
    pcm: Vec<i16>,
    /// FNV-1a of every produced sample (little-endian bytes).
    pcm_hash: u64,
    /// Total produced samples (one per CPU cycle).
    samples: u64,
}

impl Apu {
    /// Power-on state ("CPU power up state": `$4015` = 0 silencing all
    /// channels, `$4017` = 0 with the frame IRQ therefore enabled; the
    /// noise LFSR is 1, the DMC level 0, per the channel pages).
    pub fn new(region: Region) -> Self {
        let mut noise = Noise::new();
        // Rate index 0 of the region's table (mode clear).
        noise.write_period(0x00, region);
        let mut dmc = Dmc::new();
        // Rate index 0 of the region's table (no IRQ, no loop).
        dmc.write_flags(0x00, region);
        Apu {
            region,
            pulse: [Pulse::new(true), Pulse::new(false)],
            tri: Triangle::new(),
            noise,
            dmc,
            enable: [false; 5],
            frame_cycle: 0,
            frame_mode5: false,
            frame_irq_inhibit: false,
            frame_irq_flag: false,
            frame_reset_countdown: None,
            half: false,
            dc_x: 0,
            dc_y: 0,
            capturing: false,
            pcm: Vec::new(),
            pcm_hash: FNV_OFFSET_BASIS,
            samples: 0,
        }
    }

    /// Reset effect ("APU": power-up and reset have the effect of
    /// writing $00 to $4015; the power-up table adds triangle phase 0
    /// and DMC level &= 1; the frame-counter register is unchanged).
    pub fn reset(&mut self) {
        self.enable = [false; 5];
        self.pulse[0].length = 0;
        self.pulse[1].length = 0;
        self.tri.length = 0;
        self.tri.seq_pos = 0;
        self.noise.length = 0;
        self.dmc.bytes_remaining = 0;
        self.dmc.irq_flag = false;
        self.dmc.level &= 1;
    }

    /// The current APU half-cycle phase (for the `$4017` write delay).
    pub fn half_phase(&self) -> bool {
        self.half
    }

    /// The CPU IRQ level: the frame flag or the DMC flag, asserted
    /// continuously until cleared.
    pub fn irq_level(&self) -> bool {
        self.frame_irq_flag || self.dmc.irq_flag
    }

    /// One CPU cycle of the APU. Called by the bus after the PPU dots
    /// of the same cycle (spec §5.0.1 order: CPU, PPU, APU).
    pub fn tick(&mut self) {
        // 1. A pending $4017 write resets the sequencer 3 or 4 CPU
        //    cycles after the write ("APU Frame Counter"); a mode-1
        //    write also generates the quarter and half signals.
        if let Some(mut left) = self.frame_reset_countdown {
            left -= 1;
            if left == 0 {
                self.frame_reset_countdown = None;
                self.frame_cycle = 0;
                if self.frame_mode5 {
                    self.clock_quarter();
                    self.clock_half();
                }
            } else {
                self.frame_reset_countdown = Some(left);
            }
        }

        // 2. The frame sequencer.
        self.frame_cycle += 1;
        let table = frame_table(self.region, self.frame_mode5);
        for &q in table.quarters.iter() {
            if q != 0 && self.frame_cycle == q {
                self.clock_quarter();
            }
        }
        for &h in table.halfes.iter() {
            if self.frame_cycle == h {
                self.clock_half();
            }
        }
        if let Some(irq_at) = table.irq_set {
            if self.frame_cycle == irq_at && !self.frame_irq_inhibit {
                self.frame_irq_flag = true;
            }
        }
        if self.frame_cycle >= table.wrap {
            self.frame_cycle = 0;
        }

        // 3. The APU half-cycle phase: the APU-rate timers advance on
        //    every second CPU cycle.
        self.half = !self.half;

        // 4. The triangle timer runs at the CPU rate.
        self.tri.tick_timer();

        // 5. The pulse/noise/DMC timers run at the APU rate.
        if self.half {
            self.pulse[0].tick_timer();
            self.pulse[1].tick_timer();
            self.noise.tick_timer();
            self.dmc.tick_timer();
        }

        // 6. The DMC memory reader wants a fetch whenever the buffer
        //    is empty and bytes remain (the bus performs the stall).
        if self.dmc.needs_fetch() {
            self.dmc.dma_pending = true;
        }

        // 7. The mixer: one sample per CPU cycle (the native rate).
        let sample = self.mix_sample();
        self.push_sample(sample);
    }

    /// Quarter-frame signals: the envelopes and the triangle linear
    /// counter.
    fn clock_quarter(&mut self) {
        self.pulse[0].env.clock();
        self.pulse[1].env.clock();
        self.noise.env.clock();
        self.tri.clock_quarter();
    }

    /// Half-frame signals: the length counters and the sweep units.
    fn clock_half(&mut self) {
        self.pulse[0].clock_half();
        self.pulse[1].clock_half();
        self.tri.clock_half();
        self.noise.clock_half();
    }

    /// The integer mixer in Q15 ("APU Mixer"):
    /// `pulse_out = 95.88 / (8128/(p1+p2) + 100)` and
    /// `tnd_out = 159.79 / (1/S + 100)` with
    /// `S = t/8227 + n/12241 + d/22638`; a zero group contributes
    /// zero; the sum stays within 0..1. Followed by the DC blocker
    /// (first-order high-pass, pole 65535/65536).
    fn mix_sample(&mut self) -> i16 {
        let p1 = u64::from(self.pulse[0].output());
        let p2 = u64::from(self.pulse[1].output());
        let t = u64::from(self.tri.output());
        let n = u64::from(self.noise.output());
        let d = u64::from(self.dmc.level);
        let pulse_q15 = PULSE_MIX[usize::from((p1 + p2) as u16)];
        // tnd_out = 159.79*S / (1 + 100*S) = 15979*N / (100*(D + 100*N))
        // with N the common-denominator numerator over D.
        let num_tnd = t * TND_T + n * TND_N + d * TND_D;
        let den_tnd = 100u128 * (u128::from(TND_DENOM) + 100u128 * u128::from(num_tnd));
        let numerator = 15_979u128 * u128::from(num_tnd) * 32_768u128 + den_tnd / 2;
        let tnd_q15 = i32::try_from(numerator / den_tnd).unwrap_or(i32::MAX);
        let total_q15 = pulse_q15 + tnd_q15;

        // DC blocker in Q15: y = x - x_prev + (y_prev * 65535 >> 16).
        let x = i64::from(total_q15);
        let y = x - self.dc_x + ((self.dc_y * 65_535) >> 16);
        self.dc_x = x;
        self.dc_y = y;
        y.clamp(-32_767, 32_767) as i16
    }

    /// Record one output sample: the hash is always kept (the PCM
    /// fingerprint of the run), the bytes only while capturing.
    fn push_sample(&mut self, sample: i16) {
        self.samples += 1;
        let mut hash = self.pcm_hash;
        for byte in sample.to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME);
        }
        self.pcm_hash = hash;
        if self.capturing {
            self.pcm.push(sample);
        }
    }

    /// The `$4015` write ("APU"): channel enables (a zero bit clears
    /// the length counter immediately), the DMC stop/restart rule and
    /// the DMC interrupt flag clear.
    pub fn write_status(&mut self, value: u8) {
        self.enable[0] = value & 0x01 != 0;
        self.enable[1] = value & 0x02 != 0;
        self.enable[2] = value & 0x04 != 0;
        self.enable[3] = value & 0x08 != 0;
        self.enable[4] = value & 0x10 != 0;
        if !self.enable[0] {
            self.pulse[0].length = 0;
        }
        if !self.enable[1] {
            self.pulse[1].length = 0;
        }
        if !self.enable[2] {
            self.tri.length = 0;
        }
        if !self.enable[3] {
            self.noise.length = 0;
        }
        if self.enable[4] {
            // Restart only when the bytes counter is at zero.
            if self.dmc.bytes_remaining == 0 {
                self.dmc.addr = self.dmc.sample_addr;
                self.dmc.bytes_remaining = self.dmc.sample_len;
            }
        } else {
            self.dmc.bytes_remaining = 0;
        }
        self.dmc.irq_flag = false;
    }

    /// The `$4015` read ("APU"): the length/DMC status bits and the
    /// two interrupt flags; reading clears the frame flag.
    pub fn read_status(&mut self) -> u8 {
        let mut value = 0u8;
        if self.pulse[0].length > 0 {
            value |= 0x01;
        }
        if self.pulse[1].length > 0 {
            value |= 0x02;
        }
        if self.tri.length > 0 {
            value |= 0x04;
        }
        if self.noise.length > 0 {
            value |= 0x08;
        }
        if self.dmc.bytes_remaining > 0 {
            value |= 0x10;
        }
        if self.frame_irq_flag {
            value |= 0x40;
        }
        if self.dmc.irq_flag {
            value |= 0x80;
        }
        self.frame_irq_flag = false;
        value
    }

    /// The `$4017` write: the mode and IRQ inhibit bits and the
    /// sequencer reset 3 or 4 CPU cycles later (the delay rule uses
    /// the APU half-cycle phase: a write during an APU cycle resets
    /// after 3, between APU cycles after 4). Setting the inhibit flag
    /// clears the frame flag.
    pub fn write_frame_counter(&mut self, value: u8, write_during_apu_cycle: bool) {
        self.frame_mode5 = value & 0x80 != 0;
        self.frame_irq_inhibit = value & 0x40 != 0;
        if self.frame_irq_inhibit {
            self.frame_irq_flag = false;
        }
        self.frame_reset_countdown = Some(if write_during_apu_cycle { 3 } else { 4 });
    }

    /// Register writes for `$4000-$4013`.
    pub fn write_register(&mut self, addr: u16, value: u8) {
        match addr {
            0x4000 => self.pulse[0].write_ctrl(value),
            0x4001 => self.pulse[0].write_sweep(value),
            0x4002 => self.pulse[0].write_timer_lo(value),
            0x4003 => self.pulse[0].write_timer_hi(value, self.enable[0]),
            0x4004 => self.pulse[1].write_ctrl(value),
            0x4005 => self.pulse[1].write_sweep(value),
            0x4006 => self.pulse[1].write_timer_lo(value),
            0x4007 => self.pulse[1].write_timer_hi(value, self.enable[1]),
            0x4008 => self.tri.write_linear(value),
            // $4009 is unused.
            0x400A => self.tri.write_timer_lo(value),
            0x400B => self.tri.write_timer_hi(value, self.enable[2]),
            0x400C => self.noise.write_ctrl(value),
            // $400D is unused.
            0x400E => self.noise.write_period(value, self.region),
            0x400F => self.noise.write_length(value, self.enable[3]),
            0x4010 => self.dmc.write_flags(value, self.region),
            0x4011 => self.dmc.level = value & 0x7F,
            0x4012 => {
                self.dmc.sample_addr = 0xC000 | (u16::from(value) << 6);
            }
            0x4013 => {
                self.dmc.sample_len = u16::from(value) * 16 + 1;
            }
            _ => {}
        }
    }

    /// True when the memory reader wants a DMA fetch (the bus stalls
    /// the CPU and reads the byte).
    pub fn dma_request(&self) -> bool {
        self.dmc.dma_pending
    }

    /// Take the fetch request, returning the address to read.
    pub fn take_dma_request(&mut self) -> Option<u16> {
        if self.dmc.dma_pending {
            self.dmc.dma_pending = false;
            Some(self.dmc.addr)
        } else {
            None
        }
    }

    /// Complete the fetch with the byte read by the bus.
    pub fn dma_complete(&mut self, value: u8) {
        self.dmc.dma_complete(value);
    }

    /// Enable or disable PCM byte capture (the fingerprint is computed
    /// regardless).
    pub fn set_capture(&mut self, on: bool) {
        self.capturing = on;
        if !on {
            self.pcm.clear();
        }
    }

    /// Drain the captured samples.
    pub fn take_pcm(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.pcm)
    }

    /// The PCM fingerprint (FNV-1a over every produced sample).
    pub fn pcm_hash(&self) -> u64 {
        self.pcm_hash
    }

    /// Total produced samples (one per CPU cycle since power-on).
    pub fn samples(&self) -> u64 {
        self.samples
    }

    /// The complete APU state as bytes, for the machine state
    /// fingerprint: every field that shapes future behavior (the PCM
    /// buffer is an output artifact and is deliberately excluded, as
    /// is the always-on sample hash).
    pub fn state_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(96);
        let enables = self
            .enable
            .iter()
            .enumerate()
            .fold(0u8, |acc, (i, &on)| acc | (u8::from(on) << i));
        out.push(enables);
        out.extend_from_slice(&self.frame_cycle.to_le_bytes());
        let frame = u8::from(self.frame_mode5) << 7
            | u8::from(self.frame_irq_inhibit) << 6
            | u8::from(self.frame_irq_flag) << 5
            | u8::from(self.half) << 4
            | u8::from(self.dmc.irq_flag) << 3
            | u8::from(self.dmc.dma_pending) << 2;
        out.push(frame);
        out.push(self.frame_reset_countdown.unwrap_or(0));
        out.extend_from_slice(&self.dc_x.to_le_bytes());
        out.extend_from_slice(&self.dc_y.to_le_bytes());
        self.pulse[0].state_bytes(&mut out);
        self.pulse[1].state_bytes(&mut out);
        self.tri.state_bytes(&mut out);
        self.noise.state_bytes(&mut out);
        self.dmc.state_bytes(&mut out);
        out
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh APU with the four length-carried channels enabled.
    fn apu_all_enabled() -> Apu {
        let mut apu = Apu::new(Region::Ntsc);
        apu.write_status(0x0F);
        apu
    }

    #[test]
    fn length_table_spot_values_match_the_published_table() {
        // blargg's 2-len_table source, verbatim rows.
        assert_eq!(LENGTH_TABLE[0], 10);
        assert_eq!(LENGTH_TABLE[1], 254);
        assert_eq!(LENGTH_TABLE[2], 20);
        assert_eq!(LENGTH_TABLE[3], 2);
        assert_eq!(LENGTH_TABLE[15], 14);
        assert_eq!(LENGTH_TABLE[16], 12);
        assert_eq!(LENGTH_TABLE[31], 30);
    }

    #[test]
    fn fourth_register_write_reloads_length_only_when_enabled() {
        let mut apu = apu_all_enabled();
        // Index 3 of the table = 2.
        apu.write_register(0x4003, 3 << 3);
        assert_eq!(apu.pulse[0].length, 2, "enabled channel reloads");
        // Disable through $4015, then the write must not reload.
        apu.write_status(0x0E);
        assert_eq!(apu.pulse[0].length, 0, "disable clears the length");
        apu.write_register(0x4003, 3 << 3);
        assert_eq!(
            apu.pulse[0].length, 0,
            "no reload while disabled (category-B apu_test rule 7)"
        );
    }

    #[test]
    fn halt_flag_still_reloads_and_suspends_clocking() {
        // Category-B apu_test rule 8: with the halt flag set the
        // fourth-register write still reloads, and length clocking is
        // suspended.
        let mut apu = apu_all_enabled();
        apu.write_register(0x4000, 0x20 | 0x0F); // halt, constant volume 15
        apu.write_register(0x4003, 31 << 3);
        assert_eq!(apu.pulse[0].length, 30);
        for _ in 0..40_000 {
            apu.tick();
        }
        assert_eq!(apu.pulse[0].length, 30, "halt suspends clocking");
    }

    #[test]
    fn disable_via_4015_clears_length_immediately() {
        let mut apu = apu_all_enabled();
        apu.write_register(0x4003, 31 << 3);
        assert_eq!(apu.pulse[0].length, 30);
        apu.write_status(0x00);
        assert_eq!(apu.pulse[0].length, 0);
    }

    #[test]
    fn length_clocks_on_half_frames_and_reaches_zero() {
        let mut apu = apu_all_enabled();
        apu.write_register(0x4003, 3 << 3); // length 2
                                            // Two half-frames of the 4-step NTSC sequence (14913 and
                                            // 29829 CPU cycles) decrement it to zero.
        for _ in 0..30_000 {
            apu.tick();
        }
        assert_eq!(apu.pulse[0].length, 0, "expired through two half frames");
    }

    #[test]
    fn pulse_period_below_8_mutes_the_channel() {
        let mut apu = apu_all_enabled();
        apu.write_register(0x4000, 0x10 | 0x0F); // constant volume 15
        apu.write_register(0x4002, 0x05); // period 5 < 8
        apu.write_register(0x4003, 31 << 3);
        assert_eq!(apu.pulse[0].output(), 0, "timer period below 8 mutes");
        apu.write_register(0x4002, 0x20); // period 0x20
        apu.pulse[0].seq_pos = 1; // the "1" step of duty 0
        assert_ne!(apu.pulse[0].output(), 0, "an in-range period plays");
    }

    #[test]
    fn sweep_target_overflow_mutes_even_when_disabled() {
        // "APU Sweep": with negate clear, shift zero and a period of
        // at least $400 the target doubles past $7FF and the channel
        // is muted even though the sweep unit is disabled.
        let mut apu = apu_all_enabled();
        apu.write_register(0x4000, 0x10 | 0x0F);
        apu.write_register(0x4001, 0x00); // disabled, shift 0, negate 0
        apu.write_register(0x4002, 0x00);
        apu.write_register(0x4003, 0x04); // period 0x400
        assert_eq!(apu.pulse[0].output(), 0, "overflow target mutes");
    }

    #[test]
    fn sweep_negate_uses_ones_complement_on_pulse1() {
        let mut apu = apu_all_enabled();
        apu.pulse[0].timer_period = 0x0054;
        apu.pulse[0].sweep_negate = true;
        apu.pulse[0].sweep_shift = 1;
        // Pulse 1: 0x54 - (0x54 >> 1) - 1 = 0x54 - 0x2A - 1 = 0x29.
        assert_eq!(apu.pulse[0].sweep_target(), 0x29);
        // Pulse 2 with the same setup: 0x54 - 0x2A = 0x2A.
        apu.pulse[1].timer_period = 0x0054;
        apu.pulse[1].sweep_negate = true;
        apu.pulse[1].sweep_shift = 1;
        assert_eq!(apu.pulse[1].sweep_target(), 0x2A);
    }

    #[test]
    fn sweep_moves_the_period_at_half_frame_when_armed() {
        let mut apu = apu_all_enabled();
        apu.write_register(0x4004, 0x10 | 0x0F);
        // Enabled, period field 3 (reload at 3), negate, shift 1.
        apu.write_register(0x4005, 0x80 | (3 << 4) | 0x08 | 1);
        apu.write_register(0x4006, 0x54);
        apu.write_register(0x4007, 0x00);
        assert_eq!(apu.pulse[1].timer_period, 0x54);
        // The reload flag from the $4005 write holds the divider at
        // the period, so the first half-frame clock already moves the
        // timer to the target 0x2A.
        apu.pulse[1].clock_half();
        assert_eq!(apu.pulse[1].timer_period, 0x2A);
    }

    #[test]
    fn pulse_frequency_follows_the_documented_formula() {
        // f = CPU / (16 * (p + 1)) for p = 0xFD (period 253):
        // 1789773 / (16 * 254) = 440.3 Hz. Counting the "1" steps of
        // the duty-0 sequencer over a long window pins the same
        // formula without the DC-blocker settling transient: exactly
        // one eighth of the CPU cycles are high.
        let mut apu = apu_all_enabled();
        apu.tri.seq_pos = 15; // park the frozen power-on triangle
        apu.write_register(0x4000, 0x10 | 0x0F); // constant volume, duty 0
        apu.write_register(0x4002, 0xFD);
        apu.write_register(0x4003, 31 << 3);
        let mut highs = 0u32;
        for _ in 0..90_000 {
            apu.tick();
            if apu.pulse[0].output() != 0 {
                highs += 1;
            }
        }
        let expected = 90_000 / 8;
        assert!(
            (i64::from(highs) - i64::from(expected)).abs() < 700,
            "high samples {highs} far from the formula value {expected}"
        );
    }

    #[test]
    fn triangle_steps_every_period_plus_one_cpu_cycles() {
        // f = CPU / (32 * (p + 1)); with p = 0xFD the 32-step sequence
        // wraps every 32 * 254 = 8128 CPU cycles.
        let mut apu = Apu::new(Region::Ntsc);
        apu.write_status(0x0F);
        apu.write_register(0x4008, 0xFF); // control on, reload 127
        apu.write_register(0x400A, 0xFD);
        apu.write_register(0x400B, 31 << 3);
        // The reload flag keeps the linear counter at 127.
        apu.tri.linear_counter = 127;
        let start = apu.tri.seq_pos;
        for _ in 0..8128 {
            apu.tick();
        }
        assert_eq!(apu.tri.seq_pos, start, "one full sequence period");
        for _ in 0..254 {
            apu.tick();
        }
        assert_eq!(apu.tri.seq_pos, start + 1, "one step per period+1");
    }

    #[test]
    fn noise_table_entries_in_cpu_cycles() {
        assert_eq!(NOISE_PERIODS_NTSC[0], 4);
        assert_eq!(NOISE_PERIODS_NTSC[15], 4068);
        assert_eq!(NOISE_PERIODS_PAL[0], 4);
        assert_eq!(NOISE_PERIODS_PAL[15], 3778);
    }

    #[test]
    fn noise_lfsr_powers_up_as_one_and_shifts_at_the_table_rate() {
        let mut apu = Apu::new(Region::Ntsc);
        assert_eq!(apu.noise.lfsr, 1, "documented power-up value");
        // Rate index 0: a shift every 4 CPU cycles = every 2 APU
        // half-cycles; count shifts over 400 CPU cycles.
        apu.noise.write_period(0x00, Region::Ntsc);
        let mut shifts = 0u32;
        let mut last = apu.noise.lfsr;
        for _ in 0..400 {
            apu.tick();
            if apu.noise.lfsr != last {
                shifts += 1;
                last = apu.noise.lfsr;
            }
        }
        assert_eq!(shifts, 100, "one shift every 4 CPU cycles");
    }

    #[test]
    fn frame_irq_flag_set_at_the_documented_point_then_read_cleared() {
        let mut apu = Apu::new(Region::Ntsc);
        // $4017 = 0 at power-on: the 4-step frame IRQ is enabled.
        for _ in 0..29_827 {
            apu.tick();
        }
        assert!(!apu.frame_irq_flag, "not yet set at 29827 cycles");
        apu.tick();
        assert!(apu.frame_irq_flag, "set at the first documented point");
        let status = apu.read_status();
        assert_eq!(status & 0x40, 0x40, "readable before the clear");
        assert!(!apu.frame_irq_flag, "reading clears the frame flag");
    }

    #[test]
    fn frame_irq_disabled_by_inhibit_and_five_step() {
        // blargg 3-irq_flag rules: mode $40 and $80 never set it.
        let mut apu = Apu::new(Region::Ntsc);
        apu.write_frame_counter(0x40, false);
        for _ in 0..60_000 {
            apu.tick();
        }
        assert!(!apu.frame_irq_flag, "inhibit clears and blocks");
        let mut apu = Apu::new(Region::Ntsc);
        apu.write_frame_counter(0x80, false);
        for _ in 0..60_000 {
            apu.tick();
        }
        assert!(!apu.frame_irq_flag, "5-step mode never sets the flag");
        // Writing $00 or $80 does not clear an already-set flag; only
        // $40/$C0 (the inhibit bit) does.
        let mut apu = Apu::new(Region::Ntsc);
        for _ in 0..29_830 {
            apu.tick();
        }
        assert!(apu.frame_irq_flag);
        apu.write_frame_counter(0x00, false);
        assert!(apu.frame_irq_flag, "$00 leaves the flag alone");
        apu.write_frame_counter(0x40, false);
        assert!(!apu.frame_irq_flag, "the inhibit bit clears it");
    }

    #[test]
    fn writing_80_to_4017_clocks_length_immediately() {
        // blargg 1-len_ctr rule 4: two $80 writes clock the length
        // twice (it expires); rule 5: $00 writes do not clock it.
        let mut apu = Apu::new(Region::Ntsc);
        apu.write_status(0x0F);
        apu.write_register(0x4003, 3 << 3); // length 2
        apu.write_frame_counter(0x80, false);
        // The reset lands 3-4 cycles later and clocks once.
        for _ in 0..8 {
            apu.tick();
        }
        assert_eq!(apu.pulse[0].length, 1);
        apu.write_frame_counter(0x80, false);
        for _ in 0..8 {
            apu.tick();
        }
        assert_eq!(apu.pulse[0].length, 0, "two mode-1 writes expire it");
        // A mode-0 write must not clock.
        apu.write_register(0x4003, 3 << 3);
        apu.write_frame_counter(0x00, false);
        for _ in 0..8 {
            apu.tick();
        }
        assert_eq!(apu.pulse[0].length, 2, "$00 write does not clock length");
    }

    #[test]
    fn envelope_decays_and_loops_at_the_quarter_rate() {
        // Volume 0 makes the divider reload to zero, so every quarter
        // clock after the start decays the level by one.
        let mut apu = Apu::new(Region::Ntsc);
        apu.write_status(0x0F);
        apu.write_register(0x4000, 0x20); // halt(loop) + envelope mode
        apu.write_register(0x4003, 31 << 3); // restarts the envelope
        for _ in 0..7457 {
            apu.tick();
        }
        assert_eq!(apu.pulse[0].env.decay, 15, "the start clock loads 15");
        for _ in 0..14913 - 7457 {
            apu.tick();
        }
        assert_eq!(apu.pulse[0].env.decay, 14, "decayed one step");
        // With the halt (loop) flag the decay returns to 15 after the
        // full 16-step decay: 17 quarter clocks in total (4 sequence
        // wraps plus one more quarter).
        let mut apu = Apu::new(Region::Ntsc);
        apu.write_status(0x0F);
        apu.write_register(0x4000, 0x20);
        apu.write_register(0x4003, 31 << 3);
        for _ in 0..4 * 29830 + 7457 + 5 {
            apu.tick();
        }
        assert_eq!(
            apu.pulse[0].env.decay, 15,
            "the looped envelope returned to 15 after a full decay"
        );
    }

    #[test]
    fn read_status_reports_channel_and_dmc_bits() {
        let mut apu = Apu::new(Region::Ntsc);
        // The DMC sample registers first, then the enable that starts it.
        apu.write_register(0x4012, 0x3C);
        apu.write_register(0x4013, 0x02); // 33 bytes
        apu.write_status(0x1F);
        apu.write_register(0x4003, 31 << 3);
        apu.write_register(0x400B, 31 << 3);
        apu.write_register(0x400F, 31 << 3);
        apu.write_register(0x4007, 31 << 3);
        let status = apu.read_status();
        assert_eq!(status & 0x0F, 0x0F, "four length bits set");
        assert_eq!(status & 0x10, 0x10, "DMC active (33 bytes remain)");
        assert_eq!(status & 0x20, 0, "bit 5 belongs to the bus layer");
        assert_eq!(status & 0xC0, 0, "no IRQ flags raised");
    }

    #[test]
    fn write_status_restarts_the_dmc_sample_only_from_zero() {
        let mut apu = Apu::new(Region::Ntsc);
        apu.write_register(0x4012, 0x00); // $C000
        apu.write_register(0x4013, 0x01); // 17 bytes
        apu.write_status(0x10);
        assert_eq!(apu.dmc.bytes_remaining, 17);
        // A second enable while bytes remain must not restart.
        apu.dmc.addr = 0xC009;
        apu.dmc.bytes_remaining = 8;
        apu.write_status(0x10);
        assert_eq!(apu.dmc.bytes_remaining, 8, "no restart mid-sample");
        assert_eq!(apu.dmc.addr, 0xC009);
    }

    #[test]
    fn dmc_dma_completes_advances_and_sets_irq_at_the_end() {
        let mut apu = Apu::new(Region::Ntsc);
        apu.write_register(0x4010, 0x80); // IRQ enabled, no loop
        apu.write_register(0x4012, 0x01); // $C040
        apu.write_register(0x4013, 0x00); // 1 byte
        apu.write_status(0x10);
        apu.tick(); // the reader arms the fetch
        assert!(apu.dma_request());
        let addr = apu.take_dma_request().expect("the fetch address");
        assert_eq!(addr, 0xC040);
        apu.dma_complete(0xAA);
        assert_eq!(apu.dmc.sample_buffer, 0xAA);
        assert!(!apu.dmc.buffer_empty);
        assert_eq!(apu.dmc.addr, 0xC041);
        assert_eq!(apu.dmc.bytes_remaining, 0);
        assert!(apu.dmc.irq_flag, "the IRQ flag is set when it ends");
        assert!(apu.irq_level(), "the IRQ line follows the flag");
        // A $4015 write clears the DMC flag.
        apu.write_status(0x10);
        assert!(!apu.dmc.irq_flag);
        // The loop flag restarts instead.
        apu.write_register(0x4010, 0xC0); // loop + IRQ enabled
        apu.dmc.bytes_remaining = 1;
        apu.dma_complete(0x55);
        assert_eq!(apu.dmc.bytes_remaining, 1, "the sample restarted");
        assert!(!apu.dmc.irq_flag, "no IRQ when looping");
    }

    #[test]
    fn mixer_endpoint_values_follow_the_published_formulas() {
        // The power-on triangle is parked at its zero step so it stays
        // out of the sums (its frozen value is a pure DC offset).
        // Only pulse 1 at constant volume 15: the Q15 mix value for
        // p = 15 is 95.88*15/(8128/15+100) = 0.14937 -> about 4896.
        let mut apu = Apu::new(Region::Ntsc);
        apu.tri.seq_pos = 15;
        apu.write_status(0x01);
        apu.write_register(0x4000, 0x10 | 0x0F);
        apu.write_register(0x4002, 0x20);
        apu.write_register(0x4003, 31 << 3);
        apu.pulse[0].seq_pos = 1; // the "1" step of duty 0
        let total = apu.mix_sample();
        assert!(
            (3_000..7_000).contains(&i32::from(total)),
            "pulse-only sample {total} inside the documented window"
        );
        // Only the DMC at level 127 with everything else silent.
        let mut apu = Apu::new(Region::Ntsc);
        apu.tri.seq_pos = 15;
        apu.dmc.level = 127;
        let tnd = apu.mix_sample();
        // 159.79/(22638/127 + 100) = 0.5745 -> about 18822 in Q15.
        assert!(
            (15_000..21_000).contains(&i32::from(tnd)),
            "DMC-only sample {tnd} inside the documented window"
        );
    }

    #[test]
    fn dc_blocker_removes_a_constant_offset() {
        // A constant nonzero input decays toward zero output with the
        // documented pole (the amplitude falls below 1% after about
        // 300k samples).
        let mut apu = Apu::new(Region::Ntsc);
        apu.dmc.level = 64;
        apu.set_capture(true);
        for _ in 0..400_000 {
            apu.tick();
        }
        let tail = &apu.pcm[390_000..];
        let max = tail.iter().map(|&s| i32::from(s).abs()).max().unwrap_or(0);
        assert!(
            max < 200,
            "constant input filtered to near zero (max {max})"
        );
    }

    #[test]
    fn power_on_transient_decays_through_the_dc_blocker() {
        // At power-on the triangle sequencer is frozen at 15 — a real
        // DC offset the blocker decays away (bounded, monotonically
        // shrinking, never a persistent offset).
        let mut apu = Apu::new(Region::Ntsc);
        apu.set_capture(true);
        for _ in 0..600 {
            apu.tick();
        }
        assert!(
            apu.pcm[1].abs() > 1_000,
            "the frozen triangle contributes a bounded transient"
        );
        assert!(apu.pcm[10].abs() < apu.pcm[1].abs(), "decaying");
        assert!(apu.pcm[500].abs() < apu.pcm[10].abs(), "still decaying");
    }

    #[test]
    fn zeroed_triangle_and_silence_produce_a_zero_stream() {
        let mut apu = Apu::new(Region::Ntsc);
        apu.tri.seq_pos = 15; // the zero step of the frozen sequencer
        apu.set_capture(true);
        for _ in 0..1_000 {
            apu.tick();
        }
        assert!(apu.pcm.iter().all(|&s| s == 0));
        assert_eq!(apu.samples, 1_000);
    }

    #[test]
    fn state_bytes_change_with_every_channel_field() {
        let base = Apu::new(Region::Ntsc).state_bytes();
        let mut apu = Apu::new(Region::Ntsc);
        apu.pulse[0].timer = 5;
        assert_ne!(apu.state_bytes(), base, "pulse timer is hashed");
        let mut apu = Apu::new(Region::Ntsc);
        apu.tri.seq_pos = 3;
        assert_ne!(apu.state_bytes(), base, "triangle position is hashed");
        let mut apu = Apu::new(Region::Ntsc);
        apu.noise.lfsr = 0x1234;
        assert_ne!(apu.state_bytes(), base, "noise LFSR is hashed");
        let mut apu = Apu::new(Region::Ntsc);
        apu.dmc.level = 7;
        assert_ne!(apu.state_bytes(), base, "DMC level is hashed");
        let mut apu = Apu::new(Region::Ntsc);
        apu.dc_x = 5;
        assert_ne!(apu.state_bytes(), base, "the DC filter state is hashed");
    }

    #[test]
    fn reset_silences_channels_and_keeps_the_frame_register() {
        let mut apu = Apu::new(Region::Ntsc);
        apu.write_status(0x1F);
        apu.write_register(0x4003, 31 << 3);
        apu.write_register(0x4011, 0x30);
        apu.write_frame_counter(0x40, false);
        let mode = apu.frame_mode5;
        let inhibit = apu.frame_irq_inhibit;
        apu.reset();
        assert_eq!(apu.pulse[0].length, 0);
        assert_eq!(apu.tri.seq_pos, 0);
        assert_eq!(apu.dmc.level, 0x30 & 1);
        assert_eq!((apu.frame_mode5, apu.frame_irq_inhibit), (mode, inhibit));
    }

    #[test]
    fn pal_frame_table_uses_the_pal_cycle_counts() {
        let ntsc = frame_table(Region::Ntsc, false);
        let pal = frame_table(Region::Pal, false);
        assert_eq!(ntsc.quarters[0], 7457);
        assert_eq!(pal.quarters[0], 8313);
        assert_eq!(pal.wrap, 33254);
        assert_eq!(frame_table(Region::Dendy, false).wrap, 33254);
    }
}
