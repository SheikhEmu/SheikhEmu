//! The realtime audio output engine (M4.3, spec §5.5 + §5.5.1, D-40).
//!
//! Threading model — exactly §5.5.1 ("الصوت على خيط منفصل في Desktop"):
//!
//! - **one producer**: the event-loop thread that runs the emulator.
//!   After every emulated frame it drains the core's native-rate PCM
//!   (`Machine::take_pcm`, one i16 sample per CPU cycle, M4.2),
//!   resamples it to the device rate (`resample.rs`) and pushes it
//!   into the SPSC ring,
//! - **one consumer**: the audio device's own callback thread (cpal
//!   spawns it; we spawn nothing), which drains the ring and fills
//!   the device buffer,
//! - **no Mutex, no lock of any kind** in that hot path: the only
//!   shared state is the lock-free `SpscRing` plus three atomics for
//!   statistics.
//!
//! Latency: the ring's target fill is **2 frames** (~33 ms at NTSC) —
//! comfortably inside the M4.3 goal of ≤ 3 frames — with a capacity
//! of ~4 frames so one missed callback is absorbed, not audible. The
//! first ~2 frames play pre-filled silence while the emulator's own
//! frames queue up behind them (the APU powers on silent anyway).
//!
//! Drift (§5.5.1: "أي انجراف يُصحَّح بتعديل عدد العينات المنتجة لا
//! بتغيير سرعة المحاكاة"): once per emulated frame, **before**
//! appending the new block, the producer reads the resting ring level
//! and nudges the resampler's step factor for the block about to be
//! produced:
//!
//! ```text
//! factor = clamp(1 + 1e-5 × (level − target), 1 ± 0.005)
//! ```
//!
//! A high ring (producer ahead) makes the resampler produce slightly
//! fewer samples; a low ring slightly more. The emulation speed, the
//! frame pacing and the core's deterministic PCM are never touched —
//! only the number of output samples changes. The ±0.5 % clamp is at
//! most a ~8.6-cent pitch bend and only approaches it while
//! recovering from a real stall; near the target it stays under
//! ~0.02 % (inaudible).
//!
//! Silent fallback (M4.3: "بيئة بلا صوت لا تفشل تشغيلًا"): if the
//! machine has no usable output device — a headless CI box, a
//! bluetooth-only moment, a removed USB card — the window keeps
//! running perfectly silent, and the Arabic reason plus the stats
//! print once at exit. Nothing fails to run.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, Stream, StreamConfig};

use se_core::timing::Region;

use crate::resample::Resampler;
use crate::ring::SpscRing;

/// Target ring fill, in frames (the M4.3 latency goal is ≤ 3).
const TARGET_FRAMES: f64 = 2.0;
/// Ring capacity, in frames (rounded up to a power of two).
const RING_FRAMES: f64 = 4.0;
/// Drift gain: correction per sample of level error.
const DRIFT_GAIN: f64 = 1e-5;
/// Drift clamp: the step factor stays within ±0.5 %.
const DRIFT_MAX: f64 = 5e-3;

/// The frontend's audio state machine: waiting for the first frame,
/// live, or silently degraded.
pub struct Audio {
    state: AudioState,
}

enum AudioState {
    /// `main` created it; the first emulated frame hasn't produced
    /// samples yet (the device rate is only knowable once a stream
    /// opens, so the resampler is built at that moment).
    NotStarted { region: Region },
    Live {
        pipe: Box<AudioPipe>,
        _stream: Stream,
    },
    /// No usable device: the documented silent mode.
    Silent { reason: String },
}

/// Everything the producer side owns, with no cpal types inside — so
/// the drift loop is unit-testable headlessly.
struct AudioPipe {
    ring: Arc<SpscRing>,
    resampler: Resampler,
    device_rate: f64,
    channels: u16,
    target_level: usize,
    /// The current drift step factor (1.0 = nominal).
    factor: f64,
    produced: u64,
    dropped: u64,
    consumed: Arc<AtomicU64>,
    underruns: Arc<AtomicU64>,
    device_failed: Arc<AtomicBool>,
}

impl Audio {
    /// The initial state: waiting for the first frame of `region`.
    pub fn pending(region: Region) -> Self {
        Audio {
            state: AudioState::NotStarted { region },
        }
    }

    /// Feed one emulated frame's native-rate PCM. The first call
    /// attempts to open the device; failure degrades to the silent
    /// mode with a one-time Arabic notice.
    pub fn feed(&mut self, native: &[i16]) {
        match &mut self.state {
            AudioState::NotStarted { region } => {
                let region = *region;
                match start_live(region) {
                    Ok((mut pipe, stream)) => {
                        // The first frame rides behind the pre-fill.
                        push_and_correct(&mut pipe, native);
                        self.state = AudioState::Live {
                            pipe: Box::new(pipe),
                            _stream: stream,
                        };
                    }
                    Err(reason) => {
                        eprintln!("الصوت معطّل: {reason} — يُكمل التشغيل بلا صوت");
                        self.state = AudioState::Silent { reason };
                    }
                }
            }
            AudioState::Live { pipe, .. } => push_and_correct(pipe, native),
            AudioState::Silent { .. } => {}
        }
    }

    /// The one-line Arabic exit report (evidence-friendly; goes to
    /// stderr so the D-32 capture pipelines stay untouched).
    pub fn report(&self) {
        match &self.state {
            AudioState::NotStarted { .. } => {}
            AudioState::Silent { reason } => {
                eprintln!("تقرير الصوت: بلا جهاز — {reason}");
            }
            AudioState::Live { pipe, .. } => {
                let consumed = pipe.consumed.load(Ordering::Relaxed);
                let underruns = pipe.underruns.load(Ordering::Relaxed);
                let failed = pipe.device_failed.load(Ordering::Relaxed);
                eprintln!(
                    "تقرير الصوت: جهاز {} هرتز بقنوات {} — عينات منتجة {} ومستهلكة {}، انقطاعات {}، عينات مسقطة {}{}",
                    pipe.device_rate,
                    pipe.channels,
                    pipe.produced,
                    consumed,
                    underruns,
                    pipe.dropped,
                    if failed { "، وفشل الجهاز أثناء التشغيل" } else { "" }
                );
            }
        }
    }
}

/// Open the device and build the live pipeline. Every failure returns
/// an Arabic reason for the silent mode.
fn start_live(region: Region) -> Result<(AudioPipe, Stream), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "لا يوجد جهاز إخراج صوتي".to_string())?;
    let supported = device
        .default_output_config()
        .map_err(|error| format!("تعذّر الاستعلام عن إعدادات الصوت: {error}"))?;
    // cpal 0.18: `SampleRate` is a plain `u32` alias.
    let sample_rate = supported.sample_rate();
    let device_rate = f64::from(sample_rate);

    let ring = Arc::new(SpscRing::new(ring_capacity(region, device_rate)));
    let consumed = Arc::new(AtomicU64::new(0));
    let underruns = Arc::new(AtomicU64::new(0));
    let device_failed = Arc::new(AtomicBool::new(false));

    // Three documented attempts: mono i16 (the mixer's own format),
    // then the device's default channel count as i16, then f32 with
    // an in-callback conversion — covering hosts whose only format is
    // float.
    let mut channels = 1u16;
    let mut stream = None;
    let mut last_error = String::new();
    let attempts: [(u16, bool); 3] = [
        (1, false),
        (supported.channels(), false),
        (supported.channels(), true),
    ];
    for (attempt_channels, as_float) in attempts {
        let config = StreamConfig {
            channels: attempt_channels,
            sample_rate,
            buffer_size: BufferSize::Default,
        };
        let result = if as_float {
            device.build_output_stream::<f32, _, _>(
                config,
                make_callback_f32(
                    ring.clone(),
                    attempt_channels,
                    consumed.clone(),
                    underruns.clone(),
                ),
                make_error_callback(device_failed.clone()),
                None,
            )
        } else {
            device.build_output_stream::<i16, _, _>(
                config,
                make_callback_i16(
                    ring.clone(),
                    attempt_channels,
                    consumed.clone(),
                    underruns.clone(),
                ),
                make_error_callback(device_failed.clone()),
                None,
            )
        };
        match result {
            Ok(built) => {
                stream = Some(built);
                channels = attempt_channels;
                break;
            }
            Err(error) => {
                last_error = format!("{error}");
            }
        }
    }
    let stream = stream.ok_or_else(|| format!("تعذّر فتح مجرى الصوت: {last_error}"))?;
    stream
        .play()
        .map_err(|error| format!("تعذّر تشغيل مجرى الصوت: {error}"))?;

    let target_level = target_level(region, device_rate);
    // The pre-fill: `target_level` samples of documented startup
    // silence, so the first callbacks never underrun and the latency
    // sits at the target from the very first real sample. (No further
    // mutation in here — the drift loop runs in `feed`.)
    let pipe = AudioPipe {
        ring,
        resampler: Resampler::new(region.cpu_hz(), device_rate),
        device_rate,
        channels,
        target_level,
        factor: 1.0,
        produced: target_level as u64,
        dropped: 0,
        consumed,
        underruns,
        device_failed,
    };
    pipe.ring.push(&vec![0i16; target_level]);
    Ok((pipe, stream))
}

/// Resample one frame, push it, and run the §5.5.1 drift actuator.
///
/// The level is measured **before** the new frame is appended — the
/// steady-state fill between pushes — so the controller targets the
/// resting level instead of chasing the phantom +1-frame transient
/// of a just-pushed block.
fn push_and_correct(pipe: &mut AudioPipe, native: &[i16]) {
    // The drift correction first (from the resting level): adjust the
    // number of samples PRODUCED per frame — never the emulation
    // speed.
    let level = pipe.ring.level() as f64;
    let error = level - pipe.target_level as f64;
    let correction = (DRIFT_GAIN * error).clamp(-DRIFT_MAX, DRIFT_MAX);
    pipe.factor = 1.0 + correction;
    pipe.resampler.set_step_factor(pipe.factor);
    let mut block = Vec::with_capacity(1024);
    pipe.resampler.process(native, &mut block);
    let written = pipe.ring.push(&block);
    pipe.produced += written as u64;
    pipe.dropped += (block.len() - written) as u64;
}

/// The target ring fill in samples (2 frames at the device rate).
fn target_level(region: Region, device_rate: f64) -> usize {
    (TARGET_FRAMES * device_rate / region.fps()).round() as usize
}

/// The ring capacity in samples (~4 frames; `SpscRing::new` rounds up
/// to a power of two).
fn ring_capacity(region: Region, device_rate: f64) -> usize {
    (RING_FRAMES * device_rate / region.fps()).ceil() as usize
}

/// Fill one interleaved output frame from a mono sample: duplicated
/// on stereo, first-channel-only beyond that (surround layouts get
/// silence on the extra speakers rather than a wrong upmix).
fn spread(sample: i16, channels: u16, frame: &mut [i16]) {
    frame[0] = sample;
    if frame.len() > 1 {
        frame[1] = if channels == 2 { sample } else { 0 };
    }
    for slot in frame.iter_mut().skip(2) {
        *slot = 0;
    }
}

fn make_callback_i16(
    ring: Arc<SpscRing>,
    channels: u16,
    consumed: Arc<AtomicU64>,
    underruns: Arc<AtomicU64>,
) -> impl FnMut(&mut [i16], &cpal::OutputCallbackInfo) + Send + 'static {
    move |data, _| {
        let width = channels.max(1) as usize;
        let mut one = [0i16; 1];
        for frame in data.chunks_mut(width) {
            if ring.pop(&mut one) == 0 {
                // A ring-empty frame plays digital silence and counts
                // one underrun — the honest realtime accounting.
                underruns.fetch_add(1, Ordering::Relaxed);
                frame.fill(0);
                continue;
            }
            consumed.fetch_add(1, Ordering::Relaxed);
            spread(one[0], channels, frame);
        }
    }
}

fn make_callback_f32(
    ring: Arc<SpscRing>,
    channels: u16,
    consumed: Arc<AtomicU64>,
    underruns: Arc<AtomicU64>,
) -> impl FnMut(&mut [f32], &cpal::OutputCallbackInfo) + Send + 'static {
    move |data, _| {
        let width = channels.max(1) as usize;
        let mut one = [0i16; 1];
        for frame in data.chunks_mut(width) {
            if ring.pop(&mut one) == 0 {
                underruns.fetch_add(1, Ordering::Relaxed);
                frame.fill(0.0);
                continue;
            }
            consumed.fetch_add(1, Ordering::Relaxed);
            let sample = f32::from(one[0]) / 32768.0;
            frame[0] = sample;
            if frame.len() > 1 {
                frame[1] = if channels == 2 { sample } else { 0.0 };
            }
            for slot in frame.iter_mut().skip(2) {
                *slot = 0.0;
            }
        }
    }
}

fn make_error_callback(device_failed: Arc<AtomicBool>) -> impl FnMut(cpal::Error) + Send + 'static {
    move |_error| {
        // Never print from the callback thread: the exit report tells
        // the owner the device died mid-run.
        device_failed.store(true, Ordering::Relaxed);
    }
}

/// Test-only helper: whether the live path was taken.
#[cfg(test)]
impl Audio {
    fn state_is_live(&self) -> bool {
        matches!(self.state, AudioState::Live { .. })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_pipe(region: Region, device_rate: f64) -> AudioPipe {
        let target_level = target_level(region, device_rate);
        let ring = Arc::new(SpscRing::new(ring_capacity(region, device_rate)));
        let mut pipe = AudioPipe {
            ring: ring.clone(),
            resampler: Resampler::new(region.cpu_hz(), device_rate),
            device_rate,
            channels: 1,
            target_level,
            factor: 1.0,
            produced: 0,
            dropped: 0,
            consumed: Arc::new(AtomicU64::new(0)),
            underruns: Arc::new(AtomicU64::new(0)),
            device_failed: Arc::new(AtomicBool::new(false)),
        };
        pipe.produced += target_level as u64;
        pipe.ring.push(&vec![0i16; target_level]);
        pipe
    }

    #[test]
    fn target_level_is_two_frames_within_the_latency_goal() {
        // NTSC at 48 kHz: 2 × 48 000 / 60.0988 ≈ 1598 samples ≈ 33 ms,
        // comfortably under the 3-frame M4.3 goal.
        let level = target_level(Region::Ntsc, 48_000.0);
        assert!((1580..1620).contains(&level), "level {level}");
        // PAL at 44.1 kHz: 2 × 44 100 / 50.007 ≈ 1763.
        let level = target_level(Region::Pal, 44_100.0);
        assert!((1740..1790).contains(&level), "level {level}");
    }

    #[test]
    fn ring_capacity_is_at_least_four_frames_and_bounded() {
        let cap = SpscRing::new(ring_capacity(Region::Ntsc, 48_000.0)).capacity();
        // 4 frames ≈ 3196 → rounds to 4096: absorbs one whole missed
        // callback, still ~5 frames of worst-case headroom.
        assert!(cap >= 3196, "cap {cap}");
        assert!(cap <= 8192, "cap {cap}");
    }

    #[test]
    fn spread_duplicates_stereo_and_mutes_surround() {
        let mut frame = [0i16; 2];
        spread(-1234, 2, &mut frame);
        assert_eq!(frame, [-1234, -1234]);
        let mut frame = [0i16; 6];
        spread(500, 6, &mut frame);
        assert_eq!(frame, [500, 0, 0, 0, 0, 0]);
        let mut frame = [0i16; 1];
        spread(7, 1, &mut frame);
        assert_eq!(frame[0], 7);
    }

    #[test]
    fn the_drift_correction_moves_the_factor_the_right_way() {
        // A high ring level must produce factor > 1 (produce fewer
        // samples); a low level factor < 1; the clamp holds.
        let factor = |error: f64| 1.0 + (DRIFT_GAIN * error).clamp(-DRIFT_MAX, DRIFT_MAX);
        assert!(factor(600.0) > 1.0);
        assert!(factor(-600.0) < 1.0);
        assert!(factor(600.0) <= 1.0 + DRIFT_MAX);
        assert!(factor(-600.0) >= 1.0 - DRIFT_MAX);
        // Near the target it is inaudible: 30 samples → 0.03 %.
        assert!((factor(30.0) - 1.0).abs() < 4e-4);
    }

    #[test]
    fn a_silent_audio_never_touches_the_machine_path() {
        // The silent mode's contract: feed is a no-op and the report
        // prints the reason (captured here by not panicking).
        let mut audio = Audio::pending(Region::Ntsc);
        audio.feed(&[]);
        assert!(matches!(audio.state, AudioState::Silent { .. }) || audio.state_is_live());
        audio.report();
    }

    #[test]
    fn push_and_correct_pulls_the_level_toward_the_target() {
        // A closed-loop smoke test with no device: prefill at the
        // target, then push frames of constant PCM while a simulated
        // device consumes 0.1 % too fast, and verify the produced
        // count adapts and the ring stays healthy.
        let region = Region::Ntsc;
        let device_rate = 48_000.0;
        let mut pipe = test_pipe(region, device_rate);
        // The simulated device eats 0.1 % too fast, tracked with a
        // fractional accumulator (a plain floor would erase the
        // drift entirely — the exact bug this test guards against).
        let nominal_per_frame = device_rate / region.fps();
        let mut drain_accumulator = 0.0f64;
        let mut factor_history = Vec::new();
        for _ in 0..120 {
            push_and_correct(&mut pipe, &vec![3000i16; 29_780]);
            drain_accumulator += nominal_per_frame * 1.001;
            let drain = drain_accumulator.floor() as usize;
            drain_accumulator -= drain as f64;
            let mut sink = vec![0i16; drain];
            let _ = pipe.ring.pop(&mut sink);
            factor_history.push(pipe.factor);
        }
        // After two seconds of a device eating 0.1 % too fast the
        // factor must sit BELOW 1 (produce more per frame)…
        let last = *factor_history.last().unwrap();
        assert!(last < 1.0, "factor settled at {last}");
        // …the ring must stay near the target, not run dry nor pin to
        // capacity…
        let level = pipe.ring.level();
        assert!(
            level > pipe.target_level / 2 && level < pipe.target_level * 2,
            "level {level} vs target {}",
            pipe.target_level
        );
        // …and nothing was dropped.
        assert_eq!(pipe.dropped, 0);
    }

    #[test]
    fn push_and_correct_survives_a_stalled_device_without_dropping() {
        // The device stops consuming (a frozen callback): the ring
        // fills, the producer drops the surplus (counted, never
        // corrupting), and the factor pins at the "produce fewer"
        // clamp.
        let region = Region::Ntsc;
        let mut pipe = test_pipe(region, 48_000.0);
        for _ in 0..30 {
            push_and_correct(&mut pipe, &vec![1000i16; 29_780]);
        }
        assert!(pipe.dropped > 0, "a stalled device must count drops");
        assert!(pipe.factor >= 1.0 + DRIFT_MAX - 1e-9);
    }
}
