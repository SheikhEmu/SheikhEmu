//! The cartridge mapper framework — M3/T1 scope of spec §6.1.
//!
//! Behavior of every board below is taken from its public NESdev Wiki
//! page (verified through the public archive, D-07), not from
//! guessing; per-board citations live in `docs/MAPPER_MATRIX.md`:
//!
//! - **0 NROM**: fixed 16 KiB mirrored or 32 KiB linear (M2 code).
//! - **1 MMC1**: the minimal gate board of D-22 (serial protocol),
//!   `implemented-untested` in the matrix until the M5 gate.
//! - **2 UxROM**: write `$8000-$FFFF`; bits 2-0 (UNROM) or 3-0
//!   (UOROM) select the 16 KiB bank at `$8000`; `$C000` fixed to the
//!   last bank. Bus conflicts exist on real boards and games work
//!   around them in software — not emulated at B1 (documented).
//! - **3 CNROM**: write `$8000-$FFFF`; value bits 0-1 select the 8 KiB
//!   CHR bank (oversize images extend the mask); PRG 32 KiB unbanked.
//!   AND-type bus conflicts on submapper 2 boards — not emulated at
//!   B1 (documented).
//! - **7 AxROM**: write `$8000-$FFFF`; bits 0-2 select the 32 KiB bank
//!   at `$8000`; bit 4 selects the 1 KiB single-screen CIRAM page.
//!   CHR is 8 KiB RAM. AMROM/AOROM bus conflicts — not emulated at B1.
//! - **11 ColorDreams**: write `$8000-$FFFF`; bits 0-1 select the
//!   32 KiB PRG bank, bits 4-7 the 8 KiB CHR bank. Bus conflicts:
//!   yes — not emulated at B1 (documented).
//! - **34 BNROM** (submapper 2, or 0 with CHR ≤ 8 KiB per the wiki
//!   heuristic): write `$8000-$FFFF`; value bits select the 32 KiB
//!   bank. Submapper 1 (NINA-001/002) is refused explicitly.
//!   AND-type bus conflicts — not emulated at B1.
//! - **66 GxROM**: write `$8000-$FFFF`; bits 6-7 select the 32 KiB
//!   PRG bank, bits 0-1 the 8 KiB CHR bank. Bus conflicts: yes — not
//!   emulated at B1.
//! - **71 Camerica**: bank select on writes `$C000-$FFFF` (bits 0-3
//!   of the value, 16 KiB units at `$8000`, `$C000` fixed last). The
//!   one-screen mirroring register (`$8000-$9FFF`, bit 4) exists only
//!   on the BF9097 board; per the documented FCEUX compatibility rule
//!   the default (submapper 0) applies single-screen mirroring only
//!   when `$9000-$9FFF` is written, ignoring `$8000-$8FFF`. No bus
//!   conflicts.
//! - **87 J87 (Jaleco)**: PRG fixed (16 KiB mirrored or 32 KiB); CHR
//!   bank register on writes `$6000-$7FFF` with the two bank bits
//!   **reversed** (bit 0 = high bit, bit 1 = low bit). No bus
//!   conflicts.
//! - **101 (J87 misdump)**: identical to 87 but the bank bits are in
//!   normal order (`CCCC CCHL`, bits 2-7 are the oversize extension).
//!
//! The §6.1 `Mapper` trait is realized here in its compressed T1
//! form: CPU-side read/write, mirroring, the CHR window, hashed state
//! and board info. The IRQ/A12/scanline/audio hooks of the full trait
//! become real with MMC3 in M5; the registry builds boards from
//! `(mapper, submapper)` and refuses unknown ids with an explicit
//! error (the §6.1 fallback target is MMC3, which arrives in M5 —
//! until then the refusal message is the documented explicit failure
//! path, no black screens).

use crate::ppu::{Chr, Mirroring};

/// 16 KiB PRG unit.
const PRG_16K: usize = 0x4000;
/// 32 KiB PRG unit.
const PRG_32K: usize = 0x8000;
/// 8 KiB CHR unit.
const CHR_8K: usize = 0x2000;

/// The minimal MMC1 (mapper 1) of D-22 — moved verbatim from the M2
/// bus so its calibrated behavior (ppu_vbl_nmi gate) is untouched.
#[derive(Clone, Debug)]
pub struct Mmc1 {
    image: Vec<u8>,
    /// Register 0: screen mode bits 0-1, PRG mode bits 2-3.
    pub control: u8,
    /// The 5-bit serial shift accumulator and its bit counter.
    shift: u8,
    bits: u8,
    /// Register 3: the PRG bank select.
    pub prg_bank: u8,
}

impl Mmc1 {
    /// Build from an image whose length is a multiple of 16 KiB.
    fn new(image: Vec<u8>) -> Result<Self, MapperError> {
        if image.is_empty() || !image.len().is_multiple_of(PRG_16K) {
            return Err(MapperError::BadPrgSize(image.len()));
        }
        Ok(Mmc1 {
            image,
            // Power-up state: PRG mode 3 (switch $8000 / fixed
            // $C000), screen mode 3.
            control: 0x0C,
            shift: 0,
            bits: 0,
            prg_bank: 0,
        })
    }

    fn banks(&self) -> usize {
        self.image.len() / PRG_16K
    }

    /// The 16 KiB bank serving `$8000-$BFFF`.
    fn low_bank(&self) -> usize {
        let bank = usize::from(self.prg_bank & 0x0F);
        match (self.control >> 2) & 3 {
            0 => bank & !1, // 32 KiB switch: pair start.
            1 => 0,         // fixed first bank.
            _ => bank,      // 2/3: switchable low half.
        }
    }

    /// The 16 KiB bank serving `$C000-$FFFF`.
    fn high_bank(&self) -> usize {
        let bank = usize::from(self.prg_bank & 0x0F);
        let last = self.banks() - 1;
        match (self.control >> 2) & 3 {
            0 => (bank & !1) + 1, // 32 KiB switch: pair second.
            1 => bank,            // switchable high half.
            _ => last,            // 2/3: fixed last bank.
        }
    }

    fn read(&self, addr: u16) -> u8 {
        let bank = if addr < 0xC000 {
            self.low_bank()
        } else {
            self.high_bank()
        };
        self.image[bank * PRG_16K + usize::from(addr & 0x3FFF)]
    }

    /// Feed one CPU write of the serial protocol.
    fn write(&mut self, addr: u16, value: u8) {
        if value & 0x80 != 0 {
            self.shift = 0;
            self.bits = 0;
            // The documented reset side effect: PRG mode back to 3.
            self.control = (self.control & 0x0F) | 0x0C;
            return;
        }
        self.shift |= (value & 1) << self.bits;
        self.bits += 1;
        if self.bits == 5 {
            let reg = (addr >> 13) & 3;
            let latched = self.shift;
            self.shift = 0;
            self.bits = 0;
            match reg {
                0 => self.control = latched & 0x0F,
                3 => self.prg_bank = latched & 0x0F,
                _ => {} // CHR bank registers: no effect with 8 KiB RAM.
            }
        }
    }

    /// The screen mode the control register selects.
    fn mirroring(&self) -> Mirroring {
        match self.control & 3 {
            0 => Mirroring::OneScreenA,
            1 => Mirroring::OneScreenB,
            2 => Mirroring::Vertical,
            _ => Mirroring::Horizontal,
        }
    }
}

/// The Camerica mirroring state: fixed from the header until the
/// documented `$9000-$9FFF` one-screen write arrives (FCEUX compat).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CamericaMirror {
    Header,
    OneScreenA,
    OneScreenB,
}

/// The concrete board state of a cartridge.
#[derive(Clone, Debug)]
enum Board {
    /// Mapper 0: fixed NROM (16 KiB mirrored or 32 KiB linear).
    Nrom { mirror16k: bool },
    /// Mapper 1: the minimal MMC1 (D-22).
    Mmc1(Mmc1),
    /// Mapper 2: 16 KiB bank at `$8000`, fixed last at `$C000`.
    Uxrom { bank: u8 },
    /// Mapper 3: 8 KiB CHR bank window, PRG fixed.
    Cnrom { bank: u8 },
    /// Mapper 7: 32 KiB bank at `$8000` + single-screen page bit.
    Axrom { bank: u8, page1: bool },
    /// Mapper 11: 32 KiB PRG + 8 KiB CHR banks in one register.
    ColorDreams { prg_bank: u8, chr_bank: u8 },
    /// Mapper 34 (submapper 2 / heuristic 0): 32 KiB bank at `$8000`.
    Bnrom { bank: u8 },
    /// Mapper 66: 32 KiB PRG (bits 6-7) + 8 KiB CHR (bits 0-1).
    Gxrom { prg_bank: u8, chr_bank: u8 },
    /// Mapper 71: 16 KiB bank via `$C000-$FFFF`, one-screen via
    /// `$9000-$9FFF` (compat rule).
    Camerica { bank: u8, mirror: CamericaMirror },
    /// Mapper 87 (J87): PRG fixed, CHR bank bits reversed.
    J87 { bank: u8 },
    /// Mapper 101 (J87 misdump): PRG fixed, CHR bank bits in order.
    J101 { bank: u8 },
}

impl Board {
    /// The iNES mapper number of this board (state-independent).
    fn id(&self) -> u16 {
        match self {
            Board::Nrom { .. } => 0,
            Board::Mmc1(_) => 1,
            Board::Uxrom { .. } => 2,
            Board::Cnrom { .. } => 3,
            Board::Axrom { .. } => 7,
            Board::ColorDreams { .. } => 11,
            Board::Bnrom { .. } => 34,
            Board::Gxrom { .. } => 66,
            Board::Camerica { .. } => 71,
            Board::J87 { .. } => 87,
            Board::J101 { .. } => 101,
        }
    }

    /// The human board name (reports, the mapper matrix).
    fn name(&self) -> &'static str {
        match self {
            Board::Nrom { .. } => "NROM",
            Board::Mmc1(_) => "MMC1 (minimal, D-22)",
            Board::Uxrom { .. } => "UxROM (UNROM/UOROM)",
            Board::Cnrom { .. } => "CNROM",
            Board::Axrom { .. } => "AxROM",
            Board::ColorDreams { .. } => "ColorDreams",
            Board::Bnrom { .. } => "BNROM",
            Board::Gxrom { .. } => "GxROM (GNROM/MHROM)",
            Board::Camerica { .. } => "Camerica (BF9093/96/97)",
            Board::J87 { .. } => "J87 (Jaleco)",
            Board::J101 { .. } => "J87 misdump (mapper 101)",
        }
    }

    /// Hashed state (four bytes; the determinism signature).
    fn state(&self) -> (u8, u8, u8, u8) {
        match self {
            Board::Nrom { .. } => (0, 0, 0, 0),
            Board::Mmc1(m) => (m.control, m.prg_bank, m.shift, m.bits),
            Board::Uxrom { bank } => (*bank, 0, 0, 0),
            Board::Cnrom { bank } => (*bank, 0, 0, 0),
            Board::Axrom { bank, page1 } => (*bank, u8::from(*page1), 0, 0),
            Board::ColorDreams { prg_bank, chr_bank } => (*prg_bank, *chr_bank, 0, 0),
            Board::Bnrom { bank } => (*bank, 0, 0, 0),
            Board::Gxrom { prg_bank, chr_bank } => (*prg_bank, *chr_bank, 0, 0),
            Board::Camerica { bank, mirror } => {
                let code = match mirror {
                    CamericaMirror::Header => 0u8,
                    CamericaMirror::OneScreenA => 1,
                    CamericaMirror::OneScreenB => 2,
                };
                (*bank, code, 0, 0)
            }
            Board::J87 { bank } | Board::J101 { bank } => (*bank, 0, 0, 0),
        }
    }
}

/// A built cartridge: PRG image, CHR memory, the board state and the
/// header facts the board needs.
#[derive(Clone, Debug)]
pub struct Cartridge {
    prg: Vec<u8>,
    chr: Chr,
    board: Board,
    /// Header nametable arrangement (fixed-mirroring boards).
    header_mirroring: Mirroring,
}

impl Cartridge {
    /// The iNES mapper number.
    pub fn mapper(&self) -> u16 {
        self.board.id()
    }

    /// The board name (reports).
    pub fn board_name(&self) -> &'static str {
        self.board.name()
    }

    /// Read a PRG byte at a CPU address in `$8000-$FFFF`.
    pub fn prg_read(&self, addr: u16) -> u8 {
        match &self.board {
            Board::Nrom { mirror16k } => {
                if *mirror16k {
                    self.prg[usize::from(addr & 0x3FFF)]
                } else {
                    self.prg[usize::from(addr & 0x7FFF)]
                }
            }
            Board::Mmc1(mmc1) => mmc1.read(addr),
            Board::Uxrom { bank } | Board::Camerica { bank, .. } => {
                let bank = usize::from(*bank);
                if addr < 0xC000 {
                    self.prg[bank * PRG_16K + usize::from(addr & 0x3FFF)]
                } else {
                    // Fixed to the last bank.
                    let last = self.prg.len() / PRG_16K - 1;
                    self.prg[last * PRG_16K + usize::from(addr & 0x3FFF)]
                }
            }
            Board::Axrom { bank, .. }
            | Board::Bnrom { bank }
            | Board::ColorDreams { prg_bank: bank, .. }
            | Board::Gxrom { prg_bank: bank, .. } => {
                let bank = usize::from(*bank);
                self.prg[bank * PRG_32K + usize::from(addr & 0x7FFF)]
            }
            // PRG fixed boards: 16 KiB mirrored or 32 KiB linear.
            Board::Cnrom { .. } | Board::J87 { .. } | Board::J101 { .. } => {
                if self.prg.len() == PRG_16K {
                    self.prg[usize::from(addr & 0x3FFF)]
                } else {
                    self.prg[usize::from(addr & 0x7FFF)]
                }
            }
        }
    }

    /// Feed one CPU write that can reach a mapper register; the
    /// caller routes `$8000-$FFFF` (and `$6000-$7FFF` for the J87
    /// family) here, and the board's own register range filters it.
    pub fn cpu_write(&mut self, addr: u16, value: u8) {
        // Register ranges: the J87 family (87/101) listens on
        // `$6000-$7FFF` only; every other board's registers start at
        // `$8000`. Writes outside a board's range change nothing.
        let j87 = matches!(self.board, Board::J87 { .. } | Board::J101 { .. });
        if j87 == (addr >= 0x8000) {
            return;
        }
        // Bank masks computed up front (the match holds `&mut
        // self.board`).
        let prg_mask = self.prg_bank_mask();
        let prg16_mask = self.prg16_bank_mask();
        let chr_mask = self.chr_bank_mask();
        match &mut self.board {
            Board::Nrom { .. } => {}
            Board::Mmc1(mmc1) => mmc1.write(addr, value),
            Board::Uxrom { bank } => {
                *bank = value & prg16_mask;
            }
            Board::Bnrom { bank } => {
                *bank = value & prg_mask;
            }
            Board::Cnrom { bank } => {
                *bank = value & chr_mask;
            }
            Board::Axrom { bank, page1 } => {
                *bank = value & 0x07 & prg_mask;
                *page1 = value & 0x10 != 0;
            }
            Board::ColorDreams { prg_bank, chr_bank } => {
                *prg_bank = value & 0x03 & prg_mask;
                *chr_bank = (value >> 4) & 0x0F & chr_mask;
            }
            Board::Gxrom { prg_bank, chr_bank } => {
                *prg_bank = (value >> 6) & 0x03 & prg_mask;
                *chr_bank = value & 0x03 & chr_mask;
            }
            Board::Camerica { bank, mirror } => {
                if addr >= 0xC000 {
                    *bank = value & 0x0F & prg16_mask;
                } else if (0x9000..0xA000).contains(&addr) {
                    // The documented FCEUX compatibility rule: apply
                    // the one-screen page only on $9000-$9FFF writes.
                    *mirror = if value & 0x10 != 0 {
                        CamericaMirror::OneScreenB
                    } else {
                        CamericaMirror::OneScreenA
                    };
                }
            }
            Board::J87 { bank } => {
                // Bits reversed: bit 0 = high bit, bit 1 = low bit.
                let reversed = ((value & 1) << 1) | ((value >> 1) & 1);
                *bank = reversed & chr_mask;
            }
            Board::J101 { bank } => {
                *bank = value & chr_mask;
            }
        }
    }

    /// The effective nametable arrangement.
    pub fn mirroring(&self) -> Mirroring {
        match &self.board {
            Board::Mmc1(mmc1) => mmc1.mirroring(),
            Board::Axrom { page1, .. } => {
                if *page1 {
                    Mirroring::OneScreenB
                } else {
                    Mirroring::OneScreenA
                }
            }
            Board::Camerica { mirror, .. } => match mirror {
                CamericaMirror::Header => self.header_mirroring,
                CamericaMirror::OneScreenA => Mirroring::OneScreenA,
                CamericaMirror::OneScreenB => Mirroring::OneScreenB,
            },
            _ => self.header_mirroring,
        }
    }

    /// The 8 KiB CHR bank window the PPU should show (CHR ROM
    /// banking boards; 0 otherwise).
    pub fn chr_bank(&self) -> usize {
        let banks = match &self.chr {
            Chr::Rom(rom) => rom.len() / CHR_8K,
            Chr::Ram(_) => 0,
        };
        let bank = match &self.board {
            Board::Cnrom { bank }
            | Board::ColorDreams { chr_bank: bank, .. }
            | Board::Gxrom { chr_bank: bank, .. }
            | Board::J87 { bank }
            | Board::J101 { bank } => usize::from(*bank),
            _ => 0,
        };
        if banks == 0 {
            0
        } else {
            bank % banks
        }
    }

    /// Mapper register state for the determinism hash (same tuple
    /// convention as the M2 bus: NROM all zero, MMC1 its registers).
    pub fn state(&self) -> (u8, u8, u8, u8) {
        self.board.state()
    }

    /// PRG bank mask for the image size (32 KiB-unit boards).
    fn prg_bank_mask(&self) -> u8 {
        let banks = (self.prg.len() / PRG_32K).clamp(1, 16);
        (banks - 1) as u8
    }

    /// 16 KiB-unit bank mask (UxROM/Camerica).
    fn prg16_bank_mask(&self) -> u8 {
        let banks = (self.prg.len() / PRG_16K).clamp(1, 16);
        (banks - 1) as u8
    }

    /// CHR bank mask for the CHR image size (8 KiB units).
    fn chr_bank_mask(&self) -> u8 {
        let banks = match &self.chr {
            Chr::Rom(rom) => rom.len() / CHR_8K,
            Chr::Ram(_) => 1,
        };
        (banks.clamp(1, 256) - 1) as u8
    }
}

/// Failures while building a cartridge for a mapper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapperError {
    /// The mapper id has no implementation yet.
    Unsupported(u16),
    /// The submapper combination is refused (e.g. NINA under 34).
    UnsupportedSubmapper(u16, u8),
    /// PRG image size is not valid for the board.
    BadPrgSize(usize),
    /// CHR image size is not valid for the board.
    BadChrSize(usize),
}

impl std::fmt::Display for MapperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MapperError::Unsupported(mapper) => write!(
                f,
                "mapper {mapper} is not implemented (T1 set: 0,2,3,7,11,34,66,71,87,101 \
                 plus the minimal MMC1 1)"
            ),
            MapperError::UnsupportedSubmapper(mapper, submapper) => write!(
                f,
                "mapper {mapper} submapper {submapper} is refused (34/1 = NINA-001/002 \
                 waits for the mapper families milestone)"
            ),
            MapperError::BadPrgSize(size) => {
                write!(f, "PRG size {size} bytes is invalid for this board")
            }
            MapperError::BadChrSize(size) => {
                write!(f, "CHR size {size} bytes is invalid for this board")
            }
        }
    }
}

impl std::error::Error for MapperError {}

/// The board registry: builds cartridges from `(mapper, submapper)`
/// and cartridge data (§6.1). Unknown ids are refused explicitly —
/// the §6.1 fallback target (MMC3) arrives in M5.
pub struct Registry;

/// The input facts of [`Registry::build`].
pub struct CartSpec {
    pub mapper: u16,
    pub submapper: u8,
    pub prg: Vec<u8>,
    pub chr: Chr,
    pub mirroring: Mirroring,
}

impl Registry {
    /// Validate the images for the board and build the cartridge.
    pub fn build(spec: CartSpec) -> Result<Cartridge, MapperError> {
        let CartSpec {
            mapper,
            submapper,
            prg,
            chr,
            mirroring,
        } = spec;

        let multiple_of = |len: usize, unit: usize| len != 0 && len.is_multiple_of(unit);
        let chr_rom_banks = match &chr {
            Chr::Rom(rom) => {
                if rom.is_empty() || rom.len() % CHR_8K != 0 {
                    return Err(MapperError::BadChrSize(rom.len()));
                }
                rom.len() / CHR_8K
            }
            // CHR RAM is one fixed 8 KiB window.
            Chr::Ram(_) => 0,
        };

        let board = match mapper {
            0 => {
                if prg.len() == PRG_16K {
                    Board::Nrom { mirror16k: true }
                } else if prg.len() == PRG_32K {
                    Board::Nrom { mirror16k: false }
                } else {
                    return Err(MapperError::BadPrgSize(prg.len()));
                }
            }
            1 => Board::Mmc1(Mmc1::new(prg.clone())?),
            2 => {
                if !multiple_of(prg.len(), PRG_16K) || prg.len() < 2 * PRG_16K {
                    return Err(MapperError::BadPrgSize(prg.len()));
                }
                Board::Uxrom { bank: 0 }
            }
            3 => {
                if prg.len() != PRG_16K && prg.len() != PRG_32K {
                    return Err(MapperError::BadPrgSize(prg.len()));
                }
                if chr_rom_banks == 0 {
                    return Err(MapperError::BadChrSize(0));
                }
                Board::Cnrom { bank: 0 }
            }
            7 => {
                if !multiple_of(prg.len(), PRG_32K) {
                    return Err(MapperError::BadPrgSize(prg.len()));
                }
                Board::Axrom {
                    bank: 0,
                    page1: false,
                }
            }
            11 => {
                if !multiple_of(prg.len(), PRG_32K) {
                    return Err(MapperError::BadPrgSize(prg.len()));
                }
                if chr_rom_banks == 0 {
                    return Err(MapperError::BadChrSize(0));
                }
                Board::ColorDreams {
                    prg_bank: 0,
                    chr_bank: 0,
                }
            }
            34 => {
                // NES 2.0 submapper 1 is NINA-001/002; submapper 2 is
                // BNROM; submapper 0 follows the documented CHR-size
                // heuristic (BNROM when CHR is 0-8 KiB).
                match submapper {
                    1 => return Err(MapperError::UnsupportedSubmapper(34, 1)),
                    2 => {}
                    0 if chr_rom_banks <= 1 => {}
                    0 => return Err(MapperError::UnsupportedSubmapper(34, 0)),
                    other => return Err(MapperError::UnsupportedSubmapper(34, other)),
                }
                if !multiple_of(prg.len(), PRG_32K) {
                    return Err(MapperError::BadPrgSize(prg.len()));
                }
                Board::Bnrom { bank: 0 }
            }
            66 => {
                if !multiple_of(prg.len(), PRG_32K) {
                    return Err(MapperError::BadPrgSize(prg.len()));
                }
                if chr_rom_banks == 0 {
                    return Err(MapperError::BadChrSize(0));
                }
                Board::Gxrom {
                    prg_bank: 0,
                    chr_bank: 0,
                }
            }
            71 => {
                if !multiple_of(prg.len(), PRG_16K) || prg.len() < 2 * PRG_16K {
                    return Err(MapperError::BadPrgSize(prg.len()));
                }
                Board::Camerica {
                    bank: 0,
                    mirror: CamericaMirror::Header,
                }
            }
            87 => {
                if prg.len() != PRG_16K && prg.len() != PRG_32K {
                    return Err(MapperError::BadPrgSize(prg.len()));
                }
                if chr_rom_banks == 0 {
                    return Err(MapperError::BadChrSize(0));
                }
                Board::J87 { bank: 0 }
            }
            101 => {
                if prg.len() != PRG_16K && prg.len() != PRG_32K {
                    return Err(MapperError::BadPrgSize(prg.len()));
                }
                if chr_rom_banks == 0 {
                    return Err(MapperError::BadChrSize(0));
                }
                Board::J101 { bank: 0 }
            }
            other => return Err(MapperError::Unsupported(other)),
        };

        Ok(Cartridge {
            prg,
            chr,
            board,
            header_mirroring: mirroring,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// PRG image with a distinct signature byte at every bank start
    /// (16 KiB and 32 KiB units) and at the unit end.
    fn signed_prg(units: usize, unit: usize) -> Vec<u8> {
        let mut prg = vec![0u8; units * unit];
        for bank in 0..units {
            prg[bank * unit] = 0xA0 | bank as u8;
            prg[bank * unit + unit - 1] = 0xB0 | bank as u8;
        }
        prg
    }

    fn chr_rom(banks: usize) -> Chr {
        let mut rom = vec![0u8; banks * CHR_8K];
        for bank in 0..banks {
            rom[bank * CHR_8K] = 0xC0 | bank as u8;
            rom[bank * CHR_8K + CHR_8K - 1] = 0xD0 | bank as u8;
        }
        Chr::Rom(rom)
    }

    fn build(mapper: u16, prg: Vec<u8>, chr: Chr) -> Cartridge {
        Registry::build(CartSpec {
            mapper,
            submapper: 0,
            prg,
            chr,
            mirroring: Mirroring::Vertical,
        })
        .expect("builds")
    }

    #[test]
    fn uxrom_switches_the_16k_window_and_keeps_the_last_bank_fixed() {
        // 4 banks: bank 3 is the fixed one.
        let mut cart = build(2, signed_prg(4, PRG_16K), Chr::Ram(Box::new([0; 0x2000])));
        assert_eq!(cart.prg_read(0x8000), 0xA0, "power-on bank 0");
        for bank in 0u8..4 {
            cart.cpu_write(0x8000, bank);
            assert_eq!(cart.prg_read(0x8000), 0xA0 | bank, "bank {bank}");
            assert_eq!(cart.prg_read(0xBFFF), 0xB0 | bank, "bank {bank} tail");
            // $C000 is the fixed last bank regardless of the register.
            assert_eq!(cart.prg_read(0xC000), 0xA3);
            assert_eq!(cart.prg_read(0xFFFF), 0xB3);
        }
        // Values beyond the image mask wrap (4 banks -> 2 bits).
        cart.cpu_write(0xFFFF, 0x15);
        assert_eq!(cart.prg_read(0x8000), 0xA0 | (0x15 & 3), "masked to 2 bits");
    }

    #[test]
    fn cnrom_switches_chr_and_keeps_prg_fixed() {
        let mut cart = build(3, signed_prg(1, PRG_32K), chr_rom(4));
        for bank in 0u8..4 {
            cart.cpu_write(0x9000, bank);
            assert_eq!(cart.chr_bank(), usize::from(bank));
        }
        // PRG ignores the register: 32 KiB linear.
        assert_eq!(cart.prg_read(0x8000), 0xA0);
        assert_eq!(cart.prg_read(0xFFFF), 0xB0);
    }

    #[test]
    fn axrom_switches_32k_and_single_screen_page() {
        let mut cart = build(7, signed_prg(4, PRG_32K), Chr::Ram(Box::new([0; 0x2000])));
        cart.cpu_write(0x8000, 0x01);
        assert_eq!(cart.prg_read(0x8000), 0xA1);
        assert_eq!(cart.prg_read(0xFFFF), 0xB1);
        assert_eq!(cart.mirroring(), Mirroring::OneScreenA);
        cart.cpu_write(0x8000, 0x12); // bank 2 + page bit
        assert_eq!(cart.prg_read(0x8000), 0xA2);
        assert_eq!(cart.mirroring(), Mirroring::OneScreenB);
    }

    #[test]
    fn color_dreams_switches_prg_and_chr_together() {
        let mut cart = build(11, signed_prg(2, PRG_32K), chr_rom(4));
        // PRG bank 1, CHR bank 2 in one write: v = 1 | 2 << 4.
        cart.cpu_write(0x8000, 0x21);
        assert_eq!(cart.prg_read(0x8000), 0xA1);
        assert_eq!(cart.chr_bank(), 2);
        cart.cpu_write(0x8000, 0x03);
        assert_eq!(cart.prg_read(0x8000), 0xA0 | (3 & 1), "PRG wraps to 1");
        assert_eq!(cart.chr_bank(), 0);
        cart.cpu_write(0x8000, 0x30);
        assert_eq!(cart.prg_read(0x8000), 0xA0);
        assert_eq!(cart.chr_bank(), 3);
    }

    #[test]
    fn bnrom_switches_the_whole_32k_window() {
        let mut cart = build(34, signed_prg(4, PRG_32K), Chr::Ram(Box::new([0; 0x2000])));
        for bank in 0u8..4 {
            cart.cpu_write(0xE000, bank);
            assert_eq!(cart.prg_read(0x8000), 0xA0 | bank);
            assert_eq!(cart.prg_read(0xFFFF), 0xB0 | bank);
        }
    }

    #[test]
    fn gxrom_switches_prg_high_bits_and_chr_low_bits() {
        let mut cart = build(66, signed_prg(2, PRG_32K), chr_rom(4));
        // PRG bank 1 (bits 6-7), CHR bank 3 (bits 0-1).
        cart.cpu_write(0x8000, (1 << 6) | 3);
        assert_eq!(cart.prg_read(0x8000), 0xA1);
        assert_eq!(cart.chr_bank(), 3);
        cart.cpu_write(0x8000, 0x01);
        assert_eq!(cart.prg_read(0x8000), 0xA0);
        assert_eq!(cart.chr_bank(), 1);
    }

    #[test]
    fn camerica_switches_only_from_c000_and_one_screen_from_9000() {
        let mut cart = build(71, signed_prg(4, PRG_16K), Chr::Ram(Box::new([0; 0x2000])));
        // Writes below $C000 select nothing (except the mirroring
        // rule at $9000-$9FFF).
        cart.cpu_write(0x8000, 0x02);
        assert_eq!(cart.prg_read(0x8000), 0xA0, "bank select ignored at $8000");
        cart.cpu_write(0xC000, 0x02);
        assert_eq!(cart.prg_read(0x8000), 0xA2);
        assert_eq!(cart.prg_read(0xC000), 0xA3, "fixed last bank");
        // Mirroring: header until a $9000-$9FFF write; bit 4 picks
        // the page; $8000-$8FFF writes stay ignored (compat rule).
        assert_eq!(cart.mirroring(), Mirroring::Vertical);
        cart.cpu_write(0x8800, 0x10);
        assert_eq!(cart.mirroring(), Mirroring::Vertical);
        cart.cpu_write(0x9000, 0x00);
        assert_eq!(cart.mirroring(), Mirroring::OneScreenA);
        cart.cpu_write(0x9FFF, 0x10);
        assert_eq!(cart.mirroring(), Mirroring::OneScreenB);
    }

    #[test]
    fn j87_selects_chr_with_reversed_bits() {
        let mut cart = build(87, signed_prg(1, PRG_32K), chr_rom(4));
        // Written bit 0 = high bank bit, bit 1 = low bank bit.
        for (value, bank) in [(0u8, 0usize), (2, 1), (1, 2), (3, 3)] {
            cart.cpu_write(0x7000, value);
            assert_eq!(cart.chr_bank(), bank, "value {value:02X}");
        }
        // PRG stays fixed.
        assert_eq!(cart.prg_read(0x8000), 0xA0);
        assert_eq!(cart.prg_read(0xFFFF), 0xB0);
    }

    #[test]
    fn mapper_101_selects_chr_with_bits_in_order() {
        let mut cart = build(101, signed_prg(1, PRG_32K), chr_rom(4));
        for bank in 0u8..4 {
            cart.cpu_write(0x6000, bank);
            assert_eq!(cart.chr_bank(), usize::from(bank));
        }
    }

    #[test]
    fn registry_refuses_what_the_docs_do_not_cover() {
        // NINA under mapper 34.
        let err = Registry::build(CartSpec {
            mapper: 34,
            submapper: 1,
            prg: signed_prg(2, PRG_32K),
            chr: chr_rom(2),
            mirroring: Mirroring::Vertical,
        })
        .unwrap_err();
        assert_eq!(err, MapperError::UnsupportedSubmapper(34, 1));
        // Unknown mapper.
        let err = Registry::build(CartSpec {
            mapper: 42,
            submapper: 0,
            prg: signed_prg(2, PRG_32K),
            chr: Chr::Ram(Box::new([0; 0x2000])),
            mirroring: Mirroring::Vertical,
        })
        .unwrap_err();
        assert_eq!(err, MapperError::Unsupported(42));
        // UxROM needs a multiple of 16 KiB (and a fixed bank).
        let err = Registry::build(CartSpec {
            mapper: 2,
            submapper: 0,
            prg: vec![0u8; 24 * 1024],
            chr: Chr::Ram(Box::new([0; 0x2000])),
            mirroring: Mirroring::Vertical,
        })
        .unwrap_err();
        assert_eq!(err, MapperError::BadPrgSize(24 * 1024));
    }

    #[test]
    fn nrom_state_tuple_stays_all_zero_and_mmc1_unchanged() {
        // The M2 determinism hash contract.
        let cart = build(0, signed_prg(1, PRG_16K), Chr::Ram(Box::new([0; 0x2000])));
        assert_eq!(cart.state(), (0, 0, 0, 0));
        let mut mmc1 = build(1, signed_prg(2, PRG_16K), Chr::Ram(Box::new([0; 0x2000])));
        assert_eq!(mmc1.state(), (0x0C, 0, 0, 0));
        // Serial protocol, LSB first: 1,0,0,0,0 latches control=$01.
        for bit in [1u8, 0, 0, 0, 0] {
            mmc1.cpu_write(0x8000, bit);
        }
        // 5th write latches control = $01 (screen mode 1).
        assert_eq!(mmc1.state().0, 0x01);
    }
}
