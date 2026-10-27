//! Runs a game in real time, for players with a window: boots it, steps
//! the VM one engine frame per display frame, and feeds it the clock and
//! the input. Drawing (`yuris-render`) and sound output (pulling
//! [`Player::audio`]) are the caller's.
//!
//! `yuris boot` runs the same VM headless with simulated time; this is the
//! wall-clock version of its loop.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use yuris_format::vfs::GameFs;
use yuris_host::audio::Mixer;
use yuris_host::Host;
use yuris_vm::boot::Bootstrap;
use yuris_vm::{GroupVm, Key, MouseButton, ResumeResponse, VmEvent, VmSuspend};

/// Longest wall time one [`Player::step`] runs scripts for. A frame that
/// needs more goes on in the next step, so the display keeps going.
const STEP_BUDGET: Duration = Duration::from_millis(100);
/// Longest clock advance per step: after a stall the game slows down
/// rather than skipping its timed waits.
const MAX_STEP_MS: u64 = 250;

/// Errors opening a game.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The game folder or its archives could not be read.
    #[error("cannot open the game: {0}")]
    Files(String),
    /// The scripts could not be booted.
    #[error("cannot boot the game: {0}")]
    Boot(String),
    /// A font file is not a font.
    #[error("not a font: {0}")]
    Font(String),
}

/// Text encoding of the game's strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Encoding {
    /// Shift-JIS (Japanese releases).
    #[default]
    ShiftJis,
    /// GBK (Chinese patches).
    Gbk,
    /// UTF-8.
    Utf8,
    /// Shift-JIS or GBK, whichever the game's scripts are written in
    /// (see [`guess_encoding`]).
    Auto,
}

/// How to open a game.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// The game folder (or its `pac` folder).
    pub game_dir: PathBuf,
    /// A translation patch's launcher (an Enigma Virtual Box executable)
    /// whose packed files win over the game's.
    pub launcher: Option<PathBuf>,
    /// Where `save\` goes; the game folder's `save` by default.
    pub save_dir: Option<PathBuf>,
    /// Font files; the first is the default face.
    pub fonts: Vec<FontFile>,
    /// Text encoding.
    pub encoding: Encoding,
    /// Local time offset from UTC, for the date the game reads (`@_TIME`).
    pub utc_offset_ms: i64,
}

/// A font file (TTF/OTF/TTC) and the face to use in it.
#[derive(Debug, Clone, Default)]
pub struct FontFile {
    /// The file.
    pub path: PathBuf,
    /// Face index in a collection, used when `name` is None.
    pub index: u32,
    /// PostScript or family name of the face to use.
    pub name: Option<String>,
}

impl FontFile {
    /// The first face of a file.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into(), index: 0, name: None }
    }
}

/// The index of the face named `name` (PostScript or family name) in a
/// font file or collection.
fn face_index(data: &[u8], name: &str) -> Option<u32> {
    let count = ttf_parser::fonts_in_collection(data).unwrap_or(1);
    (0..count).find(|&i| {
        ttf_parser::Face::parse(data, i).is_ok_and(|f| {
            f.names().into_iter().any(|n| {
                matches!(n.name_id, ttf_parser::name_id::POST_SCRIPT_NAME | ttf_parser::name_id::FULL_NAME | ttf_parser::name_id::FAMILY)
                    && n.to_string().is_some_and(|s| s == name)
            })
        })
    })
}

/// Where the game is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Running.
    Running,
    /// The main script ended.
    Ended,
    /// The VM stopped on an error.
    Failed(String),
}

/// A running game.
pub struct Player {
    vm: GroupVm,
    screen: (u32, u32),
    audio: Arc<Mutex<Mixer>>,
    status: Status,
    /// Engine clock, ms since start.
    clock_ms: u64,
    /// Wall clock at start (ms since 1970, local time).
    start_ms: i64,
    /// The VM is suspended in a WAIT and needs a resume.
    waiting: bool,
    /// Display frames to skip for a frame-counted WAIT without tasks.
    skip_frames: u64,
    /// Clock time a timed WAIT without tasks ends.
    wait_until: u64,
    cursor: (i64, i64),
    /// Buttons held now: left, right, middle.
    held: [bool; 3],
    /// Buttons pressed since the last frame: a press and release between
    /// two frames still counts for one frame.
    pressed: [bool; 3],
    wheel: i32,
    keys_held: Vec<Key>,
    keys_pressed: Vec<Key>,
    /// Commands reported as unsupported, logged once each.
    unsupported: std::collections::HashSet<u8>,
}

impl Player {
    /// Opens and boots a game. Nothing runs before the first [`Self::step`].
    pub fn open(config: &Config) -> Result<Self, Error> {
        let mut fs = GameFs::open_with_launcher(&config.game_dir, config.launcher.as_deref())
            .map_err(|e| Error::Files(e.to_string()))?;
        if let Some(dir) = &config.save_dir {
            fs.set_save_dir(dir.clone());
        }
        let fs = Arc::new(fs);
        let booted = Bootstrap { fs: fs.clone(), key: None, entry_label: None }
            .boot()
            .map_err(|e| Error::Boot(e.to_string()))?;
        let key = booted.key;
        let mut vm = booted.vm;
        let cfg = fs
            .read("ysbin\\yscfg.ybn")
            .ok()
            .and_then(|b| yuris_format::yscf::YscfFile::from_bytes(&b).ok());
        let screen = match &cfg {
            Some(c) if c.screen_width > 0 && c.screen_height > 0 => (c.screen_width, c.screen_height),
            _ => (1280, 720),
        };
        let encoding = match config.encoding {
            Encoding::Auto => {
                let guessed = guess_encoding(&fs, key);
                tracing::info!("script text looks like {guessed:?}");
                guessed
            }
            chosen => chosen,
        };
        let mut host = Host::new(Some(fs.clone()), screen);
        host.encoding = match encoding {
            Encoding::ShiftJis => encoding_rs::SHIFT_JIS,
            Encoding::Gbk => encoding_rs::GBK,
            Encoding::Utf8 => encoding_rs::UTF_8,
            Encoding::Auto => unreachable!(),
        };
        if encoding == Encoding::Gbk {
            vm.set_codepage(yuris_vm::Codepage::Gbk);
        }
        if let Some(c) = &cfg {
            host.caption = host.encoding.encode(&c.caption).0.into_owned();
        }
        for font in &config.fonts {
            let path = &font.path;
            let bytes = std::fs::read(path).map_err(|e| Error::Font(format!("{}: {e}", path.display())))?;
            let index = match &font.name {
                Some(name) => face_index(&bytes, name)
                    .ok_or_else(|| Error::Font(format!("{} has no face {name}", path.display())))?,
                None => font.index,
            };
            let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if !host.fonts.add_face(&name, bytes, index) {
                return Err(Error::Font(path.display().to_string()));
            }
        }
        let audio = host.audio.clone();
        vm.set_native(Box::new(host));
        vm.set_strict(false);
        vm.set_trace(false);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        Ok(Self {
            vm,
            screen,
            audio,
            status: Status::Running,
            clock_ms: 0,
            start_ms: now + config.utc_offset_ms,
            waiting: false,
            skip_frames: 0,
            wait_until: 0,
            cursor: (0, 0),
            held: [false; 3],
            pressed: [false; 3],
            wheel: 0,
            keys_held: Vec::new(),
            keys_pressed: Vec::new(),
            unsupported: Default::default(),
        })
    }

    /// Overrides the virtual display capabilities exposed as @114..@116.
    /// Does not resize the game canvas or change cursor/letterbox coordinates.
    pub fn set_display_metrics(&mut self, display: yuris_vm::DisplayMetrics) {
        self.vm.set_display_metrics(display);
    }

    /// The game's screen size (yscfg).
    pub fn screen(&self) -> (u32, u32) {
        self.screen
    }

    /// The native host: the layer tree to draw is `host().tree`.
    pub fn host(&self) -> &Host {
        self.vm.native_as::<Host>().expect("the player's host")
    }

    /// The sound mixer. Pull [`Mixer::render`] from the audio thread at
    /// [`yuris_host::audio::OUTPUT_RATE`], interleaved stereo.
    pub fn audio(&self) -> Arc<Mutex<Mixer>> {
        self.audio.clone()
    }

    /// The VM, for diagnostics.
    pub fn vm(&self) -> &GroupVm {
        &self.vm
    }

    /// Where the game is.
    pub fn status(&self) -> &Status {
        &self.status
    }

    /// Moves the cursor (screen pixels).
    pub fn pointer(&mut self, x: i64, y: i64) {
        self.cursor = (x, y);
    }

    /// Presses or releases a mouse button (`MouseButton::Left`, `Right`
    /// or `Middle`).
    pub fn button(&mut self, b: MouseButton, down: bool) {
        let i = match b {
            MouseButton::Left => 0,
            MouseButton::Right => 1,
            MouseButton::Middle => 2,
            MouseButton::WheelUp => return self.wheel(1),
            MouseButton::WheelDown => return self.wheel(-1),
        };
        self.held[i] = down;
        self.pressed[i] |= down;
    }

    /// Turns the wheel: positive is up (away from the player), one per
    /// notch. Each notch is one frame of the wheel button.
    pub fn wheel(&mut self, notches: i32) {
        self.wheel = self.wheel.saturating_add(notches);
    }

    /// Presses or releases a key.
    pub fn key(&mut self, k: Key, down: bool) {
        if down {
            if !self.keys_held.contains(&k) {
                self.keys_held.push(k);
            }
            if !self.keys_pressed.contains(&k) {
                self.keys_pressed.push(k);
            }
        } else {
            self.keys_held.retain(|&h| h != k);
        }
    }

    /// Advances the clock by `dt_ms` and runs the game's next frame, if
    /// it is due. Call once per display frame.
    pub fn step(&mut self, dt_ms: u32) -> &Status {
        if self.status != Status::Running {
            return &self.status;
        }
        self.clock_ms += u64::from(dt_ms).min(MAX_STEP_MS);
        if self.skip_frames > 0 {
            self.skip_frames -= 1;
            return &self.status;
        }
        if self.clock_ms < self.wait_until {
            return &self.status;
        }
        if let Err(e) = self.begin_frame() {
            self.status = Status::Failed(e);
            return &self.status;
        }
        let started = Instant::now();
        loop {
            let s = match self.vm.run(1000) {
                Ok(s) => s,
                Err(e) => {
                    self.status = Status::Failed(e.to_string());
                    break;
                }
            };
            self.drain_events();
            match s {
                VmSuspend::None => {
                    if started.elapsed() >= STEP_BUDGET {
                        break;
                    }
                }
                VmSuspend::Wait { counter, time_ms } => {
                    // With tasks the scheduler keeps the waits itself and
                    // this is always one frame; without, the host counts.
                    self.waiting = true;
                    self.skip_frames = counter.unwrap_or(1).saturating_sub(1);
                    self.wait_until = time_ms.map_or(0, |t| self.clock_ms + t);
                    break;
                }
                VmSuspend::Complete => {
                    self.status = Status::Ended;
                    break;
                }
                VmSuspend::Error(m) => {
                    self.status = Status::Failed(m);
                    break;
                }
            }
        }
        &self.status
    }

    /// The engine's start of a frame: clock, date, cursor and buttons,
    /// then the resume out of the last WAIT.
    fn begin_frame(&mut self) -> Result<(), String> {
        let vm = &mut self.vm;
        vm.set_clock_ms(self.clock_ms);
        vm.set_date_time(date_time(self.start_ms + self.clock_ms as i64)).map_err(|e| e.to_string())?;
        vm.set_input_cursor(self.cursor.0, self.cursor.1);
        for &k in &self.keys_pressed {
            vm.set_key(k, true);
        }
        for &k in &self.keys_held {
            vm.set_key(k, true);
        }
        if self.wheel > 0 {
            vm.set_mouse_button(MouseButton::WheelUp, true);
            self.wheel -= 1;
        } else if self.wheel < 0 {
            vm.set_mouse_button(MouseButton::WheelDown, true);
            self.wheel += 1;
        }
        let b = |i: usize| self.held[i] || self.pressed[i];
        vm.update_mouse_buttons(b(0), b(1), b(2)).map_err(|e| e.to_string())?;
        // Keys pressed and released since the last frame go up now.
        for k in std::mem::take(&mut self.keys_pressed) {
            if !self.keys_held.contains(&k) {
                vm.set_key(k, false);
            }
        }
        self.pressed = [false; 3];
        if self.waiting {
            self.waiting = false;
            vm.resume(ResumeResponse::Continue).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn drain_events(&mut self) {
        for e in self.vm.take_events() {
            if let VmEvent::Unsupported { command, reason, .. } = e {
                if self.unsupported.insert(command) {
                    tracing::warn!("{} not supported: {reason}", self.vm.command_map().name(command));
                }
            }
        }
    }
}

/// `@_TIME` row 0 for a time in ms since 1970 (already in local time):
/// year, month, day, weekday (0 = Sunday), hour, minute, second, ms.
pub fn date_time(ms: i64) -> [i64; 8] {
    let days = ms.div_euclid(86_400_000);
    let in_day = ms.rem_euclid(86_400_000);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let weekday = (days + 4).rem_euclid(7);
    [
        year,
        month,
        day,
        weekday,
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1000 % 60,
        in_day % 1000,
    ]
}

/// The key for a Windows virtual-key code, as a hardware keyboard sends
/// it (Japanese layout for the punctuation keys).
pub fn key_from_vk(vk: i32) -> Option<Key> {
    use Key::*;
    const LETTERS: [Key; 26] = [A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z];
    const DIGITS: [Key; 10] = [D0, D1, D2, D3, D4, D5, D6, D7, D8, D9];
    const NUMPAD: [Key; 10] = [Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9];
    const FN: [Key; 24] = [
        F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20, F21, F22, F23, F24,
    ];
    Some(match vk {
        0x41..=0x5a => LETTERS[(vk - 0x41) as usize],
        0x30..=0x39 => DIGITS[(vk - 0x30) as usize],
        0x60..=0x69 => NUMPAD[(vk - 0x60) as usize],
        0x70..=0x87 => FN[(vk - 0x70) as usize],
        0x6a => NumMul,
        0x6b => NumAdd,
        0x6d => NumSub,
        0x6e => NumDec,
        0x6f => NumDiv,
        0x08 => Backspace,
        0x09 => Tab,
        0x0d => Enter,
        0x20 => Space,
        0x10 | 0xa0 | 0xa1 => Shift,
        0x11 | 0xa2 | 0xa3 => Ctrl,
        0x12 | 0xa4 | 0xa5 => Alt,
        0x25 => Left,
        0x26 => Up,
        0x27 => Right,
        0x28 => Down,
        0x2d => Insert,
        0x2e => Delete,
        0x2f => Help,
        0x91 => Scroll,
        0x90 => NumLock,
        0x1b => Escape,
        0x21 => PageUp,
        0x22 => PageDown,
        0x24 => Home,
        0x23 => End,
        0xbd => Hyphen,
        0xde => Circumflex,
        0xdc => Yen,
        0xc0 => At,
        0xdb => LeftBracket,
        0xbb => Semicolon,
        0xba => Colon,
        0xdd => RightBracket,
        0xbc => Comma,
        0xbe => Period,
        0xbf => Slash,
        0xe2 => Backslash,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_keys() {
        // 2026-10-25 12:34:56.789 UTC was a Sunday.
        assert_eq!(date_time(1_792_931_696_789), [2026, 10, 25, 0, 12, 34, 56, 789]);
        assert_eq!(key_from_vk(0x41), Some(Key::A));
        assert_eq!(key_from_vk(0x7b), Some(Key::F12));
        assert_eq!(key_from_vk(0x11), Some(Key::Ctrl));
        assert_eq!(key_from_vk(0xe2), Some(Key::Backslash));
        assert_eq!(key_from_vk(0x07), None);
    }

    #[test]
    fn faces_by_name() {
        let Ok(data) = std::fs::read("/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc") else { return };
        assert_eq!(face_index(&data, "WenQuanYiZenHei"), Some(0));
        assert_eq!(face_index(&data, "WenQuanYi Zen Hei Mono"), Some(1));
        assert_eq!(face_index(&data, "MS Gothic"), None);
    }
}

/// Whether the game's scripts are written in Shift-JIS or GBK. Translation
/// patches often keep the Japanese file names, so the names can't tell.
///
/// Counts the two-byte characters of each kind in the scripts' expression
/// pools, where the strings live: Japanese text is full of Shift-JIS kana,
/// kanji and punctuation (lead bytes 0x81-0x9F), which GBK puts in its
/// rarely used extension area, and Chinese text of GB2312 hanzi (both bytes
/// 0xA1 and up, from 0xB0), which Shift-JIS reads as half-width katakana.
/// A patch keeps much of the Japanese (Erewhon CHS: 586k GBK pairs next to
/// 953k Shift-JIS ones, the Japanese release 900 GBK-looking ones), so a
/// few percent of GB2312 pairs already make it Chinese.
pub fn guess_encoding(fs: &GameFs, key: [u8; 4]) -> Encoding {
    let (mut gbk, mut sjis) = (0usize, 0usize);
    for id in yuris_vm::host::list_script_ids(fs) {
        let Ok(bytes) = fs.read(&yuris_vm::host::script_path(id)) else { continue };
        let Ok(script) = yuris_format::ystb::YstbFile::from_bytes(&bytes, key) else { continue };
        let pool = script.content();
        let mut i = 0;
        while i + 1 < pool.len() {
            let (lead, trail) = (pool[i], pool[i + 1]);
            if lead < 0x80 {
                i += 1;
                continue;
            }
            if (0x81..=0x9F).contains(&lead) && (0x40..=0xFC).contains(&trail) && trail != 0x7F {
                sjis += 1;
            } else if (0xB0..=0xF7).contains(&lead) && (0xA1..=0xFE).contains(&trail) {
                gbk += 1;
            }
            i += 2;
        }
    }
    tracing::debug!("two-byte characters: {gbk} GBK, {sjis} Shift-JIS");
    if gbk * 20 > sjis {
        Encoding::Gbk
    } else {
        Encoding::ShiftJis
    }
}
