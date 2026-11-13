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
    /// The PRG image is not a valid size for the board.
    BadPrgSize(usize),
    /// The CHR image is not a valid size for the board.
    BadChrSize(usize),
    /// The submapper combination is refused.
    UnsupportedSubmapper(u16, u8),
    /// A command argument is missing or malformed.
    BadArgument(String),
    /// The CPU halted (JAM opcode) during the run.
    Halted { cycles: u64, instructions: u64 },
    /// The trace log file could not be written.
    TraceUnwritable,
    /// The frame dump PNG could not be written.
    FrameUnwritable,
    /// The RAM dump file could not be written.
    RamUnwritable,
    /// The video frame directory could not be written.
    VideoUnwritable,
    /// The input script file could not be opened.
    ScriptUnreadable,
    /// The input script could not be parsed (Arabic reason).
    ScriptBad(String),
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
                "اللوحة رقم {mapper} غير مدعومة بعد — المدعوم حاليًا: طاقم T1 (NROM 0 وUxROM 2 وCNROM 3 وAxROM 7 وColorDreams 11 وBNROM 34 وGxROM 66 وCamerica 71 وJ87 87/101) وMMC1 الأدنى (1، لوحة بوابة M2)؛ باقي العائلات تصل في M5"
            ),
            CommandError::BadPrgSize(size) => write!(
                f,
                "حجم PRG ({size} بايت) ليس حجمًا صالحًا لهذه اللوحة — لا نخمّن تخطيطًا غير موثّق"
            ),
            CommandError::BadChrSize(size) => write!(
                f,
                "حجم CHR ({size} بايت) ليس حجمًا صالحًا لهذه اللوحة — لا نخمّن تخطيطًا غير موثّق"
            ),
            CommandError::UnsupportedSubmapper(mapper, submapper) => write!(
                f,
                "اللوحة {mapper} بالمصرف الفرعي {submapper} مرفوضة صراحة (NINA-001/002 تصل مع عائلات اللوحات في M5) — رسالة موثّقة لا شاشة سوداء"
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
            CommandError::RamUnwritable => {
                write!(f, "تعذر كتابة ملف تفريغ الذاكرة — تحقق من المسار والصلاحيات")
            }
            CommandError::VideoUnwritable => {
                write!(f, "تعذر كتابة إطارات الفيديو — تحقق من المجلد والصلاحيات")
            }
            CommandError::ScriptUnreadable => {
                write!(f, "تعذر فتح ملف سيناريو الإدخال — تحقق من المسار")
            }
            CommandError::ScriptBad(reason) => {
                write!(f, "سيناريو الإدخال غير صالح: {reason}")
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
            se_core::bus::BusError::UnsupportedMapper(mapper) => {
                CommandError::UnsupportedMapper(mapper)
            }
            se_core::bus::BusError::UnsupportedSubmapper(mapper, submapper) => {
                CommandError::UnsupportedSubmapper(mapper, submapper)
            }
            se_core::bus::BusError::BadChrSize(size) => CommandError::BadChrSize(size),
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
        Some("script-help") => {
            print_script_help();
            0
        }
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
    println!("        [--dump-plane <ملف>] [--input-script <ملف>] [--dump-ram <ملف>] [--dump-video <مجلد>]");
    println!("  se-cli script-help    صيغة ملف سيناريو الإدخال");
    println!("  se-cli -h | --help      هذه المساعدة");
}

/// The `script-help` text: the input scenario file format (M3).
fn print_script_help() {
    println!("صيغة سيناريو الإدخال (يُطبّق على المتحكم الأول):");
    println!("  كل سطر: رقم-الإطار [أسماء الأزرار...]");
    println!("  رقم الإطار عددي من صفر؛ الحالة تسري من بداية ذلك الإطار حتى أقرب سطر لاحق.");
    println!("  أسماء الأزرار: a b select start up down left right");
    println!("  سطر رقم بلا أزرار = لا شيء مضغوط؛ الأسطر التي تبدأ بـ # تعليقات.");
    println!("مثال:");
    println!("  0");
    println!("  30 start");
    println!("  120 left");
}

/// A deterministic input scenario: button snapshots applied to
/// controller 1 at frame boundaries (the frame index counts
/// completed frames; entry N applies from the start of frame N).
#[derive(Debug)]
struct InputScript {
    schedule: Vec<(u32, u8)>,
}

impl InputScript {
    /// Parse the scenario text. Plain matching with Arabic errors.
    fn parse(text: &str) -> Result<Self, String> {
        use se_core::{BTN_A, BTN_B, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SELECT, BTN_START, BTN_UP};
        let mut schedule: Vec<(u32, u8)> = Vec::new();
        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let frame_text = parts.next().unwrap_or_default();
            let frame = frame_text
                .parse::<u32>()
                .map_err(|_| format!("السطر {}: رقم إطار غير صالح: {frame_text}", index + 1))?;
            let mut buttons = 0u8;
            for name in parts {
                buttons |= match name.to_ascii_lowercase().as_str() {
                    "a" => BTN_A,
                    "b" => BTN_B,
                    "select" => BTN_SELECT,
                    "start" => BTN_START,
                    "up" => BTN_UP,
                    "down" => BTN_DOWN,
                    "left" => BTN_LEFT,
                    "right" => BTN_RIGHT,
                    other => {
                        return Err(format!("السطر {}: اسم زر غير معروف: {other}", index + 1));
                    }
                };
            }
            schedule.push((frame, buttons));
        }
        // Frame order tolerance: sort, then let the last entry at a
        // duplicated frame supersede earlier ones.
        schedule.sort_by_key(|(frame, _)| *frame);
        let mut merged: Vec<(u32, u8)> = Vec::new();
        for (frame, buttons) in schedule {
            if let Some(last) = merged.last_mut() {
                if last.0 == frame {
                    last.1 = buttons;
                    continue;
                }
            }
            merged.push((frame, buttons));
        }
        Ok(InputScript { schedule: merged })
    }
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
    dump_plane: Option<String>,
    /// `--dump-ram`: write RAM ($0000-$07FF) then cartridge WRAM
    /// ($6000-$7FFF) to a file at the end of the run (the M3 game
    /// gate reads its telemetry cells out of WRAM).
    dump_ram: Option<String>,
    /// `--dump-video <dir>`: write every completed frame as a PPM
    /// image into the directory (the M3 owner-review video source).
    /// The directory must already exist.
    dump_video: Option<String>,
    input_script: Option<String>,
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
            dump_plane: None,
            dump_ram: None,
            dump_video: None,
            input_script: None,
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
                "--dump-plane" => {
                    let value = flags.get(i + 1).ok_or_else(|| {
                        CommandError::BadArgument("--dump-plane يحتاج مسار ملف".to_string())
                    })?;
                    options.dump_plane = Some(value.clone());
                    i += 2;
                }
                "--dump-ram" => {
                    let value = flags.get(i + 1).ok_or_else(|| {
                        CommandError::BadArgument("--dump-ram يحتاج مسار ملف".to_string())
                    })?;
                    options.dump_ram = Some(value.clone());
                    i += 2;
                }
                "--dump-video" => {
                    let value = flags.get(i + 1).ok_or_else(|| {
                        CommandError::BadArgument("--dump-video يحتاج مسار مجلد".to_string())
                    })?;
                    options.dump_video = Some(value.clone());
                    i += 2;
                }
                "--input-script" => {
                    let value = flags.get(i + 1).ok_or_else(|| {
                        CommandError::BadArgument("--input-script يحتاج مسار ملف".to_string())
                    })?;
                    options.input_script = Some(value.clone());
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
    // The M3 mapper set: the T1 family plus the minimal MMC1 (the
    // M2 gate board, D-22). Anything else waits for M5 and gets the
    // explicit Arabic refusal — never a silent run.
    if !matches!(rom.mapper, 0 | 1 | 2 | 3 | 7 | 11 | 34 | 66 | 71 | 87 | 101) {
        return Err(CommandError::UnsupportedMapper(rom.mapper));
    }
    let region = region_of(&rom);
    // Cartridge data for the M2 bus: CHR (ROM banks, or 8 KiB RAM when
    // the board ships none — the documented mapper-0 convention), the
    // nametable mirroring, and PRG RAM (the battery bit or an explicit
    // NES 2.0 size; MMC1 boards in the gate's harness always carry
    // the $6000 WRAM the blargg protocol needs — D-22).
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
    let prg_ram = rom.battery || rom.prg_ram.is_some_and(|bytes| bytes > 0) || rom.mapper == 1;
    let mut machine = Machine::from_rom(
        rom.prg,
        rom.mapper,
        rom.submapper,
        chr,
        mirroring,
        prg_ram,
        region,
    )?;

    // Test-ROM automation entry (nestest convention) or a real reset.
    match options.start_pc {
        Some(pc) => machine.start_automation(pc),
        None => machine.reset(),
    }

    // The input scenario (controller 1): snapshots applied at frame
    // boundaries, before the frame runs (deterministic by
    // construction — the same file gives the same run).
    let script = match options.input_script.as_deref() {
        Some(file) => Some(
            InputScript::parse(
                &fs::read_to_string(file).map_err(|_| CommandError::ScriptUnreadable)?,
            )
            .map_err(CommandError::ScriptBad)?,
        ),
        None => None,
    };

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
    // The `--dump-video` frame counter (reported after the run).
    let mut video_written = 0u32;
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
            // The input scenario advances at frame boundaries.
            let video = options.dump_video.as_deref().map(std::path::PathBuf::from);
            let mut picture: Option<Box<[u8; 256 * 240 * 3]>> =
                video.as_ref().map(|_| Box::new([0; 256 * 240 * 3]));
            let mut script_cursor = 0usize;
            let mut buttons = 0u8;
            for frame_index in 0..frames {
                if let Some(script) = &script {
                    while script_cursor < script.schedule.len()
                        && script.schedule[script_cursor].0 <= frame_index
                    {
                        buttons = script.schedule[script_cursor].1;
                        script_cursor += 1;
                    }
                    machine.set_controller_buttons(0, buttons);
                }
                let target = machine.ppu().frame() + 1;
                while machine.ppu().frame() < target {
                    trace_one(&machine, &mut trace_writer);
                    if let Err(se_core::CpuError::Halted) = machine.step() {
                        halted = Some((machine.cycle(), machine.instructions));
                        break;
                    }
                }
                if let (Some(dir), Some(picture)) = (&video, picture.as_mut()) {
                    machine.ppu().last_frame_rgb(picture);
                    let path = dir.join(format!("f{:05}.ppm", frame_index));
                    let mut data = Vec::with_capacity(15 + 256 * 240 * 3);
                    data.extend_from_slice(b"P6\n256 240\n255\n");
                    data.extend_from_slice(picture.as_slice());
                    fs::write(path, data).map_err(|_| CommandError::VideoUnwritable)?;
                    video_written += 1;
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
    if let Some(dir) = options.dump_video.as_deref() {
        report.push_str(&format!("إطارات الفيديو: {video_written} إطارًا → {dir}\n"));
    }
    report.push_str(&format!("المنطقة: {region_text}\n"));
    report.push_str(&format!("مدة التشغيل: {length_text}\n"));
    report.push_str(&format!("الإطارات المكتملة: {}\n", machine.ppu().frame()));
    report.push_str(&format!("الدورات المنفّذة: {}\n", machine.cycle()));
    report.push_str(&format!("التعليمات المنفّذة: {}\n", machine.instructions));
    if let Some(file) = options.trace.as_deref() {
        report.push_str(&format!("سجل التتبّع: {file}\n"));
    }
    if let Some(file) = options.input_script.as_deref() {
        let events = script.as_ref().map(|s| s.schedule.len()).unwrap_or(0);
        report.push_str(&format!("سيناريو الإدخال: {file} (أحداث: {events})\n"));
    }
    if options.hash {
        report.push_str(&format!("بصمة الحالة: {:016x}\n", machine.state_hash()));
        report.push_str(&format!(
            "بصمة الإطار: {:016x}\n",
            machine.ppu().last_plane_hash()
        ));
    }
    // The blargg $6000 protocol report (when the cartridge carries
    // WRAM): the $DE $B0 signature, the status byte and the
    // null-terminated text.
    if machine.bus.prg_ram().is_some() {
        report.push_str(&blargg_report(&machine));
    }
    if let Some(file) = options.dump_frame.as_deref() {
        // Heap-allocated: the fixed RGB plane stays out of the stack.
        let mut rgb = Box::new([0u8; 256 * 240 * 3]);
        machine.ppu().last_frame_rgb(&mut rgb);
        png::write_png(std::path::Path::new(file), rgb.as_slice(), 256, 240)
            .map_err(|_| CommandError::FrameUnwritable)?;
        report.push_str(&format!("صورة الإطار: {file}\n"));
    }
    if let Some(file) = options.dump_ram.as_deref() {
        // RAM first (2 KiB), then cartridge WRAM when present (8 KiB
        // or the board's size): the deterministic post-run snapshot.
        let mut dump = Vec::with_capacity(0x800 + 0x2000);
        dump.extend_from_slice(machine.bus.ram());
        if let Some(wram) = machine.bus.prg_ram() {
            dump.extend_from_slice(wram);
        }
        let bytes = dump.len();
        fs::write(std::path::Path::new(file), &dump).map_err(|_| CommandError::RamUnwritable)?;
        report.push_str(&format!("تفريغ الذاكرة: {file} ({bytes} بايت)\n"));
    }
    if let Some(file) = options.dump_plane.as_deref() {
        // The raw palette-index plane (256x240 bytes): the golden
        // comparison artifact for the self-made calibration ROMs.
        fs::write(std::path::Path::new(file), machine.ppu().last_frame_plane())
            .map_err(|_| CommandError::FrameUnwritable)?;
        report.push_str(&format!("مستوى الإطار الخام: {file}\n"));
    }
    Ok(report)
}

/// Read the blargg test protocol at `$6000-$7FFF` and render the
/// Arabic report: the `$DE $B0` signature, the status byte
/// (`$80` = still running, `$00` = all tests passed, `$01..` = the
/// failing test number) and the null-terminated ASCII text.
fn blargg_report(machine: &Machine) -> String {
    let wram = machine.bus.prg_ram().expect("wram present");
    let signature_ok = wram[1] == 0xDE && wram[2] == 0xB0;
    let status = wram[0];
    let text: String = wram[4..0x100]
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect();
    let mut out = String::new();
    out.push_str(&format!(
        "بروتوكول blargg: {}\n",
        if signature_ok {
            "موقّع DEB0"
        } else {
            "بلا توقيع (الروم لم يكتب النتائج)"
        }
    ));
    if !signature_ok {
        return out;
    }
    match status {
        0x80 => out.push_str("حالة الاختبار: ما زال يعمل (80)\n"),
        0x00 => out.push_str("حالة الاختبار: نجاح كامل (00)\n"),
        code => out.push_str(&format!("حالة الاختبار: فشل الرمز {code:02X} — راجع النص\n")),
    }
    if !text.is_empty() {
        out.push_str(&format!("نص النتيجة: {text}\n"));
    }
    out
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
    fn run_dump_ram_writes_ram_and_reports_it() {
        let path = temp_path("dumpram");
        fs::write(&path, nop_rom()).expect("write ROM");
        let dump = temp_path("dumpram.bin");
        let flags: [String; 4] = [
            "--frames".to_string(),
            "2".to_string(),
            "--dump-ram".to_string(),
            dump.to_str().expect("utf8").to_string(),
        ];
        let report = run_command(path.to_str().expect("utf8"), &flags).expect("runs");
        let first = fs::read(&dump).expect("dump written");
        // Same run again: the dump is byte-identical (determinism).
        let again = run_command(path.to_str().expect("utf8"), &flags).expect("runs");
        fs::remove_file(&path).ok();
        let second = fs::read(&dump).expect("dump written again");
        fs::remove_file(&dump).ok();
        // NROM with no battery and no NES 2.0 PRG-RAM size: RAM only.
        assert_eq!(first.len(), 0x800);
        assert_eq!(first, second, "deterministic RAM dump");
        assert_eq!(
            report.lines().count(),
            again.lines().count(),
            "identical reports"
        );
        assert!(report.contains("تفريغ الذاكرة"));
    }

    #[test]
    fn run_dump_video_writes_one_ppm_per_frame() {
        let path = temp_path("vid");
        fs::write(&path, nop_rom()).expect("write ROM");
        let dir = std::env::temp_dir().join("se-cli-dump-video-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create dir");
        let flags: [String; 4] = [
            "--frames".to_string(),
            "3".to_string(),
            "--dump-video".to_string(),
            dir.to_str().expect("utf8").to_string(),
        ];
        let report = run_command(path.to_str().expect("utf8"), &flags).expect("runs");
        fs::remove_file(&path).ok();
        let names: Vec<_> = fs::read_dir(&dir)
            .expect("dir readable")
            .filter_map(|entry| entry.ok().map(|e| e.file_name()))
            .collect();
        assert_eq!(names.len(), 3, "one file per frame");
        let first = fs::read(dir.join("f00000.ppm")).expect("frame 0");
        assert_eq!(&first[..3], b"P6\n");
        assert_eq!(first.len(), 15 + 256 * 240 * 3);
        assert!(report.contains("إطارات الفيديو: 3"));
        let _ = fs::remove_dir_all(&dir);
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
    fn run_rejects_unimplemented_mappers_with_arabic_error() {
        // Byte 6 high nibble carries the low mapper nibble: 0x40 =
        // mapper 4 (MMC3 territory, M5).
        let mut v = vec![
            b'N', b'E', b'S', 0x1A, 1, 1, 0x40, 0x00, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        v.extend(vec![0xEA; 16 * 1024]);
        v.extend(vec![0x00; 8 * 1024]);
        let path = temp_path("mapper");
        fs::write(&path, v).expect("write ROM");
        let flags = ["--frames".to_string(), "1".to_string()];
        let err = run_command(path.to_str().expect("utf8"), &flags).expect_err("mapper");
        fs::remove_file(&path).ok();
        let text = err.to_string();
        assert!(text.contains("اللوحة رقم 4"));
        assert!(text.contains("T1"));
    }

    #[test]
    fn run_rejects_invalid_board_sizes_with_arabic_error() {
        // Mapper 2 (UxROM) with a 16 KiB image: the Registry refuses
        // the undocumented size instead of guessing a layout.
        let mut v = vec![
            b'N', b'E', b'S', 0x1A, 1, 1, 0x20, 0x00, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        v.extend(vec![0xEA; 16 * 1024]);
        v.extend(vec![0x00; 8 * 1024]);
        let path = temp_path("mapper-size");
        fs::write(&path, v).expect("write ROM");
        let flags = ["--frames".to_string(), "1".to_string()];
        let err = run_command(path.to_str().expect("utf8"), &flags).expect_err("size");
        fs::remove_file(&path).ok();
        let text = err.to_string();
        assert!(text.contains("حجم PRG"));
        assert!(text.contains("لا نخمّن"));
    }

    #[test]
    fn run_accepts_the_minimal_mmc1_for_the_m2_gate() {
        // Mapper 1 with a 32 KiB image (two 16 KiB banks) executes
        // headless through the minimal MMC1 (D-22).
        let mut v = vec![
            b'N', b'E', b'S', 0x1A, 2, 0, 0x10, 0x00, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        let mut prg = vec![0xEA; 32 * 1024];
        prg[0..3].copy_from_slice(&[0x4C, 0x00, 0x80]);
        prg[0x3FFA..0x4000].copy_from_slice(&[0x00, 0x80, 0x00, 0x80, 0x00, 0x80]);
        v.extend(prg);
        let path = temp_path("mmc1");
        fs::write(&path, v).expect("write ROM");
        let flags = ["--frames".to_string(), "2".to_string()];
        let report = run_command(path.to_str().expect("utf8"), &flags).expect("runs");
        fs::remove_file(&path).ok();
        assert!(report.contains("الإطارات المكتملة: 2"));
        assert!(report.contains("بروتوكول blargg"));
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

    // ---- input script (M3) -------------------------------------------------

    #[test]
    fn input_script_parses_names_comments_and_dedups() {
        let script =
            InputScript::parse("# scenario\n\n0\n30 start\n30 left a\n60\n").expect("valid script");
        use se_core::{BTN_A, BTN_LEFT};
        // The later line at the same frame supersedes the earlier one.
        assert_eq!(
            script.schedule,
            vec![(0, 0), (30, BTN_LEFT | BTN_A), (60, 0)]
        );
    }

    #[test]
    fn input_script_rejects_unknown_buttons_in_arabic() {
        let err = InputScript::parse("10 turbo").expect_err("bad button");
        assert!(err.contains("اسم زر غير معروف: turbo"));
    }

    #[test]
    fn run_with_an_input_script_is_deterministic() {
        let path = temp_path("script");
        fs::write(&path, nop_rom()).expect("write ROM");
        let script_path = temp_path("script-input").with_extension("txt");
        fs::write(&script_path, "0\n10 start right\n20\n").expect("write script");
        let script_text = script_path.to_str().expect("utf8").to_string();
        let flags = [
            "--frames".to_string(),
            "3".to_string(),
            "--hash".to_string(),
            "--input-script".to_string(),
            script_text.clone(),
        ];
        let first = run_command(path.to_str().expect("utf8"), &flags).expect("runs");
        let second = run_command(path.to_str().expect("utf8"), &flags).expect("runs");
        fs::remove_file(&path).ok();
        fs::remove_file(&script_path).ok();
        assert!(first.contains("سيناريو الإدخال"));
        assert!(first.contains("أحداث: 3"));
        fn hash_line(report: &str) -> &str {
            report
                .lines()
                .find(|line| line.contains("بصمة الحالة"))
                .expect("hash line present")
        }
        assert_eq!(hash_line(&first), hash_line(&second));
    }

    #[test]
    fn run_reports_an_unreadable_script_in_arabic() {
        let path = temp_path("script-missing");
        fs::write(&path, nop_rom()).expect("write ROM");
        let flags = [
            "--frames".to_string(),
            "1".to_string(),
            "--input-script".to_string(),
            "/definitely/not/a/script.txt".to_string(),
        ];
        let err = run_command(path.to_str().expect("utf8"), &flags).expect_err("missing");
        fs::remove_file(&path).ok();
        assert!(err.to_string().contains("سيناريو الإدخال"));
    }

    #[test]
    fn script_help_prints_the_format() {
        let code = run(&["se-cli".to_string(), "script-help".to_string()]);
        assert_eq!(code, 0);
    }
}
