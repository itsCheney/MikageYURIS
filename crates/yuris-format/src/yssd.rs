//! YSSD save data (`save\*.sd`) and SNP decoding.
//!
//! Read from the v555 sample's save folder (6 files, 28 blocks) and the
//! engine's LOAD handler; see `docs/formats/ysbin.md`.
//!
//! ```text
//! [header 0x1010]  magic "YSSD", version u32 (0x1E0), block count u32,
//!                  table length u32 (0x400), offsets 0x400 × u32
//!                  (offsets[i] = file offset of block i+1; 0 = empty)
//! [block]          u32 dno, u8 type (0 normal), u8 strict dims flag,
//!                  u16 var id, u32 packed len, u32 unpacked len,
//!                  SNP payload
//! [payload]        u32 type (1 INT, 2 FLT, 3 STR), u32 dims, u32[dims]
//!                  bounds, u32 data len, data (INT/FLT 8 bytes per
//!                  element; STR u32 len + bytes per element)
//! ```
//!
//! SNP (`YSSNP.DLL`) is raw Snappy: a varint with the unpacked length, then
//! literal and copy elements.

use yuris_core::{Error, Result};

/// YSSD magic：`YSSD`。
pub const YSSD_MAGIC: [u8; 4] = *b"YSSD";

/// 偏移表容量（样本恒 0x400）。
pub const OFFSET_TABLE_LEN: usize = 0x400;

/// 一个 YSSD 块（压缩形态）。
#[derive(Debug, Clone, PartialEq)]
pub struct YssdBlock {
    /// 块号（= DNO，1 基；脚本 LOAD 槽 2）。
    pub dno: u32,
    /// 块类型（0 = 普通单块；2 = 多子块 —— 语料未出现，不实现）。
    pub btype: u8,
    /// 严格维数标志（1 = 载荷维数必须与变量声明完全一致）。
    pub strict: u8,
    /// 目标变量 id（与脚本槽 3 引用一致；双重记录）。
    pub var_id: u16,
    /// SNP 压缩载荷。
    pub compressed: Vec<u8>,
}

/// 已解析的 YSSD 文件。
#[derive(Debug, Clone)]
pub struct YssdFile {
    /// Version (the engine's: 480 in the v555 sample, 481 in Erewhon).
    pub version: u32,
    /// 块表（按出现顺序；DNO → 块经 `block()` 查找）。
    blocks: Vec<YssdBlock>,
    /// DNO → blocks 下标（偏移表展开）。
    index: Vec<Option<u32>>,
}

impl YssdFile {
    /// 从字节解析（header + 偏移表 + 全部块头）。
    ///
    /// 块数据区整体读取（块间可能共享尾部容差 —— 逐块按压缩长切分）。
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < 0x10 {
            return Err(Error::format("YSSD too short"));
        }
        if &data[0..4] != &YSSD_MAGIC {
            return Err(Error::BadMagic {
                expected: YSSD_MAGIC,
                actual: data[0..4].try_into().unwrap(),
            });
        }
        // The engine's version: 480 in the v555 sample, 481 in Erewhon.
        let version = u32::from_le_bytes(data[4..8].try_into().unwrap());
        let table_end = 0x10 + OFFSET_TABLE_LEN * 4;
        if data.len() < table_end {
            return Err(Error::format("YSSD offset table truncated"));
        }
        let mut blocks = Vec::new();
        let mut index = vec![None::<u32>; OFFSET_TABLE_LEN];
        for (i, off) in data[0x10..table_end]
            .chunks_exact(4)
            .enumerate()
            .map(|(i, c)| (i, u32::from_le_bytes(c.try_into().unwrap())))
        {
            if off == 0 {
                continue;
            }
            let off = off as usize;
            if off + 0x10 > data.len() {
                return Err(Error::format(format!(
                    "YSSD block[{}] offset {off:#x} out of range",
                    i + 1
                )));
            }
            let dno = u32::from_le_bytes(data[off..off + 4].try_into().unwrap());
            let btype = data[off + 4];
            let strict = data[off + 5];
            let var_id = u16::from_le_bytes(data[off + 6..off + 8].try_into().unwrap());
            let clen = u32::from_le_bytes(data[off + 8..off + 12].try_into().unwrap()) as usize;
            if off + 0x10 + clen > data.len() {
                return Err(Error::format(format!(
                    "YSSD block[{}] payload truncated (need {clen}B)",
                    i + 1
                )));
            }
            index[i] = Some(blocks.len() as u32);
            blocks.push(YssdBlock {
                dno,
                btype,
                strict,
                var_id,
                compressed: data[off + 0x10..off + 0x10 + clen].to_vec(),
            });
        }
        Ok(Self { version, blocks, index })
    }

    /// 按 DNO（1 基）取块。
    pub fn block(&self, dno: u32) -> Option<&YssdBlock> {
        if dno == 0 || dno as usize > OFFSET_TABLE_LEN {
            return None;
        }
        self.index
            .get((dno - 1) as usize)
            .and_then(|s| *s)
            .and_then(|i| self.blocks.get(i as usize))
    }

    /// 块总数。
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// An empty file.
    pub fn new(version: u32) -> Self {
        Self { version, blocks: Vec::new(), index: vec![None; OFFSET_TABLE_LEN] }
    }

    /// Adds a block, replacing the one with the same DNO. DNOs outside
    /// 1..=1024 are ignored.
    pub fn set_block(&mut self, block: YssdBlock) {
        let Some(slot) = (block.dno as usize).checked_sub(1).filter(|&i| i < OFFSET_TABLE_LEN) else {
            return;
        };
        match self.index[slot] {
            Some(i) => self.blocks[i as usize] = block,
            None => {
                self.index[slot] = Some(self.blocks.len() as u32);
                self.blocks.push(block);
            }
        }
    }

    /// Writes the file: header, offset table, then the blocks in DNO order.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&YSSD_MAGIC);
        out.extend_from_slice(&self.version.to_le_bytes());
        out.extend_from_slice(&(self.blocks.len() as u32).to_le_bytes());
        out.extend_from_slice(&(OFFSET_TABLE_LEN as u32).to_le_bytes());
        out.resize(0x10 + OFFSET_TABLE_LEN * 4, 0);
        for (slot, i) in self.index.iter().enumerate() {
            let Some(i) = i else { continue };
            let b = &self.blocks[*i as usize];
            let at = out.len() as u32;
            out[0x10 + slot * 4..0x14 + slot * 4].copy_from_slice(&at.to_le_bytes());
            out.extend_from_slice(&b.dno.to_le_bytes());
            out.push(b.btype);
            out.push(b.strict);
            out.extend_from_slice(&b.var_id.to_le_bytes());
            out.extend_from_slice(&(b.compressed.len() as u32).to_le_bytes());
            out.extend_from_slice(&(snp_length(&b.compressed) as u32).to_le_bytes());
            out.extend_from_slice(&b.compressed);
        }
        out
    }
}

/// The unpacked length an SNP stream starts with (0 if it is malformed).
fn snp_length(src: &[u8]) -> usize {
    let mut n = 0usize;
    for (k, &b) in src.iter().take(5).enumerate() {
        n |= ((b & 0x7f) as usize) << (7 * k);
        if b < 0x80 {
            return n;
        }
    }
    0
}

/// SNP compression: standard Snappy, any decoder (ours and the engine's)
/// reads it. Greedy matching through a hash of 4-byte sequences, copies
/// within 64 KiB (Erewhon's 256x256 config arrays are mostly zeros).
pub fn snp_compress(data: &[u8]) -> Vec<u8> {
    const HASH_BITS: u32 = 14;
    let mut out = Vec::with_capacity(data.len() / 2 + 16);
    let mut n = data.len();
    loop {
        if n < 0x80 {
            out.push(n as u8);
            break;
        }
        out.push((n as u8 & 0x7f) | 0x80);
        n >>= 7;
    }
    let hash = |p: usize| {
        let v = u32::from_le_bytes(data[p..p + 4].try_into().unwrap());
        (v.wrapping_mul(0x1e35_a7bd) >> (32 - HASH_BITS)) as usize
    };
    let mut table = vec![usize::MAX; 1 << HASH_BITS];
    let mut lit_start = 0;
    let mut p = 0;
    while p + 4 <= data.len() {
        let h = hash(p);
        let cand = table[h];
        table[h] = p;
        if cand != usize::MAX && p - cand <= 0xffff && data[cand..cand + 4] == data[p..p + 4] {
            let mut len = 4;
            while p + len < data.len() && data[cand + len] == data[p + len] {
                len += 1;
            }
            snp_literal(&mut out, &data[lit_start..p]);
            let off = (p - cand) as u16;
            let mut left = len;
            while left > 0 {
                // A copy2 holds up to 64 bytes; keep the last one at 4+.
                let k = if left > 64 { if left - 64 < 4 { 60 } else { 64 } } else { left };
                out.push((((k - 1) as u8) << 2) | 2);
                out.extend_from_slice(&off.to_le_bytes());
                left -= k;
            }
            p += len;
            lit_start = p;
        } else {
            p += 1;
        }
    }
    snp_literal(&mut out, &data[lit_start..]);
    out
}

/// Appends literal runs for `lit`.
fn snp_literal(out: &mut Vec<u8>, lit: &[u8]) {
    for chunk in lit.chunks(0x10000) {
        let len = chunk.len() - 1;
        if len < 60 {
            out.push((len as u8) << 2);
        } else {
            // Tag 61: the length - 1 follows in 2 bytes.
            out.push(61 << 2);
            out.extend_from_slice(&(len as u16).to_le_bytes());
        }
        out.extend_from_slice(chunk);
    }
}

/// SNP（snappy 变体）解压。
///
/// 字面量长度 = `(tag>>2)+1`；copy1/2/4 与标准 snappy 一致
///（YSSNP.DLL `FUN_10001d40` + 派发表 `DAT_1000ba40` 实锤）。
pub fn snp_uncompress(src: &[u8]) -> Result<Vec<u8>> {
    // varint 未压长度（最多 5 字节）
    let mut expected: u64 = 0;
    let mut p = 0;
    let mut shift = 0;
    let mut done = false;
    while p < src.len() && shift < 35 {
        let b = src[p];
        p += 1;
        expected |= ((b & 0x7F) as u64) << shift;
        if b < 0x80 {
            done = true;
            break;
        }
        shift += 7;
    }
    if !done {
        return Err(Error::format("SNP varint header missing"));
    }
    let expected = expected as usize;
    let mut out = Vec::with_capacity(expected);
    while p < src.len() {
        let tag = src[p];
        match tag & 3 {
            0 => {
                // 字面量:长度 = (tag>>2)+1;60..63 → 1..4 额外长度字节(同样 +1)
                let mut n = (tag >> 2) as usize + 1;
                p += 1;
                if n > 0x3C {
                    let extra = n - 1 - 59;
                    if p + extra > src.len() {
                        return Err(Error::format("SNP literal length truncated"));
                    }
                    let mut v: usize = 0;
                    for (k, b) in src[p..p + extra].iter().enumerate() {
                        v |= (*b as usize) << (8 * k);
                    }
                    p += extra;
                    n = v + 1;
                }
                if p + n > src.len() {
                    return Err(Error::format("SNP literal truncated"));
                }
                out.extend_from_slice(&src[p..p + n]);
                p += n;
            }
            1 => {
                // copy1:len = 4 + ((tag>>2)&7);offset = ((tag>>5)<<8)|下一字节
                if p + 2 > src.len() {
                    return Err(Error::format("SNP copy1 truncated"));
                }
                let n = 4 + ((tag >> 2) & 7) as usize;
                let off = (((tag >> 5) as usize) << 8) | src[p + 1] as usize;
                p += 2;
                copy_back(&mut out, off, n)?;
            }
            2 => {
                // copy2:len = (tag>>2)+1;offset = u16 LE
                if p + 3 > src.len() {
                    return Err(Error::format("SNP copy2 truncated"));
                }
                let n = (tag >> 2) as usize + 1;
                let off = u16::from_le_bytes([src[p + 1], src[p + 2]]) as usize;
                p += 3;
                copy_back(&mut out, off, n)?;
            }
            _ => {
                // copy4:len = (tag>>2)+1;offset = u32 LE
                if p + 5 > src.len() {
                    return Err(Error::format("SNP copy4 truncated"));
                }
                let n = (tag >> 2) as usize + 1;
                let off = u32::from_le_bytes(src[p + 1..p + 5].try_into().unwrap()) as usize;
                p += 5;
                copy_back(&mut out, off, n)?;
            }
        }
    }
    if out.len() != expected {
        return Err(Error::format(format!(
            "SNP length mismatch: got {}, header says {expected}",
            out.len()
        )));
    }
    Ok(out)
}

/// 回引用拷贝（offset 相对 out 末尾；逐字节 —— 引擎语义允许重叠）。
fn copy_back(out: &mut Vec<u8>, off: usize, n: usize) -> Result<()> {
    if off == 0 || off > out.len() {
        return Err(Error::format(format!(
            "SNP copy offset {off} out of range (out len {})",
            out.len()
        )));
    }
    for _ in 0..n {
        let b = out[out.len() - off];
        out.push(b);
    }
    Ok(())
}

/// 解压后的 YSSD 载荷（`FUN_0044564d` 消费形态）。
#[derive(Debug, Clone, PartialEq)]
pub struct YssdPayload {
    /// 类型：1=INT 2=FLT 3=STR（须与目标变量描述符类型一致）。
    pub ty: u8,
    /// 维数（0 = 标量）。
    pub dims: Vec<u32>,
    /// 数据（INT/FLT = 8B/元素；STR = 逐元素 {u32 长度 + 字节}）。
    pub data: Vec<u8>,
}

impl YssdPayload {
    /// 从解压后的字节解析。
    pub fn from_bytes(raw: &[u8]) -> Result<Self> {
        if raw.len() < 12 {
            return Err(Error::format("YSSD payload too short"));
        }
        let ty = u32::from_le_bytes(raw[0..4].try_into().unwrap()) as u8;
        let dim = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
        if dim > 8 {
            return Err(Error::format(format!(
                "YSSD payload dim count {dim} > 8 (引擎 900000 同族)"
            )));
        }
        if raw.len() < 8 + dim * 4 + 4 {
            return Err(Error::format("YSSD payload dims truncated"));
        }
        let dims = raw[8..8 + dim * 4]
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let dlen = u32::from_le_bytes(
            raw[8 + dim * 4..12 + dim * 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let data_start = 12 + dim * 4;
        if raw.len() < data_start + dlen {
            return Err(Error::format("YSSD payload data truncated"));
        }
        Ok(Self {
            ty,
            dims,
            data: raw[data_start..data_start + dlen].to_vec(),
        })
    }

    /// The unpacked payload bytes (the inverse of [`Self::from_bytes`]).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + self.dims.len() * 4 + self.data.len());
        out.extend_from_slice(&u32::from(self.ty).to_le_bytes());
        out.extend_from_slice(&(self.dims.len() as u32).to_le_bytes());
        for d in &self.dims {
            out.extend_from_slice(&d.to_le_bytes());
        }
        out.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.data);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_read() {
        let payload = YssdPayload { ty: 1, dims: vec![3], data: (0..24).collect() };
        let raw = payload.to_bytes();
        let big: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let zeros = vec![0u8; 524_288];
        let mut noise = Vec::new();
        let mut x = 1u32;
        for _ in 0..100_000 {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            noise.push((x >> 16) as u8 & 7);
        }
        for data in [raw.clone(), big.clone(), Vec::new(), zeros.clone(), noise, vec![1, 2, 3]] {
            assert_eq!(snp_uncompress(&snp_compress(&data)).unwrap(), data);
        }
        assert!(snp_compress(&zeros).len() < 30_000);
        let mut f = YssdFile::new(481);
        for dno in [5, 2, 5] {
            f.set_block(YssdBlock { dno, btype: 0, strict: 1, var_id: 7, compressed: snp_compress(&raw) });
        }
        let back = YssdFile::from_bytes(&f.to_bytes()).unwrap();
        assert_eq!(back.version, 481);
        assert_eq!(back.block_count(), 2);
        let b = back.block(5).unwrap();
        assert_eq!(YssdPayload::from_bytes(&snp_uncompress(&b.compressed).unwrap()).unwrap(), payload);
        assert!(back.block(1).is_none());
    }
}
