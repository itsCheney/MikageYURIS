//! `yuris`: YU-RIS development tools (no GUI).
//!
//! Subcommands:
//!
//! ```text
//! yuris boot     <game dir> [--launcher EXE] [--key-hex HEX] [--max-groups N]
//! yuris evb      <exe or package>
//! yuris ypf list     <bn.ypf> [--limit N]
//! yuris ypf extract  <bn.ypf> (--name NAME | --index N) [-o OUT]
//! yuris ystb guess-key <bn.ypf> (--name NAME | --index N)
//! yuris ystb info    <bn.ypf> (--name NAME | --index N) [--key-hex HEX]
//! yuris ystb slots   <bn.ypf> (--name NAME | --index N) [--key-hex HEX] [--limit N]
//! yuris ystb groups  <bn.ypf> (--name NAME | --index N) [--from N] [--count N]
//! yuris yscf         <bn.ypf> --name '$ysbin\yscfg.ybn'
//! ```
//!
//! The format notes are in `docs/formats/`.

use std::path::PathBuf;

mod boot;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use yuris_format::{ypf::YpfArchive, yscf::YscfFile, ystb::{guess_key, YstbFile}};

#[derive(Parser)]
#[command(name = "yuris", version, about = "YU-RIS format inspector")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Opens a game folder and runs the boot chain headless, reporting
    /// versions, the script key and unimplemented commands.
    Boot {
        /// Game folder (or its pac folder).
        dir: PathBuf,
        /// Launcher exe packed with Enigma Virtual Box (translation patches);
        /// its files and external package are mounted over the game.
        #[arg(long)]
        launcher: Option<PathBuf>,
        /// 4-byte script key in hex; inferred from the scripts by default.
        #[arg(long)]
        key_hex: Option<String>,
        /// Entry label (SYSTEM_START by default).
        #[arg(long)]
        entry: Option<String>,
        /// Group budget (the main loop yields every frame; without a backend
        /// this is what stops the run).
        #[arg(long, default_value_t = 200_000)]
        max_groups: usize,
        /// Writes the VM event stream as JSONL.
        #[arg(long)]
        jsonl: Option<PathBuf>,
        /// Stops at the first unimplemented command (by default it is recorded
        /// and skipped).
        #[arg(long)]
        strict: bool,
    },
    /// List the files of an Enigma Virtual Box exe or external package.
    Evb {
        /// Packed exe or package (e.g. `.mys`).
        path: PathBuf,
    },
    /// YPF archives.
    Ypf {
        #[command(subcommand)]
        sub: YpfCmd,
    },
    /// YSTB scripts.
    Ystb {
        #[command(subcommand)]
        sub: YstbCmd,
    },
    /// YSCM command table.
    Yscm {
        /// Path to bn.ypf.
        path: PathBuf,
        /// Entry name (default `ysbin\ysc.ybn`).
        #[arg(long)]
        name: Option<String>,
        /// Entry index.
        #[arg(long)]
        index: Option<usize>,
        /// Only list command names and parameter counts.
        #[arg(long)]
        list: bool,
        /// Only show the parameters of one command.
        #[arg(long)]
        cmd: Option<String>,
    },
    /// YSCF project config.
    Yscf {
        /// Path to bn.ypf.
        path: PathBuf,
        /// Entry name (such as `ysbin\yscfg.ybn`), or use --index.
        #[arg(long)]
        name: Option<String>,
        /// Entry index.
        #[arg(long)]
        index: Option<usize>,
    },
}

#[derive(Subcommand)]
enum YpfCmd {
    /// Lists all entries.
    List {
        /// Path to bn.ypf.
        path: PathBuf,
        /// Only show the first N.
        #[arg(long)]
        limit: Option<usize>,
        /// Name XOR key (inferred from the archive by default).
        #[arg(long)]
        key: Option<u8>,
    },
    /// Extracts an entry.
    Extract {
        /// Path to bn.ypf.
        path: PathBuf,
        /// Entry name.
        #[arg(long)]
        name: Option<String>,
        /// Entry index.
        #[arg(long)]
        index: Option<usize>,
        /// Output file (stdout by default).
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum YstbCmd {
    /// Guesses the script XOR key.
    GuessKey {
        /// Path to bn.ypf.
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        index: Option<usize>,
    },
    /// Prints the header and slot statistics.
    Info {
        /// Path to bn.ypf.
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        index: Option<usize>,
        /// 4-byte key (8 hex characters).
        #[arg(long)]
        key_hex: Option<String>,
    },
    /// Prints the slot table.
    Slots {
        /// Path to bn.ypf.
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        index: Option<usize>,
        #[arg(long)]
        key_hex: Option<String>,
        /// Only show the first N.
        #[arg(long, default_value_t = 24)]
        limit: usize,
    },
    /// Lists command groups: group number, command name (from this game's
    /// ysc.ybn), windows and their instructions.
    Groups {
        /// Path to bn.ypf.
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        index: Option<usize>,
        #[arg(long)]
        key_hex: Option<String>,
        /// First group.
        #[arg(long, default_value_t = 0)]
        from: usize,
        /// Number of groups.
        #[arg(long, default_value_t = 40)]
        count: usize,
    },
    /// Disassembles content windows (unknown opcodes stay raw).
    Disasm {
        /// Path to bn.ypf.
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        index: Option<usize>,
        #[arg(long)]
        key_hex: Option<String>,
        /// Only show the first N windows.
        #[arg(long, default_value_t = 12)]
        limit: usize,
    },
}

fn resolve_name(ypf: &YpfArchive, name: &Option<String>, index: &Option<usize>) -> Result<String> {
    if let Some(n) = name {
        return Ok(n.clone());
    }
    if let Some(i) = index {
        return ypf
            .entries()
            .get(*i)
            .map(|e| e.name_lossy())
            .with_context(|| format!("index {i} out of range"));
    }
    bail!("need --name or --index");
}

fn open_ypf(path: &PathBuf, name_key: Option<u8>) -> Result<YpfArchive> {
    let data = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    YpfArchive::from_bytes(data, name_key).map_err(Into::into)
}

fn get_script(ypf: &YpfArchive, name: &str) -> Result<Vec<u8>> {
    ypf.read(name).with_context(|| format!("read entry {name}"))
}

fn parse_key(hex: &str) -> Result<[u8; 4]> {
    let b = hex::decode(hex).context("key must be hex")?;
    if b.len() != 4 {
        bail!("key must be exactly 4 bytes (8 hex chars)");
    }
    Ok([b[0], b[1], b[2], b[3]])
}

/// Minimal hex decoding (avoids a dependency).
mod hex {
    /// Decodes hex.
    pub fn decode(s: &str) -> anyhow::Result<Vec<u8>> {
        let s = s.trim();
        if s.len() % 2 != 0 {
            anyhow::bail!("odd hex length");
        }
        let mut out = Vec::with_capacity(s.len() / 2);
        for i in 0..s.len() / 2 {
            out.push(u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)?);
        }
        Ok(out)
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.cmd {
        Cmd::Evb { path } => {
            let pkg = yuris_format::evb::EvbPackage::open(&path)?
                .with_context(|| format!("{} is not an Enigma Virtual Box exe or package", path.display()))?;
            if let Some(ext) = std::fs::read(&path)
                .ok()
                .and_then(|b| yuris_format::evb::external_package_name(&b))
            {
                println!("external package: {ext}");
            }
            println!("{:>10} {:>10}  name", "size", "stored");
            for f in pkg.files() {
                println!("{:>10} {:>10}  {}", f.size, f.stored, f.name);
            }
        }
        Cmd::Boot { dir, launcher, key_hex, entry, max_groups, jsonl, strict } => {
            boot::run(boot::BootArgs {
                dir,
                launcher,
                key: key_hex.as_deref().map(parse_key).transpose()?,
                entry,
                max_groups,
                jsonl,
                strict,
            })?;
        }
        Cmd::Ypf { sub } => match sub {
            YpfCmd::List { path, limit, key } => {
                let ypf = open_ypf(&path, key)?;
                let h = ypf.header();
                let idx = ypf.index();
                println!(
                    "version={} count={} header_size={:#x} name_key={:#04x} wide_offsets={} name_hash={:?}",
                    h.version,
                    h.file_count,
                    h.header_size,
                    idx.name_key,
                    idx.wide_offsets,
                    idx.name_hash
                );
                let entries = ypf.entries();
                let n = limit.unwrap_or(entries.len());
                println!("{:>4}  {:<24} {:>4} {:>4} {:>9} {:>9} {:>9}", "#", "name", "type", "zlib", "uncomp", "stored", "offset");
                for (i, e) in entries.iter().take(n).enumerate() {
                    println!(
                        "{:>4}  {:<24} {:>4} {:>4} {:>9} {:>9} {:>9}",
                        i,
                        e.name_lossy().replace('\\', "/"),
                        e.file_type,
                        e.compression,
                        e.uncompressed_len,
                        e.stored_len,
                        format!("{:#x}", e.offset)
                    );
                }
            }
            YpfCmd::Extract { path, name, index, out } => {
                let ypf = open_ypf(&path, None)?;
                let name = resolve_name(&ypf, &name, &index)?;
                let data = get_script(&ypf, &name)?;
                match out {
                    Some(p) => {
                        std::fs::write(&p, &data)?;
                        println!("wrote {} bytes -> {}", data.len(), p.display());
                    }
                    None => {
                        use std::io::Write;
                        std::io::stdout().write_all(&data)?;
                    }
                }
            }
        },
        Cmd::Ystb { sub } => match sub {
            YstbCmd::GuessKey { path, name, index } => {
                let ypf = open_ypf(&path, None)?;
                let name = resolve_name(&ypf, &name, &index)?;
                let blob = get_script(&ypf, &name)?;
                match guess_key(&blob) {
                    Ok((k, score)) => println!(
                        "key = {}  score = {:.4}  (key_score = max(contiguity, window-closure), 1.0 = perfect)",
                        k.iter().map(|b| format!("{b:02x}")).collect::<String>(),
                        score
                    ),
                    Err(e) => bail!("guess failed: {e}"),
                }
            }
            YstbCmd::Info { path, name, index, key_hex } => {
                let ypf = open_ypf(&path, None)?;
                let name = resolve_name(&ypf, &name, &index)?;
                let blob = get_script(&ypf, &name)?;
                let key = match key_hex {
                    Some(h) => parse_key(&h)?,
                    None => guess_key(&blob)?.0,
                };
                let f = YstbFile::from_bytes(&blob, key)?;
                let h = f.header();
                println!("key = {}", kstr(&key));
                println!(
                    "version={} unknown1={} unknown2={}",
                    h.version, h.unknown1, h.unknown2
                );
                println!(
                    "part1={} command={} content={} part4={}",
                    h.part1_len, h.command_len, h.content_len, h.part4_len
                );
                println!("slots={} contiguity={:.4}", f.slots().len(), f.contiguity_score());
                let mut hist: std::collections::BTreeMap<u32, usize> = Default::default();
                for s in f.slots() {
                    *hist.entry(s.tag).or_default() += 1;
                }
                println!("tag histogram:");
                for (t, n) in hist {
                    println!("  0x{t:08X}  x{n}");
                }
            }
            YstbCmd::Slots { path, name, index, key_hex, limit } => {
                let ypf = open_ypf(&path, None)?;
                let name = resolve_name(&ypf, &name, &index)?;
                let blob = get_script(&ypf, &name)?;
                let key = match key_hex {
                    Some(h) => parse_key(&h)?,
                    None => guess_key(&blob)?.0,
                };
                let f = YstbFile::from_bytes(&blob, key)?;
                println!("{:>4}  {:<12} {:>8} {:>8}", "#", "tag", "len", "offset");
                for (i, s) in f.slots().iter().take(limit).enumerate() {
                    println!(
                        "{:>4}  0x{:<10} {:>8} {:>8}",
                        i,
                        format!("{:08X}", s.tag),
                        s.len,
                        s.offset
                    );
                }
            }
            YstbCmd::Groups { path, name, index, key_hex, from, count } => {
                let ypf = open_ypf(&path, None)?;
                let name = resolve_name(&ypf, &name, &index)?;
                let blob = get_script(&ypf, &name)?;
                let key = match key_hex {
                    Some(h) => parse_key(&h)?,
                    None => guess_key(&blob)?.0,
                };
                let f = YstbFile::from_bytes(&blob, key)?;
                let yscm = ypf
                    .read("ysbin\\ysc.ybn")
                    .ok()
                    .and_then(|b| yuris_format::yscm::YscmTable::from_bytes(&b).ok());
                let groups = f.groups()?;
                let first = f.group_first_slots(&groups);
                for (pc, g) in groups.iter().enumerate().skip(from).take(count) {
                    let cmd = yscm.as_ref().and_then(|t| t.commands().get(g.command_type as usize));
                    println!(
                        "{pc:>5}  {:<12} param={:#06x}",
                        cmd.map(|c| c.name.as_str()).unwrap_or("?"),
                        g.param
                    );
                    for w in f.group_windows(first[pc], g) {
                        let slot = (w.tag & 0xff) as usize;
                        let pname = cmd
                            .and_then(|c| c.params.get(slot))
                            .map(|p| p.name.as_str())
                            .unwrap_or("");
                        let text = match f.window_bytes_pooled(w) {
                            Some(bytes) => match yuris_script::decode_window(bytes) {
                                Ok(ins) => ins.iter().map(fmt_insn).collect::<Vec<_>>().join(" "),
                                Err(e) => format!("<{e}>"),
                            },
                            None => "<out of pool>".into(),
                        };
                        println!("         [{slot:>2} {pname:<8}] tag={:08x} {text}", w.tag);
                    }
                }
            }
            YstbCmd::Disasm { path, name, index, key_hex, limit } => {
                let ypf = open_ypf(&path, None)?;
                let name = resolve_name(&ypf, &name, &index)?;
                let blob = get_script(&ypf, &name)?;
                let key = match key_hex {
                    Some(h) => parse_key(&h)?,
                    None => guess_key(&blob)?.0,
                };
                let f = YstbFile::from_bytes(&blob, key)?;
                println!("key = {}", kstr(&key));
                let mut shown = 0usize;
                let mut unknown_ops: std::collections::BTreeMap<u8, usize> = Default::default();
                for (i, s) in f.slots().iter().enumerate() {
                    if shown >= limit {
                        break;
                    }
                    let Ok(w) = f.slot_content(s) else {
                        println!("slot #{i:<4} tag=0x{:08X} off={:<6} len={:<4} <out-of-range special record>", s.tag, s.offset, s.len);
                        shown += 1;
                        continue;
                    };
                    let instrs = yuris_script::decode_window(w)
                        .with_context(|| format!("slot #{i}"))?;
                    println!(
                        "slot #{i:<4} tag=0x{:08X} off={:<6} len={:<4} instrs={}",
                        s.tag,
                        s.offset,
                        s.len,
                        instrs.len()
                    );
                    for ins in instrs {
                        let detail = match &ins.kind {
                            yuris_script::Insn::PushInt(v) => format!("{v}"),
                            yuris_script::Insn::PushFloat(v) => format!("{v}"),
                            yuris_script::Insn::PushStr(p) => {
                                format!("{:?}", String::from_utf8_lossy(p))
                            }
                            yuris_script::Insn::PushVar(r)
                            | yuris_script::Insn::PushVarRef(r)
                            | yuris_script::Insn::PushVarIndexed(r) => {
                                let tag = match r.space {
                                    yuris_script::VarSpace::At => "@",
                                    yuris_script::VarSpace::Dollar => "$",
                                    yuris_script::VarSpace::Hash => "#",
                                    yuris_script::VarSpace::Backtick => "`",
                                    yuris_script::VarSpace::Unknown(_) => "?",
                                };
                                format!("{}{}", tag, r.id)
                            }
                            yuris_script::Insn::Unknown { raw_op, operand } => {
                                *unknown_ops.entry(*raw_op).or_default() += 1;
                                format!(
                                    "op={:#04x} operand={}",
                                    raw_op,
                                    operand.iter().map(|b| format!("{b:02x}")).collect::<String>()
                                )
                            }
                            _ => String::new(),
                        };
                        println!(
                            "    +{:<4} {:<11} {}",
                            ins.offset,
                            ins.kind.mnemonic(),
                            detail
                        );
                    }
                    shown += 1;
                }
                if !unknown_ops.is_empty() {
                    println!("unknown opcodes: {unknown_ops:?}  (see docs/opcode/opcode-table.md)");
                }
            }
        },
        Cmd::Yscm { path, name, index, list, cmd } => {
            let ypf = open_ypf(&path, None)?;
            let entry_name = match (name, index) {
                (Some(n), _) => n,
                (None, Some(i)) => resolve_name(&ypf, &None, &Some(i))?,
                (None, None) => "ysbin\\ysc.ybn".to_string(),
            };
            let blob = get_script(&ypf, &entry_name)?;
            let t = yuris_format::yscm::YscmTable::from_bytes(&blob)
                .with_context(|| format!("parse {entry_name}"))?;
            println!(
                "version={} commands={} params={} body_end={:#x} tail={}",
                t.version(),
                t.commands().len(),
                t.param_total(),
                t.tail_offset,
                t.tail.len()
            );
            if let Some(c) = &cmd {
                let c = t
                    .command(c)
                    .with_context(|| format!("command {c} not found"))?;
                println!("{} ({} params)", c.name, c.params.len());
                for p in &c.params {
                    println!("  {:<14} type=0x{:04X} ({})", p.name, p.ty, p.ty);
                }
            } else if list {
                for c in t.commands() {
                    println!("{:<14} {}", c.name, c.params.len());
                }
            } else {
                for c in t.commands() {
                    let ps: Vec<String> = c
                        .params
                        .iter()
                        .map(|p| format!("{}:{}", p.name, p.ty))
                        .collect();
                    println!("[{:3}] {:<14} {}", 0, c.name, ps.join(","));
                }
            }
        }
        Cmd::Yscf { path, name, index } => {
            let ypf = open_ypf(&path, None)?;
            let name = resolve_name(&ypf, &name, &index)?;
            let blob = get_script(&ypf, &name)?;
            let f = YscfFile::from_bytes(&blob)?;
            println!("version={}", f.version);
            println!("screen = {}x{}", f.screen_width, f.screen_height);
            println!(
                "file_priority: dev={} debug={} release={}  (unpacked read = {})",
                f.file_priority_dev,
                f.file_priority_debug,
                f.file_priority_release,
                if f.unpacked_read_enabled() { "ENABLED" } else { "disabled" }
            );
            println!("image_type_slots = {:?}", f.image_type_slots);
            println!("sound_type_slots = {:?}", f.sound_type_slots);
            println!("caption = {:?}", f.caption);
        }
    }
    Ok(())
}

/// One instruction as compact text.
fn fmt_insn(ins: &yuris_script::Instruction) -> String {
    use yuris_script::{Insn, VarSpace};
    match &ins.kind {
        Insn::PushInt(v) => format!("{v}"),
        Insn::PushFloat(v) => format!("{v}f"),
        Insn::PushStr(p) => {
            let (t, _, _) = encoding_rs::SHIFT_JIS.decode(p);
            format!("{t:?}")
        }
        Insn::PushVar(r) | Insn::PushVarRef(r) | Insn::PushVarIndexed(r) => {
            let tag = match r.space {
                VarSpace::At => "@",
                VarSpace::Dollar => "$",
                VarSpace::Hash => "#",
                VarSpace::Backtick => "`",
                VarSpace::Unknown(_) => "?",
            };
            format!("{}{tag}{}", ins.kind.mnemonic(), r.id)
        }
        Insn::Unknown { raw_op, operand } => format!(
            "op{raw_op:02x}:{}",
            operand.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ),
        other => other.mnemonic().to_string(),
    }
}

fn kstr(k: &[u8; 4]) -> String {
    k.iter().map(|b| format!("{b:02x}")).collect()
}
