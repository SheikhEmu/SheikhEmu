//! Error and warning messages for cartridge parsing.
//!
//! Messages are in Arabic (owner-facing CLI language, decision D-06);
//! technical identifiers inside the messages stay in Latin script.

use core::fmt;

/// Fatal parsing errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RomError {
    /// The file is shorter than the fixed 16-byte iNES header.
    TooShort,
    /// The file does not start with the `NES<EOF>` signature.
    BadMagic,
    /// The PRG/CHR data present is shorter than the header announces.
    Truncated,
    /// A size field exceeds the maximum allowed by the specification.
    Oversized {
        /// Which area was oversized (`"PRG"` or `"CHR"`).
        area: &'static str,
        /// The announced size in bytes.
        announced: u64,
        /// The maximum allowed size in bytes.
        limit: u64,
    },
}

impl fmt::Display for RomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RomError::TooShort => write!(
                f,
                "الملف أقصر من ترويسة iNES القياسية (16 بايت) — لا يمكن قراءته"
            ),
            RomError::BadMagic => write!(
                f,
                "توقيع الملف غير صحيح: كل ملف NES يجب أن يبدأ بالبايتات \
                 NES ثم 1A — هذا الملف ليس ملف ROM صالحًا"
            ),
            RomError::Truncated => write!(
                f,
                "بيانات الكارتريدج ناقصة: الترويسة تعلن حجمًا أكبر من \
                 ما يحتويه الملف فعليًا"
            ),
            RomError::Oversized {
                area,
                announced,
                limit,
            } => write!(
                f,
                "حجم {area} المعلن ({announced} بايت) يتجاوز الحد الأقصى \
                 المسموح به في المواصفة ({limit} بايت)"
            ),
        }
    }
}

impl std::error::Error for RomError {}

/// Non-fatal header observations reported to the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Warning {
    /// Bytes 12-15 of an old iNES header are non-zero (DiskDude-era
    /// pollution); only the low mapper nibble is trusted.
    DiskDudeGarbage,
    /// NES 2.0: the battery bit and the PRG-NVRAM field disagree.
    BatteryMismatch {
        /// Whether the battery bit is set while no NVRAM is declared.
        battery_without_nvram: bool,
    },
    /// Old iNES header with zero CHR-ROM; the traditional assumption of
    /// 8 KiB of CHR-RAM will be used.
    ChrRamAssumed,
    /// Trailing bytes after PRG/CHR while the header declares no
    /// miscellaneous ROM area.
    MiscRomUndeclared {
        /// Number of unexplained trailing bytes.
        bytes: usize,
    },
    /// The console type is not a regular NES/Famicom (VS, Playchoice or
    /// an extended console type).
    NonStandardConsole,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Warning::DiskDudeGarbage => write!(
                f,
                "ترويسة iNES قديمة ملوثة (بايتات 12-15 غير صفرية، أسلوب \
                 DiskDude) — سيُعتمد رقم الـmapper من النبلة السفلى فقط"
            ),
            Warning::BatteryMismatch {
                battery_without_nvram,
            } => {
                if *battery_without_nvram {
                    write!(
                        f,
                        "بت البطارية مضبوط لكن حقل PRG-NVRAM في NES 2.0 \
                         يساوي صفرًا — تعارض مع المواصفة"
                    )
                } else {
                    write!(
                        f,
                        "حقل PRG-NVRAM في NES 2.0 يعلن ذاكرة غير متطايرة \
                         لكن بت البطارية غير مضبوط — تعارض مع المواصفة"
                    )
                }
            }
            Warning::ChrRamAssumed => write!(
                f,
                "ترويسة iNES بدون CHR-ROM — سيُفترض التقليد المتعارف عليه: \
                 8 كيلوبايت CHR-RAM"
            ),
            Warning::MiscRomUndeclared { bytes } => write!(
                f,
                "توجد {bytes} بايت زائدة بعد نهاية PRG/CHR بينما لا تعلن \
                 الترويسة أي منطقة ROM إضافية"
            ),
            Warning::NonStandardConsole => write!(
                f,
                "نوع الجهاز ليس NES/Famicom القياسي (VS / Playchoice / \
                 نوع موسّع) — لن يعمل هذا الكارتريدج على المضاهاة القياسية \
                 في M0"
            ),
        }
    }
}
