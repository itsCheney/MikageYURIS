//! The GPU compositor against the CPU one, on trees that use every mode,
//! clipped and translucent sets, and nested sets. Skipped without a GPU
//! (on Linux, Mesa's lavapipe is enough).

use std::sync::Arc;

use yuris_format::vfs::RgbaImage;
use yuris_host::compose::render_window;
use yuris_host::layers::LayerTree;
use yuris_render::Renderer;

/// A deterministic pattern with varying colour and alpha.
fn pattern(w: u32, h: u32, seed: u32) -> RgbaImage {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    let mut s = seed.wrapping_mul(2_654_435_761).max(1);
    for y in 0..h {
        for x in 0..w {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            let a = if (x / 3 + y / 5 + seed) % 4 == 0 { 255 } else { (s >> 24) as u8 };
            data.extend_from_slice(&[(x * 9 + seed * 40) as u8, (y * 7) as u8, (s >> 8) as u8, a]);
        }
    }
    RgbaImage { width: w, height: h, data }
}

fn layer(t: &mut LayerTree, id: &str, img: RgbaImage, x: i64, y: i64, z: i64, alpha: i64, mode: i64) {
    let (l, _) = t.find_or_create(id.as_bytes()).unwrap();
    let l = t.get_mut(l).unwrap();
    l.image = Arc::new(img);
    (l.x, l.y, l.z, l.alpha, l.mode) = (x, y, z, alpha, mode);
}

fn set(t: &mut LayerTree, id: &str, x: i64, y: i64, z: i64, sx: i64, sy: i64, alpha: i64, mode: i64) {
    let (l, _) = t.find_or_create(id.as_bytes()).unwrap();
    let l = t.get_mut(l).unwrap();
    (l.x, l.y, l.z, l.sx, l.sy, l.alpha, l.mode) = (x, y, z, sx, sy, alpha, mode);
}

fn compare(r: &mut Renderer, t: &LayerTree, screen: (u32, u32), what: &str) {
    let cpu = render_window(t, 0, screen.0, screen.1);
    let gpu = r.render_image(t, screen);
    assert_eq!((gpu.width, gpu.height), (cpu.width, cpu.height));
    let mut worst = 0;
    let mut at = (0, 0);
    for (i, (a, b)) in cpu.data.chunks_exact(4).zip(gpu.data.chunks_exact(4)).enumerate() {
        for c in 0..3 {
            let d = (i32::from(a[c]) - i32::from(b[c])).abs();
            if d > worst {
                worst = d;
                at = (i as u32 % screen.0, i as u32 / screen.0);
            }
        }
    }
    // Integer rounding on the CPU against float on the GPU, compounded
    // by a few layers.
    assert!(worst <= 4, "{what}: largest difference {worst} at {at:?}");
}

#[test]
fn gpu_matches_cpu() {
    let Some(mut r) = Renderer::headless() else {
        eprintln!("no GPU adapter; skipped");
        return;
    };
    let screen = (96, 64);
    let mut t = LayerTree::new();
    layer(&mut t, "BG", RgbaImage { width: 96, height: 64, data: [40, 90, 160, 255].repeat(96 * 64) }, 0, 0, 0, 256, 0);
    compare(&mut r, &t, screen, "background");
    for (i, mode) in [0, 1, 2, 3, 5, 6, 7, 8, 4].into_iter().enumerate() {
        let i = i as i64;
        layer(&mut t, &format!("M{mode}"), pattern(20, 18, i as u32 + 1), -5 + i * 11, 3 + i * 5, 10 + i, 256 - i * 20, mode);
        compare(&mut r, &t, screen, &format!("mode {mode}"));
    }
    // An opaque clipped set, drawn straight into its parent.
    set(&mut t, "S1/", 30, 10, 50, 25, 20, 256, 0);
    layer(&mut t, "S1/A", pattern(40, 40, 7), -6, -4, 0, 256, 0);
    compare(&mut r, &t, screen, "clipped set");
    // A translucent set with a mode, with a nested translucent set.
    set(&mut t, "S2/", 10, 20, 60, 50, 30, 180, 3);
    layer(&mut t, "S2/A", pattern(30, 30, 9), 5, 5, 1, 256, 0);
    set(&mut t, "S2/N/", 8, 2, 2, 0, 0, 128, 1);
    layer(&mut t, "S2/N/B", pattern(16, 16, 11), 0, 0, 0, 200, 5);
    compare(&mut r, &t, screen, "translucent sets");
    // An unsized translucent set and a disabled layer.
    set(&mut t, "S3/", 4, 4, 70, 0, 0, 100, 0);
    layer(&mut t, "S3/A", pattern(24, 24, 13), 60, 30, 0, 256, 2);
    layer(&mut t, "OFF", pattern(96, 64, 3), 0, 0, 99, 256, 0);
    let off = t.find(b"OFF").unwrap();
    t.get_mut(off).unwrap().enabled = false;
    compare(&mut r, &t, screen, "unsized set");
    // Changing an image in place uploads it again.
    let a = t.find(b"S1/A").unwrap();
    Arc::make_mut(&mut t.get_mut(a).unwrap().image).data.fill(255);
    compare(&mut r, &t, screen, "image changed in place");
    assert!(r.cached_images() > 0);
}
