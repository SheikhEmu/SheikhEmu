//! CPU-visible memory map of the NES for milestone M2.
//!
//! The map follows the public NESdev Wiki "CPU memory map":
//!
//! | Range        | M2 implementation                                        |
//! |--------------|----------------------------------------------------------|
//! | `$0000-$07FF`| 2 KiB internal RAM                                        |
//! | `$0008-$1FFF`| mirrors of the RAM (repeat every `$0800`)                 |
//! | `$2000-$3FFF`| PPU registers, mirrored every 8 bytes (real PPU in M2)    |
//! | `$4000-$4017`| APU registers and IO (fixed `$FF` read until M4, D-18)   |
//! | `$4014`      | OAM DMA trigger (write launches the 513/514-cycle DMA)   |
//! | `$4018-$401F`| test-mode registers (unused)                              |
//! | `$4020-$5FFF`| cartridge expansion area (open bus)                        |
//! | `$6000-$7FFF`| PRG RAM when the cartridge declares it                     |
//! | `$8000-$FFFF`| PRG ROM (NROM mapping, see [`NromPrg`])                    |
//!
//! Open bus (NESdev "CPU open bus"): reading an address with no device
//! returns the last value driven on the CPU data bus, kept in a
//! latch. Full open bus is a B2 item; the latch is the documented B1
//! simplification (spec §5.3).
//!
//! ## Clocking model (scheduler decision D-16, spec §5.0.1)
//!
//! The bus owns the master CPU-cycle counter. Every CPU read/write
//! performed through the [`cpu::Bus`] trait is exactly one CPU cycle:
//! the access is serviced first, then the PPU advances by the
//! region's dots-per-cycle ratio (3 on NTSC/Dendy; the PAL 16:5 ratio
//! via an integer accumulator). The machine true-ups any internal
//! (non-access) instruction cycles at instruction end, so the PPU is
//! exactly synchronized at every instruction boundary, and direct
//! register accesses land on their exact cycle inside an instruction.

use crate::cpu::Bus;
use crate::ppu::{Chr, Mirroring, Ppu};
use crate::timing::Region;

/// RAM size in bytes (2 KiB).
pub const RAM_BYTES: usize = 2048;

/// PRG RAM size in bytes (8 KiB at `$6000-$7FFF`).
pub const PRG_RAM_BYTES: usize = 8192;

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
            NromPrg::Mirror16k(prg) => prg[usize::from(addr & 0x3FFF)],
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

/// The M2 CPU bus: RAM + NROM PRG + optional PRG RAM + the real PPU.
#[derive(Clone, Debug)]
pub struct NesBus {
    ram: [u8; RAM_BYTES],
    prg: NromPrg,
    prg_ram: Option<Box<[u8; PRG_RAM_BYTES]>>,
    ppu: Ppu,
    region: Region,
    latch: u8,
    /// Master CPU cycle count (advances by one per serviced access
    /// and by the DMA and true-up paths).
    cycles: u64,
    /// Fractional dot accumulator: `acc += cpu_div` per cycle, and
    /// one PPU dot ticks per `ppu_div` accumulated (NTSC/Dendy 12/4
    /// = 3 dots; PAL 16/5 = 3.2 dots per cycle).
    dot_acc: u64,
}

impl NesBus {
    /// Build a bus carrying just a PRG image (CHR-RAM 8 KiB, vertical
    /// mirroring — the documented NROM-without-CHR convention).
    pub fn new(prg: Vec<u8>, region: Region) -> Result<Self, BusError> {
        Self::with_cartridge(
            prg,
            Chr::Ram(Box::new([0; 0x2000])),
            Mirroring::Vertical,
            false,
            region,
        )
    }

    /// Build a bus from cartridge data: PRG, CHR, nametable mirroring
    /// and whether `$6000-$7FFF` PRG RAM is present.
    pub fn with_cartridge(
        prg: Vec<u8>,
        chr: Chr,
        mirroring: Mirroring,
        prg_ram: bool,
        region: Region,
    ) -> Result<Self, BusError> {
        Ok(NesBus {
            ram: [0; RAM_BYTES],
            prg: NromPrg::new(prg)?,
            prg_ram: prg_ram.then(|| Box::new([0u8; PRG_RAM_BYTES])),
            ppu: Ppu::new(region, chr, mirroring),
            region,
            latch: 0,
            cycles: 0,
            dot_acc: 0,
        })
    }

    /// The master CPU cycle counter.
    pub fn cycle(&self) -> u64 {
        self.cycles
    }

    /// Set the master cycle counter (automation entry).
    pub fn set_cycles(&mut self, cycles: u64) {
        self.cycles = cycles;
    }

    /// Advance `n` CPU cycles without a bus access (the true-up for
    /// internal instruction cycles; each still ticks the PPU).
    pub fn advance_cycles(&mut self, n: u64) {
        for _ in 0..n {
            self.advance_one_cycle();
        }
    }

    /// One CPU cycle: the PPU advances by the region's dots ratio.
    fn advance_one_cycle(&mut self) {
        self.cycles += 1;
        self.dot_acc += u64::from(self.region.cpu_divisor());
        let per = u64::from(self.region.ppu_divisor());
        while self.dot_acc >= per {
            self.dot_acc -= per;
            self.ppu.tick();
        }
    }

    /// The PPU (shared with the machine for frames, hashing and CLI).
    pub fn ppu(&self) -> &Ppu {
        &self.ppu
    }

    /// Mutable PPU access (machine integration, tests).
    pub fn ppu_mut(&mut self) -> &mut Ppu {
        &mut self.ppu
    }

    /// Read without side effects (no latch refresh, no register
    /// side effects); used by the trace logger and the check tools.
    pub fn peek(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => self.ram[usize::from(addr & 0x07FF)],
            0x2000..=0x3FFF => {
                // A pure view of $2002; the other registers read as
                // open bus.
                if addr & 7 == 2 {
                    self.ppu.status_bits()
                } else {
                    self.latch
                }
            }
            // APU and IO registers: the documented fixed $FF (the
            // reference trace shows this constant; D-18) until the
            // APU arrives in M4.
            0x4000..=0x4017 => 0xFF,
            0x6000..=0x7FFF => match &self.prg_ram {
                Some(ram) => ram[usize::from(addr & 0x1FFF)],
                None => self.latch,
            },
            0x8000..=0xFFFF => self.prg.read(addr),
            _ => self.latch,
        }
    }

    /// Direct RAM access for tests and state hashing.
    pub fn ram(&self) -> &[u8; RAM_BYTES] {
        &self.ram
    }

    /// The PRG RAM bytes when present (state hashing, the blargg
    /// protocol reader).
    pub fn prg_ram(&self) -> Option<&[u8; PRG_RAM_BYTES]> {
        self.prg_ram.as_deref()
    }

    /// The open-bus latch (part of the hashed state).
    pub fn latch(&self) -> u8 {
        self.latch
    }

    /// Service a read against the devices (no clocking).
    fn service_read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x1FFF => self.ram[usize::from(addr & 0x07FF)],
            0x2000..=0x3FFF => match addr & 7 {
                2 => self.ppu.read_status(self.latch),
                4 => self.ppu.read_oam(),
                7 => self.ppu.read_data(),
                // $2000/$2001/$2003/$2005/$2006 read as open bus.
                _ => self.latch,
            },
            // APU/IO: the fixed $FF (D-18); $4014 is write-only.
            0x4000..=0x4017 => 0xFF,
            0x6000..=0x7FFF => match &self.prg_ram {
                Some(ram) => ram[usize::from(addr & 0x1FFF)],
                None => self.latch,
            },
            0x8000..=0xFFFF => self.prg.read(addr),
            _ => self.latch,
        }
    }

    /// Service a write against the devices (no clocking). Returns the
    /// OAM DMA page when the write triggers a DMA.
    fn service_write(&mut self, addr: u16, value: u8) -> Option<u8> {
        let mut dma: Option<u8> = None;
        match addr {
            0x0000..=0x1FFF => self.ram[usize::from(addr & 0x07FF)] = value,
            0x2000..=0x3FFF => match addr & 7 {
                0 => self.ppu.write_ctrl(value),
                1 => self.ppu.write_mask(value),
                // Writes to $2002 are ignored (documented).
                2 => {}
                3 => self.ppu.write_oam_addr(value),
                4 => self.ppu.write_oam_data(value),
                5 => self.ppu.write_scroll(value),
                6 => self.ppu.write_addr(value),
                _ => self.ppu.write_data(value),
            },
            0x4000..=0x4017 => {
                if addr == 0x4014 {
                    dma = Some(value);
                }
                // Other APU/IO writes are dropped until M4/M6.
            }
            0x6000..=0x7FFF => {
                if let Some(ram) = self.prg_ram.as_deref_mut() {
                    ram[usize::from(addr & 0x1FFF)] = value;
                }
            }
            0x8000..=0xFFFF => {
                // NROM has no writeable PRG.
            }
            _ => {
                // Unmapped writes are dropped.
            }
        }
        dma
    }

    /// Run the OAM DMA for `page`: one alignment cycle (two when the
    /// write ended on an odd cycle — the documented 513/514-cycle
    /// rule), then 256 read/write pairs, each ticking the PPU.
    fn run_oam_dma(&mut self, page: u8) {
        let idles = 1 + u64::from(self.cycles % 2 == 1);
        for _ in 0..idles {
            self.advance_one_cycle();
        }
        for i in 0..256u16 {
            let value = self.service_read((u16::from(page) << 8) | i);
            self.advance_one_cycle();
            self.service_write(0x2004, value);
            self.advance_one_cycle();
        }
    }
}

impl Bus for NesBus {
    fn read(&mut self, addr: u16) -> u8 {
        let value = self.service_read(addr);
        self.latch = value;
        self.advance_one_cycle();
        value
    }

    fn write(&mut self, addr: u16, value: u8) {
        self.latch = value;
        let dma = self.service_write(addr, value);
        self.advance_one_cycle();
        if let Some(page) = dma {
            self.run_oam_dma(page);
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
        let mut bus = NesBus::new(prg16(0), Region::Ntsc).expect("16 KiB PRG");
        bus.write(0x0010, 0xAB);
        assert_eq!(bus.peek(0x0810), 0xAB);
        assert_eq!(bus.peek(0x1010), 0xAB);
        assert_eq!(bus.peek(0x1810), 0xAB);
        bus.write(0x1811, 0xCD);
        assert_eq!(bus.peek(0x0011), 0xCD);
    }

    #[test]
    fn nrom_16k_is_mirrored_at_c000() {
        let mut prg = vec![0u8; 16 * 1024];
        prg[0] = 0xAA;
        prg[0x3F00] = 0xBB;
        let bus = NesBus::new(prg, Region::Ntsc).expect("16 KiB PRG");
        assert_eq!(bus.peek(0x8000), 0xAA);
        assert_eq!(bus.peek(0xC000), 0xAA, "$C000 mirrors $8000");
        assert_eq!(bus.peek(0xBF00), 0xBB);
        assert_eq!(bus.peek(0xFF00), 0xBB, "$FF00 mirrors $BF00");
    }

    #[test]
    fn nrom_32k_maps_linearly() {
        let mut prg = vec![0u8; 32 * 1024];
        prg[0x0000] = 0x11;
        prg[0x4000] = 0x22;
        prg[0x7FFF] = 0x33;
        let bus = NesBus::new(prg, Region::Ntsc).expect("32 KiB PRG");
        assert_eq!(bus.peek(0x8000), 0x11);
        assert_eq!(bus.peek(0xC000), 0x22);
        assert_eq!(bus.peek(0xFFFF), 0x33);
    }

    #[test]
    fn odd_prg_sizes_are_rejected_not_guessed() {
        assert_eq!(
            NesBus::new(vec![0u8; 24 * 1024], Region::Ntsc).map(|_| ()),
            Err(BusError::UnsupportedPrgSize(24 * 1024))
        );
    }

    #[test]
    fn open_bus_returns_the_last_driven_byte() {
        let mut bus = NesBus::new(prg16(0), Region::Ntsc).expect("16 KiB PRG");
        bus.write(0x0010, 0x5A);
        assert_eq!(bus.read(0x0010), 0x5A);
        // $4020 has no device: the previous value stays on the bus.
        assert_eq!(bus.read(0x4020), 0x5A);
        // A write also drives the bus.
        bus.write(0x5000, 0x77);
        assert_eq!(bus.read(0x6000), 0x77);
        // A PRG read drives the bus too.
        assert_eq!(bus.read(0x8000), 0x00);
        assert_eq!(bus.read(0x5000), 0x00);
    }

    #[test]
    fn ppu_registers_are_live_devices() {
        let mut bus = NesBus::new(prg16(0), Region::Ntsc).expect("16 KiB PRG");
        // $2002 first clears w set by a $2005 write...
        bus.write(0x2005, 0x00);
        assert_eq!(bus.read(0x2002) & 0x80, 0x00);
        // ...and $2006 pairs now assemble the address cleanly.
        bus.write(0x2006, 0x20);
        bus.write(0x2006, 0x00);
        bus.write(0x2007, 0x42);
        // Read it back through the buffer semantics.
        bus.write(0x2006, 0x20);
        bus.write(0x2006, 0x00);
        assert_eq!(bus.read(0x2007), 0x00, "first read is the stale buffer");
        assert_eq!(bus.read(0x2007), 0x42, "second read sees the write");
    }

    #[test]
    fn every_access_advances_the_master_cycle() {
        let mut bus = NesBus::new(prg16(0), Region::Ntsc).expect("16 KiB PRG");
        assert_eq!(bus.cycle(), 0);
        bus.read(0x0000);
        assert_eq!(bus.cycle(), 1);
        bus.write(0x0001, 0xFF);
        assert_eq!(bus.cycle(), 2);
        bus.advance_cycles(3);
        assert_eq!(bus.cycle(), 5);
        // NTSC: 3 dots per cycle — 5 cycles = 15 dots = (0, 15).
        assert_eq!(bus.ppu().position(), (0, 15));
    }

    #[test]
    fn pal_advances_16_dots_per_5_cycles() {
        let mut bus = NesBus::new(prg16(0), Region::Pal).expect("16 KiB PRG");
        bus.advance_cycles(5);
        assert_eq!(bus.cycle(), 5);
        assert_eq!(bus.ppu().position(), (0, 16));
        // 312 lines per PAL frame.
        bus.advance_cycles(341 * 312 / 3 - 5);
        assert_eq!(bus.ppu().frame(), 1);
    }

    #[test]
    fn oam_dma_transfers_256_bytes_and_charges_513_or_514() {
        let mut bus = NesBus::with_cartridge(
            prg16(0),
            Chr::Ram(Box::new([0; 0x2000])),
            Mirroring::Vertical,
            true,
            Region::Ntsc,
        )
        .expect("16 KiB PRG");
        // Fill CPU page 2 ($0200-$02FF, the internal RAM) — the DMA
        // source — with an identifiable sequence (service writes only:
        // no clocking, so the parity below is stable).
        for i in 0..256u16 {
            let _ = bus.service_write(0x0200 | i, (i & 0xFF) as u8);
        }
        // The write is the very first cycle (odd) -> 514 DMA cycles.
        assert_eq!(bus.cycle(), 0);
        bus.write(0x4014, 0x02);
        assert_eq!(bus.cycle(), 1 + 514);
        assert_eq!(bus.ppu().oam()[0], 0x00);
        assert_eq!(bus.ppu().oam()[255], 0xFF);

        // cycles is now 515 (odd): a write now lands on cycle 516
        // (even) -> 513 DMA cycles.
        bus.write(0x4014, 0x02);
        assert_eq!(bus.cycle(), 515 + 1 + 513);
    }

    #[test]
    fn prg_ram_is_read_write_when_declared() {
        let mut bus = NesBus::with_cartridge(
            prg16(0),
            Chr::Ram(Box::new([0; 0x2000])),
            Mirroring::Vertical,
            true,
            Region::Ntsc,
        )
        .expect("16 KiB PRG");
        bus.write(0x6000, 0x2A);
        assert_eq!(bus.read(0x6000), 0x2A);
        // The window is a linear 8 KiB at $6000-$7FFF.
        bus.write(0x7FFF, 0x33);
        assert_eq!(bus.peek(0x7FFF), 0x33);
        // Without PRG RAM the range is open bus.
        let mut bare = NesBus::new(prg16(0), Region::Ntsc).expect("16 KiB PRG");
        bare.write(0x6000, 0x11);
        assert_eq!(bare.read(0x6000), 0x11, "open bus echoes the write");
    }

    #[test]
    fn peek_does_not_disturb_the_latch_or_flags() {
        let mut bus = NesBus::new(prg16(0), Region::Ntsc).expect("16 KiB PRG");
        bus.write(0x0001, 0x42);
        bus.read(0x0001);
        // Peeking PPU space must not refresh the latch.
        let _ = bus.peek(0x2002);
        assert_eq!(bus.read(0x5000), 0x42);
    }
}
