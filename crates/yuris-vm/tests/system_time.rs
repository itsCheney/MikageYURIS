//! Date rows must initialize even when YSVR already declared the array.
mod common;
use yuris_format::{ystb::YstbFile, ysvr::{YsvrTable, YsvrEntry, YsvrInit}};
use yuris_value::{VarRef, VarSpace, Value};
use yuris_vm::{GroupVm, cmd};

#[test]
fn date_initialization_is_independent_of_ysvr_declaration() {
    for predeclared in [false, true] {
        let bytes = common::make_ystb([0; 4], &[(cmd::RETURN, 0, 0)], &[]);
        let mut vm = GroupVm::load(YstbFile::from_bytes(&bytes, [0; 4]).unwrap()).unwrap();
        if predeclared {
            vm.apply_ysvr(&YsvrTable { version: 555, consumed: 0, entries: vec![YsvrEntry {
                kind: 1, category: 0, script: 0, var_id: 0, ty: 1,
                bounds: vec![2, 8], init: YsvrInit::Int(0),
            }] }).unwrap();
        }
        let start = [2026, 10, 26, 1, 12, 34, 56, 789];
        let later = [2026, 10, 27, 2, 1, 2, 3, 4];
        let r = VarRef { space: VarSpace::At, id: 0 };
        vm.set_date_time(start).unwrap();
        vm.set_date_time(later).unwrap();
        for i in 0..8 {
            assert_eq!(vm.store().get_elem(&r, &[0, i as i64]).unwrap(), &Value::Int(later[i]));
            assert_eq!(vm.store().get_elem(&r, &[1, i as i64]).unwrap(), &Value::Int(start[i]));
        }
    }
}
