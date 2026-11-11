//! Region timing constants for NTSC, PAL and Dendy consoles.
//!
//! All values follow the public NESdev Wiki "Cycle reference chart":
//!
//! - NTSC master clock 21477272.727 Hz (6 × NTSC subcarrier), CPU = master/12,
//!   PPU dot = master/4, so one CPU cycle spans 3 PPU dots.
//! - PAL master clock 26601712.5 Hz, CPU = master/16, PPU dot = master/5,
//!   so one CPU cycle spans 16/5 = 3.2 PPU dots. The PAL ratio is *not* 3:1;
//!   this exact point is a classic emulator bug and is pinned by tests below.
//! - Dendy keeps the PAL PPU (master/5) but divides the master clock by 15
//!   for the CPU, restoring a 3:1 CPU/PPU ratio on PAL-style video timing.
//!
//! One PPU scanline is 341 dots. NTSC uses 262 scanlines per frame with the
//! well-known odd-frame dot skip, giving an average of 89341.5 dots per
//! frame. PAL and Dendy use 312 scanlines = 106392 dots per frame.

/// PPU dots per regular scanline.
pub const DOTS_PER_SCANLINE: u32 = 341;

/// NTSC scanlines per frame (262, with the odd-frame pre-render dot skip).
pub const NTSC_SCANLINES_PER_FRAME: u32 = 262;

/// PAL / Dendy scanlines per frame.
pub const PAL_SCANLINES_PER_FRAME: u32 = 312;

/// NTSC master clock frequency in Hz (6 × NTSC subcarrier 3579545.45 Hz).
pub const NTSC_MASTER_CLOCK_HZ: f64 = 21_477_272.727_272_727;

/// PAL and Dendy master clock frequency in Hz.
pub const PAL_MASTER_CLOCK_HZ: f64 = 26_601_712.5;

/// Console region whose clocks this module describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Region {
    /// RP2C02 PPU / RP2A03 CPU, 60.0988 fps.
    Ntsc,
    /// RP2C07 PPU / RP2A07 CPU, 50.0070 fps, 16:5 CPU:PPU ratio.
    Pal,
    /// UA6538 PPU with 3:1 CPU:PPU ratio on PAL video timing.
    Dendy,
}

/// All supported regions, in declaration order.
pub const ALL_REGIONS: [Region; 3] = [Region::Ntsc, Region::Pal, Region::Dendy];

impl Region {
    /// Master clock frequency in Hz.
    pub const fn master_clock_hz(self) -> f64 {
        match self {
            Region::Ntsc => NTSC_MASTER_CLOCK_HZ,
            Region::Pal | Region::Dendy => PAL_MASTER_CLOCK_HZ,
        }
    }

    /// Master clock divisor producing the CPU clock.
    pub const fn cpu_divisor(self) -> u32 {
        match self {
            Region::Ntsc => 12,
            Region::Pal => 16,
            Region::Dendy => 15,
        }
    }

    /// Master clock divisor producing the PPU dot clock.
    pub const fn ppu_divisor(self) -> u32 {
        match self {
            Region::Ntsc => 4,
            Region::Pal | Region::Dendy => 5,
        }
    }

    /// Scanlines per frame (including the single pre-render line).
    pub const fn scanlines_per_frame(self) -> u32 {
        match self {
            Region::Ntsc => NTSC_SCANLINES_PER_FRAME,
            Region::Pal | Region::Dendy => PAL_SCANLINES_PER_FRAME,
        }
    }

    /// CPU clock frequency in Hz.
    pub fn cpu_hz(self) -> f64 {
        self.master_clock_hz() / f64::from(self.cpu_divisor())
    }

    /// PPU dot clock frequency in Hz.
    pub fn ppu_hz(self) -> f64 {
        self.master_clock_hz() / f64::from(self.ppu_divisor())
    }

    /// PPU dots covered by a single CPU cycle (3 for NTSC/Dendy, 3.2 for PAL).
    pub fn dots_per_cpu_cycle(self) -> f64 {
        f64::from(self.cpu_divisor()) / f64::from(self.ppu_divisor())
    }

    /// Average PPU dots per frame.
    ///
    /// NTSC frames alternate between 89341 and 89342 dots because the
    /// pre-render line is 340 dots on odd frames (the famous dot skip), so
    /// the frame average is 89341.5. PAL and Dendy frames are exactly
    /// 106392 dots (312 scanlines, no dot skip).
    pub fn ppu_dots_per_frame(self) -> f64 {
        match self {
            Region::Ntsc => f64::from(DOTS_PER_SCANLINE * self.scanlines_per_frame()) - 0.5,
            Region::Pal | Region::Dendy => {
                f64::from(DOTS_PER_SCANLINE * self.scanlines_per_frame())
            }
        }
    }

    /// Average CPU cycles per frame (float: the true averages are fractional).
    pub fn cpu_cycles_per_frame(self) -> f64 {
        self.ppu_dots_per_frame() / self.dots_per_cpu_cycle()
    }

    /// Average frames per second.
    pub fn fps(self) -> f64 {
        self.ppu_hz() / self.ppu_dots_per_frame()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntsc_cpu_ppu_ratio_is_three() {
        assert_eq!(Region::Ntsc.dots_per_cpu_cycle(), 3.0);
    }

    #[test]
    fn pal_cpu_ppu_ratio_is_sixteen_fifths_not_three() {
        // PAL divides the master clock by 16 for the CPU and by 5 for the
        // PPU, so one CPU cycle spans 16/5 = 3.2 PPU dots. A 3:1 ratio here
        // is a classic emulator timing bug.
        assert_eq!(Region::Pal.dots_per_cpu_cycle(), 3.2);
        assert_ne!(Region::Pal.dots_per_cpu_cycle(), 3.0);
    }

    #[test]
    fn dendy_cpu_ppu_ratio_is_three_with_pal_ppu() {
        assert_eq!(Region::Dendy.dots_per_cpu_cycle(), 3.0);
        assert_eq!(Region::Dendy.ppu_divisor(), Region::Pal.ppu_divisor());
        assert_eq!(
            Region::Dendy.scanlines_per_frame(),
            Region::Pal.scanlines_per_frame()
        );
    }

    #[test]
    fn documented_cpu_clocks() {
        assert!((Region::Ntsc.cpu_hz() - 1_789_772.73).abs() < 0.01);
        assert!((Region::Pal.cpu_hz() - 1_662_607.03).abs() < 0.01);
        assert!((Region::Dendy.cpu_hz() - 1_773_447.5).abs() < 0.01);
    }

    #[test]
    fn documented_ppu_dots_per_frame() {
        assert_eq!(Region::Ntsc.ppu_dots_per_frame(), 89_341.5);
        assert_eq!(Region::Pal.ppu_dots_per_frame(), 106_392.0);
        assert_eq!(Region::Dendy.ppu_dots_per_frame(), 106_392.0);
    }

    #[test]
    fn documented_cpu_cycles_per_frame() {
        // Canonical values: 29780.5 / 33247.5 / 35464 CPU cycles per frame.
        assert_eq!(Region::Ntsc.cpu_cycles_per_frame(), 29_780.5);
        assert_eq!(Region::Pal.cpu_cycles_per_frame(), 33_247.5);
        assert_eq!(Region::Dendy.cpu_cycles_per_frame(), 35_464.0);
    }

    #[test]
    fn documented_frame_rates() {
        assert!((Region::Ntsc.fps() - 60.0988).abs() < 0.001);
        assert!((Region::Pal.fps() - 50.007).abs() < 0.001);
        assert!((Region::Dendy.fps() - Region::Pal.fps()).abs() < 0.001);
    }

    #[test]
    fn scanline_constants() {
        assert_eq!(DOTS_PER_SCANLINE, 341);
        assert_eq!(Region::Ntsc.scanlines_per_frame(), 262);
        assert_eq!(Region::Pal.scanlines_per_frame(), 312);
    }

    #[test]
    fn all_regions_are_covered() {
        for region in ALL_REGIONS {
            assert!(region.cpu_hz() > 0.0);
            assert!(region.ppu_hz() > region.cpu_hz());
            assert!(region.master_clock_hz() > region.ppu_hz());
            assert!(region.fps() > 49.0 && region.fps() < 61.0);
        }
    }
}
