# MikageYURIS

A YU-RIS runtime in Rust, written for [Mikage](https://github.com/itsCheney/Mikage-Next),
an iOS player for visual novels. It reads a game's original archives and
compiled scripts and runs them; nothing is converted ahead of time.

This is a fork of [Weiss-UltimateSavior/Yuris-Tyn](https://github.com/Weiss-UltimateSavior/Yuris-Tyn).
Work happens on the `mikage` branch. Upstream's desktop player, renderer and
scenario player were removed; the format readers and the VM were kept and
reworked, and the native command layer was written anew.

## Status

The test game is **Erewhon** (CLOCKUP, 2018; engine 481) with its Chinese
translation patch. Run headless, it boots through the logo and the first-run
config screen to the title, starts a new game and plays the opening and the
first scenes: text with its glow, backgrounds, event CGs, sprites placed by
`MATH3D`, BGM and sound effects. Clicks advance the text. Saving into a
slot (with its date and thumbnail) and loading it back from the title work;
the story goes on from the saved line. The backlog and skip mode work.

For an app, `yuris-player` runs the game in real time, `yuris-render`
draws its frames with wgpu (Metal on iOS), and `yuris-mobile` puts both
behind a C interface: the app gives it a view, steps it from its display
link and pulls the sound from its audio callback (see
[docs/engine/player.md](docs/engine/player.md)). The GPU frames match the
CPU compositor's: Erewhon played to its first scenes this way differs by at
most 1/255 per channel.

Not done yet:

- the app side: a Swift session in Mikage that hosts the C interface
  (`yuris-mobile`) has not been written yet;
- CGACT TRANSITION, masks, movies;
- other games and engine versions: the format readers handle 481 and 555,
  but only Erewhon has been run this far.

## Crates

| Crate | What it does |
|---|---|
| `yuris-core` | Errors, byte helpers, murmur2 |
| `yuris-format` | YPF archives, YSTB scripts and the `ysbin` tables; the game file system (`vfs`), Enigma Virtual Box packages (`evb`) |
| `yuris-value` | INT/FLT/STR values and variable storage |
| `yuris-script` | Decodes and evaluates expression bytecode |
| `yuris-vm` | The command-level VM: control flow, tasks, text, input, MATH/MATH3D, boot chain |
| `yuris-host` | Native commands: CG layer tree and CPU compositor, fonts and text, sound mixer, window queries |
| `yuris-render` | GPU compositor (wgpu): draws the layer tree like the CPU one |
| `yuris-player` | Runs a game in real time: frame stepping, clock, input |
| `yuris-mobile` | C interface for apps (`platform/ios/include/yuris_mobile.h`), static or dynamic library |
| `yuris-tools` | The `yuris` command line tool |

## Building

```sh
cargo build --release
cargo test
```

Rust 2021. Tests that need game files skip themselves when the files are
absent; the GPU tests skip without a GPU (on Linux, Mesa's lavapipe,
`mesa-vulkan-drivers`, is enough).

For iOS: `cargo build --release -p yuris-mobile --target aarch64-apple-ios`
gives `libyuris_mobile.a` and `.dylib`; the header is
`platform/ios/include/yuris_mobile.h`.

## The `yuris` tool

```sh
yuris ypf list <archive.ypf>
yuris ypf extract <archive.ypf> --name <entry> -o <file>
yuris ystb info|slots|groups|disasm ...       # script containers
yuris yscm <bn.ypf> [--cmd SOUND]             # the game's command table
yuris yscf <bn.ypf>                           # project config
yuris evb <launcher.exe|package.mys> [--extract NAME -o FILE]
yuris boot <game folder> [options]            # run the game headless
yuris play <game folder> [options]            # run it as the app does, GPU frames
```

`boot` runs the game with a simulated clock (one frame = 1/60 s) and reports
where it stopped, which commands are not implemented yet, and the task call
stacks. Useful options:

| Option | Effect |
|---|---|
| `--launcher <exe>` | Mounts a translation patch packed with Enigma Virtual Box |
| `--max-groups N` | Command group budget; the run stops there |
| `--click X,Y@MS` | Clicks once at game time MS (repeatable) |
| `--auto-click X,Y@MS` | From MS on, clicks every 1.5 s |
| `--shot FILE.png@MS` | Saves the screen at MS (repeatable) |
| `--wav FILE` | Records the mixed sound output |
| `--save-dir DIR` | Save folder (default: the game's `save`) |
| `--font FILE --encoding gbk` | Font for text drawing, and the scripts' text encoding |
| `--cgs TEXT`, `--vars`, `--dump` | Layer and variable reports at the end |
| `--jsonl FILE --trace` | The VM's event stream |

`play` runs the real-time player at 60 Hz and draws with the GPU compositor
(`--seconds N`, `--click`, `--auto-click`, `--shot`, `--font`, `--encoding`,
`--launcher`, `--save-dir` as above); `--compare` checks each screenshot
against the CPU compositor. It also prints the time scripts and drawing
take per frame.

Erewhon (Chinese patch), from the title to the first scenes:

```sh
yuris boot <Erewhon> --launcher <Erewhon>/erewhon_chs.exe \
  --font wqy-zenhei.ttc --encoding gbk --max-groups 520000000 \
  --click 576,609@33000 --click 1100,90@125000 --auto-click 576,620@130000 \
  --shot scene.png@245000 --wav opening.wav
```

## Documentation

- [docs/formats/ypf.md](docs/formats/ypf.md): YPF archives
- [docs/formats/ystb.md](docs/formats/ystb.md): YSTB script containers and their encryption
- [docs/formats/ysbin.md](docs/formats/ysbin.md): the other `ysbin` tables (YSCM, YSLB, YSVR, YSTL, YSCF, YSTD, YSER) and save data (YSSD)
- [docs/formats/evb.md](docs/formats/evb.md): Enigma Virtual Box packages used by translation patches
- [docs/engine/bytecode.md](docs/engine/bytecode.md): expression bytecode
- [docs/engine/vm.md](docs/engine/vm.md): how scripts run
- [docs/engine/host.md](docs/engine/host.md): native commands (layers, text, sound)
- [docs/engine/player.md](docs/engine/player.md): real-time player, GPU compositor and the C interface for apps
- [docs/engine/reversing.md](docs/engine/reversing.md): engine addresses and how the undocumented parts were found

The source carries most details at the place they are used; the module
comments of `yuris-vm` (`tasks`, `text`, `input`, `math`, `commands`) and
`yuris-host` (`audio`, `layers`, `text`) are the reference for those parts.

## Legal

This repository contains no game data, no engine binaries and no code
decompiled from them. You need your own copy of a game to run it. YU-RIS
and the games belong to their owners.

`shimamura-sakura/yuri` and VNTranslationTools' notes were read for format
facts only; no code was taken from them. `crates/yuris-format/src/evb.rs`
comes from ethornell-rs and stays under MPL-2.0.

## License

GPL-3.0-or-later (see [LICENSE](LICENSE)), except `evb.rs` (MPL-2.0).
