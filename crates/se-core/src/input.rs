//! Standard NES controllers (the two joypads on `$4016`/`$4017`).
//!
//! Protocol per the public NESdev Wiki "Standard controller" page
//! (verified through the public archive, D-07):
//!
//! - Writing `1` then `0` to bit 0 of `$4016` strobes **both**
//!   controllers: while the strobe is high the shift registers
//!   continuously reload from the live button states and reads keep
//!   returning the state of the first button (A); once the strobe goes
//!   low the reload stops and each read shifts out one button.
//! - The 8-bit report order is: A, B, Select, Start, Up, Down, Left,
//!   Right — bit 0 of the read, where **1 = pressed** on the NES (the
//!   4021 stores unpressed as 1 but the console inverts the line, so
//!   the CPU sees pressed as 1).
//! - Reads after the eighth return **1** on official controllers.
//! - Bits 5-7 of the read are open bus (`$40` on a standard NES);
//!   bits 1-2 (expansion/microphone) are Famicom-only and read 0 on
//!   the NES. The documented B1 value is therefore `0x40 | bit`.
//!
//! The button bit order is chosen so that the report byte equals the
//! internal button bitmap: `BTN_A` is bit 0 … `BTN_RIGHT` is bit 7.
//!
//! Buttons are pushed from the outside (CLI script, autopilot,
//! frontend keyboard) as snapshots per frame; no threads, no
//! wall-clock dependency (D-16/D-26). Controller state is input, not
//! machine state, and is deliberately excluded from the determinism
//! hash: identical runs with identical input scripts hash identically.

/// Button bit assignments (the standard report order).
pub const BTN_A: u8 = 1 << 0;
/// Button B.
pub const BTN_B: u8 = 1 << 1;
/// Button Select.
pub const BTN_SELECT: u8 = 1 << 2;
/// Button Start.
pub const BTN_START: u8 = 1 << 3;
/// Button Up.
pub const BTN_UP: u8 = 1 << 4;
/// Button Down.
pub const BTN_DOWN: u8 = 1 << 5;
/// Button Left.
pub const BTN_LEFT: u8 = 1 << 6;
/// Button Right.
pub const BTN_RIGHT: u8 = 1 << 7;

/// Open-bus bits the standard NES drives on a controller read.
const OPEN_BUS_BITS: u8 = 0x40;

/// The two controller ports: live button snapshots, the strobe, and
/// the serial state.
#[derive(Clone, Debug, Default)]
pub struct Controllers {
    /// Live button bitmaps pushed by the host, port 0 and 1.
    live: [u8; 2],
    /// Snapshot inside the shift registers while the strobe is high
    /// (continuously reloaded), frozen when it goes low.
    latched: [u8; 2],
    /// The strobe line (`$4016` bit 0).
    strobe: bool,
    /// Bits consumed by reads since the strobe went low, per port.
    reads: [u8; 2],
}

impl Controllers {
    /// Push the live button bitmap for one port (host-side snapshot).
    pub fn set_live(&mut self, port: u8, buttons: u8) {
        if port < 2 {
            self.live[usize::from(port)] = buttons;
        }
    }

    /// The live bitmap of one port (autopilot bookkeeping, tests).
    pub fn live(&self, port: u8) -> u8 {
        if port < 2 {
            self.live[usize::from(port)]
        } else {
            0
        }
    }

    /// Write `$4016` bit 0: the shared strobe of both ports.
    pub fn write_strobe(&mut self, value: u8) {
        let high = value & 1 != 0;
        if high {
            // Continuously reload from the live buttons while high.
            self.latched = self.live;
            self.strobe = true;
        } else if self.strobe {
            // Falling edge: freeze the snapshot and arm the shifter.
            self.strobe = false;
            self.reads = [0; 2];
        }
    }

    /// Read one controller port: the current serial output with the
    /// documented open-bus bits. Advances the shifter while the strobe
    /// is low (the side effect real hardware has on a read).
    pub fn read(&mut self, port: u8) -> u8 {
        let index = usize::from(port.min(1));
        let bit = if self.strobe {
            // Strobe high: continuously the live state of button A.
            self.live[index] & BTN_A
        } else if self.reads[index] >= 8 {
            // Official controllers return 1 after the eighth read.
            1
        } else {
            let bit = (self.latched[index] >> self.reads[index]) & 1;
            self.reads[index] += 1;
            bit
        };
        OPEN_BUS_BITS | bit
    }

    /// The value a read would return, with no side effects (the
    /// trace-logger view).
    pub fn peek(&self, port: u8) -> u8 {
        let index = usize::from(port.min(1));
        let bit = if self.strobe {
            self.live[index] & BTN_A
        } else if self.reads[index] >= 8 {
            1
        } else {
            (self.latched[index] >> self.reads[index]) & 1
        };
        OPEN_BUS_BITS | bit
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Strobe high then low, then read the full report of port 0.
    #[test]
    fn report_order_is_a_b_select_start_up_down_left_right() {
        let mut pads = Controllers::default();
        // Press A (bit0) and Start (bit3) only.
        pads.set_live(0, BTN_A | BTN_START);
        pads.write_strobe(1);
        pads.write_strobe(0);
        // Expected serial bits: A=1, B=0, Sel=0, Start=1, Up..Right=0.
        let expected = [1u8, 0, 0, 1, 0, 0, 0, 0];
        for (i, want) in expected.iter().enumerate() {
            assert_eq!(pads.read(0), 0x40 | want, "button slot {i}");
        }
    }

    #[test]
    fn reads_after_eight_return_one_official() {
        let mut pads = Controllers::default();
        pads.write_strobe(1);
        pads.write_strobe(0);
        for _ in 0..8 {
            let _ = pads.read(0);
        }
        assert_eq!(pads.read(0), 0x41);
        assert_eq!(pads.read(0), 0x41, "keeps returning 1");
    }

    #[test]
    fn strobe_high_reads_return_live_a_continuously() {
        let mut pads = Controllers::default();
        pads.write_strobe(1);
        pads.set_live(0, BTN_A);
        assert_eq!(pads.read(0), 0x41, "live A pressed");
        // No shift advance while high: repeated reads stay on A.
        pads.set_live(0, 0);
        assert_eq!(pads.read(0), 0x40, "live A released mid-strobe");
    }

    #[test]
    fn falling_edge_freezes_and_re_reloads_on_next_strobe() {
        let mut pads = Controllers::default();
        pads.set_live(0, BTN_RIGHT);
        pads.write_strobe(1);
        pads.write_strobe(0);
        // Change live buttons after the snapshot: frozen report stays.
        pads.set_live(0, BTN_LEFT);
        let mut report = [0u8; 8];
        for slot in report.iter_mut() {
            *slot = pads.read(0) & 1;
        }
        assert_eq!(report[7], 1, "frozen bit is Right, not Left");
        // A new strobe cycle re-loads the new live state.
        pads.write_strobe(1);
        pads.write_strobe(0);
        let mut report2 = [0u8; 8];
        for slot in report2.iter_mut() {
            *slot = pads.read(0) & 1;
        }
        assert_eq!(report2[6], 1, "reloaded bit is Left");
    }

    #[test]
    fn both_ports_report_independently() {
        let mut pads = Controllers::default();
        pads.set_live(0, BTN_UP);
        pads.set_live(1, BTN_DOWN);
        pads.write_strobe(1);
        pads.write_strobe(0);
        // Port 0: Up is the 5th slot; port 1: Down is the 6th.
        let mut p0 = [0u8; 8];
        let mut p1 = [0u8; 8];
        for i in 0..8 {
            p0[i] = pads.read(0) & 1;
            p1[i] = pads.read(1) & 1;
        }
        assert_eq!(p0[4], 1);
        assert_eq!(p1[5], 1);
        assert_eq!(p0[5], 0);
        assert_eq!(p1[4], 0);
    }

    #[test]
    fn peek_does_not_advance_the_shifter() {
        let mut pads = Controllers::default();
        pads.set_live(0, BTN_B);
        pads.write_strobe(1);
        pads.write_strobe(0);
        assert_eq!(pads.peek(0), 0x40, "first peek sees slot A");
        assert_eq!(pads.peek(0), 0x40, "peek again: still slot A");
        let _ = pads.read(0);
        assert_eq!(pads.peek(0), 0x41, "after one read the slot is B");
    }

    #[test]
    fn upper_bits_are_the_documented_open_bus() {
        let mut pads = Controllers::default();
        assert_eq!(pads.read(0) & 0xF8, 0x40);
        assert_eq!(pads.read(1) & 0xF8, 0x40);
    }
}
