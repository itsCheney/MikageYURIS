//! YSVR variable table (`ysbin\ysv.ybn`).
//!
//! Variables have no names at run time, only ids; YSVR declares them with
//! their bounds and initial values. See `docs/formats/ysbin.md`.
//!
//! ```text
//! [header 10]  magic "YSVR", version u32, count u16
//! [entries]    count × {
//!     u8  kind      1 global, 2 script-local (applied when the script loads), 3 other
//!     u8  category  save category (absent in the legacy layout)
//!     u16 script    owning script
//!     u16 var_id
//!     u8  ty        1 INT, 2 FLT, 3 STR, 0 declared only
//!     u8  dims
//!     u32[dims]     bounds
//!     init: ty 1 = i64, ty 2 = f64, ty 3 = u16 len + bytes, otherwise none
//! }
//! ```

use yuris_core::{Error, Reader, Result};

/// YSVR magic：`YSVR`。
pub const YSVR_MAGIC: [u8; 4] = *b"YSVR";

/// header 长度（magic 4 + version 4 + count 2）。
pub const HEADER_LEN: usize = 10;

/// Physical entry layout; independent of the file's reported version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YsvrLayout {
    /// Seven-byte entry prefix, without a category byte.
    Legacy,
    /// Eight-byte entry prefix, including a category byte.
    Category,
}

/// Structural result for one candidate, including why it was rejected.
#[derive(Debug, Clone)]
pub struct LayoutEvidence {
    /// Candidate layout.
    pub layout: YsvrLayout,
    /// Present only when the entire file validated under this candidate.
    pub table: Option<YsvrTable>,
    /// Parsing/validation error for a rejected candidate.
    pub error: Option<String>,
}

/// 变量初值。
#[derive(Debug, Clone, PartialEq)]
pub enum YsvrInit {
    /// INT（i64）。
    Int(i64),
    /// FLT（f64，按位保存以保持 Eq）。
    Float(f64),
    /// STR（SJIS 字节，原样）。
    Str(Vec<u8>),
    /// 仅声明，无初值（ty == 0）。
    None,
}

/// 一条变量定义。
#[derive(Debug, Clone, PartialEq)]
pub struct YsvrEntry {
    /// 匹配类别：1=全局初值 2=按脚本初值 3=其他（语义 Unknown，不猜）。
    pub kind: u8,
    /// 描述符 byte0（存档过滤类别）。
    pub category: u8,
    /// 脚本号（kind==2 的匹配键）。
    pub script: u16,
    /// 变量 id。
    pub var_id: u16,
    /// 类型：1=INT 2=FLT 3=STR 0=仅声明。
    pub ty: u8,
    /// 各维边界（`dims` 个）。
    pub bounds: Vec<u32>,
    /// 初值。
    pub init: YsvrInit,
}

/// 已解析的 YSVR。
#[derive(Debug, Clone)]
pub struct YsvrTable {
    /// 引擎版本（样本 555）。
    pub version: u32,
    /// 变量定义条目。
    pub entries: Vec<YsvrEntry>,
    /// 条目流结束位置（样本 == 文件长度，精确闭合）。
    pub consumed: usize,
}

impl YsvrTable {
    /// Selects the sole structurally valid layout, never by a version threshold.
    /// Ambiguous files (including empty tables) require an explicit layout.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let evidence = Self::inspect_layouts(data);
        let mut valid = evidence.iter().filter_map(|c| c.table.as_ref());
        let first = valid.next();
        if valid.next().is_some() {
            return Err(Error::format("ambiguous YSVR layouts: Legacy and Category; explicit layout required"));
        }
        first.cloned().ok_or_else(|| Error::format(format!("no valid YSVR layout: {evidence:?}")))
    }

    /// Tests both candidates and retains rejection reasons for diagnostics.
    pub fn inspect_layouts(data: &[u8]) -> Vec<LayoutEvidence> {
        [YsvrLayout::Legacy, YsvrLayout::Category].into_iter().map(|layout| {
            match Self::from_bytes_with_layout(data, layout) {
                Ok(table) => LayoutEvidence { layout, table: Some(table), error: None },
                Err(e) => LayoutEvidence { layout, table: None, error: Some(e.to_string()) },
            }
        }).collect()
    }

    /// Parses an explicit layout with the same structural checks as detection.
    /// Legacy categories are derived as SYS=0 for IDs <1000, DEF=1 otherwise,
    /// following shimamura-sakura/yuri (183b85e), ysvr.py readV000.
    pub fn from_bytes_with_layout(data: &[u8], layout: YsvrLayout) -> Result<Self> {
        let mut r = Reader::new(data);
        let magic = r.u32_bytes()?;
        if magic != YSVR_MAGIC {
            return Err(Error::BadMagic {
                expected: YSVR_MAGIC,
                actual: magic,
            });
        }
        let version = r.u32_le()?;
        let count = r.u16_le()? as usize;

        let prefix_len = if layout == YsvrLayout::Legacy { 7 } else { 8 };
        if count > r.remaining() / prefix_len {
            return Err(Error::format("YSVR count exceeds available entry prefixes"));
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let kind = r.u8()?;
            let category = if layout == YsvrLayout::Category { r.u8()? } else { 0 };
            let script = r.u16_le()?;
            let var_id = r.u16_le()?;
            let ty = r.u8()?;
            let dims = r.u8()? as usize;
            let category = if layout == YsvrLayout::Legacy { u8::from(var_id >= 1000) } else { category };
            if !(1..=3).contains(&kind) || ty > 3 || category > 3 || (var_id < 1000 && category != 0) {
                return Err(Error::format(format!("YSVR entry {i}: invalid scope/type/category (kind={kind}, id={var_id}, type={ty}, category={category})")));
            }
            if dims > r.remaining() / 4 {
                return Err(Error::format(format!("YSVR entry {i}: truncated dimensions")));
            }

            let mut bounds = Vec::with_capacity(dims);
            for _ in 0..dims {
                bounds.push(r.u32_le()?);
            }

            let init = match ty {
                1 => YsvrInit::Int(i64::from_le_bytes(r.bytes(8)?.try_into().unwrap())),
                2 => YsvrInit::Float(f64::from_le_bytes(r.bytes(8)?.try_into().unwrap())),
                3 => {
                    let len = r.u16_le()? as usize;
                    YsvrInit::Str(r.bytes(len)?.to_vec())
                }
                _ => YsvrInit::None,
            };
            entries.push(YsvrEntry {
                kind,
                category,
                script,
                var_id,
                ty,
                bounds,
                init,
            });
        }

        if r.remaining() != 0 {
            return Err(Error::format(format!("YSVR trailing bytes: {}", r.remaining())));
        }
        Ok(Self {
            version,
            entries,
            consumed: r.pos(),
        })
    }

    /// 版本。
    pub fn version(&self) -> u32 {
        self.version
    }

    /// 全部条目。
    pub fn entries(&self) -> &[YsvrEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(layout: YsvrLayout, version: u32) -> Vec<u8> {
        let mut b = b"YSVR".to_vec();
        b.extend(version.to_le_bytes());
        b.extend(4u16.to_le_bytes());
        for (kind, id, ty, dims) in [
            (1u8, 0u16, 1u8, vec![2u32, 8]),
            (2, 1200, 2, vec![]),
            (3, 1300, 3, vec![4]),
            (1, 1400, 0, vec![0]),
        ] {
            b.push(kind);
            if layout == YsvrLayout::Category { b.push(u8::from(id >= 1000)); }
            b.extend(7u16.to_le_bytes());
            b.extend(id.to_le_bytes());
            b.extend([ty, dims.len() as u8]);
            for d in dims { b.extend(d.to_le_bytes()); }
            match ty {
                1 => b.extend((-123i64).to_le_bytes()),
                2 => b.extend(1.25f64.to_le_bytes()),
                3 => { b.extend(3u16.to_le_bytes()); b.extend([0x81, 0x40, 0xff]); }
                _ => (),
            }
        }
        b
    }

    #[test]
    fn layout_is_independent_of_reported_version() {
        for version in [450, 480, 481, 490, 554, 555] {
            for layout in [YsvrLayout::Legacy, YsvrLayout::Category] {
                let b = fixture(layout, version);
                let evidence = YsvrTable::inspect_layouts(&b);
                let valid: Vec<_> = evidence.iter().filter(|e| e.table.is_some()).collect();
                assert_eq!(valid.len(), 1);
                assert_eq!(valid[0].layout, layout);
                let table = YsvrTable::from_bytes(&b).unwrap();
                assert_eq!(table.version, version);
                assert_eq!(table.consumed, b.len());
                assert_eq!(table.entries[0].category, 0);
                assert_eq!(table.entries[1].category, 1);
                assert_eq!(table.entries[0].bounds, [2, 8]);
                assert_eq!(table.entries[0].init, YsvrInit::Int(-123));
                assert_eq!(table.entries[1].init, YsvrInit::Float(1.25));
                assert_eq!(table.entries[2].init, YsvrInit::Str(vec![0x81, 0x40, 0xff]));
                assert_eq!(table.entries[3].init, YsvrInit::None);
                assert_eq!(table.entries[3].bounds, [0]);
            }
        }
    }

    #[test]
    fn rejects_truncation_trailing_bytes_and_invalid_fields() {
        for layout in [YsvrLayout::Legacy, YsvrLayout::Category] {
            let b = fixture(layout, 555);
            for end in 0..b.len() {
                assert!(YsvrTable::from_bytes_with_layout(&b[..end], layout).is_err(), "accepted prefix {end}");
            }
            let mut trailing = b.clone();
            trailing.push(0);
            assert!(YsvrTable::from_bytes_with_layout(&trailing, layout).is_err());
            let shift = usize::from(layout == YsvrLayout::Category);
            for (offset, value) in [(10, 4), (15 + shift, 4), (16 + shift, 255)] {
                let mut bad = b.clone();
                bad[offset] = value;
                assert!(YsvrTable::from_bytes_with_layout(&bad, layout).is_err());
            }
            let mut bad_count = b.clone();
            bad_count[8..10].copy_from_slice(&u16::MAX.to_le_bytes());
            assert!(YsvrTable::from_bytes(&bad_count).is_err());
        }
        let mut bad = fixture(YsvrLayout::Category, 555);
        bad[11] = 1; // a system variable cannot carry a user save category
        assert!(YsvrTable::from_bytes_with_layout(&bad, YsvrLayout::Category).is_err());
    }

    #[test]
    fn ambiguous_empty_table_requires_explicit_layout() {
        let mut b = b"YSVR".to_vec();
        b.extend(555u32.to_le_bytes());
        b.extend(0u16.to_le_bytes());
        assert_eq!(YsvrTable::inspect_layouts(&b).iter().filter(|c| c.table.is_some()).count(), 2);
        assert!(YsvrTable::from_bytes(&b).unwrap_err().to_string().contains("ambiguous"));
        for layout in [YsvrLayout::Legacy, YsvrLayout::Category] {
            assert!(YsvrTable::from_bytes_with_layout(&b, layout).unwrap().entries.is_empty());
        }
    }
}
