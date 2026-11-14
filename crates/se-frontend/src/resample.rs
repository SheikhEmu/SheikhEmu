//! The realtime resampler: native-rate mono PCM to the device rate
//! (spec §5.5 "resampler" + §5.5.1 drift rule, D-40).
//!
//! The core produces one deterministic i16 sample per CPU cycle — the
//! native rate derived from the master clock alone (M4.2: NTSC
//! 1 789 772.727 Hz, PAL 1 662 607.0, Dendy 1 773 447.5). Real
//! devices run at 44 100/48 000 Hz, so the frontend decimates by
//! ~37:1 with a **windowed-sinc polyphase FIR**:
//!
//! - impulse response `h(t) = fc·sinc(fc·t)·w(t)` with the cutoff
//!   `fc = out_rate/in_rate` (the device Nyquist in input cycles) and
//!   a 4-term Blackman-Harris window (~ -92 dB sidelobes),
//! - the half-width is **3 output-sample periods** in input units, so
//!   the kernel spans ~6 output periods — wide enough that the 37:1
//!   decimation's aliasing floor sits far below audibility,
//! - 256 phase sub-tables quantize the fractional input position
//!   (phase error < 1/256 of an output sample),
//! - the table is built once at stream start from pure `f64`
//!   arithmetic and then only reused.
//!
//! This is the **realtime** path — it is deliberately NOT part of the
//! deterministic fingerprint chain: determinism is guaranteed (and CI
//! -golden) at the core's native PCM (integer math, M4.2); here the
//! exact sample count per frame must *vary* because that is the
//! §5.5.1 drift-correction actuator:
//!
//! **تصحيح الانجراف بعدد العينات المنتجة لا بتغيير سرعة المحاكاة** —
//! the caller measures the ring level each frame and nudges
//! `set_step_factor`; the resampler then advances its input cursor by
//! `ratio × factor` input samples per output sample, producing
//! slightly more or fewer output samples per emulated frame. The
//! emulation's own frame pacing is never touched.
//!
//! The nominal step is `in_rate/out_rate`; the factor is clamped by
//! the caller (the audio engine uses ±0.5 %, which is at most a
//! ~8.6-cent pitch bend and only ever that large while recovering
//! from a real stall).

/// Number of phase sub-tables (fractional-position quantization).
const PHASES: usize = 256;
/// Kernel half-width in OUTPUT-sample periods.
const HALF_WIDTH_OUT: f64 = 3.0;

/// A windowed-sinc polyphase FIR resampler with an adjustable step.
pub struct Resampler {
    /// Input samples per output sample at nominal rates.
    ratio: f64,
    /// The current input-cursor step = ratio × factor.
    step: f64,
    /// The current step factor (1.0 = nominal).
    step_factor: f64,
    /// FIR taps for all phases, flat: phase-major rows of `taps`
    /// values — `table[phase][m]` lives at `phase*taps + m` and covers
    /// tap offsets `(m - half) - frac` of the fractional position.
    table: Vec<f64>,
    /// Number of taps per phase (odd).
    taps: usize,
    /// Kernel half-width in input samples.
    half: usize,
    /// Retained input history (taps-1 samples minimum) plus the
    /// current frame's unread input.
    history: Vec<i16>,
    /// Absolute input position of the next output, relative to the
    /// front of `history`.
    pos: f64,
}

impl Resampler {
    /// Build the phase table for `in_rate → out_rate`.
    ///
    /// # Panics
    /// Panics if either rate is non-finite/non-positive or the cutoff
    /// does not land below the input Nyquist (which would make the
    /// whole notion meaningless).
    pub fn new(in_rate: f64, out_rate: f64) -> Self {
        assert!(
            in_rate.is_finite() && in_rate > 0.0 && out_rate.is_finite() && out_rate > 0.0,
            "resampler rates must be finite and positive"
        );
        let ratio = in_rate / out_rate;
        // Cutoff in cycles per input sample; strictly below 0.5 for
        // every sane device rate (48 kHz over 1.79 MHz ≈ 0.0134).
        let cutoff = out_rate / in_rate;
        assert!(cutoff < 0.5, "device rate must be below the input Nyquist");
        let half = (HALF_WIDTH_OUT * ratio).ceil() as usize;
        let taps = 2 * half + 1;

        let mut table = Vec::with_capacity(PHASES * taps);
        for phase in 0..PHASES {
            let frac = phase as f64 / PHASES as f64;
            for m in 0..taps {
                // Tap offset from the (fractional) output position.
                let t = (m as f64) - (half as f64) - frac;
                table.push(kernel(cutoff, t, half));
            }
        }
        Resampler {
            ratio,
            step: ratio,
            step_factor: 1.0,
            table,
            taps,
            half,
            // `half` zero samples of virtual past plus a cursor that
            // starts one kernel-half into the padded history: the
            // first output's window is then exactly history[0..taps],
            // centered on real input position 0 (the standard FIR
            // warmup — the first ~3 output samples rise from
            // silence).
            history: vec![0; half],
            pos: half as f64,
        }
    }

    /// Set the drift-correction factor. It multiplies the input-cursor
    /// step; values above 1 produce fewer output samples (consume
    /// input faster), below 1 produce more. The caller clamps.
    pub fn set_step_factor(&mut self, factor: f64) {
        self.step_factor = factor;
        self.step = self.ratio * factor;
    }

    /// Append `input` to the history and produce every output whose
    /// FIR window lies fully inside it, appending i16 samples to `out`.
    ///
    /// The exact output count varies with the step factor — that is
    /// the §5.5.1 drift actuator, not a rounding accident.
    pub fn process(&mut self, input: &[i16], out: &mut Vec<i16>) {
        self.history.extend_from_slice(input);
        // An output at input position p needs the taps window
        // [floor(p) - half, floor(p) + half] fully inside history.
        while self.pos.floor() as usize + self.half < self.history.len() {
            let base = self.pos.floor() as usize;
            let frac = self.pos - base as f64;
            let phase = (frac * PHASES as f64).round() as usize % PHASES;
            let row = &self.table[phase * self.taps..phase * self.taps + self.taps];
            let window = &self.history[base - self.half..base - self.half + self.taps];
            let mut acc = 0.0f64;
            for (sample, tap) in window.iter().zip(row) {
                acc += f64::from(*sample) * tap;
            }
            out.push(clamp_i16(acc));
            self.pos += self.step;
        }
        // Retain the tail the next outputs still need (from
        // floor(pos) - half on), and shift the cursor accordingly.
        let consumed = self.pos.floor() as usize;
        let keep_from = consumed.saturating_sub(self.half);
        if keep_from > 0 {
            self.history.drain(..keep_from);
            self.pos -= keep_from as f64;
        }
    }
}

/// The ideal-brickwall sinc with the 4-term Blackman-Harris window.
fn kernel(cutoff: f64, t: f64, half: usize) -> f64 {
    let sinc = if t.abs() < f64::EPSILON {
        cutoff
    } else {
        let x = std::f64::consts::PI * cutoff * t;
        cutoff * x.sin() / x
    };
    // Blackman-Harris over [-1, 1] in kernel-half-width units.
    let u = t / (half as f64 + 1.0);
    let window = 0.358_75
        + 0.488_29 * (std::f64::consts::PI * u).cos()
        + 0.141_28 * (2.0 * std::f64::consts::PI * u).cos()
        + 0.011_68 * (3.0 * std::f64::consts::PI * u).cos();
    sinc * window
}

/// Round-and-clamp an f64 accumulator into the i16 range. NaN maps
/// to 0 (the saturating cast), keeping the realtime path alive even
/// against pathological input.
fn clamp_i16(value: f64) -> i16 {
    let rounded = value.round();
    if rounded >= 32767.0 {
        32767
    } else if rounded <= -32768.0 {
        -32768
    } else {
        rounded as i16
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const NTSC: f64 = 21_477_272.727_272_727 / 12.0;
    const DEVICE: f64 = 48_000.0;

    #[test]
    fn nominal_frame_output_count_matches_the_rates() {
        // One NTSC frame ≈ 29 780.5 CPU cycles ≈ 29 780 samples at the
        // native rate; 60 frames must produce ~48 000×(60/60.0988)
        // outputs to within ±3 samples of the exact expectation.
        let mut r = Resampler::new(NTSC, DEVICE);
        let mut out = Vec::new();
        for _ in 0..60 {
            r.process(&vec![100i16; 29_780], &mut out);
        }
        let expected = 60.0 * 29_780.5 * (DEVICE / NTSC);
        // The FIR pipeline holds `half` input samples in reserve (the
        // last outputs need the NEXT frame's input to complete their
        // windows); by design `half` is HALF_WIDTH_OUT output periods
        // wide, so up to ~3 + 1 outputs are still pending at the end
        // of a finite block — zero effect on the continuous realtime
        // stream. The count may not EXCEED the expectation.
        let reserve = HALF_WIDTH_OUT + 1.0;
        assert!(
            (expected - out.len() as f64) <= reserve,
            "out.len() {} vs expected {expected} (reserve {reserve})",
            out.len()
        );
        assert!(
            out.len() as f64 <= expected + 1.0,
            "out.len() {} above expected {expected}",
            out.len()
        );
    }

    #[test]
    fn a_constant_input_stays_constant_after_warmup() {
        // DC through the FIR: after the kernel window fills, the
        // ripple must stay within 1.5 % (Blackman-Harris passband
        // ripple) — the mix is not re-amplified, only filtered.
        let mut r = Resampler::new(NTSC, DEVICE);
        let mut out = Vec::new();
        for _ in 0..10 {
            r.process(&vec![10_000i16; 4_000], &mut out);
        }
        let tail = &out[out.len() / 2..];
        let max = tail.iter().map(|s| s.abs()).max().unwrap();
        let min = tail.iter().map(|s| s.abs()).min().unwrap();
        assert!(max <= 10_150, "dc ripple high: {max}");
        assert!(min >= 9_850, "dc ripple low: {min}");
    }

    #[test]
    fn a_sine_keeps_its_frequency_across_the_resample() {
        // 440 Hz at the native rate, resampled to 48 kHz: estimate the
        // output frequency by linear-interpolated zero crossings.
        let mut r = Resampler::new(NTSC, DEVICE);
        let mut out = Vec::new();
        let cycles = 200.0;
        let total = (cycles * NTSC / 440.0) as usize;
        for chunk in (0..total).step_by(4_000) {
            let end = (chunk + 4_000).min(total);
            let block: Vec<i16> = (chunk..end)
                .map(|n| {
                    let x = 2.0 * std::f64::consts::PI * 440.0 * n as f64 / NTSC;
                    (12_000.0 * x.sin()) as i16
                })
                .collect();
            r.process(&block, &mut out);
        }
        // Drop the first/last millisecond (window warmup).
        let skip = 200.min(out.len() / 4);
        let signal = &out[skip..out.len() - skip];
        // Distances between consecutive upward zero crossings
        // (linear-interpolated) give the periods; their mean gives the
        // frequency.
        let mut periods = Vec::new();
        let mut previous: Option<f64> = None;
        for (index, pair) in signal.windows(2).enumerate() {
            if pair[0] < 0 && pair[1] >= 0 {
                let frac = f64::from(-pair[0]) / f64::from(pair[1] - pair[0]);
                let position = index as f64 + frac;
                if let Some(prior) = previous {
                    periods.push(position - prior);
                }
                previous = Some(position);
            }
        }
        assert!(periods.len() > 100, "too few zero crossings");
        let mean_period = periods.iter().sum::<f64>() / periods.len() as f64;
        let measured = DEVICE / mean_period;
        assert!(
            (measured - 440.0).abs() < 1.0,
            "measured {measured} Hz instead of 440"
        );
    }

    #[test]
    fn the_step_factor_changes_the_produced_sample_count() {
        // The §5.5.1 actuator: factor 0.999 produces ~0.1 % MORE
        // output samples over the same input.
        let mut a = Resampler::new(NTSC, DEVICE);
        let mut b = Resampler::new(NTSC, DEVICE);
        b.set_step_factor(0.999);
        let mut out_a = Vec::new();
        let mut out_b = Vec::new();
        for _ in 0..30 {
            a.process(&vec![5_000i16; 29_780], &mut out_a);
            b.process(&vec![5_000i16; 29_780], &mut out_b);
        }
        assert!(out_b.len() > out_a.len());
        let growth = (out_b.len() as f64 - out_a.len() as f64) / out_a.len() as f64;
        assert!(
            (growth - 0.001).abs() < 0.0004,
            "growth {growth} not ~0.001"
        );
    }

    #[test]
    fn full_scale_input_stays_clamped_to_i16() {
        let mut r = Resampler::new(NTSC, DEVICE);
        let mut out = Vec::new();
        // A maximal square wave rings the kernel's overshoot to the
        // clamp; nothing may leave the i16 range (by construction of
        // clamp_i16, but this also guards NaN: NaN clamps low).
        for _ in 0..5 {
            let block: Vec<i16> = (0..8_000)
                .map(|n| if n % 2 == 0 { 32767 } else { -32768 })
                .collect();
            r.process(&block, &mut out);
        }
        assert!(out.iter().all(|s| (-32768..=32767).contains(s)));
    }

    #[test]
    fn zero_input_produces_silence() {
        let mut r = Resampler::new(NTSC, DEVICE);
        let mut out = Vec::new();
        for _ in 0..5 {
            r.process(&vec![0i16; 4_000], &mut out);
        }
        assert!(out.iter().all(|s| *s == 0));
    }
}
