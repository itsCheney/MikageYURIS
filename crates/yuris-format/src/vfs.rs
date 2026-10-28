//! The game's virtual file system: loose files plus YPF archives, with an
//! optional Enigma Virtual Box overlay from a translation patch's launcher.
//!
//! Lookup order (the engine's own order is not reversed yet; this is our
//! convention):
//! 1. `save\` paths go to the save folder (`<game>\save` unless the host
//!    sets another) and nowhere else: save files a patch packs (Erewhon
//!    CHS ships the translator's `save\config.sd`) are left out, as the
//!    game would take them for a returning player's and skip the first-run
//!    setup that `global.sd` would otherwise carry (its message window
//!    then sits above the screen);
//! 2. files packed into the launcher exe or its external package (Enigma
//!    Virtual Box shows them ahead of the real files);
//! 3. loose files below the game folder (patches often ship loose files);
//! 4. archives: `update*`/`patch*` first, higher numbers first, then the
//!    rest by name. For duplicate names the first mounted archive wins.
//!
//! A `.ypf` inside the overlay replaces the real archive at the same path
//! (Erewhon's CHS patch ships `pac\bn.ypf` and `update9.ypf` that way).
//!
//! Scripts often leave out extensions (such as
//! `cgsys/main/button/type1/tip_meswindow`); the engine completes them, see
//! [`GameFs::resolve_with_ext`].

use std::cmp::Reverse;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use yuris_core::{Error, Result};

use crate::evb::{external_package_name, EvbFile, EvbPackage};
use crate::ypf::{normalize_name, YpfEntry, YpfReader};

/// Image extensions to try (the image types of the YSCM CG commands).
pub const IMAGE_EXTS: [&str; 4] = ["png", "bmp", "jpg", "gif"];

/// The game's file system.
#[derive(Debug)]
pub struct GameFs {
    root: PathBuf,
    save_dir: PathBuf,
    packs: Vec<YpfReader>,
    lookup: HashMap<Vec<u8>, (usize, usize)>,
    loose: HashMap<Vec<u8>, PathBuf>,
    overlay: Vec<EvbPackage>,
    virtual_files: HashMap<Vec<u8>, (usize, usize)>,
    /// Archives and packages that were skipped: (name, reason).
    pub skipped: Vec<(String, String)>,
}

/// Where a path resolved to.
enum Hit<'a> {
    Disk(PathBuf),
    Virtual(&'a EvbPackage, &'a EvbFile),
    Pack(&'a YpfReader, &'a YpfEntry),
}

/// An archive to mount: a real file, or one stored inside an overlay package.
enum PackSource {
    File(PathBuf),
    Packed(usize, usize),
}

/// Lookup key for a name as Unicode text: scripts name files in CP932.
fn cp932_key(name: &str) -> Vec<u8> {
    let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode(name);
    normalize_name(&bytes)
}

/// Mount order of an archive by its file name: `update*`/`patch*` first,
/// higher numbers first (later patches override earlier ones), then by name.
fn pack_rank(rel: &[u8]) -> (u8, Reverse<u32>, Vec<u8>) {
    let file = rel.rsplit(|&b| b == b'\\').next().unwrap_or(rel);
    let stem = file.strip_suffix(b".ypf").unwrap_or(file);
    for prefix in [&b"update"[..], b"patch"] {
        if let Some(rest) = stem.strip_prefix(prefix) {
            let n = std::str::from_utf8(rest).ok().and_then(|s| s.parse().ok()).unwrap_or(0);
            return (0, Reverse(n), rel.to_vec());
        }
    }
    (1, Reverse(0), rel.to_vec())
}

impl GameFs {
    /// Scans a game folder: `<game>\*.ypf` and `<game>\pac\*.ypf`. Given the
    /// `pac` folder itself, its parent is the game folder.
    pub fn open(dir: &Path) -> Result<Self> {
        Self::open_with_launcher(dir, None)
    }

    /// Like [`GameFs::open`], also mounting the files a launcher exe packed
    /// with Enigma Virtual Box carries, in the exe itself or in the external
    /// package it names. Their paths are relative to the game folder.
    pub fn open_with_launcher(dir: &Path, launcher: Option<&Path>) -> Result<Self> {
        let is_pac = dir
            .file_name()
            .map(|n| n.eq_ignore_ascii_case("pac"))
            .unwrap_or(false);
        let root = if is_pac {
            dir.parent().unwrap_or(dir).to_path_buf()
        } else {
            dir.to_path_buf()
        };
        if !root.is_dir() {
            return Err(Error::format(format!("not a directory: {}", root.display())));
        }
        let mut fs = Self {
            save_dir: root.join("save"),
            root,
            packs: Vec::new(),
            lookup: HashMap::new(),
            loose: HashMap::new(),
            overlay: Vec::new(),
            virtual_files: HashMap::new(),
            skipped: Vec::new(),
        };
        if let Some(exe) = launcher {
            fs.load_overlay(exe)?;
        }

        // Archives by their path below the game folder; overlay ones replace
        // real ones at the same path.
        let mut found: HashMap<Vec<u8>, (String, PackSource)> = HashMap::new();
        for (sub, prefix) in [(fs.root.clone(), ""), (fs.root.join("pac"), "pac\\")] {
            let Ok(rd) = std::fs::read_dir(&sub) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                let is_ypf = p
                    .extension()
                    .map(|x| x.eq_ignore_ascii_case("ypf"))
                    .unwrap_or(false);
                if is_ypf && p.is_file() {
                    found.insert(cp932_key(&format!("{prefix}{name}")), (name, PackSource::File(p)));
                }
            }
        }
        let mut virtual_files = HashMap::new();
        for (pi, pkg) in fs.overlay.iter().enumerate() {
            let pkg_name = pkg.path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            for (fi, f) in pkg.files().iter().enumerate() {
                let key = cp932_key(&f.name);
                if !key.ends_with(b".ypf") {
                    virtual_files.entry(key).or_insert((pi, fi));
                } else if !f.is_plain() {
                    // Compressed archives would have to be unpacked whole first.
                    fs.skipped.push((format!("{pkg_name}:{}", f.name), "compressed in its package".into()));
                } else if !found.get(&key).is_some_and(|(_, s)| matches!(s, PackSource::Packed(..))) {
                    found.insert(key, (format!("{pkg_name}:{}", f.name), PackSource::Packed(pi, fi)));
                }
            }
        }
        fs.virtual_files = virtual_files;

        let mut found: Vec<_> = found.into_iter().collect();
        found.sort_by_key(|(rel, _)| pack_rank(rel));
        fs.index_loose();
        for (_, (label, source)) in found {
            let opened = match source {
                PackSource::File(p) => YpfReader::open(&p, None),
                PackSource::Packed(pi, fi) => {
                    let pkg = &fs.overlay[pi];
                    let f = &pkg.files()[fi];
                    YpfReader::open_at(pkg.path(), f.offset, f.size, label.clone(), None)
                }
            };
            match opened {
                Ok(r) => fs.mount(r),
                Err(e) => fs.skipped.push((label, e.to_string())),
            }
        }
        Ok(fs)
    }

    fn load_overlay(&mut self, exe: &Path) -> Result<()> {
        if let Some(pkg) = EvbPackage::open(exe)? {
            if !pkg.files().is_empty() {
                self.overlay.push(pkg);
            }
        }
        let Some(name) = external_package_name(&std::fs::read(exe)?) else {
            return Ok(());
        };
        let folder = exe.parent().unwrap_or(&self.root);
        let path = folder.join(&name);
        if !path.is_file() {
            self.skipped.push((name, "external package missing".into()));
            return Ok(());
        }
        match EvbPackage::open(&path) {
            Ok(Some(pkg)) => self.overlay.push(pkg),
            Ok(None) => self.skipped.push((name, "not an Enigma Virtual Box package".into())),
            Err(e) => self.skipped.push((name, e.to_string())),
        }
        Ok(())
    }

    /// Mounts an archive, below the ones already mounted.
    pub fn mount(&mut self, pack: YpfReader) {
        let pi = self.packs.len();
        for (ei, e) in pack.index().entries.iter().enumerate() {
            self.lookup.entry(normalize_name(&e.name)).or_insert((pi, ei));
        }
        self.packs.push(pack);
    }

    /// The game folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Mounted archives, highest priority first.
    pub fn packs(&self) -> &[YpfReader] {
        &self.packs
    }

    /// Mounted Enigma Virtual Box packages (launcher exe, external package).
    pub fn overlay(&self) -> &[EvbPackage] {
        &self.overlay
    }

    /// Sets the save folder that `save\` paths map to.
    pub fn set_save_dir(&mut self, dir: PathBuf) {
        self.save_dir = dir;
    }

    /// The save folder.
    pub fn save_dir(&self) -> &Path {
        &self.save_dir
    }

    /// Re-indexes loose files below the game folder, ignoring case. Names
    /// are keyed in CP932 like the scripts. The save folder is not indexed;
    /// it is checked live.
    pub fn index_loose(&mut self) {
        fn walk(dir: &Path, rel: &str, depth: usize, out: &mut HashMap<Vec<u8>, PathBuf>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                let r = if rel.is_empty() { name } else { format!("{rel}\\{name}") };
                if p.is_dir() {
                    if depth < 8 {
                        walk(&p, &r, depth + 1, out);
                    }
                } else {
                    out.insert(cp932_key(&r), p);
                }
            }
        }
        self.loose.clear();
        walk(&self.root, "", 0, &mut self.loose);
    }

    fn locate(&self, norm: &[u8]) -> Option<Hit<'_>> {
        if let Some(rest) = norm.strip_prefix(b"save\\") {
            let (rest, _, _) = encoding_rs::SHIFT_JIS.decode(rest);
            let p = self.save_dir.join(rest.replace('\\', "/"));
            return p.is_file().then_some(Hit::Disk(p));
        }
        if let Some(&(pi, fi)) = self.virtual_files.get(norm) {
            let pkg = &self.overlay[pi];
            return Some(Hit::Virtual(pkg, &pkg.files()[fi]));
        }
        if let Some(p) = self.loose.get(norm) {
            return Some(Hit::Disk(p.clone()));
        }
        let &(pi, ei) = self.lookup.get(norm)?;
        let pack = &self.packs[pi];
        Some(Hit::Pack(pack, &pack.index().entries[ei]))
    }

    /// Origin selected by the same precedence rules as `read`.
    pub fn source_of(&self, path: &str) -> Option<String> {
        match self.locate(&normalize_name(path.as_bytes()))? {
            Hit::Disk(p) => Some(format!("loose:{}", p.strip_prefix(&self.root).unwrap_or(&p).display())),
            Hit::Pack(pack, entry) => Some(format!("{}:{}", pack.name(), entry.name_lossy())),
            Hit::Virtual(pkg, file) => Some(format!("{}:{}", pkg.path().file_name()?.to_string_lossy(), file.name)),
        }
    }

    /// Every file name visible through the layers above, normalized (see
    /// [`normalize_name`]), without duplicates; save files are not listed.
    pub fn file_names(&self) -> Vec<Vec<u8>> {
        let mut names: Vec<Vec<u8>> = self
            .virtual_files
            .keys()
            .chain(self.loose.keys())
            .chain(self.lookup.keys())
            .cloned()
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Whether a file exists (in any layer).
    pub fn exists(&self, path: &str) -> bool {
        self.locate(&normalize_name(path.as_bytes())).is_some()
    }

    /// Reads a file, unpacked.
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        match self.locate(&normalize_name(path.as_bytes())) {
            Some(Hit::Disk(p)) => Ok(std::fs::read(p)?),
            Some(Hit::Virtual(pkg, f)) => pkg.read(f),
            Some(Hit::Pack(pack, e)) => pack.read_entry(e),
            None => Err(Error::format(format!("file not found: {path}"))),
        }
    }

    /// Resolves a path that may lack its extension: the path itself if it
    /// exists, otherwise the first of `exts` that does.
    pub fn resolve_with_ext(&self, path: &str, exts: &[&str]) -> Option<String> {
        if self.exists(path) {
            return Some(path.to_string());
        }
        exts.iter()
            .map(|x| format!("{path}.{x}"))
            .find(|c| self.exists(c))
    }

    /// Decodes an image to RGBA (PNG, JPEG or YCG), identified by content. The extension may be left
    /// out.
    pub fn image_rgba(&self, path: &str) -> Option<RgbaImage> {
        let hit = self.resolve_with_ext(path, &IMAGE_EXTS)?;
        decode_image_rgba(&self.read(&hit).ok()?)
    }

    /// Image width and height from the file header (PNG, BMP, JPEG). The
    /// extension may be left out.
    pub fn image_size(&self, path: &str) -> Option<(u32, u32)> {
        let hit = self.resolve_with_ext(path, &IMAGE_EXTS)?;
        let head = match self.locate(&normalize_name(hit.as_bytes()))? {
            Hit::Disk(p) => {
                use std::io::Read;
                let mut f = std::fs::File::open(p).ok()?;
                let mut buf = vec![0u8; 64 * 1024];
                let n = f.read(&mut buf).ok()?;
                buf.truncate(n);
                buf
            }
            Hit::Virtual(pkg, f) => pkg.read(f).ok()?,
            Hit::Pack(pack, e) => pack.read_prefix(e, 64 * 1024).ok()?,
        };
        image_header_size(&head)
    }
}

/// A decoded image, 8-bit RGBA, rows top to bottom.
#[derive(Debug, Clone)]
pub struct RgbaImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub data: Vec<u8>,
}

impl RgbaImage {
    /// The pixel at (x, y), or None outside the image.
    pub fn pixel(&self, x: i64, y: i64) -> Option<[u8; 4]> {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return None;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        self.data.get(i..i + 4).map(|p| [p[0], p[1], p[2], p[3]])
    }
}

/// Decodes a PNG into RGBA8 (palette, gray and 16-bit inputs are expanded).
pub fn decode_png_rgba(bytes: &[u8]) -> Option<RgbaImage> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return None,
    };
    let mut data = Vec::with_capacity(w * h * 4);
    for row in buf.chunks(info.line_size).take(h) {
        for px in row[..w * channels].chunks(channels) {
            data.extend_from_slice(&match channels {
                1 => [px[0], px[0], px[0], 255],
                2 => [px[0], px[0], px[0], px[1]],
                3 => [px[0], px[1], px[2], 255],
                _ => [px[0], px[1], px[2], px[3]],
            });
        }
    }
    Some(RgbaImage { width: info.width, height: info.height, data })
}

/// Decodes a YCG image, the engine's own format (Erewhon's CHS patch
/// stores its translated UI images as YCG under `.png` names).
///
/// ```text
/// u8  magic[4] = "YCG\0"
/// u32 width, height, bpp (32)
/// u32 method          // 1 = zlib (the only one seen)
/// u32 strips
/// strip × strips: u32 first_row, last_row, unpacked_len, packed_len
/// packed strip data, in order: BGRA rows, top to bottom
/// ```
pub fn decode_ycg_rgba(b: &[u8]) -> Option<RgbaImage> {
    use std::io::Read;
    let u32_at = |o: usize| b.get(o..o + 4).map(|x| u32::from_le_bytes(x.try_into().unwrap()));
    if b.get(..4)? != b"YCG\0" {
        return None;
    }
    let (w, h, bpp, method, strips) = (u32_at(4)?, u32_at(8)?, u32_at(12)?, u32_at(16)?, u32_at(20)?);
    if method != 1 || !(bpp == 32 || bpp == 24) || w == 0 || h == 0 || w > 16384 || h > 16384 {
        return None;
    }
    let src_px = (bpp / 8) as usize;
    let mut data = vec![0u8; w as usize * h as usize * 4];
    let mut pos = 24 + strips as usize * 16;
    for i in 0..strips as usize {
        let t = 24 + i * 16;
        let (first, last, unpacked, packed) = (u32_at(t)?, u32_at(t + 4)?, u32_at(t + 8)?, u32_at(t + 12)?);
        let packed = b.get(pos..pos + packed as usize)?;
        pos += packed.len();
        let mut out = Vec::with_capacity(unpacked as usize);
        flate2::read::ZlibDecoder::new(packed).read_to_end(&mut out).ok()?;
        let row_len = w as usize * src_px;
        for (k, row) in out.chunks(row_len).enumerate() {
            let y = first as usize + k;
            if y > last as usize || y >= h as usize || row.len() < row_len {
                break;
            }
            for (x, px) in row.chunks(src_px).enumerate() {
                let o = (y * w as usize + x) * 4;
                let a = if src_px == 4 { px[3] } else { 255 };
                data[o..o + 4].copy_from_slice(&[px[2], px[1], px[0], a]);
            }
        }
    }
    Some(RgbaImage { width: w, height: h, data })
}

/// Decodes a PNG or YCG image into RGBA8.
pub fn decode_image_rgba(bytes: &[u8]) -> Option<RgbaImage> {
    if bytes.starts_with(b"YCG\0") {
        decode_ycg_rgba(bytes)
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        decode_jpeg_rgba(bytes)
    } else {
        decode_png_rgba(bytes)
    }
}

/// Decodes RGB/grayscale JPEG data, including patches using `.png` names.
pub fn decode_jpeg_rgba(bytes: &[u8]) -> Option<RgbaImage> {
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    let channels = match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => 1,
        jpeg_decoder::PixelFormat::RGB24 => 3,
        _ => return None,
    };
    let len = usize::from(info.width).checked_mul(usize::from(info.height))?;
    if pixels.len() != len.checked_mul(channels)? { return None; }
    let mut data = Vec::with_capacity(len.checked_mul(4)?);
    for pixel in pixels.chunks_exact(channels) {
        let rgb = if channels == 1 { [pixel[0]; 3] } else { [pixel[0], pixel[1], pixel[2]] };
        data.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    Some(RgbaImage { width: u32::from(info.width), height: u32::from(info.height), data })
}

/// Image width and height from a file header.
pub fn image_header_size(h: &[u8]) -> Option<(u32, u32)> {
    if h.len() >= 12 && h[..4] == *b"YCG\0" {
        let w = u32::from_le_bytes(h[4..8].try_into().ok()?);
        let hh = u32::from_le_bytes(h[8..12].try_into().ok()?);
        return Some((w, hh));
    }
    if h.len() >= 24 && h[..8] == *b"\x89PNG\r\n\x1a\n" {
        let w = u32::from_be_bytes(h[16..20].try_into().ok()?);
        let hh = u32::from_be_bytes(h[20..24].try_into().ok()?);
        return Some((w, hh));
    }
    if h.len() >= 26 && h[..2] == *b"BM" {
        let w = i32::from_le_bytes(h[18..22].try_into().ok()?);
        let hh = i32::from_le_bytes(h[22..26].try_into().ok()?);
        return Some((w.unsigned_abs(), hh.unsigned_abs()));
    }
    if h.len() >= 4 && h[..2] == [0xFF, 0xD8] {
        // Walk the segments up to a SOFn marker.
        let mut i = 2;
        while i + 9 < h.len() {
            if h[i] != 0xFF {
                return None;
            }
            let marker = h[i + 1];
            let len = u16::from_be_bytes([h[i + 2], h[i + 3]]) as usize;
            if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
                let hh = u16::from_be_bytes([h[i + 5], h[i + 6]]) as u32;
                let w = u16::from_be_bytes([h[i + 7], h[i + 8]]) as u32;
                return Some((w, hh));
            }
            i += 2 + len;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ypf::{build, NameHash};

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v
    }

    #[test]
    fn loose_overrides_pack_and_ext_is_resolved() {
        let dir = std::env::temp_dir().join(format!("gamefs-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pac")).unwrap();
        std::fs::create_dir_all(dir.join("save")).unwrap();
        let tip = png(1350, 200);
        let files: [(&[u8], u8, bool, &[u8]); 2] = [
            (b"cgsys\\main\\tip.png", 3, false, &tip),
            (b"ysbin\\ysc.ybn", 0, true, b"from pack"),
        ];
        std::fs::write(dir.join("pac/bn.ypf"), build(500, 0xFF, true, NameHash::Murmur2, &files)).unwrap();
        let patch: [(&[u8], u8, bool, &[u8]); 1] = [(b"ysbin\\ysc.ybn", 0, false, b"from update")];
        std::fs::write(dir.join("pac/update1.ypf"), build(500, 0xC9, true, NameHash::Murmur2, &patch)).unwrap();
        std::fs::write(dir.join("save/00.sd"), b"YSSD").unwrap();
        std::fs::write(dir.join("pac/broken.ypf"), b"YPF\0junk").unwrap();

        let fs = GameFs::open(&dir.join("pac")).unwrap();
        assert_eq!(fs.root(), dir.as_path());
        assert_eq!(fs.skipped.len(), 1);
        assert_eq!(fs.read("ysbin/ysc.ybn").unwrap(), b"from update");
        assert_eq!(fs.image_size("cgsys/main/tip"), Some((1350, 200)));
        assert!(fs.exists("save\\00.sd"));
        assert!(!fs.exists("save\\01.sd"));
        std::fs::create_dir_all(dir.join("ysbin")).unwrap();
        std::fs::write(dir.join("ysbin/YSC.ybn"), b"loose").unwrap();
        let mut fs = fs;
        fs.index_loose();
        assert_eq!(fs.read("ysbin\\ysc.ybn").unwrap(), b"loose");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn launcher_package_overlays_the_game() {
        let dir = std::env::temp_dir().join(format!("gamefs-evb-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("pac")).unwrap();
        let ypf = |files: &[(&[u8], &[u8])]| {
            let files: Vec<(&[u8], u8, bool, &[u8])> = files.iter().map(|&(n, d)| (n, 0, false, d)).collect();
            build(481, 0xFF, true, NameHash::Murmur2, &files)
        };
        std::fs::write(
            dir.join("pac/bn.ypf"),
            ypf(&[(b"ysbin\\ysc.ybn", b"jp"), (b"ysbin\\only_jp.ybn", b"jp")]),
        )
        .unwrap();
        std::fs::write(dir.join("pac/update1.ypf"), ypf(&[(b"ysbin\\x.ybn", b"u1")])).unwrap();
        let chs_bn = ypf(&[(b"ysbin\\ysc.ybn", b"chs")]);
        let update9 = ypf(&[(b"ysbin\\x.ybn", b"u9")]);
        let tip = png(4, 3);
        let pkg = crate::evb::build_package(&[
            ("update9.ypf", &update9),
            ("pac/bn.ypf", &chs_bn),
            ("save/config.sd", b"default config"),
            ("cg/tip.png", &tip),
        ]);
        std::fs::write(dir.join("chs.mys"), pkg).unwrap();
        let mut exe = vec![0u8; 0x40];
        exe[..2].copy_from_slice(b"MZ");
        for u in " %DEFAULT FOLDER%\\chs.mys".encode_utf16() {
            exe.extend(u.to_le_bytes());
        }
        exe.extend(b"EVB\0");
        std::fs::write(dir.join("game_chs.exe"), exe).unwrap();

        let fs = GameFs::open_with_launcher(&dir, Some(&dir.join("game_chs.exe"))).unwrap();
        assert!(fs.skipped.is_empty(), "{:?}", fs.skipped);
        assert_eq!(fs.overlay().len(), 1);
        let names: Vec<_> = fs.packs().iter().map(|p| p.name()).collect();
        assert_eq!(names, ["chs.mys:update9.ypf", "update1.ypf", "chs.mys:pac/bn.ypf"]);
        // The packaged bn.ypf replaces the real one instead of stacking on it.
        assert_eq!(fs.read("ysbin\\ysc.ybn").unwrap(), b"chs");
        assert!(!fs.exists("ysbin\\only_jp.ybn"));
        assert_eq!(fs.read("ysbin\\x.ybn").unwrap(), b"u9");
        assert_eq!(fs.image_size("cg/tip"), Some((4, 3)));
        // The patch's own save files are not the player's.
        assert!(!fs.exists("save\\config.sd"));
        std::fs::create_dir_all(dir.join("save")).unwrap();
        std::fs::write(dir.join("save/config.sd"), b"saved").unwrap();
        assert_eq!(fs.read("save\\config.sd").unwrap(), b"saved");

        // Without the launcher the real archives are used.
        let plain = GameFs::open(&dir).unwrap();
        assert_eq!(plain.read("ysbin\\ysc.ybn").unwrap(), b"jp");
        assert_eq!(plain.read("ysbin\\x.ybn").unwrap(), b"u1");
        std::fs::remove_dir_all(&dir).ok();
    }
}
