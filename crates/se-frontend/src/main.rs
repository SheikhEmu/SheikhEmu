//! The M3.3 minimal window frontend of SheikhEmu.
//!
//! Spec scope of this milestone (§5.5 + §5.5.1 + §7.3, D-26/D-27):
//!
//! - a window with the 256x240 picture at a correct **integer** scale
//!   (`scale.rs`), black letterbox bars around it,
//! - **region frame limiting** (§5.5 "vsync or frame-limit"): one
//!   emulated frame per real frame period of the ROM's region
//!   (`pacing.rs`), paced with `std::time::Instant` here in the host
//!   shell — the core itself never sees a wall clock or a thread
//!   (D-16/D-26),
//! - the §7.3 default keyboard control: arrows = D-pad, Z = A,
//!   X = B, Enter = Start, both Shifts = Select (`keys.rs`),
//! - **no sound** (that is M4's APU milestone), no pause menu, no
//!   fullscreen — the minimal honest window,
//! - everything on the single event-loop thread: one core thread, no
//!   worker threads, no locks (§5.5.1, D-26).
//!
//! The two external dependencies (`winit`, `softbuffer`) are the
//! D-27 owner-approved whitelist, used only by this crate.
//!
//! Window systems differ; the CI never opens a window (it builds and
//! runs the pure modules' tests headlessly). The window itself is
//! verified by the owner locally — see PROGRESS.md.

mod keys;
mod pacing;
mod scale;

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use se_core::{Chr, CpuError, Machine, Mirroring};
use se_rom::{parse, TimingRegion};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, OwnedDisplayHandle};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

use scale::placement;

/// The supported mapper set of M3 (the CLI's own gate, mirrored here
/// so a double-click on an unsupported board gives the same explicit
/// Arabic refusal instead of a black screen).
const M3_MAPPERS: [u16; 11] = [0, 1, 2, 3, 7, 11, 34, 66, 71, 87, 101];

/// How far behind the pacer is allowed to fall before it resyncs to
/// "one frame from now" instead of racing through a burst of frames
/// (a window drag or a slow disk can stall the loop for seconds).
const CATCH_UP_BUDGET: u32 = 3;

/// The running application: the emulated machine, the window state,
/// the keyboard bitmap, and the frame deadline.
struct Frontend {
    machine: Machine,
    /// Live button bitmap of player 1 (§7.3 default control).
    buttons: u8,
    /// The RGB staging picture (the PPU fills it each frame).
    picture: Box<[u8; 256 * 240 * 3]>,
    /// One emulated frame's real-time period (region pacing).
    period: Duration,
    /// The instant the next emulated frame is due.
    deadline: Instant,
    /// The window title (kept off the hot path).
    window_title: String,
    /// The softbuffer display context (from the event loop's owned
    /// display handle — the documented winit/softbuffer interop).
    context: softbuffer::Context<OwnedDisplayHandle>,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<OwnedDisplayHandle, Rc<Window>>>,
}

impl Frontend {
    /// Present the current picture into the window with integer
    /// scaling and letterboxing (the hot path: no allocation, no
    /// formatting, plain loops only).
    fn present(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let (width, height) = {
            let size = window.inner_size();
            (size.width, size.height)
        };
        if width == 0 || height == 0 {
            return;
        }
        if surface
            .resize(
                NonZeroU32::new(width).unwrap_or(NonZeroU32::MIN),
                NonZeroU32::new(height).unwrap_or(NonZeroU32::MIN),
            )
            .is_err()
        {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };
        let place = placement(width, height);
        // The letterbox bars (top, bottom, and both sides).
        for pixel in buffer.iter_mut() {
            *pixel = 0;
        }
        let stride = width;
        for row in 0..240u32 {
            let source = &self.picture[(row as usize) * 256 * 3..(row as usize + 1) * 256 * 3];
            let base_y = place.offset_y + row * place.scale;
            for repetition in 0..place.scale {
                let row_start = (base_y + repetition) * stride + place.offset_x;
                for (column, chunk) in source.as_chunks::<3>().0.iter().enumerate() {
                    // softbuffer's u32 pixel: 0x00RRGGBB.
                    let pixel = (u32::from(chunk[0]) << 16)
                        | (u32::from(chunk[1]) << 8)
                        | u32::from(chunk[2]);
                    for sub in 0..place.scale {
                        let target = row_start + (column as u32) * place.scale + sub;
                        buffer[target as usize] = pixel;
                    }
                }
            }
        }
        let _ = buffer.present();
    }

    /// Run one emulated frame, refresh the staging picture, present.
    fn run_one_frame(&mut self) -> Result<(), CpuError> {
        // The input snapshot lands before the frame runs (the same
        // frame-boundary contract as the CLI's input script).
        self.machine.set_controller_buttons(0, self.buttons);
        self.machine.run_frames(1)?;
        self.machine.ppu().last_frame_rgb(&mut self.picture);
        self.present();
        Ok(())
    }
}

impl ApplicationHandler for Frontend {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // A second Resumed on a live window (a macOS dock re-activate)
        // is a no-op; the surface is tied to this window only.
        if self.window.is_some() {
            return;
        }
        let window = Rc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title(self.window_title.clone())
                        .with_inner_size(LogicalSize::new(768.0, 720.0)),
                )
                .expect("window creation"),
        );
        let surface =
            softbuffer::Surface::new(&self.context, window.clone()).expect("softbuffer surface");
        self.window = Some(window);
        self.surface = Some(surface);
        self.run_one_frame().expect("first frame");
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(code),
                        state,
                        ..
                    },
                ..
            } => {
                if let Some(bit) = keys::key_to_bit(code) {
                    match state {
                        ElementState::Pressed => self.buttons |= bit,
                        ElementState::Released => self.buttons &= !bit,
                    }
                }
                if code == KeyCode::Escape {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(_) | WindowEvent::RedrawRequested => {
                // Re-present the current picture at the new size; the
                // emulator's own frames keep arriving from the pacer.
                self.present();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.deadline {
            if self.run_one_frame().is_err() {
                // A jammed CPU (JAM on a broken ROM): close honestly
                // instead of showing a frozen window.
                eprintln!("توقّف المعالج (روم معطوب؟) — تُغلق النافذة");
                event_loop.exit();
                return;
            }
            self.deadline += self.period;
            // Resync instead of bursting through a stall (drag, swap).
            if Instant::now() >= self.deadline + self.period * CATCH_UP_BUDGET {
                self.deadline = Instant::now() + self.period;
            }
        }
        // Sleep until the next frame is due; input events wake us early.
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.deadline));
    }
}

fn main() {
    // The Arabic usage text (D-06: user-facing messages are Arabic).
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("محاكي SheikhEmu — الواجهة الدنيا للمرحلة M3");
        eprintln!(
            "الاستخدام: {} <ملف الروم .nes>",
            args.first().map(String::as_str).unwrap_or("se-frontend")
        );
        std::process::exit(2);
    }
    let path = PathBuf::from(&args[1]);
    let data = match std::fs::read(&path) {
        Ok(data) => data,
        Err(_) => {
            eprintln!("تعذّر قراءة ملف الروم: {}", args[1]);
            std::process::exit(2);
        }
    };
    let rom = match parse(&data) {
        Ok(rom) => rom,
        Err(error) => {
            eprintln!("تعذّر تحليل الروم: {error}");
            std::process::exit(2);
        }
    };
    if !M3_MAPPERS.contains(&rom.mapper) {
        eprintln!(
            "اللوحة رقم {} غير مدعومة بعد — اللوحات المدعومة في M3: 0 و1 و2 و3 و7 و11 و34 و66 و71 و87 و101",
            rom.mapper
        );
        std::process::exit(3);
    }
    let region = match rom.timing {
        Some(TimingRegion::Pal) => se_core::Region::Pal,
        Some(TimingRegion::Dendy) => se_core::Region::Dendy,
        _ => se_core::Region::Ntsc,
    };
    let chr = if rom.chr.is_empty() {
        Chr::Ram(Box::new([0; 0x2000]))
    } else {
        Chr::Rom(rom.chr.clone())
    };
    let mirroring = match rom.mirroring {
        se_rom::Mirroring::Vertical => Mirroring::Vertical,
        se_rom::Mirroring::Horizontal => Mirroring::Horizontal,
        se_rom::Mirroring::FourScreen => Mirroring::FourScreen,
    };
    let prg_ram = rom.battery || rom.prg_ram.is_some_and(|bytes| bytes > 0) || rom.mapper == 1;
    let mut machine = match Machine::from_rom(
        rom.prg,
        rom.mapper,
        rom.submapper,
        chr,
        mirroring,
        prg_ram,
        region,
    ) {
        Ok(machine) => machine,
        Err(error) => {
            eprintln!("تعذّر تركيب الكارتريدج: {error}");
            std::process::exit(3);
        }
    };
    machine.reset();

    let event_loop = match EventLoop::new() {
        Ok(loop_) => loop_,
        Err(error) => {
            eprintln!("تعذّر فتح نافذة على هذا النظام: {error}");
            std::process::exit(4);
        }
    };
    // The softbuffer display context comes from the event loop itself
    // (the documented winit/softbuffer interop: owned display handle).
    let context =
        softbuffer::Context::new(event_loop.owned_display_handle()).expect("softbuffer context");
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "rom".to_string());
    let mut frontend = Frontend {
        machine,
        buttons: 0,
        picture: Box::new([0; 256 * 240 * 3]),
        period: pacing::frame_period(region),
        deadline: Instant::now(),
        window_title: format!("محاكي SheikhEmu — {file_name}"),
        context,
        window: None,
        surface: None,
    };
    if let Err(error) = event_loop.run_app(&mut frontend) {
        eprintln!("أُغلقت حلقة الأحداث بخطأ: {error}");
        std::process::exit(5);
    }
}
