//! CG / CGINFO / CGEND through the VM and [`yuris_host::Host`] (moved
//! from yuris-vm's P7.1 tests when the CG state left the VM).
//!
//! - CG with a FILE creates the layer; without FILE, or `FILE=""`, an
//!   unknown layer is not created (kemonomichi2 0x423864 returns 0).
//! - CGINFO SX/SY answer the loaded image's size (engine watch oracle:
//!   `cgsys\dummy.png` 1×1 → 1, the 1350×200 message window → 1350).
//!   These tests have no file system, so images load as 0×0.
//! - A layer that does not exist answers 0 to every query.
//! - CGEND removes the layer.

#[path = "../../yuris-vm/tests/common/mod.rs"]
mod common;

use common::{make_ystb, SAMPLE_KEY};
use yuris_format::ystb::YstbFile;
use yuris_value::{Value, VarRef, VarSpace};
use yuris_host::Host;
use yuris_vm::{GroupVm, VmSuspend, cmd};

fn var(id: u16) -> VarRef {
    VarRef {
        space: VarSpace::At,
        id,
    }
}

/// 字面量字符串窗口(带引号定界,与语料 0x4d 形态一致)。
fn w_str(s: &str) -> Vec<u8> {
    let mut v = vec![0x4d];
    let inner: Vec<u8> = format!("\"{s}\"").into_bytes();
    v.extend_from_slice(&(inner.len() as u16).to_le_bytes());
    v.extend_from_slice(&inner);
    v
}

fn w_push1() -> Vec<u8> {
    vec![0x42, 1, 0, 1]
}

fn w_var(id: u16) -> Vec<u8> {
    let mut v = vec![0x48, 3, 0, 0x40];
    v.extend_from_slice(&id.to_le_bytes());
    v
}

struct W {
    slot: u8,
    bytes: Vec<u8>,
}

fn build(groups: &[(u8, u8, u16)], wins: &[W]) -> GroupVm {
    let key = SAMPLE_KEY;
    let windows: Vec<([u8; 4], u32, u32, Vec<u8>)> = wins
        .iter()
        .map(|w| ([w.slot, 0, 0, 0], w.bytes.len() as u32, 0, w.bytes.clone()))
        .collect();
    let refs: Vec<([u8; 4], u32, u32, &[u8])> = windows
        .iter()
        .map(|(t, l, o, b)| (*t, *l, *o, b.as_slice()))
        .collect();
    let bytes = make_ystb(key, groups, &refs);
    let script = YstbFile::from_bytes(&bytes, key).unwrap();
    let mut vm = GroupVm::load(script).unwrap();
    vm.set_native(Box::new(Host::new(None, (640, 480))));
    vm
}

/// CG with a FILE creates the layer; with no file system its image is 0×0,
/// so SX, SY and the COLOR at (0, 0) answer 0, and EXIST answers 1.
#[test]
fn cginfo_sx_sy_color_defaults() {
    let vm = build(
        &[
            (cmd::CG, 3, 0),
            (cmd::CGINFO, 3, 0),
            (cmd::CGINFO, 3, 0),
            (cmd::CGINFO, 5, 0),
            (cmd::CGINFO, 3, 0),
            (cmd::RETURN, 0, 0),
        ],
        &[
            W { slot: 0, bytes: w_str("btnA") },   // CG ID
            W { slot: 46, bytes: w_str("btn.png") }, // CG FILE(非空)
            W { slot: 4, bytes: vec![0x42, 1, 0, 10] }, // X=10
            W { slot: 0, bytes: w_str("btnA") },   // CGINFO ID
            W { slot: 13, bytes: w_push1() },      // SX 查询
            W { slot: 33, bytes: w_var(9000) },    // LET → @9000
            W { slot: 0, bytes: w_str("btnA") },
            W { slot: 14, bytes: w_push1() },      // SY 查询
            W { slot: 33, bytes: w_var(9001) },
            W { slot: 0, bytes: w_str("btnA") },
            W { slot: 24, bytes: w_push1() },      // COLOR 查询
            W { slot: 34, bytes: vec![0x42, 1, 0, 0] }, // SET=0
            W { slot: 35, bytes: vec![0x42, 1, 0, 0] }, // SET2=0
            W { slot: 33, bytes: w_var(9002) },
            W { slot: 0, bytes: w_str("btnA") },
            W { slot: 2, bytes: w_push1() }, // EXIST
            W { slot: 33, bytes: w_var(9003) },
        ],
    );
    let mut vm = vm;
    let s = vm.run(100).unwrap();
    assert!(matches!(s, VmSuspend::Complete), "{s:?}");
    assert_eq!(
        vm.store().get(&var(9000)).unwrap(),
        &Value::Int(0),
        "SX = image width (0×0 without files)"
    );
    assert_eq!(
        vm.store().get(&var(9001)).unwrap(),
        &Value::Int(0),
        "SY = image height"
    );
    assert_eq!(vm.store().get(&var(9002)).unwrap(), &Value::Int(0), "no pixel at (0, 0)");
    assert_eq!(vm.store().get(&var(9003)).unwrap(), &Value::Int(1), "EXIST");
}

/// CG without FILE, or with `FILE=""`, does not create → CGINFO = 0.
#[test]
fn cginfo_no_file_not_created() {
    // CG(btnB, X) —— 无 FILE 槽
    let mut vm = build(
        &[(cmd::CG, 2, 0), (cmd::CGINFO, 3, 0), (cmd::RETURN, 0, 0)],
        &[
            W { slot: 0, bytes: w_str("btnB") },
            W { slot: 4, bytes: vec![0x42, 1, 0, 10] },
            W { slot: 0, bytes: w_str("btnB") },
            W { slot: 13, bytes: w_push1() },
            W { slot: 33, bytes: w_var(9010) },
        ],
    );
    let s = vm.run(100).unwrap();
    assert!(matches!(s, VmSuspend::Complete), "{s:?}");
    assert_eq!(
        vm.store().get(&var(9010)).unwrap(),
        &Value::Int(0),
        "无 FILE 未注册 → SX=0"
    );

    // CG(btnC, FILE="") —— 空 FILE(s9 g1114 形态:4d 02 00 22 22)
    let mut vm = build(
        &[(cmd::CG, 2, 0), (cmd::CGINFO, 3, 0), (cmd::RETURN, 0, 0)],
        &[
            W { slot: 0, bytes: w_str("btnC") },
            W { slot: 46, bytes: w_str("") },
            W { slot: 0, bytes: w_str("btnC") },
            W { slot: 13, bytes: w_push1() },
            W { slot: 33, bytes: w_var(9011) },
        ],
    );
    let s = vm.run(100).unwrap();
    assert!(matches!(s, VmSuspend::Complete), "{s:?}");
    assert_eq!(
        vm.store().get(&var(9011)).unwrap(),
        &Value::Int(0),
        "FILE=\"\" 未注册 → SX=0"
    );
}

/// CGEND removes the layer → CGINFO = 0.
#[test]
fn cgend_unregisters() {
    let mut vm = build(
        &[
            (cmd::CG, 2, 0),
            (cmd::CGEND, 1, 0),
            (cmd::CGINFO, 3, 0),
            (cmd::RETURN, 0, 0),
        ],
        &[
            W { slot: 0, bytes: w_str("btnD") },
            W { slot: 46, bytes: w_str("d.png") },
            W { slot: 0, bytes: w_str("btnD") },
            W { slot: 0, bytes: w_str("btnD") },
            W { slot: 13, bytes: w_push1() },
            W { slot: 33, bytes: w_var(9020) },
        ],
    );
    let s = vm.run(100).unwrap();
    assert!(matches!(s, VmSuspend::Complete), "{s:?}");
    assert_eq!(
        vm.store().get(&var(9020)).unwrap(),
        &Value::Int(0),
        "CGEND 后未注册 → SX=0"
    );
}

/// X/Y set by CG are answered by CGINFO.
#[test]
fn cginfo_x_y_roundtrip() {
    let mut vm = build(
        &[
            (cmd::CG, 4, 0),
            (cmd::CGINFO, 3, 0),
            (cmd::CGINFO, 3, 0),
            (cmd::RETURN, 0, 0),
        ],
        &[
            W { slot: 0, bytes: w_str("btnE") },
            W { slot: 46, bytes: w_str("e.png") },
            W { slot: 4, bytes: vec![0x42, 1, 0, 100] }, // X=100
            W { slot: 5, bytes: vec![0x42, 1, 0, 90] }, // Y=90
            W { slot: 0, bytes: w_str("btnE") },
            W { slot: 5, bytes: w_push1() }, // X 查询(CGINFO 槽5)
            W { slot: 33, bytes: w_var(9030) },
            W { slot: 0, bytes: w_str("btnE") },
            W { slot: 6, bytes: w_push1() }, // Y 查询(CGINFO 槽6)
            W { slot: 33, bytes: w_var(9031) },
        ],
    );
    let s = vm.run(100).unwrap();
    assert!(matches!(s, VmSuspend::Complete), "{s:?}");
    assert_eq!(vm.store().get(&var(9030)).unwrap(), &Value::Int(100));
    assert_eq!(vm.store().get(&var(9031)).unwrap(), &Value::Int(90));
}
