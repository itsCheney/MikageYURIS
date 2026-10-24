//! YSTL script list (`ysbin\yst_list.ybn`).
//!
//! One entry per script number: the source file it was compiled from and
//! per-script counts. SYSTEMINFO answers SCRIPTNUM, SCRIPTFILE and TEXTNUM
//! from it.
//!
//! ```text
//! u8  magic[4] = "YSTL"
//! u32 version
//! u32 count
//! entry × count
//!   u32 id             // script number, = the entry's position
//!   u32 source_len
//!   u8  source[source_len]  // CP932, e.g. "data\script\eris\button\global_f.yst"
//!   u64 timestamp      // source file time
//!   i32 variables      // -1 for a source that produced no script
//!   i32 labels
//!   i32 texts
//! ```
//!
//! Checked on Erewhon (v481, 426 entries): the text counts add up to the
//! total in `ysbin\yst.ybn` (YSTD), and the 8 entries without a compiled
//! `yst%05d.ybn` have -1 counts.

use yuris_core::{Error, Reader, Result};

/// YSTL magic, `YSTL`.
pub const YSTL_MAGIC: [u8; 4] = *b"YSTL";

/// One script of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YstlEntry {
    /// Script number.
    pub id: u32,
    /// Source path as stored (CP932 bytes, `\` separated).
    pub source: Vec<u8>,
    /// Source file time, as stored.
    pub timestamp: u64,
    /// Variable count (-1 when the source produced no script).
    pub variables: i32,
    /// Label count.
    pub labels: i32,
    /// Text count (the read-tracking tables are sized by it).
    pub texts: i32,
}

/// A parsed YSTL.
#[derive(Debug, Clone)]
pub struct YstlTable {
    /// Version.
    pub version: u32,
    /// Entries, in file order.
    pub entries: Vec<YstlEntry>,
}

impl YstlTable {
    /// Parses `yst_list.ybn`.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let magic = r.u32_bytes()?;
        if magic != YSTL_MAGIC {
            return Err(Error::BadMagic {
                expected: YSTL_MAGIC,
                actual: magic,
            });
        }
        let version = r.u32_le()?;
        let count = r.u32_le()? as usize;
        if count > r.remaining() / 28 {
            return Err(Error::format(format!("YSTL count {count} too large")));
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let id = r.u32_le()?;
            let len = r.u32_le()? as usize;
            let source = r.bytes(len)?.to_vec();
            let lo = r.u32_le()? as u64;
            let hi = r.u32_le()? as u64;
            entries.push(YstlEntry {
                id,
                source,
                timestamp: lo | (hi << 32),
                variables: r.u32_le()? as i32,
                labels: r.u32_le()? as i32,
                texts: r.u32_le()? as i32,
            });
        }
        if r.remaining() != 0 {
            return Err(Error::format(format!("YSTL: {} trailing bytes", r.remaining())));
        }
        Ok(Self { version, entries })
    }

    /// The entry of a script number.
    pub fn get(&self, id: u32) -> Option<&YstlEntry> {
        match self.entries.get(id as usize) {
            Some(e) if e.id == id => Some(e),
            _ => self.entries.iter().find(|e| e.id == id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(entries: &[(&[u8], i32, i32, i32)]) -> Vec<u8> {
        let mut v = b"YSTL".to_vec();
        v.extend(481u32.to_le_bytes());
        v.extend((entries.len() as u32).to_le_bytes());
        for (i, (src, vars, labels, texts)) in entries.iter().enumerate() {
            v.extend((i as u32).to_le_bytes());
            v.extend((src.len() as u32).to_le_bytes());
            v.extend(*src);
            v.extend(0x01d2_bdba_9118_7ac8u64.to_le_bytes());
            for n in [vars, labels, texts] {
                v.extend(n.to_le_bytes());
            }
        }
        v
    }

    #[test]
    fn parses_entries() {
        let bytes = build(&[
            (b"data\\script\\eris\\button\\global_f.yst", 199, 0, 0),
            (b"data\\script\\userscript\\start.txt", -1, -1, -1),
            (b"data\\script\\userscript\\a.txt", 0, 16, 37),
        ]);
        let t = YstlTable::from_bytes(&bytes).unwrap();
        assert_eq!(t.version, 481);
        assert_eq!(t.entries.len(), 3);
        assert_eq!(t.get(0).unwrap().variables, 199);
        assert_eq!(t.get(1).unwrap().texts, -1);
        assert_eq!(t.get(2).unwrap().source, b"data\\script\\userscript\\a.txt");
        assert!(YstlTable::from_bytes(&bytes[..bytes.len() - 1]).is_err());
    }
}
