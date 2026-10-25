# Player, GPU compositor and the C interface

Three crates turn the headless runtime into something an app can show.

## yuris-player: real time

`Player::open` does what `yuris boot` does at start (game file system,
launcher overlay, boot chain, yscfg screen size, host, fonts, encoding)
and `Player::step(dt_ms)` runs one engine frame per display frame:

1. The clock moves on by `dt_ms` (at most 250 ms, so a stall slows the
   game down instead of skipping its timed waits).
2. The frame starts: `@_TIME`, the cursor and the button and key states are
   written, then the VM resumes from its last WAIT.
3. The VM runs until it waits again. With tasks, the scheduler keeps the
   waits itself (`wait_until_ms` against the clock) and every WAIT the host
   sees is one frame; without tasks, the player counts frames and times.

A step runs scripts for at most 100 ms of wall time; a frame that needs more
goes on in the next step. A click that goes down and up between two frames
still holds the button for one frame. Keys arrive as Windows virtual-key
codes (`key_from_vk`). Sound positions advance as the mixer is pulled, so
the caller pulls `Player::audio` from its audio callback in real time.

## yuris-render: the GPU compositor

`Renderer` draws window 0's set the same way `compose::render_window` does,
with wgpu:

- each image layer is a texture, uploaded on first use and kept while the
  image lives and is drawn (dropped after 180 frames unused). The cache
  holds a weak reference, so an image changed in place through
  `Arc::make_mut` moves to a new allocation and is uploaded again;
- layers are quads in Z order at whole pixels, sampled 1:1; set clips are
  scissor rectangles;
- an opaque normal set draws straight into its parent, any other set is
  drawn into a texture of its own first (its size, or the parent's when it
  has none), then drawn with its opacity and mode;
- normal, add, subtract and screen are blend states; multiply, lighten,
  darken and difference copy the target and compute the CPU formula in the
  shader. Other modes draw as normal, as on the CPU;
- the frame is drawn into a screen-sized texture over black and then scaled
  into the output with black bars (`fit`), linearly filtered.

The output format should be a non-sRGB one (`Bgra8Unorm`): the game's
colours are sRGB values already. `Renderer::headless` and `render_image`
read a frame back for tests and tools; `tests/compare.rs` checks every mode
and kind of set against the CPU compositor (at most 2/255 apart), and
`yuris play --compare` does the same on a real game.

## yuris-mobile: the C interface

`platform/ios/include/yuris_mobile.h`. The app:

1. opens a session with `yuris_open` (game folder, patch launcher, save
   folder, fonts, encoding, UTC offset);
2. gives it a view with `yuris_set_surface(session, uiView, w, h)` in
   drawable pixels (again after a size change). The view's layer should be a
   `CAMetalLayer`; wgpu makes the Metal surface from it;
3. calls `yuris_step(session, dt_ms)` from its display link. The step runs
   the frame and draws it if the layer tree changed, so a still screen
   costs no GPU work;
4. passes touches as `yuris_pointer` (drawable pixels; mapped through the
   letterbox to screen pixels), `yuris_wheel` and `yuris_key`;
5. pulls sound with `yuris_audio_render` (44.1 kHz stereo floats) on its
   audio thread, through a `YurisAudio` reference that outlives the session
   until released;
6. closes with `yuris_close`.

Errors are reported as NULL or -1 / `YURIS_FAILED` with the text in
`yuris_last_error`; panics are caught at the boundary. `yuris_set_log_callback`
forwards the runtime's log records (only `yuris*` targets).

Build for iOS with `cargo build --release -p yuris-mobile --target
aarch64-apple-ios` (and `aarch64-apple-ios-sim`); the crate is a
`staticlib` and a `cdylib`.
