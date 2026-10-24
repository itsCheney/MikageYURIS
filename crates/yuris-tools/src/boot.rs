//! `yuris boot`: runs the boot chain headless and reports the game files,
//! versions and key, how far the VM gets before it stops, and which commands
//! are not implemented yet. The first step when bringing up a new game.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use yuris_format::vfs::GameFs;
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
}

fn hex(k: &[u8]) -> String {
    k.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn run(args: BootArgs) -> Result<()> {
    let fs = GameFs::open_with_launcher(&args.dir, args.launcher.as_deref())
        .with_context(|| format!("open {}", args.dir.display()))?;
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
    vm.set_strict(args.strict);
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

    let mut seen = 0usize;
    let mut frames = 0usize;
    let mut kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut unsupported: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut subsystem: BTreeMap<String, usize> = BTreeMap::new();
    let stop = loop {
        let s = match vm.run(1000) {
            Ok(s) => s,
            Err(e) => break format!("error: {e}"),
        };
        for e in &vm.events()[seen..] {
            if let Some(o) = out.as_mut() {
                writeln!(o, "{}", yuris_vm::event_json(e))?;
            }
            *kinds.entry(kind(e)).or_default() += 1;
            match e {
                VmEvent::Unsupported { command, reason, .. } => {
                    let slot = unsupported.entry(cmd_name(*command)).or_insert((0, reason.clone()));
                    slot.0 += 1;
                }
                VmEvent::Subsystem { command, .. } => {
                    *subsystem.entry(cmd_name(*command)).or_default() += 1;
                }
                _ => {}
            }
        }
        seen = vm.events().len();
        if vm.executed_groups() >= args.max_groups {
            break format!("group budget {} reached", args.max_groups);
        }
        match s {
            VmSuspend::None => {}
            VmSuspend::Wait { .. } => {
                frames += 1;
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

    println!();
    println!(
        "stopped: {stop}\n  at yst{:05} pc {}, {} groups, {} waits",
        vm.script_id(),
        vm.pc(),
        vm.executed_groups(),
        frames
    );
    println!("events:");
    for (k, n) in &kinds {
        println!("  {k:<14} {n}");
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

fn kind(e: &VmEvent) -> &'static str {
    match e {
        VmEvent::GroupExecuted { .. } => "GroupExecuted",
        VmEvent::Jump { .. } => "Jump",
        VmEvent::Call { .. } => "Call",
        VmEvent::Text { .. } => "Text",
        VmEvent::Cg { .. } => "Cg",
        VmEvent::Sound { .. } => "Sound",
        VmEvent::VarQuery { .. } => "VarQuery",
        VmEvent::Declaration { .. } => "Declaration",
        VmEvent::CgAct { .. } => "CgAct",
        VmEvent::CgInfo { .. } => "CgInfo",
        VmEvent::Load { .. } => "Load",
        VmEvent::Save { .. } => "Save",
        VmEvent::CgEnd { .. } => "CgEnd",
        VmEvent::ScriptSwitch { .. } => "ScriptSwitch",
        VmEvent::Unsupported { .. } => "Unsupported",
        VmEvent::Subsystem { .. } => "Subsystem",
    }
}
