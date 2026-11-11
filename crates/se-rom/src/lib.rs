//! `se-rom` — cartridge image loading for SheikhEmu.
//!
//! Parses the iNES and NES 2.0 header formats (public specification,
//! verified against the NESdev Wiki copy preserved in the Internet
//! Archive, snapshot of 2025-11-15). Error and warning texts are written
//! in Arabic because they surface directly to the CLI user; identifiers
//! and documentation stay in English.

pub mod error;
pub mod ines;

pub use error::{RomError, Warning};
pub use ines::{parse, InesRom, Mirroring, TimingRegion};
