//! `se-cli` — the Arabic-native command line interface of SheikhEmu.
//!
//! M0 provides one real command: `info <file>`, which parses an iNES /
//! NES 2.0 cartridge image and prints a full Arabic report, including
//! header warnings. Deliberately unimplemented commands (such as
//! actually running a ROM with frame hashing) are *not* advertised; they
//! arrive with the emulation milestones instead of being stubbed.

use std::env;
use std::fs;
use std::process::ExitCode;

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
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::FileUnreadable => {
                write!(f, "تعذر فتح الملف أو قراءته — تحقق من المسار")
            }
            CommandError::Parse(err) => write!(f, "{err}"),
        }
    }
}

impl From<RomError> for CommandError {
    fn from(err: RomError) -> Self {
        CommandError::Parse(err)
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
    println!("  se-cli -h | --help      هذه المساعدة");
}

/// Parse `path` and produce the Arabic cartridge report.
fn info_command(path: &str) -> Result<String, CommandError> {
    let data = fs::read(path).map_err(|_| CommandError::FileUnreadable)?;
    let rom = parse(&data)?;
    Ok(format_report(path, &rom))
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
