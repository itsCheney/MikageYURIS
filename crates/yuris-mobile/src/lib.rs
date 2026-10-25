//! The C interface for app hosts: `platform/ios/include/yuris_mobile.h`.
//!
//! A session is a [`Player`] plus a wgpu surface on the host's view and a
//! [`Renderer`] for it. The host steps it once per display frame; the step
//! runs the game's frame and draws it when the layers changed. Sound is
//! pulled by the host from its audio callback through a [`YurisAudio`].
//!
//! Every entry point catches panics and reports them as errors.

use std::cell::RefCell;
use std::ffi::{c_char, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
#[cfg(target_vendor = "apple")]
use std::ptr::NonNull;
use std::sync::{Mutex, Once};

use yuris_host::audio::OUTPUT_RATE;

mod audio;
use yuris_player::{key_from_vk, Config, Encoding, FontFile, Player, Status};
use yuris_render::{fit, Renderer};
use yuris_vm::MouseButton;

/// `YURIS_ABI_VERSION`.
pub const ABI_VERSION: u32 = 1;

/// `YurisConfig`.
#[repr(C)]
pub struct YurisConfig {
    pub struct_size: u32,
    pub game_dir: *const c_char,
    pub launcher: *const c_char,
    pub save_dir: *const c_char,
    pub encoding: u32,
    pub utc_offset_ms: i64,
}

/// The GPU side of a session, made on the first surface.
struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    renderer: Renderer,
    surface: Option<wgpu::Surface<'static>>,
    config: Option<wgpu::SurfaceConfiguration>,
    /// The view the surface was made for.
    view: *mut c_void,
    /// Layer tree generation last drawn; None after a (re)configure.
    drawn: Option<u64>,
}

/// `YurisSession`.
pub struct YurisSession {
    player: Player,
    gpu: Option<Gpu>,
    /// Surface size in pixels, for mapping the pointer.
    surface_size: (u32, u32),
}

/// `YurisAudio`: the sound output, fed from the session's mixer.
pub struct YurisAudio(audio::AudioOutput);

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn set_error(msg: impl Into<String>) {
    let msg = msg.into().replace('\0', " ");
    tracing::error!("{msg}");
    LAST_ERROR.with(|e| *e.borrow_mut() = CString::new(msg).unwrap_or_default());
}

fn clear_error() {
    LAST_ERROR.with(|e| *e.borrow_mut() = CString::default());
}

/// Runs `f`, turning a panic into an error and `fallback`.
fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(v) => v,
        Err(p) => {
            let msg = p
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| p.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown".into());
            set_error(format!("panic: {msg}"));
            fallback
        }
    }
}

/// A borrowed C string, None for NULL or empty.
///
/// # Safety
/// `p` is NULL or a NUL-terminated string.
unsafe fn string(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let s = CStr::from_ptr(p).to_string_lossy().into_owned();
    (!s.is_empty()).then_some(s)
}

#[no_mangle]
pub extern "C" fn yuris_abi_version() -> u32 {
    ABI_VERSION
}

#[no_mangle]
pub extern "C" fn yuris_last_error() -> *const c_char {
    LAST_ERROR.with(|e| e.borrow().as_ptr())
}

// ---- logging ----

type LogCallback = extern "C" fn(level: i32, target: *const c_char, message: *const c_char, context: *mut c_void);

struct LogSink {
    callback: LogCallback,
    context: usize,
    max_level: i32,
}

static LOG: Mutex<Option<LogSink>> = Mutex::new(None);
static LOG_INIT: Once = Once::new();

struct CallbackLayer;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CallbackLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let level = match *event.metadata().level() {
            tracing::Level::ERROR => 1,
            tracing::Level::WARN => 2,
            tracing::Level::INFO => 3,
            tracing::Level::DEBUG => 4,
            tracing::Level::TRACE => 5,
        };
        // Copied out so the host is called without the lock held: a
        // callback that calls back into the runtime must not deadlock.
        let (callback, context) = match LOG.lock().ok().as_deref() {
            Some(Some(sink)) if level <= sink.max_level => (sink.callback, sink.context),
            _ => return,
        };
        struct Visit(String);
        impl tracing::field::Visit for Visit {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if !self.0.is_empty() {
                    self.0.push(' ');
                }
                if field.name() == "message" {
                    self.0.push_str(&format!("{value:?}"));
                } else {
                    self.0.push_str(&format!("{}={value:?}", field.name()));
                }
            }
        }
        let mut v = Visit(String::new());
        event.record(&mut v);
        let target = CString::new(event.metadata().target()).unwrap_or_default();
        let msg = CString::new(v.0.replace('\0', " ")).unwrap_or_default();
        callback(level, target.as_ptr(), msg.as_ptr(), context as *mut c_void);
    }
}

#[no_mangle]
pub extern "C" fn yuris_set_log_callback(callback: Option<LogCallback>, context: *mut c_void, max_level: i32) {
    guard((), || {
        if let Ok(mut sink) = LOG.lock() {
            *sink = callback.map(|callback| LogSink { callback, context: context as usize, max_level });
        }
        LOG_INIT.call_once(|| {
            use tracing_subscriber::prelude::*;
            // wgpu logs through `log`; only our crates' records are wanted.
            let filter = tracing_subscriber::filter::filter_fn(|m| m.target().starts_with("yuris"));
            let _ = tracing_subscriber::registry().with(CallbackLayer.with_filter(filter)).try_init();
        });
    })
}

// ---- fonts ----

/// Fonts for the sessions opened from now on.
static FONTS: Mutex<Vec<FontFile>> = Mutex::new(Vec::new());

/// # Safety
/// `path` and `face` are NULL or NUL-terminated strings.
#[no_mangle]
pub unsafe extern "C" fn yuris_register_font(path: *const c_char, face: *const c_char) -> i32 {
    guard(0, || {
        let Some(path) = string(path) else { return 0 };
        if !std::path::Path::new(&path).is_file() {
            set_error(format!("no font file {path}"));
            return 0;
        }
        if let Ok(mut fonts) = FONTS.lock() {
            fonts.push(FontFile { path: PathBuf::from(path), index: 0, name: string(face) });
        }
        1
    })
}

#[no_mangle]
pub extern "C" fn yuris_clear_fonts() {
    guard((), || {
        if let Ok(mut fonts) = FONTS.lock() {
            fonts.clear();
        }
    })
}

// ---- sessions ----

/// # Safety
/// `config` points to a valid `YurisConfig` whose strings and arrays are
/// valid for the call.
#[no_mangle]
pub unsafe extern "C" fn yuris_open(config: *const YurisConfig) -> *mut YurisSession {
    guard(std::ptr::null_mut(), || {
        clear_error();
        let Some(c) = config.as_ref() else {
            set_error("no config");
            return std::ptr::null_mut();
        };
        if (c.struct_size as usize) < std::mem::size_of::<YurisConfig>() {
            set_error("config struct_size too small");
            return std::ptr::null_mut();
        }
        let Some(game_dir) = string(c.game_dir) else {
            set_error("no game folder");
            return std::ptr::null_mut();
        };
        let fonts = FONTS.lock().map(|f| f.clone()).unwrap_or_default();
        let cfg = Config {
            game_dir: PathBuf::from(game_dir),
            launcher: string(c.launcher).map(PathBuf::from),
            save_dir: string(c.save_dir).map(PathBuf::from),
            fonts,
            encoding: match c.encoding {
                1 => Encoding::Gbk,
                2 => Encoding::Utf8,
                _ => Encoding::ShiftJis,
            },
            utc_offset_ms: c.utc_offset_ms,
        };
        match Player::open(&cfg) {
            Ok(player) => {
                let (w, h) = player.screen();
                tracing::info!("opened {} ({w}x{h})", cfg.game_dir.display());
                Box::into_raw(Box::new(YurisSession { player, gpu: None, surface_size: (w, h) }))
            }
            Err(e) => {
                set_error(e.to_string());
                std::ptr::null_mut()
            }
        }
    })
}

/// # Safety
/// `session` is a session from `yuris_open`; the out pointers are NULL or valid.
#[no_mangle]
pub unsafe extern "C" fn yuris_screen_size(session: *const YurisSession, width: *mut u32, height: *mut u32) {
    guard((), || {
        let Some(s) = session.as_ref() else { return };
        let (w, h) = s.player.screen();
        if let Some(p) = width.as_mut() {
            *p = w;
        }
        if let Some(p) = height.as_mut() {
            *p = h;
        }
    })
}

#[cfg(any(target_os = "ios", target_os = "tvos", target_os = "visionos"))]
fn surface_target(view: *mut c_void) -> Option<wgpu::SurfaceTargetUnsafe> {
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle, UiKitDisplayHandle, UiKitWindowHandle};
    let view = NonNull::new(view)?;
    Some(wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: RawDisplayHandle::UiKit(UiKitDisplayHandle::new()),
        raw_window_handle: RawWindowHandle::UiKit(UiKitWindowHandle::new(view)),
    })
}

#[cfg(target_os = "macos")]
fn surface_target(view: *mut c_void) -> Option<wgpu::SurfaceTargetUnsafe> {
    use raw_window_handle::{AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle};
    let view = NonNull::new(view)?;
    Some(wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: RawDisplayHandle::AppKit(AppKitDisplayHandle::new()),
        raw_window_handle: RawWindowHandle::AppKit(AppKitWindowHandle::new(view)),
    })
}

#[cfg(not(target_vendor = "apple"))]
fn surface_target(_view: *mut c_void) -> Option<wgpu::SurfaceTargetUnsafe> {
    None
}

impl YurisSession {
    fn attach(&mut self, view: *mut c_void, width: u32, height: u32) -> Result<(), String> {
        if view.is_null() || width == 0 || height == 0 {
            return Err("no view or an empty size".into());
        }
        let same_view = self.gpu.as_ref().is_some_and(|g| g.view == view && g.surface.is_some());
        if !same_view {
            let target = surface_target(view).ok_or("views are not supported on this platform")?;
            if self.gpu.is_none() {
                let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                    backends: wgpu::Backends::PRIMARY,
                    ..Default::default()
                });
                // SAFETY: the host keeps the view alive while attached.
                let surface = unsafe { instance.create_surface_unsafe(target) }.map_err(|e| e.to_string())?;
                let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                }))
                .map_err(|e| format!("no GPU adapter: {e}"))?;
                let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                    label: Some("yuris"),
                    required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
                    ..Default::default()
                }))
                .map_err(|e| format!("no GPU device: {e}"))?;
                let format = pick_format(&surface.get_capabilities(&adapter));
                let renderer = Renderer::new(device, queue, format);
                tracing::info!("GPU: {:?}, surface format {format:?}", adapter.get_info().name);
                self.gpu = Some(Gpu { instance, adapter, renderer, surface: Some(surface), config: None, view, drawn: None });
            } else {
                let gpu = self.gpu.as_mut().expect("gpu");
                // SAFETY: as above.
                let surface = unsafe { gpu.instance.create_surface_unsafe(target) }.map_err(|e| e.to_string())?;
                if pick_format(&surface.get_capabilities(&gpu.adapter)) != gpu.renderer.out_format() {
                    return Err("the new view wants another pixel format".into());
                }
                gpu.surface = Some(surface);
                gpu.view = view;
            }
        }
        let gpu = self.gpu.as_mut().expect("gpu");
        let surface = gpu.surface.as_ref().expect("surface");
        let caps = surface.get_capabilities(&gpu.adapter);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: gpu.renderer.out_format(),
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
                wgpu::CompositeAlphaMode::Opaque
            } else {
                caps.alpha_modes[0]
            },
            view_formats: Vec::new(),
        };
        surface.configure(gpu.renderer.device(), &config);
        gpu.config = Some(config);
        gpu.drawn = None;
        self.surface_size = (width, height);
        Ok(())
    }

    /// Draws the frame if the layers changed since the last one.
    fn draw(&mut self) {
        let Some(gpu) = self.gpu.as_mut() else { return };
        let (Some(surface), Some(config)) = (gpu.surface.as_ref(), gpu.config.as_ref()) else { return };
        let host = self.player.host();
        if gpu.drawn == Some(host.tree.generation) {
            return;
        }
        let frame = match surface.get_current_texture() {
            Ok(f) => f,
            Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
                surface.configure(gpu.renderer.device(), config);
                return;
            }
            Err(e) => {
                tracing::warn!("no drawable: {e}");
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        gpu.renderer.render(&host.tree, self.player.screen(), &view, (config.width, config.height));
        frame.present();
        gpu.drawn = Some(host.tree.generation);
    }

    /// Surface pixels to screen pixels.
    fn to_screen(&self, x: f32, y: f32) -> (i64, i64) {
        let screen = self.player.screen();
        let r = fit(screen, self.surface_size);
        let sx = (x - r[0]) * screen.0 as f32 / r[2].max(1.0);
        let sy = (y - r[1]) * screen.1 as f32 / r[3].max(1.0);
        (sx.floor() as i64, sy.floor() as i64)
    }
}

/// A surface format without sRGB conversion: the engine's colours are
/// already sRGB values, as on Windows.
fn pick_format(caps: &wgpu::SurfaceCapabilities) -> wgpu::TextureFormat {
    caps.formats
        .iter()
        .copied()
        .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
        .or_else(|| caps.formats.iter().copied().find(|f| !f.is_srgb()))
        .unwrap_or(wgpu::TextureFormat::Bgra8Unorm)
}

/// # Safety
/// `session` is a session from `yuris_open`; `ui_view` a live UIView.
#[no_mangle]
pub unsafe extern "C" fn yuris_set_surface(session: *mut YurisSession, ui_view: *mut c_void, width_px: u32, height_px: u32) -> i32 {
    guard(-1, || {
        let Some(s) = session.as_mut() else {
            set_error("no session");
            return -1;
        };
        match s.attach(ui_view, width_px, height_px) {
            Ok(()) => 0,
            Err(e) => {
                set_error(e);
                -1
            }
        }
    })
}

/// # Safety
/// `session` is a session from `yuris_open`.
#[no_mangle]
pub unsafe extern "C" fn yuris_detach_surface(session: *mut YurisSession) {
    guard((), || {
        if let Some(g) = session.as_mut().and_then(|s| s.gpu.as_mut()) {
            g.surface = None;
            g.config = None;
            g.view = std::ptr::null_mut();
        }
    })
}

/// # Safety
/// `session` is a session from `yuris_open`.
#[no_mangle]
pub unsafe extern "C" fn yuris_step(session: *mut YurisSession, dt_ms: u32) -> i32 {
    guard(2, || {
        let Some(s) = session.as_mut() else {
            set_error("no session");
            return 2;
        };
        let status = s.player.step(dt_ms).clone();
        s.draw();
        match status {
            Status::Running => 0,
            Status::Ended => 1,
            Status::Failed(m) => {
                set_error(m);
                2
            }
        }
    })
}

/// # Safety
/// `session` is a session from `yuris_open`.
#[no_mangle]
pub unsafe extern "C" fn yuris_pointer(session: *mut YurisSession, phase: i32, x: f32, y: f32) {
    guard((), || {
        let Some(s) = session.as_mut() else { return };
        let (sx, sy) = s.to_screen(x, y);
        s.player.pointer(sx, sy);
        match phase {
            1 => s.player.button(MouseButton::Left, true),
            2 => s.player.button(MouseButton::Left, false),
            3 => s.player.button(MouseButton::Right, true),
            4 => s.player.button(MouseButton::Right, false),
            _ => {}
        }
    })
}

/// # Safety
/// `session` is a session from `yuris_open`.
#[no_mangle]
pub unsafe extern "C" fn yuris_wheel(session: *mut YurisSession, notches: i32) {
    guard((), || {
        if let Some(s) = session.as_mut() {
            s.player.wheel(notches);
        }
    })
}

/// # Safety
/// `session` is a session from `yuris_open`.
#[no_mangle]
pub unsafe extern "C" fn yuris_key(session: *mut YurisSession, vk: i32, down: i32) {
    guard((), || {
        let Some(s) = session.as_mut() else { return };
        if let Some(k) = key_from_vk(vk) {
            s.player.key(k, down != 0);
        }
    })
}

/// # Safety
/// `session` is a session from `yuris_open`.
#[no_mangle]
pub unsafe extern "C" fn yuris_audio(session: *const YurisSession) -> *mut YurisAudio {
    guard(std::ptr::null_mut(), || match session.as_ref() {
        Some(s) => match audio::AudioOutput::new(s.player.audio()) {
            Ok(out) => Box::into_raw(Box::new(YurisAudio(out))),
            Err(e) => {
                set_error(format!("no audio thread: {e}"));
                std::ptr::null_mut()
            }
        },
        None => std::ptr::null_mut(),
    })
}

#[no_mangle]
pub extern "C" fn yuris_audio_sample_rate() -> u32 {
    OUTPUT_RATE
}

/// # Safety
/// `audio` is from `yuris_audio`, used by one thread at a time; `out`
/// holds `2 * frames` floats.
#[no_mangle]
pub unsafe extern "C" fn yuris_audio_render(audio: *mut YurisAudio, out: *mut f32, frames: u32) {
    if out.is_null() {
        return;
    }
    let buf = std::slice::from_raw_parts_mut(out, 2 * frames as usize);
    let ok = catch_unwind(AssertUnwindSafe(|| match audio.as_mut() {
        Some(a) => {
            a.0.render(buf);
            true
        }
        None => false,
    }));
    if !matches!(ok, Ok(true)) {
        buf.fill(0.0);
    }
}

/// # Safety
/// `audio` is from `yuris_audio`, released once.
#[no_mangle]
pub unsafe extern "C" fn yuris_audio_release(audio: *mut YurisAudio) {
    if !audio.is_null() {
        drop(Box::from_raw(audio));
    }
}

/// # Safety
/// `session` is from `yuris_open`, closed once.
#[no_mangle]
pub unsafe extern "C" fn yuris_close(session: *mut YurisSession) {
    guard((), || {
        if !session.is_null() {
            drop(Box::from_raw(session));
        }
    })
}
