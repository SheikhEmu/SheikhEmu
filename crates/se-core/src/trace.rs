//! CPU trace logger with the reference-trace line format.
//!
//! The format matches the golden `nestest.log` reference trace line for
//! line and was derived from that public calibration artifact (the
//! M1.5 comparison data, category B — output only):
//!
//! ```text
//! C000  4C F5 C5  JMP $C5F5                       A:00 X:00 Y:00 P:24 SP:FD PPU:  0, 21 CYC:7
//! C5F7  86 00     STX $00 = 00                    A:00 X:00 Y:00 P:26 SP:FD PPU:  0, 36 CYC:12
//! ```
//!
//! Layout: `PC` (4 hex), two spaces, the opcode bytes left-aligned in 10
//! columns, the disassembly left-aligned in 32 columns, then
//! `A,X,Y,P,SP` (P shown without the phantom B flag), `PPU:scanline,dot`
//! (3-wide right-aligned each) and `CYC:` (decimal).
//!
//! Operand annotations follow the reference convention: memory reads
//! show `= value` (the value *before* the instruction modifies it),
//! indexed modes show `@ effective-address` first, indirect modes also
//! show the intermediate pointer, stores show the stored value, and
//! `JMP ($ind)` shows the resolved target. Undocumented opcodes are
//! spelled with the community names and a leading `*`, as in the
//! reference trace.

use crate::cpu::opcodes::{info, AddrMode, Mnemonic};
use crate::machine::Machine;

/// Render one trace line for the machine's *next* instruction.
///
/// Pure observation: memory is peeked, nothing is consumed or mutated,
/// so logging never perturbs execution.
pub fn format_trace_line(machine: &Machine) -> String {
    let pc = machine.cpu.pc;
    let opcode = machine.bus.peek(pc);
    let opinfo = info(opcode);

    let (bytes_field, disasm_field) = disassemble(machine, pc, opcode, opinfo);
    let (scanline, dot) = machine.ppu_position();
    // P never contains the phantom B flag in this core, but mask it the
    // way the reference trace displays it, defensively.
    let p = machine.cpu.p & !0x10;

    format!(
        "{:04X}  {:<9}{:<33}A:{:02X} X:{:02X} Y:{:02X} P:{:02X} SP:{:02X} PPU:{:>3},{:>3} CYC:{}",
        pc,
        bytes_field,
        disasm_field,
        machine.cpu.a,
        machine.cpu.x,
        machine.cpu.y,
        p,
        machine.cpu.sp,
        scanline,
        dot,
        machine.cycle()
    )
}

/// The bytes field and the disassembly field for one instruction.
fn disassemble(
    machine: &Machine,
    pc: u16,
    opcode: u8,
    opinfo: crate::cpu::opcodes::OpInfo,
) -> (String, String) {
    let peek = |addr: u16| machine.bus.peek(addr);
    let mnemonic_text = mnemonic_label(opinfo.mnemonic, opinfo.official);

    // Operand bytes for the addressing mode.
    let operand_len: u16 = match opinfo.mode {
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
    let mut bytes_field = format!("{:02X}", opcode);
    for i in 0..operand_len {
        bytes_field.push_str(&format!(" {:02X}", peek(pc + 1 + i)));
    }

    // Effective addressing (read-only) and the operand text.
    let x = machine.cpu.x;
    let y = machine.cpu.y;
    let operand = match opinfo.mode {
        AddrMode::Imp => String::new(),
        AddrMode::Acc => " A".to_string(),
        AddrMode::Imm => format!(" #${:02X}", peek(pc + 1)),
        AddrMode::Zp => {
            let addr = u16::from(peek(pc + 1));
            format!(" ${:02X}{}", addr, value_note(machine, addr))
        }
        AddrMode::Zpx | AddrMode::Zpy => {
            let base = peek(pc + 1);
            let idx = if opinfo.mode == AddrMode::Zpx { x } else { y };
            let addr = u16::from(base.wrapping_add(idx));
            format!(
                " ${:02X},{} @ {:02X}{}",
                base,
                index_letter(opinfo.mode),
                addr,
                value_note(machine, addr)
            )
        }
        AddrMode::Abs => {
            let addr = read16_at(peek, pc + 1);
            // JMP and JSR use the operand as a target, not as data: no
            // value annotation (verified against the reference trace).
            let note = match opinfo.mnemonic {
                Mnemonic::Jmp | Mnemonic::Jsr => String::new(),
                _ => value_note(machine, addr),
            };
            format!(" ${:04X}{}", addr, note)
        }
        AddrMode::Abx | AddrMode::Aby => {
            let base = read16_at(peek, pc + 1);
            let idx = if opinfo.mode == AddrMode::Abx { x } else { y };
            let addr = base.wrapping_add(u16::from(idx));
            format!(
                " ${:04X},{} @ {:04X}{}",
                base,
                index_letter(opinfo.mode),
                addr,
                value_note(machine, addr)
            )
        }
        AddrMode::Ind => {
            let ptr = read16_at(peek, pc + 1);
            // The $xxFF page-wrap bug: the high byte wraps within the page.
            let target = read16_bug(peek, ptr);
            format!(" (${:04X}) = {:04X}", ptr, target)
        }
        AddrMode::Izx => {
            let base = peek(pc + 1);
            let ptr = u16::from(base.wrapping_add(x));
            let target = read16_zp(peek, ptr as u8);
            let value = peek(target);
            format!(
                " (${:02X},X) @ {:02X} = {:04X}{}",
                base,
                ptr,
                target,
                read_note(value)
            )
        }
        AddrMode::Izy => {
            let base = peek(pc + 1);
            let ptr = read16_zp(peek, base);
            let target = ptr.wrapping_add(u16::from(y));
            let value = peek(target);
            format!(
                " (${:02X}),Y = {:04X} @ {:04X}{}",
                base,
                ptr,
                target,
                read_note(value)
            )
        }
        AddrMode::Rel => {
            let offset = peek(pc + 1) as i8;
            let target = pc.wrapping_add(2).wrapping_add(offset as i16 as u16);
            format!(" ${:04X}", target)
        }
    };

    (bytes_field, format!("{mnemonic_text}{operand}"))
}

/// `X` or `Y` for the indexed modes.
fn index_letter(mode: AddrMode) -> char {
    match mode {
        AddrMode::Zpx | AddrMode::Abx => 'X',
        AddrMode::Zpy | AddrMode::Aby => 'Y',
        _ => ' ',
    }
}

/// Annotation for zero-page/absolute (non-indexed, non-indirect) modes:
/// the byte currently at the effective address, i.e. *before* the
/// instruction runs (verified line-by-line against the reference trace:
/// stores, loads and read-modify-writes all show the old memory value).
fn value_note(machine: &Machine, addr: u16) -> String {
    format!(" = {:02X}", machine.bus.peek(addr))
}

/// Annotation for the indexed/indirect modes, where the effective value
/// has already been peeked by the caller.
fn read_note(read_value: u8) -> String {
    format!(" = {:02X}", read_value)
}

/// Mnemonic text, with the reference-trace spellings: a `*` prefix for
/// undocumented opcodes, `ISB`/`SBC` for the two renamed ones and `JAM`
/// for the halt family.
fn mnemonic_label(mnemonic: Mnemonic, official: bool) -> String {
    // One star column precedes the mnemonic: a space for official
    // opcodes, '*' for undocumented ones (matches the reference trace,
    // where the star occupies the column right before the mnemonic).
    let name = match mnemonic {
        Mnemonic::Isc => "ISB",
        Mnemonic::Usbc => "SBC",
        Mnemonic::Kil => "JAM",
        other => other.name(),
    };
    if official {
        format!(" {name}")
    } else {
        format!("*{name}")
    }
}

/// Little-endian 16-bit read through a peek function.
fn read16_at(peek: impl Fn(u16) -> u8, addr: u16) -> u16 {
    u16::from(peek(addr)) | u16::from(peek(addr.wrapping_add(1))) << 8
}

/// Little-endian 16-bit zero-page read with the page-0 wrap: the high
/// byte comes from `(zp + 1) & 0xFF`.
fn read16_zp(peek: impl Fn(u16) -> u8, zp: u8) -> u16 {
    let lo = u16::from(peek(u16::from(zp)));
    let hi = u16::from(peek(u16::from(zp.wrapping_add(1))));
    lo | hi << 8
}

/// The `JMP (ind)` page-wrap read: the high byte comes from the page
/// start when the pointer's low byte is $FF.
fn read16_bug(peek: impl Fn(u16) -> u8, ptr: u16) -> u16 {
    let lo = u16::from(peek(ptr));
    let hi_addr = (ptr & 0xFF00) | u16::from(ptr.wrapping_add(1) as u8);
    lo | u16::from(peek(hi_addr)) << 8
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::Bus;
    use crate::timing::Region;

    /// 16 KiB NROM helper: place bytes at an address, vectors at the end.
    fn machine_with(prg_at: u16, bytes: &[u8]) -> Machine {
        let mut prg = vec![0xEA; 16 * 1024];
        let start = usize::from(prg_at & 0x3FFF);
        prg[start..start + bytes.len()].copy_from_slice(bytes);
        prg[0x3FFA..0x4000].copy_from_slice(&[0x00, 0xC0, 0x00, 0xC0, 0x00, 0xC0]);
        let mut machine = Machine::new(prg, Region::Ntsc).expect("prg");
        machine.start_automation(prg_at);
        machine
    }

    /// Golden line 1 of the reference trace (public calibration data).
    #[test]
    fn matches_the_reference_first_line() {
        let machine = machine_with(0xC000, &[0x4C, 0xF5, 0xC5]);
        let line = format_trace_line(&machine);
        assert_eq!(
            line,
            "C000  4C F5 C5  JMP $C5F5                       A:00 X:00 Y:00 P:24 SP:FD PPU:  0, 21 CYC:7"
        );
    }

    /// Golden line 3 of the reference trace (a zero-page store).
    #[test]
    fn matches_the_reference_store_line() {
        let mut machine = machine_with(0xC5F7, &[0x86, 0x00]);
        // After LDX #$00 the CPU sits at C5F7 with X=0 and P=$26.
        machine.cpu.p = 0x26;
        machine.cpu.pc = 0xC5F7;
        // Advance to master cycle 12 (the automation base is 7).
        machine.bus.advance_cycles(5);
        let line = format_trace_line(&machine);
        assert_eq!(
            line,
            "C5F7  86 00     STX $00 = 00                    A:00 X:00 Y:00 P:26 SP:FD PPU:  0, 36 CYC:12"
        );
    }

    #[test]
    fn indexed_and_indirect_annotations() {
        // LDA ($10),Y with the pointer at $10 holding $0146 and Y=$FF.
        let mut machine = machine_with(0xC000, &[0xB1, 0x10]);
        machine.bus.write(0x0010, 0x46);
        machine.bus.write(0x0011, 0x01);
        machine.cpu.y = 0xFF;
        machine.bus.write(0x0245, 0x5A);
        let line = format_trace_line(&machine);
        assert!(line.contains("LDA ($10),Y = 0146 @ 0245 = 5A"));

        // LDA ($10,X): pointer lands at $80, target $0200, value $3C.
        let mut machine = machine_with(0xC000, &[0xA1, 0x10]);
        machine.cpu.x = 0x70;
        machine.bus.write(0x0080, 0x00);
        machine.bus.write(0x0081, 0x02);
        machine.bus.write(0x0200, 0x3C);
        let line = format_trace_line(&machine);
        assert!(line.contains("LDA ($10,X) @ 80 = 0200 = 3C"));

        // LDA $30,X with X=5 reads the zero page $35.
        let mut machine = machine_with(0xC000, &[0xB5, 0x30]);
        machine.cpu.x = 0x05;
        machine.bus.write(0x0035, 0x77);
        let line = format_trace_line(&machine);
        assert!(line.contains("LDA $30,X @ 35 = 77"));

        // LDA $0230,Y (4-digit effective address).
        let mut machine = machine_with(0xC000, &[0xB9, 0x30, 0x02]);
        machine.cpu.y = 0x33;
        machine.bus.write(0x0263, 0x99);
        let line = format_trace_line(&machine);
        assert!(line.contains("LDA $0230,Y @ 0263 = 99"));
    }

    #[test]
    fn undocumented_spelling_and_annotations() {
        // *SLO zp: effective address and the pre-modification value.
        let mut machine = machine_with(0xC000, &[0x07, 0x11]);
        machine.bus.write(0x0011, 0x81);
        let line = format_trace_line(&machine);
        assert!(line.contains("*SLO $11 = 81"));

        // *NOP abs,X reads memory at the effective address.
        let mut machine = machine_with(0xC000, &[0x1C, 0x00, 0x02]);
        machine.cpu.x = 0x40;
        machine.bus.write(0x0240, 0x9D);
        let line = format_trace_line(&machine);
        assert!(line.contains("*NOP $0200,X @ 0240 = 9D"));

        // *ISB carries the community name.
        let machine = machine_with(0xC000, &[0xE3, 0x10]);
        let line = format_trace_line(&machine);
        assert!(line.contains("*ISB ($10,X)"));

        // *SBC immediate keeps the plain spelling.
        let machine = machine_with(0xC000, &[0xEB, 0x05]);
        let line = format_trace_line(&machine);
        assert!(line.contains("*SBC #$05"));
    }

    #[test]
    fn branch_and_jmp_indirect_operands() {
        // BNE +8 from C000: target C00A, printed absolute.
        let machine = machine_with(0xC000, &[0xD0, 0x08]);
        let line = format_trace_line(&machine);
        assert!(line.contains("BNE $C00A"));

        // JMP ($C1FF): the page-wrap bug reads the high byte at $C100.
        // Values are baked into the PRG image (PRG writes are ignored).
        let mut prg = vec![0xEA; 16 * 1024];
        prg[0..3].copy_from_slice(&[0x6C, 0xFF, 0xC1]);
        prg[0x1FF] = 0x00; // low byte of the pointer at $C1FF.
        prg[0x100] = 0xC1; // wrapped high byte from $C100.
        prg[0x3FFA..0x4000].copy_from_slice(&[0x00, 0xC0, 0x00, 0xC0, 0x00, 0xC0]);
        let mut machine = Machine::new(prg, Region::Ntsc).expect("prg");
        machine.start_automation(0xC000);
        let line = format_trace_line(&machine);
        assert!(line.contains("JMP ($C1FF) = C100"), "got: {line}");
    }

    #[test]
    fn ppu_position_column_formats_three_wide() {
        // A line deep into a frame must keep 3-wide PPU numbers.
        let mut machine = machine_with(0xC000, &[0xEA]);
        machine.bus.advance_cycles(26_548 - 7);
        let line = format_trace_line(&machine);
        assert!(line.contains("PPU:233,191 CYC:26548"), "got: {line}");
    }

    #[test]
    fn trace_does_not_disturb_the_open_bus_latch() {
        let mut machine = machine_with(0xC000, &[0xAD, 0x00, 0x02]);
        machine.bus.write(0x0200, 0x42);
        // Reading through the bus trait updates the latch; peeking
        // (as the tracer does) must not.
        let before = machine.bus.latch();
        let _ = format_trace_line(&machine);
        assert_eq!(machine.bus.latch(), before);
    }
}
