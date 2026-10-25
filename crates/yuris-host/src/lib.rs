//! The native command host for the YU-RIS VM.
//!
//! [`Host`] implements [`yuris_vm::native::Native`]: the CG family on a
//! [`layers::LayerTree`], FONT and CGACT TEXT through [`text::Fonts`],
//! SOUND layers on an [`audio::Mixer`], and the WINDOWINFO answers. [`compose::render_window`]
//! turns the tree into the frame the player shows; headless tools use it
//! for screenshots.
//!
//! Semantics come from the official YU-RIS manual (CG系命令, FONT, SOUND,
//! WINDOW) and from what Erewhon's scripts expect; where both are silent the
//! choice is noted at the handler.

pub mod audio;
pub mod compose;
pub mod layers;
pub mod ops;
pub mod text;

use std::any::Any;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use yuris_core::Result;
use yuris_format::vfs::{GameFs, RgbaImage};
use yuris_value::Value;
use yuris_vm::native::{Native, NativeCall};

use crate::audio::{Mixer, SoundCall};
use crate::layers::{LayerPath, LayerTree};
use crate::ops::Rect;
use crate::text::Fonts;

/// How many decoded images [`Host`] keeps besides those on layers.
const IMAGE_CACHE: usize = 96;

/// The native host.
pub struct Host {
    /// The layer tree.
    pub tree: LayerTree,
    /// Fonts and FONT slots.
    pub fonts: Fonts,
    /// SOUND layers. Shared so a player's audio callback can pull
    /// [`audio::Mixer::render`] on its own thread.
    pub audio: Arc<Mutex<Mixer>>,
    /// Screen size (yscfg), for WINDOWINFO and screenshots.
    pub screen: (u32, u32),
    /// Window caption (yscfg), for WINDOWINFO CAPTION.
    pub caption: Vec<u8>,
    /// Text encoding of the game's strings.
    pub encoding: &'static encoding_rs::Encoding,
    /// Distinct problems met (unknown parameters, missing files), in order.
    pub notes: Vec<String>,
    fs: Option<Arc<GameFs>>,
    images: HashMap<Vec<u8>, Option<Arc<RgbaImage>>>,
    image_order: VecDeque<Vec<u8>>,
    noted: HashSet<String>,
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host")
            .field("layers", &self.tree.len())
            .field("screen", &self.screen)
            .field("fonts", &self.fonts)
            .finish()
    }
}

impl Host {
    /// A host reading images from `fs` (None: images load as 0×0).
    pub fn new(fs: Option<Arc<GameFs>>, screen: (u32, u32)) -> Self {
        Self {
            tree: LayerTree::new(),
            fonts: Fonts::default(),
            audio: Arc::new(Mutex::new(Mixer::default())),
            screen,
            caption: Vec::new(),
            encoding: encoding_rs::SHIFT_JIS,
            notes: Vec::new(),
            fs,
            images: HashMap::new(),
            image_order: VecDeque::new(),
            noted: HashSet::new(),
        }
    }

    /// The main window's frame.
    pub fn render(&self) -> RgbaImage {
        compose::render_window(&self.tree, 0, self.screen.0, self.screen.1)
    }

    fn note(&mut self, key: String) {
        if self.noted.insert(key.clone()) {
            tracing::warn!("{key}");
            if self.notes.len() < 500 {
                self.notes.push(key);
            }
        }
    }

    fn decode(&self, b: &[u8]) -> String {
        self.encoding.decode(b).0.into_owned()
    }

    /// Loads an image by game path (cached). None when missing/undecodable.
    fn load_image(&mut self, file: &[u8]) -> Option<Arc<RgbaImage>> {
        if let Some(hit) = self.images.get(file) {
            return hit.clone();
        }
        let path = self.decode(file);
        let img = self.fs.as_ref().and_then(|fs| fs.image_rgba(&path)).map(Arc::new);
        if img.is_none() && self.fs.is_some() {
            self.note(format!("CG: cannot load image {path}"));
        }
        if self.image_order.len() >= IMAGE_CACHE {
            if let Some(old) = self.image_order.pop_front() {
                self.images.remove(&old);
            }
        }
        self.image_order.push_back(file.to_vec());
        self.images.insert(file.to_vec(), img.clone());
        img
    }

    /// CG: creates a layer (FILE given, or a set path) or changes one.
    /// Without FILE an unknown image layer is not created (the engine
    /// returns quietly; Erewhon's button code relies on CGINFO then finding
    /// nothing). FILE on an existing layer swaps the image and keeps the
    /// attributes. The manual says `FILE=""` makes a 1×1 black image, but
    /// the reversed handler (kemonomichi2 0x423864) creates nothing for it,
    /// so it counts as no FILE here. With SX and SY as well it makes a
    /// black SX×SY canvas: Erewhon draws its text glow into one
    /// (`CG ID=_FONT_TMP FILE="" SX=.. SY=..`, then clears its alpha).
    fn cg(&mut self, c: &NativeCall<'_>) {
        let Some(id) = c.bytes("ID") else { return };
        let path = LayerPath::parse(id);
        let file = c.bytes("FILE").filter(|f| !f.is_empty()).map(<[u8]>::to_vec);
        let canvas = match (c.bytes("FILE"), c.int("SX"), c.int("SY")) {
            (Some([]), Some(w), Some(h)) if w > 0 && h > 0 => Some((w.min(8192) as u32, h.min(8192) as u32)),
            _ => None,
        };
        let found = if file.is_some() || canvas.is_some() || path.set {
            self.tree.find_or_create(id).map(|(l, _)| l)
        } else {
            self.tree.find(id)
        };
        let Some(lid) = found else { return };
        let image = match (&file, canvas) {
            (Some(f), _) => Some(self.load_image(f).unwrap_or_else(|| Arc::new(layers::empty_image()))),
            (None, Some((w, h))) => Some(Arc::new(ops::solid(w, h, [0, 0, 0, 255]))),
            (None, None) => None,
        };
        let Some(l) = self.tree.get_mut(lid) else { return };
        if let (Some(img), false) = (image, l.is_set) {
            l.image = img;
            l.file = file.unwrap_or_default();
        }
        for a in c.args {
            let n = || match &a.value {
                Value::Int(n) => *n,
                Value::Float(f) => *f as i64,
                Value::Str(_) => 0,
            };
            match a.param {
                "E" => l.enabled = n() != 0,
                "A" => l.alpha = n(),
                "X" => l.x = n(),
                "Y" => l.y = n(),
                "Z" => l.z = n(),
                "SX" => l.sx = n(),
                "SY" => l.sy = n(),
                "MODE" => l.mode = n(),
                "T" => l.t = n(),
                "TA" => l.ta = n(),
                "MID" => l.mid = str_of(&a.value),
                "TID" => l.tid = str_of(&a.value),
                _ => {}
            }
        }
    }

    /// CGINFO: the first query parameter given decides the answer. A layer
    /// that does not exist answers 0 to everything (seen in Erewhon:
    /// the button code tells "no such layer" apart by that 0).
    fn cginfo(&mut self, c: &NativeCall<'_>) -> Option<Value> {
        const QUERIES: &[&str] = &[
            "EXIST", "E", "A", "TID", "T", "TA", "X", "Y", "Z", "SX", "SY", "MODE", "COLOR",
            "ONMOUSE", "ONMOUSE2", "ONMOUSE3",
        ];
        let id = c.bytes("ID")?;
        let q = c.first_flag(QUERIES)?;
        let Some(lid) = self.tree.find(id) else { return Some(Value::Int(0)) };
        let l = self.tree.get(lid)?;
        let v = match q {
            "EXIST" => 1,
            "E" => i64::from(l.enabled),
            "A" => l.alpha,
            "TID" => return Some(Value::Str(l.tid.clone())),
            "T" => l.t,
            "TA" => l.ta,
            "X" => l.x,
            "Y" => l.y,
            "Z" => l.z,
            "SX" => l.width(),
            "SY" => l.height(),
            "MODE" => l.mode,
            "COLOR" => {
                let (x, y) = (c.int("SET").unwrap_or(0), c.int("SET2").unwrap_or(0));
                l.image.pixel(x, y).map_or(0, |[r, g, b, _]| rgb(r, g, b))
            }
            // ONMOUSE: RGB under the point, ONMOUSE2: alpha, ONMOUSE3: ARGB;
            // -1 off the layer. The point is SET/SET2 when given, else the
            // cursor; E is not checked (Erewhon asks the hidden OFF image
            // of a hovered button).
            _ => {
                let (px, py) = match (c.int("SET"), c.int("SET2")) {
                    (Some(x), Some(y)) => (x, y),
                    _ => c.cursor,
                };
                let (ax, ay) = self.tree.absolute_pos(lid);
                match l.image.pixel(px - ax, py - ay) {
                    None => -1,
                    Some([r, g, b, a]) => match q {
                        "ONMOUSE" => rgb(r, g, b),
                        "ONMOUSE2" => i64::from(a),
                        _ => (i64::from(a) << 24) | rgb(r, g, b),
                    },
                }
            }
        };
        Some(Value::Int(v))
    }

    fn cgend(&mut self, c: &NativeCall<'_>) {
        if let Some(lid) = c.bytes("ID").and_then(|id| self.tree.find(id)) {
            self.tree.remove(lid);
        }
    }

    /// The area X/Y–X2/Y2 of an image, or all of it. A missing corner
    /// keeps the image edge (Erewhon gives X/X2 alone for column copies).
    fn area(c: &NativeCall<'_>, img: &RgbaImage) -> Rect {
        let all = Rect::of(img);
        if !(c.has("X") || c.has("Y") || c.has("X2") || c.has("Y2")) {
            return all;
        }
        Rect {
            x0: c.int("X").unwrap_or(all.x0),
            y0: c.int("Y").unwrap_or(all.y0),
            x1: c.int("X2").unwrap_or(all.x1),
            y1: c.int("Y2").unwrap_or(all.y1),
        }
    }

    /// The image of layer `id`: its own, or for a set, the set composed
    /// (the manual's screenshot use, `CGACT[COPY2=1 ID=:/W0/ ID2=TMP]`).
    fn layer_image(&self, id: &[u8]) -> Option<Arc<RgbaImage>> {
        let lid = self.tree.find(id)?;
        let l = self.tree.get(lid)?;
        if !l.is_set {
            return Some(l.image.clone());
        }
        let (w, h) = if l.sx > 0 && l.sy > 0 { (l.sx as u32, l.sy as u32) } else { self.screen };
        Some(Arc::new(compose::compose_set(&self.tree, lid, w, h).to_image()))
    }

    /// Stores `img` as the image of `id2`, creating it; `attrs_from`
    /// copies the attributes too (COPY, as opposed to COPY2).
    fn put_image(&mut self, id2: &[u8], img: RgbaImage, attrs_from: Option<&[u8]>) {
        let src = attrs_from.and_then(|s| self.tree.find(s)).and_then(|s| self.tree.get(s).cloned());
        let Some((lid, _)) = self.tree.find_or_create(id2) else { return };
        let Some(l) = self.tree.get_mut(lid) else { return };
        if let Some(s) = src {
            l.enabled = s.enabled;
            l.alpha = s.alpha;
            l.x = s.x;
            l.y = s.y;
            l.z = s.z;
            l.mode = s.mode;
            l.mid = s.mid;
            l.tid = s.tid;
            l.t = s.t;
            l.ta = s.ta;
        }
        l.is_set = false;
        l.image = Arc::new(img);
        l.file.clear();
    }

    /// Runs `f` on layer `id`'s image in place.
    fn edit_image(&mut self, id: &[u8], f: impl FnOnce(&mut RgbaImage)) {
        let Some(lid) = self.tree.find(id) else { return };
        let Some(l) = self.tree.get_mut(lid) else { return };
        if !l.is_set {
            f(Arc::make_mut(&mut l.image));
        }
    }

    /// CGACT. Each switch parameter is one operation; ID is the source,
    /// ID2 (when given) the destination.
    fn cgact(&mut self, c: &NativeCall<'_>) {
        const OPS: &[&str] = &[
            "COPY", "COPY2", "SWAP", "SWAP2", "SIZE", "SIZE2", "SIZE4", "ROTATION", "RESIZE",
            "TRIM", "MERGE", "MERGE2", "SAIDO", "MEIDO", "AKARUSA", "MONO", "VERTICAL",
            "HORIZONTAL", "MOSAIC", "BLUR", "BLUR5", "BLUR6", "DIFFUSION", "REVERSE",
            "RECTPAINT", "RECTPAINT2", "RECTPAINT3", "TEXT", "TRANSITION",
        ];
        let Some(id) = c.bytes("ID").map(<[u8]>::to_vec) else { return };
        let id2 = c.bytes("ID2").map(<[u8]>::to_vec);
        let Some(op) = c.first_flag(OPS) else {
            let given: Vec<&str> = c.args.iter().map(|a| a.param).collect();
            self.note(format!("CGACT without a known operation: {given:?}"));
            return;
        };
        let set = c.int("SET").unwrap_or(0);
        match op {
            "COPY" | "COPY2" => {
                let Some(src) = self.layer_image(&id) else { return };
                let part = ops::crop(&src, Self::area(c, &src));
                let attrs = (op == "COPY").then_some(id.as_slice());
                self.put_image(id2.as_deref().unwrap_or(&id), part, attrs);
            }
            "SWAP" | "SWAP2" => {
                let (Some(a), Some(b)) = (self.tree.find(&id), id2.as_deref().and_then(|i| self.tree.find(i))) else { return };
                let (Some(la), Some(lb)) = (self.tree.get(a).cloned(), self.tree.get(b).cloned()) else { return };
                let swap_attrs = op == "SWAP";
                for (dst, src) in [(a, lb), (b, la)] {
                    if let Some(l) = self.tree.get_mut(dst) {
                        l.image = src.image.clone();
                        l.file = src.file.clone();
                        if swap_attrs {
                            l.enabled = src.enabled;
                            l.alpha = src.alpha;
                            l.x = src.x;
                            l.y = src.y;
                            l.z = src.z;
                            l.mode = src.mode;
                        }
                    }
                }
            }
            "SIZE" | "SIZE2" | "SIZE4" => {
                let Some(src) = self.layer_image(&id) else { return };
                let part = ops::crop(&src, Self::area(c, &src));
                let w = c.int("SX").unwrap_or(i64::from(part.width)).max(0) as u32;
                let h = c.int("SY").unwrap_or(i64::from(part.height)).max(0) as u32;
                if c.has("FX") || c.has("FSX") {
                    self.note("CGACT SIZE: FX/FY/FSX/FSY ignored".into());
                }
                let out = if op == "SIZE" { ops::scale_nearest(&part, w, h) } else { ops::scale_bilinear(&part, w, h) };
                match &id2 {
                    Some(d) => self.put_image(d, out, Some(&id)),
                    None => self.edit_image(&id, |img| *img = out),
                }
            }
            "RESIZE" => {
                let (l, t, r, b) = (c.int("X").unwrap_or(0), c.int("Y").unwrap_or(0), c.int("X2").unwrap_or(0), c.int("Y2").unwrap_or(0));
                self.edit_image(&id, |img| *img = ops::resize_edges(img, l, t, r, b));
            }
            "TRIM" | "ROTATION" => {
                let Some(lid) = self.tree.find(&id) else { return };
                let Some(lay) = self.tree.get_mut(lid) else { return };
                let (img, ox, oy) = if op == "TRIM" {
                    ops::trim(&lay.image, if set == 0 { 15 } else { set })
                } else {
                    ops::rotate(&lay.image, c.float("SET").unwrap_or(0.0))
                };
                lay.image = Arc::new(img);
                lay.x += ox;
                lay.y += oy;
            }
            "MERGE" | "MERGE2" => {
                // MERGE draws ID over ID2 with ID's opacity; MERGE2 is said
                // to apply both layers' opacity, which here is the same
                // (ID2's opacity stays an attribute of ID2). X/Y place ID.
                let Some(dst) = id2 else { return };
                let Some(src) = self.layer_image(&id) else { return };
                let alpha = self.tree.find(&id).and_then(|l| self.tree.get(l)).map_or(256, |l| l.alpha);
                let (x, y) = (c.int("X").unwrap_or(0), c.int("Y").unwrap_or(0));
                self.edit_image(&dst, |img| ops::merge(img, &src, x, y, alpha));
            }
            "TEXT" => self.cgact_text(c, &id),
            "TRANSITION" => self.note("CGACT TRANSITION is not implemented".into()),
            _ => {
                // Pixel effects: on the area of ID in place, or on a copy of
                // that area stored in ID2.
                let Some(src) = self.layer_image(&id) else { return };
                let area = Self::area(c, &src);
                let mut img = match &id2 {
                    Some(_) => ops::crop(&src, area),
                    None => (*src).clone(),
                };
                let r = if id2.is_some() { Rect::of(&img) } else { area };
                let set_rgb = set & 0xffffff;
                match op {
                    "SAIDO" => ops::saturation(&mut img, r, set),
                    "MEIDO" => ops::channel_gain(&mut img, r, set_rgb),
                    "AKARUSA" => ops::brightness(&mut img, r, set),
                    "MONO" => ops::mono(&mut img, r, set),
                    "VERTICAL" => ops::flip(&mut img, true),
                    "HORIZONTAL" => ops::flip(&mut img, false),
                    "MOSAIC" => ops::mosaic(&mut img, r, set),
                    "BLUR" | "BLUR5" | "BLUR6" | "DIFFUSION" => ops::blur(&mut img, r, set.max(1)),
                    "REVERSE" => ops::invert(&mut img, r),
                    "RECTPAINT" => ops::paint_rgb(&mut img, r, set),
                    "RECTPAINT2" => ops::paint_alpha(&mut img, r, set),
                    "RECTPAINT3" => ops::paint_argb(&mut img, r, set),
                    _ => {}
                }
                if matches!(op, "BLUR5" | "BLUR6" | "DIFFUSION") {
                    self.note(format!("CGACT {op} drawn as a box blur"));
                }
                match &id2 {
                    Some(d) => self.put_image(d, img, Some(&id)),
                    None => self.edit_image(&id, |dst| *dst = img),
                }
            }
        }
    }

    /// CGACT TEXT: SETSTR drawn into layer ID at X/Y with FONT slot BNO.
    /// SET (not in the manual; read from the v481 engine's glyph routine)
    /// draws the string several times in the same color: 1-10 at the nine
    /// offsets (-SET..SET in steps of SET, both axes), an outline; 11 and
    /// up at (d, 0), (0, d) and (d, d) with d = SET - 10, a drop shadow.
    fn cgact_text(&mut self, c: &NativeCall<'_>, id: &[u8]) {
        let Some(s) = c.bytes("SETSTR") else { return };
        let text = self.decode(s);
        let slot = self.fonts.slot(c.int("BNO").unwrap_or(0).max(0) as usize);
        let face = self.decode(&slot.name);
        let (x, y) = (c.int("X").unwrap_or(0), c.int("Y").unwrap_or(0));
        let offsets = text_offsets(c.int("SET").unwrap_or(0));
        let fonts = self.fonts.clone();
        let mut drew = true;
        self.edit_image(id, |img| {
            for &(dx, dy) in &offsets {
                drew = fonts.draw(img, x + dx, y + dy, &slot, &face, &text);
            }
        });
        if !drew {
            self.note("CGACT TEXT: no font registered".into());
        }
    }

    fn font(&mut self, c: &NativeCall<'_>) {
        let bno = c.int("BNO").unwrap_or(0).max(0) as usize;
        let face_name = c
            .int("NO")
            .and_then(|n| usize::try_from(n).ok())
            .and_then(|n| self.fonts.face_name(n.saturating_sub(1)).map(|s| s.as_bytes().to_vec()));
        let slot = self.fonts.slot_mut(bno);
        if let Some(n) = face_name {
            slot.name = n;
        }
        for a in c.args {
            let n = match &a.value {
                Value::Int(n) => *n,
                Value::Float(f) => *f as i64,
                Value::Str(_) => 0,
            };
            match a.param {
                "NAME" => slot.name = str_of(&a.value),
                "COLOR" => slot.color = n as u32 & 0xffffff,
                "COLOR2" => slot.color2 = n as u32 & 0xffffff,
                "SX" => slot.sx = n,
                "SY" => slot.sy = n,
                "BOLD" => slot.bold = n != 0,
                "ANTIALIASING" => slot.antialias = n != 0,
                "DRAWMODE" => slot.drawmode = n,
                "GRADATION" => slot.gradation = n,
                "ROTATION" => slot.rotation = n,
                _ => {}
            }
        }
    }

    fn fontinfo(&mut self, c: &NativeCall<'_>) -> Option<Value> {
        let slot = self.fonts.slot(c.int("BNO").unwrap_or(0).max(0) as usize);
        let q = c.first_flag(&[
            "COLOR", "COLOR2", "SX", "SY", "BOLD", "ANTIALIASING", "DRAWMODE", "NUM", "NAME", "LANG",
        ])?;
        Some(match q {
            "COLOR" => Value::Int(i64::from(slot.color)),
            "COLOR2" => Value::Int(i64::from(slot.color2)),
            "SX" => Value::Int(slot.sx),
            "SY" => Value::Int(slot.sy),
            "BOLD" => Value::Int(i64::from(slot.bold)),
            "ANTIALIASING" => Value::Int(i64::from(slot.antialias)),
            "DRAWMODE" => Value::Int(slot.drawmode),
            "NUM" => Value::Int(self.fonts.face_count() as i64),
            // LANG: whether face NO supports the game's language. The engine
            // answered non-zero for every installed font (kemonomichi2
            // trace); the registered faces are CJK.
            "LANG" => Value::Int(1),
            _ => match c.int("NO") {
                Some(n) => {
                    let name = self.fonts.face_name((n.max(1) - 1) as usize).unwrap_or("").to_string();
                    Value::Str(self.encoding.encode(&name).0.into_owned())
                }
                None => Value::Str(slot.name),
            },
        })
    }

    fn mixer(&self) -> std::sync::MutexGuard<'_, Mixer> {
        self.audio.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Reads a sound file: as given, then with `.ogg`, then `.wav`.
    fn load_sound(&mut self, file: &[u8]) -> Option<Vec<u8>> {
        let path = self.decode(file);
        let fs = self.fs.clone()?;
        let data = fs.resolve_with_ext(&path, &["ogg", "wav"]).and_then(|p| fs.read(&p).ok());
        if data.is_none() {
            self.note(format!("SOUND: cannot load {path}"));
        }
        data
    }

    fn sound(&mut self, c: &NativeCall<'_>) {
        let Some(id) = c.bytes("ID").map(<[u8]>::to_vec) else { return };
        let file = match c.bytes("FILE") {
            Some(f) => {
                let f = f.to_vec();
                let data = self.load_sound(&f);
                Some((f, data))
            }
            None => None,
        };
        let call = SoundCall {
            file,
            enabled: c.int("E").map(|n| n != 0),
            play: c.int("PLAY").map(|n| n != 0),
            pause: c.int("PAUSE").map(|n| n != 0),
            fade: c.int("FADE"),
            start: c.int("START"),
            restart: c.int("RESTART"),
            end: c.int("END"),
            loop_count: c.int("LOOP"),
            volume: c.int("VOLUME"),
            pan: c.int("PAN"),
            pitch: c.int("PITCH").or_else(|| c.int("SPEED")),
        };
        self.mixer().sound(&id, call);
    }

    /// SOUNDINFO: the first query parameter given decides the answer; a
    /// missing layer answers 0 (FILE: "").
    fn soundinfo(&mut self, c: &NativeCall<'_>) -> Option<Value> {
        let id = c.bytes("ID")?.to_vec();
        let q = c.first_flag(&[
            "EXIST", "E", "PLAY", "PAUSE", "VOLUME", "PAN", "PITCH", "FILE", "FILETYPE", "START", "RESTART",
            "END", "LOOP", "LOOPCOUNT", "TIME", "TOTALTIME", "CHANNEL", "BIT", "SAMPLING", "DATALEFT",
            "DATARIGHT", "CODEC",
        ])?;
        let m = self.mixer();
        let Some(l) = m.layers.get(&id) else {
            return Some(if q == "FILE" { Value::Str(Vec::new()) } else { Value::Int(0) });
        };
        let (channels, bits, rate) = l.format();
        Some(match q {
            "FILE" => Value::Str(l.file.clone()),
            q => Value::Int(match q {
                "EXIST" => 1,
                "E" => i64::from(l.enabled),
                "PLAY" => i64::from(l.playing),
                "PAUSE" => i64::from(l.paused),
                "VOLUME" => l.volume,
                "PAN" => l.pan,
                "PITCH" => l.pitch,
                "FILETYPE" | "CODEC" => l.codec,
                "START" => l.start_ms,
                "RESTART" => l.restart_ms,
                "END" => l.end_ms,
                "LOOP" => l.loop_count,
                "LOOPCOUNT" => l.loops_done,
                "TIME" => l.time_ms(),
                "TOTALTIME" => l.total_ms(),
                "CHANNEL" => channels,
                "BIT" => bits,
                "SAMPLING" => rate,
                "DATALEFT" => l.levels()[0],
                _ => l.levels()[1],
            }),
        })
    }

    /// WINDOWINFO for the main window (NO=0): client and window size are
    /// the screen, never full screen, always active.
    fn windowinfo(&mut self, c: &NativeCall<'_>) -> Option<Value> {
        let no = c.int("NO").unwrap_or(0);
        let q = c.first_flag(&[
            "EXIST", "X", "Y", "SX", "SY", "WSX", "WSY", "FULLSCREEN", "FULLSCREEN2", "ACTIVE",
            "CAPTION", "DISPLAYNO",
        ])?;
        if no != 0 {
            return Some(Value::Int(0));
        }
        Some(match q {
            "EXIST" | "ACTIVE" => Value::Int(1),
            "SX" | "WSX" => Value::Int(i64::from(self.screen.0)),
            "SY" | "WSY" => Value::Int(i64::from(self.screen.1)),
            "CAPTION" => Value::Str(self.caption.clone()),
            _ => Value::Int(0),
        })
    }
}

/// A string parameter's bytes (empty for a number).
fn str_of(v: &Value) -> Vec<u8> {
    match v {
        Value::Str(b) => b.clone(),
        _ => Vec::new(),
    }
}

fn rgb(r: u8, g: u8, b: u8) -> i64 {
    (i64::from(r) << 16) | (i64::from(g) << 8) | i64::from(b)
}

impl Native for Host {
    fn call(&mut self, c: &NativeCall<'_>) -> Result<Option<Value>> {
        Ok(match c.name {
            "CG" => {
                self.cg(c);
                None
            }
            "CGINFO" => self.cginfo(c),
            "CGEND" => {
                self.cgend(c);
                None
            }
            "CGACT" => {
                self.cgact(c);
                None
            }
            "FONT" => {
                self.font(c);
                None
            }
            "FONTINFO" => self.fontinfo(c),
            "SOUND" => {
                self.sound(c);
                None
            }
            "SOUNDEND" => {
                let id = c.bytes("ID").map(<[u8]>::to_vec);
                self.mixer().remove(id.as_deref());
                None
            }
            "SOUNDINFO" => self.soundinfo(c),
            "WINDOWINFO" => self.windowinfo(c),
            _ => None,
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Where CGACT TEXT draws its copies for a SET value (see `cgact_text`).
fn text_offsets(set: i64) -> Vec<(i64, i64)> {
    match set {
        1..=10 => [set, 0, -set]
            .iter()
            .flat_map(|&dx| [-set, 0, set].map(|dy| (dx, dy)))
            .collect(),
        11.. => {
            let d = set - 10;
            vec![(d, 0), (0, d), (d, d)]
        }
        _ => vec![(0, 0)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yuris_vm::native::NativeArg;

    fn call<'a>(name: &'static str, args: &'a [NativeArg], cursor: (i64, i64)) -> NativeCall<'a> {
        NativeCall { command: 0, name, args, has_let: true, cursor, clock_ms: 0 }
    }

    fn arg(param: &'static str, value: Value) -> NativeArg {
        NativeArg { slot: 0, param, value }
    }

    #[test]
    fn onmouse_answers_pixel_under_cursor() {
        let mut h = Host::new(None, (64, 64));
        let (set, _) = h.tree.find_or_create(b"S/").unwrap();
        h.tree.get_mut(set).unwrap().x = 10;
        let (l, _) = h.tree.find_or_create(b"S/B").unwrap();
        {
            let l = h.tree.get_mut(l).unwrap();
            l.image = Arc::new(ops::solid(4, 4, [0x12, 0x34, 0x56, 0x80]));
            l.x = 2;
            l.enabled = false;
        }
        let q = |h: &mut Host, what: &'static str, cursor| {
            let args = [arg("ID", Value::Str(b"S/B".to_vec())), arg(what, Value::Int(1))];
            h.cginfo(&call("CGINFO", &args, cursor))
        };
        assert_eq!(q(&mut h, "ONMOUSE2", (12, 0)), Some(Value::Int(0x80)));
        assert_eq!(q(&mut h, "ONMOUSE", (15, 3)), Some(Value::Int(0x123456)));
        assert_eq!(q(&mut h, "ONMOUSE3", (12, 0)), Some(Value::Int(0x80123456)));
        assert_eq!(q(&mut h, "ONMOUSE2", (11, 0)), Some(Value::Int(-1)));
        assert_eq!(q(&mut h, "SX", (0, 0)), Some(Value::Int(4)));
        let args = [arg("ID", Value::Str(b"NOPE".to_vec())), arg("ONMOUSE2", Value::Int(1))];
        assert_eq!(h.cginfo(&call("CGINFO", &args, (0, 0))), Some(Value::Int(0)));
    }

    #[test]
    fn cgact_copy_paint_merge() {
        let mut h = Host::new(None, (8, 8));
        let (a, _) = h.tree.find_or_create(b"A").unwrap();
        h.tree.get_mut(a).unwrap().image = Arc::new(ops::solid(4, 4, [0, 0, 0, 0]));
        let args = [
            arg("ID", Value::Str(b"A".to_vec())),
            arg("RECTPAINT3", Value::Int(1)),
            arg("SET", Value::Int(0xff00ff00)),
            arg("X", Value::Int(1)),
            arg("X2", Value::Int(3)),
        ];
        h.cgact(&call("CGACT", &args, (0, 0)));
        let img = h.tree.get(a).unwrap().image.clone();
        assert_eq!(img.pixel(0, 0), Some([0, 0, 0, 0]));
        assert_eq!(img.pixel(1, 3), Some([0, 255, 0, 255]));
        let args = [
            arg("ID", Value::Str(b"A".to_vec())),
            arg("ID2", Value::Str(b"B".to_vec())),
            arg("COPY2", Value::Int(1)),
        ];
        h.cgact(&call("CGACT", &args, (0, 0)));
        let b = h.tree.find(b"B").unwrap();
        assert_eq!(h.tree.get(b).unwrap().image.pixel(2, 2), Some([0, 255, 0, 255]));
    }

    #[test]
    fn text_set_offsets() {
        assert_eq!(text_offsets(0), [(0, 0)]);
        let o = text_offsets(3);
        assert_eq!(o.len(), 9);
        assert!(o.contains(&(-3, 3)) && o.contains(&(0, 0)) && o.contains(&(3, -3)));
        assert_eq!(text_offsets(11), [(1, 0), (0, 1), (1, 1)]);
    }
}
