//! 跨脚本上下文:多脚本执行宿主(P0)。
//!
//! 单脚本 [`GroupVm`](crate::GroupVm) 是引擎执行单元的核心;但真实游戏流程
//! 在 `yst%05d.ybn` 之间不断跳转(GOSUB 占语料 27065 组,最高频)。
//! 本模块提供「按脚本号取脚本上下文」的接口与实现,供 GroupVm 跨脚本切换。
//!
//! [`FsScriptHost`] 从游戏文件系统按 script_id 读 `ysbin\yst%05d.ybn`。

use std::collections::HashMap;
use std::sync::Arc;

use yuris_core::{Error, Result};
use yuris_format::vfs::GameFs;
use yuris_format::ystb::{CommandGroup, YstbFile};

/// 一个已解析脚本的可执行上下文(组表 + 各组窗口起始下标)。
#[derive(Debug, Clone)]
pub struct ScriptCtx {
    /// 脚本号(yst%05d)。
    pub script_id: u16,
    /// 已解析的 YSTB。
    pub script: YstbFile,
    /// 命令组表(part1 区)。
    pub groups: Vec<CommandGroup>,
    /// 每组第一个窗口在 slots 区的下标(引擎 obj+0x10 记录指针数组的等价)。
    pub first_slots: Vec<usize>,
}

impl ScriptCtx {
    /// 从脚本字节解析出上下文。组模型校验失败(非 v555 形态)→ `Err`。
    pub fn parse(script_id: u16, script: YstbFile) -> Result<Self> {
        let groups = script.groups()?;
        let first_slots = script.group_first_slots(&groups);
        Ok(Self {
            script_id,
            script,
            groups,
            first_slots,
        })
    }
}

/// 脚本加载抽象:按脚本号取 [`ScriptCtx`]。
///
/// 实现方负责读取并解析(如 [`FsScriptHost`]),可缓存。
pub trait ScriptHost {
    /// 取脚本号 `id` 的上下文(惰性加载 + 缓存由实现方决定)。
    fn load(&mut self, id: u16) -> Result<ScriptCtx>;

    /// 全部可用脚本号(启动链全量声明消费用;默认空)。
    fn script_ids(&mut self) -> Result<Vec<u16>> {
        Ok(Vec::new())
    }
}

/// 基于内存脚本表(测试/少量脚本用)的宿主。
///
/// 由外部预注入 `id → YstbFile`,按需 parse + 缓存。
#[derive(Debug, Clone, Default)]
pub struct InMemoryHost {
    scripts: HashMap<u16, YstbFile>,
}

impl InMemoryHost {
    /// 空宿主。
    pub fn new() -> Self {
        Self::default()
    }

    /// 注入一个脚本。
    pub fn insert(&mut self, id: u16, script: YstbFile) -> &mut Self {
        self.scripts.insert(id, script);
        self
    }
}

impl ScriptHost for InMemoryHost {
    fn load(&mut self, id: u16) -> Result<ScriptCtx> {
        let script = self
            .scripts
            .get(&id)
            .ok_or_else(|| Error::format(format!("script {id} not injected")))?
            .clone();
        ScriptCtx::parse(id, script)
    }
}


/// 基于游戏文件系统的宿主(真实游戏用):按 script_id 读
/// `ysbin\yst%05d.ybn`(封包或松散文件),解析结果缓存。
pub struct FsScriptHost {
    fs: Arc<GameFs>,
    /// 脚本区 XOR 密钥(4 字节循环,随游戏不同)。
    key: [u8; 4],
    cache: HashMap<u16, ScriptCtx>,
}

impl FsScriptHost {
    /// 用给定密钥建宿主。
    pub fn new(fs: Arc<GameFs>, key: [u8; 4]) -> Self {
        Self { fs, key, cache: HashMap::new() }
    }

    /// 脚本密钥。
    pub fn key(&self) -> [u8; 4] {
        self.key
    }

    /// 读文件系统内任一文件(启动链读 YSLB/YSVR 等)。
    pub fn read_entry(&self, path: &str) -> Result<Vec<u8>> {
        self.fs.read(path)
    }
}

/// 脚本路径。
pub fn script_path(id: u16) -> String {
    format!("ysbin\\yst{id:05}.ybn")
}

/// 列出文件系统里全部 `ysbin\yst%05d.ybn` 的脚本号(升序)。
pub fn list_script_ids(fs: &GameFs) -> Vec<u16> {
    let mut ids: Vec<u16> = Vec::new();
    let mut take = |name: &[u8]| {
        let n = yuris_format::ypf::normalize_name(name);
        let id = n
            .strip_prefix(b"ysbin\\yst")
            .and_then(|r| r.strip_suffix(b".ybn"))
            .filter(|d| d.len() == 5 && d.iter().all(u8::is_ascii_digit))
            .and_then(|d| std::str::from_utf8(d).ok()?.parse::<u16>().ok());
        if let Some(id) = id {
            ids.push(id);
        }
    };
    for pack in fs.packs() {
        for e in &pack.index().entries {
            take(&e.name);
        }
    }
    if let Ok(rd) = std::fs::read_dir(fs.root().join("ysbin")) {
        for e in rd.flatten() {
            take(format!("ysbin\\{}", e.file_name().to_string_lossy()).as_bytes());
        }
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// 推断脚本密钥:对前若干个脚本各自猜,取得票最多者。
pub fn guess_script_key(fs: &GameFs) -> Result<[u8; 4]> {
    let mut votes: HashMap<[u8; 4], usize> = HashMap::new();
    for id in list_script_ids(fs).into_iter().take(16) {
        let Ok(bytes) = fs.read(&script_path(id)) else { continue };
        if let Ok((k, _)) = yuris_format::ystb::guess_key(&bytes) {
            *votes.entry(k).or_default() += 1;
        }
    }
    votes
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(k, _)| k)
        .ok_or_else(|| Error::format("YSTB key not determinable (no ysbin\\yst*.ybn guessed)"))
}

impl ScriptHost for FsScriptHost {
    fn load(&mut self, id: u16) -> Result<ScriptCtx> {
        if let Some(ctx) = self.cache.get(&id) {
            return Ok(ctx.clone());
        }
        let bytes = self.fs.read(&script_path(id))?;
        let script = yuris_format::ystb::YstbFile::from_bytes(&bytes, self.key)?;
        let ctx = ScriptCtx::parse(id, script)?;
        self.cache.insert(id, ctx.clone());
        Ok(ctx)
    }

    fn script_ids(&mut self) -> Result<Vec<u16>> {
        Ok(list_script_ids(&self.fs))
    }
}
