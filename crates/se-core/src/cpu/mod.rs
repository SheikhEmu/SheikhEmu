//! 6502 CPU (2A03 core, no decimal mode) execution engine.
//!
//! The engine executes all 151 documented opcodes with cycle-accurate
//! base timings, including the +1 page-cross penalty for indexed reads,
//! the 2/3/4 cycle branch timing, the JMP `(ind)` page-wrap bug and the
//! zero-page wrap of indexed zero-page modes. Undocumented opcodes are
//! rejected with [`CpuError::UnsupportedOp`] instead of being executed or
//! panicking; they arrive in M1.
//!
//! The 2A03 has no decimal mode, so ADC/SBC ignore the D flag (the flag
//! exists and CLD/SED manipulate it, but arithmetic is always binary).

use core::fmt;

pub mod opcodes;

use self::opcodes::{info, AddrMode, Mnemonic, OpInfo};

/// CPU-visible memory bus.
pub trait Bus {
    /// Read one byte.
    fn read(&self, addr: u16) -> u8;
    /// Write one byte.
    fn write(&mut self, addr: u16, value: u8);
}

/// Status register `P` bit masks.
mod flags {
    pub const C: u8 = 0x01;
    pub const Z: u8 = 0x02;
    pub const I: u8 = 0x04;
    pub const D: u8 = 0x08;
    pub const B: u8 = 0x10;
    pub const U: u8 = 0x20;
    pub const V: u8 = 0x40;
    pub const N: u8 = 0x80;
}

/// Errors produced while executing an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuError {
    /// The opcode is undocumented and not executed in this milestone.
    UnsupportedOp(u8),
}

impl fmt::Display for CpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CpuError::UnsupportedOp(opcode) => write!(
                f,
                "undocumented opcode 0x{opcode:02X} is not executed in M0"
            ),
        }
    }
}

impl std::error::Error for CpuError {}

/// A 6502 CPU instance.
#[derive(Clone, Copy, Debug)]
pub struct Cpu {
    /// Accumulator.
    pub a: u8,
    /// Index register X.
    pub x: u8,
    /// Index register Y.
    pub y: u8,
    /// Stack pointer (offset into page 1).
    pub sp: u8,
    /// Program counter.
    pub pc: u16,
    /// Status register `P`.
    pub p: u8,
}

impl Cpu {
    /// Power-up state per public documentation: A=X=Y=0, P=$34, S=$FD.
    pub const fn new() -> Self {
        Cpu {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFD,
            pc: 0,
            p: 0x34,
        }
    }

    /// Set the program counter (used after fetching the reset vector).
    pub const fn with_pc(self, pc: u16) -> Self {
        Cpu {
            pc,
            a: self.a,
            x: self.x,
            y: self.y,
            sp: self.sp,
            p: self.p,
        }
    }

    /// Fetch, decode and execute one instruction; returns the cycles used.
    pub fn step(&mut self, bus: &mut dyn Bus) -> Result<u8, CpuError> {
        let opcode = self.fetch8(bus);
        let opinfo = info(opcode);
        if !opinfo.official {
            return Err(CpuError::UnsupportedOp(opcode));
        }
        Ok(self.execute(bus, opinfo))
    }

    // ---- register and flag helpers -------------------------------------

    fn set_nz(&mut self, value: u8) {
        if value == 0 {
            self.p |= flags::Z;
        } else {
            self.p &= !flags::Z;
        }
        if value & flags::N != 0 {
            self.p |= flags::N;
        } else {
            self.p &= !flags::N;
        }
    }

    fn set_flag(&mut self, mask: u8, on: bool) {
        if on {
            self.p |= mask;
        } else {
            self.p &= !mask;
        }
    }

    fn carry(&self) -> bool {
        self.p & flags::C != 0
    }

    // ---- program fetch --------------------------------------------------

    fn fetch8(&mut self, bus: &mut dyn Bus) -> u8 {
        let byte = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        byte
    }

    fn fetch16(&mut self, bus: &mut dyn Bus) -> u16 {
        let lo = u16::from(self.fetch8(bus));
        let hi = u16::from(self.fetch8(bus));
        lo | hi << 8
    }

    fn read16(&self, bus: &dyn Bus, addr: u16) -> u16 {
        let lo = u16::from(bus.read(addr));
        let hi = u16::from(bus.read(addr.wrapping_add(1)));
        lo | hi << 8
    }

    /// Read a 16-bit pointer stored at a zero-page address; the high byte
    /// comes from `(zp + 1) & 0xFF`, wrapping inside page 0.
    fn read16_zp(&self, bus: &dyn Bus, zp: u8) -> u16 {
        let lo = u16::from(bus.read(u16::from(zp)));
        let hi = u16::from(bus.read(u16::from(zp.wrapping_add(1))));
        lo | hi << 8
    }

    /// Read the target of `JMP (ind)` with the $xxFF page-wrap bug: the high
    /// byte comes from the start of the same page, not from the next page.
    fn read16_indirect_buggy(&self, bus: &dyn Bus, ptr: u16) -> u16 {
        let lo = u16::from(bus.read(ptr));
        let hi_addr = (ptr & 0xFF00) | (ptr.wrapping_add(1) & 0xFF);
        let hi = u16::from(bus.read(hi_addr));
        lo | hi << 8
    }

    // ---- stack ----------------------------------------------------------

    fn push(&mut self, bus: &mut dyn Bus, value: u8) {
        bus.write(0x0100 | u16::from(self.sp), value);
        self.sp = self.sp.wrapping_sub(1);
    }

    fn pull(&mut self, bus: &mut dyn Bus) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        bus.read(0x0100 | u16::from(self.sp))
    }

    fn push16(&mut self, bus: &mut dyn Bus, value: u16) {
        self.push(bus, (value >> 8) as u8);
        self.push(bus, (value & 0xFF) as u8);
    }

    fn pull16(&mut self, bus: &mut dyn Bus) -> u16 {
        let lo = u16::from(self.pull(bus));
        let hi = u16::from(self.pull(bus));
        lo | hi << 8
    }

    // ---- addressing -----------------------------------------------------

    /// Resolve the effective address for `mode`, consuming operand bytes
    /// (advancing PC) and reporting whether a page was crossed.
    fn resolve(&mut self, bus: &mut dyn Bus, mode: AddrMode) -> (u16, bool) {
        match mode {
            AddrMode::Imp | AddrMode::Acc => (0, false),
            AddrMode::Imm => {
                let addr = self.pc;
                self.pc = self.pc.wrapping_add(1);
                (addr, false)
            }
            AddrMode::Zp => (u16::from(self.fetch8(bus)), false),
            AddrMode::Zpx => {
                let zp = self.fetch8(bus);
                (u16::from(zp.wrapping_add(self.x)), false)
            }
            AddrMode::Zpy => {
                let zp = self.fetch8(bus);
                (u16::from(zp.wrapping_add(self.y)), false)
            }
            AddrMode::Abs => (self.fetch16(bus), false),
            AddrMode::Abx => {
                let base = self.fetch16(bus);
                let addr = base.wrapping_add(u16::from(self.x));
                (addr, crossed_page(base, addr))
            }
            AddrMode::Aby => {
                let base = self.fetch16(bus);
                let addr = base.wrapping_add(u16::from(self.y));
                (addr, crossed_page(base, addr))
            }
            AddrMode::Ind => {
                let ptr = self.fetch16(bus);
                (self.read16_indirect_buggy(bus, ptr), false)
            }
            AddrMode::Izx => {
                let zp = self.fetch8(bus).wrapping_add(self.x);
                (self.read16_zp(bus, zp), false)
            }
            AddrMode::Izy => {
                let zp = self.fetch8(bus);
                let base = self.read16_zp(bus, zp);
                let addr = base.wrapping_add(u16::from(self.y));
                (addr, crossed_page(base, addr))
            }
            AddrMode::Rel => {
                let offset = self.fetch8(bus) as i8;
                let target = self.pc.wrapping_add(offset as i16 as u16);
                (target, crossed_page(self.pc, target))
            }
        }
    }

    // ---- arithmetic helpers ---------------------------------------------

    /// `A = A + addend + C` (binary; the 2A03 has no decimal mode).
    fn do_add(&mut self, addend: u8) {
        let a = self.a;
        let sum = u16::from(a) + u16::from(addend) + u16::from(u8::from(self.carry()));
        let result = (sum & 0xFF) as u8;
        self.set_flag(flags::C, sum > 0xFF);
        // Overflow: addend and A share a sign, and the result flips it.
        let overflow = (!(a ^ addend) & (a ^ result)) & flags::N != 0;
        self.set_flag(flags::V, overflow);
        self.a = result;
        self.set_nz(result);
    }

    /// Compare a register against memory (r - m).
    fn do_cmp(&mut self, reg: u8, m: u8) {
        self.set_flag(flags::C, reg >= m);
        self.set_nz(reg.wrapping_sub(m));
    }

    /// Common read cycle accounting: base + optional page-cross penalty.
    fn read_cycles(&self, opinfo: OpInfo, crossed: bool) -> u8 {
        opinfo.cycles + u8::from(opinfo.page_cross && crossed)
    }

    // ---- execution ------------------------------------------------------

    fn execute(&mut self, bus: &mut dyn Bus, opinfo: OpInfo) -> u8 {
        use Mnemonic as M;

        match opinfo.mnemonic {
            // Loads.
            M::Lda | M::Ldx | M::Ldy => {
                let (addr, crossed) = self.resolve(bus, opinfo.mode);
                let m = bus.read(addr);
                match opinfo.mnemonic {
                    M::Lda => self.a = m,
                    M::Ldx => self.x = m,
                    M::Ldy => self.y = m,
                    _ => unreachable!("load family"),
                }
                self.set_nz(m);
                self.read_cycles(opinfo, crossed)
            }

            // Stores (no page-cross penalty on writes).
            M::Sta | M::Stx | M::Sty => {
                let (addr, _) = self.resolve(bus, opinfo.mode);
                let value = match opinfo.mnemonic {
                    M::Sta => self.a,
                    M::Stx => self.x,
                    M::Sty => self.y,
                    _ => unreachable!("store family"),
                };
                bus.write(addr, value);
                opinfo.cycles
            }

            // Transfers.
            M::Tax => {
                self.x = self.a;
                self.set_nz(self.x);
                opinfo.cycles
            }
            M::Tay => {
                self.y = self.a;
                self.set_nz(self.y);
                opinfo.cycles
            }
            M::Txa => {
                self.a = self.x;
                self.set_nz(self.a);
                opinfo.cycles
            }
            M::Tya => {
                self.a = self.y;
                self.set_nz(self.a);
                opinfo.cycles
            }
            M::Tsx => {
                self.x = self.sp;
                self.set_nz(self.x);
                opinfo.cycles
            }
            M::Txs => {
                self.sp = self.x;
                opinfo.cycles
            }

            // Arithmetic and logic.
            M::Adc => {
                let (addr, crossed) = self.resolve(bus, opinfo.mode);
                self.do_add(bus.read(addr));
                self.read_cycles(opinfo, crossed)
            }
            M::Sbc => {
                let (addr, crossed) = self.resolve(bus, opinfo.mode);
                self.do_add(!bus.read(addr));
                self.read_cycles(opinfo, crossed)
            }
            M::And => {
                let (addr, crossed) = self.resolve(bus, opinfo.mode);
                self.a &= bus.read(addr);
                self.set_nz(self.a);
                self.read_cycles(opinfo, crossed)
            }
            M::Ora => {
                let (addr, crossed) = self.resolve(bus, opinfo.mode);
                self.a |= bus.read(addr);
                self.set_nz(self.a);
                self.read_cycles(opinfo, crossed)
            }
            M::Eor => {
                let (addr, crossed) = self.resolve(bus, opinfo.mode);
                self.a ^= bus.read(addr);
                self.set_nz(self.a);
                self.read_cycles(opinfo, crossed)
            }
            M::Cmp => {
                let (addr, crossed) = self.resolve(bus, opinfo.mode);
                self.do_cmp(self.a, bus.read(addr));
                self.read_cycles(opinfo, crossed)
            }
            M::Cpx => {
                let (addr, crossed) = self.resolve(bus, opinfo.mode);
                self.do_cmp(self.x, bus.read(addr));
                self.read_cycles(opinfo, crossed)
            }
            M::Cpy => {
                let (addr, crossed) = self.resolve(bus, opinfo.mode);
                self.do_cmp(self.y, bus.read(addr));
                self.read_cycles(opinfo, crossed)
            }
            M::Bit => {
                let (addr, crossed) = self.resolve(bus, opinfo.mode);
                let m = bus.read(addr);
                self.set_flag(flags::Z, self.a & m == 0);
                self.set_flag(flags::N, m & flags::N != 0);
                self.set_flag(flags::V, m & flags::V != 0);
                self.read_cycles(opinfo, crossed)
            }

            // Shifts and rotates.
            M::Asl | M::Lsr | M::Rol | M::Ror => {
                let (addr, _) = self.resolve(bus, opinfo.mode);
                let value = if opinfo.mode == AddrMode::Acc {
                    self.a
                } else {
                    bus.read(addr)
                };
                let old_carry = u8::from(self.carry());
                let (result, new_carry) = match opinfo.mnemonic {
                    M::Asl => (value << 1, value >> 7),
                    M::Lsr => (value >> 1, value & 1),
                    M::Rol => ((value << 1) | old_carry, value >> 7),
                    M::Ror => ((value >> 1) | old_carry << 7, value & 1),
                    _ => unreachable!("shift family"),
                };
                self.set_flag(flags::C, new_carry != 0);
                self.set_nz(result);
                if opinfo.mode == AddrMode::Acc {
                    self.a = result;
                } else {
                    bus.write(addr, result);
                }
                opinfo.cycles
            }

            // Inc / dec.
            M::Inc | M::Dec => {
                let (addr, _) = self.resolve(bus, opinfo.mode);
                let m = bus.read(addr);
                let result = if opinfo.mnemonic == M::Inc {
                    m.wrapping_add(1)
                } else {
                    m.wrapping_sub(1)
                };
                self.set_nz(result);
                bus.write(addr, result);
                opinfo.cycles
            }
            M::Inx => {
                self.x = self.x.wrapping_add(1);
                self.set_nz(self.x);
                opinfo.cycles
            }
            M::Iny => {
                self.y = self.y.wrapping_add(1);
                self.set_nz(self.y);
                opinfo.cycles
            }
            M::Dex => {
                self.x = self.x.wrapping_sub(1);
                self.set_nz(self.x);
                opinfo.cycles
            }
            M::Dey => {
                self.y = self.y.wrapping_sub(1);
                self.set_nz(self.y);
                opinfo.cycles
            }

            // Control flow.
            M::Jmp => {
                let (addr, _) = self.resolve(bus, opinfo.mode);
                self.pc = addr;
                opinfo.cycles
            }
            M::Jsr => {
                // Operand is consumed before pushing PC-1.
                let target = self.fetch16(bus);
                self.push16(bus, self.pc.wrapping_sub(1));
                self.pc = target;
                opinfo.cycles
            }
            M::Rts => {
                let return_addr = self.pull16(bus);
                self.pc = return_addr.wrapping_add(1);
                opinfo.cycles
            }
            M::Rti => {
                let p = self.pull(bus);
                self.p = (p & !flags::B) | flags::U;
                let pc = self.pull16(bus);
                self.pc = pc;
                opinfo.cycles
            }
            M::Brk => {
                // Skip the signature byte after the opcode.
                self.pc = self.pc.wrapping_add(1);
                self.push16(bus, self.pc);
                self.push(bus, self.p | flags::B | flags::U);
                self.p |= flags::I;
                self.pc = self.read16(bus, 0xFFFE);
                opinfo.cycles
            }
            M::Bcc | M::Bcs | M::Beq | M::Bmi | M::Bne | M::Bpl | M::Bvc | M::Bvs => {
                let taken = match opinfo.mnemonic {
                    M::Bcc => !self.carry(),
                    M::Bcs => self.carry(),
                    M::Beq => self.p & flags::Z != 0,
                    M::Bne => self.p & flags::Z == 0,
                    M::Bmi => self.p & flags::N != 0,
                    M::Bpl => self.p & flags::N == 0,
                    M::Bvs => self.p & flags::V != 0,
                    M::Bvc => self.p & flags::V == 0,
                    _ => unreachable!("branch family"),
                };
                let (target, crossed) = self.resolve(bus, opinfo.mode);
                if taken {
                    self.pc = target;
                    // 3 cycles when taken, plus 1 if the page was crossed.
                    opinfo.cycles + 1 + u8::from(crossed)
                } else {
                    opinfo.cycles
                }
            }

            // Stack.
            M::Pha => {
                self.push(bus, self.a);
                opinfo.cycles
            }
            M::Php => {
                self.push(bus, self.p | flags::B | flags::U);
                opinfo.cycles
            }
            M::Pla => {
                let value = self.pull(bus);
                self.a = value;
                self.set_nz(value);
                opinfo.cycles
            }
            M::Plp => {
                let value = self.pull(bus);
                self.p = (value & !flags::B) | flags::U;
                opinfo.cycles
            }

            // Flag operations.
            M::Clc => {
                self.p &= !flags::C;
                opinfo.cycles
            }
            M::Sec => {
                self.p |= flags::C;
                opinfo.cycles
            }
            M::Cli => {
                self.p &= !flags::I;
                opinfo.cycles
            }
            M::Sei => {
                self.p |= flags::I;
                opinfo.cycles
            }
            M::Cld => {
                self.p &= !flags::D;
                opinfo.cycles
            }
            M::Sed => {
                self.p |= flags::D;
                opinfo.cycles
            }
            M::Clv => {
                self.p &= !flags::V;
                opinfo.cycles
            }
            M::Nop => opinfo.cycles,

            // Undocumented mnemonics never reach execute(); step() rejects
            // them first. This arm exists only to keep the match exhaustive.
            M::Kil
            | M::Slo
            | M::Rla
            | M::Sre
            | M::Rra
            | M::Sax
            | M::Lax
            | M::Sha
            | M::Shx
            | M::Shy
            | M::Tas
            | M::Las
            | M::Dcp
            | M::Isc
            | M::Anc
            | M::Alr
            | M::Arr
            | M::Sbx
            | M::Usbc
            | M::Ane => unreachable!("undocumented mnemonic reached execute()"),
        }
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether `addr` crosses the 64 KiB page boundary of `base`.
fn crossed_page(base: u16, addr: u16) -> bool {
    (base ^ addr) & 0xFF00 != 0
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat 64 KiB memory bus used for CPU tests.
    struct ArrayBus {
        mem: Box<[u8; 0x10000]>,
    }

    impl ArrayBus {
        fn new() -> Self {
            ArrayBus {
                mem: Box::new([0; 0x10000]),
            }
        }
        fn load(&mut self, addr: u16, bytes: &[u8]) {
            for (i, byte) in bytes.iter().enumerate() {
                self.mem[usize::from(addr) + i] = *byte;
            }
        }
        fn poke(&mut self, addr: u16, value: u8) {
            self.mem[usize::from(addr)] = value;
        }
        fn peek(&self, addr: u16) -> u8 {
            self.mem[usize::from(addr)]
        }
    }

    impl Bus for ArrayBus {
        fn read(&self, addr: u16) -> u8 {
            self.mem[usize::from(addr)]
        }
        fn write(&mut self, addr: u16, value: u8) {
            self.mem[usize::from(addr)] = value;
        }
    }

    fn cpu_at(pc: u16, program: &[u8]) -> (Cpu, ArrayBus) {
        let mut bus = ArrayBus::new();
        bus.load(pc, program);
        (Cpu::new().with_pc(pc), bus)
    }

    /// Execute every instruction of `program` until PC reaches the end of
    /// the loaded range, then return the CPU and bus. Programs used with
    /// this helper must not jump outside the loaded range.
    fn run_program(pc: u16, program: &[u8]) -> (Cpu, ArrayBus) {
        let (mut cpu, mut bus) = cpu_at(pc, program);
        let end = pc + u16::try_from(program.len()).expect("test program under 64 KiB");
        let mut guard = 0;
        while cpu.pc < end {
            cpu.step(&mut bus).expect("official opcode");
            guard += 1;
            assert!(guard <= program.len() + 1, "run_program did not terminate");
        }
        (cpu, bus)
    }

    const C: u8 = flags::C;
    const Z: u8 = flags::Z;
    const V: u8 = flags::V;
    const N: u8 = flags::N;
    const I: u8 = flags::I;

    // ---- state -----------------------------------------------------------

    #[test]
    fn powerup_state_matches_documented_values() {
        let cpu = Cpu::new();
        assert_eq!(cpu.a, 0);
        assert_eq!(cpu.x, 0);
        assert_eq!(cpu.y, 0);
        assert_eq!(cpu.sp, 0xFD);
        assert_eq!(cpu.p, 0x34);
        assert_eq!(cpu.pc, 0);
    }

    // ---- loads / stores --------------------------------------------------

    #[test]
    fn lda_imm_sets_register_and_flags() {
        let (cpu, _) = run_program(0xC000, &[0xA9, 0x80]);
        assert_eq!(cpu.a, 0x80);
        assert!(cpu.p & N != 0);
        assert!(cpu.p & Z == 0);
        assert_eq!(cpu.pc, 0xC002);
    }

    #[test]
    fn lda_imm_zero_sets_z() {
        let (cpu, _) = run_program(0xC000, &[0xA9, 0x00]);
        assert!(cpu.p & Z != 0);
        assert!(cpu.p & N == 0);
    }

    #[test]
    fn ldx_imm_ldy_imm() {
        let (cpu, _) = run_program(0xC000, &[0xA2, 0x42]);
        assert_eq!(cpu.x, 0x42);
        let (cpu, _) = run_program(0xC000, &[0xA0, 0x42]);
        assert_eq!(cpu.y, 0x42);
    }

    #[test]
    fn lda_zp_and_sta_zp() {
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA5, 0x10]);
        bus.poke(0x0010, 0x33);
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.a, 0x33);

        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA9, 0x77, 0x85, 0x20]);
        cpu.step(&mut bus).expect("official opcode");
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(bus.peek(0x0020), 0x77);
    }

    #[test]
    fn sta_abs_and_stx_sty_zp() {
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA9, 0x11, 0x8D, 0x00, 0x02]);
        cpu.step(&mut bus).expect("official opcode");
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(bus.peek(0x0200), 0x11);

        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA2, 0x99, 0x86, 0x30]);
        cpu.step(&mut bus).expect("official opcode");
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(bus.peek(0x0030), 0x99);

        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA0, 0x88, 0x84, 0x40]);
        cpu.step(&mut bus).expect("official opcode");
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(bus.peek(0x0040), 0x88);
    }

    #[test]
    fn sta_zpx_wraps_inside_page_zero() {
        // STA $10,X with X=$FF writes to $0F, not $010F.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA2, 0xFF, 0xA9, 0x5A, 0x95, 0x10]);
        for _ in 0..3 {
            cpu.step(&mut bus).expect("official opcode");
        }
        assert_eq!(bus.peek(0x000F), 0x5A);
        assert_eq!(bus.peek(0x010F), 0);
    }

    #[test]
    fn lda_abx_with_and_without_page_cross() {
        // Base $1000, X=$0F: no crossing, 4 cycles.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xBD, 0x00, 0x10]);
        bus.poke(0x100F, 0x2A);
        cpu.x = 0x0F;
        let cycles = cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cycles, 4);
        assert_eq!(cpu.a, 0x2A);

        // Base $10F0, X=$10: crossing into page 11, 4+1 cycles.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xBD, 0xF0, 0x10]);
        bus.poke(0x1100, 0x3B);
        cpu.x = 0x10;
        let cycles = cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cycles, 5);
        assert_eq!(cpu.a, 0x3B);
    }

    #[test]
    fn lda_aby_page_cross_penalty() {
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xB9, 0xF0, 0x10]);
        bus.poke(0x1100, 0x1D);
        cpu.y = 0x10;
        let cycles = cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cycles, 5);
    }

    #[test]
    fn lda_izx_wraps_pointer_inside_page_zero() {
        // LDA ($FF,X) with X=$02 reads the pointer from $01/$02.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA2, 0x02, 0xA1, 0xFF]);
        bus.poke(0x0001, 0x00);
        bus.poke(0x0002, 0x30);
        bus.poke(0x3000, 0x7E);
        for _ in 0..2 {
            cpu.step(&mut bus).expect("official opcode");
        }
        assert_eq!(cpu.a, 0x7E);
    }

    #[test]
    fn lda_izy_with_and_without_page_cross() {
        // Pointer $2000, Y=$10: no crossing, 5 cycles.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xB1, 0x10]);
        bus.poke(0x0010, 0x00);
        bus.poke(0x0011, 0x20);
        bus.poke(0x2010, 0x41);
        cpu.y = 0x10;
        let cycles = cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cycles, 5);
        assert_eq!(cpu.a, 0x41);

        // Pointer $20F0, Y=$10: crossing, 6 cycles.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xB1, 0x10]);
        bus.poke(0x0010, 0xF0);
        bus.poke(0x0011, 0x20);
        bus.poke(0x2100, 0x42);
        cpu.y = 0x10;
        let cycles = cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cycles, 6);
        assert_eq!(cpu.a, 0x42);
    }

    // ---- arithmetic ------------------------------------------------------

    #[test]
    fn adc_flag_table() {
        // 0x50 + 0x50: overflow, no carry.
        let (cpu, _) = run_program(0xC000, &[0xA9, 0x50, 0x69, 0x50]);
        assert_eq!(cpu.a, 0xA0);
        assert!(cpu.p & V != 0);
        assert!(cpu.p & C == 0);

        // 0x90 + 0x90: carry and overflow.
        let (cpu, _) = run_program(0xC000, &[0xA9, 0x90, 0x69, 0x90]);
        assert_eq!(cpu.a, 0x20);
        assert!(cpu.p & C != 0);
        assert!(cpu.p & V != 0);

        // 0xFF + 0x01: carry with zero result.
        let (cpu, _) = run_program(0xC000, &[0xA9, 0xFF, 0x69, 0x01]);
        assert_eq!(cpu.a, 0x00);
        assert!(cpu.p & C != 0);
        assert!(cpu.p & Z != 0);

        // 0x0F + 0x01: clean positive result, no flags.
        let (cpu, _) = run_program(0xC000, &[0xA9, 0x0F, 0x69, 0x01]);
        assert_eq!(cpu.a, 0x10);
        assert!(cpu.p & (C | V | Z | N) == 0);
    }

    #[test]
    fn adc_with_carry_in() {
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x69, 0x10]);
        cpu.a = 0x20;
        cpu.p |= C;
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.a, 0x31);
    }

    #[test]
    fn sbc_flag_table() {
        // 0x00 - 0x01 with carry set: borrow, wraps to 0xFF.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xE9, 0x01]);
        cpu.a = 0x00;
        cpu.p |= C;
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.a, 0xFF);
        assert!(cpu.p & C == 0);
        assert!(cpu.p & N != 0);

        // 5 - 3 with carry: no borrow.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xE9, 0x03]);
        cpu.a = 0x05;
        cpu.p |= C;
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.a, 0x02);
        assert!(cpu.p & C != 0);

        // 0x7F - 0xFF with carry: signed overflow (127 - (-1) = 128).
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xE9, 0xFF]);
        cpu.a = 0x7F;
        cpu.p |= C;
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.a, 0x80);
        assert!(cpu.p & V != 0);
    }

    #[test]
    fn logic_ops() {
        let (cpu, _) = run_program(0xC000, &[0xA9, 0xF0, 0x29, 0x0F]);
        assert_eq!(cpu.a, 0x00);
        assert!(cpu.p & Z != 0);

        let (cpu, _) = run_program(0xC000, &[0xA9, 0xF0, 0x09, 0x0F]);
        assert_eq!(cpu.a, 0xFF);

        let (cpu, _) = run_program(0xC000, &[0xA9, 0xF0, 0x49, 0x3C]);
        assert_eq!(cpu.a, 0xCC);
    }

    #[test]
    fn cmp_flag_table() {
        // Equal: carry and zero.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xC9, 0x10]);
        cpu.a = 0x10;
        cpu.step(&mut bus).expect("official opcode");
        assert!(cpu.p & C != 0);
        assert!(cpu.p & Z != 0);

        // Below: no carry, negative.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xC9, 0x10]);
        cpu.a = 0x0F;
        cpu.step(&mut bus).expect("official opcode");
        assert!(cpu.p & C == 0);
        assert!(cpu.p & N != 0);

        // Above: carry, positive.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xC9, 0x10]);
        cpu.a = 0x20;
        cpu.step(&mut bus).expect("official opcode");
        assert!(cpu.p & C != 0);
        assert!(cpu.p & N == 0);
    }

    #[test]
    fn bit_sets_n_v_z() {
        // N and V come from memory, Z from A & M.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x24, 0x10]);
        cpu.a = 0x40;
        bus.poke(0x0010, 0xC0);
        cpu.step(&mut bus).expect("official opcode");
        assert!(cpu.p & N != 0);
        assert!(cpu.p & V != 0);
        assert!(cpu.p & Z == 0);

        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x24, 0x10]);
        cpu.a = 0x02;
        bus.poke(0x0010, 0xC0);
        cpu.step(&mut bus).expect("official opcode");
        assert!(cpu.p & Z != 0);
    }

    // ---- shifts ----------------------------------------------------------

    #[test]
    fn asl_acc_and_lsr_acc() {
        let (cpu, _) = run_program(0xC000, &[0xA9, 0x81, 0x0A]);
        assert_eq!(cpu.a, 0x02);
        assert!(cpu.p & C != 0);

        let (cpu, _) = run_program(0xC000, &[0xA9, 0x03, 0x4A]);
        assert_eq!(cpu.a, 0x01);
        assert!(cpu.p & C != 0);
    }

    #[test]
    fn rol_acc_and_ror_acc_carry_chains() {
        // ROL with carry set: 0x01 -> 0x03, C stays 0.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x2A]);
        cpu.a = 0x01;
        cpu.p |= C;
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.a, 0x03);
        assert!(cpu.p & C == 0);

        // ROR with carry set: 0x08 -> 0x84, C goes 0.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x6A]);
        cpu.a = 0x08;
        cpu.p |= C;
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.a, 0x84);
        assert!(cpu.p & C == 0);
    }

    #[test]
    fn asl_zp_read_modify_write() {
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x06, 0x10]);
        bus.poke(0x0010, 0xC0);
        let cycles = cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cycles, 5);
        assert_eq!(bus.peek(0x0010), 0x80);
        assert!(cpu.p & C != 0);
    }

    #[test]
    fn inc_dec_memory_and_registers() {
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xE6, 0x10, 0xC6, 0x11]);
        bus.poke(0x0010, 0xFF);
        bus.poke(0x0011, 0x00);
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(bus.peek(0x0010), 0x00);
        assert!(cpu.p & Z != 0);
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(bus.peek(0x0011), 0xFF);
        assert!(cpu.p & N != 0);

        // INX wraps 0xFF -> 0x00, DEX wraps 0x00 -> 0xFF.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xE8]);
        cpu.x = 0xFF;
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.x, 0x00);
        assert!(cpu.p & Z != 0);

        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xCA]);
        cpu.x = 0x00;
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.x, 0xFF);
        assert!(cpu.p & N != 0);

        let (cpu, _) = run_program(0xC000, &[0xC8, 0x88]);
        assert_eq!(cpu.y, 0x00);
    }

    // ---- control flow -----------------------------------------------------

    #[test]
    fn jmp_abs_and_cycles() {
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x4C, 0x00, 0x80]);
        let cycles = cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.pc, 0x8000);
        assert_eq!(cycles, 3);
    }

    #[test]
    fn jmp_ind_wraps_pointer_inside_page() {
        // JMP ($2FFF): low byte from $2FFF, high byte from $2F00 (the start
        // of the same page, per the 6502 page-wrap bug).
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x6C, 0xFF, 0x2F]);
        bus.poke(0x2FFF, 0x34);
        bus.poke(0x2F00, 0x12);
        bus.poke(0x3000, 0xAB); // must NOT be used
        let cycles = cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.pc, 0x1234);
        assert_eq!(cycles, 5);
    }

    #[test]
    fn jsr_rts_roundtrip() {
        // Subroutine at $1234 returns, then execution continues at $C003.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x20, 0x34, 0x12]);
        bus.load(0x1234, &[0x60]); // RTS
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.pc, 0x1234);
        // JSR pushed the address of its own operand's last byte ($C002).
        assert_eq!(bus.peek(0x01FD), 0xC0);
        assert_eq!(bus.peek(0x01FC), 0x02);
        assert_eq!(cpu.sp, 0xFB);
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.pc, 0xC003);
        assert_eq!(cpu.sp, 0xFD);
    }

    #[test]
    fn brk_rti_roundtrip() {
        // BRK at $C000 pushes $C002 and vectors through $FFFE to $0800;
        // RTI there restores P and returns to $C002.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x00, 0xEA]);
        bus.load(0x0800, &[0x40]); // RTI
        bus.poke(0xFFFE, 0x00);
        bus.poke(0xFFFF, 0x08);
        let cycles = cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cycles, 7);
        assert_eq!(cpu.pc, 0x0800);
        assert!(cpu.p & I != 0);
        // P was pushed with B and U set, one byte below the return address.
        assert_eq!(bus.peek(0x01FD), 0xC0);
        assert_eq!(bus.peek(0x01FC), 0x02);
        let pushed_p = bus.peek(0x01FB);
        assert!(pushed_p & flags::B != 0);
        assert!(pushed_p & flags::U != 0);
        assert_eq!(cpu.sp, 0xFA);

        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.pc, 0xC002);
        assert_eq!(cpu.sp, 0xFD);
        // B is never restored by RTI/PLP.
        assert!(cpu.p & flags::B == 0);
    }

    #[test]
    fn branches_taken_and_not_taken_cycle_counts() {
        // BEQ with Z clear: not taken, 2 cycles.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xF0, 0x10]);
        cpu.p &= !Z;
        let cycles = cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cycles, 2);
        assert_eq!(cpu.pc, 0xC002);

        // BEQ with Z set, same page: taken, 3 cycles.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xF0, 0x10]);
        cpu.p |= Z;
        let cycles = cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cycles, 3);
        assert_eq!(cpu.pc, 0xC012);

        // BEQ with Z set, crossing into the next page: taken, 4 cycles.
        // Opcode at $C1EE, operand at $C1EF, so PC after the operand is
        // $C1F0; target $C200 lies one page away.
        let (mut cpu, mut bus) = cpu_at(0xC1EE, &[0xF0, 0x10]);
        cpu.p |= Z;
        let cycles = cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cycles, 4);
        assert_eq!(cpu.pc, 0xC200);
    }

    #[test]
    fn branch_backward_negative_offset() {
        // BNE -3 jumps three bytes back (opcode + operand + 1).
        let (mut cpu, mut bus) = cpu_at(0xC010, &[0xD0, 0xFD]);
        cpu.p &= !Z;
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.pc, 0xC00F);
    }

    #[test]
    fn all_branch_conditions() {
        // (opcode, flag mask, expected taken) with the flag bit set.
        let cases: [(u8, u8, bool); 8] = [
            (0x10, flags::N, false), // BPL
            (0x30, flags::N, true),  // BMI
            (0x50, flags::V, false), // BVC
            (0x70, flags::V, true),  // BVS
            (0x90, flags::C, false), // BCC
            (0xB0, flags::C, true),  // BCS
            (0xD0, flags::Z, false), // BNE
            (0xF0, flags::Z, true),  // BEQ
        ];
        for (opcode, mask, expected_taken) in cases {
            let (mut cpu, mut bus) = cpu_at(0xC000, &[opcode, 0x08]);
            cpu.p |= mask;
            cpu.step(&mut bus).expect("official opcode");
            let taken = cpu.pc == 0xC00A;
            assert_eq!(
                taken, expected_taken,
                "opcode 0x{opcode:02X} with flag 0x{mask:02X}"
            );
        }
    }

    // ---- stack -------------------------------------------------------------

    #[test]
    fn pha_pla_and_php_plp_roundtrip() {
        let (cpu, _bus) = run_program(0xC000, &[0xA9, 0x42, 0x48, 0xA9, 0x00, 0x68]);
        assert_eq!(cpu.a, 0x42);
        assert_eq!(cpu.sp, 0xFD);

        // PHP pushes B|U set; PLP restores flags except B.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x08, 0x28]);
        cpu.p = 0x01;
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(bus.peek(0x01FD), 0x01 | flags::B | flags::U);
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.p & !flags::U, 0x01);
    }

    // ---- transfers and flags -------------------------------------------------

    #[test]
    fn transfers() {
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA9, 0x30, 0xAA, 0xA8, 0x8A, 0x98]);
        for _ in 0..6 {
            cpu.step(&mut bus).expect("official opcode");
        }
        assert_eq!(cpu.a, 0x30);
        assert_eq!(cpu.x, 0x30);
        assert_eq!(cpu.y, 0x30);

        // TSX sets flags; TXS does not.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xBA]);
        cpu.sp = 0x00;
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.x, 0x00);
        assert!(cpu.p & Z != 0);

        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x9A]);
        cpu.x = 0x80;
        let p_before = cpu.p;
        cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cpu.sp, 0x80);
        assert_eq!(cpu.p, p_before);
    }

    #[test]
    fn flag_operations() {
        let (cpu, _) = run_program(0xC000, &[0x38, 0x18]);
        assert!(cpu.p & C == 0);
        let (cpu, _) = run_program(0xC000, &[0x18, 0x38]);
        assert!(cpu.p & C != 0);
        let (cpu, _) = run_program(0xC000, &[0x78, 0x58]);
        assert!(cpu.p & I == 0);
        let (cpu, _) = run_program(0xC000, &[0xF8, 0xB8]);
        assert!(cpu.p & V == 0);
        let (cpu, _) = run_program(0xC000, &[0xD8, 0xF8]);
        assert!(cpu.p & flags::D != 0);
    }

    #[test]
    fn nop_takes_two_cycles() {
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xEA]);
        let cycles = cpu.step(&mut bus).expect("official opcode");
        assert_eq!(cycles, 2);
        assert_eq!(cpu.pc, 0xC001);
    }

    // ---- undocumented -----------------------------------------------------

    #[test]
    fn undocumented_opcodes_return_error_not_panic() {
        for opcode in [0x02u8, 0x1A, 0x9C, 0xB3, 0xEB, 0xFF] {
            let (mut cpu, mut bus) = cpu_at(0xC000, &[opcode]);
            let result = cpu.step(&mut bus);
            assert_eq!(result, Err(CpuError::UnsupportedOp(opcode)));
            // PC still advanced past the opcode byte only.
            assert_eq!(cpu.pc, 0xC001);
        }
    }

    #[test]
    fn every_official_opcode_executes_without_panicking() {
        // Structural sweep: for each official opcode, craft the operand bytes
        // the mode requires, run once, and require the documented cycle
        // count. This catches missing match arms and mis-decoded modes.
        for opcode in 0u8..=255 {
            let opinfo = info(opcode);
            if !opinfo.official {
                continue;
            }
            let pc: u16 = 0x0200;
            // Operand bytes: fill with values that stay in page 2 and point
            // at readable memory.
            let operand_len = match opinfo.mode {
                AddrMode::Imp | AddrMode::Acc => 0,
                AddrMode::Imm
                | AddrMode::Zp
                | AddrMode::Zpx
                | AddrMode::Zpy
                | AddrMode::Izx
                | AddrMode::Izy
                | AddrMode::Rel => 1,
                AddrMode::Abs | AddrMode::Abx | AddrMode::Aby | AddrMode::Ind => 2,
            };
            let mut program = vec![opcode];
            match operand_len {
                0 => {}
                1 => program.push(0x10),
                _ => program.extend_from_slice(&[0x30, 0x02]),
            }
            let (mut cpu, mut bus) = cpu_at(pc, &program);
            // Give registers sane values so indexed modes stay in page 2,
            // and keep the power-up flags so every branch is not taken.
            cpu.x = 0;
            cpu.y = 0;
            cpu.sp = 0xFD;
            // A usable zero-page pointer at $10 -> $0230.
            bus.poke(0x0010, 0x30);
            bus.poke(0x0011, 0x02);
            // Stack area reads as zero (RTI/RTS/PLA pull zeros).
            for sp in 0x0100..=0x01FF {
                bus.poke(sp, 0);
            }
            // JMP/JSR/BRK targets must be executable: point them at a NOP.
            bus.poke(0x0230, 0xEA);
            bus.poke(0xFFFE, 0x30);
            bus.poke(0xFFFF, 0x02);
            let cycles = cpu.step(&mut bus);
            // With X = Y = 0 no page is crossed, so the result must equal
            // the base cycle count, except for the four branches that are
            // taken with the power-up flag state P=$34 (N=V=C=Z=0):
            // BPL, BVC, BCC, BNE take 3 cycles with this same-page offset.
            let taken_at_powerup = matches!(
                opinfo.mnemonic,
                Mnemonic::Bpl | Mnemonic::Bvc | Mnemonic::Bcc | Mnemonic::Bne
            );
            let expected = match (opinfo.mode, taken_at_powerup) {
                (AddrMode::Rel, true) => opinfo.cycles + 1,
                _ => opinfo.cycles,
            };
            assert_eq!(
                cycles,
                Ok(expected),
                "opcode 0x{opcode:02X} ({:?}) unexpected cycle count",
                opinfo.mnemonic
            );
        }
    }
}
