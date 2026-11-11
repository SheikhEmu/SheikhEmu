//! `se-core` — the SheikhEmu emulation core.
//!
//! This crate contains the timing constants, the memory bus, the M1
//! execution shell and CPU building blocks of the emulator. It is
//! written against public hardware documentation only (NESdev Wiki
//! "Cycle reference chart", "Timing", "CPU memory map" and "CPU
//! unimplemented instructions" pages, NES 2.0 header specification)
//! and contains no third-party code.

pub mod bus;
pub mod cpu;
pub mod machine;
pub mod timing;
pub mod trace;

pub use bus::{BusError, NesBus, NromPrg};
pub use cpu::{Bus, Cpu, CpuError};
pub use machine::Machine;
pub use timing::Region;
