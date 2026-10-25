//! Image operations behind CGACT (straight-alpha RGBA8 images).
//!
//! Value scales follow the manual's examples: MEIDO `SET=0xffff00` is "red
//! 200%, green 200%, blue 0%", so 0x80 per channel is 100%; SAIDO and
//! AKARUSA are taken the same way (128 = unchanged).

use yuris_format::vfs::RgbaImage;

/// A pixel rectangle [x0, x1) × [y0, y1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left.
    pub x0: i64,
    /// Top.
    pub y0: i64,
    /// Right (exclusive).
    pub x1: i64,
    /// Bottom (exclusive).
    pub y1: i64,
}

impl Rect {
    /// The whole of `img`.
    pub fn of(img: &RgbaImage) -> Self {
        Self { x0: 0, y0: 0, x1: i64::from(img.width), y1: i64::from(img.height) }
    }

    /// Clamped to `img`.
    pub fn clamp_to(self, img: &RgbaImage) -> Self {
        let w = i64::from(img.width);
        let h = i64::from(img.height);
        Self {
            x0: self.x0.clamp(0, w),
            y0: self.y0.clamp(0, h),
            x1: self.x1.clamp(0, w),
            y1: self.y1.clamp(0, h),
        }
    }

    /// Width (0 when empty).
    pub fn width(&self) -> i64 {
        (self.x1 - self.x0).max(0)
    }

    /// Height (0 when empty).
    pub fn height(&self) -> i64 {
        (self.y1 - self.y0).max(0)
    }
}

/// A `w`×`h` image of one pixel value.
pub fn solid(w: u32, h: u32, px: [u8; 4]) -> RgbaImage {
    RgbaImage { width: w, height: h, data: px.repeat(w as usize * h as usize) }
}

/// The part of `img` inside `r`.
pub fn crop(img: &RgbaImage, r: Rect) -> RgbaImage {
    let r = r.clamp_to(img);
    let (w, h) = (r.width() as usize, r.height() as usize);
    let mut data = Vec::with_capacity(w * h * 4);
    for y in r.y0..r.y1 {
        let s = (y as usize * img.width as usize + r.x0 as usize) * 4;
        data.extend_from_slice(&img.data[s..s + w * 4]);
    }
    RgbaImage { width: w as u32, height: h as u32, data }
}

/// Nearest-neighbour scaling (SIZE).
pub fn scale_nearest(img: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    if img.width == 0 || img.height == 0 || w == 0 || h == 0 {
        return solid(w, h, [0; 4]);
    }
    let mut data = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        let sy = (u64::from(y) * u64::from(img.height) / u64::from(h)) as usize;
        for x in 0..w {
            let sx = (u64::from(x) * u64::from(img.width) / u64::from(w)) as usize;
            let i = (sy * img.width as usize + sx) * 4;
            data.extend_from_slice(&img.data[i..i + 4]);
        }
    }
    RgbaImage { width: w, height: h, data }
}

/// Bilinear scaling (SIZE2), alpha-weighted so transparent pixels do not
/// darken edges.
pub fn scale_bilinear(img: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    if img.width == 0 || img.height == 0 || w == 0 || h == 0 {
        return solid(w, h, [0; 4]);
    }
    let (iw, ih) = (img.width as usize, img.height as usize);
    let px = |x: usize, y: usize| {
        let i = (y * iw + x) * 4;
        let p = &img.data[i..i + 4];
        let a = f32::from(p[3]);
        [f32::from(p[0]) * a, f32::from(p[1]) * a, f32::from(p[2]) * a, a]
    };
    let mut data = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        let fy = ((y as f32 + 0.5) * ih as f32 / h as f32 - 0.5).clamp(0.0, (ih - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        let y1 = (y0 + 1).min(ih - 1);
        for x in 0..w {
            let fx = ((x as f32 + 0.5) * iw as f32 / w as f32 - 0.5).clamp(0.0, (iw - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let x1 = (x0 + 1).min(iw - 1);
            let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
            let mut out = [0f32; 4];
            for k in 0..4 {
                let top = a[k] + (b[k] - a[k]) * tx;
                let bot = c[k] + (d[k] - c[k]) * tx;
                out[k] = top + (bot - top) * ty;
            }
            let alpha = out[3];
            if alpha <= 0.0 {
                data.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                data.extend_from_slice(&[
                    (out[0] / alpha).round().clamp(0.0, 255.0) as u8,
                    (out[1] / alpha).round().clamp(0.0, 255.0) as u8,
                    (out[2] / alpha).round().clamp(0.0, 255.0) as u8,
                    alpha.round().clamp(0.0, 255.0) as u8,
                ]);
            }
        }
    }
    RgbaImage { width: w, height: h, data }
}

/// Applies `f` to every pixel of `img` inside `r`.
pub fn map_pixels(img: &mut RgbaImage, r: Rect, mut f: impl FnMut(&mut [u8])) {
    let r = r.clamp_to(img);
    let w = img.width as usize;
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let i = (y as usize * w + x as usize) * 4;
            f(&mut img.data[i..i + 4]);
        }
    }
}

/// RECTPAINT: RGB := 0xRRGGBB.
pub fn paint_rgb(img: &mut RgbaImage, r: Rect, rgb: i64) {
    let c = [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8];
    map_pixels(img, r, |p| p[..3].copy_from_slice(&c));
}

/// RECTPAINT2: alpha := a (0..=255).
pub fn paint_alpha(img: &mut RgbaImage, r: Rect, a: i64) {
    let a = a.clamp(0, 255) as u8;
    map_pixels(img, r, |p| p[3] = a);
}

/// RECTPAINT3: ARGB := 0xAARRGGBB.
pub fn paint_argb(img: &mut RgbaImage, r: Rect, argb: i64) {
    let c = [(argb >> 16) as u8, (argb >> 8) as u8, argb as u8, (argb >> 24) as u8];
    map_pixels(img, r, |p| p.copy_from_slice(&c));
}

/// Draws `src` (opacity `alpha`, 0..=256) over `dst` at (dx, dy) (MERGE).
pub fn merge(dst: &mut RgbaImage, src: &RgbaImage, dx: i64, dy: i64, alpha: i64) {
    let k = alpha.clamp(0, 256) as u32;
    let r = Rect { x0: dx, y0: dy, x1: dx + i64::from(src.width), y1: dy + i64::from(src.height) }.clamp_to(dst);
    let dw = dst.width as usize;
    let sw = src.width as usize;
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let si = ((y - dy) as usize * sw + (x - dx) as usize) * 4;
            let s = &src.data[si..si + 4];
            let sa = (u32::from(s[3]) * k) >> 8;
            if sa == 0 {
                continue;
            }
            let di = (y as usize * dw + x as usize) * 4;
            let d = &mut dst.data[di..di + 4];
            let da = u32::from(d[3]);
            let oa = sa * 255 + da * (255 - sa);
            if oa == 0 {
                continue;
            }
            for c in 0..3 {
                let v = (u32::from(s[c]) * sa * 255 + u32::from(d[c]) * da * (255 - sa)) / oa;
                d[c] = v.min(255) as u8;
            }
            d[3] = ((oa + 127) / 255).min(255) as u8;
        }
    }
}

/// RESIZE: grows (positive) or cuts (negative) each edge without scaling.
pub fn resize_edges(img: &RgbaImage, left: i64, top: i64, right: i64, bottom: i64) -> RgbaImage {
    let w = (i64::from(img.width) + left + right).max(0) as u32;
    let h = (i64::from(img.height) + top + bottom).max(0) as u32;
    let mut out = solid(w, h, [0; 4]);
    for y in 0..i64::from(h) {
        let sy = y - top;
        if sy < 0 || sy >= i64::from(img.height) {
            continue;
        }
        for x in 0..i64::from(w) {
            let sx = x - left;
            if sx < 0 || sx >= i64::from(img.width) {
                continue;
            }
            let si = (sy as usize * img.width as usize + sx as usize) * 4;
            let di = (y as usize * w as usize + x as usize) * 4;
            out.data[di..di + 4].copy_from_slice(&img.data[si..si + 4]);
        }
    }
    out
}

/// SAIDO: saturation, 128 = unchanged, 0 = grey.
pub fn saturation(img: &mut RgbaImage, r: Rect, set: i64) {
    let k = set as f32 / 128.0;
    map_pixels(img, r, |p| {
        let l = 0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2]);
        for c in &mut p[..3] {
            *c = (l + (f32::from(*c) - l) * k).round().clamp(0.0, 255.0) as u8;
        }
    });
}

/// MEIDO: per-channel gain, 0x80 = 100% for each of R, G, B.
pub fn channel_gain(img: &mut RgbaImage, r: Rect, rgb: i64) {
    let g = [((rgb >> 16) & 0xff) as u32, ((rgb >> 8) & 0xff) as u32, (rgb & 0xff) as u32];
    map_pixels(img, r, |p| {
        for c in 0..3 {
            p[c] = (u32::from(p[c]) * g[c] / 128).min(255) as u8;
        }
    });
}

/// AKARUSA: brightness gain, 128 = unchanged.
pub fn brightness(img: &mut RgbaImage, r: Rect, set: i64) {
    let g = set.clamp(0, 1 << 16) as u32;
    map_pixels(img, r, |p| {
        for c in &mut p[..3] {
            *c = (u32::from(*c) * g / 128).min(255) as u8;
        }
    });
}

/// MONO: two tones by luminance threshold.
pub fn mono(img: &mut RgbaImage, r: Rect, threshold: i64) {
    map_pixels(img, r, |p| {
        let l = (299 * u32::from(p[0]) + 587 * u32::from(p[1]) + 114 * u32::from(p[2])) / 1000;
        let v = if i64::from(l) >= threshold { 255 } else { 0 };
        p[..3].fill(v);
    });
}

/// REVERSE: negative.
pub fn invert(img: &mut RgbaImage, r: Rect) {
    map_pixels(img, r, |p| {
        for c in &mut p[..3] {
            *c = 255 - *c;
        }
    });
}

/// VERTICAL / HORIZONTAL flips.
pub fn flip(img: &mut RgbaImage, vertical: bool) {
    let (w, h) = (img.width as usize, img.height as usize);
    let src = img.data.clone();
    for y in 0..h {
        for x in 0..w {
            let (sx, sy) = if vertical { (x, h - 1 - y) } else { (w - 1 - x, y) };
            let si = (sy * w + sx) * 4;
            let di = (y * w + x) * 4;
            img.data[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
}

/// MOSAIC with `block`-pixel cells.
pub fn mosaic(img: &mut RgbaImage, r: Rect, block: i64) {
    let r = r.clamp_to(img);
    let b = block.max(1);
    let w = img.width as usize;
    let mut by = r.y0;
    while by < r.y1 {
        let mut bx = r.x0;
        while bx < r.x1 {
            let i = (by as usize * w + bx as usize) * 4;
            let c = [img.data[i], img.data[i + 1], img.data[i + 2], img.data[i + 3]];
            map_pixels(img, Rect { x0: bx, y0: by, x1: (bx + b).min(r.x1), y1: (by + b).min(r.y1) }, |p| {
                p.copy_from_slice(&c)
            });
            bx += b;
        }
        by += b;
    }
}

/// BLUR: box blur of radius `radius` (horizontal then vertical pass),
/// alpha-weighted so transparent pixels do not tint the result.
pub fn blur(img: &mut RgbaImage, r: Rect, radius: i64) {
    let r = r.clamp_to(img);
    let rad = radius.clamp(0, 64);
    if rad == 0 || r.width() == 0 || r.height() == 0 {
        return;
    }
    let w = img.width as usize;
    for pass in 0..2 {
        let src = img.data.clone();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let mut acc = [0u32; 4];
                let mut n = 0u32;
                for d in -rad..=rad {
                    let (sx, sy) = if pass == 0 { (x + d, y) } else { (x, y + d) };
                    if sx < r.x0 || sx >= r.x1 || sy < r.y0 || sy >= r.y1 {
                        continue;
                    }
                    let i = (sy as usize * w + sx as usize) * 4;
                    let a = u32::from(src[i + 3]);
                    for k in 0..3 {
                        acc[k] += u32::from(src[i + k]) * a;
                    }
                    acc[3] += a;
                    n += 1;
                }
                let i = (y as usize * w + x as usize) * 4;
                if acc[3] > 0 {
                    for k in 0..3 {
                        img.data[i + k] = (acc[k] / acc[3]) as u8;
                    }
                }
                img.data[i + 3] = (acc[3] / n.max(1)) as u8;
            }
        }
    }
}

/// TRIM: cuts fully transparent rows/columns. `sides`: 8 top, 4 bottom,
/// 2 left, 1 right. Returns the image and how far its origin moved.
pub fn trim(img: &RgbaImage, sides: i64) -> (RgbaImage, i64, i64) {
    let (w, h) = (i64::from(img.width), i64::from(img.height));
    let opaque = |x: i64, y: i64| img.data[((y * w + x) * 4 + 3) as usize] != 0;
    let row = |y: i64| (0..w).any(|x| opaque(x, y));
    let col = |x: i64, y0: i64, y1: i64| (y0..y1).any(|y| opaque(x, y));
    let (mut y0, mut y1) = (0, h);
    if sides & 8 != 0 {
        while y0 < y1 && !row(y0) {
            y0 += 1;
        }
    }
    if sides & 4 != 0 {
        while y1 > y0 && !row(y1 - 1) {
            y1 -= 1;
        }
    }
    let (mut x0, mut x1) = (0, w);
    if sides & 2 != 0 {
        while x0 < x1 && !col(x0, y0, y1) {
            x0 += 1;
        }
    }
    if sides & 1 != 0 {
        while x1 > x0 && !col(x1 - 1, y0, y1) {
            x1 -= 1;
        }
    }
    (crop(img, Rect { x0, y0, x1, y1 }), x0, y0)
}

/// ROTATION: clockwise by `deg` degrees about the centre, nearest sample,
/// on a canvas big enough for the result. Returns the image and how far
/// its origin moved.
pub fn rotate(img: &RgbaImage, deg: f64) -> (RgbaImage, i64, i64) {
    let (w, h) = (f64::from(img.width), f64::from(img.height));
    let (s, c) = deg.to_radians().sin_cos();
    let nw = (w * c.abs() + h * s.abs()).ceil().max(1.0);
    let nh = (w * s.abs() + h * c.abs()).ceil().max(1.0);
    let mut out = solid(nw as u32, nh as u32, [0; 4]);
    let (cx, cy, ncx, ncy) = (w / 2.0, h / 2.0, nw / 2.0, nh / 2.0);
    for y in 0..nh as i64 {
        for x in 0..nw as i64 {
            let (dx, dy) = (x as f64 + 0.5 - ncx, y as f64 + 0.5 - ncy);
            // Inverse of a clockwise rotation (y points down).
            let sx = dx * c + dy * s + cx;
            let sy = -dx * s + dy * c + cy;
            if let Some(p) = img.pixel(sx.floor() as i64, sy.floor() as i64) {
                let di = (y as usize * nw as usize + x as usize) * 4;
                out.data[di..di + 4].copy_from_slice(&p);
            }
        }
    }
    let ox = ((w - nw) / 2.0).round() as i64;
    let oy = ((h - nh) / 2.0).round() as i64;
    (out, ox, oy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_trim_merge() {
        let img = solid(4, 2, [10, 20, 30, 255]);
        let big = resize_edges(&img, 1, 2, -1, 0);
        assert_eq!((big.width, big.height), (4, 4));
        assert_eq!(big.pixel(0, 2), Some([0, 0, 0, 0]));
        assert_eq!(big.pixel(1, 2), Some([10, 20, 30, 255]));
        let (t, ox, oy) = trim(&big, 15);
        assert_eq!((t.width, t.height, ox, oy), (3, 2, 1, 2));
        let mut dst = solid(4, 4, [0, 0, 0, 0]);
        merge(&mut dst, &solid(2, 2, [255, 0, 0, 255]), 1, 1, 256);
        assert_eq!(dst.pixel(1, 1), Some([255, 0, 0, 255]));
        assert_eq!(dst.pixel(0, 0), Some([0, 0, 0, 0]));
        let s = scale_bilinear(&solid(2, 2, [50, 60, 70, 255]), 5, 3);
        assert_eq!(s.pixel(4, 2), Some([50, 60, 70, 255]));
    }
}
