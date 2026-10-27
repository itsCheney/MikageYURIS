//! Boot chain: infer the archive name key and script key from a synthetic
//! game folder (pac/bn.ypf) and run to the end of the script.

mod common;

use std::sync::Arc;

use common::make_ystb;
use yuris_core::murmur2;
use yuris_format::vfs::GameFs;
use yuris_format::ypf::{build, NameHash};
use yuris_vm::boot::Bootstrap;
use yuris_vm::{cmd, VmSuspend};

const KEY: [u8; 4] = [0x11, 0x22, 0x33, 0x44];

fn yslb(labels: &[(&[u8], u32, u16)]) -> Vec<u8> {
    let mut v = b"YSLB".to_vec();
    v.extend_from_slice(&555u32.to_le_bytes());
    v.extend_from_slice(&(labels.len() as u32).to_le_bytes());
    v.extend_from_slice(&[0u8; 256 * 4]);
    for (name, pc, sid) in labels {
        v.push(name.len() as u8);
        v.extend_from_slice(name);
        v.extend_from_slice(&murmur2(name).to_le_bytes());
        v.extend_from_slice(&pc.to_le_bytes());
        v.extend_from_slice(&sid.to_le_bytes());
        v.extend_from_slice(&[0, 0]);
    }
    v
}

fn ysvr_empty() -> Vec<u8> {
    let mut v = b"YSVR".to_vec();
    v.extend_from_slice(&555u32.to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes());
    v
}

#[test]
fn boots_synthetic_game_with_inferred_keys() {
    let dir = std::env::temp_dir().join(format!("yuris-boot-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("pac")).unwrap();

    // RETURN (empty frame stack = script end) with a return value window
    // (push 1) for key guessing
    let w_ret: &[u8] = &[0x42, 1, 0, 1];
    let script = make_ystb(KEY, &[(cmd::RETURN, 1, 0)], &[([0, 0, 0, 0], w_ret.len() as u32, 0, w_ret)]);
    let ysl = yslb(&[(b"SYSTEM_START", 0, 0)]);
    let ysv = ysvr_empty();
    let files: [(&[u8], u8, bool, &[u8]); 3] = [
        (b"ysbin\\yst00000.ybn", 0, true, &script),
        (b"ysbin\\ysl.ybn", 0, true, &ysl),
        (b"ysbin\\ysv.ybn", 0, true, &ysv),
    ];
    std::fs::write(dir.join("pac/bn.ypf"), build(500, 0x5A, true, NameHash::Murmur2, &files)).unwrap();

    let fs = Arc::new(GameFs::open(&dir).unwrap());
    assert_eq!(fs.packs()[0].index().name_key, 0x5A);
    let booted = Bootstrap { fs, key: None, entry_label: None }.boot().unwrap();
    assert_eq!(booted.key, KEY);
    assert_eq!(booted.script_id, 0);
    let mut vm = booted.vm;
    assert_eq!(vm.run(100).unwrap(), VmSuspend::Complete);
    assert_eq!(vm.global_int(115), Some(1280)); // absent config fallback
    assert_eq!(vm.global_int(116), Some(720));
    std::fs::create_dir_all(dir.join("ysbin")).unwrap();
    for (w, h) in [(800u32, 600u32), (1280, 720), (1920, 1080), (0, 0)] {
        let mut cfg = vec![0u8; 0x4e];
        cfg[..4].copy_from_slice(b"YSCF");
        cfg[4..8].copy_from_slice(&555u32.to_le_bytes());
        cfg[0x10..0x14].copy_from_slice(&w.to_le_bytes());
        cfg[0x14..0x18].copy_from_slice(&h.to_le_bytes());
        std::fs::write(dir.join("ysbin/yscfg.ybn"), cfg).unwrap();
        let fs = Arc::new(GameFs::open(&dir).unwrap());
        let vm = Bootstrap { fs, key: None, entry_label: None }.boot().unwrap().vm;
        let expected = if w == 0 { (1280, 720) } else { (w, h) };
        assert_eq!(vm.global_int(114), Some(32));
        assert_eq!(vm.global_int(115), Some(i64::from(expected.0)));
        assert_eq!(vm.global_int(116), Some(i64::from(expected.1)));
    }

    if std::env::var_os("YURIS_KEEP_TEST_DIR").is_none() {
        std::fs::remove_dir_all(&dir).ok();
    }
}
