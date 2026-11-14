# السياسة القانونية والامتثال لمشروع SheikhEmu (LEGAL POLICY)

> **مصدر حرفي:** هذا الملف نقلٌ حرفي للأقسام §11 و §12 من ملف المواصفة
> المرفوع من المالك `SHEIKHEMU_PROMPT 3.md` (وصل بتاريخ 2026-11-12
> وأُغلق به B-04) مع القوائم النهائية كما وردت.

---

## 11. المنصات والتوزيع والتكلفة

### 11.1 ما يُبنى وأين يُنشر
| القناة | الشكل | التكلفة | ملاحظة |
|---|---|---|---|
| GitHub Releases | APK لكل ABI + zip ويندوز + tar لينكس + mac zip + html | **0** | القناة الأساسية، بلا حدود تنزيل، 2 GB/ملف |
| GitHub Pages | صفحة تنزيل + نسخة ويب | **0** | سياسة الخصوصية هنا أيضاً |
| IzzyOnDroid / F‑Droid | APK | **0** | يشترطون مفتوح المصدر وقابلية البناء |
| AppGallery | APK أو AAB | **0** | تحقق هوية 1–2 يوم، مراجعة 1–3 أيام |
| Google Play | AAB | 25$ مرة واحدة | targetSdk 36، 16KB، Data safety، 12 مختبِراً×14 يوماً للحساب الشخصي الجديد |
| App Store | ipa | **99$/سنة** | Xcode 26+/iOS 26 SDK (منذ أبريل 2026)، iOS 27 SDK من أبريل 2027، جهاز Mac، بلا JIT (لا مشكلة لـ6502)، §4.7 تسمح محاكيات الأجهزة القديمة |
| HarmonyOS NEXT | HAP | 0 لكن مشروع منفصل | **لا يشغّل APK** — خارج النطاق (M11 إن طُلب) |

### 11.2 بناء APK وتوقيعه
`cargo-ndk` + Gradle + NDK **في CI فقط**. `minSdk 24`، `targetSdk 36`، `extractNativeLibs=false`، ABI splits، **صفر صلاحيات** (SAF فقط، بلا `INTERNET`)، intent filters لفتح `.nes/.fds/.unf/.nsf/.zip`. مفاتيح التوقيع في GitHub Secrets بترميز base64، **ولا شيء منها في المستودع**، ومعمارياتها: `arm64-v8a` أساسي + `armeabi-v7a` + `x86_64` للمحاكيات. على كل وسم: ملفات بأسماء واضحة + `SHA256SUMS.txt` + أحجام بالبايت في `RELEASE_NOTES.md`.

### 11.3 مواد المتاجر
لقطات **من واجهة التطبيق فقط** وبألعاب منزلية أو PNG مُولّد — **ممنوع لقطة من لعبة تجارية**. أيقونات 1024/512/216 حسب المنصة، وصف عربي+إنجليزي، سياسة خصوصية منشورة، استبيان تصنيف عمري، وموضع بارز: «لا يحتوي التطبيق على أي ألعاب؛ أنت تستورد ملفاتك التي تملك الحق في استخدامها». تحقّق من متطلبات التصنيف المحلي في السعودية/الخليج قبل الطرح وسجّلها — لا تفترضها.

### 11.4 مسار بلا مال (افتراضي المشروع)
الافتراض: **يصل المشروع للمستخدم بتكلفة 0**. الويب/PWA أولاً (يعمل على iPhone بلا متجر)، ثم APK على GitHub، ثم AppGallery، ثم IzzyOnDroid. **لا تُدخل في التصميم أي خدمة مدفوعة** (لا Firebase مدفوع، لا crash reporting مدفوع، لا CDN). لا تبرّع داخل التطبيق ولا ميزة محجوبة. عند توفر المال: Play (25$) قبل App Store.
**بوابة مالية آلية في CI:** `grep` يفشل عند `admob|ads|billing|purchase|subscription|analytics|firebase|sentry|crashlytics|api_key|license_key|trial|paywall`، وفحص `AndroidManifest` يفشل عند `INTERNET` أو صلاحيات تخزين، و`docs/COST_CHECK.md` يُحدَّث قبل كل إصدار.

---

## 12. الامتثال والنسب (قانوني + جدار العزل)

> هذا القسم مبني على فشل موثّق في مشروع محاكي NES مفتوح المصدر (RustyNES): نُقل كود من Mesen2 وpuNES وFCEUX رغم تعليمات استخدامها كصناديق سوداء، ثم أُعيدت صياغة التعليقات الصادقة («منقول من Mesen2») إلى «مقارنة بالمخرج». **النية لم تفشل؛ الفيزياء سمحت بالوصول.** المصدر: `doublegate/RustyNES/docs/{ai-emulator-provenance-guardrails, provenance-failure-postmortem}.md`.

### 12.1 ما هو مسموح وما هو ممنوع
| البند | القرار |
|---|---|
| كتابة محاكٍ للعتاد من الصفر | مسموح عملياً (سوابق أمريكية: Connectix/Bleem!) — **ليست قاعدة عالمية، ولا تحمي ROMs/BIOS، ولا تُغني عن مشورة محلية** |
| ROM أو BIOS أو FDS BIOS في المستودع/الاختبارات/الإصدار | **ممنوع تماماً** |
| روابط ألعاب أو مواقع قرصنة | **ممنوع** في الكود أو README أو مواد المتجر |
| أصول/أصوات/خطوط/لقطات نينتندو أو ألعاب تجارية | **ممنوع** |
| «Nintendo/Famicom/NES» كاسم أو أيقونة أو نطاق | **ممنوع** كعلامة؛ يجوز وصفياً: «يشغّل ملفات `.nes`» |
| **فتح/قراءة كود محاكٍ مرجعي** | **ممنوع** — لا للفهم، ولا لثابت واحد، ولا مرة |
| تشغيل محاكٍ مرجعي وقياس مخرجه (oracle) | **مسموح** |
| نقل مكتبة بوعي (فئة D) | مسموح برخصة متوافقة + إسناد على أربعة أسطح |

### 12.2 تصنيف المدخلات
**A** توثيق عتاد ⇒ نفّذ بحرية · **B** رومات اختبار ⇒ شغّلها، وأودِع فقط المرخّص (فهرس رخصة لكل ملف) · **C** محاكيات ⇒ oracle بالمخرج فقط · **D** مكوّن منقول ⇒ رخصة متوافقة + إسناد.
**الخط الذي يُتجاوز دائماً:** استخدام C كأنه A. اللحظة التي يُعلّم فيها كود المرجع كودك، صار D.

### 12.3 الجدار (فيزيائي لا نصي)
1. صفر كود مرجعي في شجرة العمل (لا `refs/`، لا `vendor/emulators/`).
2. إن لزم للبناء: يوضع **خارج المشروع وخارج مسارات الوكيل**.
3. التفاعل **I/O فقط** (إطار PNG، trace، تفريغ صوتي، سجل مسجّلات).
4. **الأفضل: golden vectors** تُلتقط مرة واحدة بيد بشرية في `tests/golden/` ثم لا يلمس الوكيل المرجع.
5. افرض الجدار في **إعداد صلاحيات الأدوات** (deny‑list/sandbox) لا في النص فقط.

**تلوّث بيانات التدريب (Training contamination) — ثغرة لا يسدّها الجدار وحده:**
الجدار يمنع *فتح الملفات*، ولا يمنع أن «يتذكّر» النموذج بنية أو ثوابت من مشروع مفتوح المصدر رآه في تدريبه. لذلك:
1. **كل كتلة منطق غير بديهية** (جدول، ترتيب عمليات، معالجة حدّية) يجب أن تُشتق من **فقرة توثيق محددة** تُذكر في تعليق: `// nesdev: PPU_scrolling (fine X/Y)`.
2. أي كتلة لا مرجع توثيقي لها تُوسم `// UNVERIFIED-DESIGN: reason` وتدخل قائمة مراجعة بشرية إلزامية قبل النشر.
3. `tools/constant_sniff.sh` يوسّع ليكشف **البنية** لا الجداول فقط: تسلسل دوال متطابق، أسماء متطابقة، ترتيب فروع غير معتاد ⇒ بند مراجعة.
4. عند التوثيق الناقص: اشتقّ من مبدأ العتاد، واكتب «لم أجد مرجعاً لهذا التفصيل» في `DECISIONS.md` — ولا تكتبه بثقة.

**بروتوكول «blocker الـ oracle» (مهم — يمنع الالتفاف):**
إذا وجدت نفسك عالقاً في سلوك لا يُحسم من التوثيق ولا من golden vectors المتوفرة:
1. **لا تفتح الكود** — ولو لسطر واحد، ولو «للتأكد».
2. اكتب في `BLOCKERS.md`: السلوك المطلوب · ما جرّبته · ما يظهر في golden vectors · الفرق المتوقع.
3. أكمل بقية العمل (B1) واترك هذا السلوك مؤجلاً مع تصنيفه في `docs/PRECISION_BACKLOG.md`.
4. **اطلب من المالك** بجملة واحدة: «شغّل المحاكي الفلاني على هذا الاختبار وسلّمني المخرج (لقطة/سجل)» — المالك ينفّذ يدوياً، وأنت تقارن.
5. إن لم يتوفر، ابقَ على السلوك الموثّق وسجّل «غير مؤكد» — **لا تخترع ولا تقرأ الكود**. الالتفاف هنا يُبطل المشروع كله، لا ميزة واحدة.

### 12.4 الإسناد إن اشتُقّ كود (أربعة أسطح متسقة)
`// Provenance: derived from <Project>'s <Function> (<file>), <SPDX>` في الموضع · وسم SPDX أعلى الملف · صف في `docs/PROVENANCE.md` · سطر في `NOTICE`. **وبلا إفراط:** «يطابق سلوك X» مقارنة oracle وليست اشتقاقاً.

### 12.5 حساب الرخصة
`GPL-2.0-only` ≠ `GPL-2.0-or-later` (اقرأ الترويسات). اشتقاق من GPLv3 ⇒ المشروع كله `GPL-3.0-or-later`. GPLv2‑only + GPLv3 = عدم توافق لا يُحل بإعادة التسمية. **الترخيص الافتراضي MIT** ويبقى صالحاً فقط ما دام §12.3 قائماً؛ أول اشتقاق يوقف النشر حتى إعادة الترخيص.

### 12.6 الفحص الآلي (في CI من M0)
```bash
git ls-files | grep -Ei 'reference-?emulators?|vendor/emulators/|/(mesen|mesen2|bsnes|higan|ares|fceux|nestopia|punes|blastem|sameboy|mame)/' && exit 1
git grep -nEi 'ref-proj/|reference-oracles/|vendor/emulators/' -- '*.rs' '*.toml' && exit 1
tools/provenance_check.sh    # تعليق "derived" ⇔ صف في PROVENANCE.md ⇔ SPDX
tools/constant_sniff.sh      # جداول/ثوابت غريبة ⇒ بند مراجعة
cargo deny check licenses    # قائمة سماح صريحة
```
+ سؤال PR إلزامي بنعم/لا: «هل استند هذا الكود إلى **مصدر** محاكٍ مرجعي؟» + **مراجعة بشرية** قبل كل إصدار مُسجّلة في `docs/PROVENANCE_REVIEW.md`. الوكيل لا يُصادق على نفسه.

### 12.7 النصوص والسياسات الإلزامية
`LEGAL.md` (نص «لا يوزّع ألعاباً ولا BIOS؛ أنت مسؤول عن ملفات تملك الحق فيها؛ العلامات لأصحابها») · `TAKEDOWN.md` (إزالة خلال 72 ساعة + بريد) · قسم «ما لا يفعله الشيخ إيمو» في README · `docs/LEGAL_LOG.md` عند أي شكوى · وجهّز **اسماً بديلاً مسبقاً** (`Sheikh8`) حتى لا يتوقف المشروع.

### 12.8 علاج الانزلاق إن حدث
لا تُنظّف التعليقات ولا تُعيد تسميتها (**هذا أسوأ من النقل نفسه**) ⇒ احصر المدى ⇒ أعد الترخيص ⇒ أسند على أربعة أسطح ⇒ اكتب ADR + post‑mortem ⇒ ثبّت الجدار.

### 12.9 الخصوصية
صفر صلاحيات، `INTERNET` غير معلن، صفر تحليلات، معالجة محلية 100% في الويب، توقيع ثابت + بصمات SHA‑256، وعامل أي ملف ROM كمدخل غير موثوق (بلا `unwrap()`، فحص حدود، رسائل خطأ عربية واضحة).

---

## 13. سجل شجرة التبعيات الخارجية — تدقيق M4.3 (D-40، 2026-11-14)

> **المصدر:** هذه الإضافة توثّق تدقيق شجرة `cpal` الكامل قبل أي كود
> صوتي (شرط المالك الحرفي في اعتماد M4.3، وبنود D-37 رقم 5–7).
> الطريقة: `cargo tree -p se-frontend -e normal,build` لكل من
> `x86_64-unknown-linux-gnu` و`x86_64-pc-windows-msvc` و
> `aarch64-apple-darwin` (منصات CI الثلاث)، ثم مطابقة كل زوج
> (اسم، إصدار) مع حقل `license` في `cargo metadata`. الجدول أدناه هو
> **الاتحاد الكامل** للتبعيات الخارجية للواجهة بعد تفعيل cpal —
> 132 زوجًا، منها 24 زوجًا جديدًا دخلت مع cpal والباقي خطوط winit
> وsoftbuffer القائمة من D-27 (كثير منها مشترك بين الشجرتين).

### 13.1 النتيجة الحاكمة

- **صفر تراخيص فيروسية أو تشاركية** في الشجرة كلها: لا عائلة GPL
  (GPL/LGPL/AGPL)، ولا SSPL، ولا CC-BY-SA، ولا CDDL/MPL/EUPL،
  ولا Sleepycat — تحقق آلي بكلمات مفتاحية على كل تعبير ترخيص في
  الاتحاد، لا انتقاء يدوي.
- كل التعبيرات تحلّ مقابل قائمة allow القائمة بلا **أي إضافة
  جديدة**: عائلات MIT / Apache-2.0 / BSD-2-Clause / BSD-3-Clause /
  Zlib / ISC / Unlicense (فروع OR مع MIT تكفي) / Unicode-3.0 /
  Apache-2.0 WITH LLVM-exception (فرع OR مع Apache-2.0/MIT يكفي).
- `cargo deny check` أخضر بالأربعة (advisories/bans/licenses/sources)
> بعد إدخال cpal، وطلب `bans` بند skip واحد جديد فقط بسبب مكتوب
> (block2@0.5.1 — خط objc2-foundation 0.2 القديم من winit مقابل
> خط 0.6 من cpal؛ انظر deny.toml).
- الإصدار المقفل في `Cargo.lock`: **cpal 0.18.2** (مع alsa 0.11.0
  وalsa-sys 0.4.0 على لينكس، وwindows 0.62.2 على ويندوز، و
  coreaudio-rs 0.14.2 مع خط objc2 0.6 على macOS). صفر تغيير إصدار
  لأي crate قائم عند إدخال cpal (القفل أضاف فقط).

### 13.2 الجدول الكامل (الاتحاد عبر المنصات الثلاث)
<!-- 132 rows -->
| ab_glyph | 0.2.32 | لينكس | Apache-2.0 |
| ab_glyph_rasterizer | 0.1.10 | لينكس | Apache-2.0 |
| ahash | 0.8.12 | لينكس | MIT OR Apache-2.0 |
| alsa | 0.11.0 | لينكس | Apache-2.0/MIT |
| alsa-sys | 0.4.0 | لينكس | MIT |
| arrayref | 0.3.9 | لينكس | BSD-2-Clause |
| arrayvec | 0.7.8 | لينكس | MIT OR Apache-2.0 |
| as-raw-xcb-connection | 1.0.1 | لينكس | MIT OR Apache-2.0 |
| bitflags | 1.3.2 | macOS | MIT/Apache-2.0 |
| bitflags | 2.13.2 | لينكس، ويندوز، macOS | MIT OR Apache-2.0 |
| block2 | 0.5.1 | macOS | MIT |
| block2 | 0.6.2 | macOS | MIT |
| bytemuck | 1.25.2 | لينكس | Zlib OR Apache-2.0 OR MIT |
| bytemuck_derive | 1.12.1 | لينكس | Zlib OR Apache-2.0 OR MIT |
| calloop | 0.13.0 | لينكس | MIT |
| calloop-wayland-source | 0.3.0 | لينكس | MIT |
| cc | 1.11.1 | لينكس | MIT OR Apache-2.0 |
| cfg-if | 1.0.5 | لينكس | MIT OR Apache-2.0 |
| cfg_aliases | 0.2.2 | لينكس، ويندوز، macOS | MIT |
| core-foundation | 0.9.4 | macOS | MIT OR Apache-2.0 |
| core-foundation-sys | 0.8.7 | macOS | MIT OR Apache-2.0 |
| core-graphics | 0.23.2 | macOS | MIT OR Apache-2.0 |
| core-graphics-types | 0.1.3 | macOS | MIT OR Apache-2.0 |
| coreaudio-rs | 0.14.2 | macOS | MIT/Apache-2.0 |
| cpal | 0.18.2 | لينكس، ويندوز، macOS | Apache-2.0 |
| ctor | 0.10.1 | لينكس | Apache-2.0 OR MIT |
| cursor-icon | 1.2.0 | لينكس، ويندوز، macOS | MIT OR Apache-2.0 OR Zlib |
| dasp_sample | 0.11.0 | لينكس، ويندوز، macOS | MIT OR Apache-2.0 |
| dispatch | 0.2.0 | macOS | MIT |
| dispatch2 | 0.3.1 | macOS | Zlib OR Apache-2.0 OR MIT |
| dlib | 0.5.3 | لينكس | MIT |
| downcast-rs | 1.2.1 | لينكس | MIT/Apache-2.0 |
| dpi | 0.1.2 | لينكس، ويندوز، macOS | Apache-2.0 AND MIT |
| drm | 0.14.1 | لينكس | MIT |
| drm-ffi | 0.9.1 | لينكس | MIT |
| drm-fourcc | 2.2.0 | لينكس | MIT |
| drm-sys | 0.8.1 | لينكس | MIT |
| fastrand | 2.5.0 | لينكس | Apache-2.0 OR MIT |
| find-msvc-tools | 0.1.15 | لينكس | MIT OR Apache-2.0 |
| foreign-types | 0.5.0 | macOS | MIT/Apache-2.0 |
| foreign-types-macros | 0.2.4 | macOS | MIT/Apache-2.0 |
| foreign-types-shared | 0.3.1 | macOS | MIT/Apache-2.0 |
| gethostname | 1.1.0 | لينكس | Apache-2.0 |
| getrandom | 0.3.4 | لينكس | MIT OR Apache-2.0 |
| libc | 0.2.191 | لينكس، macOS | MIT OR Apache-2.0 |
| libloading | 0.8.9 | لينكس | ISC |
| linux-raw-sys | 0.12.1 | لينكس | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| linux-raw-sys | 0.4.15 | لينكس | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| linux-raw-sys | 0.9.4 | لينكس | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| log | 0.4.35 | لينكس | MIT OR Apache-2.0 |
| mach2 | 0.6.0 | macOS | BSD-2-Clause OR MIT OR Apache-2.0 |
| memchr | 2.8.3 | لينكس | Unlicense OR MIT |
| memmap2 | 0.9.12 | لينكس | MIT OR Apache-2.0 |
| objc-sys | 0.3.5 | macOS | MIT |
| objc2 | 0.5.3 | macOS | MIT |
| objc2 | 0.6.5 | macOS | MIT |
| objc2-app-kit | 0.2.2 | macOS | MIT |
| objc2-audio-toolbox | 0.3.2 | macOS | Zlib OR Apache-2.0 OR MIT |
| objc2-core-audio | 0.3.2 | macOS | Zlib OR Apache-2.0 OR MIT |
| objc2-core-audio-types | 0.3.2 | macOS | Zlib OR Apache-2.0 OR MIT |
| objc2-core-foundation | 0.3.2 | macOS | Zlib OR Apache-2.0 OR MIT |
| objc2-core-graphics | 0.3.2 | macOS | Zlib OR Apache-2.0 OR MIT |
| objc2-encode | 4.1.0 | macOS | MIT |
| objc2-foundation | 0.2.2 | macOS | MIT |
| objc2-foundation | 0.3.2 | macOS | MIT |
| objc2-quartz-core | 0.3.2 | macOS | Zlib OR Apache-2.0 OR MIT |
| once_cell | 1.21.4 | لينكس | MIT OR Apache-2.0 |
| owned_ttf_parser | 0.25.1 | لينكس | Apache-2.0 |
| percent-encoding | 2.3.2 | لينكس | MIT OR Apache-2.0 |
| pin-project-lite | 0.2.17 | لينكس، ويندوز، macOS | Apache-2.0 OR MIT |
| pkg-config | 0.3.34 | لينكس | MIT OR Apache-2.0 |
| polling | 3.11.0 | لينكس | Apache-2.0 OR MIT |
| proc-macro2 | 1.0.107 | لينكس، ويندوز، macOS | MIT OR Apache-2.0 |
| quick-xml | 0.41.0 | لينكس | MIT |
| quote | 1.0.47 | لينكس، ويندوز، macOS | MIT OR Apache-2.0 |
| raw-window-handle | 0.6.2 | لينكس، ويندوز، macOS | MIT OR Apache-2.0 OR Zlib |
| rustix | 0.38.44 | لينكس | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| rustix | 1.1.6 | لينكس | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| scoped-tls | 1.0.1 | لينكس | MIT/Apache-2.0 |
| sctk-adwaita | 0.10.1 | لينكس | MIT |
| shlex | 2.0.1 | لينكس | MIT OR Apache-2.0 |
| slab | 0.4.12 | لينكس | MIT |
| smallvec | 1.16.5 | لينكس | MIT OR Apache-2.0 |
| smithay-client-toolkit | 0.19.2 | لينكس | MIT |
| smol_str | 0.2.2 | لينكس، ويندوز، macOS | MIT OR Apache-2.0 |
| softbuffer | 0.4.8 | لينكس، ويندوز، macOS | MIT OR Apache-2.0 |
| strict-num | 0.1.1 | لينكس | MIT |
| syn | 2.0.119 | لينكس، ويندوز | MIT OR Apache-2.0 |
| syn | 3.0.10 | لينكس، macOS | MIT OR Apache-2.0 |
| thiserror | 1.0.69 | لينكس | MIT OR Apache-2.0 |
| thiserror-impl | 1.0.69 | لينكس | MIT OR Apache-2.0 |
| tiny-skia | 0.11.4 | لينكس | BSD-3-Clause |
| tiny-skia-path | 0.11.4 | لينكس | BSD-3-Clause |
| tiny-xlib | 0.2.5 | لينكس | MIT OR Apache-2.0 OR Zlib |
| tracing | 0.1.44 | لينكس، ويندوز، macOS | MIT |
| tracing-core | 0.1.36 | لينكس، ويندوز، macOS | MIT |
| ttf-parser | 0.25.1 | لينكس | MIT OR Apache-2.0 |
| unicode-ident | 1.0.27 | لينكس، ويندوز، macOS | (MIT OR Apache-2.0) AND Unicode-3.0 |
| unicode-segmentation | 1.13.3 | ويندوز | MIT OR Apache-2.0 |
| version_check | 0.9.5 | لينكس | MIT/Apache-2.0 |
| wayland-backend | 0.3.17 | لينكس | MIT |
| wayland-client | 0.31.15 | لينكس | MIT |
| wayland-csd-frame | 0.3.0 | لينكس | MIT |
| wayland-cursor | 0.31.14 | لينكس | MIT |
| wayland-protocols | 0.32.13 | لينكس | MIT |
| wayland-protocols-plasma | 0.3.12 | لينكس | MIT |
| wayland-protocols-wlr | 0.3.12 | لينكس | MIT |
| wayland-scanner | 0.31.11 | لينكس | MIT |
| wayland-sys | 0.31.11 | لينكس | MIT |
| windows | 0.62.2 | ويندوز | MIT OR Apache-2.0 |
| windows-collections | 0.3.2 | ويندوز | MIT OR Apache-2.0 |
| windows-core | 0.62.2 | ويندوز | MIT OR Apache-2.0 |
| windows-future | 0.3.2 | ويندوز | MIT OR Apache-2.0 |
| windows-implement | 0.60.2 | ويندوز | MIT OR Apache-2.0 |
| windows-interface | 0.59.3 | ويندوز | MIT OR Apache-2.0 |
| windows-link | 0.2.1 | ويندوز | MIT OR Apache-2.0 |
| windows-numerics | 0.3.1 | ويندوز | MIT OR Apache-2.0 |
| windows-result | 0.4.1 | ويندوز | MIT OR Apache-2.0 |
| windows-strings | 0.5.1 | ويندوز | MIT OR Apache-2.0 |
| windows-sys | 0.52.0 | ويندوز | MIT OR Apache-2.0 |
| windows-sys | 0.61.2 | ويندوز | MIT OR Apache-2.0 |
| windows-targets | 0.52.6 | ويندوز | MIT OR Apache-2.0 |
| windows-threading | 0.2.1 | ويندوز | MIT OR Apache-2.0 |
| windows_x86_64_msvc | 0.52.6 | ويندوز | MIT OR Apache-2.0 |
| winit | 0.30.13 | لينكس، ويندوز، macOS | Apache-2.0 |
| x11-dl | 2.21.0 | لينكس | MIT |
| x11rb | 0.13.2 | لينكس | MIT OR Apache-2.0 |
| x11rb-protocol | 0.13.2 | لينكس | MIT OR Apache-2.0 |
| xcursor | 0.3.11 | لينكس | MIT |
| xkbcommon-dl | 0.4.2 | لينكس | MIT |
| xkeysym | 0.2.1 | لينكس | MIT OR Apache-2.0 OR Zlib |
| zerocopy | 0.8.63 | لينكس | BSD-2-Clause OR Apache-2.0 OR MIT |

الأعمدة: crate / الإصدار المقفل / المنصات التي يظهر فيها / تعبير
الترخيص كما ورد في بيانات السجل. الترتيب أبجدي. أي ترخيص مستقبلي
خارج هذه العائلات المتساهلة = توقف فوري وإبلاغ المالك (شرط M4.3).

### 13.3 ملاحظات خصوصية

- الشجرة كلها مكتبات صوت/نافذة/نظام معيارية؛ لا توجد أي حزمة
  شبكية أو تحليلات أو تتبّع (بوابة cost تظل تراقب المصدر أيضًا).
- `alsa-sys` يربط ضد `libasound` فقط (لا كود منقول — صناديق سوداء
  عبر واجهاتها العامة، تصنيف المكوّن D في §12.2 لا ينطبق لأن لا
  كود منها نُسخ إلى المستودع).
