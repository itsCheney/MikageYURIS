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
    /// Read-only version/profile evidence; never selects runtime behavior.
    pub detection: yuris_format::detection::DetectionReport,
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

/// YSVR files (length, FNV-1a 64) of games whose system variables were
/// checked against the 494 dictionary's semantics by playing them:
/// SkyChord, and Erewhon (the same ysv.ybn in the Japanese release and the
/// Chinese patch; the input, text and path IDs were first worked out on
/// it).
const VERIFIED_494_YSVR: &[(usize, u64)] = &[(47166, 0xd472c3293659379d), (43871, 0xae9b27ee25b9d6fb)];

/// Picks the system-variable IDs from the YSVR evidence, strongest first:
///
/// 1. a verified fingerprint with the unique exact 494 shape (SkyChord);
/// 2. the one dictionary family whose complete shape matches exactly;
/// 3. the one family whose shapes match at every ID the runtime uses
///    (commercial builds add or change other system variables);
/// 4. otherwise Unknown, which disables system reads and writes.
///
/// 2 and 3 are shape evidence only: YSVR has no names. They are still far
/// more likely right than Unknown, under which no game gets input or text.
fn select_system_var_map(detection: &yuris_format::detection::DetectionReport, known_fingerprint: bool)
    -> (yuris_core::SystemVarMap, &'static str) {
    use yuris_core::{SystemVarMap, system_vars::DICTIONARY_IDS};
    if detection.ysvr_fingerprint.is_none() { return (SystemVarMap::Unknown, "ysvr_missing_or_unreadable"); }
    if detection.ysvr_layouts.iter().filter(|c| c.table.is_some()).count() != 1 {
        return (SystemVarMap::Unknown, "ysvr_layout_unresolved");
    }
    let family = |c: &yuris_format::detection::Candidate| DICTIONARY_IDS.iter().find(|(n, _)| *n == c.dictionaries);
    let exact: Vec<_> = detection.candidates.iter().filter(|c| c.exact_shape()).filter_map(family).collect();
    if let [(name, _)] = exact[..] {
        if *name == "494" && known_fingerprint { return (SystemVarMap::SkyChord, "verified_fingerprint_and_shape"); }
        return (SystemVarMap::Dictionary(name), "exact_shape_unverified");
    }
    let anchored: Vec<_> = detection.candidates.iter()
        .filter_map(|c| family(c).filter(|(_, ids)| ids.anchors().iter()
            .all(|id| !c.missing.contains(id) && !c.conflicting.contains(id))))
        .collect();
    match anchored[..] {
        [(name, _)] => (SystemVarMap::Dictionary(name), "anchor_shape_unverified"),
        [] => (SystemVarMap::Unknown, "system_shape_unrecognized"),
        _ => (SystemVarMap::Unknown, "system_shape_ambiguous"),
    }
}

impl Bootstrap {
    /// Runs the boot chain and returns a ready [`GroupVm`].
    ///
    /// - Reads YSLB, YSVR, YSCM and the scripts from the file system.
    /// - The default entry label is `SYSTEM_START`.
    /// - The file system also answers the VM's file queries (FILEINFO,
    ///   CGINFO, LOAD).
    pub fn boot(self) -> Result<Booted> {
        self.boot_impl(None)
    }

    /// Explicit compatibility override for an independently verified game.
    /// Logged separately from detection; does not change reported evidence.
    pub fn boot_with_system_var_map(self, map: yuris_core::SystemVarMap) -> Result<Booted> {
        self.boot_impl(Some(map))
    }

    fn boot_impl(self, override_map: Option<yuris_core::SystemVarMap>) -> Result<Booted> {
        let detection = yuris_format::detection::DetectionReport::inspect(&self.fs);
        detection.log();
        let known_fingerprint = detection.ysvr_fingerprint.is_some_and(|f| VERIFIED_494_YSVR.contains(&f));
        let (detected, reason) = select_system_var_map(&detection, known_fingerprint);
        let known_shape = detection.candidates.iter().filter(|c| c.exact_shape()).count() == 1;
        if matches!(detected, yuris_core::SystemVarMap::Dictionary(_)) {
            tracing::warn!(target: "yuris_version", "runtime_map {detected:?} chosen by {reason}: not verified on this game; register its YSVR fingerprint once it plays correctly");
        }
        let map = override_map.unwrap_or(detected);
        tracing::info!(target: "yuris_version", "runtime_map detected={detected:?} selected={map:?} reason={reason} override={override_map:?} fingerprint_match={known_fingerprint} shape_match={known_shape}");
        let mut stage = "start";
        let result = self.boot_detected(detection, map, &mut stage);
        match &result {
            Ok(b) => tracing::info!(target: "yuris_boot", "boot_success script={} pc={} applied_ysvr={}", b.script_id, b.entry_pc, b.applied_ysvr),
            Err(e) => tracing::error!(target: "yuris_boot", "boot_failed stage={stage} error={e}"),
        }
        result
    }

    fn boot_detected(self, detection: yuris_format::detection::DetectionReport,
        map: yuris_core::SystemVarMap, stage: &mut &'static str) -> Result<Booted> {
        *stage = "script_key";
        let key = match self.key {
            Some(k) => k,
            None => guess_script_key(&self.fs)?,
        };
        let mut host = FsScriptHost::new(self.fs.clone(), key);
        *stage = "label_table";
        let yslb_data = host.read_entry("ysbin\\ysl.ybn")?;
        let yslb = YslbTable::from_bytes(&yslb_data)?;

        // Entry label
        let entry: &[u8] = self.entry_label.as_deref().unwrap_or(b"SYSTEM_START");
        *stage = "entry_label";
        let entry_idx = yslb.find(entry).ok_or_else(|| {
            Error::format(format!("entry label not found: {:?}", String::from_utf8_lossy(entry)))
        })?;
        let label = &yslb.labels()[entry_idx];
        let (entry_pc, script_id) = (label.target_pc, label.script_id);

        // Read YSVR before the host moves into the VM.
        *stage = "variable_table";
        let ysvr_data = host.read_entry("ysbin\\ysv.ybn")?;
        let ysvr = yuris_format::ysvr::YsvrTable::from_bytes(&ysvr_data)?;

        *stage = "entry_script";
        let ctx = host.load(script_id)?;

        // Scripts index commands in this game's ysc.ybn order; map them to the
        // canonical order by name.
        *stage = "command_table";
        let cmd_map = Arc::new(match host.read_entry("ysbin\\ysc.ybn") {
            Ok(b) => CommandMap::from_yscm(&YscmTable::from_bytes(&b)?),
            Err(_) => CommandMap::identity(),
        });

        // VM with the entry script loaded
        *stage = "vm_initialization";
        let mut vm = GroupVm::load(ctx.script)?;
        vm.set_system_var_map(map);
        // A virtual display matching the game canvas is the portable default.
        // These are device capabilities, not iOS surface/letterbox dimensions.
        let display = self.fs.read("ysbin\\yscfg.ybn").ok()
            .and_then(|b| yuris_format::yscf::YscfFile::from_bytes(&b).ok())
            .and_then(|c| yuris_script::eval::DisplayMetrics::new(c.screen_width, c.screen_height, 32))
            .unwrap_or_else(|| yuris_script::eval::DisplayMetrics::new(1280, 720, 32).unwrap());
        vm.set_display_metrics(display);
        vm.set_command_map(cmd_map.clone());
        vm.set_script_id(script_id);

        // Labels: name -> (target_pc, script_id)
        let mut labels: HashMap<Vec<u8>, (u32, u16)> = HashMap::new();
        for l in yslb.labels() {
            labels.insert(l.name.clone(), (l.target_pc, l.script_id));
        }
        vm.set_labels(labels);

        *stage = "script_declarations";
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
        if let Ok(b) = self.fs.read("ysbin\\yst_list.ybn") {
            vm.set_script_list(Arc::new(yuris_format::ystl::YstlTable::from_bytes(&b)?));
        }

        // YSVR initial values, after the declarations. Kinds 1 and 3 apply
        // here; kind 2 applies when its script first loads.
        *stage = "variable_initializers";
        let applied_ysvr = vm.apply_ysvr(&ysvr)?;
        vm.apply_ysvr_for_script(script_id)?;

        // $1045 (the save folder string) stays the empty YSVR value: the
        // engine finds save/*.sd through its VFS, not through $1045.

        // The engine fetches then increments, so start at the entry group.
        vm.set_pc(entry_pc as usize);

        Ok(Booted {
            detection,
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

#[cfg(test)]
mod map_selection_tests {
    use super::select_system_var_map;
    use yuris_core::SystemVarMap;
    use yuris_format::detection::{compare_system_shapes, profile_table, DetectionReport};
    use yuris_format::ysvr::{LayoutEvidence, YsvrLayout, YsvrTable};

    fn report(t: YsvrTable) -> DetectionReport {
        DetectionReport {
            candidates: compare_system_shapes(&t),
            ysvr_layouts: vec![LayoutEvidence { layout: YsvrLayout::Category, table: Some(t), error: None }],
            ysvr_fingerprint: Some((1, 1)),
            ..Default::default()
        }
    }

    #[test]
    fn every_family_selects_its_own_ids() {
        for (name, ids) in yuris_core::system_vars::DICTIONARY_IDS {
            let (map, reason) = select_system_var_map(&report(profile_table(name).unwrap()), false);
            assert_eq!(map, SystemVarMap::Dictionary(name), "{name}");
            assert_eq!(reason, "exact_shape_unverified");
            assert_eq!(map.ids(), Some(*ids));
        }
        let (map, _) = select_system_var_map(&report(profile_table("494").unwrap()), true);
        assert_eq!(map, SystemVarMap::SkyChord);
    }

    #[test]
    fn changes_away_from_used_ids_keep_the_family() {
        let mut t = profile_table("480").unwrap();
        // A commercial build: one unused variable retyped, one extra.
        t.entries[300].ty = 3;
        let mut extra = t.entries[0].clone();
        extra.var_id = 600;
        t.entries.push(extra);
        let (map, reason) = select_system_var_map(&report(t.clone()), false);
        assert_eq!((map, reason), (SystemVarMap::Dictionary("480"), "anchor_shape_unverified"));
        // A used one (_INP) changed: no family left.
        t.entries[134].bounds = vec![512];
        assert_eq!(select_system_var_map(&report(t), false).0, SystemVarMap::Unknown);
    }

    #[test]
    fn unrelated_tables_stay_unknown() {
        let mut t = profile_table("494").unwrap();
        t.entries.truncate(20);
        assert_eq!(select_system_var_map(&report(t), true), (SystemVarMap::Unknown, "system_shape_unrecognized"));
        let mut r = report(profile_table("494").unwrap());
        r.ysvr_fingerprint = None;
        assert_eq!(select_system_var_map(&r, false).0, SystemVarMap::Unknown);
    }
}
