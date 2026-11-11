//! 6502 CPU (2A03 core, no decimal mode) execution engine.
//!
//! The engine executes all 256 opcodes — the 151 documented ones and
//! the 105 undocumented ones — with cycle-accurate base timings,
//! including the +1 page-cross penalty for indexed reads, the 2/3/4
//! cycle branch timing, the JMP `(ind)` page-wrap bug and the zero-page
//! wrap of indexed zero-page modes. Undocumented behavior follows the
//! NESdev Wiki write-ups on unofficial opcodes; behaviors that are
//! unstable on real hardware are marked `UNVERIFIED-DESIGN` (see
//! DECISIONS.md) because the nestest calibration never executes them.
//! The JAM/KIL family halts the CPU; [`Cpu::step`] then returns
//! [`CpuError::Halted`] instead of panicking.
//!
//! The 2A03 has no decimal mode, so ADC/SBC ignore the D flag (the flag
//! exists and CLD/SED manipulate it, but arithmetic is always binary).

use core::fmt;

pub mod opcodes;

use self::opcodes::{info, AddrMode, Mnemonic, OpInfo};

/// CPU-visible memory bus.
pub trait Bus {
    /// Read one byte. Mutable because a real bus has read side effects
    /// (the open-bus latch, registers that clear on read); the trace
    /// logger uses side-effect-free peeks instead.
    fn read(&mut self, addr: u16) -> u8;
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
    /// The CPU was halted by a JAM/KIL opcode; only a reset resumes it.
    Halted,
}

impl fmt::Display for CpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CpuError::Halted => write!(
                f,
                "CPU halted by a JAM opcode; only a reset resumes execution"
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
    /// Halted by a JAM/KIL opcode (stays until the state is reset).
    pub halted: bool,
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
            halted: false,
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
            halted: self.halted,
        }
    }

    /// Fetch, decode and execute one instruction; returns the cycles used.
    ///
    /// All 256 opcodes execute; the only error is a halted CPU (JAM).
    pub fn step(&mut self, bus: &mut dyn Bus) -> Result<u8, CpuError> {
        if self.halted {
            return Err(CpuError::Halted);
        }
        let opcode = self.fetch8(bus);
        let opinfo = info(opcode);
        Ok(self.execute(bus, opinfo))
    }

    /// Service a hardware interrupt (NMI or IRQ).
    ///
    /// Seven cycles: the return address is pushed, `P` is pushed without
    /// the B flag but with the unused bit set, the I flag is raised and
    /// the PC is loaded from `vector` (NESdev Wiki, "CPU interrupts").
    pub fn service_interrupt(&mut self, bus: &mut dyn Bus, vector: u16) -> u8 {
        self.push16(bus, self.pc);
        self.push(bus, self.p & !flags::B | flags::U);
        self.p |= flags::I;
        self.pc = self.read16(bus, vector);
        7
    }

    /// Power-on / reset sequence: seven cycles, `S -= 3`, I set, PC from
    /// `$FFFC` (NESdev Wiki, "Reset": the phantom stack pushes only
    /// decrement S — nothing is written).
    pub fn reset(&mut self, bus: &mut dyn Bus) -> u8 {
        self.sp = self.sp.wrapping_sub(3);
        self.p |= flags::I;
        self.pc = self.read16(bus, 0xFFFC);
        self.halted = false;
        7
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

    fn read16(&self, bus: &mut dyn Bus, addr: u16) -> u16 {
        let lo = u16::from(bus.read(addr));
        let hi = u16::from(bus.read(addr.wrapping_add(1)));
        lo | hi << 8
    }

    /// Read a 16-bit pointer stored at a zero-page address; the high byte
    /// comes from `(zp + 1) & 0xFF`, wrapping inside page 0.
    fn read16_zp(&self, bus: &mut dyn Bus, zp: u8) -> u16 {
        let lo = u16::from(bus.read(u16::from(zp)));
        let hi = u16::from(bus.read(u16::from(zp.wrapping_add(1))));
        lo | hi << 8
    }

    /// Read the target of `JMP (ind)` with the $xxFF page-wrap bug: the high
    /// byte comes from the start of the same page, not from the next page.
    fn read16_indirect_buggy(&self, bus: &mut dyn Bus, ptr: u16) -> u16 {
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
            M::Nop => {
                // The official NOP is implied; the undocumented NOP
                // variants still consume their operand and read the
                // addressed byte (reads count for timing and the open
                // bus latch, but the value is discarded).
                if opinfo.mode == AddrMode::Imp {
                    opinfo.cycles
                } else {
                    let (addr, crossed) = self.resolve(bus, opinfo.mode);
                    let _ = bus.read(addr);
                    self.read_cycles(opinfo, crossed)
                }
            }

            // ---- undocumented opcodes (M1.4) -------------------------------
            // Behavior per the NESdev Wiki write-ups on unofficial
            // opcodes. The M1.5 nestest trace calibration verifies every
            // opcode that nestest actually executes (all families below
            // except the unstable ones, which nestest never runs).
            M::Slo | M::Rla | M::Sre | M::Rra => {
                // Read-modify-write combined with an accumulator op:
                // SLO = ASL + ORA, RLA = ROL + AND, SRE = LSR + EOR and
                // RRA = ROR + ADC, where the ADC consumes the carry the
                // ROR just produced.
                let (addr, _) = self.resolve(bus, opinfo.mode);
                let value = bus.read(addr);
                let old_carry = u8::from(self.carry());
                let (result, carry) = match opinfo.mnemonic {
                    M::Slo => (value << 1, value >> 7),
                    M::Rla => ((value << 1) | old_carry, value >> 7),
                    M::Sre => (value >> 1, value & 1),
                    M::Rra => ((value >> 1) | old_carry << 7, value & 1),
                    _ => unreachable!("rmw combine family"),
                };
                bus.write(addr, result);
                self.set_flag(flags::C, carry != 0);
                match opinfo.mnemonic {
                    M::Slo => {
                        self.a |= result;
                        self.set_nz(self.a);
                    }
                    M::Rla => {
                        self.a &= result;
                        self.set_nz(self.a);
                    }
                    M::Sre => {
                        self.a ^= result;
                        self.set_nz(self.a);
                    }
                    M::Rra => self.do_add(result),
                    _ => unreachable!("rmw combine family"),
                }
                opinfo.cycles
            }

            M::Sax => {
                // SAX stores A & X and touches no flags.
                let (addr, _) = self.resolve(bus, opinfo.mode);
                bus.write(addr, self.a & self.x);
                opinfo.cycles
            }

            M::Lax => {
                // LAX loads A and X with the same byte. The immediate
                // form (LXA) is unstable on real hardware; the simple
                // "load both" model is used and marked UNVERIFIED-DESIGN
                // in DECISIONS.md.
                let (addr, crossed) = self.resolve(bus, opinfo.mode);
                let m = bus.read(addr);
                self.a = m;
                self.x = m;
                self.set_nz(m);
                self.read_cycles(opinfo, crossed)
            }

            M::Dcp => {
                // DCP decrements memory, then compares A with the result.
                let (addr, _) = self.resolve(bus, opinfo.mode);
                let result = bus.read(addr).wrapping_sub(1);
                bus.write(addr, result);
                self.do_cmp(self.a, result);
                opinfo.cycles
            }

            M::Isc => {
                // ISC increments memory, then SBCs the result into A.
                let (addr, _) = self.resolve(bus, opinfo.mode);
                let result = bus.read(addr).wrapping_add(1);
                bus.write(addr, result);
                self.do_add(!result);
                opinfo.cycles
            }

            M::Anc | M::Alr | M::Arr | M::Sbx | M::Usbc | M::Ane => {
                // Immediate-only undocumented operations.
                let (addr, _) = self.resolve(bus, opinfo.mode);
                let imm = bus.read(addr);
                match opinfo.mnemonic {
                    // ANC = AND, then C copies N (bit 7 of A).
                    M::Anc => {
                        self.a &= imm;
                        self.set_nz(self.a);
                        self.set_flag(flags::C, self.a & flags::N != 0);
                    }
                    // ALR = AND, then LSR A.
                    M::Alr => {
                        self.a &= imm;
                        self.set_flag(flags::C, self.a & 1 != 0);
                        self.a >>= 1;
                        self.set_nz(self.a);
                    }
                    // ARR = AND, then rotate right through C with
                    // V = bit6 ^ bit7 of the result.
                    // UNVERIFIED-DESIGN: nestest never executes ARR; the
                    // flag formula follows the public 6502 write-ups.
                    M::Arr => {
                        self.a &= imm;
                        let bit0 = self.a & 1;
                        let old_carry = u8::from(self.carry());
                        self.a = (self.a >> 1) | old_carry << 7;
                        self.set_flag(flags::C, bit0 != 0);
                        let overflow = (self.a & 0x40) ^ (self.a & flags::N) != 0;
                        self.set_flag(flags::V, overflow);
                        self.set_nz(self.a);
                    }
                    // SBX subtracts the immediate from A & X into X.
                    M::Sbx => {
                        let t = self.a & self.x;
                        self.set_flag(flags::C, t >= imm);
                        self.x = t.wrapping_sub(imm);
                        self.set_nz(self.x);
                    }
                    // USBC is a plain SBC immediate.
                    M::Usbc => self.do_add(!imm),
                    // ANE is unstable; the simple A & X & imm model is
                    // used. UNVERIFIED-DESIGN (magic-constant variants).
                    M::Ane => {
                        self.a &= self.x & imm;
                        self.set_nz(self.a);
                    }
                    _ => unreachable!("immediate undocumented family"),
                }
                opinfo.cycles
            }

            M::Sha | M::Shx | M::Shy | M::Tas => {
                // Unstable stores AND-ed with the high address byte:
                // SHA stores A & X, SHX/SHY store the index, TAS first
                // copies A & X into S.
                // UNVERIFIED-DESIGN: formulas from public write-ups;
                // nestest never executes them, and the page-cross
                // corruption variants are simplified away (documented in
                // DECISIONS.md).
                let (addr, _) = self.resolve(bus, opinfo.mode);
                let and_mask = ((addr >> 8) as u8).wrapping_add(1);
                match opinfo.mnemonic {
                    M::Sha | M::Tas => {
                        let t = self.a & self.x;
                        if opinfo.mnemonic == M::Tas {
                            self.sp = t;
                        }
                        bus.write(addr, t & and_mask);
                    }
                    M::Shx => {
                        bus.write(addr, self.x & and_mask);
                    }
                    M::Shy => {
                        bus.write(addr, self.y & and_mask);
                    }
                    _ => unreachable!("unstable store family"),
                }
                opinfo.cycles
            }

            M::Las => {
                // LAS loads A, X and S with memory AND S.
                // UNVERIFIED-DESIGN: per public write-ups; nestest never
                // executes it.
                let (addr, crossed) = self.resolve(bus, opinfo.mode);
                let m = bus.read(addr);
                self.sp &= m;
                self.a = self.sp;
                self.x = self.sp;
                self.set_nz(self.a);
                self.read_cycles(opinfo, crossed)
            }

            M::Kil => {
                // JAM locks the CPU until a reset. Two cycles is the
                // nominal count (nothing runs afterwards).
                self.halted = true;
                opinfo.cycles
            }
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
        fn read(&mut self, addr: u16) -> u8 {
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
    fn jam_opcodes_halt_instead_of_panicking() {
        for opcode in [
            0x02u8, 0x12, 0x22, 0x32, 0x42, 0x52, 0x62, 0x72, 0x92, 0xB2, 0xD2, 0xF2,
        ] {
            let (mut cpu, mut bus) = cpu_at(0xC000, &[opcode]);
            // The JAM itself executes (nominal 2 cycles) and halts.
            assert_eq!(cpu.step(&mut bus), Ok(2), "opcode {opcode:02X}");
            assert!(cpu.halted);
            assert_eq!(cpu.step(&mut bus), Err(CpuError::Halted));
        }
    }

    // ---- unofficial opcode semantics (M1.4) --------------------------------

    #[test]
    fn slo_shifts_memory_and_ors_into_a() {
        // SLO zp ($07): memory <<= 1 with C = old bit7, A |= memory.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA9, 0x01, 0x07, 0x10]);
        bus.poke(0x0010, 0x81);
        cpu.step(&mut bus).expect("LDA");
        let cycles = cpu.step(&mut bus).expect("SLO");
        assert_eq!(cycles, 5);
        assert_eq!(bus.peek(0x0010), 0x02);
        assert_eq!(cpu.a, 0x03);
        assert!(cpu.p & C != 0);
        assert!(cpu.p & N == 0);
    }

    #[test]
    fn rla_rolls_memory_and_ands_into_a() {
        // RLA zp ($27): memory = ROL(memory), A &= memory.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x38, 0xA9, 0xF0, 0x27, 0x11]);
        bus.poke(0x0011, 0xB5);
        cpu.step(&mut bus).expect("SEC");
        cpu.step(&mut bus).expect("LDA");
        let cycles = cpu.step(&mut bus).expect("RLA");
        assert_eq!(cycles, 5);
        // 0xB5 rolled left with C=1 gives 0x6B; new C = 1 (old bit7).
        assert_eq!(bus.peek(0x0011), 0x6B);
        assert_eq!(cpu.a, 0xF0 & 0x6B);
        assert!(cpu.p & C != 0);
        assert!(cpu.p & N == 0);
    }

    #[test]
    fn sre_shifts_memory_and_xors_into_a() {
        // SRE zp ($47): memory >>= 1 with C = old bit0, A ^= memory.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA9, 0x55, 0x47, 0x12]);
        bus.poke(0x0012, 0x07);
        cpu.step(&mut bus).expect("LDA");
        let cycles = cpu.step(&mut bus).expect("SRE");
        assert_eq!(cycles, 5);
        assert_eq!(bus.peek(0x0012), 0x03);
        assert!(cpu.p & C != 0);
        assert_eq!(cpu.a, 0x55 ^ 0x03);
    }

    #[test]
    fn rra_uses_the_ror_carry_in_the_adc() {
        // RRA zp ($67) with C=0: ROR(0xFF) = 0x7F with the new C = 1;
        // ADC then computes 0x01 + 0x7F + 1 = 0x81.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x18, 0xA9, 0x01, 0x67, 0x20]);
        bus.poke(0x0020, 0xFF);
        cpu.step(&mut bus).expect("CLC");
        cpu.step(&mut bus).expect("LDA");
        let cycles = cpu.step(&mut bus).expect("RRA");
        assert_eq!(cycles, 5);
        assert_eq!(bus.peek(0x0020), 0x7F);
        assert_eq!(cpu.a, 0x81);
        // 0x81 fits in the byte: no carry out; N set; V set (+ + -).
        assert!(cpu.p & C == 0);
        assert!(cpu.p & N != 0);
        assert!(cpu.p & V != 0);
    }

    #[test]
    fn sax_stores_a_and_x() {
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA9, 0x3C, 0xA2, 0x0F, 0x87, 0x30]);
        for _ in 0..2 {
            cpu.step(&mut bus).expect("setup");
        }
        let cycles = cpu.step(&mut bus).expect("SAX");
        assert_eq!(cycles, 3);
        assert_eq!(bus.peek(0x0030), 0x3C & 0x0F);
    }

    #[test]
    fn lax_loads_a_and_x() {
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA7, 0x40]);
        bus.poke(0x0040, 0xC2);
        let cycles = cpu.step(&mut bus).expect("LAX");
        assert_eq!(cycles, 3);
        assert_eq!(cpu.a, 0xC2);
        assert_eq!(cpu.x, 0xC2);
        assert!(cpu.p & N != 0);
    }

    #[test]
    fn dcp_decrements_and_compares() {
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA9, 0x10, 0xC7, 0x50]);
        bus.poke(0x0050, 0x05);
        cpu.step(&mut bus).expect("LDA");
        let cycles = cpu.step(&mut bus).expect("DCP");
        assert_eq!(cycles, 5);
        assert_eq!(bus.peek(0x0050), 0x04);
        // A = 0x10 >= 0x04: carry set, result 0x0C (N clear, Z clear).
        assert!(cpu.p & C != 0);
        assert!(cpu.p & N == 0);
        assert!(cpu.p & Z == 0);
    }

    #[test]
    fn isc_increments_and_sbc() {
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x38, 0xA9, 0x00, 0xE7, 0x60]);
        bus.poke(0x0060, 0x00);
        cpu.step(&mut bus).expect("SEC");
        cpu.step(&mut bus).expect("LDA");
        let cycles = cpu.step(&mut bus).expect("ISC");
        assert_eq!(cycles, 5);
        assert_eq!(bus.peek(0x0060), 0x01);
        // 0x00 - 0x01 - 0: borrow out, result 0xFF.
        assert_eq!(cpu.a, 0xFF);
        assert!(cpu.p & C == 0);
        assert!(cpu.p & N != 0);
    }

    #[test]
    fn undocumented_nops_consume_operands() {
        // NOP zp ($04) is 3 cycles and reads the byte.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x04, 0x10]);
        assert_eq!(cpu.step(&mut bus), Ok(3));
        assert_eq!(cpu.pc, 0xC002);

        // NOP abs,X ($1C): 4 cycles, +1 when a page is crossed.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x1C, 0xF0, 0x02]);
        cpu.x = 0x20;
        assert_eq!(cpu.step(&mut bus), Ok(5));
        assert_eq!(cpu.pc, 0xC003);

        // Implied undocumented NOPs stay 2 cycles.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x1A]);
        assert_eq!(cpu.step(&mut bus), Ok(2));
    }

    #[test]
    fn anc_alr_and_sbx_semantics() {
        // ANC imm ($0B): A &= imm; C = N.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA9, 0x0F, 0x0B, 0xF0]);
        cpu.step(&mut bus).expect("LDA");
        assert_eq!(cpu.step(&mut bus), Ok(2));
        assert_eq!(cpu.a, 0x00);
        assert!(cpu.p & C == 0);
        assert!(cpu.p & Z != 0);

        // ALR imm ($4B): A &= imm then LSR.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA9, 0x0F, 0x4B, 0x0F]);
        cpu.step(&mut bus).expect("LDA");
        assert_eq!(cpu.step(&mut bus), Ok(2));
        assert_eq!(cpu.a, 0x07);
        assert!(cpu.p & C != 0);

        // SBX imm ($CB): X = (A & X) - imm, carry from the comparison.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA9, 0xF0, 0xA2, 0x0F, 0xCB, 0x0A]);
        cpu.step(&mut bus).expect("LDA");
        cpu.step(&mut bus).expect("LDX");
        assert_eq!(cpu.step(&mut bus), Ok(2));
        assert_eq!(cpu.x, 0xF6);
        assert!(cpu.p & C == 0);
        assert!(cpu.p & N != 0);
    }

    #[test]
    fn usbc_is_sbc_immediate() {
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x38, 0xA9, 0x05, 0xEB, 0x03]);
        cpu.step(&mut bus).expect("SEC");
        cpu.step(&mut bus).expect("LDA");
        assert_eq!(cpu.step(&mut bus), Ok(2));
        assert_eq!(cpu.a, 0x02);
        assert!(cpu.p & C != 0);
    }

    #[test]
    fn unstable_store_family_writes_the_anded_value() {
        // SHY abs,X ($9C): stores Y & (high byte + 1).
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA0, 0x3B, 0x9C, 0x00, 0x04]);
        cpu.step(&mut bus).expect("LDY");
        assert_eq!(cpu.step(&mut bus), Ok(5));
        // Mask 0x04 + 1 = 0x05; 0x3B & 0x05 = 0x01.
        assert_eq!(bus.peek(0x0400), 0x01);

        // SHX abs,Y ($9E): stores X & (high byte + 1).
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xA2, 0x6E, 0x9E, 0x00, 0x03]);
        cpu.step(&mut bus).expect("LDX");
        assert_eq!(cpu.step(&mut bus), Ok(5));
        // Mask 0x03 + 1 = 0x04; 0x6E & 0x04 = 0x04.
        assert_eq!(bus.peek(0x0300), 0x04);
    }

    #[test]
    fn tas_and_las_model_the_documented_formulas() {
        // TAS abs,Y ($9B): S = A & X; memory = S & (high byte + 1).
        let (mut cpu, mut bus) = cpu_at(
            0xC000,
            &[0xA9, 0x37, 0xA2, 0x1F, 0xA0, 0x00, 0x9B, 0x00, 0x06],
        );
        for _ in 0..3 {
            cpu.step(&mut bus).expect("setup");
        }
        assert_eq!(cpu.step(&mut bus), Ok(5));
        assert_eq!(cpu.sp, 0x17);
        assert_eq!(bus.peek(0x0600), 0x07);

        // LAS abs,Y ($BB): A = X = S = memory & S.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xBB, 0x00, 0x05]);
        bus.poke(0x0500, 0xFF);
        assert_eq!(cpu.step(&mut bus), Ok(4));
        assert_eq!(cpu.a, 0xFD);
        assert_eq!(cpu.x, 0xFD);
        assert_eq!(cpu.sp, 0xFD);
    }

    // ---- interrupts (M1.3) ---------------------------------------------------

    #[test]
    fn nmi_sequence_pushes_and_jumps_to_vector() {
        let (mut cpu, mut bus) = cpu_at(0xC123, &[0xEA]);
        bus.poke(0xFFFA, 0x00);
        bus.poke(0xFFFB, 0xC0);
        let cycles = cpu.service_interrupt(&mut bus, 0xFFFA);
        assert_eq!(cycles, 7);
        assert_eq!(cpu.pc, 0xC000);
        assert!(cpu.p & I != 0);
        assert_eq!(cpu.sp, 0xFA);
        // Return address on the stack, high byte first.
        assert_eq!(bus.peek(0x01FD), 0xC1);
        assert_eq!(bus.peek(0x01FC), 0x23);
        // P without the B flag, with the unused bit set.
        assert_eq!(bus.peek(0x01FB), 0x24);
    }

    #[test]
    fn irq_sequence_uses_vector_fffe() {
        let (mut cpu, mut bus) = cpu_at(0x8005, &[0xEA]);
        bus.poke(0xFFFE, 0x00);
        bus.poke(0xFFFF, 0x90);
        let cycles = cpu.service_interrupt(&mut bus, 0xFFFE);
        assert_eq!(cycles, 7);
        assert_eq!(cpu.pc, 0x9000);
        assert!(cpu.p & I != 0);
    }

    #[test]
    fn reset_decrements_sp_by_three_and_sets_i() {
        let (mut cpu, mut bus) = cpu_at(0x0000, &[]);
        bus.poke(0xFFFC, 0x00);
        bus.poke(0xFFFD, 0xC0);
        let cycles = cpu.reset(&mut bus);
        assert_eq!(cycles, 7);
        assert_eq!(cpu.pc, 0xC000);
        assert_eq!(cpu.sp, 0xFA);
        assert!(cpu.p & I != 0);
    }

    #[test]
    fn rti_restores_the_interrupted_state() {
        let (mut cpu, mut bus) = cpu_at(0xC123, &[0xEA]);
        bus.poke(0xFFFA, 0x00);
        bus.poke(0xFFFB, 0xC0);
        bus.poke(0xC000, 0x40); // RTI at the NMI target.
        cpu.service_interrupt(&mut bus, 0xFFFA);
        assert_eq!(cpu.step(&mut bus), Ok(6));
        assert_eq!(cpu.pc, 0xC123);
        assert_eq!(cpu.sp, 0xFD);
    }

    #[test]
    fn brk_pushes_b_set_service_interrupt_pushes_b_clear() {
        // BRK pushes P with the B flag set; hardware interrupts push it
        // clear. Both set the unused bit.
        let (mut cpu, mut bus) = cpu_at(0xC000, &[0x00, 0xEA]);
        bus.poke(0xFFFE, 0x50);
        bus.poke(0xFFFF, 0xC0);
        cpu.p = 0x24;
        let cycles = cpu.step(&mut bus).expect("BRK");
        assert_eq!(cycles, 7);
        assert_eq!(bus.peek(0x01FB), 0x34);

        let (mut cpu, mut bus) = cpu_at(0xC000, &[0xEA]);
        cpu.p = 0x24;
        cpu.service_interrupt(&mut bus, 0xFFFE);
        assert_eq!(bus.peek(0x01FB), 0x24);
    }

    #[test]
    fn every_opcode_executes_with_documented_cycles() {
        // Structural sweep: for each of the 256 opcodes, craft the operand
        // bytes the mode requires, run once, and require the documented
        // cycle count. This catches missing match arms and mis-decoded
        // modes. JAM opcodes halt instead and are covered above.
        for opcode in 0u8..=255 {
            let opinfo = info(opcode);
            if opinfo.mnemonic == Mnemonic::Kil {
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
