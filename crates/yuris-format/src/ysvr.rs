//! YSVR variable table (`ysbin\ysv.ybn`).
//!
//! Variables have no names at run time, only ids; YSVR declares them with
//! their bounds and initial values. See `docs/formats/ysbin.md`.
//!
//! ```text
//! [header 10]  magic "YSVR", version u32, count u16
//! [entries]    count × {
//!     u8  kind      1 global, 2 script-local (applied when the script loads), 3 other
//!     u8  category  save category
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
    /// 从**已解压**的 YSVR 字节解析。
    ///
    /// 逐条消费 `count` 个条目；类型 ∉ {1,2,3} 时按引擎行为不消费初值段
    /// （不报错、不猜测）。
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
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

        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let kind = r.u8()?;
            let category = r.u8()?;
            let script = r.u16_le()?;
            let var_id = r.u16_le()?;
            let ty = r.u8()?;
            let dims = r.u8()? as usize;

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
            let _ = i;
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
