use super::*;
use yuris_value::{ElemType, VarRef, VarSpace};
use yuris_script::eval::Evaluator;
fn vm() -> GroupVm {
    let mut bytes = b"YSTB".to_vec();
    for word in [555u32, 1, 4, 0, 0, 4, 0] { bytes.extend(word.to_le_bytes()); }
    bytes.extend([cmd::RETURN, 0, 0, 0]);
    bytes.extend([0; 4]);
    GroupVm::load(yuris_format::ystb::YstbFile::from_bytes(&bytes, [0; 4]).unwrap()).unwrap()
}
fn expression(vm: &mut GroupVm, r: VarRef, indices: &[i64]) -> yuris_core::Result<Value> {
    let mut bytes = vec![if indices.is_empty() { 0x48 } else { 0x56 }, 3, 0, r.space.prefix()];
    bytes.extend(r.id.to_le_bytes());
    for &i in indices { bytes.extend([0x42, 1, 0, i as u8]); }
    if !indices.is_empty() { bytes.extend([0x29, 1, 0, 0]); }
    let mut ev = Evaluator::with_locals(&mut vm.store, vm.frames.last().map(|f| &f.locals));
    ev.set_cursor(vm.input_cursor.0, vm.input_cursor.1);
    ev.set_display_metrics(vm.display_metrics);
    ev.set_clock_ms(vm.clock_ms as i64);
    ev.set_loop_counter(vm.loops.last().map(|l| l.counter as i64).unwrap_or(0));
    Ok(ev.eval_window(&bytes)?.remove(0))
}
fn check(vm: &mut GroupVm, r: VarRef, indices: &[i64], expected: Value) {
    assert_eq!(vm.read_var_value(&r, indices), Some(expected.clone()));
    assert_eq!(expression(vm, r, indices).unwrap(), expected);
}
#[test]
fn computed_system_reads_agree_across_expression_and_command_paths() {
    let mut vm = vm();
    vm.set_clock_ms(12345);
    vm.set_input_cursor(321, 654);
    vm.loops.push(LoopFrame { body_start: 0, exit_target: 1, limit: None, counter: 7, if_depth: 0 });
    for (id, expected) in [(48, 7), (133, 321), (138, 654), (114, 32), (115, 1920), (116, 1080)] {
        let r = VarRef { space: VarSpace::At, id };
        vm.store.set(&r, Value::Int(-99)); // computed values override stale storage
        check(&mut vm, r, &[], Value::Int(expected));
        check(&mut vm, r, &[0], Value::Int(expected));
    }
    let clock = VarRef { space: VarSpace::At, id: 1 };
    vm.store.declare_array(&clock, ElemType::Int, &[5]);
    check(&mut vm, clock, &[1], Value::Int(12345));
    check(&mut vm, clock, &[], Value::Int(0));
    check(&mut vm, clock, &[2], Value::Int(0));
}
#[test]
fn frame_defaults_and_array_bounds_agree() {
    let mut vm = vm();
    vm.frames.push(GosubFrame { return_pc: 0, script_id: 0, locals: VariableStore::new(), if_nest_depth: 0, loop_depth: 0 });
    for (space, id, ty, zero, nonzero) in [
        (VarSpace::At, 53, ElemType::Int, Value::Int(0), Value::Int(9)),
        (VarSpace::At, 54, ElemType::Float, Value::Float(0.0), Value::Float(2.5)),
        (VarSpace::Dollar, 55, ElemType::Str, Value::Str(vec![]), Value::Str(b"local".to_vec())),
    ] {
        let r = VarRef { space, id };
        vm.store.declare_array(&r, ty, &[2]);
        vm.store.set_elem(&r, &[1], nonzero.clone()).unwrap();
        check(&mut vm, r, &[1], zero.clone()); // no global fallback
        vm.frames.last_mut().unwrap().locals.declare_array(&r, ty, &[2]);
        vm.frames.last_mut().unwrap().locals.set_elem(&r, &[1], nonzero.clone()).unwrap();
        check(&mut vm, r, &[1], nonzero);
        check(&mut vm, r, &[9], zero);
    }
    for (space, id, expected) in [
        (VarSpace::At, 60, Value::Int(0)),
        (VarSpace::At, 61, Value::Float(0.0)),
        (VarSpace::Dollar, 62, Value::Str(vec![])),
    ] {
        check(&mut vm, VarRef { space, id }, &[1], expected);
    }
    let r = VarRef { space: VarSpace::At, id: 1200 };
    vm.store.declare_array(&r, ElemType::Int, &[0]);
    check(&mut vm, r, &[], Value::Int(0));
    vm.frames.last_mut().unwrap().locals.declare_array(&r, ElemType::Int, &[1]);
    assert!(expression(&mut vm, r, &[9]).is_err());
    assert!(vm.read_var_value(&r, &[9]).is_none());
    let unknown = VarRef { space: VarSpace::At, id: 65000 };
    assert!(expression(&mut vm, unknown, &[]).is_err());
    assert!(vm.read_var_value(&unknown, &[]).is_none());
}

#[test]
fn display_capabilities_are_independent_of_cursor_and_storage() {
    let mut vm = vm();
    vm.set_input_cursor(123, 456);
    for (w, h, depth) in [(800, 600, 16), (1280, 720, 32), (1920, 1080, 32), (2560, 1440, 32)] {
        vm.set_display_metrics(DisplayMetrics::new(w, h, depth).unwrap());
        for (id, v) in [(114, depth), (115, w), (116, h)] {
            let r = VarRef { space: VarSpace::At, id };
            check(&mut vm, r, &[], Value::Int(i64::from(v)));
            check(&mut vm, r, &[0], Value::Int(i64::from(v)));
        }
        check(&mut vm, VarRef { space: VarSpace::At, id: 133 }, &[], Value::Int(123));
        check(&mut vm, VarRef { space: VarSpace::At, id: 138 }, &[], Value::Int(456));
    }
    assert!(DisplayMetrics::new(0, 720, 32).is_none());
    assert!(DisplayMetrics::new(1280, 0, 32).is_none());
    assert!(DisplayMetrics::new(1280, 720, 0).is_none());
}
