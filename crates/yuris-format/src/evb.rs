// This file is derived from ethornell-rs (crates/ethornell-archive/src/evb.rs,
// commit 229baae) and stays under the Mozilla Public License 2.0:
// SPDX-License-Identifier: MPL-2.0
// A copy of the MPL is at https://mozilla.org/MPL/2.0/.

//! Files packed with Enigma Virtual Box.
//!
//! Translation patches often ship as the original exe wrapped with Enigma
//! Virtual Box. The packed files appear in the exe's folder, ahead of the
//! real files. They live either in the exe (its ".enigma1" section) or in a
//! separate package the exe names, such as Erewhon's `erewhon_chs.mys`; both
//! use the same table.
//!
//! Layout: "EVB\0" and 60 bytes, a main node, then one node per folder or
//! file in pre-order. A file's data follows the table, in table order;
//! compressed files are aPLib chunks.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use yuris_core::{Error, Result};

const MAGIC: &[u8; 4] = b"EVB\0";
const NODE_FILE: u8 = 2;
const NODE_FOLDER: u8 = 3;
/// The executable's own folder.
const DEFAULT_FOLDER: &str = "%DEFAULT FOLDER%";
const MAX_FILES: usize = 100_000;

/// One packed file.
#[derive(Debug, Clone)]
pub struct EvbFile {
    /// Path below the executable's folder, '/' separated.
    pub name: String,
    /// Unpacked size.
    pub size: u64,
    /// Offset of the stored data in the package file.
    pub offset: u64,
    /// Stored size; equal to `size` when the file is not compressed.
    pub stored: u64,
}

impl EvbFile {
    /// Whether the data is stored as is (and can be read in place).
    pub fn is_plain(&self) -> bool {
        self.stored == self.size
    }
}

/// The packed files of an exe or an external package.
#[derive(Debug, Clone)]
pub struct EvbPackage {
    path: PathBuf,
    files: Vec<EvbFile>,
}

fn error(message: impl Into<String>) -> Error {
    Error::format(format!("EVB: {}", message.into()))
}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn read_at(file: &mut std::fs::File, offset: u64, length: usize) -> Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; length];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// The file range of the ".enigma1" section, if `file` is a PE image with one.
fn enigma_section(file: &mut std::fs::File, length: u64) -> Result<Option<(u64, u64)>> {
    if length < 0x40 {
        return Ok(None);
    }
    let dos = read_at(file, 0, 0x40)?;
    if !dos.starts_with(b"MZ") {
        return Ok(None);
    }
    let pe = u32_at(&dos, 0x3C).unwrap_or(0) as u64;
    if pe + 24 > length {
        return Ok(None);
    }
    let header = read_at(file, pe, 24)?;
    if !header.starts_with(b"PE\0\0") {
        return Ok(None);
    }
    let sections = u16_at(&header, 6).unwrap_or(0) as u64;
    let optional = u16_at(&header, 20).unwrap_or(0) as u64;
    let table = pe + 24 + optional;
    if table + sections * 40 > length {
        return Ok(None);
    }
    let table = read_at(file, table, (sections * 40) as usize)?;
    for section in table.chunks_exact(40) {
        if section.starts_with(b".enigma1") {
            let size = u32_at(section, 16).unwrap_or(0) as u64;
            let start = u32_at(section, 20).unwrap_or(0) as u64;
            if start < length {
                return Ok(Some((start, size.min(length - start))));
            }
        }
    }
    Ok(None)
}

impl EvbPackage {
    /// Opens an exe packed with Enigma Virtual Box, or an external package
    /// (a file that starts with the table). None when `path` is neither.
    pub fn open(path: &Path) -> Result<Option<Self>> {
        let mut file = std::fs::File::open(path)?;
        let length = file.metadata()?.len();
        let (start, span) = match read_at(&mut file, 0, 4.min(length as usize)) {
            Ok(head) if head == MAGIC => (0, length),
            _ => match enigma_section(&mut file, length)? {
                Some(range) => range,
                None => return Ok(None),
            },
        };
        // The table sits at the start of the range; only look near it.
        let probe = read_at(&mut file, start, span.min(1 << 20) as usize)?;
        let Some(magic) = probe.windows(4).position(|w| w == MAGIC) else {
            return Ok(None);
        };
        let table_start = start + magic as u64;
        let table_len = (start + span - table_start).min(64 << 20) as usize;
        let table = read_at(&mut file, table_start, table_len)?;
        let files = parse_table(&table, table_start)?;
        if files.iter().any(|f| f.offset + f.stored > length) {
            return Err(error("file data beyond the package"));
        }
        Ok(Some(Self {
            path: path.to_path_buf(),
            files,
        }))
    }

    /// The package (or exe) path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// All packed files, in table order.
    pub fn files(&self) -> &[EvbFile] {
        &self.files
    }

    /// Reads and unpacks one file.
    pub fn read(&self, f: &EvbFile) -> Result<Vec<u8>> {
        let mut file = std::fs::File::open(&self.path)?;
        let stored = read_at(&mut file, f.offset, f.stored as usize)?;
        if f.is_plain() {
            return Ok(stored);
        }
        unpack_chunks(&stored, f.size as usize).map_err(|e| error(format!("{}: {e}", f.name)))
    }
}

/// The external package an exe asks Enigma Virtual Box to load, as a path
/// relative to the exe's folder (Erewhon's CHS exe names
/// `%DEFAULT FOLDER%\erewhon_chs.mys` just before its own table). The string
/// is not always NUL-terminated: in that exe it runs straight into the
/// `EVB\0` magic, so it also ends there or at the first control character.
pub fn external_package_name(exe: &[u8]) -> Option<String> {
    let needle: Vec<u8> = format!("{DEFAULT_FOLDER}\\")
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    let at = exe.windows(needle.len()).position(|w| w == needle.as_slice())?;
    let mut units = Vec::new();
    let mut i = at + needle.len();
    while let Some(u) = u16_at(exe, i) {
        if u < 0x20 || exe[i..].starts_with(MAGIC) {
            break;
        }
        units.push(u);
        i += 2;
        if units.len() > 260 {
            return None;
        }
    }
    let name = String::from_utf16(&units).ok()?;
    (!name.is_empty() && !name.contains(['/', ':'])).then(|| name.replace('\\', "/"))
}

fn parse_table(table: &[u8], table_start: u64) -> Result<Vec<EvbFile>> {
    let short = || error("file table truncated");
    // Pack header (64) and main node (size, 8 unknown bytes, object count).
    let main_size = u32_at(table, 64).ok_or_else(short)? as u64;
    let top_level = u32_at(table, 76).ok_or_else(short)? as usize;
    // The data of the first file starts where the main node says the table
    // ends; the nodes themselves start one byte before the main node's end.
    let mut data = table_start + 80 + main_size - 12;
    let mut at = 79usize;
    let mut files = Vec::new();
    // (folder path, children left) for each open folder.
    let mut folders: Vec<(String, usize)> = vec![(String::new(), top_level)];
    while let Some((_, left)) = folders.last() {
        if *left == 0 {
            folders.pop();
            continue;
        }
        if let Some(last) = folders.last_mut() {
            last.1 -= 1;
        }
        let objects = u32_at(table, at + 12).ok_or_else(short)? as usize;
        at += 16;
        let mut units = Vec::new();
        loop {
            let unit = u16_at(table, at).ok_or_else(short)?;
            at += 2;
            if unit == 0 {
                break;
            }
            units.push(unit);
        }
        let kind = *table.get(at).ok_or_else(short)?;
        at += 1;
        let name = String::from_utf16(&units).map_err(|_| error("node name is not UTF-16"))?;
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':']) {
            return Err(error("invalid node name"));
        }
        let parent = folders.last().map(|(path, _)| path.as_str()).unwrap_or("");
        match kind {
            NODE_FILE => {
                let size = u32_at(table, at + 2).ok_or_else(short)? as u64;
                let stored = u32_at(table, at + 49).ok_or_else(short)? as u64;
                at += 53;
                files.push(EvbFile {
                    name: join(parent, &name),
                    size,
                    offset: data,
                    stored,
                });
                data += stored;
                if files.len() > MAX_FILES {
                    return Err(error("too many files"));
                }
            }
            NODE_FOLDER => {
                at += 25;
                let path = if folders.len() == 1 && name == DEFAULT_FOLDER {
                    String::new()
                } else {
                    join(parent, &name)
                };
                if folders.len() > 64 {
                    return Err(error("folders nested too deep"));
                }
                folders.push((path, objects));
            }
            _ => return Err(error(format!("unknown node type {kind}"))),
        }
    }
    Ok(files)
}

fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

/// A compressed file: a chunk table (its byte size and 4 unknown bytes,
/// then 12 bytes per chunk whose first word is the chunk's packed size),
/// then the aPLib-packed chunks.
fn unpack_chunks(stored: &[u8], size: usize) -> std::result::Result<Vec<u8>, &'static str> {
    let table_size = u32_at(stored, 0).ok_or("chunk table truncated")? as usize;
    let table = stored.get(8..table_size).ok_or("chunk table truncated")?;
    let mut data = stored.get(table_size..).ok_or("chunk table truncated")?;
    let mut out = Vec::with_capacity(size);
    for entry in table.chunks(12) {
        let packed = u32_at(entry, 0).ok_or("chunk table truncated")? as usize;
        let chunk = data.get(..packed).ok_or("chunk beyond the file")?;
        data = &data[packed..];
        aplib_depack(chunk, &mut out, size)?;
    }
    if out.len() != size {
        return Err("unpacked size mismatch");
    }
    Ok(out)
}

struct Bits<'a> {
    input: &'a [u8],
    at: usize,
    tag: u8,
    left: u8,
}

impl Bits<'_> {
    fn byte(&mut self) -> Option<u8> {
        let byte = *self.input.get(self.at)?;
        self.at += 1;
        Some(byte)
    }
    fn bit(&mut self) -> Option<u32> {
        if self.left == 0 {
            self.tag = self.byte()?;
            self.left = 8;
        }
        self.left -= 1;
        let bit = (self.tag >> 7) as u32;
        self.tag <<= 1;
        Some(bit)
    }
    fn gamma(&mut self) -> Option<usize> {
        let mut value = 1usize;
        loop {
            value = value.checked_mul(2)? + self.bit()? as usize;
            if self.bit()? == 0 {
                return Some(value);
            }
        }
    }
}

/// Appends one aPLib-packed block (optionally with its "AP32" header) to
/// `out`, which may not grow past `limit`. Back-references reach into what
/// earlier blocks unpacked.
fn aplib_depack(
    packed: &[u8],
    out: &mut Vec<u8>,
    limit: usize,
) -> std::result::Result<(), &'static str> {
    let mut input = packed;
    if packed.starts_with(b"AP32") && packed.len() >= 24 {
        let header = u32_at(packed, 4).unwrap_or(0) as usize;
        let length = u32_at(packed, 8).unwrap_or(0) as usize;
        input = packed
            .get(header..header.saturating_add(length))
            .ok_or("aPLib header out of range")?;
    }
    let block_start = out.len();
    let mut bits = Bits {
        input,
        at: 0,
        tag: 0,
        left: 0,
    };
    let copy = |out: &mut Vec<u8>, offset: usize, length: usize| {
        // Each chunk is packed on its own.
        if offset == 0 || offset > out.len() - block_start {
            return Err("aPLib offset out of range");
        }
        if out.len() + length > limit {
            return Err("aPLib output larger than the file");
        }
        for _ in 0..length {
            out.push(out[out.len() - offset]);
        }
        Ok(())
    };
    let push = |out: &mut Vec<u8>, byte: u8| {
        if out.len() >= limit {
            return Err("aPLib output larger than the file");
        }
        out.push(byte);
        Ok(())
    };
    let Some(first) = bits.byte() else {
        return Ok(());
    };
    push(out, first)?;
    let mut last_offset = 0usize;
    let mut after_match = false;
    let truncated = "aPLib data truncated";
    loop {
        if bits.bit().ok_or(truncated)? == 0 {
            push(out, bits.byte().ok_or(truncated)?)?;
            after_match = false;
            continue;
        }
        if bits.bit().ok_or(truncated)? == 0 {
            let high = bits.gamma().ok_or(truncated)?;
            if !after_match && high == 2 {
                let length = bits.gamma().ok_or(truncated)?;
                copy(out, last_offset, length)?;
            } else {
                let high = high - if after_match { 2 } else { 3 };
                let offset = (high << 8) + bits.byte().ok_or(truncated)? as usize;
                let mut length = bits.gamma().ok_or(truncated)?;
                if offset >= 32000 {
                    length += 1;
                }
                if offset >= 1280 {
                    length += 1;
                }
                if offset < 128 {
                    length += 2;
                }
                copy(out, offset, length)?;
                last_offset = offset;
            }
            after_match = true;
        } else if bits.bit().ok_or(truncated)? == 0 {
            let byte = bits.byte().ok_or(truncated)? as usize;
            let (offset, length) = (byte >> 1, 2 + (byte & 1));
            if offset == 0 {
                return Ok(());
            }
            copy(out, offset, length)?;
            last_offset = offset;
            after_match = true;
        } else {
            let mut offset = 0usize;
            for _ in 0..4 {
                offset = (offset << 1) + bits.bit().ok_or(truncated)? as usize;
            }
            if offset == 0 {
                push(out, 0)?;
            } else {
                copy(out, offset, 1)?;
            }
            after_match = false;
        }
    }
}

/// Builds an external package holding `files` stored as is, for tests.
/// Paths may have one folder level ("pac/bn.ypf").
#[cfg(test)]
pub(crate) fn build_package(files: &[(&str, &[u8])]) -> Vec<u8> {
    fn node(name: &str, kind: u8, objects: u32) -> Vec<u8> {
        let mut bytes = vec![0; 12];
        bytes.extend(objects.to_le_bytes());
        for unit in name.encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        bytes.extend([0, 0, kind]);
        if kind == NODE_FOLDER {
            bytes.extend([0; 25]);
        }
        bytes
    }
    fn file_node(name: &str, size: u32) -> Vec<u8> {
        let mut bytes = node(name, NODE_FILE, 0);
        let mut extra = vec![0u8; 53];
        extra[2..6].copy_from_slice(&size.to_le_bytes());
        extra[49..53].copy_from_slice(&size.to_le_bytes());
        bytes.extend(extra);
        bytes
    }
    // Top-level entries in order of first appearance: a file, or a folder
    // with its files.
    let mut top: Vec<(&str, Vec<(&str, &[u8])>)> = Vec::new();
    for &(path, data) in files {
        match path.split_once('/') {
            Some((dir, name)) => match top.iter_mut().find(|(d, kids)| *d == dir && !kids.is_empty()) {
                Some((_, kids)) => kids.push((name, data)),
                None => top.push((dir, vec![(name, data)])),
            },
            None => top.push((path, Vec::new())),
        }
    }
    let mut nodes = node(DEFAULT_FOLDER, NODE_FOLDER, top.len() as u32);
    let mut data: Vec<u8> = Vec::new();
    for (name, kids) in &top {
        if kids.is_empty() {
            let bytes = files.iter().find(|(p, _)| p == name).unwrap().1;
            nodes.extend(file_node(name, bytes.len() as u32));
            data.extend(bytes);
        } else {
            nodes.extend(node(name, NODE_FOLDER, kids.len() as u32));
            for (kid, bytes) in kids {
                nodes.extend(file_node(kid, bytes.len() as u32));
                data.extend(*bytes);
            }
        }
    }
    let mut table = MAGIC.to_vec();
    table.resize(64, 0);
    // Main node: its size is counted from its own 4th byte (see parse_table).
    let main_size = (nodes.len() - 1 + 12) as u32;
    table.extend(main_size.to_le_bytes());
    table.extend([0; 8]);
    table.extend(1u32.to_le_bytes());
    table.extend(&nodes[1..]);
    table.extend(data);
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes aPLib bits most significant first, with the tag byte placed
    /// ahead of the literal bytes it covers, as the packer does.
    #[derive(Default)]
    struct Writer {
        out: Vec<u8>,
        tag_at: Option<usize>,
        used: u8,
    }
    impl Writer {
        fn bit(&mut self, bit: u8) {
            if self.tag_at.is_none() || self.used == 8 {
                self.tag_at = Some(self.out.len());
                self.out.push(0);
                self.used = 0;
            }
            let at = self.tag_at.unwrap();
            self.out[at] |= bit << (7 - self.used);
            self.used += 1;
        }
        fn gamma(&mut self, value: usize) {
            let bits = usize::BITS - value.leading_zeros();
            for index in (0..bits - 1).rev() {
                self.bit(((value >> index) & 1) as u8);
                self.bit(u8::from(index != 0));
            }
        }
    }

    #[test]
    fn aplib_literals_matches_and_end() {
        let mut w = Writer::default();
        w.out.push(b'a');
        // literal 'b'
        w.bit(0);
        w.out.push(b'b');
        // short match: offset 2, length 2 -> "ab"
        w.bit(1);
        w.bit(1);
        w.bit(0);
        w.out.push(2 << 1);
        // repeat last offset (gamma 2 right after a literal is required, so
        // put a literal first): literal 'c', then rep-match length 2 -> "bc"
        // would need offset 2 from "...abc": out is "ababc" -> "bc"
        w.bit(0);
        w.out.push(b'c');
        w.bit(1);
        w.bit(0);
        w.gamma(2);
        w.gamma(2);
        // 4-bit offset match of 1 byte: offset 1 -> 'c'
        w.bit(1);
        w.bit(1);
        w.bit(1);
        for bit in [0, 0, 0, 1] {
            w.bit(bit);
        }
        // a zero byte
        w.bit(1);
        w.bit(1);
        w.bit(1);
        for _ in 0..4 {
            w.bit(0);
        }
        // end marker
        w.bit(1);
        w.bit(1);
        w.bit(0);
        w.out.push(0);
        let mut out = Vec::new();
        aplib_depack(&w.out, &mut out, 64).unwrap();
        assert_eq!(out, b"ababcbcc\0");
    }

    #[test]
    fn rejects_offsets_before_the_output() {
        let mut out = Vec::new();
        // 'a', then a short match at offset 5.
        assert!(aplib_depack(&[b'a', 0b1100_0000, 5 << 1], &mut out, 64).is_err());
    }

    /// An image with a .enigma1 section holding `ipl._bp` and `Sub/x.txt`.
    fn packed_exe() -> Vec<u8> {
        let table = build_package(&[("ipl._bp", b"hello"), ("Sub/x.txt", b"hi")]);
        let section_at = 0x200usize;
        let mut exe = vec![0u8; section_at];
        exe[..2].copy_from_slice(b"MZ");
        exe[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        exe[0x80..0x84].copy_from_slice(b"PE\0\0");
        exe[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
        exe[0x94..0x96].copy_from_slice(&0u16.to_le_bytes());
        let section = 0x80 + 24;
        exe[section..section + 8].copy_from_slice(b".enigma1");
        exe[section + 16..section + 20].copy_from_slice(&(table.len() as u32).to_le_bytes());
        exe[section + 20..section + 24].copy_from_slice(&(section_at as u32).to_le_bytes());
        exe.extend(table);
        exe
    }

    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("evb-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn read_by_name(pkg: &EvbPackage, name: &str) -> Vec<u8> {
        let f = pkg.files().iter().find(|f| f.name == name).unwrap();
        pkg.read(f).unwrap()
    }

    #[test]
    fn lists_and_reads_packed_files() {
        let p = temp_file("packed.exe", &packed_exe());
        let pkg = EvbPackage::open(&p).unwrap().unwrap();
        let names: Vec<_> = pkg.files().iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["ipl._bp", "Sub/x.txt"]);
        assert_eq!(read_by_name(&pkg, "ipl._bp"), b"hello");
        assert_eq!(read_by_name(&pkg, "Sub/x.txt"), b"hi");
    }

    #[test]
    fn reads_external_packages() {
        // An external package is the table on its own, without an exe around it.
        let exe = packed_exe();
        let table = &exe[0x200..];
        let p = temp_file("patch.mys", table);
        let pkg = EvbPackage::open(&p).unwrap().unwrap();
        assert_eq!(read_by_name(&pkg, "Sub/x.txt"), b"hi");
    }

    #[test]
    fn finds_the_external_package_name() {
        let mut exe = b"MZ....".to_vec();
        for u in "\0 %DEFAULT FOLDER%\\patch\\chs.mys".encode_utf16() {
            exe.extend(u.to_le_bytes());
        }
        exe.extend([0, 0]);
        assert_eq!(external_package_name(&exe).as_deref(), Some("patch/chs.mys"));
        // Unterminated, followed by the table magic (Erewhon's CHS exe).
        let mut exe = Vec::new();
        for u in " %DEFAULT FOLDER%\\erewhon_chs.mys".encode_utf16() {
            exe.extend(u.to_le_bytes());
        }
        exe.extend(b"EVB\0@\0\0\0");
        assert_eq!(external_package_name(&exe).as_deref(), Some("erewhon_chs.mys"));
        assert_eq!(external_package_name(b"MZ no package"), None);
    }

    #[test]
    fn plain_files_are_not_packages() {
        let p = temp_file("plain.bin", b"PackFile    \0\0\0\0");
        assert!(EvbPackage::open(&p).unwrap().is_none());
    }
}
