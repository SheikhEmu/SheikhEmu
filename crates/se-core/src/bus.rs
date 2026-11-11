//! CPU-visible memory map of the NES for milestone M1.
//!
//! The map follows the public NESdev Wiki "CPU memory map":
//!
//! | Range        | M1 implementation                                  |
//! |--------------|----------------------------------------------------|
//! | `$0000-$07FF`| 2 KiB internal RAM                                  |
//! | `$0008-$1FFF`| mirrors of the RAM (repeat every `$0800`)           |
//! | `$2000-$3FFF`| PPU registers, mirrored every 8 bytes (stub in M1)  |
//! | `$4000-$4017`| APU registers and IO (fixed `$FF` read in M1)           |
//! | `$4018-$401F`| test-mode registers (unused)                       |
//! | `$4020-$5FFF`| cartridge expansion area (open bus)                |
//! | `$6000-$7FFF`| PRG RAM (absent on NROM: open bus)                 |
//! | `$8000-$FFFF`| PRG ROM (NROM mapping, see [`NromPrg`])            |
//!
//! Open bus (NESdev "CPU open bus"): reading an address with no device
//! returns the last value driven on the CPU data bus. The bus keeps that
//! value in a latch: every real read or write updates the latch with the
//! transferred byte, and unmapped reads return the latch. This is the
//! documented B1-level simplification of open bus required by the spec
//! (full open bus is a B2 item).
//!
//! PPU and APU do not exist until M2/M3; their registers behave as open
//! bus in M1, and writes to them are accepted and dropped (they only
//! refresh the latch, like every write does). Nothing in M1 depends on
//! them; `hello.nes` and `nestest` never touch them.

use crate::cpu::Bus;

/// RAM size in bytes (2 KiB).
pub const RAM_BYTES: usize = 2048;

/// PRG mapping of an NROM (mapper 0) cartridge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NromPrg {
    /// 16 KiB PRG mirrored at `$8000` and `$C000` (NROM-128).
    Mirror16k(Vec<u8>),
    /// 32 KiB PRG mapped linearly at `$8000` (NROM-256).
    Full32k(Vec<u8>),
}

impl NromPrg {
    /// Build an NROM mapping from a PRG image.
    ///
    /// Only the two documented NROM sizes are accepted; other sizes are
    /// rejected because guessing a mapping would be inventing hardware.
    pub fn new(prg: Vec<u8>) -> Result<Self, BusError> {
        const PRG_16K: usize = 16 * 1024;
        const PRG_32K: usize = 32 * 1024;
        match prg.len() {
            PRG_16K => Ok(NromPrg::Mirror16k(prg)),
            PRG_32K => Ok(NromPrg::Full32k(prg)),
            other => Err(BusError::UnsupportedPrgSize(other)),
        }
    }

    /// Read PRG ROM at a CPU address in `$8000-$FFFF`.
    fn read(&self, addr: u16) -> u8 {
        match self {
            // Mirrored: `$C000-$FFFF` repeats the same 16 KiB image.
            NromPrg::Mirror16k(prg) => prg[usize::from(addr & 0x3FFF)],
            // Linear: the whole 32 KiB fills `$8000-$FFFF`.
            NromPrg::Full32k(prg) => prg[usize::from(addr & 0x7FFF)],
        }
    }
}

/// Failures while constructing a bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusError {
    /// PRG image size is not a supported NROM size (in bytes).
    UnsupportedPrgSize(usize),
}

impl std::fmt::Display for BusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BusError::UnsupportedPrgSize(size) => write!(
                f,
                "PRG size {size} bytes is not an NROM size (16 or 32 KiB)"
            ),
        }
    }
}

impl std::error::Error for BusError {}

/// The M1 CPU bus: RAM + NROM PRG + open-bus everything else.
#[derive(Clone, Debug)]
pub struct NesBus {
    ram: [u8; RAM_BYTES],
    prg: NromPrg,
    latch: u8,
}

impl NesBus {
    /// Build a bus carrying the given PRG image.
    ///
    /// RAM powers up as all zeros. Real hardware RAM power-on content is
    /// unspecified; a fixed zero pattern keeps emulation deterministic
    /// (required by the scheduler decision D-16) and matches what the
    /// reference traces expect.
    pub fn new(prg: Vec<u8>) -> Result<Self, BusError> {
        Ok(NesBus {
            ram: [0; RAM_BYTES],
            prg: NromPrg::new(prg)?,
            latch: 0,
        })
    }

    /// Read without side effects (does not refresh the open-bus latch).
    ///
    /// Used by the trace logger, which must observe memory without
    /// perturbing the state it describes.
    pub fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => self.ram[usize::from(addr & 0x07FF)],
            0x8000..=0xFFFF => self.prg.read(addr),
            // APU and IO registers: the M1 stub returns the fixed $FF
            // the reference trace shows for this range (the calibration
            // observed $FF on every read, independent of the last bus
            // value). Real register semantics arrive with the APU in
            // M4; the constant is the documented B1 simplification.
            0x4000..=0x4017 => 0xFF,
            // Remaining ranges (PPU stub, test mode, expansion, no PRG
            // RAM on NROM) behave as open bus: the latch value.
            _ => self.latch,
        }
    }

    /// Direct RAM access for tests and state hashing.
    pub fn ram(&self) -> &[u8; RAM_BYTES] {
        &self.ram
    }

    /// The open-bus latch (part of the hashed state).
    pub fn latch(&self) -> u8 {
        self.latch
    }
}

impl Bus for NesBus {
    fn read(&mut self, addr: u16) -> u8 {
        let value = self.peek(addr);
        // Reading refreshes the open-bus latch with the transferred byte.
        self.latch = value;
        value
    }

    fn write(&mut self, addr: u16, value: u8) {
        // Writing drives the data bus, refreshing the latch.
        self.latch = value;
        if let 0x0000..=0x1FFF = addr {
            // PPU/APU/IO/unmapped writes are dropped in M1 (no devices
            // yet); only RAM accepts writes.
            self.ram[usize::from(addr & 0x07FF)] = value;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Simple 16 KiB PRG image with a recognizable pattern.
    fn prg16(pattern: u8) -> Vec<u8> {
        vec![pattern; 16 * 1024]
    }

    #[test]
    fn ram_mirrors_every_0x0800() {
        let mut bus = NesBus::new(prg16(0)).expect("16 KiB PRG");
        bus.write(0x0010, 0xAB);
        // The mirror at $0810 reads back the same cell.
        assert_eq!(bus.peek(0x0810), 0xAB);
        assert_eq!(bus.peek(0x1010), 0xAB);
        assert_eq!(bus.peek(0x1810), 0xAB);
        // Writes through a mirror hit the same cell.
        bus.write(0x1811, 0xCD);
        assert_eq!(bus.peek(0x0011), 0xCD);
    }

    #[test]
    fn nrom_16k_is_mirrored_at_c000() {
        let mut prg = vec![0u8; 16 * 1024];
        prg[0] = 0xAA; // $8000
        prg[0x3F00] = 0xBB; // $BF00
        let bus = NesBus::new(prg).expect("16 KiB PRG");
        assert_eq!(bus.peek(0x8000), 0xAA);
        assert_eq!(bus.peek(0xC000), 0xAA, "$C000 mirrors $8000");
        assert_eq!(bus.peek(0xBF00), 0xBB);
        assert_eq!(bus.peek(0xFF00), 0xBB, "$FF00 mirrors $BF00");
    }

    #[test]
    fn nrom_32k_maps_linearly() {
        let mut prg = vec![0u8; 32 * 1024];
        prg[0x0000] = 0x11; // $8000
        prg[0x4000] = 0x22; // $C000
        prg[0x7FFF] = 0x33; // $FFFF
        let bus = NesBus::new(prg).expect("32 KiB PRG");
        assert_eq!(bus.peek(0x8000), 0x11);
        assert_eq!(bus.peek(0xC000), 0x22);
        assert_eq!(bus.peek(0xFFFF), 0x33);
    }

    #[test]
    fn odd_prg_sizes_are_rejected_not_guessed() {
        assert_eq!(
            NesBus::new(vec![0u8; 24 * 1024]).map(|_| ()),
            Err(BusError::UnsupportedPrgSize(24 * 1024))
        );
    }

    #[test]
    fn open_bus_returns_the_last_driven_byte() {
        let mut bus = NesBus::new(prg16(0)).expect("16 KiB PRG");
        // Drive the bus with a RAM read, then read an unmapped address.
        bus.write(0x0010, 0x5A);
        assert_eq!(bus.read(0x0010), 0x5A);
        // $4020 has no device: the previous value stays on the bus.
        assert_eq!(bus.read(0x4020), 0x5A);
        // A write also drives the bus.
        bus.write(0x2000, 0x77);
        assert_eq!(bus.read(0x6000), 0x77);
        // A PRG read drives the bus too.
        assert_eq!(bus.read(0x8000), 0x00);
        assert_eq!(bus.read(0x2001), 0x00);
    }

    #[test]
    fn ppu_and_apu_writes_are_dropped_in_m1() {
        let mut bus = NesBus::new(prg16(0)).expect("16 KiB PRG");
        bus.write(0x2000, 0x0F);
        bus.write(0x4015, 0x0F);
        // Nothing was allocated behind those addresses; RAM untouched.
        assert_eq!(bus.peek(0x0000), 0);
        // PPU register reads (M2 device) still follow open bus: the last
        // value driven on the bus.
        assert_eq!(bus.read(0x2002), 0x0F);
        // APU register reads return the reference-observed fixed $FF.
        assert_eq!(bus.read(0x4015), 0xFF);
        assert_eq!(bus.peek(0x4004), 0xFF);
        assert_eq!(bus.peek(0x4017), 0xFF);
    }

    #[test]
    fn peek_does_not_disturb_the_latch() {
        let mut bus = NesBus::new(prg16(0)).expect("16 KiB PRG");
        bus.write(0x0001, 0x42);
        bus.read(0x0001); // latch = 0x42
                          // Peeking PPU space must not refresh the latch.
        let _ = bus.peek(0x2002);
        assert_eq!(bus.read(0x5000), 0x42);
    }
}
