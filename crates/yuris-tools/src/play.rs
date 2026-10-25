//! `yuris play`: runs a game the way an app does (`yuris-player` stepped
//! at 60 Hz, frames drawn by the GPU compositor), headless. Screenshots
//! come from the GPU; `--compare` checks each against the CPU compositor.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use yuris_player::{Config, Encoding, Player, Status};
use yuris_render::Renderer;
use yuris_vm::MouseButton;

/// Options for `yuris play`.
pub struct PlayArgs {
    pub dir: PathBuf,
    pub launcher: Option<PathBuf>,
    pub save_dir: Option<PathBuf>,
    pub fonts: Vec<PathBuf>,
    pub encoding: String,
    /// Game time to run, in seconds.
    pub seconds: u64,
    /// Clicks, `X,Y@MS`.
    pub clicks: Vec<String>,
    /// `X,Y@MS`: from MS on, click at X,Y every 1.5 s.
    pub auto_click: Option<String>,
    /// Screenshots, `FILE.png@MS`.
    pub shots: Vec<String>,
    /// Compare each screenshot with the CPU compositor.
    pub compare: bool,
}

fn point_at(s: &str, what: &str) -> Result<(i64, i64, u64)> {
    s.split_once('@')
        .and_then(|(xy, t)| {
            let (x, y) = xy.split_once(',')?;
            Some((x.trim().parse().ok()?, y.trim().parse().ok()?, t.trim().parse().ok()?))
        })
        .with_context(|| format!("{what} wants X,Y@MS, got {s:?}"))
}

pub fn run(args: PlayArgs) -> Result<()> {
    let config = Config {
        game_dir: args.dir.clone(),
        launcher: args.launcher.clone(),
        save_dir: args.save_dir.clone(),
        fonts: args.fonts.iter().map(yuris_player::FontFile::new).collect(),
        encoding: match args.encoding.to_ascii_lowercase().as_str() {
            "gbk" | "gb2312" | "cp936" => Encoding::Gbk,
            "utf8" | "utf-8" => Encoding::Utf8,
            "sjis" | "shift-jis" | "shiftjis" | "cp932" => Encoding::ShiftJis,
            _ => Encoding::Auto,
        },
        utc_offset_ms: 0,
    };
    let mut player = Player::open(&config)?;
    let screen = player.screen();
    println!("screen: {}x{}", screen.0, screen.1);
    let mut renderer = Renderer::headless().context("no GPU adapter (on Linux, install Mesa's lavapipe)")?;

    let mut clicks = args.clicks.iter().map(|c| point_at(c, "--click")).collect::<Result<Vec<_>>>()?;
    clicks.sort_by_key(|c| c.2);
    let auto_click = args.auto_click.as_deref().map(|c| point_at(c, "--auto-click")).transpose()?;
    let mut shots: Vec<(PathBuf, u64)> = Vec::new();
    for sh in &args.shots {
        match sh.rsplit_once('@').and_then(|(f, t)| Some((PathBuf::from(f), t.trim().parse().ok()?))) {
            Some(p) => shots.push(p),
            None => bail!("--shot wants FILE.png@MS, got {sh:?}"),
        }
    }
    shots.sort_by_key(|s| s.1);

    let (mut next_click, mut next_shot) = (0, 0);
    let mut last_auto = 0u64;
    let mut release = false;
    let mut clock = 0u64;
    let mut frames = 0u64;
    let mut vm_time = Duration::ZERO;
    let mut gpu_time = Duration::ZERO;
    let mut slowest = Duration::ZERO;
    let mut drawn = None;
    let end = args.seconds * 1000;
    while clock < end {
        // 60 Hz: 16 or 17 ms, so 60 frames take a second.
        let dt = ((frames + 1) * 1000 / 60 - frames * 1000 / 60) as u32;
        frames += 1;
        clock += u64::from(dt);
        if release {
            player.button(MouseButton::Left, false);
            release = false;
        }
        if let Some(&(x, y, t)) = clicks.get(next_click) {
            if clock >= t {
                println!("click at {x},{y} ({:.1} s)", clock as f64 / 1000.0);
                player.pointer(x, y);
                player.button(MouseButton::Left, true);
                release = true;
                next_click += 1;
            }
        }
        if let Some((x, y, from)) = auto_click {
            if clock >= from && clock >= last_auto + 1500 && !release {
                player.pointer(x, y);
                player.button(MouseButton::Left, true);
                release = true;
                last_auto = clock;
            }
        }
        let t0 = Instant::now();
        let status = player.step(dt).clone();
        let t1 = Instant::now();
        // Draw as an app would: only when the layers changed.
        let generation = player.host().tree.generation;
        let mut frame = None;
        if drawn != Some(generation) {
            frame = Some(renderer.render_image(&player.host().tree, screen));
            drawn = Some(generation);
        }
        let t2 = Instant::now();
        vm_time += t1 - t0;
        gpu_time += t2 - t1;
        slowest = slowest.max(t1 - t0);
        while let Some((file, t)) = shots.get(next_shot) {
            if clock < *t {
                break;
            }
            let img = match frame.take() {
                Some(f) => f,
                None => renderer.render_image(&player.host().tree, screen),
            };
            crate::boot::save_png(file, &img)?;
            print!("screenshot {} ({:.1} s)", file.display(), clock as f64 / 1000.0);
            if args.compare {
                let cpu = player.host().render();
                let worst = cpu
                    .data
                    .iter()
                    .zip(&img.data)
                    .map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs())
                    .max()
                    .unwrap_or(0);
                let off = cpu
                    .data
                    .chunks_exact(4)
                    .zip(img.data.chunks_exact(4))
                    .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| (i32::from(*x) - i32::from(*y)).abs() > 4))
                    .count();
                print!("  vs CPU: largest difference {worst}, {off} pixels over 4");
            }
            println!();
            next_shot += 1;
        }
        match status {
            Status::Running => {}
            Status::Ended => {
                println!("the game ended ({:.1} s)", clock as f64 / 1000.0);
                break;
            }
            Status::Failed(m) => bail!("stopped at {:.1} s: {m}", clock as f64 / 1000.0),
        }
    }
    let n = frames.max(1) as u32;
    println!(
        "{frames} frames, {:.1} s game time; per frame: scripts {:.2} ms (slowest {:.1} ms), drawing {:.2} ms; {} textures",
        clock as f64 / 1000.0,
        (vm_time / n).as_secs_f64() * 1000.0,
        slowest.as_secs_f64() * 1000.0,
        (gpu_time / n).as_secs_f64() * 1000.0,
        renderer.cached_images()
    );
    Ok(())
}
