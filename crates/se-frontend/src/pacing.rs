//! Region frame pacing (spec §5.5: "vsync or frame-limit").
//!
//! M3.3 uses the frame-limit side of the spec: one emulated frame per
//! real-time frame period of the console's region, paced with
//! `std::time::Instant` in the frontend only. The core itself keeps
//! zero wall-clock and zero thread knowledge (D-16/D-26); this module
//! is the host-side shell around it.
//!
//! The periods derive from the same `se_core::timing` constants the
//! engine is built on (master clock, PPU divisor, average dots per
//! frame with NTSC's odd-frame dot skip), so the frontend and the
//! engine can never disagree:
//!
//! - NTSC: 21 477 272.727 Hz master, /4 PPU, 89 341.5 average dots
//!   => 60.0988 fps => 16 639 347 ns.
//! - PAL: 26 601 712.5 Hz master, /5 PPU, 106 392 dots exactly
//!   => 50.0070 fps => 19 997 209 ns.
//! - Dendy: the PAL PPU (same 106 392 dots) => 19 997 209 ns.

use std::time::Duration;

use se_core::timing::Region;

/// The real-time period of one emulated frame in a region.
///
/// Derived from the engine's own constants: the region's average PPU
/// dots per frame (NTSC includes the odd-frame dot skip, so the
/// long-run rate is exactly the console's) over the region's PPU dot
/// clock.
pub fn frame_period(region: Region) -> Duration {
    let seconds = region.ppu_dots_per_frame() / region.ppu_hz();
    Duration::from_secs_f64(seconds)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntsc_period_matches_the_documented_60_0988_fps() {
        let period = frame_period(Region::Ntsc);
        // 89 341.5 average dots (the dot skip included) at
        // 5 369 318.18 dots-per-second = 16 639 263.5 ns, rounded to
        // the nearest nanosecond by from_secs_f64.
        assert_eq!(period.as_nanos(), 16_639_263);
        let fps = 1.0 / period.as_secs_f64();
        assert!((fps - 60.0988).abs() < 0.0001, "fps {fps}");
    }

    #[test]
    fn pal_period_matches_the_documented_50_0070_fps() {
        let period = frame_period(Region::Pal);
        // 106 392 dots / 5 320 342.5 dots-per-second = 19 997 208.8 ns.
        assert_eq!(period.as_nanos(), 19_997_209);
        let fps = 1.0 / period.as_secs_f64();
        assert!((fps - 50.0070).abs() < 0.0001, "fps {fps}");
    }

    #[test]
    fn dendy_shares_the_pal_picture_rate() {
        assert_eq!(frame_period(Region::Dendy), frame_period(Region::Pal));
    }

    #[test]
    fn periods_are_within_the_sane_range() {
        for region in se_core::timing::ALL_REGIONS {
            let period = frame_period(region);
            assert!(period.as_millis() >= 15 && period.as_millis() <= 22);
        }
    }
}
