//! 启动链:从合成游戏目录(pac/bn.ypf)推断封包名字 key 与脚本密钥并跑到脚本结束。

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

    // RETURN(帧空 = 脚本结束),带一个返回值窗口(push 1)供密钥推断
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
    if std::env::var_os("YURIS_KEEP_TEST_DIR").is_none() {
        std::fs::remove_dir_all(&dir).ok();
    }
}
