//! `se-core` — the SheikhEmu emulation core.
//!
//! This crate contains the timing constants, the memory bus, the CPU,
//! the PPU (M2), the APU (M4.2) and the execution shell of the
//! emulator. It is written against public hardware documentation only
//! (NESdev Wiki "Cycle reference chart", "Timing", "CPU memory map",
//! "CPU unimplemented instructions", "PPU rendering", "PPU registers",
//! "PPU scrolling", "PPU frame timing" pages, the APU pages, NES 2.0
//! header specification) and contains no third-party code.

pub mod apu;
pub mod bus;
pub mod cpu;
pub mod input;
pub mod machine;
pub mod mapper;
pub mod ppu;
pub mod timing;
pub mod trace;

pub use apu::Apu;
pub use bus::{BusError, NesBus};
pub use cpu::{Bus, Cpu, CpuError};
pub use input::{
    Controllers, BTN_A, BTN_B, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SELECT, BTN_START, BTN_UP,
};
pub use machine::Machine;
pub use mapper::{CartSpec, Cartridge, MapperError, Registry};
pub use ppu::{Chr, Mirroring, Ppu, PALETTE_2C02};
pub use timing::Region;
