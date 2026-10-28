//! Read-only version evidence. Never selects runtime semantics from a header.
use crate::{vfs::GameFs, ysvr::YsvrTable, system_profiles::PROFILES};

/// A header and its actual VFS origin, including absent/unreadable files.
#[derive(Debug, Clone)]
pub struct FileEvidence {
    /// Logical path requested from the VFS.
    pub path: String,
    /// Archive/member or loose file actually selected by VFS priority.
    pub source: Option<String>,
    /// Raw header value; not an inferred layout or compatibility claim.
    pub version: Option<u32>,
    /// Read or header validation failure.
    pub error: Option<String>,
}
/// Shape evidence for a dictionary family, not a claim of semantic compatibility.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// Public SDK dictionary filenames sharing this shape.
    pub dictionaries: &'static str,
    /// Number of entries with the expected type and dimensions.
    pub matched: usize,
    /// Expected IDs absent from the table.
    pub missing: Vec<u16>,
    /// IDs with incompatible shapes, scopes, or duplicate definitions.
    pub conflicting: Vec<u16>,
    /// Additional active IDs in the system-variable range.
    pub unexpected: Vec<u16>,
}
impl Candidate {
    /// Whether the complete known shape matches without extra active IDs.
    pub fn exact_shape(&self) -> bool {
        self.missing.is_empty() && self.conflicting.is_empty() && self.unexpected.is_empty()
    }
}
/// Diagnostic report. No profile from this report is applied to the VM.
#[derive(Debug, Clone, Default)]
pub struct DetectionReport {
    /// Per-file evidence after patch resolution.
    pub files: Vec<FileEvidence>,
    /// Shape comparisons, only when exactly one YSVR layout validates.
    pub candidates: Vec<Candidate>,
    /// Structural YSVR candidates, including parse failures and ambiguity.
    pub ysvr_layouts: Vec<crate::ysvr::LayoutEvidence>,
    /// Decompressed YSVR byte count and FNV-1a-64 compatibility fingerprint.
    pub ysvr_fingerprint: Option<(usize, u64)>,
    /// Conflicts and limitations that prevent semantic conclusions.
    pub warnings: Vec<String>,
}

/// Compare complete ordered system-variable shapes independently of header versions.
pub fn compare_system_shapes(table: &YsvrTable) -> Vec<Candidate> {
    PROFILES.iter().map(|&(name, shape)| {
        let rows: Vec<_> = shape.split(';').collect();
        let mut c = Candidate { dictionaries: name, matched: 0, missing: vec![], conflicting: vec![], unexpected: vec![] };
        for (id, row) in rows.iter().enumerate() {
            let (ty, dims) = row.split_once(':').unwrap_or((row, ""));
            let ty: u8 = ty.parse().expect("generated type");
            let dims: Vec<u32> = dims.split(',').filter(|s| !s.is_empty()).map(|s| s.parse().expect("generated dimension")).collect();
            let entries: Vec<_> = table.entries.iter().filter(|e| e.var_id == id as u16).collect();
            if entries.is_empty() { c.missing.push(id as u16); }
            else if entries.len() == 1 && entries[0].kind == 1 && entries[0].ty == ty && entries[0].bounds == dims { c.matched += 1; }
            else { c.conflicting.push(id as u16); }
        }
        c.unexpected = table.entries.iter().filter(|e| e.var_id < 1000 && e.var_id as usize >= rows.len() && e.ty != 0).map(|e| e.var_id).collect();
        c
    }).collect()
}

/// A synthetic YSVR holding exactly a dictionary family's system variables
/// (declarations only), for tests and tools.
pub fn profile_table(dictionaries: &str) -> Option<YsvrTable> {
    let shape = PROFILES.iter().find(|(n, _)| *n == dictionaries)?.1;
    let entries = shape.split(';').enumerate().map(|(id, row)| {
        let (ty, dims) = row.split_once(':').unwrap_or((row, ""));
        crate::ysvr::YsvrEntry { kind: 1, category: 0, script: 0, var_id: id as u16, ty: ty.parse().expect("generated type"),
            bounds: dims.split(',').filter(|s| !s.is_empty()).map(|s| s.parse().expect("generated dimension")).collect(),
            init: crate::ysvr::YsvrInit::None }
    }).collect();
    Some(YsvrTable { version: 555, entries, consumed: 0 })
}

impl DetectionReport {
    /// Examines resolved files. Parse failures become evidence, not boot errors.
    pub fn inspect(fs: &GameFs) -> Self {
        let mut report = Self::default();
        let mut paths: Vec<String> = ["ysbin/yscfg.ybn", "ysbin/ysv.ybn", "ysbin/ysc.ybn", "ysbin/ysl.ybn"].iter().map(|s| s.to_string()).collect();
        paths.extend(fs.file_names().into_iter().filter(|n| n.starts_with(b"ysbin\\yst") && n.ends_with(b".ybn") && n.get(9).is_some_and(u8::is_ascii_digit)).map(|n| String::from_utf8_lossy(&n).into_owned()));
        paths.sort(); paths.dedup();
        for path in paths {
            let mut e = FileEvidence { source: fs.source_of(&path), path: path.clone(), version: None, error: None };
            match fs.read(&path) {
                Ok(b) => {
                    let magic: &[u8] = if path.ends_with("yscfg.ybn") { b"YSCF" } else if path.ends_with("ysv.ybn") { b"YSVR" } else if path.ends_with("ysc.ybn") { b"YSCM" } else if path.ends_with("ysl.ybn") { b"YSLB" } else { b"YSTB" };
                    if b.len() >= 8 && &b[..4] == magic { e.version = Some(u32::from_le_bytes(b[4..8].try_into().unwrap())); }
                    else { e.error = Some("missing/invalid header".into()); }
                    if magic == b"YSVR" {
                        report.ysvr_fingerprint = Some((b.len(), b.iter().fold(0xcbf29ce484222325u64,
                            |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100000001b3))));
                        report.ysvr_layouts = YsvrTable::inspect_layouts(&b);
                        let valid: Vec<_> = report.ysvr_layouts.iter().filter_map(|c| c.table.as_ref()).collect();
                        match valid.as_slice() {
                            [table] => report.candidates = compare_system_shapes(table),
                            [] => report.warnings.push("no structurally valid YSVR layout; shape matching withheld".into()),
                            _ => report.warnings.push("ambiguous YSVR layouts; explicit layout required; shape matching withheld".into()),
                        }
                    }
                }
                Err(err) => e.error = Some(err.to_string()),
            }
            report.files.push(e);
        }
        let versions: std::collections::BTreeSet<_> = report.files.iter().filter_map(|f| f.version).collect();
        if versions.len() > 1 { report.warnings.push(format!("mixed reported versions {versions:?}; not automatically incompatible")); }
        let matches = report.candidates.iter().filter(|c| c.exact_shape()).count();
        report.warnings.push(match matches {
            0 => "system-variable profile unknown; shape detection does not select runtime semantics",
            1 => "unique shape match only; semantic compatibility unconfirmed; shape detection does not select runtime semantics",
            _ => "ambiguous system-variable shapes; shape detection does not select runtime semantics",
        }.into());
        report
    }
    /// Sends bounded summaries to the existing desktop/mobile tracing pipeline.
    pub fn log(&self) {
        let mut counts = std::collections::BTreeMap::new();
        for f in &self.files {
            if let Some(v) = f.version { *counts.entry(v).or_insert(0usize) += 1; }
            if !f.path.replace('\\', "/").starts_with("ysbin/yst") || f.error.is_some() { tracing::info!(target: "yuris_version", "file={} source={:?} reported={:?} error={:?}", f.path, f.source, f.version, f.error); }
        }
        tracing::info!(target: "yuris_version", "reported version counts={counts:?}");
        let valid: Vec<_> = self.ysvr_layouts.iter().filter(|c| c.table.is_some()).map(|c| c.layout).collect();
        let status = match valid.len() { 0 => "unreadable", 1 => "selected", _ => "ambiguous" };
        tracing::info!(target: "yuris_version", "YSVR layout_status={status} candidates={valid:?} fingerprint={:?}", self.ysvr_fingerprint);
        for c in &self.ysvr_layouts { tracing::info!(target: "yuris_version", "YSVR layout={:?} valid={} error={:?}", c.layout, c.table.is_some(), c.error); }
        for c in &self.candidates { tracing::info!(target: "yuris_version", "dictionary={} matched={} missing={} conflicts={} extra={} exact_shape={}", c.dictionaries, c.matched, c.missing.len(), c.conflicting.len(), c.unexpected.len(), c.exact_shape()); }
        for c in &self.candidates {
            // Bound each line for mobile exporters; preserve every conflicting ID.
            for (kind, ids) in [("missing", &c.missing), ("conflicting", &c.conflicting), ("unexpected", &c.unexpected)] {
                for chunk in ids.chunks(32) {
                    tracing::info!(target: "yuris_version", "dictionary={} {kind}_ids={chunk:?}", c.dictionaries);
                }
            }
        }
        for w in &self.warnings { tracing::warn!(target: "yuris_version", "{w}"); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn table(profile: usize) -> YsvrTable {
        profile_table(PROFILES[profile].0).unwrap()
    }
    #[test]
    fn all_profiles_match_independently_of_reported_version() {
        for i in 0..PROFILES.len() {
            let t = table(i);
            let c = compare_system_shapes(&t);
            assert!(c[i].exact_shape());
            assert_eq!(c.iter().filter(|c| c.exact_shape()).count(), 1);
        }
        let old = table(4); // 480 dictionary, despite the header reporting 555
        assert_eq!(old.entries[134].bounds, [1024]);
        let new = table(6);
        assert_eq!(new.entries[144].bounds, [1024]);
    }
    #[test]
    fn one_input_anchor_is_not_a_profile_and_duplicates_conflict() {
        let mut t = table(6);
        t.entries.retain(|e| e.var_id == 144);
        assert!(compare_system_shapes(&t).iter().all(|c| !c.exact_shape()));
        let mut t = table(6);
        t.entries.push(t.entries[144].clone());
        assert!(compare_system_shapes(&t)[6].conflicting.contains(&144));
    }
    #[test]
    fn patch_origin_and_header_come_from_same_vfs_resolution() {
        use crate::ypf::{build, NameHash};
        let dir = std::env::temp_dir().join(format!("yuris-detection-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("pac")).unwrap();
        for (name, version) in [("pac/ysbin.ypf", 480u32), ("update2.ypf", 555)] {
            let mut b = b"YSCF".to_vec(); b.extend(version.to_le_bytes());
            let files: [(&[u8], u8, bool, &[u8]); 1] = [(b"ysbin\\yscfg.ybn", 0, false, &b)];
            std::fs::write(dir.join(name), build(500, 0xc9, true, NameHash::Murmur2, &files)).unwrap();
        }
        let fs = GameFs::open(&dir).unwrap();
        let report = DetectionReport::inspect(&fs);
        let config = report.files.iter().find(|f| f.path.ends_with("yscfg.ybn")).unwrap();
        assert_eq!(config.version, Some(555));
        assert!(config.source.as_ref().unwrap().starts_with("update2.ypf:"));
        assert!(report.files.iter().any(|f| f.path.ends_with("ysv.ybn") && f.error.is_some()));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
