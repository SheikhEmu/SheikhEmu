//! CPU-visible memory map of the NES.
//!
//! The map follows the public NESdev Wiki "CPU memory map":
//!
//! | Range        | M4 implementation                                        |
//! |--------------|----------------------------------------------------------|
//! | `$0000-$07FF`| 2 KiB internal RAM                                        |
//! | `$0008-$1FFF`| mirrors of the RAM (repeat every `$0800`)                 |
//! | `$2000-$3FFF`| PPU registers, mirrored every 8 bytes (real PPU in M2)    |
//! | `$4000-$4013`| APU channel registers (real APU in M4; reads keep the D-18 reference `$FF`) |
//! | `$4014`      | OAM DMA trigger (write launches the 513/514-cycle DMA)   |
//! | `$4015`      | APU status (live read since M4.2)                        |
//! | `$4016`      | strobe write (both joypads) / joypad 1 read (M3)        |
//! | `$4017`      | joypad 2 read (M3) / APU frame counter write (M4.2)      |
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
//! ## Clocking model (scheduler decision D-20, spec §5.0.1)
//!
//! The bus owns the master CPU-cycle counter. Every CPU read/write
//! performed through the [`cpu::Bus`] trait is exactly one CPU cycle:
//! the access is serviced first, then the PPU advances by the
//! region's dots-per-cycle ratio (3 on NTSC/Dendy; the PAL 16:5 ratio
//! via an integer accumulator), then the APU ticks once — and the DMC
//! memory reader's fetches stall the CPU as deferred cycles (D-38
//! B1: three idle cycles plus the fetch cycle) charged to the same
//! master clock.

use crate::apu::Apu;
use crate::cpu::Bus;
use crate::input::Controllers;
use crate::mapper::{CartSpec, Cartridge, MapperError, Registry};
use crate::ppu::{Chr, Mirroring, Ppu};
use crate::timing::Region;

/// RAM size in bytes (2 KiB).
pub const RAM_BYTES: usize = 2048;

/// PRG RAM size in bytes (8 KiB at `$6000-$7FFF`).
pub const PRG_RAM_BYTES: usize = 8192;

/// Failures while constructing a bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusError {
    /// PRG image size is not valid for the board (in bytes).
    UnsupportedPrgSize(usize),
    /// The mapper id has no implementation yet.
    UnsupportedMapper(u16),
    /// The submapper combination is refused.
    UnsupportedSubmapper(u16, u8),
    /// CHR image size is not valid for the board.
    BadChrSize(usize),
}

impl From<MapperError> for BusError {
    fn from(err: MapperError) -> Self {
        match err {
            MapperError::Unsupported(mapper) => BusError::UnsupportedMapper(mapper),
            MapperError::UnsupportedSubmapper(mapper, submapper) => {
                BusError::UnsupportedSubmapper(mapper, submapper)
            }
            MapperError::BadPrgSize(size) => BusError::UnsupportedPrgSize(size),
            MapperError::BadChrSize(size) => BusError::BadChrSize(size),
        }
    }
}

impl std::fmt::Display for BusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BusError::UnsupportedPrgSize(size) => {
                write!(f, "PRG size {size} bytes is not valid for this board")
            }
            BusError::UnsupportedMapper(mapper) => write!(
                f,
                "mapper {mapper} is not implemented (T1 set: 0,2,3,7,11,34,66,71,87,101 \
                 plus the minimal MMC1 1)"
            ),
            BusError::UnsupportedSubmapper(mapper, submapper) => write!(
                f,
                "mapper {mapper} submapper {submapper} is refused (34/1 = NINA-001/002 \
                 waits for the mapper families milestone)"
            ),
            BusError::BadChrSize(size) => {
                write!(f, "CHR size {size} bytes is not valid for this board")
            }
        }
    }
}

impl std::error::Error for BusError {}

/// The M4 CPU bus: RAM + the cartridge (T1 mapper framework) +
/// optional PRG RAM + the real PPU, controllers and APU.
#[derive(Clone, Debug)]
pub struct NesBus {
    ram: [u8; RAM_BYTES],
    cart: Cartridge,
    prg_ram: Option<Box<[u8; PRG_RAM_BYTES]>>,
    ppu: Ppu,
    region: Region,
    latch: u8,
    /// Master CPU cycle count (advances by one per serviced access
    /// and by the DMA and true-up paths).
    cycles: u64,
    /// The two standard controllers ($4016/$4017, M3).
    pads: Controllers,
    /// The APU (M4.2): five channels, the frame sequencer, the
    /// deterministic integer mixer and the PCM stream.
    apu: Apu,
    /// Fractional dot accumulator: `acc += cpu_div` per cycle, and
    /// one PPU dot ticks per `ppu_div` accumulated (NTSC/Dendy 12/4
    /// = 3 dots; PAL 16/5 = 3.2 dots per cycle).
    dot_acc: u64,
}

impl NesBus {
    /// Build a bus carrying just a PRG image (NROM, CHR-RAM 8 KiB,
    /// vertical mirroring — the documented NROM-without-CHR
    /// convention).
    pub fn new(prg: Vec<u8>, region: Region) -> Result<Self, BusError> {
        Self::with_mapper(
            prg,
            0,
            0,
            Chr::Ram(Box::new([0; 0x2000])),
            Mirroring::Vertical,
            false,
            region,
        )
    }

    /// Build a bus from cartridge data for mapper 0 (NROM).
    pub fn with_cartridge(
        prg: Vec<u8>,
        chr: Chr,
        mirroring: Mirroring,
        prg_ram: bool,
        region: Region,
    ) -> Result<Self, BusError> {
        Self::with_mapper(prg, 0, 0, chr, mirroring, prg_ram, region)
    }

    /// Build a bus for a mapper and submapper (the T1 set of M3 plus
    /// the minimal MMC1 of D-22; the Registry validates and refuses
    /// the rest). The initial nametable mirroring comes from the
    /// header; boards that own mirroring (MMC1, AxROM, Camerica)
    /// re-apply it on their register writes — AxROM applies its
    /// power-on single-screen page immediately.
    pub fn with_mapper(
        prg: Vec<u8>,
        mapper: u16,
        submapper: u8,
        chr: Chr,
        mirroring: Mirroring,
        prg_ram: bool,
        region: Region,
    ) -> Result<Self, BusError> {
        let chr_for_ppu = chr.clone();
        let cart = Registry::build(CartSpec {
            mapper,
            submapper,
            prg,
            chr,
            mirroring,
        })?;
        let power_mirroring = match cart.mapper() {
            7 => cart.mirroring(),
            _ => mirroring,
        };
        let mut ppu = Ppu::new(region, chr_for_ppu, power_mirroring);
        ppu.set_chr_bank(cart.chr_bank());
        Ok(NesBus {
            ram: [0; RAM_BYTES],
            cart,
            prg_ram: prg_ram.then(|| Box::new([0u8; PRG_RAM_BYTES])),
            ppu,
            region,
            latch: 0,
            cycles: 0,
            dot_acc: 0,
            pads: Controllers::default(),
            apu: Apu::new(region),
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
    /// internal instruction cycles; each still ticks the PPU and the
    /// APU, and services DMC fetches).
    pub fn advance_cycles(&mut self, n: u64) {
        for _ in 0..n {
            self.advance_one_cycle();
        }
    }

    /// One CPU cycle of the raw master clock: the PPU dots of the
    /// cycle, then the APU (spec §5.0.1 order: CPU, PPU, APU). No DMC
    /// fetch is serviced inside (the caller decides — the OAM DMA
    /// defers DMC fetches per D-38 B1).
    fn tick_cycle(&mut self) {
        self.cycles += 1;
        self.dot_acc += u64::from(self.region.cpu_divisor());
        let per = u64::from(self.region.ppu_divisor());
        while self.dot_acc >= per {
            self.dot_acc -= per;
            self.ppu.tick();
        }
        self.apu.tick();
    }

    /// One CPU cycle plus the DMC memory-reader stall: when the APU
    /// wants a sample byte, the CPU is suspended for four cycles —
    /// three idle cycles, then the fetch cycle itself that reads the
    /// byte from the cartridge ("APU DMC"; D-38 B1, the alignment
    /// variants are B2). The stall cycles tick the PPU and APU as
    /// normal cycles of the same master clock.
    fn advance_one_cycle(&mut self) {
        self.tick_cycle();
        if let Some(addr) = self.apu.take_dma_request() {
            for _ in 0..3 {
                self.tick_cycle();
            }
            self.tick_cycle();
            let value = self.service_read(addr);
            self.apu.dma_complete(value);
        }
    }

    /// The APU (shared with the machine for PCM capture, the state
    /// hash and the CLI).
    pub fn apu(&self) -> &Apu {
        &self.apu
    }

    /// Mutable APU access (machine integration, tests).
    pub fn apu_mut(&mut self) -> &mut Apu {
        &mut self.apu
    }

    /// The PPU (shared with the machine for frames, hashing and CLI).
    pub fn ppu(&self) -> &Ppu {
        &self.ppu
    }

    /// Mutable PPU access (machine integration, tests).
    pub fn ppu_mut(&mut self) -> &mut Ppu {
        &mut self.ppu
    }

    /// The two controllers ($4016/$4017): live button snapshots are
    /// pushed per frame by the host (CLI script / autopilot /
    /// frontend keyboard) before the frame runs.
    pub fn controllers_mut(&mut self) -> &mut Controllers {
        &mut self.pads
    }

    /// A side-effect-free view of the controllers (trace logger).
    pub fn controllers(&self) -> &Controllers {
        &self.pads
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
            // Joypads: the current serial output, no shifter advance.
            0x4016 => self.pads.peek(0),
            0x4017 => self.pads.peek(1),
            // APU registers: the D-18 reference `$FF` view is kept
            // for the trace logger and the check tools (the reference
            // log's annotation convention — byte-identical nestest
            // calibration). The live CPU read of `$4015` goes through
            // `service_read`; the write-only registers keep the
            // documented B1 constant until the B2 open-bus pass.
            0x4000..=0x4017 => 0xFF,
            0x6000..=0x7FFF => match &self.prg_ram {
                Some(ram) => ram[usize::from(addr & 0x1FFF)],
                None => self.latch,
            },
            0x8000..=0xFFFF => self.cart.prg_read(addr),
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

    /// Mapper register state for the determinism hash.
    pub fn mapper_state(&self) -> (u8, u8, u8, u8) {
        self.cart.state()
    }

    /// The board identity (reports).
    pub fn board_name(&self) -> &'static str {
        self.cart.board_name()
    }

    /// The mapper number.
    pub fn mapper(&self) -> u16 {
        self.cart.mapper()
    }

    /// Apply the cartridge's dynamic mirroring / CHR window to the
    /// PPU after a register write (cheap and idempotent).
    fn sync_cartridge_effects(&mut self) {
        let mirroring = self.cart.mirroring();
        if self.ppu.mirroring() != mirroring {
            self.ppu.set_mirroring(mirroring);
        }
        self.ppu.set_chr_bank(self.cart.chr_bank());
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
            // Joypads (M3): $4016 = port 1, $4017 = port 2; a read
            // advances the shifter (the real side effect).
            0x4016 => self.pads.read(0),
            0x4017 => self.pads.read(1),
            // APU status: the live read (M4.2) with the frame flag
            // cleared by the read; bit 5 is open bus (the last driven
            // bus value). The write-only registers below keep the
            // D-18 reference $FF for reads.
            0x4015 => {
                let status = self.apu.read_status();
                (status & 0xDF) | (self.latch & 0x20)
            }
            0x4000..=0x4017 => 0xFF,
            0x6000..=0x7FFF => match &self.prg_ram {
                Some(ram) => ram[usize::from(addr & 0x1FFF)],
                None => self.latch,
            },
            0x8000..=0xFFFF => self.cart.prg_read(addr),
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
                // $4016 bit 0 strobes both joypads (M3).
                if addr == 0x4016 {
                    self.pads.write_strobe(value);
                }
                // The APU registers (M4.2): channel writes, the status
                // register and the frame-counter write with the
                // documented 3-or-4-cycle reset delay (the delay rule
                // uses the APU half-cycle phase: a write serviced just
                // before an APU-rate cycle counts as "during" it).
                if addr == 0x4015 {
                    self.apu.write_status(value);
                } else if addr == 0x4017 {
                    let during_apu_cycle = !self.apu.half_phase();
                    self.apu.write_frame_counter(value, during_apu_cycle);
                } else if addr != 0x4014 && addr != 0x4016 {
                    self.apu.write_register(addr, value);
                }
            }
            0x6000..=0x7FFF => {
                if let Some(ram) = self.prg_ram.as_deref_mut() {
                    ram[usize::from(addr & 0x1FFF)] = value;
                }
                // The J87 family (mappers 87/101) listens on
                // $6000-$7FFF itself; every other board's register
                // range starts at $8000 and the call is a no-op.
                self.cart.cpu_write(addr, value);
                self.sync_cartridge_effects();
            }
            0x8000..=0xFFFF => {
                // Mapper register writes (the Cartridge filters the
                // per-board address ranges itself).
                self.cart.cpu_write(addr, value);
                self.sync_cartridge_effects();
            }
            _ => {
                // Unmapped writes are dropped.
            }
        }
        dma
    }

    /// Run the OAM DMA for `page`: one alignment cycle (two when the
    /// write ended on an odd cycle — the documented 513/514-cycle
    /// rule), then 256 read/write pairs, each ticking the PPU and the
    /// APU. DMC fetches requested during the transfer are deferred to
    /// the next regular cycle (D-38 B1 defers the DMC/OAM interleave).
    fn run_oam_dma(&mut self, page: u8) {
        let idles = 1 + u64::from(self.cycles % 2 == 1);
        for _ in 0..idles {
            self.tick_cycle();
        }
        for i in 0..256u16 {
            let value = self.service_read((u16::from(page) << 8) | i);
            self.tick_cycle();
            self.service_write(0x2004, value);
            self.tick_cycle();
        }
    }
}

impl Bus for NesBus {
    fn read(&mut self, addr: u16) -> u8 {
        let value = self.service_read(addr);
        // $4015 is internal to the CPU: the read value does not
        // affect the open-bus latch (NESdev "APU").
        if addr != 0x4015 {
            self.latch = value;
        }
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

    #[test]
    fn joypads_are_live_devices_with_the_strobe_protocol() {
        let mut bus = NesBus::new(prg16(0), Region::Ntsc).expect("16 KiB PRG");
        // No buttons pressed: strobe + 8 reads give a released report.
        bus.write(0x4016, 1);
        bus.write(0x4016, 0);
        for _ in 0..8 {
            assert_eq!(bus.read(0x4016) & 1, 0);
        }
        assert_eq!(bus.read(0x4016), 0x41, "after 8: official 1s");
        // Press A on port 1: the next cycle reports it.
        bus.controllers_mut().set_live(0, crate::input::BTN_A);
        bus.write(0x4016, 1);
        bus.write(0x4016, 0);
        assert_eq!(bus.read(0x4016) & 1, 1, "slot A pressed");
        assert_eq!(bus.read(0x4016) & 1, 0, "slot B released");
        // Port 2 stays independent through $4017.
        bus.controllers_mut().set_live(1, crate::input::BTN_RIGHT);
        bus.write(0x4016, 1);
        bus.write(0x4016, 0);
        for slot in 0..7 {
            assert_eq!(bus.read(0x4017) & 1, 0, "port 2 slot {slot}");
        }
        assert_eq!(bus.read(0x4017) & 1, 1, "port 2 Right pressed");
    }
}

#[cfg(test)]
mod chr_window_tests {
    use super::*;

    /// A 32 KiB PRG image plus a 4-bank CHR ROM with signatures at the
    /// bank starts/ends (the CNROM test shape).
    fn chr4() -> Chr {
        let mut rom = vec![0u8; 4 * 0x2000];
        for bank in 0..4 {
            rom[bank * 0x2000] = 0xC0 | bank as u8;
            rom[bank * 0x2000 + 0x1FFF] = 0xC4 | bank as u8;
        }
        Chr::Rom(rom)
    }

    #[test]
    fn cnrom_chr_window_reads_through_2007() {
        let mut bus = NesBus::with_mapper(
            vec![0xEA; 32 * 1024],
            3,
            0,
            chr4(),
            Mirroring::Vertical,
            false,
            Region::Ntsc,
        )
        .expect("cnrom");
        // Bank 0 through the window at $0000 and $1FFF.
        bus.write(0x8000, 0x00);
        bus.write(0x2006, 0x00);
        bus.write(0x2006, 0x00);
        let _ = bus.read(0x2007); // discard the buffered read
        assert_eq!(bus.read(0x2007), 0xC0, "bank 0 window start");
        bus.write(0x2006, 0x1F);
        bus.write(0x2006, 0xFF);
        let _ = bus.read(0x2007);
        assert_eq!(bus.read(0x2007), 0xC4, "bank 0 window end");
        // Bank 3.
        bus.write(0x8000, 0x03);
        bus.write(0x2006, 0x00);
        bus.write(0x2006, 0x00);
        let _ = bus.read(0x2007);
        assert_eq!(bus.read(0x2007), 0xC3, "bank 3 window start");
    }
}
