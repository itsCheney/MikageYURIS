//! Boot chain.
//!
//! The engine boots (FUN_0046b63c): YSCM command table -> variable
//! descriptors -> load YSVR -> apply initial values -> look up the entry
//! label by hash -> create the task -> load the script -> run.
//!
//! This module ties the parts together: [`GameFs`] (game files),
//! [`YslbTable`] (entry label lookup), [`FsScriptHost`] (script loading) and
//! [`GroupVm`] (execution). The caller may name the entry label; by default
//! it is `SYSTEM_START` from YSLB.

use std::collections::HashMap;
use std::sync::Arc;

use yuris_core::{Error, Result};
use yuris_format::vfs::GameFs;
use yuris_format::yscm::YscmTable;
use yuris_format::yslb::YslbTable;

use crate::commands::CommandMap;
use crate::GroupVm;
use yuris_value::Value;
use crate::host::{guess_script_key, ScriptHost, FsScriptHost};

/// Boot options.
pub struct Bootstrap {
    /// The game's file system.
    pub fs: Arc<GameFs>,
    /// Script XOR key; None guesses it from the scripts ([`guess_script_key`]).
    pub key: Option<[u8; 4]>,
    /// Entry label (`SYSTEM_START` from YSLB by default).
    pub entry_label: Option<Vec<u8>>,
}

/// Boot result: an initialized VM that has not run yet.
pub struct Booted {
    /// The VM, with host, labels and entry script set.
    pub vm: GroupVm,
    /// Entry script id.
    pub script_id: u16,
    /// Entry group index.
    pub entry_pc: u32,
    /// Number of YSVR initial values applied.
    pub applied_ysvr: usize,
    /// The script key in use.
    pub key: [u8; 4],
}

impl Bootstrap {
    /// Runs the boot chain and returns a ready [`GroupVm`].
    ///
    /// - Reads YSLB, YSVR, YSCM and the scripts from the file system.
    /// - The default entry label is `SYSTEM_START`.
    /// - The file system also answers the VM's file queries (FILEINFO,
    ///   CGINFO, LOAD).
    pub fn boot(self) -> Result<Booted> {
        let key = match self.key {
            Some(k) => k,
            None => guess_script_key(&self.fs)?,
        };
        let mut host = FsScriptHost::new(self.fs.clone(), key);
        let yslb_data = host.read_entry("ysbin\\ysl.ybn")?;
        let yslb = YslbTable::from_bytes(&yslb_data)?;

        // Entry label
        let entry: &[u8] = self.entry_label.as_deref().unwrap_or(b"SYSTEM_START");
        let entry_idx = yslb.find(entry).ok_or_else(|| {
            Error::format(format!("entry label not found: {:?}", String::from_utf8_lossy(entry)))
        })?;
        let label = &yslb.labels()[entry_idx];
        let (entry_pc, script_id) = (label.target_pc, label.script_id);

        // Read YSVR before the host moves into the VM.
        let ysvr_data = host.read_entry("ysbin\\ysv.ybn")?;
        let ysvr = yuris_format::ysvr::YsvrTable::from_bytes(&ysvr_data)?;

        let ctx = host.load(script_id)?;

        // Scripts index commands in this game's ysc.ybn order; map them to the
        // canonical order by name.
        let cmd_map = Arc::new(match host.read_entry("ysbin\\ysc.ybn") {
            Ok(b) => CommandMap::from_yscm(&YscmTable::from_bytes(&b)?),
            Err(_) => CommandMap::identity(),
        });

        // VM with the entry script loaded
        let mut vm = GroupVm::load(ctx.script)?;
        vm.set_command_map(cmd_map.clone());
        vm.set_script_id(script_id);

        // Labels: name -> (target_pc, script_id)
        let mut labels: HashMap<Vec<u8>, (u32, u16)> = HashMap::new();
        for l in yslb.labels() {
            labels.insert(l.name.clone(), (l.target_pc, l.script_id));
        }
        vm.set_labels(labels);

        // Consume the declarations (INT/FLT/STR) of every script up front:
        // the engine's variable descriptors are global, and without this the
        // boot chain reads undefined variables (@1417, @6292). Declarations
        // come before the YSVR initial values. Each script is marked as
        // consumed so switch_script does not redo it: redoing it would
        // rebuild arrays with their declared bounds and overwrite the YSVR
        // state (s47 $2729 bounds [1] reset to [2] shifts a button table).
        {
            let mut n = 0usize;
            for sid in host.script_ids()? {
                let mut ctx = host.load(sid)?;
                cmd_map.remap(&mut ctx);
                n += crate::consume_declarations_into(vm.store_mut(), &ctx)?;
                vm.mark_declarations_consumed(sid);
            }
            let _ = n;
        }

        // The host serves cross-script calls from here on.
        vm.set_host(Box::new(host));
        vm.set_file_probe(self.fs.clone());

        // YSVR initial values, after the declarations. Kinds 1 and 3 apply
        // here; kind 2 applies when its script first loads.
        let applied_ysvr = vm.apply_ysvr(&ysvr)?;
        vm.apply_ysvr_for_script(script_id)?;

        // $1045 (the save folder string) stays the empty YSVR value: the
        // engine finds save/*.sd through its VFS, not through $1045.

        // The engine fetches then increments, so start at the entry group.
        vm.set_pc(entry_pc as usize);

        Ok(Booted {
            vm,
            script_id,
            entry_pc,
            applied_ysvr,
            key,
        })
    }
}

impl GroupVm {
    /// 事件流推进到首个 Wait/Complete。供启动链驱动。
    pub fn set_pc(&mut self, pc: usize) {
        self.pc = pc;
    }

    /// P9.2 输入注入(播放器):按键名写入引擎输入串 `$55[1]`。
    ///
    /// `$55` 属帧系统变量族(成果 52:系统族无全局回退)——键链
    /// (s190 `STR $6409 = $55[1]`)在**其执行帧**内读取,注入时刻的
    /// 当前帧未必是同一帧(实测踩坑)。广播写入:全局 + 全部活动帧,
    /// 保证任一层级的键链本帧都能读到。
    pub fn inject_input_key(&mut self, key: &[u8]) -> Result<()> {
        let r = yuris_value::VarRef {
            space: yuris_value::VarSpace::Dollar,
            id: 55,
        };
        let v = Value::Str(key.to_vec());
        for f in &mut self.frames {
            if f.locals.has_array(&r) {
                let _ = f.locals.set_elem(&r, &[1], v.clone());
            }
        }
        self.store.set_elem(&r, &[1], v)
    }

    /// 诊断:读注入串(顶帧优先,与求值读路径对称)。
    pub fn input_key_snapshot(&self) -> String {
        let r = yuris_value::VarRef {
            space: yuris_value::VarSpace::Dollar,
            id: 55,
        };
        if let Some(f) = self.frames.last() {
            if let Ok(yuris_value::Value::Str(bs)) = f.locals.get_elem(&r, &[1]) {
                return String::from_utf8_lossy(bs).into_owned();
            }
        }
        match self.store.get_elem(&r, &[1]) {
            Ok(yuris_value::Value::Str(bs)) => String::from_utf8_lossy(bs).into_owned(),
            _ => "<undef>".into(),
        }
    }

    /// 清除输入注入(全帧 + 全局;见 [`Self::inject_input_key`])。
    pub fn clear_input_key(&mut self) -> Result<()> {
        let r = yuris_value::VarRef {
            space: yuris_value::VarSpace::Dollar,
            id: 55,
        };
        let v = Value::Str(Vec::new());
        for f in &mut self.frames {
            if f.locals.has_array(&r) {
                let _ = f.locals.set_elem(&r, &[1], v.clone());
            }
        }
        self.store.set_elem(&r, &[1], v)
    }
}
