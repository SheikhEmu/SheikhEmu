//! The complete 6502 opcode table (all 256 opcodes).
//!
//! Every entry carries its addressing mode, base cycle count, whether an
//! extra cycle applies when a page is crossed (read instructions only) and
//! whether the opcode is one of the 151 documented opcodes. The 105
//! undocumented opcodes are named after their widely used community names
//! (KIL, SLO, RLA, ...) and are *not* executed by the core in M0; the CPU
//! returns an `UnsupportedOp` error for them instead of panicking.
//!
//! Reference: public 6502 instruction set documentation (NESdev Wiki
//! "CPU unimplemented instructions" and "CPU memory addressing").

/// Addressing mode of an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AddrMode {
    /// 1 byte, operand unused.
    Imp,
    /// 1 byte, operates on the accumulator.
    Acc,
    /// 2 bytes, immediate operand.
    Imm,
    /// 2 bytes, zero page address.
    Zp,
    /// 2 bytes, zero page address + X (wraps inside page 0).
    Zpx,
    /// 2 bytes, zero page address + Y (wraps inside page 0).
    Zpy,
    /// 3 bytes, absolute address.
    Abs,
    /// 3 bytes, absolute address + X.
    Abx,
    /// 3 bytes, absolute address + Y.
    Aby,
    /// 3 bytes, indirect pointer (JMP only, with the $xxFF page-wrap bug).
    Ind,
    /// 2 bytes, zero page pointer indexed by X (wraps inside page 0).
    Izx,
    /// 2 bytes, zero page pointer dereferenced then indexed by Y.
    Izy,
    /// 2 bytes, relative branch offset.
    Rel,
}

/// Instruction mnemonic. Official mnemonics cover the 151 documented
/// opcodes; the extra variants describe the undocumented ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mnemonic {
    // Loads, stores, transfers.
    Lda,
    Ldx,
    Ldy,
    Sta,
    Stx,
    Sty,
    Tax,
    Tay,
    Txa,
    Tya,
    Tsx,
    Txs,
    // Arithmetic and logic.
    Adc,
    Sbc,
    And,
    Ora,
    Eor,
    Cmp,
    Cpx,
    Cpy,
    Bit,
    // Shifts, rotates, inc/dec.
    Asl,
    Lsr,
    Rol,
    Ror,
    Inc,
    Dec,
    Inx,
    Iny,
    Dex,
    Dey,
    // Control flow.
    Jmp,
    Jsr,
    Rts,
    Brk,
    Rti,
    // Branches.
    Bcc,
    Bcs,
    Beq,
    Bmi,
    Bne,
    Bpl,
    Bvc,
    Bvs,
    // Stack.
    Pha,
    Php,
    Pla,
    Plp,
    // Flag operations and NOP.
    Clc,
    Cld,
    Cli,
    Clv,
    Sec,
    Sed,
    Sei,
    Nop,
    // Undocumented mnemonics (never executed in M0).
    Kil,
    Slo,
    Rla,
    Sre,
    Rra,
    Sax,
    Lax,
    Sha,
    Shx,
    Shy,
    Tas,
    Las,
    Dcp,
    Isc,
    Anc,
    Alr,
    Arr,
    Sbx,
    Usbc,
    Ane,
}

impl Mnemonic {
    /// Uppercase community spelling of this mnemonic.
    pub const fn name(self) -> &'static str {
        match self {
            Mnemonic::Lda => "LDA",
            Mnemonic::Ldx => "LDX",
            Mnemonic::Ldy => "LDY",
            Mnemonic::Sta => "STA",
            Mnemonic::Stx => "STX",
            Mnemonic::Sty => "STY",
            Mnemonic::Tax => "TAX",
            Mnemonic::Tay => "TAY",
            Mnemonic::Txa => "TXA",
            Mnemonic::Tya => "TYA",
            Mnemonic::Tsx => "TSX",
            Mnemonic::Txs => "TXS",
            Mnemonic::Adc => "ADC",
            Mnemonic::Sbc => "SBC",
            Mnemonic::And => "AND",
            Mnemonic::Ora => "ORA",
            Mnemonic::Eor => "EOR",
            Mnemonic::Cmp => "CMP",
            Mnemonic::Cpx => "CPX",
            Mnemonic::Cpy => "CPY",
            Mnemonic::Bit => "BIT",
            Mnemonic::Asl => "ASL",
            Mnemonic::Lsr => "LSR",
            Mnemonic::Rol => "ROL",
            Mnemonic::Ror => "ROR",
            Mnemonic::Inc => "INC",
            Mnemonic::Dec => "DEC",
            Mnemonic::Inx => "INX",
            Mnemonic::Iny => "INY",
            Mnemonic::Dex => "DEX",
            Mnemonic::Dey => "DEY",
            Mnemonic::Jmp => "JMP",
            Mnemonic::Jsr => "JSR",
            Mnemonic::Rts => "RTS",
            Mnemonic::Brk => "BRK",
            Mnemonic::Rti => "RTI",
            Mnemonic::Bcc => "BCC",
            Mnemonic::Bcs => "BCS",
            Mnemonic::Beq => "BEQ",
            Mnemonic::Bmi => "BMI",
            Mnemonic::Bne => "BNE",
            Mnemonic::Bpl => "BPL",
            Mnemonic::Bvc => "BVC",
            Mnemonic::Bvs => "BVS",
            Mnemonic::Pha => "PHA",
            Mnemonic::Php => "PHP",
            Mnemonic::Pla => "PLA",
            Mnemonic::Plp => "PLP",
            Mnemonic::Clc => "CLC",
            Mnemonic::Cld => "CLD",
            Mnemonic::Cli => "CLI",
            Mnemonic::Clv => "CLV",
            Mnemonic::Sec => "SEC",
            Mnemonic::Sed => "SED",
            Mnemonic::Sei => "SEI",
            Mnemonic::Nop => "NOP",
            Mnemonic::Kil => "KIL",
            Mnemonic::Slo => "SLO",
            Mnemonic::Rla => "RLA",
            Mnemonic::Sre => "SRE",
            Mnemonic::Rra => "RRA",
            Mnemonic::Sax => "SAX",
            Mnemonic::Lax => "LAX",
            Mnemonic::Sha => "SHA",
            Mnemonic::Shx => "SHX",
            Mnemonic::Shy => "SHY",
            Mnemonic::Tas => "TAS",
            Mnemonic::Las => "LAS",
            Mnemonic::Dcp => "DCP",
            Mnemonic::Isc => "ISC",
            Mnemonic::Anc => "ANC",
            Mnemonic::Alr => "ALR",
            Mnemonic::Arr => "ARR",
            Mnemonic::Sbx => "SBX",
            Mnemonic::Usbc => "USBC",
            Mnemonic::Ane => "ANE",
        }
    }
}

/// Static description of one opcode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpInfo {
    /// Instruction mnemonic.
    pub mnemonic: Mnemonic,
    /// Addressing mode.
    pub mode: AddrMode,
    /// Base cycle count.
    pub cycles: u8,
    /// Whether a page crossing adds one cycle (indexed reads only).
    pub page_cross: bool,
    /// Whether the opcode is a documented (official) one.
    pub official: bool,
}

const fn op(
    mnemonic: Mnemonic,
    mode: AddrMode,
    cycles: u8,
    page_cross: bool,
    official: bool,
) -> OpInfo {
    OpInfo {
        mnemonic,
        mode,
        cycles,
        page_cross,
        official,
    }
}

/// The full 256-entry opcode table, indexed by opcode byte.
#[rustfmt::skip]
pub const OPCODES: [OpInfo; 256] = [
    // row 0x00
    op(Mnemonic::Brk, AddrMode::Imp, 7, false, true),
    op(Mnemonic::Ora, AddrMode::Izx, 6, false, true),
    op(Mnemonic::Kil, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Slo, AddrMode::Izx, 8, false, false),
    op(Mnemonic::Nop, AddrMode::Zp, 3, false, false),
    op(Mnemonic::Ora, AddrMode::Zp, 3, false, true),
    op(Mnemonic::Asl, AddrMode::Zp, 5, false, true),
    op(Mnemonic::Slo, AddrMode::Zp, 5, false, false),
    op(Mnemonic::Php, AddrMode::Imp, 3, false, true),
    op(Mnemonic::Ora, AddrMode::Imm, 2, false, true),
    op(Mnemonic::Asl, AddrMode::Acc, 2, false, true),
    op(Mnemonic::Anc, AddrMode::Imm, 2, false, false),
    op(Mnemonic::Nop, AddrMode::Abs, 4, false, false),
    op(Mnemonic::Ora, AddrMode::Abs, 4, false, true),
    op(Mnemonic::Asl, AddrMode::Abs, 6, false, true),
    op(Mnemonic::Slo, AddrMode::Abs, 6, false, false),
    // row 0x10
    op(Mnemonic::Bpl, AddrMode::Rel, 2, false, true),
    op(Mnemonic::Ora, AddrMode::Izy, 5, true, true),
    op(Mnemonic::Kil, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Slo, AddrMode::Izy, 8, false, false),
    op(Mnemonic::Nop, AddrMode::Zpx, 4, false, false),
    op(Mnemonic::Ora, AddrMode::Zpx, 4, false, true),
    op(Mnemonic::Asl, AddrMode::Zpx, 6, false, true),
    op(Mnemonic::Slo, AddrMode::Zpx, 6, false, false),
    op(Mnemonic::Clc, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Ora, AddrMode::Aby, 4, true, true),
    op(Mnemonic::Nop, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Slo, AddrMode::Aby, 7, false, false),
    op(Mnemonic::Nop, AddrMode::Abx, 4, true, false),
    op(Mnemonic::Ora, AddrMode::Abx, 4, true, true),
    op(Mnemonic::Asl, AddrMode::Abx, 7, false, true),
    op(Mnemonic::Slo, AddrMode::Abx, 7, false, false),
    // row 0x20
    op(Mnemonic::Jsr, AddrMode::Abs, 6, false, true),
    op(Mnemonic::And, AddrMode::Izx, 6, false, true),
    op(Mnemonic::Kil, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Rla, AddrMode::Izx, 8, false, false),
    op(Mnemonic::Bit, AddrMode::Zp, 3, false, true),
    op(Mnemonic::And, AddrMode::Zp, 3, false, true),
    op(Mnemonic::Rol, AddrMode::Zp, 5, false, true),
    op(Mnemonic::Rla, AddrMode::Zp, 5, false, false),
    op(Mnemonic::Plp, AddrMode::Imp, 4, false, true),
    op(Mnemonic::And, AddrMode::Imm, 2, false, true),
    op(Mnemonic::Rol, AddrMode::Acc, 2, false, true),
    op(Mnemonic::Anc, AddrMode::Imm, 2, false, false),
    op(Mnemonic::Bit, AddrMode::Abs, 4, false, true),
    op(Mnemonic::And, AddrMode::Abs, 4, false, true),
    op(Mnemonic::Rol, AddrMode::Abs, 6, false, true),
    op(Mnemonic::Rla, AddrMode::Abs, 6, false, false),
    // row 0x30
    op(Mnemonic::Bmi, AddrMode::Rel, 2, false, true),
    op(Mnemonic::And, AddrMode::Izy, 5, true, true),
    op(Mnemonic::Kil, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Rla, AddrMode::Izy, 8, false, false),
    op(Mnemonic::Nop, AddrMode::Zpx, 4, false, false),
    op(Mnemonic::And, AddrMode::Zpx, 4, false, true),
    op(Mnemonic::Rol, AddrMode::Zpx, 6, false, true),
    op(Mnemonic::Rla, AddrMode::Zpx, 6, false, false),
    op(Mnemonic::Sec, AddrMode::Imp, 2, false, true),
    op(Mnemonic::And, AddrMode::Aby, 4, true, true),
    op(Mnemonic::Nop, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Rla, AddrMode::Aby, 7, false, false),
    op(Mnemonic::Nop, AddrMode::Abx, 4, true, false),
    op(Mnemonic::And, AddrMode::Abx, 4, true, true),
    op(Mnemonic::Rol, AddrMode::Abx, 7, false, true),
    op(Mnemonic::Rla, AddrMode::Abx, 7, false, false),
    // row 0x40
    op(Mnemonic::Rti, AddrMode::Imp, 6, false, true),
    op(Mnemonic::Eor, AddrMode::Izx, 6, false, true),
    op(Mnemonic::Kil, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Sre, AddrMode::Izx, 8, false, false),
    op(Mnemonic::Nop, AddrMode::Zp, 3, false, false),
    op(Mnemonic::Eor, AddrMode::Zp, 3, false, true),
    op(Mnemonic::Lsr, AddrMode::Zp, 5, false, true),
    op(Mnemonic::Sre, AddrMode::Zp, 5, false, false),
    op(Mnemonic::Pha, AddrMode::Imp, 3, false, true),
    op(Mnemonic::Eor, AddrMode::Imm, 2, false, true),
    op(Mnemonic::Lsr, AddrMode::Acc, 2, false, true),
    op(Mnemonic::Alr, AddrMode::Imm, 2, false, false),
    op(Mnemonic::Jmp, AddrMode::Abs, 3, false, true),
    op(Mnemonic::Eor, AddrMode::Abs, 4, false, true),
    op(Mnemonic::Lsr, AddrMode::Abs, 6, false, true),
    op(Mnemonic::Sre, AddrMode::Abs, 6, false, false),
    // row 0x50
    op(Mnemonic::Bvc, AddrMode::Rel, 2, false, true),
    op(Mnemonic::Eor, AddrMode::Izy, 5, true, true),
    op(Mnemonic::Kil, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Sre, AddrMode::Izy, 8, false, false),
    op(Mnemonic::Nop, AddrMode::Zpx, 4, false, false),
    op(Mnemonic::Eor, AddrMode::Zpx, 4, false, true),
    op(Mnemonic::Lsr, AddrMode::Zpx, 6, false, true),
    op(Mnemonic::Sre, AddrMode::Zpx, 6, false, false),
    op(Mnemonic::Cli, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Eor, AddrMode::Aby, 4, true, true),
    op(Mnemonic::Nop, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Sre, AddrMode::Aby, 7, false, false),
    op(Mnemonic::Nop, AddrMode::Abx, 4, true, false),
    op(Mnemonic::Eor, AddrMode::Abx, 4, true, true),
    op(Mnemonic::Lsr, AddrMode::Abx, 7, false, true),
    op(Mnemonic::Sre, AddrMode::Abx, 7, false, false),
    // row 0x60
    op(Mnemonic::Rts, AddrMode::Imp, 6, false, true),
    op(Mnemonic::Adc, AddrMode::Izx, 6, false, true),
    op(Mnemonic::Kil, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Rra, AddrMode::Izx, 8, false, false),
    op(Mnemonic::Nop, AddrMode::Zp, 3, false, false),
    op(Mnemonic::Adc, AddrMode::Zp, 3, false, true),
    op(Mnemonic::Ror, AddrMode::Zp, 5, false, true),
    op(Mnemonic::Rra, AddrMode::Zp, 5, false, false),
    op(Mnemonic::Pla, AddrMode::Imp, 4, false, true),
    op(Mnemonic::Adc, AddrMode::Imm, 2, false, true),
    op(Mnemonic::Ror, AddrMode::Acc, 2, false, true),
    op(Mnemonic::Arr, AddrMode::Imm, 2, false, false),
    op(Mnemonic::Jmp, AddrMode::Ind, 5, false, true),
    op(Mnemonic::Adc, AddrMode::Abs, 4, false, true),
    op(Mnemonic::Ror, AddrMode::Abs, 6, false, true),
    op(Mnemonic::Rra, AddrMode::Abs, 6, false, false),
    // row 0x70
    op(Mnemonic::Bvs, AddrMode::Rel, 2, false, true),
    op(Mnemonic::Adc, AddrMode::Izy, 5, true, true),
    op(Mnemonic::Kil, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Rra, AddrMode::Izy, 8, false, false),
    op(Mnemonic::Nop, AddrMode::Zpx, 4, false, false),
    op(Mnemonic::Adc, AddrMode::Zpx, 4, false, true),
    op(Mnemonic::Ror, AddrMode::Zpx, 6, false, true),
    op(Mnemonic::Rra, AddrMode::Zpx, 6, false, false),
    op(Mnemonic::Sei, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Adc, AddrMode::Aby, 4, true, true),
    op(Mnemonic::Nop, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Rra, AddrMode::Aby, 7, false, false),
    op(Mnemonic::Nop, AddrMode::Abx, 4, true, false),
    op(Mnemonic::Adc, AddrMode::Abx, 4, true, true),
    op(Mnemonic::Ror, AddrMode::Abx, 7, false, true),
    op(Mnemonic::Rra, AddrMode::Abx, 7, false, false),
    // row 0x80
    op(Mnemonic::Nop, AddrMode::Imm, 2, false, false),
    op(Mnemonic::Sta, AddrMode::Izx, 6, false, true),
    op(Mnemonic::Nop, AddrMode::Imm, 2, false, false),
    op(Mnemonic::Sax, AddrMode::Izx, 6, false, false),
    op(Mnemonic::Sty, AddrMode::Zp, 3, false, true),
    op(Mnemonic::Sta, AddrMode::Zp, 3, false, true),
    op(Mnemonic::Stx, AddrMode::Zp, 3, false, true),
    op(Mnemonic::Sax, AddrMode::Zp, 3, false, false),
    op(Mnemonic::Dey, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Nop, AddrMode::Imm, 2, false, false),
    op(Mnemonic::Txa, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Ane, AddrMode::Imm, 2, false, false),
    op(Mnemonic::Sty, AddrMode::Abs, 4, false, true),
    op(Mnemonic::Sta, AddrMode::Abs, 4, false, true),
    op(Mnemonic::Stx, AddrMode::Abs, 4, false, true),
    op(Mnemonic::Sax, AddrMode::Abs, 4, false, false),
    // row 0x90
    op(Mnemonic::Bcc, AddrMode::Rel, 2, false, true),
    op(Mnemonic::Sta, AddrMode::Izy, 6, false, true),
    op(Mnemonic::Kil, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Sha, AddrMode::Izy, 5, false, false),
    op(Mnemonic::Sty, AddrMode::Zpx, 4, false, true),
    op(Mnemonic::Sta, AddrMode::Zpx, 4, false, true),
    op(Mnemonic::Stx, AddrMode::Zpy, 4, false, true),
    op(Mnemonic::Sax, AddrMode::Zpy, 4, false, false),
    op(Mnemonic::Tya, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Sta, AddrMode::Aby, 5, false, true),
    op(Mnemonic::Txs, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Tas, AddrMode::Aby, 5, false, false),
    op(Mnemonic::Shy, AddrMode::Abx, 5, false, false),
    op(Mnemonic::Sta, AddrMode::Abx, 5, false, true),
    op(Mnemonic::Shx, AddrMode::Aby, 5, false, false),
    op(Mnemonic::Sha, AddrMode::Aby, 5, false, false),
    // row 0xA0
    op(Mnemonic::Ldy, AddrMode::Imm, 2, false, true),
    op(Mnemonic::Lda, AddrMode::Izx, 6, false, true),
    op(Mnemonic::Ldx, AddrMode::Imm, 2, false, true),
    op(Mnemonic::Lax, AddrMode::Izx, 6, false, false),
    op(Mnemonic::Ldy, AddrMode::Zp, 3, false, true),
    op(Mnemonic::Lda, AddrMode::Zp, 3, false, true),
    op(Mnemonic::Ldx, AddrMode::Zp, 3, false, true),
    op(Mnemonic::Lax, AddrMode::Zp, 3, false, false),
    op(Mnemonic::Tay, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Lda, AddrMode::Imm, 2, false, true),
    op(Mnemonic::Tax, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Lax, AddrMode::Imm, 2, false, false),
    op(Mnemonic::Ldy, AddrMode::Abs, 4, false, true),
    op(Mnemonic::Lda, AddrMode::Abs, 4, false, true),
    op(Mnemonic::Ldx, AddrMode::Abs, 4, false, true),
    op(Mnemonic::Lax, AddrMode::Abs, 4, false, false),
    // row 0xB0
    op(Mnemonic::Bcs, AddrMode::Rel, 2, false, true),
    op(Mnemonic::Lda, AddrMode::Izy, 5, true, true),
    op(Mnemonic::Kil, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Lax, AddrMode::Izy, 5, true, false),
    op(Mnemonic::Ldy, AddrMode::Zpx, 4, false, true),
    op(Mnemonic::Lda, AddrMode::Zpx, 4, false, true),
    op(Mnemonic::Ldx, AddrMode::Zpy, 4, false, true),
    op(Mnemonic::Lax, AddrMode::Zpy, 4, false, false),
    op(Mnemonic::Clv, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Lda, AddrMode::Aby, 4, true, true),
    op(Mnemonic::Tsx, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Las, AddrMode::Aby, 4, true, false),
    op(Mnemonic::Ldy, AddrMode::Abx, 4, true, true),
    op(Mnemonic::Lda, AddrMode::Abx, 4, true, true),
    op(Mnemonic::Ldx, AddrMode::Aby, 4, true, true),
    op(Mnemonic::Lax, AddrMode::Aby, 4, true, false),
    // row 0xC0
    op(Mnemonic::Cpy, AddrMode::Imm, 2, false, true),
    op(Mnemonic::Cmp, AddrMode::Izx, 6, false, true),
    op(Mnemonic::Nop, AddrMode::Imm, 2, false, false),
    op(Mnemonic::Dcp, AddrMode::Izx, 8, false, false),
    op(Mnemonic::Cpy, AddrMode::Zp, 3, false, true),
    op(Mnemonic::Cmp, AddrMode::Zp, 3, false, true),
    op(Mnemonic::Dec, AddrMode::Zp, 5, false, true),
    op(Mnemonic::Dcp, AddrMode::Zp, 5, false, false),
    op(Mnemonic::Iny, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Cmp, AddrMode::Imm, 2, false, true),
    op(Mnemonic::Dex, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Sbx, AddrMode::Imm, 2, false, false),
    op(Mnemonic::Cpy, AddrMode::Abs, 4, false, true),
    op(Mnemonic::Cmp, AddrMode::Abs, 4, false, true),
    op(Mnemonic::Dec, AddrMode::Abs, 6, false, true),
    op(Mnemonic::Dcp, AddrMode::Abs, 6, false, false),
    // row 0xD0
    op(Mnemonic::Bne, AddrMode::Rel, 2, false, true),
    op(Mnemonic::Cmp, AddrMode::Izy, 5, true, true),
    op(Mnemonic::Kil, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Dcp, AddrMode::Izy, 8, false, false),
    op(Mnemonic::Nop, AddrMode::Zpx, 4, false, false),
    op(Mnemonic::Cmp, AddrMode::Zpx, 4, false, true),
    op(Mnemonic::Dec, AddrMode::Zpx, 6, false, true),
    op(Mnemonic::Dcp, AddrMode::Zpx, 6, false, false),
    op(Mnemonic::Cld, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Cmp, AddrMode::Aby, 4, true, true),
    op(Mnemonic::Nop, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Dcp, AddrMode::Aby, 7, false, false),
    op(Mnemonic::Nop, AddrMode::Abx, 4, true, false),
    op(Mnemonic::Cmp, AddrMode::Abx, 4, true, true),
    op(Mnemonic::Dec, AddrMode::Abx, 7, false, true),
    op(Mnemonic::Dcp, AddrMode::Abx, 7, false, false),
    // row 0xE0
    op(Mnemonic::Cpx, AddrMode::Imm, 2, false, true),
    op(Mnemonic::Sbc, AddrMode::Izx, 6, false, true),
    op(Mnemonic::Nop, AddrMode::Imm, 2, false, false),
    op(Mnemonic::Isc, AddrMode::Izx, 8, false, false),
    op(Mnemonic::Cpx, AddrMode::Zp, 3, false, true),
    op(Mnemonic::Sbc, AddrMode::Zp, 3, false, true),
    op(Mnemonic::Inc, AddrMode::Zp, 5, false, true),
    op(Mnemonic::Isc, AddrMode::Zp, 5, false, false),
    op(Mnemonic::Inx, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Sbc, AddrMode::Imm, 2, false, true),
    op(Mnemonic::Nop, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Usbc, AddrMode::Imm, 2, false, false),
    op(Mnemonic::Cpx, AddrMode::Abs, 4, false, true),
    op(Mnemonic::Sbc, AddrMode::Abs, 4, false, true),
    op(Mnemonic::Inc, AddrMode::Abs, 6, false, true),
    op(Mnemonic::Isc, AddrMode::Abs, 6, false, false),
    // row 0xF0
    op(Mnemonic::Beq, AddrMode::Rel, 2, false, true),
    op(Mnemonic::Sbc, AddrMode::Izy, 5, true, true),
    op(Mnemonic::Kil, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Isc, AddrMode::Izy, 8, false, false),
    op(Mnemonic::Nop, AddrMode::Zpx, 4, false, false),
    op(Mnemonic::Sbc, AddrMode::Zpx, 4, false, true),
    op(Mnemonic::Inc, AddrMode::Zpx, 6, false, true),
    op(Mnemonic::Isc, AddrMode::Zpx, 6, false, false),
    op(Mnemonic::Sed, AddrMode::Imp, 2, false, true),
    op(Mnemonic::Sbc, AddrMode::Aby, 4, true, true),
    op(Mnemonic::Nop, AddrMode::Imp, 2, false, false),
    op(Mnemonic::Isc, AddrMode::Aby, 7, false, false),
    op(Mnemonic::Nop, AddrMode::Abx, 4, true, false),
    op(Mnemonic::Sbc, AddrMode::Abx, 4, true, true),
    op(Mnemonic::Inc, AddrMode::Abx, 7, false, true),
    op(Mnemonic::Isc, AddrMode::Abx, 7, false, false),
];

/// Look up the static description of an opcode byte.
pub const fn info(opcode: u8) -> OpInfo {
    OPCODES[opcode as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_holds_256_entries() {
        // The array is const-sized, so this is a static guarantee; assert the
        // by-hand count anyway to catch future edits.
        assert_eq!(OPCODES.len(), 256);
    }

    #[test]
    fn official_opcode_count_is_151() {
        let official = OPCODES.iter().filter(|o| o.official).count();
        assert_eq!(official, 151);
        let unofficial = OPCODES.iter().filter(|o| !o.official).count();
        assert_eq!(unofficial, 105);
    }

    #[test]
    fn page_cross_only_for_indexed_reads() {
        for entry in OPCODES.iter() {
            if entry.page_cross {
                assert!(
                    matches!(entry.mode, AddrMode::Abx | AddrMode::Aby | AddrMode::Izy),
                    "page_cross set for non-indexed mode: {entry:?}"
                );
            }
        }
        // Writes never take the page-cross penalty.
        assert!(!info(0x9D).page_cross); // STA abs,X
        assert!(!info(0x99).page_cross); // STA abs,Y
        assert!(!info(0x91).page_cross); // STA (zp),Y
                                         // Read-modify-write abs,X is always 7 cycles (baked in, no flag).
        assert!(!info(0x1E).page_cross); // ASL abs,X
        assert_eq!(info(0x1E).cycles, 7);
        // Indexed reads do take the penalty.
        assert!(info(0xBD).page_cross); // LDA abs,X
        assert!(info(0xB9).page_cross); // LDA abs,Y
        assert!(info(0xB1).page_cross); // LDA (zp),Y
    }

    #[test]
    fn spot_check_well_known_opcodes() {
        let brk = info(0x00);
        assert_eq!(
            (brk.mnemonic.name(), brk.mode, brk.cycles),
            ("BRK", AddrMode::Imp, 7)
        );

        let lda_imm = info(0xA9);
        assert_eq!(
            (lda_imm.mnemonic.name(), lda_imm.mode, lda_imm.cycles),
            ("LDA", AddrMode::Imm, 2)
        );
        assert!(lda_imm.official);

        let jmp_ind = info(0x6C);
        assert_eq!(
            (jmp_ind.mnemonic.name(), jmp_ind.mode, jmp_ind.cycles),
            ("JMP", AddrMode::Ind, 5)
        );

        let nop = info(0xEA);
        assert_eq!(
            (nop.mnemonic.name(), nop.mode, nop.cycles),
            ("NOP", AddrMode::Imp, 2)
        );

        let beq = info(0xF0);
        assert_eq!(
            (beq.mnemonic.name(), beq.mode, beq.cycles),
            ("BEQ", AddrMode::Rel, 2)
        );

        let bne = info(0xD0);
        assert_eq!(bne.mnemonic.name(), "BNE");

        let jsr = info(0x20);
        assert_eq!(
            (jsr.mnemonic.name(), jsr.mode, jsr.cycles),
            ("JSR", AddrMode::Abs, 6)
        );

        let rts = info(0x60);
        assert_eq!(
            (rts.mnemonic.name(), rts.mode, rts.cycles),
            ("RTS", AddrMode::Imp, 6)
        );

        let rti = info(0x40);
        assert_eq!(
            (rti.mnemonic.name(), rti.mode, rti.cycles),
            ("RTI", AddrMode::Imp, 6)
        );

        let pha = info(0x48);
        assert_eq!(
            (pha.mnemonic.name(), pha.mode, pha.cycles),
            ("PHA", AddrMode::Imp, 3)
        );

        let pla = info(0x68);
        assert_eq!(
            (pla.mnemonic.name(), pla.mode, pla.cycles),
            ("PLA", AddrMode::Imp, 4)
        );
    }

    #[test]
    fn spot_check_undocumented_opcodes() {
        // 0xEB is the undocumented duplicate of SBC #imm.
        let usbc = info(0xEB);
        assert_eq!(usbc.mnemonic.name(), "USBC");
        assert!(!usbc.official);

        // Illegal halt instruction.
        assert_eq!(info(0x02).mnemonic.name(), "KIL");

        // NOP abs at 0x0C (unofficial).
        let nop_abs = info(0x0C);
        assert_eq!(
            (nop_abs.mode, nop_abs.cycles, nop_abs.official),
            (AddrMode::Abs, 4, false)
        );

        // LAX (zp),Y with page-cross penalty.
        let lax = info(0xB3);
        assert_eq!(lax.mnemonic.name(), "LAX");
        assert!(lax.page_cross);
        assert!(!lax.official);
    }

    #[test]
    fn branch_opcodes_all_take_two_base_cycles() {
        for opcode in [0x10, 0x30, 0x50, 0x70, 0x90, 0xB0, 0xD0, 0xF0] {
            let entry = info(opcode);
            assert_eq!(entry.mode, AddrMode::Rel);
            assert_eq!(entry.cycles, 2);
            assert!(entry.official);
        }
    }

    #[test]
    fn every_opcode_has_a_name() {
        for entry in OPCODES.iter() {
            assert!(!entry.mnemonic.name().is_empty());
        }
    }
}
