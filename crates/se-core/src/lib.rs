//! `se-core` — the SheikhEmu emulation core.
//!
//! This crate contains the timing constants, the memory bus, the CPU,
//! the PPU (M2) and the execution shell of the emulator. It is
//! written against public hardware documentation only (NESdev Wiki
//! "Cycle reference chart", "Timing", "CPU memory map", "CPU
//! unimplemented instructions", "PPU rendering", "PPU registers",
//! "PPU scrolling", "PPU frame timing" pages, NES 2.0 header
//! specification) and contains no third-party code.

pub mod bus;
pub mod cpu;
pub mod machine;
pub mod ppu;
pub mod timing;
pub mod trace;

pub use bus::{BusError, NesBus, NromPrg};
pub use cpu::{Bus, Cpu, CpuError};
pub use machine::Machine;
pub use ppu::{Chr, Mirroring, Ppu, PALETTE_2C02};
pub use timing::Region;
