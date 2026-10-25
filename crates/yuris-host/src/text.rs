//! Fonts: the FONT state and drawing text into layer images (CGACT TEXT).
//!
//! Erewhon keeps several font settings apart by `BNO` (FONT BNO=1 ... then
//! CGACT TEXT BNO=1 ...), so the state is a table of slots; a FONT without
//! BNO changes slot 0. Faces are registered by the player (the system's
//! CJK fonts); a face name the game asks for (`ＭＳ ゴシック`) that is not
//! registered falls back to the first face, as the Siglus runtime does.

use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use yuris_format::vfs::RgbaImage;

/// One FONT slot.
#[derive(Debug, Clone, PartialEq)]
pub struct FontSlot {
    /// NAME (raw bytes, the game's encoding).
    pub name: Vec<u8>,
    /// COLOR, 0xRRGGBB.
    pub color: u32,
    /// COLOR2, 0xRRGGBB (gradient end).
    pub color2: u32,
    /// SX: character width in pixels.
    pub sx: i64,
    /// SY: character height in pixels.
    pub sy: i64,
    /// BOLD.
    pub bold: bool,
    /// ANTIALIASING.
    pub antialias: bool,
    /// DRAWMODE: 0 = the text replaces the pixels under it, 1 = blended.
    pub drawmode: i64,
    /// GRADATION: 0 = flat COLOR, otherwise a top-to-bottom COLOR→COLOR2
    /// gradient.
    pub gradation: i64,
    /// ROTATION (not applied yet).
    pub rotation: i64,
}

impl Default for FontSlot {
    fn default() -> Self {
        Self {
            name: Vec::new(),
            color: 0xffffff,
            color2: 0xffffff,
            sx: 24,
            sy: 24,
            bold: false,
            antialias: true,
            drawmode: 0,
            gradation: 0,
            rotation: 0,
        }
    }
}

/// Registered faces and the FONT slots.
#[derive(Clone, Default)]
pub struct Fonts {
    faces: Vec<(String, FontArc)>,
    /// FONT slots by BNO.
    pub slots: Vec<FontSlot>,
}

impl std::fmt::Debug for Fonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fonts")
            .field("faces", &self.faces.iter().map(|(n, _)| n).collect::<Vec<_>>())
            .field("slots", &self.slots)
            .finish()
    }
}

impl Fonts {
    /// Registers a face from TTF/OTF bytes (`index` picks a face in a
    /// TTC). Returns false if the bytes are not a font.
    pub fn add_face(&mut self, name: &str, bytes: Vec<u8>, index: u32) -> bool {
        match ab_glyph::FontVec::try_from_vec_and_index(bytes, index) {
            Ok(f) => {
                self.faces.push((name.to_string(), FontArc::new(f)));
                true
            }
            Err(_) => false,
        }
    }

    /// Number of registered faces.
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    /// Name of face `n`.
    pub fn face_name(&self, n: usize) -> Option<&str> {
        self.faces.get(n).map(|(s, _)| s.as_str())
    }

    /// FONT slot `bno`, created with defaults when new.
    pub fn slot_mut(&mut self, bno: usize) -> &mut FontSlot {
        if self.slots.len() <= bno {
            self.slots.resize(bno + 1, FontSlot::default());
        }
        &mut self.slots[bno]
    }

    /// FONT slot `bno` (defaults when never set).
    pub fn slot(&self, bno: usize) -> FontSlot {
        self.slots.get(bno).cloned().unwrap_or_default()
    }

    fn face_for(&self, name: &str) -> Option<&FontArc> {
        self.faces
            .iter()
            .find(|(n, _)| n == name)
            .or_else(|| self.faces.first())
            .map(|(_, f)| f)
    }

    /// The face that draws `c`: `face` when it has the character, else the
    /// first registered face that has it (PingFang SC has no kana, so
    /// Japanese text needs Hiragino), else `face` (its missing-glyph box).
    fn face_with<'a>(&'a self, face: &'a FontArc, c: char) -> (&'a FontArc, ab_glyph::GlyphId) {
        let gid = face.glyph_id(c);
        if gid.0 != 0 {
            return (face, gid);
        }
        self.faces
            .iter()
            .map(|(_, f)| (f, f.glyph_id(c)))
            .find(|(_, g)| g.0 != 0)
            .unwrap_or((face, gid))
    }

    /// Width in pixels of `text` drawn with `slot`.
    pub fn measure(&self, slot: &FontSlot, name: &str, text: &str) -> i64 {
        let Some(face) = self.face_for(name) else { return 0 };
        text.chars()
            .map(|c| {
                let (f, gid) = self.face_with(face, c);
                f.as_scaled(scale(slot)).h_advance(gid)
            })
            .sum::<f32>()
            .ceil() as i64
    }

    /// Draws `text` into `img` with its top-left corner at (x, y). Returns
    /// false when no face is registered.
    pub fn draw(&self, img: &mut RgbaImage, x: i64, y: i64, slot: &FontSlot, name: &str, text: &str) -> bool {
        let Some(face) = self.face_for(name) else { return false };
        let sf = face.as_scaled(scale(slot));
        let ascent = sf.ascent();
        let height = (sf.ascent() - sf.descent()).max(1.0);
        let mut pen = x as f32;
        for c in text.chars() {
            if c == '\n' {
                continue;
            }
            let (f, gid) = self.face_with(face, c);
            let glyph = gid.with_scale_and_position(scale(slot), ab_glyph::point(pen, y as f32 + ascent));
            pen += f.as_scaled(scale(slot)).h_advance(gid);
            let Some(outline) = f.outline_glyph(glyph) else { continue };
            let bb = outline.px_bounds();
            let passes: &[f32] = if slot.bold { &[0.0, 1.0] } else { &[0.0] };
            for &dx in passes {
                outline.draw(|gx, gy, cov| {
                    let px = bb.min.x as i64 + i64::from(gx) + dx as i64;
                    let py = bb.min.y as i64 + i64::from(gy);
                    let cov = if slot.antialias { cov } else if cov >= 0.5 { 1.0 } else { 0.0 };
                    let t = ((py as f32 - y as f32) / height).clamp(0.0, 1.0);
                    let rgb = if slot.gradation != 0 { lerp_rgb(slot.color, slot.color2, t) } else { slot.color };
                    put(img, px, py, rgb, cov, slot.drawmode == 0 && dx == 0.0);
                });
            }
        }
        true
    }
}

fn scale(slot: &FontSlot) -> PxScale {
    PxScale { x: slot.sx.max(1) as f32, y: slot.sy.max(1) as f32 }
}

fn lerp_rgb(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let ca = ((a >> s) & 0xff) as f32;
        let cb = ((b >> s) & 0xff) as f32;
        ((ca + (cb - ca) * t).round() as u32).min(255) << s
    };
    ch(16) | ch(8) | ch(0)
}

/// Writes one glyph pixel: `replace` sets the pixel to the glyph's color
/// and coverage (DRAWMODE 0), otherwise the glyph is blended over it.
fn put(img: &mut RgbaImage, x: i64, y: i64, rgb: u32, cov: f32, replace: bool) {
    if x < 0 || y < 0 || x >= i64::from(img.width) || y >= i64::from(img.height) {
        return;
    }
    let i = (y as usize * img.width as usize + x as usize) * 4;
    let p = &mut img.data[i..i + 4];
    let (r, g, b) = (((rgb >> 16) & 0xff) as f32, ((rgb >> 8) & 0xff) as f32, (rgb & 0xff) as f32);
    let sa = cov.clamp(0.0, 1.0);
    if replace {
        if sa <= 0.0 {
            return;
        }
        p[0] = r as u8;
        p[1] = g as u8;
        p[2] = b as u8;
        p[3] = (sa * 255.0).round() as u8;
        return;
    }
    let da = f32::from(p[3]) / 255.0;
    let oa = sa + da * (1.0 - sa);
    if oa <= 0.0 {
        return;
    }
    for (c, s) in p[..3].iter_mut().zip([r, g, b]) {
        let d = f32::from(*c);
        *c = ((s * sa + d * da * (1.0 - sa)) / oa).round().clamp(0.0, 255.0) as u8;
    }
    p[3] = (oa * 255.0).round() as u8;
}
