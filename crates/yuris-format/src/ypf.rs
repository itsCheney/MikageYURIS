//! YPF 封包解析。
//!
//! 规格:`docs/formats/ypf.md`(2026-10-24 按公开格式重写)。
//!
//! ```text
//! header (0x20)
//!   u8  magic[4] = "YPF\0"
//!   u32 version
//!   u32 file_count
//!   u32 header_size   // 索引区结束位置;< 300 版本不含 header 自身
//!   u8  pad[16]
//! entry × file_count
//!   u32 name_hash     // 解密后小写名的哈希(murmur2 / crc32 / adler32,随版本)
//!   u8  name_size     // 加密:NAME_SIZE_MAP[v ^ 0xFF]
//!   u8  name[name_size] // 逐字节 XOR name_key(常见 0xFF,样本 v500 为 0xC9)
//!   u8  file_type
//!   u8  compression   // 0 = stored, 1 = zlib
//!   u32 uncompressed_len
//!   u32 stored_len
//!   u32|u64 offset    // 新版本(约 477 起)为 u64
//!   u32 data_hash
//! ```
//!
//! 旧实现把 `name_size` 当作名字首字符、把 `file_type` 当作 NUL 终止符,
//! 只在单一样本上碰巧成立(名字多出 `$`/`%` 前缀、末条残差 −4 字节)。
//! 样本 0x20 处的 `0x0033AE52` = `murmur2("ysbin\yst00034.ybn")`,即首条的
//! name_hash,印证了上述布局。
//!
//! 偏移宽度与 name_key 都从封包自身推断,不依赖版本号(商业定制版的版本号
//! 不可信,见 yuri.md「Special versions」)。

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};

use flate2::read::ZlibDecoder;
use yuris_core::{murmur2, Error, Reader, Result};

/// YPF magic:`YPF\0`。
pub const YPF_MAGIC: [u8; 4] = *b"YPF\0";

/// header 长度。
pub const HEADER_LEN: usize = 0x20;

/// compression 字段:zlib。
pub const COMPRESSION_ZLIB: u8 = 1;

/// name_size 解密表:`NAME_SIZE_MAP[enc ^ 0xFF]`(自反置换,加密同表)。
pub const NAME_SIZE_MAP: [u8; 256] = {
    let mut m = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        m[i] = i as u8;
        i += 1;
    }
    let swaps: [(usize, usize); 12] = [
        (3, 72),
        (17, 25),
        (46, 50),
        (6, 53),
        (9, 11),
        (12, 16),
        (13, 19),
        (21, 27),
        (28, 30),
        (32, 35),
        (38, 41),
        (44, 47),
    ];
    let mut j = 0;
    while j < swaps.len() {
        let (a, b) = swaps[j];
        let t = m[a];
        m[a] = m[b];
        m[b] = t;
        j += 1;
    }
    m
};

/// 解密 name_size 字节。
pub fn decrypt_name_size(v: u8) -> u8 {
    NAME_SIZE_MAP[(v ^ 0xFF) as usize]
}

/// 加密 name_size(测试与打包用)。
pub fn encrypt_name_size(v: u8) -> u8 {
    NAME_SIZE_MAP[v as usize] ^ 0xFF
}

/// 条目名哈希算法(随版本变化;解析时逐个尝试)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameHash {
    /// 0.477 起。
    Murmur2,
    /// 0.265 起。
    Crc32,
    /// 0.200 起。
    Adler32,
}

impl NameHash {
    /// 全部候选。
    pub const ALL: [NameHash; 3] = [NameHash::Murmur2, NameHash::Crc32, NameHash::Adler32];

    /// 计算哈希。
    pub fn hash(self, data: &[u8]) -> u32 {
        match self {
            NameHash::Murmur2 => murmur2(data),
            NameHash::Crc32 => {
                let mut c = flate2::Crc::new();
                c.update(data);
                c.sum()
            }
            NameHash::Adler32 => adler32(data),
        }
    }
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// YPF 文件头。
#[derive(Debug, Clone, Copy)]
pub struct YpfHeader {
    /// 封包版本(如 500)。
    pub version: u32,
    /// 条目数。
    pub file_count: u32,
    /// 头部字段 header_size(原样)。
    pub header_size: u32,
}

/// 索引条目。
#[derive(Debug, Clone)]
pub struct YpfEntry {
    /// 解密后的条目名(原始字节,通常 CP932,分隔符 `\`)。
    pub name: Vec<u8>,
    /// 存储的名字哈希。
    pub name_hash: u32,
    /// 文件类型码(与扩展名相关)。
    pub file_type: u8,
    /// 0 = stored,1 = zlib。
    pub compression: u8,
    /// 解压后长度。
    pub uncompressed_len: u32,
    /// 数据区占用长度。
    pub stored_len: u32,
    /// 数据区绝对偏移。
    pub offset: u64,
    /// 数据哈希(原样保留,未校验)。
    pub data_hash: u32,
}

impl YpfEntry {
    /// 条目名的有损字符串形式(日志/工具用)。
    pub fn name_lossy(&self) -> String {
        let (s, _, _) = encoding_rs::SHIFT_JIS.decode(&self.name);
        s.into_owned()
    }

    fn decode(&self, raw: Vec<u8>) -> Result<Vec<u8>> {
        match self.compression {
            0 => Ok(raw),
            COMPRESSION_ZLIB => {
                let mut out = Vec::with_capacity(self.uncompressed_len as usize);
                ZlibDecoder::new(&raw[..]).read_to_end(&mut out)?;
                if out.len() != self.uncompressed_len as usize {
                    return Err(Error::format(format!(
                        "entry {}: decompressed {} bytes, header says {}",
                        self.name_lossy(),
                        out.len(),
                        self.uncompressed_len
                    )));
                }
                Ok(out)
            }
            other => Err(Error::format(format!(
                "entry {}: unknown compression {other:#x}",
                self.name_lossy()
            ))),
        }
    }
}

/// 条目名归一:`/` → `\`,ASCII 小写;CP932 双字节字符的尾字节保持不变
/// (尾字节可落在 `A`..`Z` 与 `\` 上)。
pub fn normalize_name(name: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(name.len());
    let mut i = 0;
    while i < name.len() {
        let b = name[i];
        if is_sjis_lead(b) && i + 1 < name.len() {
            out.push(b);
            out.push(name[i + 1]);
            i += 2;
            continue;
        }
        out.push(match b {
            b'/' => b'\\',
            b'A'..=b'Z' => b + 32,
            _ => b,
        });
        i += 1;
    }
    out
}

fn is_sjis_lead(b: u8) -> bool {
    matches!(b, 0x81..=0x9F | 0xE0..=0xFC)
}

/// 封包索引(不含数据区)。
#[derive(Debug, Clone)]
pub struct YpfIndex {
    /// 头部。
    pub header: YpfHeader,
    /// 名字 XOR key(推断所得)。
    pub name_key: u8,
    /// 偏移字段是否为 u64(推断所得)。
    pub wide_offsets: bool,
    /// 与 name_hash 吻合的哈希算法(None = 未能确认)。
    pub name_hash: Option<NameHash>,
    /// 全部条目(索引顺序)。
    pub entries: Vec<YpfEntry>,
    map: HashMap<Vec<u8>, usize>,
}

impl YpfIndex {
    /// 从头部 + 索引区字节解析(`buf` 至少覆盖到索引区末尾)。
    ///
    /// `file_len` = 整个封包长度,用于校验偏移;`name_key` 为 None 时自动推断。
    pub fn parse(buf: &[u8], file_len: u64, name_key: Option<u8>) -> Result<Self> {
        let mut r = Reader::new(buf);
        let magic = r.u32_bytes()?;
        if magic != YPF_MAGIC {
            return Err(Error::BadMagic {
                expected: YPF_MAGIC,
                actual: magic,
            });
        }
        let header = YpfHeader {
            version: r.u32_le()?,
            file_count: r.u32_le()?,
            header_size: r.u32_le()?,
        };

        // 偏移宽度:按版本先试,失败再试另一种;以「恰好走完索引区且偏移
        // 均落在文件内」为准。
        let prefer_wide = header.version >= 477;
        let mut last_err = None;
        let mut raw = None;
        for wide in [prefer_wide, !prefer_wide] {
            match walk_entries(buf, &header, wide, file_len) {
                Ok(v) => {
                    raw = Some((wide, v));
                    break;
                }
                Err(e) => last_err = Some(e),
            }
        }
        let (wide_offsets, raw) = match raw {
            Some(v) => v,
            None => return Err(last_err.unwrap_or_else(|| Error::format("YPF index unreadable"))),
        };

        let (name_key, name_hash) = match name_key {
            Some(k) => (k, detect_hash(&raw, k)),
            None => detect_name_key(&raw)?,
        };

        let mut entries = Vec::with_capacity(raw.len());
        let mut map = HashMap::with_capacity(raw.len());
        for (enc_name, mut e) in raw {
            e.name = enc_name.iter().map(|b| b ^ name_key).collect();
            // 同名条目:先出现者优先(与旧实现一致;引擎行为未确认)
            map.entry(normalize_name(&e.name)).or_insert(entries.len());
            entries.push(e);
        }
        Ok(Self {
            header,
            name_key,
            wide_offsets,
            name_hash,
            entries,
            map,
        })
    }

    /// 从磁盘文件只读头部与索引区。
    pub fn from_path(path: &std::path::Path, name_key: Option<u8>) -> Result<Self> {
        let mut f = std::fs::File::open(path)?;
        let file_len = f.metadata()?.len();
        let mut head = [0u8; HEADER_LEN];
        f.read_exact(&mut head)?;
        let header_size = u32::from_le_bytes(head[12..16].try_into().unwrap());
        // 索引区最远到 header_size + 0x20(< 300 版本 header_size 不含头)。
        let want = (header_size as u64 + HEADER_LEN as u64).min(file_len);
        let mut buf = vec![0u8; want as usize];
        f.seek(SeekFrom::Start(0))?;
        f.read_exact(&mut buf)?;
        Self::parse(&buf, file_len, name_key)
    }

    /// 按名字查找(归一后比较)。
    pub fn find(&self, name: &[u8]) -> Option<&YpfEntry> {
        self.map.get(&normalize_name(name)).map(|&i| &self.entries[i])
    }
}

/// 解析结果里尚未解密的名字 + 条目其余字段。
type RawEntry = (Vec<u8>, YpfEntry);

fn walk_entries(buf: &[u8], h: &YpfHeader, wide: bool, file_len: u64) -> Result<Vec<RawEntry>> {
    let mut r = Reader::new(buf);
    r.seek(HEADER_LEN);
    let mut out = Vec::with_capacity(h.file_count as usize);
    for i in 0..h.file_count {
        let name_hash = r.u32_le()?;
        let size = decrypt_name_size(r.u8()?) as usize;
        let enc_name = r.bytes(size)?.to_vec();
        let file_type = r.u8()?;
        let compression = r.u8()?;
        let uncompressed_len = r.u32_le()?;
        let stored_len = r.u32_le()?;
        let offset = if wide {
            let lo = r.u32_le()? as u64;
            let hi = r.u32_le()? as u64;
            lo | (hi << 32)
        } else {
            r.u32_le()? as u64
        };
        let data_hash = r.u32_le()?;
        if compression > 1 {
            return Err(Error::format(format!("entry #{i}: compression {compression:#x}")));
        }
        if offset.saturating_add(stored_len as u64) > file_len {
            return Err(Error::format(format!(
                "entry #{i}: data {offset:#x}+{stored_len:#x} beyond file ({file_len:#x})"
            )));
        }
        out.push((
            enc_name,
            YpfEntry {
                name: Vec::new(),
                name_hash,
                file_type,
                compression,
                uncompressed_len,
                stored_len,
                offset,
                data_hash,
            },
        ));
    }
    // 索引区终点:>= 300 版本 header_size 含头,< 300 不含;两者择一吻合即可。
    let end = r.pos() as u64;
    let hs = h.header_size as u64;
    if end != hs && end != hs + HEADER_LEN as u64 {
        return Err(Error::format(format!(
            "YPF index ends at {end:#x}, header_size says {hs:#x}"
        )));
    }
    Ok(out)
}

fn hash_matches(raw: &[RawEntry], key: u8, algo: NameHash) -> bool {
    raw.iter().take(8).all(|(enc, e)| {
        let name: Vec<u8> = enc.iter().map(|b| b ^ key).collect();
        algo.hash(&normalize_name(&name)) == e.name_hash || algo.hash(&name) == e.name_hash
    })
}

fn detect_hash(raw: &[RawEntry], key: u8) -> Option<NameHash> {
    NameHash::ALL.into_iter().find(|&a| hash_matches(raw, key, a))
}

/// 推断 name_key:先用名字哈希逐 key 验证(最可靠);哈希对不上时退回
/// 「扩展名前的 `.`」启发(`xxx.ybn` / `.png` / `.ogg`)。
fn detect_name_key(raw: &[RawEntry]) -> Result<(u8, Option<NameHash>)> {
    if raw.is_empty() {
        return Ok((0xFF, None));
    }
    for algo in NameHash::ALL {
        for key in 0..=255u8 {
            if hash_matches(raw, key, algo) {
                return Ok((key, Some(algo)));
            }
        }
    }
    let mut votes = [0u32; 256];
    for (enc, _) in raw {
        if enc.len() >= 4 {
            votes[(enc[enc.len() - 4] ^ b'.') as usize] += 1;
        }
    }
    let (key, n) = votes
        .iter()
        .enumerate()
        .max_by_key(|(_, &n)| n)
        .map(|(k, &n)| (k as u8, n))
        .unwrap();
    if n * 2 < raw.len() as u32 {
        return Err(Error::format("YPF name key not determinable"));
    }
    Ok((key, None))
}

/// 内存中的整个封包(脚本包等小封包用)。
#[derive(Debug)]
pub struct YpfArchive {
    data: Vec<u8>,
    index: YpfIndex,
}

impl YpfArchive {
    /// 从内存字节解析;`name_key` 为 None 时自动推断。
    pub fn from_bytes(data: Vec<u8>, name_key: Option<u8>) -> Result<Self> {
        let index = YpfIndex::parse(&data, data.len() as u64, name_key)?;
        Ok(Self { data, index })
    }

    /// 索引。
    pub fn index(&self) -> &YpfIndex {
        &self.index
    }

    /// 文件头。
    pub fn header(&self) -> &YpfHeader {
        &self.index.header
    }

    /// 全部条目。
    pub fn entries(&self) -> &[YpfEntry] {
        &self.index.entries
    }

    /// 按名字查找条目。
    pub fn entry(&self, name: &str) -> Option<&YpfEntry> {
        self.index.find(name.as_bytes())
    }

    /// 读取一个条目并解压。
    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let e = self
            .entry(name)
            .ok_or_else(|| Error::format(format!("entry not found: {name}")))?;
        self.read_entry(e)
    }

    /// 读取给定条目(须属于本封包)。
    pub fn read_entry(&self, e: &YpfEntry) -> Result<Vec<u8>> {
        let start = e.offset as usize;
        let raw = self
            .data
            .get(start..start + e.stored_len as usize)
            .ok_or_else(|| Error::format(format!("entry {}: data out of bounds", e.name_lossy())))?;
        e.decode(raw.to_vec())
    }
}

/// 基于文件的封包读取器:索引常驻,数据按需 seek(数百 MB 的图像包用)。
#[derive(Debug)]
pub struct YpfReader {
    path: std::path::PathBuf,
    index: YpfIndex,
}

impl YpfReader {
    /// 打开封包并解析索引区。
    pub fn open(path: &std::path::Path, name_key: Option<u8>) -> Result<Self> {
        let index = YpfIndex::from_path(path, name_key)?;
        Ok(Self {
            path: path.to_path_buf(),
            index,
        })
    }

    /// 封包路径。
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// 索引。
    pub fn index(&self) -> &YpfIndex {
        &self.index
    }

    /// 读取条目并解压。
    pub fn read(&self, name: &[u8]) -> Result<Vec<u8>> {
        let e = self.index.find(name).ok_or_else(|| {
            Error::format(format!("entry not found: {:?}", String::from_utf8_lossy(name)))
        })?;
        self.read_entry(e)
    }

    /// 读取给定条目的前 `n` 字节(未压缩条目;读图像头用)。
    pub fn read_prefix(&self, e: &YpfEntry, n: usize) -> Result<Vec<u8>> {
        if e.compression != 0 {
            let all = self.read_entry(e)?;
            return Ok(all[..n.min(all.len())].to_vec());
        }
        let mut f = std::fs::File::open(&self.path)?;
        f.seek(SeekFrom::Start(e.offset))?;
        let mut raw = vec![0u8; n.min(e.stored_len as usize)];
        f.read_exact(&mut raw)?;
        Ok(raw)
    }

    /// 读取给定条目(须属于本封包)。
    pub fn read_entry(&self, e: &YpfEntry) -> Result<Vec<u8>> {
        let mut f = std::fs::File::open(&self.path)?;
        f.seek(SeekFrom::Start(e.offset))?;
        let mut raw = vec![0u8; e.stored_len as usize];
        f.read_exact(&mut raw)?;
        e.decode(raw)
    }
}

/// 构造 YPF 封包(测试与工具用)。
///
/// `entries` = (名字, 文件类型, 是否 zlib, 内容)。
pub fn build(
    version: u32,
    name_key: u8,
    wide: bool,
    hash: NameHash,
    entries: &[(&[u8], u8, bool, &[u8])],
) -> Vec<u8> {
    use std::io::Write;
    let index_len: usize = entries
        .iter()
        .map(|(n, ..)| 4 + 1 + n.len() + 2 + 8 + if wide { 8 } else { 4 } + 4)
        .sum();
    let data_start = HEADER_LEN + index_len;
    let mut blobs = Vec::new();
    for (_, _, zlib, content) in entries {
        if *zlib {
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            z.write_all(content).unwrap();
            blobs.push(z.finish().unwrap());
        } else {
            blobs.push(content.to_vec());
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(&YPF_MAGIC);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    let header_size = if version >= 300 { data_start } else { index_len } as u32;
    out.extend_from_slice(&header_size.to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);
    let mut off = data_start as u64;
    for ((name, ty, zlib, content), blob) in entries.iter().zip(&blobs) {
        out.extend_from_slice(&hash.hash(&normalize_name(name)).to_le_bytes());
        out.push(encrypt_name_size(name.len() as u8));
        out.extend(name.iter().map(|b| b ^ name_key));
        out.push(*ty);
        out.push(u8::from(*zlib));
        out.extend_from_slice(&(content.len() as u32).to_le_bytes());
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        if wide {
            out.extend_from_slice(&off.to_le_bytes());
        } else {
            out.extend_from_slice(&(off as u32).to_le_bytes());
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        off += blob.len() as u64;
    }
    for blob in &blobs {
        out.extend_from_slice(blob);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_size_map_roundtrip() {
        for v in 0..=255u8 {
            assert_eq!(decrypt_name_size(encrypt_name_size(v)), v);
        }
        // 样本:0xED → 18 = len("ysbin\yst00034.ybn")
        assert_eq!(decrypt_name_size(0xED), 18);
    }

    #[test]
    fn sample_first_entry_hash() {
        // 旧文档记录的样本 0x20 处 u32 = 0x0033AE52
        assert_eq!(murmur2(b"ysbin\\yst00034.ybn"), 0x0033_AE52);
    }

    #[test]
    fn normalize_keeps_sjis_trail_bytes() {
        // 「ソ」= 83 5C:尾字节 0x5C 是 `\`,不能被改动
        assert_eq!(normalize_name(b"CG/A\x83\x5Cb.PNG"), b"cg\\a\x83\x5Cb.png".to_vec());
    }

    #[test]
    fn roundtrip_all_layouts_and_detects_key() {
        let files: [(&[u8], u8, bool, &[u8]); 3] = [
            (b"ysbin\\yst00000.ybn", 0, true, b"YSTB script body"),
            (b"cg\\bg\\BG01.png", 3, false, b"\x89PNG\r\n\x1a\nxxxx"),
            (b"se\\\x83\x5C.ogg", 6, false, b"OggS"),
        ];
        for (version, key, wide, hash) in [
            (500, 0xC9, true, NameHash::Murmur2),
            (480, 0xFF, true, NameHash::Murmur2),
            (300, 0xFF, false, NameHash::Crc32),
            (250, 0x90, false, NameHash::Adler32),
        ] {
            let bytes = build(version, key, wide, hash, &files);
            let ypf = YpfArchive::from_bytes(bytes, None).unwrap();
            assert_eq!(ypf.index().name_key, key, "v{version}");
            assert_eq!(ypf.index().wide_offsets, wide, "v{version}");
            assert_eq!(ypf.index().name_hash, Some(hash), "v{version}");
            assert_eq!(ypf.read("YSBIN/yst00000.ybn").unwrap(), b"YSTB script body");
            assert_eq!(ypf.read("cg\\bg\\bg01.png").unwrap(), files[1].3);
            assert_eq!(ypf.entries()[2].name, files[2].0);
        }
    }

    #[test]
    fn reader_reads_from_disk() {
        let dir = std::env::temp_dir().join(format!("ypf-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bn.ypf");
        let files: [(&[u8], u8, bool, &[u8]); 2] = [
            (b"ysbin\\ysc.ybn", 0, true, b"YSCM"),
            (b"a.txt", 0, false, b"hello"),
        ];
        std::fs::write(&p, build(500, 0xFF, true, NameHash::Murmur2, &files)).unwrap();
        let r = YpfReader::open(&p, None).unwrap();
        assert_eq!(r.read(b"ysbin\\ysc.ybn").unwrap(), b"YSCM");
        let e = r.index().find(b"A.TXT").unwrap();
        assert_eq!(r.read_prefix(e, 3).unwrap(), b"hel");
        std::fs::remove_dir_all(&dir).ok();
    }
}
