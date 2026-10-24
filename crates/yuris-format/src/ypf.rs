//! YPF archives.
//!
//! Format notes: `docs/formats/ypf.md`.
//!
//! ```text
//! header (0x20)
//!   u8  magic[4] = "YPF\0"
//!   u32 version
//!   u32 file_count
//!   u32 header_size   // end of the index; excludes the header before v300
//!   u8  pad[16]
//! entry × file_count
//!   u32 name_hash     // hash of the decrypted lowercase name (murmur2 / crc32 / adler32 by version)
//!   u8  name_size     // encrypted: NAME_SIZE_MAP[v ^ 0xFF]
//!   u8  name[name_size] // each byte XOR name_key (usually 0xFF; the v500 sample uses 0xC9)
//!   u8  file_type
//!   u8  compression   // 0 = stored, 1 = zlib
//!   u32 uncompressed_len
//!   u32 stored_len
//!   u32|u64 offset    // u64 in newer versions (about 477 on)
//!   u32 data_hash
//! ```
//!
//! The upstream reader took `name_size` for the first name character and
//! `file_type` for a NUL terminator, which happened to work on one sample
//! only (names gained a `$`/`%` prefix, the last entry was 4 bytes short).
//! The sample's u32 at 0x20, `0x0033AE52`, is `murmur2("ysbin\yst00034.ybn")`,
//! the first entry's name_hash, which confirms the layout above.
//!
//! Offset width and name_key are inferred from the archive itself, not from
//! the version number (custom commercial builds report unreliable versions).
//!
//! An archive can also live inside another file (an Enigma Virtual Box
//! package): [`YpfReader::open_at`] reads it in place at a base offset.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};

use flate2::read::ZlibDecoder;
use yuris_core::{murmur2, Error, Reader, Result};

/// YPF magic, `YPF\0`.
pub const YPF_MAGIC: [u8; 4] = *b"YPF\0";

/// Header length.
pub const HEADER_LEN: usize = 0x20;

/// `compression` value for zlib.
pub const COMPRESSION_ZLIB: u8 = 1;

/// name_size table: `NAME_SIZE_MAP[enc ^ 0xFF]`. It is an involution, so
/// encryption uses the same table.
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

/// Decrypts a name_size byte.
pub fn decrypt_name_size(v: u8) -> u8 {
    NAME_SIZE_MAP[(v ^ 0xFF) as usize]
}

/// Encrypts a name_size byte (for tests and packing).
pub fn encrypt_name_size(v: u8) -> u8 {
    NAME_SIZE_MAP[v as usize] ^ 0xFF
}

/// Entry name hash; it changed between versions, so parsing tries each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameHash {
    /// From 0.477.
    Murmur2,
    /// From 0.265.
    Crc32,
    /// From 0.200.
    Adler32,
}

impl NameHash {
    /// All candidates.
    pub const ALL: [NameHash; 3] = [NameHash::Murmur2, NameHash::Crc32, NameHash::Adler32];

    /// Hashes `data`.
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

/// YPF header.
#[derive(Debug, Clone, Copy)]
pub struct YpfHeader {
    /// Archive version (such as 500).
    pub version: u32,
    /// Number of entries.
    pub file_count: u32,
    /// The header_size field, as stored.
    pub header_size: u32,
}

/// One index entry.
#[derive(Debug, Clone)]
pub struct YpfEntry {
    /// Decrypted name: raw bytes, usually CP932, `\` separated.
    pub name: Vec<u8>,
    /// Stored name hash.
    pub name_hash: u32,
    /// File type code (follows the extension).
    pub file_type: u8,
    /// 0 = stored, 1 = zlib.
    pub compression: u8,
    /// Unpacked length.
    pub uncompressed_len: u32,
    /// Stored length.
    pub stored_len: u32,
    /// Data offset from the start of the archive.
    pub offset: u64,
    /// Data hash, kept as stored and not checked.
    pub data_hash: u32,
}

impl YpfEntry {
    /// The name as a lossy string, for logs and tools.
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

/// Normalizes a name for lookup: `/` becomes `\` and ASCII is lowercased.
/// Trail bytes of CP932 double-byte characters stay as they are, since
/// they can fall on `A`..`Z` and `\`.
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

/// Archive index, without the data.
#[derive(Debug, Clone)]
pub struct YpfIndex {
    /// Header.
    pub header: YpfHeader,
    /// Name XOR key (inferred).
    pub name_key: u8,
    /// Whether offsets are u64 (inferred).
    pub wide_offsets: bool,
    /// The hash that matches name_hash; None when none could be confirmed.
    pub name_hash: Option<NameHash>,
    /// All entries, in index order.
    pub entries: Vec<YpfEntry>,
    map: HashMap<Vec<u8>, usize>,
}

impl YpfIndex {
    /// Parses the header and index; `buf` must reach the end of the index.
    ///
    /// `file_len` is the archive length, used to check offsets. A `name_key`
    /// of None is inferred.
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

        // Offset width: try the one the version suggests, then the other.
        // The right one ends exactly at the index end with every offset
        // inside the file.
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
            // Duplicate names: the first one wins (engine behaviour unconfirmed)
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

    /// Reads only the header and index of an archive file.
    pub fn from_path(path: &std::path::Path, name_key: Option<u8>) -> Result<Self> {
        let len = std::fs::metadata(path)?.len();
        Self::from_range(path, 0, len, name_key)
    }

    /// Reads the header and index of an archive stored at `base..base+len`
    /// inside `path`. Entry offsets stay relative to `base`.
    pub fn from_range(path: &std::path::Path, base: u64, len: u64, name_key: Option<u8>) -> Result<Self> {
        let mut f = std::fs::File::open(path)?;
        if base.saturating_add(len) > f.metadata()?.len() {
            return Err(Error::format(format!("YPF range {base:#x}+{len:#x} beyond {}", path.display())));
        }
        if len < HEADER_LEN as u64 {
            return Err(Error::format("YPF too short"));
        }
        let mut head = [0u8; HEADER_LEN];
        f.seek(SeekFrom::Start(base))?;
        f.read_exact(&mut head)?;
        let header_size = u32::from_le_bytes(head[12..16].try_into().unwrap());
        // The index ends at most at header_size + 0x20 (before v300
        // header_size excludes the header).
        let want = (header_size as u64 + HEADER_LEN as u64).min(len);
        let mut buf = vec![0u8; want as usize];
        f.seek(SeekFrom::Start(base))?;
        f.read_exact(&mut buf)?;
        Self::parse(&buf, len, name_key)
    }

    /// Finds an entry by name (compared after normalizing).
    pub fn find(&self, name: &[u8]) -> Option<&YpfEntry> {
        self.map.get(&normalize_name(name)).map(|&i| &self.entries[i])
    }
}

/// An entry with its name still encrypted.
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
    // Index end: header_size includes the header from v300 and excludes it
    // before; either match is accepted.
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

/// Infers name_key: first by checking the name hash for every key (most
/// reliable), then by voting on the `.` before a 3-letter extension.
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

/// A whole archive in memory (for small archives).
#[derive(Debug)]
pub struct YpfArchive {
    data: Vec<u8>,
    index: YpfIndex,
}

impl YpfArchive {
    /// Parses an archive held in memory; a `name_key` of None is inferred.
    pub fn from_bytes(data: Vec<u8>, name_key: Option<u8>) -> Result<Self> {
        let index = YpfIndex::parse(&data, data.len() as u64, name_key)?;
        Ok(Self { data, index })
    }

    /// The index.
    pub fn index(&self) -> &YpfIndex {
        &self.index
    }

    /// The header.
    pub fn header(&self) -> &YpfHeader {
        &self.index.header
    }

    /// All entries.
    pub fn entries(&self) -> &[YpfEntry] {
        &self.index.entries
    }

    /// Finds an entry by name.
    pub fn entry(&self, name: &str) -> Option<&YpfEntry> {
        self.index.find(name.as_bytes())
    }

    /// Reads and unpacks an entry.
    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let e = self
            .entry(name)
            .ok_or_else(|| Error::format(format!("entry not found: {name}")))?;
        self.read_entry(e)
    }

    /// Reads an entry of this archive.
    pub fn read_entry(&self, e: &YpfEntry) -> Result<Vec<u8>> {
        let start = e.offset as usize;
        let raw = self
            .data
            .get(start..start + e.stored_len as usize)
            .ok_or_else(|| Error::format(format!("entry {}: data out of bounds", e.name_lossy())))?;
        e.decode(raw.to_vec())
    }
}

/// An archive read from disk: the index stays in memory and data is read on
/// demand (image archives run to hundreds of MB).
#[derive(Debug)]
pub struct YpfReader {
    path: std::path::PathBuf,
    base: u64,
    name: String,
    index: YpfIndex,
}

impl YpfReader {
    /// Opens an archive file and parses its index.
    pub fn open(path: &std::path::Path, name_key: Option<u8>) -> Result<Self> {
        let index = YpfIndex::from_path(path, name_key)?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(Self {
            path: path.to_path_buf(),
            base: 0,
            name,
            index,
        })
    }

    /// Opens an archive stored as is at `base..base+len` inside another file
    /// (such as an Enigma Virtual Box package). `name` is for display.
    pub fn open_at(
        path: &std::path::Path,
        base: u64,
        len: u64,
        name: String,
        name_key: Option<u8>,
    ) -> Result<Self> {
        let index = YpfIndex::from_range(path, base, len, name_key)?;
        Ok(Self {
            path: path.to_path_buf(),
            base,
            name,
            index,
        })
    }

    /// The file holding the archive.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Offset of the archive in that file (0 for a plain archive file).
    pub fn base(&self) -> u64 {
        self.base
    }

    /// Display name: the file name, or the name inside its package.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The index.
    pub fn index(&self) -> &YpfIndex {
        &self.index
    }

    /// Reads and unpacks an entry by name.
    pub fn read(&self, name: &[u8]) -> Result<Vec<u8>> {
        let e = self.index.find(name).ok_or_else(|| {
            Error::format(format!("entry not found: {:?}", String::from_utf8_lossy(name)))
        })?;
        self.read_entry(e)
    }

    /// Reads the first `n` bytes of an entry (for image headers); a
    /// compressed entry is unpacked whole.
    pub fn read_prefix(&self, e: &YpfEntry, n: usize) -> Result<Vec<u8>> {
        if e.compression != 0 {
            let all = self.read_entry(e)?;
            return Ok(all[..n.min(all.len())].to_vec());
        }
        let mut f = std::fs::File::open(&self.path)?;
        f.seek(SeekFrom::Start(self.base + e.offset))?;
        let mut raw = vec![0u8; n.min(e.stored_len as usize)];
        f.read_exact(&mut raw)?;
        Ok(raw)
    }

    /// Reads an entry of this archive.
    pub fn read_entry(&self, e: &YpfEntry) -> Result<Vec<u8>> {
        let mut f = std::fs::File::open(&self.path)?;
        f.seek(SeekFrom::Start(self.base + e.offset))?;
        let mut raw = vec![0u8; e.stored_len as usize];
        f.read_exact(&mut raw)?;
        e.decode(raw)
    }
}

/// Builds a YPF archive (for tests and tools).
///
/// `entries` are (name, file type, zlib?, contents).
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
        // Sample: 0xED -> 18 = len("ysbin\yst00034.ybn")
        assert_eq!(decrypt_name_size(0xED), 18);
    }

    #[test]
    fn sample_first_entry_hash() {
        // The old docs record the sample's u32 at 0x20 as 0x0033AE52
        assert_eq!(murmur2(b"ysbin\\yst00034.ybn"), 0x0033_AE52);
    }

    #[test]
    fn normalize_keeps_sjis_trail_bytes() {
        // "ソ" is 83 5C: the trail byte 0x5C is `\` and must not change
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

        // The same archive inside a bigger file.
        let inner = build(500, 0xFF, true, NameHash::Murmur2, &files);
        let mut outer = b"EVB\0 some table".to_vec();
        let base = outer.len() as u64;
        outer.extend(&inner);
        outer.extend(b"trailing data");
        let p = dir.join("chs.mys");
        std::fs::write(&p, &outer).unwrap();
        let r = YpfReader::open_at(&p, base, inner.len() as u64, "chs.mys:bn.ypf".into(), None).unwrap();
        assert_eq!(r.read(b"a.txt").unwrap(), b"hello");
        assert_eq!(r.read(b"ysbin\\ysc.ybn").unwrap(), b"YSCM");
        assert!(YpfReader::open_at(&p, base, outer.len() as u64, String::new(), None).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
