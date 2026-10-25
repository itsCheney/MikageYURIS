//! CPU compositor: draws a window's layer set into an RGBA frame.
//!
//! Canvases are premultiplied RGBA8; layer images are straight alpha.
//! Supported modes: 0 normal, 1 add, 2 subtract, 3 multiply, 5 screen,
//! 6 lighten, 7 darken, 8 difference. The other modes (and masks, MID, and
//! transitions, TID/T) draw as normal for now.

use yuris_format::vfs::RgbaImage;

use crate::layers::{LayerId, LayerTree};

/// A premultiplied RGBA8 buffer.
#[derive(Debug, Clone)]
pub struct Canvas {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes, premultiplied.
    pub data: Vec<u8>,
}

impl Canvas {
    /// A transparent canvas.
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, data: vec![0; width as usize * height as usize * 4] }
    }

    /// Converts to a straight-alpha image.
    pub fn to_image(&self) -> RgbaImage {
        let mut data = self.data.clone();
        for p in data.chunks_exact_mut(4) {
            let a = u32::from(p[3]);
            if a != 0 && a != 255 {
                for c in &mut p[..3] {
                    *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        RgbaImage { width: self.width, height: self.height, data }
    }
}

/// A clip rectangle, in canvas pixels: [x0, x1) × [y0, y1).
#[derive(Debug, Clone, Copy)]
struct Clip {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

impl Clip {
    fn intersect(self, o: Clip) -> Clip {
        Clip { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) }
    }
}

#[inline]
fn mul255(a: u32, b: u32) -> u32 {
    let t = a * b + 128;
    (t + (t >> 8)) >> 8
}

/// Blends one premultiplied source pixel (already scaled by layer opacity)
/// onto a premultiplied destination pixel.
#[inline]
fn blend_px(d: &mut [u8], s: [u32; 4], mode: i64) {
    let sa = s[3];
    if sa == 0 {
        return;
    }
    let inv = 255 - sa;
    let da = u32::from(d[3]);
    let dinv = 255 - da;
    for c in 0..3 {
        let dc = u32::from(d[c]);
        let sc = s[c];
        // Separable modes in premultiplied form:
        // f(s, d) + s·(1 − da) + d·(1 − sa).
        let rest = mul255(sc, dinv) + mul255(dc, inv);
        let v = match mode {
            1 => dc + sc,
            2 => dc.saturating_sub(sc),
            3 => mul255(sc, dc) + rest,
            5 => sc + dc - mul255(sc, dc),
            6 => mul255(sc, da).max(mul255(dc, sa)) + rest,
            7 => mul255(sc, da).min(mul255(dc, sa)) + rest,
            8 => (sc + dc).saturating_sub(2 * mul255(sc, da).min(mul255(dc, sa))),
            _ => sc + mul255(dc, inv),
        };
        d[c] = v.min(255) as u8;
    }
    d[3] = (sa + mul255(da, inv)).min(255) as u8;
}

/// Draws a straight-alpha image at (dx, dy) with opacity `alpha` (0..=256).
fn draw_image(dst: &mut Canvas, src: &RgbaImage, dx: i64, dy: i64, alpha: i64, mode: i64, clip: Clip) {
    let k = alpha.clamp(0, 256) as u32;
    if k == 0 || src.width == 0 || src.height == 0 {
        return;
    }
    let area = clip.intersect(Clip { x0: dx, y0: dy, x1: dx + i64::from(src.width), y1: dy + i64::from(src.height) });
    if area.x0 >= area.x1 || area.y0 >= area.y1 {
        return;
    }
    let dw = dst.width as usize;
    let sw = src.width as usize;
    for y in area.y0..area.y1 {
        let srow = (y - dy) as usize * sw;
        let drow = y as usize * dw;
        for x in area.x0..area.x1 {
            let si = (srow + (x - dx) as usize) * 4;
            let p = &src.data[si..si + 4];
            let a = (u32::from(p[3]) * k) >> 8;
            if a == 0 {
                continue;
            }
            let s = [mul255(u32::from(p[0]), a), mul255(u32::from(p[1]), a), mul255(u32::from(p[2]), a), a];
            let di = (drow + x as usize) * 4;
            blend_px(&mut dst.data[di..di + 4], s, mode);
        }
    }
}

/// Draws a premultiplied canvas at (dx, dy) with opacity `alpha`.
fn draw_canvas(dst: &mut Canvas, src: &Canvas, dx: i64, dy: i64, alpha: i64, mode: i64, clip: Clip) {
    let k = alpha.clamp(0, 256) as u32;
    if k == 0 {
        return;
    }
    let area = clip.intersect(Clip { x0: dx, y0: dy, x1: dx + i64::from(src.width), y1: dy + i64::from(src.height) });
    if area.x0 >= area.x1 || area.y0 >= area.y1 {
        return;
    }
    let dw = dst.width as usize;
    let sw = src.width as usize;
    for y in area.y0..area.y1 {
        let srow = (y - dy) as usize * sw;
        let drow = y as usize * dw;
        for x in area.x0..area.x1 {
            let si = (srow + (x - dx) as usize) * 4;
            let p = &src.data[si..si + 4];
            if p[3] == 0 {
                continue;
            }
            let s = [
                (u32::from(p[0]) * k) >> 8,
                (u32::from(p[1]) * k) >> 8,
                (u32::from(p[2]) * k) >> 8,
                (u32::from(p[3]) * k) >> 8,
            ];
            let di = (drow + x as usize) * 4;
            blend_px(&mut dst.data[di..di + 4], s, mode);
        }
    }
}

/// Draws the children of `set` into `dst`, with the set's origin at
/// (ox, oy) in `dst`.
fn draw_children(tree: &LayerTree, set: LayerId, dst: &mut Canvas, ox: i64, oy: i64, clip: Clip) {
    for id in tree.draw_order(set) {
        let Some(l) = tree.get(id) else { continue };
        if !l.enabled || l.alpha <= 0 {
            continue;
        }
        let (x, y) = (ox + l.x, oy + l.y);
        if !l.is_set {
            draw_image(dst, &l.image, x, y, l.alpha, l.mode, clip);
            continue;
        }
        let set_clip = if l.sx > 0 && l.sy > 0 {
            clip.intersect(Clip { x0: x, y0: y, x1: x + l.sx, y1: y + l.sy })
        } else {
            clip
        };
        if l.alpha >= 256 && l.mode == 0 {
            // Opaque normal set: drawing straight into the parent gives
            // the same pixels as composing it first.
            draw_children(tree, id, dst, x, y, set_clip);
            continue;
        }
        // Compose the set on its own, then draw it with its opacity/mode.
        let (w, h) = if l.sx > 0 && l.sy > 0 {
            (l.sx as u32, l.sy as u32)
        } else {
            (dst.width, dst.height)
        };
        let (bx, by) = if l.sx > 0 && l.sy > 0 { (x, y) } else { (0, 0) };
        let mut tmp = Canvas::new(w, h);
        let full = Clip { x0: 0, y0: 0, x1: i64::from(w), y1: i64::from(h) };
        draw_children(tree, id, &mut tmp, x - bx, y - by, full);
        draw_canvas(dst, &tmp, bx, by, l.alpha, l.mode, set_clip);
    }
}

/// Composes layer set `set` (its children, not the set's own opacity) into
/// a `width`×`height` canvas, with the set's origin at (0, 0).
pub fn compose_set(tree: &LayerTree, set: LayerId, width: u32, height: u32) -> Canvas {
    let mut c = Canvas::new(width, height);
    let clip = Clip { x0: 0, y0: 0, x1: i64::from(width), y1: i64::from(height) };
    draw_children(tree, set, &mut c, 0, 0, clip);
    c
}

/// Renders window `n`'s set (`:/Wn`) over black: the frame the player shows.
pub fn render_window(tree: &LayerTree, n: u32, width: u32, height: u32) -> RgbaImage {
    let mut c = Canvas::new(width, height);
    for p in c.data.chunks_exact_mut(4) {
        p[3] = 255;
    }
    if let Some(set) = tree.window_set(n) {
        let clip = Clip { x0: 0, y0: 0, x1: i64::from(width), y1: i64::from(height) };
        let (x, y) = tree.get(set).map_or((0, 0), |l| (l.x, l.y));
        draw_children(tree, set, &mut c, x, y, clip);
    }
    c.to_image()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn solid(w: u32, h: u32, px: [u8; 4]) -> RgbaImage {
        RgbaImage { width: w, height: h, data: px.repeat((w * h) as usize) }
    }

    #[test]
    fn z_order_alpha_and_sets() {
        let mut t = LayerTree::new();
        let (bg, _) = t.find_or_create(b"BG").unwrap();
        t.get_mut(bg).unwrap().image = Arc::new(solid(4, 4, [255, 0, 0, 255]));
        let (fg, _) = t.find_or_create(b"FG").unwrap();
        {
            let l = t.get_mut(fg).unwrap();
            l.image = Arc::new(solid(2, 2, [0, 0, 255, 255]));
            l.x = 1;
            l.y = 1;
            l.z = 5;
            l.alpha = 128;
        }
        let img = render_window(&t, 0, 4, 4);
        assert_eq!(img.pixel(0, 0), Some([255, 0, 0, 255]));
        let p = img.pixel(1, 1).unwrap();
        assert!((p[0] as i32 - 127).abs() <= 2 && (p[2] as i32 - 128).abs() <= 2, "{p:?}");
        // A clipped half-transparent set.
        let (s, _) = t.find_or_create(b"S/").unwrap();
        {
            let l = t.get_mut(s).unwrap();
            l.sx = 1;
            l.sy = 1;
            l.z = 9;
            l.alpha = 256;
        }
        let (c, _) = t.find_or_create(b"S/C").unwrap();
        t.get_mut(c).unwrap().image = Arc::new(solid(4, 4, [0, 255, 0, 255]));
        let img = render_window(&t, 0, 4, 4);
        assert_eq!(img.pixel(0, 0), Some([0, 255, 0, 255]));
        assert_eq!(img.pixel(3, 3), Some([255, 0, 0, 255]));
    }
}
