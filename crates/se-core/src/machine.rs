//! M2 execution shell: CPU + bus + the real PPU.
//!
//! This is the concrete realization of the scheduler decision (spec
//! §5.0.1, DECISIONS.md D-16): a single monotonic master counter, the
//! CPU cycle as the base unit, no threads and no wall-clock
//! dependency. The bus owns the counter and ticks the PPU by the
//! region's dots-per-cycle ratio at every CPU access (§5.0.1 order:
//! CPU read/write first, then the PPU dots of that cycle); the
//! machine true-ups the internal instruction cycles at instruction
//! end, so the PPU is exactly synchronized at every instruction
//! boundary.
//!
//! Frames are now real: `run_frames` counts PPU frame completions
//! (the pre-render-to-line-0 wrap), not a fixed cycle budget. The
//! instruction that crosses the boundary may overshoot by up to six
//! cycles into the new frame — a deterministic, documented definition.
//! NTSC odd frames are one dot shorter while rendering is enabled,
//! so rendered frames average 29,780.5 CPU cycles exactly as the
//! documented timing table promises.

use crate::apu::Apu;
use crate::bus::{BusError, NesBus};
use crate::cpu::{Cpu, CpuError};
use crate::ppu::{Chr, Mirroring, Ppu};
use crate::timing::Region;

/// FNV-1a 64-bit offset basis (public domain algorithm).
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The NMI vector.
const NMI_VECTOR: u16 = 0xFFFA;

/// The IRQ vector (the APU interrupt lines, M4.2).
const IRQ_VECTOR: u16 = 0xFFFE;

/// The status-register I flag mask.
const I_FLAG: u8 = 0x04;

/// The M2 machine: CPU + NROM bus with the real PPU.
#[derive(Clone, Debug)]
pub struct Machine {
    /// The CPU.
    pub cpu: Cpu,
    /// The CPU-visible bus (owns the PPU and the master clock).
    pub bus: NesBus,
    /// Console region (timing constants).
    pub region: Region,
    /// Instructions executed since power-on.
    pub instructions: u64,
}

impl Machine {
    /// Build a machine with a PRG image (NROM, CHR-RAM, vertical
    /// mirroring) for a region.
    pub fn new(prg: Vec<u8>, region: Region) -> Result<Self, BusError> {
        let bus = NesBus::new(prg, region)?;
        Ok(Machine {
            cpu: Cpu::new(),
            bus,
            region,
            instructions: 0,
        })
    }

    /// Build a machine from cartridge data (mapper 0 / NROM).
    pub fn from_cartridge(
        prg: Vec<u8>,
        chr: Chr,
        mirroring: Mirroring,
        prg_ram: bool,
        region: Region,
    ) -> Result<Self, BusError> {
        Self::from_rom(prg, 0, 0, chr, mirroring, prg_ram, region)
    }

    /// Build a machine for a mapper id and submapper (the T1 set of
    /// M3 plus the minimal MMC1 of D-22).
    pub fn from_rom(
        prg: Vec<u8>,
        mapper: u16,
        submapper: u8,
        chr: Chr,
        mirroring: Mirroring,
        prg_ram: bool,
        region: Region,
    ) -> Result<Self, BusError> {
        let bus = NesBus::with_mapper(prg, mapper, submapper, chr, mirroring, prg_ram, region)?;
        Ok(Machine {
            cpu: Cpu::new(),
            bus,
            region,
            instructions: 0,
        })
    }

    /// The master CPU cycle count (the bus's counter).
    pub fn cycle(&self) -> u64 {
        self.bus.cycle()
    }

    /// Power-on reset: the documented APU reset effect first ($4015
    /// as $00, triangle phase zero, DMC level bit 0), then seven CPU
    /// cycles, PC from `$FFFC`, `S -= 3`.
    pub fn reset(&mut self) {
        self.bus.apu_mut().reset();
        let before = self.bus.cycle();
        let cycles = u64::from(self.cpu.reset(&mut self.bus));
        let advanced = self.bus.cycle() - before;
        self.bus.advance_cycles(cycles - advanced);
    }

    /// Test-ROM automation entry: power state with the PC forced
    /// (nestest style) and the counters at the post-reset position.
    ///
    /// The stack pointer and registers keep their power-up values
    /// because no reset sequence runs — this reproduces the documented
    /// automation convention of the reference CPU test. The PPU jumps
    /// to dot 21 of line 0: the 21 dots the seven reset cycles would
    /// have ticked, matching the reference trace convention.
    pub fn start_automation(&mut self, pc: u16) {
        self.cpu = Cpu::new().with_pc(pc);
        self.bus.set_cycles(7);
        self.bus.ppu_mut().start_automation();
        self.instructions = 0;
    }

    /// Execute one instruction (or service one pending interrupt
    /// first); returns the CPU cycles the instruction itself consumed.
    ///
    /// NMI is edge-latched inside the PPU; the APU IRQ line
    /// (frame IRQ or DMC IRQ) is level-triggered and masked by the I
    /// flag — both are consumed here at the instruction boundary (the
    /// documented B1 sampling point; the delayed-IRQ and
    /// interrupt-hijack refinements are deferred to the scheduler
    /// milestone with real interrupt sources).
    pub fn step(&mut self) -> Result<u8, CpuError> {
        let nmi = self.bus.ppu_mut().take_nmi();
        let irq = self.bus.apu().irq_level() && self.cpu.p & I_FLAG == 0;
        let before = self.bus.cycle();
        let cycles = if nmi {
            self.cpu.service_interrupt(&mut self.bus, NMI_VECTOR)
        } else if irq {
            self.cpu.service_interrupt(&mut self.bus, IRQ_VECTOR)
        } else {
            self.cpu.step(&mut self.bus)?
        };
        let advanced = self.bus.cycle() - before;
        // True-up: every CPU cycle of the instruction ticks its PPU
        // dots, including the internal (non-access) ones. A DMA
        // already advanced more than the instruction's own cycles.
        let cycles = u64::from(cycles);
        if advanced < cycles {
            self.bus.advance_cycles(cycles - advanced);
        }
        self.instructions += 1;
        Ok(cycles as u8)
    }

    /// Run whole instructions until at least `budget` cycles are
    /// spent on the master clock.
    pub fn run_cycles(&mut self, budget: u64) -> Result<u64, CpuError> {
        while self.bus.cycle() < budget {
            self.step()?;
        }
        Ok(self.bus.cycle())
    }

    /// Run until `frames` PPU frame completions happened.
    ///
    /// Deterministic overshoot: the instruction that crosses a frame
    /// boundary finishes first (up to six cycles into the new frame,
    /// plus DMA cycles when one straddles the boundary).
    pub fn run_frames(&mut self, frames: u32) -> Result<u64, CpuError> {
        let target = self.bus.ppu().frame() + u64::from(frames);
        while self.bus.ppu().frame() < target {
            self.step()?;
        }
        Ok(self.bus.cycle())
    }

    /// PPU position as `(scanline, dot)` — the real device.
    pub fn ppu_position(&self) -> (u16, u16) {
        self.bus.ppu().position()
    }

    /// Push a live button snapshot for one controller port (the
    /// host-side input injection point: CLI script, autopilot,
    /// frontend keyboard). The snapshot is latched inside the pads at
    /// the next strobe.
    pub fn set_controller_buttons(&mut self, port: u8, buttons: u8) {
        self.bus.controllers_mut().set_live(port, buttons);
    }

    /// The PPU (frames, planes, protocol reads).
    pub fn ppu(&self) -> &Ppu {
        self.bus.ppu()
    }

    /// The APU (PCM capture, the state hash, the CLI).
    pub fn apu(&self) -> &Apu {
        self.bus.apu()
    }

    /// Mutable APU access (tests, the frontend).
    pub fn apu_mut(&mut self) -> &mut Apu {
        self.bus.apu_mut()
    }

    /// Enable or disable PCM byte capture (the fingerprint is
    /// computed regardless — see [`Self::pcm_hash`]).
    pub fn enable_pcm_capture(&mut self, on: bool) {
        self.bus.apu_mut().set_capture(on);
    }

    /// Drain the captured PCM samples (one per CPU cycle).
    pub fn take_pcm(&mut self) -> Vec<i16> {
        self.bus.apu_mut().take_pcm()
    }

    /// The PCM fingerprint (FNV-1a over every produced sample since
    /// power-on) — the same run, the same fingerprint, on every
    /// platform.
    pub fn pcm_hash(&self) -> u64 {
        self.bus.apu().pcm_hash()
    }

    /// Total PCM samples produced (one per CPU cycle since power-on).
    pub fn pcm_samples(&self) -> u64 {
        self.bus.apu().samples()
    }

    /// Deterministic state signature (FNV-1a 64) of the machine.
    ///
    /// Hashes everything two identical runs could distinguish:
    /// region, the master cycle and instruction counters, CPU
    /// registers, RAM, the open-bus latch and the complete PPU state
    /// (registers, loopy scroll, VRAM, palette, OAM, CHR RAM, timing,
    /// interrupt latch). Pure integer arithmetic, so the same inputs
    /// give the same hash on every architecture (D-16 requirement).
    pub fn state_hash(&self) -> u64 {
        let mut hash = FNV_OFFSET_BASIS;
        let region_byte = match self.region {
            Region::Ntsc => 0u8,
            Region::Pal => 1,
            Region::Dendy => 2,
        };
        hash = mix_u8(hash, region_byte);
        hash = mix_u64(hash, self.bus.cycle());
        hash = mix_u64(hash, self.instructions);
        hash = mix_u8(hash, self.cpu.a);
        hash = mix_u8(hash, self.cpu.x);
        hash = mix_u8(hash, self.cpu.y);
        hash = mix_u8(hash, self.cpu.sp);
        // The B flag is a phantom bit; hash the stored P as-is (it
        // never contains B in this implementation).
        hash = mix_u8(hash, self.cpu.p);
        hash = mix_u8(hash, u8::from(self.cpu.halted));
        hash = mix_u16(hash, self.cpu.pc);
        for byte in self.bus.ram() {
            hash = mix_u8(hash, *byte);
        }
        hash = mix_u8(hash, self.bus.latch());
        let (m0, m1, m2, m3) = self.bus.mapper_state();
        hash = mix_u8(hash, m0);
        hash = mix_u8(hash, m1);
        hash = mix_u8(hash, m2);
        hash = mix_u8(hash, m3);
        if let Some(ram) = self.bus.prg_ram() {
            for byte in ram {
                hash = mix_u8(hash, *byte);
            }
        }
        let ppu = self.bus.ppu();
        hash = mix_u8(hash, ppu.ctrl);
        hash = mix_u8(hash, ppu.mask);
        hash = mix_u8(hash, ppu.status_bits());
        hash = mix_u16(hash, ppu.scroll_v());
        hash = mix_u16(hash, ppu.scroll_t());
        hash = mix_u8(hash, ppu.fine_x());
        hash = mix_u8(hash, u8::from(ppu.write_toggle()));
        hash = mix_u8(hash, ppu.oam_addr());
        hash = mix_u8(hash, ppu.read_buffer());
        hash = mix_u16(hash, ppu.position().0);
        hash = mix_u16(hash, ppu.position().1);
        hash = mix_u64(hash, ppu.frame());
        hash = mix_u8(hash, u8::from(ppu.odd_frame()));
        hash = mix_u8(hash, u8::from(ppu.nmi_pending()));
        for byte in ppu.vram() {
            hash = mix_u8(hash, *byte);
        }
        for byte in ppu.palette_ram() {
            hash = mix_u8(hash, *byte);
        }
        for byte in ppu.oam() {
            hash = mix_u8(hash, *byte);
        }
        if let Some(chr) = ppu_chr_ram(ppu) {
            for byte in chr {
                hash = mix_u8(hash, *byte);
            }
        }
        // The complete APU state joins the state fingerprint (M4.2,
        // the declared D-37 migration): every channel, the frame
        // sequencer, the IRQ flags and the DC-blocker state — all of
        // it integer, so the fingerprint stays portable.
        for byte in self.bus.apu().state_bytes() {
            hash = mix_u8(hash, byte);
        }
        hash
    }
}

/// CHR RAM exposure for the hash (the ROM kind is static).
fn ppu_chr_ram(ppu: &Ppu) -> Option<&[u8; 0x2000]> {
    ppu.chr_ram_bytes()
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
        assert_eq!(machine.cycle(), 7);
        assert_eq!(machine.cpu.pc, 0x8000);
        assert_eq!(machine.cpu.sp, 0xFA);
        // The PPU sits exactly at the post-reset position (0, 21).
        assert_eq!(machine.ppu_position(), (0, 21));
    }

    #[test]
    fn automation_entry_keeps_power_state() {
        let mut machine = Machine::new(nop_prg(), Region::Ntsc).expect("prg");
        machine.start_automation(0xC000);
        assert_eq!(machine.cpu.pc, 0xC000);
        assert_eq!(machine.cpu.sp, 0xFD);
        assert_eq!(machine.cycle(), 7);
        assert_eq!(machine.ppu_position(), (0, 21));
    }

    #[test]
    fn run_cycles_counts_whole_instructions() {
        let mut machine = Machine::new(nop_prg(), Region::Ntsc).expect("prg");
        machine.reset();
        // 10 NOPs = 20 cycles from cycle 7.
        machine.run_cycles(27).expect("nop slide");
        assert_eq!(machine.cycle(), 27);
        assert_eq!(machine.instructions, 10);
        machine.run_cycles(28).expect("nop slide");
        assert_eq!(machine.cycle(), 29);
        assert_eq!(machine.instructions, 11);
    }

    #[test]
    fn ppu_position_matches_the_three_dots_per_cycle_rule() {
        let mut machine = Machine::new(nop_prg(), Region::Ntsc).expect("prg");
        machine.reset();
        // The nestest convention: at CPU cycle 7 the PPU sits at (0, 21).
        assert_eq!(machine.ppu_position(), (0, 21));
        machine.run_cycles(100).expect("nop slide");
        let dots = machine.cycle() * 3;
        let scanline = (dots / 341 % 262) as u16;
        let dot = (dots % 341) as u16;
        assert_eq!(machine.ppu_position(), (scanline, dot));
    }

    /// A cartridge whose reset entry loops on itself (`JMP $C000`),
    /// so long runs never wander off the PRG.
    fn loop_prg() -> Vec<u8> {
        let mut prg = vec![0xEA; 16 * 1024];
        prg[0..3].copy_from_slice(&[0x4C, 0x00, 0xC0]);
        prg[0x3FFA..0x4000].copy_from_slice(&[0x00, 0xC0, 0x00, 0xC0, 0x00, 0xC0]);
        prg
    }

    #[test]
    fn real_frames_count_without_a_fixed_budget() {
        let mut machine = Machine::new(loop_prg(), Region::Ntsc).expect("prg");
        machine.reset();
        machine.run_frames(3).expect("loop");
        // Rendering is off: every frame is the full 341x262 dots. The
        // third completion ticks dot 3*89,342 = 268,026, which lands
        // during cycle 89,342; the 3-cycle JMP instruction that
        // contains it ends at 89,344 (deterministic overshoot).
        assert_eq!(machine.cycle(), 89_344);
        assert_eq!(machine.ppu().frame(), 3);
    }

    #[test]
    fn nmi_fires_at_vblank_and_returns_exactly() {
        // A self-looping cartridge with an NMI handler that increments
        // $00F0 (placed off the execution path).
        let mut prg = vec![0xEA; 16 * 1024];
        prg[0..3].copy_from_slice(&[0x4C, 0x00, 0xC0]); // JMP $C000
        prg[0x20..0x23].copy_from_slice(&[0xE6, 0xF0, 0x40]); // $C020
        prg[0x3FFA..0x4000].copy_from_slice(&[0x20, 0xC0, 0x00, 0xC0, 0x00, 0xC0]);
        let mut machine = Machine::new(prg, Region::Ntsc).expect("prg");
        machine.reset();
        // Run to mid-frame, enable NMI, then let vblank arrive.
        machine.run_cycles(1000).expect("loop");
        machine.bus.write(0x2000, 0x80);
        machine.run_frames(1).expect("loop");
        assert_eq!(machine.bus.peek(0x00F0), 1, "the NMI handler ran");
        machine.run_frames(1).expect("loop");
        assert_eq!(machine.bus.peek(0x00F0), 2, "exactly one NMI per frame");
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
    fn different_ppu_vram_gives_a_different_hash() {
        let mut a = Machine::new(nop_prg(), Region::Ntsc).expect("prg");
        let mut b = Machine::new(nop_prg(), Region::Ntsc).expect("prg");
        a.reset();
        b.reset();
        // Write to $2000 VRAM through the register pair.
        a.bus.write(0x2006, 0x20);
        a.bus.write(0x2006, 0x00);
        a.bus.write(0x2007, 0x11);
        b.bus.write(0x2006, 0x20);
        b.bus.write(0x2006, 0x00);
        b.bus.write(0x2007, 0x22);
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

    /// A program that polls joypad 1 once (strobe + 8 reads) and
    /// stores the report at $0010..$0017, then loops.
    fn polling_prg() -> Vec<u8> {
        let mut code: Vec<u8> = Vec::new();
        // LDA #1; STA $4016 (strobe high)
        code.extend_from_slice(&[0xA9, 0x01, 0x8D, 0x16, 0x40]);
        // LDA #0; STA $4016 (strobe low)
        code.extend_from_slice(&[0xA9, 0x00, 0x8D, 0x16, 0x40]);
        // LDX #0
        code.extend_from_slice(&[0xA2, 0x00]);
        // loop: LDA $4016; AND #1; STA $10,X; INX; CPX #8; BNE loop
        let loop_start = code.len() as u16;
        code.extend_from_slice(&[0xAD, 0x16, 0x40]); // lda $4016
        code.extend_from_slice(&[0x29, 0x01]); // and #1
        code.extend_from_slice(&[0x95, 0x10]); // sta $10,x (zp,x)
        code.extend_from_slice(&[0xE8]); // inx
        code.extend_from_slice(&[0xE0, 0x08]); // cpx #8
        let back = loop_start.wrapping_sub((code.len() as u16) + 2);
        code.extend_from_slice(&[0xD0, (back as i8) as u8]); // bne loop
                                                             // JMP $8000 (an infinite slide, deterministic forever).
        code.extend_from_slice(&[0x4C, 0x00, 0x80]);
        let mut prg = vec![0xEA; 16 * 1024];
        prg[0..code.len()].copy_from_slice(&code);
        prg[0x3FFA..0x4000].copy_from_slice(&[0x00, 0x80, 0x00, 0x80, 0x00, 0x80]);
        prg
    }

    #[test]
    fn a_polled_controller_report_lands_in_ram() {
        use crate::input::BTN_A;
        let mut machine = Machine::new(polling_prg(), Region::Ntsc).expect("prg");
        machine.reset();
        machine.set_controller_buttons(0, BTN_A);
        machine.run_frames(1).expect("poll");
        let ram = machine.bus.ram();
        assert_eq!(ram[0x10], 1, "slot A");
        for slot in 1..8 {
            assert_eq!(ram[0x10 + slot], 0, "slot {slot} released");
        }
        // Same program, same buttons -> same hash (input determinism).
        let mut twin = Machine::new(polling_prg(), Region::Ntsc).expect("prg");
        twin.reset();
        twin.set_controller_buttons(0, BTN_A);
        twin.run_frames(1).expect("poll");
        assert_eq!(machine.state_hash(), twin.state_hash());
    }
}
