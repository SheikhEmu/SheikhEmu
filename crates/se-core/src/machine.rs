//! M1 execution shell: CPU + bus + a deterministic cycle counter.
//!
//! This is the concrete realization of the scheduler decision (spec
//! §5.0.1, DECISIONS.md D-16): a single monotonic master counter, the CPU
//! cycle as the base unit, no threads and no wall-clock dependency. In
//! M1 the machine simply counts CPU cycles and derives the PPU dot
//! position arithmetically (nothing renders yet); the event queue joins
//! when the PPU arrives in M2.
//!
//! Frame budget: NTSC frames average 29780.5 CPU cycles because of the
//! odd-frame dot skip, which only matters while rendering is enabled.
//! M1 uses a fixed 29780-cycle budget (see `docs/TIMING.md`); the exact
//! per-frame length becomes real with the PPU in M2. Determinism — the
//! actual M1 requirement — is exact.

use crate::bus::{BusError, NesBus};
use crate::cpu::{Cpu, CpuError};
use crate::timing::{Region, DOTS_PER_SCANLINE};

/// FNV-1a 64-bit offset basis (public domain algorithm).
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// M1 CPU-cycle budget of one frame (see the module docs).
pub fn frame_cycles(region: Region) -> u64 {
    match region {
        // Truncated from the 29780.5 average; the half cycle and the odd
        // frame skip only exist while rendering (M2+).
        Region::Ntsc => 29_780,
        Region::Pal => 33_247,
        Region::Dendy => 35_464,
    }
}

/// The M1 machine: CPU + NROM bus + cycle counter.
#[derive(Clone, Debug)]
pub struct Machine {
    /// The CPU.
    pub cpu: Cpu,
    /// The CPU-visible bus.
    pub bus: NesBus,
    /// Console region (timing constants).
    pub region: Region,
    /// CPU cycles elapsed since power-on (the master counter).
    pub cycle: u64,
    /// Instructions executed since power-on.
    pub instructions: u64,
}

impl Machine {
    /// Build a machine with a PRG image (NROM) for a region.
    pub fn new(prg: Vec<u8>, region: Region) -> Result<Self, BusError> {
        Ok(Machine {
            cpu: Cpu::new(),
            bus: NesBus::new(prg)?,
            region,
            cycle: 0,
            instructions: 0,
        })
    }

    /// Power-on reset: seven cycles, PC from `$FFFC`, `S -= 3`.
    pub fn reset(&mut self) {
        let cycles = u64::from(self.cpu.reset(&mut self.bus));
        self.cycle += cycles;
    }

    /// Test-ROM automation entry: power state with the PC forced
    /// (nestest style) and the counter at the post-reset position.
    ///
    /// The stack pointer and registers keep their power-up values
    /// because no reset sequence runs — this reproduces the documented
    /// automation convention of the reference CPU test.
    pub fn start_automation(&mut self, pc: u16) {
        self.cpu = Cpu::new().with_pc(pc);
        self.cycle = 7;
        self.instructions = 0;
    }

    /// Execute one instruction; returns the cycles it used.
    pub fn step(&mut self) -> Result<u8, CpuError> {
        let cycles = self.cpu.step(&mut self.bus)?;
        self.cycle += u64::from(cycles);
        self.instructions += 1;
        Ok(cycles)
    }

    /// Run whole instructions until at least `budget` cycles are spent.
    ///
    /// Instructions are atomic, so the counter may overshoot by up to
    /// six cycles; the definition is fixed and deterministic.
    pub fn run_cycles(&mut self, budget: u64) -> Result<u64, CpuError> {
        while self.cycle < budget {
            self.step()?;
        }
        Ok(self.cycle)
    }

    /// Run `frames` frames at the M1 per-region cycle budget.
    pub fn run_frames(&mut self, frames: u32) -> Result<u64, CpuError> {
        let target = self.cycle + u64::from(frames) * frame_cycles(self.region);
        self.run_cycles(target)
    }

    /// PPU position as `(scanline, dot)`, derived from the cycle count.
    ///
    /// With no PPU hardware in M1, the position is the arithmetic
    /// counter the real PPU would show with rendering disabled; it
    /// matches the reference trace convention (dot = dots_per_cycle *
    /// cycles, wrapping the 341-dot scanline and the per-region frame).
    pub fn ppu_position(&self) -> (u32, u32) {
        // Total PPU dots: CPU cycles * (cpu_divisor / ppu_divisor).
        // NTSC/Dendy are exactly 3 dots per cycle; PAL is 16/5 — scale
        // by cpu_div and divide by ppu_div to stay in integers.
        let dots = self.cycle * u64::from(self.region.cpu_divisor())
            / u64::from(self.region.ppu_divisor());
        let scanlines_per_frame = u64::from(self.region.scanlines_per_frame());
        let dots_per_frame = u64::from(DOTS_PER_SCANLINE) * scanlines_per_frame;
        let frame_dot = dots % dots_per_frame;
        let scanline = (frame_dot / u64::from(DOTS_PER_SCANLINE)) as u32;
        let dot = (frame_dot % u64::from(DOTS_PER_SCANLINE)) as u32;
        (scanline, dot)
    }

    /// Deterministic state signature (FNV-1a 64) of the machine.
    ///
    /// Hashes everything that two identical runs could distinguish:
    /// region, cycle and instruction counters, CPU registers, RAM and
    /// the open-bus latch. Pure integer arithmetic, so the same inputs
    /// give the same hash on every architecture (D-16 requirement).
    pub fn state_hash(&self) -> u64 {
        let mut hash = FNV_OFFSET_BASIS;
        let region_byte = match self.region {
            Region::Ntsc => 0u8,
            Region::Pal => 1,
            Region::Dendy => 2,
        };
        hash = mix_u8(hash, region_byte);
        hash = mix_u64(hash, self.cycle);
        hash = mix_u64(hash, self.instructions);
        hash = mix_u8(hash, self.cpu.a);
        hash = mix_u8(hash, self.cpu.x);
        hash = mix_u8(hash, self.cpu.y);
        hash = mix_u8(hash, self.cpu.sp);
        // The B flag is a phantom bit; hash the stored P as-is (it never
        // contains B in this implementation).
        hash = mix_u8(hash, self.cpu.p);
        hash = mix_u8(hash, u8::from(self.cpu.halted));
        hash = mix_u16(hash, self.cpu.pc);
        for byte in self.bus.ram() {
            hash = mix_u8(hash, *byte);
        }
        mix_u8(hash, self.bus.latch())
    }
}

/// FNV-1a mixing helpers (wrapping arithmetic, portable).
fn mix_u8(hash: u64, byte: u8) -> u64 {
    (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME)
}

fn mix_u16(hash: u64, value: u16) -> u64 {
    mix_u8(mix_u8(hash, (value & 0xFF) as u8), (value >> 8) as u8)
}

fn mix_u64(hash: u64, value: u64) -> u64 {
    let bytes = value.to_le_bytes();
    let mut h = hash;
    for byte in bytes {
        h = mix_u8(h, byte);
    }
    h
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::Bus;

    /// 16 KiB PRG: reset vector -> $8000, an infinite NOP slide.
    fn nop_prg() -> Vec<u8> {
        let mut prg = vec![0xEA; 16 * 1024];
        // Vectors at the end: NMI $8000, reset $8000, IRQ $8000.
        prg[0x3FFA..0x4000].copy_from_slice(&[0x00, 0x80, 0x00, 0x80, 0x00, 0x80]);
        prg
    }

    #[test]
    fn reset_reads_the_vector_and_counts_seven_cycles() {
        let mut machine = Machine::new(nop_prg(), Region::Ntsc).expect("prg");
        machine.reset();
        assert_eq!(machine.cycle, 7);
        assert_eq!(machine.cpu.pc, 0x8000);
        assert_eq!(machine.cpu.sp, 0xFA);
    }

    #[test]
    fn automation_entry_keeps_power_state() {
        let mut machine = Machine::new(nop_prg(), Region::Ntsc).expect("prg");
        machine.start_automation(0xC000);
        assert_eq!(machine.cpu.pc, 0xC000);
        assert_eq!(machine.cpu.sp, 0xFD);
        assert_eq!(machine.cycle, 7);
    }

    #[test]
    fn run_cycles_counts_whole_instructions() {
        let mut machine = Machine::new(nop_prg(), Region::Ntsc).expect("prg");
        machine.reset();
        // 10 NOPs = 20 cycles from cycle 7.
        machine.run_cycles(27).expect("nop slide");
        assert_eq!(machine.cycle, 27);
        assert_eq!(machine.instructions, 10);
        // The next instruction would cross the budget: nothing more runs.
        machine.run_cycles(28).expect("nop slide");
        assert_eq!(machine.cycle, 29);
        assert_eq!(machine.instructions, 11);
    }

    #[test]
    fn frame_budgets_follow_the_region() {
        assert_eq!(frame_cycles(Region::Ntsc), 29_780);
        assert_eq!(frame_cycles(Region::Pal), 33_247);
        assert_eq!(frame_cycles(Region::Dendy), 35_464);
    }

    #[test]
    fn run_frames_is_cycle_budget_times_frames() {
        let mut machine = Machine::new(nop_prg(), Region::Ntsc).expect("prg");
        machine.reset();
        machine.run_frames(3).expect("nop slide");
        // Whole instructions only: 7 + 29780*3 with <=6 cycles overshoot.
        assert!(machine.cycle >= 7 + 29_780 * 3);
        assert!(machine.cycle < 7 + 29_780 * 3 + 7);
    }

    #[test]
    fn ppu_position_matches_the_three_dots_per_cycle_rule() {
        let mut machine = Machine::new(nop_prg(), Region::Ntsc).expect("prg");
        machine.reset();
        // The nestest convention: at CPU cycle 7 the PPU sits at (0, 21).
        assert_eq!(machine.ppu_position(), (0, 21));
        machine.run_cycles(100).expect("nop slide");
        let dots = machine.cycle * 3;
        let scanline = (dots / 341 % 262) as u32;
        let dot = (dots % 341) as u32;
        assert_eq!(machine.ppu_position(), (scanline, dot));
    }

    #[test]
    fn same_run_same_hash() {
        let run = || {
            let mut machine = Machine::new(nop_prg(), Region::Ntsc).expect("prg");
            machine.reset();
            machine.run_frames(2).expect("nop slide");
            machine.state_hash()
        };
        assert_eq!(run(), run(), "identical runs must hash identically");
    }

    #[test]
    fn different_ram_gives_a_different_hash() {
        let mut a = Machine::new(nop_prg(), Region::Ntsc).expect("prg");
        let mut b = Machine::new(nop_prg(), Region::Ntsc).expect("prg");
        a.reset();
        b.reset();
        a.bus.write(0x0050, 0x11);
        b.bus.write(0x0050, 0x22);
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn halted_machine_reports_the_error() {
        let mut prg = vec![0xEA; 16 * 1024];
        prg[0x3FFA..0x4000].copy_from_slice(&[0x00, 0x80, 0x00, 0x80, 0x00, 0x80]);
        // Reset into the JAM opcode at $8000.
        prg[0] = 0x02;
        let mut machine = Machine::new(prg, Region::Ntsc).expect("prg");
        machine.reset();
        assert_eq!(machine.step(), Ok(2));
        assert!(machine.cpu.halted);
        assert_eq!(machine.step(), Err(CpuError::Halted));
    }
}
