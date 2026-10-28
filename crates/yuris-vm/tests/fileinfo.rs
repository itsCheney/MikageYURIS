//! FILEINFO CGFILE must use the same extension lookup as image loading.
mod common;
use common::{make_ystb, SAMPLE_KEY};
use yuris_format::{vfs::GameFs, ystb::YstbFile};
use yuris_vm::{GroupVm, VmSuspend, cmd};

#[test]
fn cgfile_resolves_extensions_without_changing_file_queries() {
    let dir = std::env::temp_dir().join(format!("yuris-fileinfo-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("cgsys")).unwrap();
    std::fs::write(dir.join("cgsys/back_off.png"), b"existence does not require decoding").unwrap();
    let fs = std::sync::Arc::new(GameFs::open(&dir).unwrap());
    for (slot, path, expected) in [
        (2, "cgsys/back_off", 1),
        (2, "cgsys/back_off.png", 1),
        (2, "cgsys/missing", 0),
        (1, "cgsys/back_off", 0),
        (1, "cgsys/back_off.png", 1),
    ] {
        let quoted = format!("\"{path}\"");
        let mut string = vec![0x4d];
        string.extend_from_slice(&(quoted.len() as u16).to_le_bytes());
        string.extend_from_slice(quoted.as_bytes());
        let yes = [0x42, 1, 0, 1];
        let mut target = vec![0x48, 3, 0, b'@'];
        target.extend_from_slice(&9000u16.to_le_bytes());
        let data = make_ystb(SAMPLE_KEY, &[(0x15, 3, 0), (cmd::RETURN, 0, 0)], &[
            ([slot, 0, 0, 0], string.len() as u32, 0, &string),
            ([4, 0, 0, 0], yes.len() as u32, 0, &yes),
            ([7, 0, 0, 0], target.len() as u32, 0, &target),
        ]);
        let mut vm = GroupVm::load(YstbFile::from_bytes(&data, SAMPLE_KEY).unwrap()).unwrap();
        vm.set_global_int(9000, -1).unwrap();
        vm.set_file_probe(fs.clone());
        assert!(matches!(vm.run(100).unwrap(), VmSuspend::Complete));
        assert_eq!(vm.global_int(9000), Some(expected), "slot={slot} path={path}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
