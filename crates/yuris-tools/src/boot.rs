//! `yuris boot`: runs the boot chain headless and reports the game files,
//! versions and key, how far the VM gets before it stops, and which commands
//! are not implemented yet. The first step when bringing up a new game.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use yuris_format::vfs::GameFs;
use yuris_host::audio::OUTPUT_RATE;
use yuris_host::Host;
use yuris_format::yscm::YscmTable;
use yuris_format::ystb::YstbFile;
use yuris_vm::boot::Bootstrap;
use yuris_vm::host::{list_script_ids, script_path};
use yuris_vm::{ResumeResponse, VmEvent, VmSuspend};

/// Options for `yuris boot`.
pub struct BootArgs {
    pub dir: PathBuf,
    pub launcher: Option<PathBuf>,
    pub key: Option<[u8; 4]>,
    pub entry: Option<String>,
    pub max_groups: usize,
    pub jsonl: Option<PathBuf>,
    pub strict: bool,
    pub trace: bool,
    pub tail: usize,
    /// Clicks, `X,Y@MS`.
    pub clicks: Vec<String>,
    /// CG name filter for the end report.
    pub cgs: Option<String>,
    /// Global @ variables for the end report.
    pub vars: Vec<u16>,
    /// Watch points, `script:pc`.
    pub watch: Vec<String>,
    /// Screenshots, `FILE.png@MS`.
    pub shots: Vec<String>,
    /// Font files to register (TTF/OTF/TTC, first face).
    pub fonts: Vec<PathBuf>,
    /// Text encoding of the scripts' strings (`sjis` or `gbk`).
    pub encoding: String,
    /// `X,Y@MS`: from MS on, click at X,Y when a text has waited a second.
    pub auto_click: Option<String>,
    /// Variables to print at the end (`$1259`, `@1264`).
    pub dumps: Vec<String>,
    /// WAVE file for the mixed sound output.
    pub wav: Option<PathBuf>,
    /// Folder for save files.
    pub save_dir: Option<PathBuf>,
}

fn hex(k: &[u8]) -> String {
    k.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn run(args: BootArgs) -> Result<()> {
    let mut fs = GameFs::open_with_launcher(&args.dir, args.launcher.as_deref())
        .with_context(|| format!("open {}", args.dir.display()))?;
    if let Some(dir) = &args.save_dir {
        fs.set_save_dir(dir.clone());
    }
    println!("game root: {}", fs.root().display());
    for pkg in fs.overlay() {
        println!("  overlay {} ({} files)", pkg.path().display(), pkg.files().len());
    }
    for p in fs.packs() {
        let idx = p.index();
        println!(
            "  pack {:<24} v{} {:>6} entries  name_key={:#04x} wide={} hash={:?}",
            p.name(),
            idx.header.version,
            idx.entries.len(),
            idx.name_key,
            idx.wide_offsets,
            idx.name_hash,
        );
    }
    for (name, why) in &fs.skipped {
        println!("  skipped {name}: {why}");
    }

    let ids = list_script_ids(&fs);
    println!("scripts: {} (yst{:05} .. yst{:05})", ids.len(), ids.first().unwrap_or(&0), ids.last().unwrap_or(&0));
    if let Some(&id) = ids.first() {
        if let Ok(b) = fs.read(&script_path(id)) {
            if b.len() >= 8 {
                println!("YSTB version: {}", u32::from_le_bytes(b[4..8].try_into().unwrap()));
            }
        }
    }
    let yscm = fs
        .read("ysbin\\ysc.ybn")
        .ok()
        .and_then(|b| YscmTable::from_bytes(&b).ok());
    match &yscm {
        Some(t) => println!("YSCM version: {}, {} commands", t.version(), t.commands().len()),
        None => println!("YSCM: ysbin\\ysc.ybn missing or unreadable"),
    }

    let fs = Arc::new(fs);
    let booted = Bootstrap {
        fs: fs.clone(),
        key: args.key,
        entry_label: args.entry.map(String::into_bytes),
    }
    .boot()
    .context("boot")?;
    println!(
        "YSTB key: {}  entry: yst{:05} pc {}  ysv initial values: {}",
        hex(&booted.key),
        booted.script_id,
        booted.entry_pc,
        booted.applied_ysvr
    );
    // Parse every script with the key; a wrong key or version fails most of them.
    let bad = ids
        .iter()
        .filter(|&&id| {
            fs.read(&script_path(id))
                .ok()
                .and_then(|b| YstbFile::from_bytes(&b, booted.key).ok())
                .and_then(|f| f.groups().ok())
                .is_none()
        })
        .count();
    println!("scripts failing to parse with this key: {bad}");

    let mut vm = booted.vm;
    let cfg = fs
        .read("ysbin\\yscfg.ybn")
        .ok()
        .and_then(|b| yuris_format::yscf::YscfFile::from_bytes(&b).ok());
    let screen = match &cfg {
        Some(c) if c.screen_width > 0 && c.screen_height > 0 => (c.screen_width, c.screen_height),
        _ => (1280, 720),
    };
    println!("screen: {}x{}", screen.0, screen.1);
    let mut host = Host::new(Some(fs.clone()), screen);
    host.encoding = match args.encoding.to_ascii_lowercase().as_str() {
        "gbk" | "gb2312" | "cp936" => encoding_rs::GBK,
        "utf8" | "utf-8" => encoding_rs::UTF_8,
        _ => encoding_rs::SHIFT_JIS,
    };
    if host.encoding == encoding_rs::GBK {
        vm.set_codepage(yuris_vm::Codepage::Gbk);
    }
    if let Some(c) = &cfg {
        host.caption = host.encoding.encode(&c.caption).0.into_owned();
    }
    for f in &args.fonts {
        let bytes = std::fs::read(f).with_context(|| format!("read {}", f.display()))?;
        let name = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if !host.fonts.add_face(&name, bytes, 0) {
            anyhow::bail!("{} is not a font", f.display());
        }
    }
    vm.set_native(Box::new(host));
    vm.set_strict(args.strict);
    // Control-flow events cost most of a run; record them only if asked.
    vm.set_trace(args.trace || args.tail > 0);
    let foreign = vm.command_map().foreign_names();
    if !foreign.is_empty() {
        println!("commands not in the v555 table: {}", foreign.join(", "));
    }
    for (cmd, pairs) in vm.command_map().slot_changes() {
        let moved: Vec<String> = pairs
            .iter()
            .map(|(g, c)| if c & 0x80 != 0 { format!("{g}->new") } else { format!("{g}->{c}") })
            .collect();
        println!("  {cmd} parameter slots: {}", moved.join(" "));
    }
    let names = vm.command_map().clone();
    let cmd_name = |c: u8| names.name(c);
    let mut out = match &args.jsonl {
        Some(p) => Some(std::io::BufWriter::new(std::fs::File::create(p)?)),
        None => None,
    };

    // Scripted input: the cursor moves to each click's point when the
    // clock reaches it, and the left button is down for that one frame.
    let mut clicks: Vec<(i64, i64, u64)> = Vec::new();
    for c in &args.clicks {
        let parsed = c.split_once('@').and_then(|(xy, t)| {
            let (x, y) = xy.split_once(',')?;
            Some((x.trim().parse().ok()?, y.trim().parse().ok()?, t.trim().parse().ok()?))
        });
        match parsed {
            Some(p) => clicks.push(p),
            None => anyhow::bail!("--click wants X,Y@MS, got {c:?}"),
        }
    }
    clicks.sort_by_key(|c| c.2);
    let auto_click: Option<(i64, i64, u64)> = match &args.auto_click {
        Some(c) => Some(
            c.split_once('@')
                .and_then(|(xy, t)| {
                    let (x, y) = xy.split_once(',')?;
                    Some((x.trim().parse().ok()?, y.trim().parse().ok()?, t.trim().parse().ok()?))
                })
                .with_context(|| format!("--auto-click wants X,Y@MS, got {c:?}"))?,
        ),
        None => None,
    };
    let text_enc = match args.encoding.to_ascii_lowercase().as_str() {
        "gbk" | "gb2312" | "cp936" => encoding_rs::GBK,
        "utf8" | "utf-8" => encoding_rs::UTF_8,
        _ => encoding_rs::SHIFT_JIS,
    };
    let mut waiting_since: Option<u64> = None;
    let mut watch = Vec::new();
    for w in &args.watch {
        match w.split_once(':').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))) {
            Some(p) => watch.push(p),
            None => anyhow::bail!("--watch wants script:pc, got {w:?}"),
        }
    }
    vm.set_watch(&watch);
    let mut shots: Vec<(PathBuf, u64)> = Vec::new();
    for sh in &args.shots {
        match sh.rsplit_once('@').and_then(|(f, t)| Some((PathBuf::from(f), t.trim().parse().ok()?))) {
            Some(p) => shots.push(p),
            None => anyhow::bail!("--shot wants FILE.png@MS, got {sh:?}"),
        }
    }
    shots.sort_by_key(|s| s.1);
    let mut next_shot = 0usize;
    let mut next_click = 0usize;
    let mut button_down = false;
    let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let mut frames = 0usize;
    let start = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    vm.set_date_time(date_time(start))?;
    let mut audio_frames = 0u64;
    let mut audio_buf: Vec<f32> = Vec::new();
    let mut wav = args.wav.as_deref().map(WavWriter::create).transpose()?;
    let mut kinds = [0usize; KINDS.len()];
    let mut unsupported: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut subsystem: BTreeMap<String, usize> = BTreeMap::new();
    let stop = loop {
        let s = match vm.run(1000) {
            Ok(s) => s,
            Err(e) => break format!("error: {e}"),
        };
        for e in &vm.take_events() {
            if let Some(o) = out.as_mut() {
                writeln!(o, "{}", yuris_vm::event_json(e))?;
            }
            if args.tail > 0 {
                if tail.len() == args.tail {
                    tail.pop_front();
                }
                tail.push_back(yuris_vm::event_json(e));
            }
            kinds[kind(e)] += 1;
            match e {
                VmEvent::Unsupported { command, reason, .. } => {
                    let slot = unsupported.entry(cmd_name(*command)).or_insert((0, reason.clone()));
                    slot.0 += 1;
                }
                VmEvent::Subsystem { command, .. } => {
                    *subsystem.entry(cmd_name(*command)).or_default() += 1;
                }
                VmEvent::TextOut { text, flags, .. } => {
                    println!(
                        "text ({:.1} s): {} [{}]",
                        vm.clock_ms() as f64 / 1000.0,
                        text_enc.decode(text).0,
                        String::from_utf8_lossy(flags).chars().filter(|c| !matches!(c, '1' | '2')).collect::<String>()
                    );
                }
                _ => {}
            }
        }
        if vm.executed_groups() >= args.max_groups {
            break format!("group budget {} reached", args.max_groups);
        }
        match s {
            VmSuspend::None => {}
            VmSuspend::Wait { counter, time_ms } => {
                // Simulated time: a frame is 1/60 s; a timed wait passes
                // its whole duration at once.
                frames += 1;
                let step = time_ms.unwrap_or(0).max(counter.unwrap_or(1) * 1000 / 60).max(1);
                vm.set_clock_ms(vm.clock_ms() + step);
                vm.set_date_time(date_time(start + vm.clock_ms() as i64))?;
                // Sound positions move only as the mixer is pulled: pull
                // as much as the clock advanced.
                if let Some(h) = vm.native_as::<Host>() {
                    let target = vm.clock_ms() as u64 * u64::from(OUTPUT_RATE) / 1000;
                    let n = target.saturating_sub(audio_frames) as usize;
                    audio_frames = target;
                    audio_buf.resize(2 * n, 0.0);
                    h.audio.lock().unwrap().render(&mut audio_buf);
                    if let Some(w) = wav.as_mut() {
                        w.write(&audio_buf)?;
                    }
                }
                while let Some((file, t)) = shots.get(next_shot) {
                    if vm.clock_ms() < *t {
                        break;
                    }
                    if let Some(h) = vm.native_as::<Host>() {
                        save_png(file, &h.render())?;
                        println!("screenshot {} ({:.1} s)", file.display(), vm.clock_ms() as f64 / 1000.0);
                    }
                    next_shot += 1;
                }
                if let Some(&(x, y, t)) = clicks.get(next_click) {
                    if vm.clock_ms() >= t {
                        println!("click at {x},{y} ({:.1} s)", vm.clock_ms() as f64 / 1000.0);
                        vm.set_input_cursor(x, y);
                        button_down = true;
                        next_click += 1;
                    }
                }
                if let Some((x, y, from)) = auto_click {
                    // The text task waits on its own (TASK E=0 on the
                    // writer), so the VM cannot tell a click wait from a
                    // text being shown: click every 1.5 s.
                    let last = *waiting_since.get_or_insert(from);
                    if vm.clock_ms() >= from && vm.clock_ms() >= last + 1500 && !button_down {
                        println!("auto-click at {x},{y} ({:.1} s)", vm.clock_ms() as f64 / 1000.0);
                        vm.set_input_cursor(x, y);
                        button_down = true;
                        waiting_since = Some(vm.clock_ms());
                    }
                }
                // A click holds the left button for one frame.
                vm.update_mouse_buttons(button_down, false, false)?;
                button_down = false;
                if let Err(e) = vm.resume(ResumeResponse::Continue) {
                    break format!("resume error: {e}");
                }
            }
            VmSuspend::Complete => break "script complete".into(),
            VmSuspend::Error(m) => break format!("halted: {m}"),
        }
    };
    if let Some(o) = out.as_mut() {
        o.flush()?;
    }
    if let Some(w) = wav.take() {
        let secs = w.finish()?;
        println!("sound output: {secs:.1} s");
    }

    println!();
    println!(
        "stopped: {stop}\n  at yst{:05} pc {}, {} groups, {} waits, {:.1} s simulated",
        vm.script_id(),
        vm.pc(),
        vm.executed_groups(),
        frames,
        vm.clock_ms() as f64 / 1000.0
    );
    // Where the run stopped, as label + offset for each GOSUB return point.
    let labels = fs
        .read("ysbin\\ysl.ybn")
        .ok()
        .and_then(|b| yuris_format::yslb::YslbTable::from_bytes(&b).ok());
    let place = |sid: u16, pc: usize| -> String {
        let best = labels.as_ref().and_then(|t| {
            t.labels()
                .iter()
                .filter(|l| l.script_id == sid && l.target_pc as usize <= pc)
                .max_by_key(|l| l.target_pc)
        });
        match best {
            Some(l) => format!(
                "yst{sid:05} pc {pc} ({}+{})",
                String::from_utf8_lossy(&l.name),
                pc - l.target_pc as usize
            ),
            None => format!("yst{sid:05} pc {pc}"),
        }
    };
    for (sid, pc, n) in vm.watch_counts() {
        println!("watch {}: {n}", place(sid, pc));
    }
    if !args.vars.is_empty() {
        let vals: Vec<String> = args
            .vars
            .iter()
            .map(|&id| match vm.global_int(id) {
                Some(n) => format!("@{id}={n}"),
                None => format!("@{id}=?"),
            })
            .collect();
        println!("vars: {}", vals.join(" "));
    }
    for d in &args.dumps {
        let (space, id) = match (d.chars().next(), d[1..].parse::<u16>()) {
            (Some('$'), Ok(id)) => (yuris_value::VarSpace::Dollar, id),
            (Some('@'), Ok(id)) => (yuris_value::VarSpace::At, id),
            _ => anyhow::bail!("--dump wants $ID or @ID, got {d:?}"),
        };
        let r = yuris_value::VarRef { space, id };
        let show = |v: &yuris_value::Value| match v {
            yuris_value::Value::Str(b) => format!("{:?}", text_enc.decode(b).0),
            other => format!("{other:?}"),
        };
        if let Some(a) = vm.store().array(&r) {
            println!("{d} {:?}:", a.bounds);
            let last = *a.bounds.last().unwrap_or(&1) as usize;
            for (i, v) in a.data.iter().enumerate() {
                let empty = matches!(v, yuris_value::Value::Int(0))
                    || matches!(v, yuris_value::Value::Str(b) if b.is_empty());
                if !empty {
                    println!("  [{}][{}] {}", i / last.max(1), i % last.max(1), show(v));
                }
            }
        } else {
            match vm.store().get_opt(&r) {
                Some(v) => println!("{d} = {}", show(v)),
                None => println!("{d} undefined"),
            }
        }
    }
    if let Some(h) = vm.native_as::<Host>() {
        // Shots asked for after the run stopped are taken at the end.
        for (file, _) in &shots[next_shot..] {
            save_png(file, &h.render())?;
            println!("screenshot {} (end of run)", file.display());
        }
        if let Some(filter) = &args.cgs {
            println!("layers matching {filter:?} (path E A x y z w h file):");
            for (id, l) in h.tree.iter() {
                let path = h.encoding.decode(&h.tree.path_of(id)).0.into_owned();
                if path.contains(filter.as_str()) {
                    let file = h.encoding.decode(&l.file).0.into_owned();
                    println!(
                        "  {path:<44} {} {:>3} {:>5} {:>5} {:>8} {:>5} {:>5} {file}",
                        u8::from(l.enabled),
                        l.alpha,
                        l.x,
                        l.y,
                        l.z,
                        l.width(),
                        l.height()
                    );
                }
            }
        }
        if !h.notes.is_empty() {
            println!("host notes ({}):", h.notes.len());
            for n in h.notes.iter().take(40) {
                println!("  {n}");
            }
        }
    }
    if !tail.is_empty() {
        println!("last {} events:", tail.len());
        for line in &tail {
            let line: String = line.chars().take(240).collect();
            println!("  {line}");
        }
    }
    if vm.has_tasks() {
        println!("tasks (call stacks, innermost last):");
        for t in vm.task_stacks() {
            let name = if t.name.is_empty() { "(main)" } else { t.name.as_str() };
            let mut state = Vec::new();
            if t.running {
                state.push("running".to_string());
            }
            if !t.enabled {
                state.push("paused".to_string());
            }
            if t.wait_frames > 0 {
                state.push(format!("waits {} frames", t.wait_frames));
            }
            if t.text_wait {
                state.push("waits on its text".to_string());
            }
            if t.wait_until_ms > vm.clock_ms() {
                state.push(format!("waits {} ms", t.wait_until_ms - vm.clock_ms()));
            }
            println!("  {name} Z={} {}", t.z, state.join(", "));
            for (sid, pc) in t.stack {
                println!("    {}", place(sid, pc));
            }
        }
    } else {
        println!("call stack (innermost last):");
        for (sid, pc) in vm.call_stack() {
            println!("  {}", place(sid, pc.saturating_sub(1)));
        }
        println!("  {}", place(vm.script_id(), vm.pc()));
    }
    println!("events:");
    for (k, n) in KINDS.iter().zip(kinds) {
        if n > 0 {
            println!("  {k:<14} {n}");
        }
    }
    if !unsupported.is_empty() {
        println!("unsupported commands (count, first reason):");
        let mut v: Vec<_> = unsupported.into_iter().collect();
        v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
        for (name, (n, why)) in v {
            let why: String = why.chars().take(100).collect();
            println!("  {name:<12} {n:>7}  {why}");
        }
    }
    if !subsystem.is_empty() {
        println!("recorded-only commands (no backend yet):");
        for (name, n) in subsystem {
            println!("  {name:<12} {n:>7}");
        }
    }
    Ok(())
}

const KINDS: [&str; 13] = [
    "GroupExecuted", "Jump", "Call", "Text", "Native", "VarQuery", "Declaration", "Load", "Save",
    "ScriptSwitch", "Unsupported", "Subsystem", "TextOut",
];

/// Index into [`KINDS`].
/// `@_TIME` fields for a Unix time in ms. UTC: the headless tool has no
/// time zone database.
fn date_time(ms: i64) -> [i64; 8] {
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

fn kind(e: &VmEvent) -> usize {
    match e {
        VmEvent::GroupExecuted { .. } => 0,
        VmEvent::Jump { .. } => 1,
        VmEvent::Call { .. } => 2,
        VmEvent::Text { .. } => 3,
        VmEvent::Native { .. } => 4,
        VmEvent::VarQuery { .. } => 5,
        VmEvent::Declaration { .. } => 6,
        VmEvent::Load { .. } => 7,
        VmEvent::Save { .. } => 8,
        VmEvent::ScriptSwitch { .. } => 9,
        VmEvent::Unsupported { .. } => 10,
        VmEvent::Subsystem { .. } => 11,
        VmEvent::TextOut { .. } => 12,
    }
}

/// Writes an RGBA image as PNG.
fn save_png(path: &std::path::Path, img: &yuris_format::vfs::RgbaImage) -> Result<()> {
    let f = std::io::BufWriter::new(std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?);
    let mut enc = png::Encoder::new(f, img.width, img.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&img.data)?;
    Ok(())
}

/// 16-bit stereo WAVE output; the header is written at the end.
struct WavWriter {
    file: std::io::BufWriter<std::fs::File>,
    frames: u64,
}

impl WavWriter {
    fn create(path: &std::path::Path) -> Result<Self> {
        let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
        file.write_all(&[0; 44])?;
        Ok(Self { file, frames: 0 })
    }

    fn write(&mut self, samples: &[f32]) -> Result<()> {
        for s in samples {
            self.file.write_all(&((s * 32767.0) as i16).to_le_bytes())?;
        }
        self.frames += samples.len() as u64 / 2;
        Ok(())
    }

    /// Writes the header; returns the length in seconds.
    fn finish(mut self) -> Result<f64> {
        use std::io::{Seek, SeekFrom};
        let data = (self.frames * 4) as u32;
        let mut h = Vec::with_capacity(44);
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&(36 + data).to_le_bytes());
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&16u32.to_le_bytes());
        h.extend_from_slice(&1u16.to_le_bytes());
        h.extend_from_slice(&2u16.to_le_bytes());
        h.extend_from_slice(&OUTPUT_RATE.to_le_bytes());
        h.extend_from_slice(&(OUTPUT_RATE * 4).to_le_bytes());
        h.extend_from_slice(&4u16.to_le_bytes());
        h.extend_from_slice(&16u16.to_le_bytes());
        h.extend_from_slice(b"data");
        h.extend_from_slice(&data.to_le_bytes());
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&h)?;
        self.file.flush()?;
        Ok(self.frames as f64 / f64::from(OUTPUT_RATE))
    }
}
