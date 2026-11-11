//! `se-cli` — the Arabic-native command line interface of SheikhEmu.
//!
//! M1 provides two real commands: `info <file>` (cartridge report) and
//! `run <file>` — headless execution of an NROM cartridge with frame
//! hashing and an optional CPU trace log in the reference format
//! (the M1.5 nestest calibration protocol). Commands that do not
//! exist yet (PPU rendering, APU output, mapper families) are not
//! advertised or stubbed; they arrive with their milestones.

use std::env;
use std::fs;
use std::io::{BufWriter, Write};
use std::process::ExitCode;

mod png;

use se_core::machine::Machine;
use se_core::ppu::{Chr, Mirroring as CoreMirroring};
use se_core::timing::Region;
use se_core::trace::format_trace_line;
use se_rom::ines::{parse, InesRom, Mirroring, TimingRegion};
use se_rom::RomError;

/// Errors surfaced by CLI commands; parsing errors are re-displayed with
/// their own Arabic messages.
#[derive(Debug)]
enum CommandError {
    /// The file could not be opened or read.
    FileUnreadable,
    /// Parsing failed; the original error carries the Arabic message.
    Parse(RomError),
    /// The cartridge mapper is not one M1 can execute.
    UnsupportedMapper(u16),
    /// The PRG image is not an NROM size (16/32 KiB).
    BadPrgSize(usize),
    /// A command argument is missing or malformed.
    BadArgument(String),
    /// The CPU halted (JAM opcode) during the run.
    Halted { cycles: u64, instructions: u64 },
    /// The trace log file could not be written.
    TraceUnwritable,
    /// The frame dump PNG could not be written.
    FrameUnwritable,
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::FileUnreadable => {
                write!(f, "تعذر فتح الملف أو قراءته — تحقق من المسار")
            }
            CommandError::Parse(err) => write!(f, "{err}"),
            CommandError::UnsupportedMapper(mapper) => write!(
                f,
                "اللوحة رقم {mapper} غير مدعومة بعد — M1 ينفّذ NROM (اللوحة 0) فقط؛ عائلات اللوحات تصل في M3 وما بعدها"
            ),
            CommandError::BadPrgSize(size) => write!(
                f,
                "حجم PRG ({size} بايت) ليس من أحجام NROM (16 أو 32 كيلوبايت) — لا نخمّن تخطيطًا غير موثّق"
            ),
            CommandError::BadArgument(message) => write!(f, "وسيط غير صالح: {message}"),
            CommandError::Halted { cycles, instructions } => write!(
                f,
                "توقف المعالج بامر JAM بعد {cycles} دورة و{instructions} تعليمة — الروم يحتاج إعادة تهيئة للاستئناف"
            ),
            CommandError::TraceUnwritable => {
                write!(f, "تعذر كتابة ملف سجل التتبّع — تحقق من المسار والصلاحيات")
            }
            CommandError::FrameUnwritable => {
                write!(f, "تعذر كتابة ملف صورة الإطار — تحقق من المسار والصلاحيات")
            }
        }
    }
}

impl From<RomError> for CommandError {
    fn from(err: RomError) -> Self {
        CommandError::Parse(err)
    }
}

impl From<se_core::bus::BusError> for CommandError {
    fn from(err: se_core::bus::BusError) -> Self {
        match err {
            se_core::bus::BusError::UnsupportedPrgSize(size) => CommandError::BadPrgSize(size),
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    ExitCode::from(run(&args))
}

/// Entry point separated from `main` for testability.
fn run(args: &[String]) -> u8 {
    match args.get(1).map(String::as_str) {
        None | Some("-h") | Some("--help") => {
            print_usage();
            0
        }
        Some("info") => match args.get(2) {
            Some(path) => match info_command(path) {
                Ok(report) => {
                    println!("{report}");
                    0
                }
                Err(err) => {
                    eprintln!("خطأ: {err}");
                    1
                }
            },
            None => {
                eprintln!("خطأ: الأمر info يحتاج مسار ملف ROM");
                eprintln!("الاستعمال: se-cli info <ملف.nes>");
                2
            }
        },
        Some("run") => match args.get(2) {
            Some(path) => match run_command(path, &args[3..]) {
                Ok(report) => {
                    println!("{report}");
                    0
                }
                Err(err) => {
                    eprintln!("خطأ: {err}");
                    1
                }
            },
            None => {
                eprintln!("خطأ: الأمر run يحتاج مسار ملف ROM");
                eprintln!("الاستعمال: se-cli run <ملف.nes> --frames N [--hash]");
                2
            }
        },
        Some(other) => {
            eprintln!("خطأ: أمر غير معروف: {other}");
            print_usage();
            2
        }
    }
}

fn print_usage() {
    println!("SheikhEmu (se-cli) — واجهة سطر الأوامر");
    println!("الاستعمال:");
    println!("  se-cli info <ملف.nes>   تقرير تفصيلي عن الكارتريدج");
    println!("  se-cli run <ملف.nes> --frames N [--hash] [--trace-cpu <ملف>]");
    println!("        [--instructions N] [--start-pc <عنوان-ست-عشري>] [--dump-frame <ملف.png>]");
    println!("  se-cli -h | --help      هذه المساعدة");
}

/// Parse `path` and produce the Arabic cartridge report.
fn info_command(path: &str) -> Result<String, CommandError> {
    let data = fs::read(path).map_err(|_| CommandError::FileUnreadable)?;
    let rom = parse(&data)?;
    Ok(format_report(path, &rom))
}

/// How long a `run` lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RunLength {
    /// A number of frames at the per-region cycle budget.
    Frames(u32),
    /// An exact instruction count (the calibration protocol).
    Instructions(u64),
}

/// Options of the `run` command.
#[derive(Debug)]
struct RunOptions {
    length: Option<RunLength>,
    hash: bool,
    trace: Option<String>,
    start_pc: Option<u16>,
    dump_frame: Option<String>,
}

impl RunOptions {
    /// Parse the flag list after the ROM path. No dependencies: plain
    /// matching with Arabic error messages.
    fn parse(flags: &[String]) -> Result<Self, CommandError> {
        let mut options = RunOptions {
            length: None,
            hash: false,
            trace: None,
            start_pc: None,
            dump_frame: None,
        };
        let mut i = 0;
        while i < flags.len() {
            match flags[i].as_str() {
                "--frames" => {
                    let value = flags
                        .get(i + 1)
                        .and_then(|f| f.parse::<u32>().ok())
                        .ok_or_else(|| {
                            CommandError::BadArgument("--frames يحتاج رقمًا موجبًا".to_string())
                        })?;
                    options.length = Some(RunLength::Frames(value));
                    i += 2;
                }
                "--instructions" => {
                    let value = flags
                        .get(i + 1)
                        .and_then(|f| f.parse::<u64>().ok())
                        .ok_or_else(|| {
                            CommandError::BadArgument("--instructions يحتاج رقمًا موجبًا".to_string())
                        })?;
                    options.length = Some(RunLength::Instructions(value));
                    i += 2;
                }
                "--hash" => {
                    options.hash = true;
                    i += 1;
                }
                "--trace-cpu" => {
                    let value = flags.get(i + 1).ok_or_else(|| {
                        CommandError::BadArgument("--trace-cpu يحتاج مسار ملف".to_string())
                    })?;
                    options.trace = Some(value.clone());
                    i += 2;
                }
                "--start-pc" => {
                    let text = flags.get(i + 1).ok_or_else(|| {
                        CommandError::BadArgument("--start-pc يحتاج عنوانًا".to_string())
                    })?;
                    let cleaned = text.trim_start_matches("0x").trim_start_matches('$');
                    options.start_pc = Some(u16::from_str_radix(cleaned, 16).map_err(|_| {
                        CommandError::BadArgument(format!("عنوان غير صالح: {text}"))
                    })?);
                    i += 2;
                }
                "--dump-frame" => {
                    let value = flags.get(i + 1).ok_or_else(|| {
                        CommandError::BadArgument("--dump-frame يحتاج مسار ملف".to_string())
                    })?;
                    options.dump_frame = Some(value.clone());
                    i += 2;
                }
                other => {
                    return Err(CommandError::BadArgument(other.to_string()));
                }
            }
        }
        if options.length.is_none() {
            return Err(CommandError::BadArgument(
                "حدّد مدة التشغيل: --frames N أو --instructions N".to_string(),
            ));
        }
        Ok(options)
    }
}

/// Region selection: the header decides, NTSC when absent (documented
/// fallback), NTSC for multi-region boards (per their iNES 2.0 meaning).
fn region_of(rom: &InesRom) -> Region {
    match rom.timing {
        Some(TimingRegion::Pal) => Region::Pal,
        Some(TimingRegion::Dendy) => Region::Dendy,
        Some(TimingRegion::Ntsc) | Some(TimingRegion::Multiple) | None => Region::Ntsc,
    }
}

/// Execute a ROM headless and produce the Arabic run report.
fn run_command(path: &str, flags: &[String]) -> Result<String, CommandError> {
    let options = RunOptions::parse(flags)?;
    let data = fs::read(path).map_err(|_| CommandError::FileUnreadable)?;
    let rom = parse(&data)?;
    if rom.mapper != 0 {
        return Err(CommandError::UnsupportedMapper(rom.mapper));
    }
    let region = region_of(&rom);
    // Cartridge data for the M2 bus: CHR (ROM banks, or 8 KiB RAM when
    // the board ships none — the documented mapper-0 convention), the
    // nametable mirroring, and PRG RAM (the battery bit or an explicit
    // NES 2.0 size).
    let chr = if rom.chr.is_empty() {
        Chr::Ram(Box::new([0; 0x2000]))
    } else {
        Chr::Rom(rom.chr.clone())
    };
    let mirroring = match rom.mirroring {
        Mirroring::Vertical => CoreMirroring::Vertical,
        Mirroring::Horizontal => CoreMirroring::Horizontal,
        Mirroring::FourScreen => CoreMirroring::FourScreen,
    };
    let prg_ram = rom.battery || rom.prg_ram.is_some_and(|bytes| bytes > 0);
    let mut machine = Machine::from_cartridge(rom.prg, chr, mirroring, prg_ram, region)?;

    // Test-ROM automation entry (nestest convention) or a real reset.
    match options.start_pc {
        Some(pc) => machine.start_automation(pc),
        None => machine.reset(),
    }

    // Optional trace writer (reference line format).
    let trace_handle = options
        .trace
        .as_ref()
        .map(|file| fs::File::create(file).map_err(|_| CommandError::TraceUnwritable));
    let mut trace_writer = match trace_handle {
        Some(result) => Some(BufWriter::new(result?)),
        None => None,
    };

    let mut halted = None;
    let trace_one = |machine: &Machine, writer: &mut Option<BufWriter<fs::File>>| {
        if let Some(w) = writer {
            let line = format_trace_line(machine);
            let _ = writeln!(w, "{line}");
        }
    };

    match options.length.unwrap_or(RunLength::Frames(1)) {
        RunLength::Instructions(count) => {
            for _ in 0..count {
                trace_one(&machine, &mut trace_writer);
                if let Err(se_core::CpuError::Halted) = machine.step() {
                    halted = Some((machine.cycle(), machine.instructions));
                    break;
                }
            }
        }
        RunLength::Frames(frames) => {
            // Real PPU frames: each iteration runs to the next frame
            // completion so a trace stays readable frame by frame.
            for _ in 0..frames {
                let target = machine.ppu().frame() + 1;
                while machine.ppu().frame() < target {
                    trace_one(&machine, &mut trace_writer);
                    if let Err(se_core::CpuError::Halted) = machine.step() {
                        halted = Some((machine.cycle(), machine.instructions));
                        break;
                    }
                }
                if halted.is_some() {
                    break;
                }
            }
        }
    }
    if let Some(writer) = trace_writer.as_mut() {
        writer.flush().map_err(|_| CommandError::TraceUnwritable)?;
    }
    if let Some((cycles, instructions)) = halted {
        return Err(CommandError::Halted {
            cycles,
            instructions,
        });
    }

    // The Arabic report.
    let file_name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let length_text = match options.length {
        Some(RunLength::Frames(frames)) => format!("{frames} إطارًا"),
        Some(RunLength::Instructions(count)) => format!("{count} تعليمات"),
        None => String::new(),
    };
    let region_text = match region {
        Region::Ntsc => "NTSC",
        Region::Pal => "PAL",
        Region::Dendy => "Dendy",
    };
    let mut report = String::new();
    report.push_str(&format!("تقرير التشغيل: {file_name}\n"));
    report.push_str(&format!("المنطقة: {region_text}\n"));
    report.push_str(&format!("مدة التشغيل: {length_text}\n"));
    report.push_str(&format!("الإطارات المكتملة: {}\n", machine.ppu().frame()));
    report.push_str(&format!("الدورات المنفّذة: {}\n", machine.cycle()));
    report.push_str(&format!("التعليمات المنفّذة: {}\n", machine.instructions));
    if let Some(file) = options.trace.as_deref() {
        report.push_str(&format!("سجل التتبّع: {file}\n"));
    }
    if options.hash {
        report.push_str(&format!("بصمة الحالة: {:016x}\n", machine.state_hash()));
        report.push_str(&format!(
            "بصمة الإطار: {:016x}\n",
            machine.ppu().last_plane_hash()
        ));
    }
    if let Some(file) = options.dump_frame.as_deref() {
        // Heap-allocated: the fixed RGB plane stays out of the stack.
        let mut rgb = Box::new([0u8; 256 * 240 * 3]);
        machine.ppu().last_frame_rgb(&mut rgb);
        png::write_png(std::path::Path::new(file), rgb.as_slice(), 256, 240)
            .map_err(|_| CommandError::FrameUnwritable)?;
        report.push_str(&format!("صورة الإطار: {file}\n"));
    }
    Ok(report)
}

/// Human-readable sizes: KiB for round multiples, bytes otherwise.
fn human_size(bytes: usize) -> String {
    if bytes.is_multiple_of(1024) && bytes >= 1024 {
        format!("{} كيلوبايت", bytes / 1024)
    } else {
        format!("{bytes} بايت")
    }
}

fn mirroring_label(mirroring: Mirroring) -> &'static str {
    match mirroring {
        Mirroring::Vertical => "عمودي (Vertical)",
        Mirroring::Horizontal => "أفقي (Horizontal)",
        Mirroring::FourScreen => "شاشة رباعية (Four-screen)",
    }
}

fn timing_label(timing: Option<TimingRegion>) -> String {
    match timing {
        None => "غير محدد (ترويسة iNES قديمة — سيُستخدم NTSC عند التشغيل)".to_string(),
        Some(t) => match t {
            TimingRegion::Ntsc => "NTSC (RP2C02)".to_string(),
            TimingRegion::Pal => "PAL (RP2C07)".to_string(),
            TimingRegion::Multiple => "متعدد المناطق (Multiple-region)".to_string(),
            TimingRegion::Dendy => "Dendy (UA6538)".to_string(),
        },
    }
}

fn optional_size(field: Option<u64>) -> String {
    match field {
        None => "غير معلن".to_string(),
        Some(bytes) => human_size(bytes as usize),
    }
}

fn console_type_label(console_type: u8) -> &'static str {
    match console_type {
        0 => "NES / Famicom قياسي",
        1 => "Nintendo Vs. System",
        2 => "Playchoice 10",
        _ => "نوع موسع (Extended)",
    }
}

/// Render the full Arabic report for a parsed ROM.
pub fn format_report(path: &str, rom: &InesRom) -> String {
    let file_name = path.rsplit(['/', '\\']).next().unwrap_or(path);

    let mut report = String::new();
    report.push_str(&format!("تقرير الكارتريدج: {file_name}\n"));
    report.push_str(&format!(
        "الصيغة: {}\n",
        if rom.nes2 { "NES 2.0" } else { "iNES" }
    ));
    report.push_str(&format!(
        "الـmapper: {}{}\n",
        rom.mapper,
        if rom.nes2 {
            format!(" (submapper: {})", rom.submapper)
        } else {
            String::new()
        }
    ));
    report.push_str(&format!(
        "PRG-ROM: {} ({})\n",
        human_size(rom.prg.len()),
        rom.prg.len()
    ));
    report.push_str(&format!(
        "CHR-ROM: {} ({})\n",
        human_size(rom.chr.len()),
        rom.chr.len()
    ));
    if let Some(trainer) = &rom.trainer {
        report.push_str(&format!("Trainer: موجود ({})\n", human_size(trainer.len())));
    }
    report.push_str(&format!("الانعكاس: {}\n", mirroring_label(rom.mirroring)));
    report.push_str(&format!(
        "البطارية (ذاكرة غير متطايرة): {}\n",
        if rom.battery { "نعم" } else { "لا" }
    ));
    report.push_str(&format!("التوقيت: {}\n", timing_label(rom.timing)));
    if rom.nes2 {
        report.push_str(&format!("PRG-RAM: {}\n", optional_size(rom.prg_ram)));
        report.push_str(&format!("PRG-NVRAM: {}\n", optional_size(rom.prg_nvram)));
        report.push_str(&format!("CHR-RAM: {}\n", optional_size(rom.chr_ram)));
        report.push_str(&format!("CHR-NVRAM: {}\n", optional_size(rom.chr_nvram)));
    }
    report.push_str(&format!(
        "نوع الجهاز: {}\n",
        console_type_label(rom.console_type)
    ));
    if !rom.misc_rom.is_empty() {
        report.push_str(&format!(
            "منطقة ROM إضافية: {} (رقم معلن: {})\n",
            human_size(rom.misc_rom.len()),
            rom.misc_rom_count
        ));
    }

    match rom.warnings.len() {
        0 => report.push_str("التحذيرات: لا يوجد\n"),
        n => {
            report.push_str(&format!("التحذيرات: {n}\n"));
            for warning in &rom.warnings {
                report.push_str(&format!("- تحذير: {warning}\n"));
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    use se_rom::Warning;

    /// Minimal NROM image for the CLI tests.
    fn nrom() -> Vec<u8> {
        let mut v = vec![
            b'N', b'E', b'S', 0x1A, 1, 1, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        v.extend(core::iter::repeat_n(0u8, 16 * 1024));
        v.extend(core::iter::repeat_n(0xFFu8, 8 * 1024));
        v
    }

    fn temp_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("se-cli-{tag}-{}.nes", std::process::id()))
    }

    #[test]
    fn info_reports_a_valid_nrom() {
        let path = temp_path("valid");
        fs::write(&path, nrom()).expect("write temp ROM");
        let report = info_command(path.to_str().expect("utf8 path")).expect("valid ROM parses");
        fs::remove_file(&path).ok();

        assert!(report.contains("تقرير الكارتريدج"));
        assert!(report.contains("الصيغة: iNES"));
        assert!(report.contains("الـmapper: 0"));
        assert!(report.contains("16 كيلوبايت"));
        assert!(report.contains("8 كيلوبايت"));
        assert!(report.contains("NTSC"));
        assert!(report.contains("التحذيرات: لا يوجد"));
    }

    #[test]
    fn info_reports_chrram_assumption_warning() {
        let mut v = vec![
            b'N', b'E', b'S', 0x1A, 1, 0, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        v.extend(core::iter::repeat_n(0u8, 16 * 1024));
        let path = temp_path("chrram");
        fs::write(&path, v).expect("write temp ROM");
        let report = info_command(path.to_str().expect("utf8 path")).expect("valid ROM parses");
        fs::remove_file(&path).ok();

        assert!(report.contains("التحذيرات: 1"));
        assert!(report.contains("CHR-RAM"));
    }

    #[test]
    fn missing_file_is_a_readable_error() {
        let err = info_command("/definitely/not/a/real/rom.nes").expect_err("missing file");
        assert!(err.to_string().contains("تعذر"));
    }

    #[test]
    fn bad_magic_error_is_in_arabic() {
        let path = temp_path("bad");
        fs::write(&path, vec![0u8; 32]).expect("write temp file");
        let err = info_command(path.to_str().expect("utf8 path")).expect_err("not a ROM");
        fs::remove_file(&path).ok();
        assert!(err.to_string().contains("ليس ملف ROM صالحًا"));
    }

    #[test]
    fn usage_without_arguments_is_not_an_error() {
        // `run` with only the program name prints usage and returns 0.
        let code = run(&["se-cli".to_string()]);
        assert_eq!(code, 0);
    }

    // ---- run command (M1) ---------------------------------------------------

    /// 16 KiB NROM image whose program is an infinite NOP slide.
    fn nop_rom() -> Vec<u8> {
        let mut v = vec![
            b'N', b'E', b'S', 0x1A, 1, 1, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let mut prg = vec![0xEA; 16 * 1024];
        prg[0x3FFA..0x4000].copy_from_slice(&[0x00, 0x80, 0x00, 0x80, 0x00, 0x80]);
        v.extend(prg);
        v.extend(vec![0x00; 8 * 1024]);
        v
    }

    #[test]
    fn run_reports_cycles_and_instructions_in_arabic() {
        let path = temp_path("run");
        fs::write(&path, nop_rom()).expect("write ROM");
        let flags: [String; 2] = ["--frames".to_string(), "2".to_string()];
        let report = run_command(path.to_str().expect("utf8"), &flags).expect("runs");
        fs::remove_file(&path).ok();
        assert!(report.contains("تقرير التشغيل"));
        assert!(report.contains("المنطقة: NTSC"));
        assert!(report.contains("الدورات المنفّذة:"));
        assert!(report.contains("التعليمات المنفّذة:"));
    }

    #[test]
    fn run_hash_is_deterministic() {
        let path = temp_path("hash");
        fs::write(&path, nop_rom()).expect("write ROM");
        let flags: [String; 3] = [
            "--frames".to_string(),
            "2".to_string(),
            "--hash".to_string(),
        ];
        let first = run_command(path.to_str().expect("utf8"), &flags).expect("runs");
        let second = run_command(path.to_str().expect("utf8"), &flags).expect("runs");
        fs::remove_file(&path).ok();
        fn hash_line(report: &str) -> &str {
            report
                .lines()
                .find(|line| line.contains("بصمة الحالة"))
                .expect("hash line present")
        }
        assert_eq!(hash_line(&first), hash_line(&second));
    }

    #[test]
    fn run_writes_a_reference_format_trace() {
        let path = temp_path("trace");
        fs::write(&path, nop_rom()).expect("write ROM");
        let trace_path = temp_path("trace-log").with_extension("log");
        let trace = trace_path.to_str().expect("utf8").to_string();
        let flags: [String; 4] = [
            "--instructions".to_string(),
            "2".to_string(),
            "--trace-cpu".to_string(),
            trace.clone(),
        ];
        let report = run_command(path.to_str().expect("utf8"), &flags).expect("runs");
        fs::remove_file(&path).ok();
        assert!(report.contains("سجل التتبّع"));
        let log = fs::read_to_string(&trace_path).expect("trace written");
        fs::remove_file(&trace_path).ok();
        // First line: reset vector $8000, NOP, at cycle 7, PPU (0,21).
        assert!(log.starts_with("8000  EA        NOP"), "got: {log}");
        assert!(log.lines().count() == 2);
    }

    #[test]
    fn run_start_pc_accepts_the_hex_forms() {
        let path = temp_path("pc");
        fs::write(&path, nop_rom()).expect("write ROM");
        for form in ["C000", "0xC000", "$C000"] {
            let flags: [String; 4] = [
                "--instructions".to_string(),
                "1".to_string(),
                "--start-pc".to_string(),
                form.to_string(),
            ];
            let report = run_command(path.to_str().expect("utf8"), &flags)
                .unwrap_or_else(|_| panic!("start-pc form {form} rejected"));
            assert!(report.contains("تقرير التشغيل"));
        }
        fs::remove_file(&path).ok();
    }

    #[test]
    fn run_rejects_missing_length_and_unknown_flags() {
        let path = temp_path("badflags");
        fs::write(&path, nop_rom()).expect("write ROM");
        let none: [String; 0] = [];
        let err = run_command(path.to_str().expect("utf8"), &none).expect_err("needs length");
        assert!(err.to_string().contains("وسيط غير صالح"));
        let bad = ["--bogus".to_string()];
        let err = run_command(path.to_str().expect("utf8"), &bad).expect_err("unknown flag");
        assert!(err.to_string().contains("وسيط غير صالح"));
        fs::remove_file(&path).ok();
    }

    #[test]
    fn run_rejects_non_nrom_mappers_with_arabic_error() {
        // Byte 6 high nibble carries the low mapper nibble: 0x10 = mapper 1.
        let mut v = vec![
            b'N', b'E', b'S', 0x1A, 1, 1, 0x10, 0x00, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        v.extend(vec![0xEA; 16 * 1024]);
        v.extend(vec![0x00; 8 * 1024]);
        let path = temp_path("mapper");
        fs::write(&path, v).expect("write ROM");
        let flags = ["--frames".to_string(), "1".to_string()];
        let err = run_command(path.to_str().expect("utf8"), &flags).expect_err("mapper");
        fs::remove_file(&path).ok();
        let text = err.to_string();
        assert!(text.contains("اللوحة رقم 1"));
        assert!(text.contains("NROM"));
    }

    #[test]
    fn run_reports_halted_cpu_in_arabic() {
        let mut prg = vec![0x02; 16 * 1024]; // JAM slide.
        prg[0x3FFA..0x4000].copy_from_slice(&[0x00, 0x80, 0x00, 0x80, 0x00, 0x80]);
        let mut v = vec![
            b'N', b'E', b'S', 0x1A, 1, 1, 0x00, 0x00, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        v.extend(prg);
        v.extend(vec![0x00; 8 * 1024]);
        let path = temp_path("jam");
        fs::write(&path, v).expect("write ROM");
        let flags = ["--frames".to_string(), "1".to_string()];
        let err = run_command(path.to_str().expect("utf8"), &flags).expect_err("halts");
        fs::remove_file(&path).ok();
        assert!(err.to_string().contains("JAM"));
    }

    #[test]
    fn unknown_command_is_an_error() {
        let code = run(&["se-cli".to_string(), "play".to_string()]);
        assert_eq!(code, 2);
        let code = run(&["se-cli".to_string(), "info".to_string()]);
        assert_eq!(code, 2);
    }

    #[test]
    fn warning_display_is_in_arabic() {
        let text = Warning::DiskDudeGarbage.to_string();
        assert!(text.contains("DiskDude") || text.contains('ت'));
    }
}
