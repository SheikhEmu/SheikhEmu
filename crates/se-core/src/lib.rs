//! `se-core` — the SheikhEmu emulation core.
//!
//! This crate contains the timing constants and CPU building blocks of the
//! emulator. It is written against public hardware documentation only
//! (NESdev Wiki "Cycle reference chart" and "Timing" pages, NES 2.0 header
//! specification) and contains no third-party code.

pub mod cpu;
pub mod timing;

pub use cpu::{Bus, Cpu, CpuError};
pub use timing::Region;
