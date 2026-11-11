//! iNES / NES 2.0 header parsing.
//!
//! The byte layout below is verified against the NES 2.0 specification as
//! archived by the Internet Archive (NESdev Wiki snapshot of 2025-11-15):
//!
//! - Identification: `NES<EOF>`; NES 2.0 additionally requires byte 7
//!   bits 2-3 to equal `10` (`flags7 & 0x0C == 0x08`).
//! - Byte 8: low nibble = submapper, high nibble = mapper bits 8-11
//!   (mapper is 12 bits in total).
//! - Byte 9: low nibble = PRG-ROM size MSB, high nibble = CHR-ROM size
//!   MSB. A nibble of `$F` switches that area to the exponent-multiplier
//!   notation `1111 EEEE EEMM`: size = `2^E * (MM*2+1)` bytes.
//! - Byte 10: low nibble = PRG-RAM (volatile) shift count, high nibble =
//!   PRG-NVRAM/EEPROM shift count; `0` means absent, otherwise
//!   `64 << count` bytes.
//! - Byte 11: same notation for CHR-RAM / CHR-NVRAM.
//! - Byte 12 bits 0-1: CPU/PPU timing (0 RP2C02 NTSC, 1 RP2C07 PAL,
//!   2 multiple-region, 3 UA6538 Dendy).
//! - Byte 14 bits 0-1: number of miscellaneous ROMs.
//!
//! For old iNES headers with garbage in bytes 12-15 (the well-known
//! "DiskDude!" pollution) only the low mapper nibble of byte 6 is
//! trusted and the header is flagged with a warning.

use se_core::Region;

use crate::error::{RomError, Warning};

/// Maximum PRG-ROM size expressible by the specification: `$EFF` units of
/// 16 KiB = 62,898,176 bytes.
pub const MAX_PRG_BYTES: u64 = 62_898_176;

/// Maximum CHR-ROM size expressible by the specification: `$EFF` units of
/// 8 KiB = 31,449,088 bytes.
pub const MAX_CHR_BYTES: u64 = 31_449_088;

const TRAINER_BYTES: usize = 512;
const HEADER_BYTES: usize = 16;

/// Hard-wired nametable arrangement from the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mirroring {
    /// Bit 0 clear: vertical arrangement (mirrored horizontally).
    Vertical,
    /// Bit 0 set: horizontal arrangement (mirrored vertically).
    Horizontal,
    /// Bit 3 set: four-screen VRAM.
    FourScreen,
}

/// CPU/PPU timing declared by a NES 2.0 header (byte 12 bits 0-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimingRegion {
    /// RP2C02 timing (value 0).
    Ntsc,
    /// RP2C07 timing (value 1).
    Pal,
    /// Multiple-region ROM (value 2): the emulator keeps its chosen region.
    Multiple,
    /// UA6538 Dendy timing (value 3).
    Dendy,
}

impl TimingRegion {
    /// Map to the emulation-core region; multiple-region ROMs carry no
    /// fixed region and the user's choice applies.
    pub const fn to_region(self) -> Option<Region> {
        match self {
            TimingRegion::Ntsc => Some(Region::Ntsc),
            TimingRegion::Pal => Some(Region::Pal),
            TimingRegion::Dendy => Some(Region::Dendy),
            TimingRegion::Multiple => None,
        }
    }
}

/// A fully parsed cartridge image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InesRom {
    /// Whether the header identifies itself as NES 2.0.
    pub nes2: bool,
    /// Mapper number, 12 bits (only the low 4 bits are trusted for
    /// garbage-era iNES headers).
    pub mapper: u16,
    /// NES 2.0 submapper (0 for iNES headers).
    pub submapper: u8,
    /// Nametable arrangement.
    pub mirroring: Mirroring,
    /// Whether the header declares battery-backed (non-volatile) memory.
    pub battery: bool,
    /// The 512-byte trainer area, if present.
    pub trainer: Option<Vec<u8>>,
    /// PRG-ROM data.
    pub prg: Vec<u8>,
    /// CHR-ROM data (empty when the cartridge uses CHR-RAM).
    pub chr: Vec<u8>,
    /// Declared CPU/PPU timing; `None` for plain iNES headers.
    pub timing: Option<TimingRegion>,
    /// PRG-RAM (volatile) size in bytes, if declared by NES 2.0 byte 10.
    pub prg_ram: Option<u64>,
    /// PRG-NVRAM/EEPROM size in bytes, if declared by NES 2.0 byte 10.
    pub prg_nvram: Option<u64>,
    /// CHR-RAM (volatile) size in bytes, if declared by NES 2.0 byte 11.
    pub chr_ram: Option<u64>,
    /// CHR-NVRAM size in bytes, if declared by NES 2.0 byte 11.
    pub chr_nvram: Option<u64>,
    /// Console type from byte 7 bits 0-1 (0 regular, 1 Vs, 2 Playchoice,
    /// 3 extended).
    pub console_type: u8,
    /// Miscellaneous ROM area bytes (after CHR) with the declared chip
    /// count from byte 14.
    pub misc_rom: Vec<u8>,
    /// Declared number of miscellaneous ROM chips.
    pub misc_rom_count: u8,
    /// Non-fatal header observations.
    pub warnings: Vec<Warning>,
}

/// Interpret a NES 2.0 shift-count nibble: `0` means absent, `n` means
/// `64 << n` bytes.
const fn shift_count_to_bytes(count: u8) -> Option<u64> {
    if count == 0 {
        None
    } else {
        Some(64u64 << count)
    }
}

/// Compute a NES 2.0 area size from the exponent-multiplier nibble pair.
/// Returns `None` when the nibble is not `$F` (linear notation applies).
fn exponent_multiplier_size(lsb: u8, nibble: u8) -> Option<u64> {
    if nibble != 0x0F {
        return None;
    }
    let exponent = u32::from(lsb >> 2);
    let multiplier = u32::from(lsb & 0x03) * 2 + 1;
    Some((1u64 << exponent).saturating_mul(u64::from(multiplier)))
}

/// Compute a NES 2.0 area size from the linear (unit-count) notation.
fn linear_units_size(lsb: u8, nibble: u8, unit_bytes: u64) -> u64 {
    let units = u64::from(lsb) | u64::from(nibble) << 8;
    units.saturating_mul(unit_bytes)
}

/// Build a test-friendly iNES header.
///
/// `f6`/`f7` are the raw flag bytes; bytes 12-15 are zero (a clean old
/// header). This helper exists for tests and is also documented in the
/// project notes for ROM tooling.
#[cfg(test)]
pub(crate) fn hdr(f6: u8, f7: u8, prg_units: u8, chr_units: u8) -> Vec<u8> {
    vec![
        b'N', b'E', b'S', 0x1A, prg_units, chr_units, f6, f7, 0, 0, 0, 0, 0, 0, 0, 0,
    ]
}

/// Parse a complete iNES / NES 2.0 image.
pub fn parse(data: &[u8]) -> Result<InesRom, RomError> {
    if data.len() < HEADER_BYTES {
        return Err(RomError::TooShort);
    }
    if &data[0..4] != b"NES\x1A" {
        return Err(RomError::BadMagic);
    }

    let flags6 = data[6];
    let flags7 = data[7];
    let nes2 = flags7 & 0x0C == 0x08;

    let mut warnings = Vec::new();

    // Old headers polluted by DiskDude-era tools: bytes 12-15 non-zero
    // means the upper mapper nibble and NES 2.0 bits are garbage.
    let garbage = !nes2 && data[12..HEADER_BYTES] != [0; 4];

    let mapper: u16 = if garbage {
        warnings.push(Warning::DiskDudeGarbage);
        u16::from(flags6 >> 4)
    } else {
        let mut m = u16::from(flags6 >> 4) | u16::from(flags7 >> 4) << 4;
        if nes2 {
            m |= u16::from(data[8] >> 4) << 8;
        }
        m
    };
    let submapper = if nes2 { data[8] & 0x0F } else { 0 };

    // Console type: byte 7 bits 0-1, forced to regular NES for garbage.
    let console_type: u8 = if garbage { 0 } else { flags7 & 0x03 };
    if console_type != 0 {
        warnings.push(Warning::NonStandardConsole);
    }

    // PRG size.
    let prg_bytes: u64 = if nes2 {
        let nibble = data[9] & 0x0F;
        exponent_multiplier_size(data[4], nibble)
            .unwrap_or_else(|| linear_units_size(data[4], nibble, 16 * 1024))
    } else {
        u64::from(data[4]) * (16 * 1024)
    };

    // CHR size.
    let chr_bytes: u64 = if nes2 {
        let nibble = data[9] >> 4;
        exponent_multiplier_size(data[5], nibble)
            .unwrap_or_else(|| linear_units_size(data[5], nibble, 8 * 1024))
    } else {
        u64::from(data[5]) * (8 * 1024)
    };

    if prg_bytes > MAX_PRG_BYTES {
        return Err(RomError::Oversized {
            area: "PRG",
            announced: prg_bytes,
            limit: MAX_PRG_BYTES,
        });
    }
    if chr_bytes > MAX_CHR_BYTES {
        return Err(RomError::Oversized {
            area: "CHR",
            announced: chr_bytes,
            limit: MAX_CHR_BYTES,
        });
    }

    let battery = flags6 & 0x02 != 0;
    let has_trainer = flags6 & 0x04 != 0;
    let mirroring = if flags6 & 0x08 != 0 {
        Mirroring::FourScreen
    } else if flags6 & 0x01 != 0 {
        Mirroring::Horizontal
    } else {
        Mirroring::Vertical
    };

    // Slice the data areas out of the file.
    let mut offset = HEADER_BYTES;
    let trainer: Option<Vec<u8>> = if has_trainer {
        let end = offset + TRAINER_BYTES;
        if data.len() < end {
            return Err(RomError::Truncated);
        }
        let t = data[offset..end].to_vec();
        offset = end;
        Some(t)
    } else {
        None
    };

    let prg_end = offset as u64 + prg_bytes;
    if (data.len() as u64) < prg_end {
        return Err(RomError::Truncated);
    }
    let prg = data[offset..prg_end as usize].to_vec();
    let offset = prg_end as usize;

    let chr_end = offset as u64 + chr_bytes;
    if (data.len() as u64) < chr_end {
        return Err(RomError::Truncated);
    }
    let chr = data[offset..chr_end as usize].to_vec();
    let offset = chr_end as usize;

    let misc_rom = data[offset..].to_vec();
    let misc_rom_count = if nes2 { data[14] & 0x03 } else { 0 };
    if !misc_rom.is_empty() && misc_rom_count == 0 {
        warnings.push(Warning::MiscRomUndeclared {
            bytes: misc_rom.len(),
        });
    }

    // NES 2.0-only fields.
    let (prg_ram, prg_nvram, chr_ram, chr_nvram): (
        Option<u64>,
        Option<u64>,
        Option<u64>,
        Option<u64>,
    ) = if nes2 {
        let prg_ram = shift_count_to_bytes(data[10] & 0x0F);
        let prg_nvram = shift_count_to_bytes(data[10] >> 4);
        let chr_ram = shift_count_to_bytes(data[11] & 0x0F);
        let chr_nvram = shift_count_to_bytes(data[11] >> 4);

        // The battery bit and the PRG-NVRAM nibble must agree (NES 2.0
        // specification, PRG-(NV)RAM/EEPROM section).
        let nvram_declared = prg_nvram.is_some();
        if battery != nvram_declared {
            warnings.push(Warning::BatteryMismatch {
                battery_without_nvram: battery,
            });
        }
        (prg_ram, prg_nvram, chr_ram, chr_nvram)
    } else {
        (None, None, None, None)
    };

    let timing: Option<TimingRegion> = if nes2 {
        Some(match data[12] & 0x03 {
            0 => TimingRegion::Ntsc,
            1 => TimingRegion::Pal,
            2 => TimingRegion::Multiple,
            _ => TimingRegion::Dendy,
        })
    } else {
        None
    };

    // Old iNES headers with no CHR-ROM: the community convention is 8 KiB
    // of CHR-RAM; NES 2.0 requires an explicit declaration instead.
    if !nes2 && chr.is_empty() {
        warnings.push(Warning::ChrRamAssumed);
    }

    Ok(InesRom {
        nes2,
        mapper,
        submapper,
        mirroring,
        battery,
        trainer,
        prg,
        chr,
        timing,
        prg_ram,
        prg_nvram,
        chr_ram,
        chr_nvram,
        console_type,
        misc_rom,
        misc_rom_count,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal NROM image: 16 KiB PRG + 8 KiB CHR, mapper 0.
    fn nrom(prg_len: usize, chr_len: usize) -> Vec<u8> {
        let mut v = hdr(0x00, 0x00, 1, 1);
        v.extend(core::iter::repeat_n(0x00u8, prg_len));
        v.extend(core::iter::repeat_n(0xFFu8, chr_len));
        v
    }

    #[test]
    fn minimal_valid_nrom() {
        let rom = parse(&nrom(16 * 1024, 8 * 1024)).expect("valid NROM");
        assert_eq!(rom.mapper, 0);
        assert!(!rom.nes2);
        assert_eq!(rom.submapper, 0);
        assert_eq!(rom.prg.len(), 16 * 1024);
        assert_eq!(rom.chr.len(), 8 * 1024);
        assert_eq!(rom.mirroring, Mirroring::Vertical);
        assert!(!rom.battery);
        assert!(rom.trainer.is_none());
        assert!(rom.timing.is_none());
        assert!(rom.prg_ram.is_none() && rom.prg_nvram.is_none());
        assert!(rom.misc_rom.is_empty());
        assert!(rom.warnings.is_empty());
    }

    #[test]
    fn short_file_is_rejected() {
        assert_eq!(parse(b"NES"), Err(RomError::TooShort));
        // 15 bytes: still too short even with a magic prefix.
        let mut v = vec![b'N', b'E', b'S', 0x1A];
        v.resize(15, 0);
        assert_eq!(parse(&v), Err(RomError::TooShort));
    }

    #[test]
    fn bad_magic_is_rejected() {
        let mut v = nrom(16 * 1024, 8 * 1024);
        v[0] = b'X';
        assert_eq!(parse(&v), Err(RomError::BadMagic));
    }

    #[test]
    fn trainer_is_extracted() {
        let mut v = hdr(0x04, 0x00, 1, 1); // trainer bit set
        v.extend(core::iter::repeat_n(0xA5u8, 512));
        v.extend(core::iter::repeat_n(0u8, 16 * 1024));
        v.extend(core::iter::repeat_n(0xFFu8, 8 * 1024));
        let rom = parse(&v).expect("valid with trainer");
        let trainer = rom.trainer.expect("trainer present");
        assert_eq!(trainer.len(), 512);
        assert!(trainer.iter().all(|&b| b == 0xA5));
        assert_eq!(rom.prg.len(), 16 * 1024);
        assert_eq!(rom.chr.len(), 8 * 1024);
    }

    #[test]
    fn battery_and_mirroring_bits() {
        let mut v = nrom(16 * 1024, 8 * 1024);
        v[6] = 0x03; // horizontal mirroring + battery
        let rom = parse(&v).expect("valid");
        assert_eq!(rom.mirroring, Mirroring::Horizontal);
        assert!(rom.battery);

        v[6] = 0x08; // four-screen
        let rom = parse(&v).expect("valid");
        assert_eq!(rom.mirroring, Mirroring::FourScreen);
    }

    #[test]
    fn diskdude_garbage_trusts_only_low_mapper_nibble() {
        // flags6 = 0x50 (mapper low nibble 5), flags7 = 0xF0 and garbage
        // in bytes 12-15: the real mapper must be 5, not 0xF5.
        let mut v = hdr(0x50, 0xF0, 1, 1);
        v[12] = 0x44; // "D"
        v[13] = 0x69; // "i"
        v[14] = 0x73; // "s"
        v[15] = 0x6B; // "k"
        v.extend(core::iter::repeat_n(0u8, 16 * 1024));
        v.extend(core::iter::repeat_n(0xFFu8, 8 * 1024));
        let rom = parse(&v).expect("parses despite garbage");
        assert_eq!(rom.mapper, 5);
        assert!(rom.warnings.contains(&Warning::DiskDudeGarbage));
        assert_eq!(rom.console_type, 0);
    }

    #[test]
    fn clean_high_mapper_bits_are_honored() {
        // No garbage, flags7 high nibble = 2 -> mapper 0x25.
        let mut v = hdr(0x50, 0x20, 1, 1);
        v.extend(core::iter::repeat_n(0u8, 16 * 1024));
        v.extend(core::iter::repeat_n(0xFFu8, 8 * 1024));
        let rom = parse(&v).expect("valid");
        assert_eq!(rom.mapper, 0x25);
        assert!(!rom.warnings.contains(&Warning::DiskDudeGarbage));
    }

    #[test]
    fn nes2_mapper_is_twelve_bits_with_submapper() {
        // mapper 0xABC, submapper 3: the low mapper nibble C sits in the
        // high nibble of flags6.
        let f6: u8 = 0xC0; // mapper low nibble C, vertical mirroring
        let f7: u8 = 0xB8; // high nibble B + NES 2.0 marker (10)
        let b8: u8 = 0xA3; // mapper bits 8-11 = A, submapper 3
        let mut v = vec![
            b'N', b'E', b'S', 0x1A, 1, 1, f6, f7, b8, 0, 0, 0, 0, 0, 0, 0,
        ];
        v.extend(core::iter::repeat_n(0u8, 16 * 1024));
        v.extend(core::iter::repeat_n(0xFFu8, 8 * 1024));
        let rom = parse(&v).expect("valid NES 2.0");
        assert!(rom.nes2);
        assert_eq!(rom.mapper, 0x0ABC);
        assert_eq!(rom.submapper, 3);
    }

    #[test]
    fn nes2_timing_byte_selects_region() {
        for (timing_byte, expected) in [
            (0u8, Some(TimingRegion::Ntsc)),
            (1, Some(TimingRegion::Pal)),
            (2, Some(TimingRegion::Multiple)),
            (3, Some(TimingRegion::Dendy)),
        ] {
            let mut v = vec![
                b'N', b'E', b'S', 0x1A, 1, 1, 0x00, 0x08, 0, 0, 0, 0, 0, 0, 0, 0,
            ];
            v[12] = timing_byte;
            v.extend(core::iter::repeat_n(0u8, 16 * 1024));
            v.extend(core::iter::repeat_n(0xFFu8, 8 * 1024));
            let rom = parse(&v).expect("valid NES 2.0");
            assert_eq!(rom.timing, expected, "byte 12 = {timing_byte}");
        }

        // Region mapping into the core.
        assert_eq!(TimingRegion::Ntsc.to_region(), Some(Region::Ntsc));
        assert_eq!(TimingRegion::Dendy.to_region(), Some(Region::Dendy));
        assert_eq!(TimingRegion::Multiple.to_region(), None);
    }

    #[test]
    fn nes2_ram_shift_counts() {
        // byte 10 = 0x70: PRG-NVRAM shift 7 (8 KiB), PRG-RAM absent.
        // byte 11 = 0x21: CHR-NVRAM shift 2 (256 B), CHR-RAM shift 1 (128 B).
        let mut v = vec![
            b'N', b'E', b'S', 0x1A, 1, 1, 0x00, 0x08, 0, 0, 0x70, 0x21, 0, 0, 0, 0,
        ];
        v.extend(core::iter::repeat_n(0u8, 16 * 1024));
        v.extend(core::iter::repeat_n(0xFFu8, 8 * 1024));
        let rom = parse(&v).expect("valid NES 2.0");
        assert_eq!(rom.prg_ram, None);
        assert_eq!(rom.prg_nvram, Some(64 << 7));
        assert_eq!(rom.chr_ram, Some(64 << 1));
        assert_eq!(rom.chr_nvram, Some(64 << 2));
        // Battery bit is clear while NVRAM is declared -> mismatch warning.
        assert!(rom.warnings.contains(&Warning::BatteryMismatch {
            battery_without_nvram: false
        }));
    }

    #[test]
    fn nes2_battery_matches_nvram_no_warning() {
        let mut v = vec![
            b'N', b'E', b'S', 0x1A, 1, 1, 0x02, 0x08, 0, 0, 0x70, 0, 0, 0, 0, 0,
        ];
        v.extend(core::iter::repeat_n(0u8, 16 * 1024));
        v.extend(core::iter::repeat_n(0xFFu8, 8 * 1024));
        let rom = parse(&v).expect("valid NES 2.0");
        assert!(rom.battery);
        assert!(!rom.warnings.contains(&Warning::BatteryMismatch {
            battery_without_nvram: true
        }));
        assert!(!rom.warnings.contains(&Warning::BatteryMismatch {
            battery_without_nvram: false
        }));
    }

    #[test]
    fn nes2_exponent_multiplier_prg_size() {
        // PRG nibble $F, byte 4 = EEEE EEMM with E = 12, MM = 01:
        // size = 2^12 * (1*2+1) = 4096 * 3 = 12288 bytes.
        let byte4 = 12u8 << 2 | 0x01;
        let mut v = vec![
            b'N', b'E', b'S', 0x1A, byte4, 1, 0x00, 0x08, 0, 0x0F, 0, 0, 0, 0, 0, 0,
        ];
        v.extend(core::iter::repeat_n(0u8, 12_288));
        v.extend(core::iter::repeat_n(0xFFu8, 8 * 1024));
        let rom = parse(&v).expect("valid NES 2.0");
        assert_eq!(rom.prg.len(), 12_288);
    }

    #[test]
    fn nes2_oversized_exponent_is_rejected() {
        // E = 24, MM = 3: 2^24 * 7 = 117,440,512 bytes > 62,898,176.
        let byte4 = 24u8 << 2 | 0x03;
        let v = vec![
            b'N', b'E', b'S', 0x1A, byte4, 1, 0x00, 0x08, 0, 0x0F, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(
            parse(&v),
            Err(RomError::Oversized {
                area: "PRG",
                announced: 117_440_512,
                limit: MAX_PRG_BYTES
            })
        );

        // CHR nibble $F with E = 23, MM = 3: 2^23 * 7 = 58,720,256 bytes
        // exceeds 31,449,088.
        let byte5 = 23u8 << 2 | 0x03;
        let mut v = vec![
            b'N', b'E', b'S', 0x1A, 1, byte5, 0x00, 0x08, 0, 0xF0, 0, 0, 0, 0, 0, 0,
        ];
        v.extend(core::iter::repeat_n(0u8, 16 * 1024));
        assert_eq!(
            parse(&v),
            Err(RomError::Oversized {
                area: "CHR",
                announced: 58_720_256,
                limit: MAX_CHR_BYTES
            })
        );
    }

    #[test]
    fn truncated_data_is_rejected() {
        let mut v = nrom(16 * 1024, 8 * 1024);
        v.truncate(HEADER_BYTES + 100);
        assert_eq!(parse(&v), Err(RomError::Truncated));
    }

    #[test]
    fn ines_without_chr_warns_about_chrram_assumption() {
        let mut v = hdr(0x00, 0x00, 1, 0);
        v.extend(core::iter::repeat_n(0u8, 16 * 1024));
        let rom = parse(&v).expect("valid");
        assert!(rom.chr.is_empty());
        assert!(rom.warnings.contains(&Warning::ChrRamAssumed));
    }

    #[test]
    fn trailing_bytes_are_misc_rom() {
        // Declared misc count = 1: trailing bytes accepted silently.
        let mut v = vec![
            b'N', b'E', b'S', 0x1A, 1, 1, 0x00, 0x08, 0, 0, 0, 0, 0, 0, 1, 0,
        ];
        v.extend(core::iter::repeat_n(0u8, 16 * 1024));
        v.extend(core::iter::repeat_n(0xFFu8, 8 * 1024));
        v.extend(core::iter::repeat_n(0x5Au8, 256));
        let rom = parse(&v).expect("valid NES 2.0");
        assert_eq!(rom.misc_rom_count, 1);
        assert_eq!(rom.misc_rom.len(), 256);
        assert!(!rom
            .warnings
            .contains(&Warning::MiscRomUndeclared { bytes: 256 }));

        // Undeclared: warning.
        v[14] = 0;
        let rom = parse(&v).expect("valid NES 2.0");
        assert!(rom
            .warnings
            .contains(&Warning::MiscRomUndeclared { bytes: 256 }));
    }

    #[test]
    fn non_standard_console_type_warns() {
        let mut v = hdr(0x00, 0x01, 1, 1); // console type 1 = Vs.
        v.extend(core::iter::repeat_n(0u8, 16 * 1024));
        v.extend(core::iter::repeat_n(0xFFu8, 8 * 1024));
        let rom = parse(&v).expect("parses");
        assert_eq!(rom.console_type, 1);
        assert!(rom.warnings.contains(&Warning::NonStandardConsole));
    }

    #[test]
    fn random_data_with_sane_header_never_panics() {
        // Robustness: arbitrary payload bytes with a plausible header must
        // parse cleanly or produce a spec-defined error, never a panic.
        let mut rng_state = 0x1234_5678u64;
        let mut next = || {
            // xorshift64*
            rng_state ^= rng_state >> 12;
            rng_state ^= rng_state << 25;
            rng_state ^= rng_state >> 27;
            (rng_state.wrapping_mul(0x2545F4914F6CDD1D) >> 33) as u8
        };
        let mut v = vec![0u8; 64 * 1024];
        for byte in v.iter_mut() {
            *byte = next();
        }
        // Sane header: NROM sizes (48 KiB PRG fits inside the buffer),
        // no flags.
        v[0] = b'N';
        v[1] = b'E';
        v[2] = b'S';
        v[3] = 0x1A;
        v[4] = 3; // 48 KiB PRG
        v[5] = 0; // no CHR (CHR-RAM assumed, warning only)
        v[6] = 0x00;
        v[7] = 0x00;
        v[12..16].copy_from_slice(&[0, 0, 0, 0]);
        let result = parse(&v);
        assert!(result.is_ok(), "random payload must parse: {result:?}");
        let rom = result.expect("checked above");
        assert_eq!(rom.prg.len(), 48 * 1024);
        assert!(rom.chr.is_empty());
    }

    #[test]
    fn error_messages_are_in_arabic() {
        let msg = RomError::TooShort.to_string();
        assert!(msg.contains('ا') || msg.contains('أ'));
        let msg = RomError::BadMagic.to_string();
        assert!(msg.contains('ت'));
        let warn = Warning::DiskDudeGarbage.to_string();
        assert!(warn.contains('ت'));
    }
}
