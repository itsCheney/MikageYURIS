//! 游戏虚拟文件系统:松散文件 + 若干 YPF 封包。
//!
//! 查找顺序(引擎侧未逆向,以下为约定):
//! 1. `save\` 前缀 → 存档目录(默认 `<game>\save`,宿主可改);
//! 2. 游戏根下的松散文件(补丁常以松散文件覆盖);
//! 3. 封包:文件名以 `update`/`patch` 开头的先挂载,其余按文件名排序;
//!    同名条目先挂载者优先。
//!
//! 脚本里的路径常省略扩展名(如 `cgsys/main/button/type1/tip_meswindow`),
//! 由引擎补全,见 [`GameFs::resolve_with_ext`]。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use yuris_core::{Error, Result};

use crate::ypf::{normalize_name, YpfEntry, YpfReader};

/// 图像扩展名候选(YSCM CG 族的图像类型)。
pub const IMAGE_EXTS: [&str; 4] = ["png", "bmp", "jpg", "gif"];

/// 游戏文件系统。
#[derive(Debug)]
pub struct GameFs {
    root: PathBuf,
    save_dir: PathBuf,
    packs: Vec<YpfReader>,
    lookup: HashMap<Vec<u8>, (usize, usize)>,
    loose: HashMap<Vec<u8>, PathBuf>,
    /// 无法解析而跳过的封包(文件名, 原因)。
    pub skipped: Vec<(String, String)>,
}

impl GameFs {
    /// 扫描游戏目录:`<game>\*.ypf` 与 `<game>\pac\*.ypf`。
    /// 传入 `pac` 目录本身时,上溯一级作为游戏根。
    pub fn open(dir: &Path) -> Result<Self> {
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
        let mut found: Vec<PathBuf> = Vec::new();
        for sub in [root.clone(), root.join("pac")] {
            let Ok(rd) = std::fs::read_dir(&sub) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                let is_ypf = p
                    .extension()
                    .map(|x| x.eq_ignore_ascii_case("ypf"))
                    .unwrap_or(false);
                if is_ypf && p.is_file() {
                    found.push(p);
                }
            }
        }
        found.sort_by_key(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let patch = name.starts_with("update") || name.starts_with("patch");
            (!patch, name)
        });

        let mut fs = Self {
            save_dir: root.join("save"),
            root,
            packs: Vec::new(),
            lookup: HashMap::new(),
            loose: HashMap::new(),
            skipped: Vec::new(),
        };
        fs.index_loose();
        for p in found {
            let label = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match YpfReader::open(&p, None) {
                Ok(r) => fs.mount(r),
                Err(e) => fs.skipped.push((label, e.to_string())),
            }
        }
        Ok(fs)
    }

    /// 挂载一个封包(优先级低于已挂载者)。
    pub fn mount(&mut self, pack: YpfReader) {
        let pi = self.packs.len();
        for (ei, e) in pack.index().entries.iter().enumerate() {
            self.lookup.entry(normalize_name(&e.name)).or_insert((pi, ei));
        }
        self.packs.push(pack);
    }

    /// 游戏根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 已挂载的封包。
    pub fn packs(&self) -> &[YpfReader] {
        &self.packs
    }

    /// 设置存档目录(`save\` 前缀映射到此)。
    pub fn set_save_dir(&mut self, dir: PathBuf) {
        self.save_dir = dir;
    }

    /// 存档目录。
    pub fn save_dir(&self) -> &Path {
        &self.save_dir
    }

    /// 重新索引游戏根下的松散文件(忽略大小写;存档目录不在索引内,实时查)。
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
                    out.insert(normalize_name(r.as_bytes()), p);
                }
            }
        }
        self.loose.clear();
        walk(&self.root, "", 0, &mut self.loose);
    }

    fn loose_path(&self, norm: &[u8]) -> Option<PathBuf> {
        if let Some(rest) = norm.strip_prefix(b"save\\") {
            let p = self.save_dir.join(String::from_utf8_lossy(rest).replace('\\', "/"));
            return p.is_file().then_some(p);
        }
        self.loose.get(norm).cloned()
    }

    fn pack_entry(&self, norm: &[u8]) -> Option<(&YpfReader, &YpfEntry)> {
        let &(pi, ei) = self.lookup.get(norm)?;
        let pack = &self.packs[pi];
        Some((pack, &pack.index().entries[ei]))
    }

    /// 文件是否存在(松散或封包内)。
    pub fn exists(&self, path: &str) -> bool {
        let norm = normalize_name(path.as_bytes());
        self.loose_path(&norm).is_some() || self.lookup.contains_key(&norm)
    }

    /// 读取文件(解压后)。
    pub fn read(&self, path: &str) -> Result<Vec<u8>> {
        let norm = normalize_name(path.as_bytes());
        if let Some(p) = self.loose_path(&norm) {
            return Ok(std::fs::read(p)?);
        }
        match self.pack_entry(&norm) {
            Some((pack, e)) => pack.read_entry(e),
            None => Err(Error::format(format!("file not found: {path}"))),
        }
    }

    /// 补扩展名解析:原名存在则原名,否则依次尝试 `exts`。返回命中的路径。
    pub fn resolve_with_ext(&self, path: &str, exts: &[&str]) -> Option<String> {
        if self.exists(path) {
            return Some(path.to_string());
        }
        exts.iter()
            .map(|x| format!("{path}.{x}"))
            .find(|c| self.exists(c))
    }

    /// 图像宽高(读文件头;PNG / BMP / JPEG)。路径可省略扩展名。
    pub fn image_size(&self, path: &str) -> Option<(u32, u32)> {
        let hit = self.resolve_with_ext(path, &IMAGE_EXTS)?;
        let norm = normalize_name(hit.as_bytes());
        let head = if let Some(p) = self.loose_path(&norm) {
            use std::io::Read;
            let mut f = std::fs::File::open(p).ok()?;
            let mut buf = vec![0u8; 64 * 1024];
            let n = f.read(&mut buf).ok()?;
            buf.truncate(n);
            buf
        } else {
            let (pack, e) = self.pack_entry(&norm)?;
            pack.read_prefix(e, 64 * 1024).ok()?
        };
        image_header_size(&head)
    }
}

/// 从文件头解析图像宽高。
pub fn image_header_size(h: &[u8]) -> Option<(u32, u32)> {
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
        // 逐段找 SOFn
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
}
