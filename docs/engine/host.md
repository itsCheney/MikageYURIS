# Native commands

Code: `crates/yuris-host`. `Host` implements the VM's `Native` trait (see
[vm.md](vm.md)) for the commands that act on the screen and the sound:

| Command | Handled by |
|---|---|
| CG, CGEND, CGINFO, CGACT | `layers.rs` (tree), `ops.rs` (image operations), `compose.rs` (drawing) |
| FONT, FONTINFO, CGACT TEXT | `text.rs` |
| SOUND, SOUNDEND, SOUNDINFO | `audio.rs` |
| WINDOWINFO | `lib.rs`: the screen size and caption from YSCF |

The semantics come from the official manual where it says something and
from what Erewhon's scripts need where it does not; each module comment
says which choices are our own. Missing files and unimplemented operations
are noted once each in `Host::notes`, which `yuris boot` prints.

## Layers

Layers are addressed by path: `/` separates sets, a trailing `/` names a
set, `.` and `..` work as in file paths and a leading `:` is the root. The
root holds `W0`, the main window, which is also the current path, so
`CG ID=ABC` creates `:/W0/ABC`. A set composes its children by Z (higher in
front) and is then drawn with its own opacity and mode; a set with a size
clips its children.

- **CG** creates a layer (FILE, or a set path) or changes one; FILE on an
  existing layer swaps the image and keeps the attributes. `FILE=""` with
  SX and SY makes a black canvas of that size; Erewhon draws its text glow
  into one.
- **CGINFO** answers the attributes, the image size, and ONMOUSE (the
  pixel under the cursor), which Erewhon's buttons use for hit tests.
- **CGACT** runs image operations: COPY, SWAP, SIZE, RESIZE, TRIM,
  ROTATION, MERGE, the colour filters (SAIDO, MEIDO, AKARUSA, MONO,
  REVERSE), flips, MOSAIC, the blurs, the RECTPAINTs and TEXT. Filter
  values use 0x80 as 100%, after the manual's MEIDO example. Blurs weight
  by alpha. TRANSITION is not done yet.

Images are PNG, or YCG (the engine's own format: zlib strips of BGRA rows,
used by Erewhon's Chinese UI images under `.png` names). Names may leave
out the extension.

## Drawing

`compose::render_window` draws the tree into an RGBA frame on the CPU, with
the blend modes normal, add, subtract, multiply, screen, lighten, darken and
difference. Rule transitions (TID, T, TA) scale each pixel by how far the
progress is past its rule level, with a TA-wide soft edge, so dark rule
areas show first (the usual convention; not reversed from the engine).
Masks (MID) draw as normal for now.
This is the reference path and what `yuris boot --shot` saves. Players
draw with the GPU compositor in `yuris-render`, which follows it (see
[player.md](player.md)).

## Text

FONT settings are kept in slots by BNO (Erewhon keeps several apart); a FONT
without BNO changes slot 0. The player registers the faces (the system's
CJK fonts), and a face the game asks for that is not registered falls back
to the first one.

**CGACT TEXT** draws text into a layer. Its SET value is not documented; it
was read from Erewhon's glyph routine:

| SET | Copies drawn |
|---|---|
| 0 | once |
| 1 to 10 | at the 9 offsets −SET..SET in steps of SET (an outline) |
| 11 and up | at (d,0), (0,d), (d,d) with d = SET − 10 (a drop shadow) |

Erewhon's message window images are fully transparent; the dark glow behind
the text is drawn by the script: it draws the text in black with TEXT SET 3,
5 or 7 into a canvas, blurs it and merges it under the text.

## Sound

A SOUND layer holds one Ogg Vorbis or WAVE file. The mixer is pulled:
`Mixer::render` writes interleaved stereo f32 at 44.1 kHz and moves every
layer on, so a player calls it from its audio callback and headless tools
call it as their clock advances (`yuris boot --wav`). `Host::audio` is an
`Arc<Mutex<Mixer>>` for that reason.

- LOOP 0 loops forever and n plays n times; START is where play begins
  (negative: start that much later), RESTART where later loops begin, END
  the end and loop point. Ogg files end at their last granule position and
  seek by decoding from the start, so loop points are sample-exact.
- VOLUME (0 to 256), PAN (0 to 512) and PITCH (0 to 512, 0.5× to 1.5×).
  FADE is how long a PLAY or VOLUME change takes; a FADE with PLAY=0 stops
  at its end.
- A layer's gain is its own volume times that of every layer whose ID is a
  `/`-ended prefix of it, and a disabled prefix mutes it. Erewhon sets the
  master volume on `:/S0/` and plays below it.
- SOUNDINFO answers the state, position, length, format (CODEC 0 PCM WAVE,
  1 Ogg, 2 compressed WAVE) and peak levels.

Linear volume and the linear pan law are guesses; the engine's mixer has
not been reversed.
