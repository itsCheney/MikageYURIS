//! The CG layer tree.
//!
//! YU-RIS manages layers by ID, as paths (manual, 概要 › ＣＧレイヤ):
//! `/` separates layer sets, a trailing `/` names a set itself, `.` and `..`
//! work as in file paths, and a leading `:` is the root. The root holds the
//! set `W0`, the main window, which is also the current path at start, so
//! `CG[ID=ABC]` creates `:/W0/ABC`. A set composes its children (by Z,
//! higher in front) and is then drawn with its own opacity and mode; a set
//! with a size clips its children to it. Layer names may contain `.`
//! (`BT.OFFES.TITLE.BTN.START=1=1`); only a whole `.` or `..` component is
//! special.

use std::sync::Arc;

use yuris_format::vfs::RgbaImage;

/// Index of a layer in its [`LayerTree`].
pub type LayerId = usize;

/// One layer or layer set.
#[derive(Debug, Clone)]
pub struct Layer {
    /// Last path component (raw bytes, the game's encoding).
    pub name: Vec<u8>,
    /// Parent set (None for the root).
    pub parent: Option<LayerId>,
    /// Children, in creation order (drawing sorts them by Z).
    pub children: Vec<LayerId>,
    /// A layer set rather than an image layer.
    pub is_set: bool,
    /// The image (image layers; straight alpha).
    pub image: Arc<RgbaImage>,
    /// The file the image was loaded from (empty for generated images).
    pub file: Vec<u8>,
    /// E: enabled (drawn).
    pub enabled: bool,
    /// A: opacity, 0..=256.
    pub alpha: i64,
    /// X, relative to the parent set.
    pub x: i64,
    /// Y, relative to the parent set.
    pub y: i64,
    /// Z: drawing priority, higher in front.
    pub z: i64,
    /// SX: set width (sets only; 0 = unclipped).
    pub sx: i64,
    /// SY: set height (sets only; 0 = unclipped).
    pub sy: i64,
    /// MODE: blend mode (0 normal, 1 add, 2 subtract, 3 multiply, 5 screen, ...).
    pub mode: i64,
    /// MID: mask layer ID.
    pub mid: Vec<u8>,
    /// TID: transition layer ID.
    pub tid: Vec<u8>,
    /// T: transition progress, 0..=256.
    pub t: i64,
    /// TA: transition accuracy.
    pub ta: i64,
}

impl Layer {
    fn new(name: Vec<u8>, parent: Option<LayerId>, is_set: bool) -> Self {
        Self {
            name,
            parent,
            children: Vec::new(),
            is_set,
            image: Arc::new(empty_image()),
            file: Vec::new(),
            enabled: true,
            alpha: 256,
            x: 0,
            y: 0,
            z: 0,
            sx: 0,
            sy: 0,
            mode: 0,
            mid: Vec::new(),
            tid: Vec::new(),
            t: 0,
            ta: 0,
        }
    }

    /// Width: the image's, or SX for a set.
    pub fn width(&self) -> i64 {
        if self.is_set {
            self.sx
        } else {
            i64::from(self.image.width)
        }
    }

    /// Height: the image's, or SY for a set.
    pub fn height(&self) -> i64 {
        if self.is_set {
            self.sy
        } else {
            i64::from(self.image.height)
        }
    }
}

/// A 0×0 image.
pub fn empty_image() -> RgbaImage {
    RgbaImage { width: 0, height: 0, data: Vec::new() }
}

/// A parsed layer ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerPath {
    /// Starts at the root (`:`) rather than the current set.
    pub absolute: bool,
    /// Components, with `.` dropped (`..` kept, resolved on lookup).
    pub parts: Vec<Vec<u8>>,
    /// Ends in `/`: names a layer set.
    pub set: bool,
}

impl LayerPath {
    /// Parses a layer ID.
    pub fn parse(id: &[u8]) -> Self {
        let mut s = id;
        let absolute = s.first() == Some(&b':');
        if absolute {
            s = &s[1..];
        }
        let set = s.last() == Some(&b'/');
        let parts = s
            .split(|&b| b == b'/')
            .filter(|p| !p.is_empty() && *p != b".")
            .map(<[u8]>::to_vec)
            .collect();
        Self { absolute, parts, set }
    }
}

/// The layer tree: the root set, `W0` in it, and the current set.
#[derive(Debug, Clone)]
pub struct LayerTree {
    slots: Vec<Option<Layer>>,
    free: Vec<LayerId>,
    root: LayerId,
    cwd: LayerId,
    /// Bumped on every change, so a renderer can skip unchanged frames.
    pub generation: u64,
}

impl Default for LayerTree {
    fn default() -> Self {
        Self::new()
    }
}

impl LayerTree {
    /// A tree with the root and `W0`, current set `W0`.
    pub fn new() -> Self {
        let mut t = Self {
            slots: vec![Some(Layer::new(Vec::new(), None, true))],
            free: Vec::new(),
            root: 0,
            cwd: 0,
            generation: 0,
        };
        let w0 = t.insert(0, b"W0".to_vec(), true);
        t.cwd = w0;
        t
    }

    /// The root set.
    pub fn root(&self) -> LayerId {
        self.root
    }

    /// The set of window `n` (`:/Wn`), if it exists.
    pub fn window_set(&self, n: u32) -> Option<LayerId> {
        self.child(self.root, format!("W{n}").as_bytes())
    }

    /// A layer by index.
    pub fn get(&self, id: LayerId) -> Option<&Layer> {
        self.slots.get(id).and_then(Option::as_ref)
    }

    /// A layer by index, for changing.
    pub fn get_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.generation += 1;
        self.slots.get_mut(id).and_then(Option::as_mut)
    }

    fn child(&self, parent: LayerId, name: &[u8]) -> Option<LayerId> {
        self.get(parent)?
            .children
            .iter()
            .copied()
            .find(|&c| self.get(c).is_some_and(|l| l.name == name))
    }

    fn insert(&mut self, parent: LayerId, name: Vec<u8>, is_set: bool) -> LayerId {
        let layer = Layer::new(name, Some(parent), is_set);
        let id = match self.free.pop() {
            Some(id) => {
                self.slots[id] = Some(layer);
                id
            }
            None => {
                self.slots.push(Some(layer));
                self.slots.len() - 1
            }
        };
        if let Some(p) = self.slots[parent].as_mut() {
            p.children.push(id);
        }
        self.generation += 1;
        id
    }

    /// Looks a layer ID up.
    pub fn find(&self, id: &[u8]) -> Option<LayerId> {
        let path = LayerPath::parse(id);
        let mut cur = if path.absolute { self.root } else { self.cwd };
        for part in &path.parts {
            cur = if part == b".." {
                self.get(cur)?.parent.unwrap_or(self.root)
            } else {
                self.child(cur, part)?
            };
        }
        Some(cur)
    }

    /// Looks a layer ID up, creating it (and any missing sets on the way)
    /// when absent. Returns the layer and whether it was created. A path
    /// that resolves to a set (`ABC/`, `:`, `..`) is never created as an
    /// image layer.
    pub fn find_or_create(&mut self, id: &[u8]) -> Option<(LayerId, bool)> {
        let path = LayerPath::parse(id);
        let mut cur = if path.absolute { self.root } else { self.cwd };
        let mut created = false;
        let n = path.parts.len();
        for (i, part) in path.parts.iter().enumerate() {
            if part == b".." {
                cur = self.get(cur)?.parent.unwrap_or(self.root);
                continue;
            }
            cur = match self.child(cur, part) {
                Some(c) => c,
                None => {
                    created = true;
                    let is_set = i + 1 < n || path.set;
                    self.insert(cur, part.clone(), is_set)
                }
            };
        }
        Some((cur, created))
    }

    /// Removes a layer and everything under it. The root and the window
    /// sets (`:/W0/`) are emptied instead of removed.
    pub fn remove(&mut self, id: LayerId) {
        if id == self.root {
            return;
        }
        let Some(parent) = self.get(id).and_then(|l| l.parent) else { return };
        if parent == self.root {
            let kids = self.get(id).map(|l| l.children.clone()).unwrap_or_default();
            for k in kids {
                self.remove(k);
            }
            return;
        }
        if let Some(p) = self.slots[parent].as_mut() {
            p.children.retain(|&c| c != id);
        }
        let mut stack = vec![id];
        while let Some(i) = stack.pop() {
            if let Some(l) = self.slots[i].take() {
                stack.extend(l.children);
                self.free.push(i);
            }
        }
        if self.get(self.cwd).is_none() {
            self.cwd = self.window_set(0).unwrap_or(self.root);
        }
        self.generation += 1;
    }

    /// Children of a set, back to front (by Z, then creation order).
    pub fn draw_order(&self, set: LayerId) -> Vec<LayerId> {
        let Some(l) = self.get(set) else { return Vec::new() };
        let mut kids = l.children.clone();
        kids.sort_by_key(|&k| self.get(k).map_or(0, |c| c.z));
        kids
    }

    /// A layer's position on screen: its X/Y plus those of its sets.
    pub fn absolute_pos(&self, id: LayerId) -> (i64, i64) {
        let (mut x, mut y) = (0, 0);
        let mut cur = Some(id);
        while let Some(i) = cur {
            let Some(l) = self.get(i) else { break };
            x += l.x;
            y += l.y;
            cur = l.parent;
        }
        (x, y)
    }

    /// The rule transition layer `l` is drawn through: TID names an image
    /// layer (the rule, usually hidden), T is the progress (0 hidden, 256
    /// shown) and TA the softness of the edge. None when there is none or
    /// it has finished (T 256).
    pub fn transition(&self, l: &Layer) -> Option<Transition> {
        if l.tid.is_empty() || l.t >= 256 {
            return None;
        }
        let id = self.find(&l.tid)?;
        let rule = self.get(id)?;
        if rule.is_set || rule.image.width == 0 || rule.image.height == 0 {
            return None;
        }
        let (x, y) = self.absolute_pos(id);
        let vague = l.ta.max(1) as f32;
        Some(Transition { rule: rule.image.clone(), x, y, t: l.t.max(0) as f32 / 256.0 * (255.0 + vague), vague })
    }

    /// Full path of a layer (`:/W0/ABC`), for listings.
    pub fn path_of(&self, id: LayerId) -> Vec<u8> {
        let mut parts = Vec::new();
        let mut cur = Some(id);
        while let Some(i) = cur {
            let Some(l) = self.get(i) else { break };
            if l.parent.is_some() {
                parts.push(l.name.clone());
            }
            cur = l.parent;
        }
        parts.reverse();
        let mut out = b":".to_vec();
        for p in parts {
            out.push(b'/');
            out.extend(p);
        }
        if self.get(id).is_some_and(|l| l.is_set) {
            out.push(b'/');
        }
        out
    }

    /// Every live layer, in index order.
    pub fn iter(&self) -> impl Iterator<Item = (LayerId, &Layer)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.as_ref().map(|l| (i, l)))
    }

    /// Number of live layers (root included).
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|l| l.is_some()).count()
    }

    /// Whether only the root and its window sets are left.
    pub fn is_empty(&self) -> bool {
        self.len() <= 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_resolve_like_folders() {
        let mut t = LayerTree::new();
        let (a, created) = t.find_or_create(b"ABC").unwrap();
        assert!(created);
        assert_eq!(t.path_of(a), b":/W0/ABC");
        let (set, _) = t.find_or_create(b"S/").unwrap();
        assert!(t.get(set).unwrap().is_set);
        let (c, _) = t.find_or_create(b"S/CG1").unwrap();
        assert_eq!(t.get(c).unwrap().parent, Some(set));
        assert_eq!(t.find(b":/W0/S/CG1"), Some(c));
        assert_eq!(t.find(b"S/../ABC"), Some(a));
        let (w2, _) = t.find_or_create(b":W2/TX01").unwrap();
        assert_eq!(t.path_of(w2), b":/W2/TX01");
        assert_eq!(t.find(b"BT.OFF.X=1"), None);
        t.remove(set);
        assert_eq!(t.find(b"S/CG1"), None);
        assert_eq!(t.find(b"ABC"), Some(a));
    }
}

/// A rule ("universal") transition: a pixel shows where the rule image is
/// darker than the progress, with an edge `vague` levels wide, so dark
/// areas of the rule appear first as T rises. Not reversed from the
/// engine: the direction and the meaning of TA are the usual ones of
/// such transitions.
#[derive(Debug, Clone)]
pub struct Transition {
    /// The rule image (its red channel is read).
    pub rule: Arc<RgbaImage>,
    /// The rule's position on screen.
    pub x: i64,
    /// The rule's position on screen.
    pub y: i64,
    /// Progress in rule levels: T / 256 · (255 + vague).
    pub t: f32,
    /// Edge width in rule levels (TA, at least 1).
    pub vague: f32,
}

impl Transition {
    /// How much of the pixel at screen (sx, sy) shows, 0..=1. The rule's
    /// edge pixels extend past it.
    pub fn coverage(&self, sx: i64, sy: i64) -> f32 {
        let (w, h) = (i64::from(self.rule.width), i64::from(self.rule.height));
        let rx = (sx - self.x).clamp(0, w - 1) as usize;
        let ry = (sy - self.y).clamp(0, h - 1) as usize;
        self.coverage_of(self.rule.data[(ry * w as usize + rx) * 4])
    }

    /// Coverage for a rule level.
    pub fn coverage_of(&self, level: u8) -> f32 {
        ((self.t - f32::from(level)) / self.vague).clamp(0.0, 1.0)
    }
}
