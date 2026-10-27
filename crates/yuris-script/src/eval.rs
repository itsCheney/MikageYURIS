//! Expression evaluator: runs one window on a stack (`docs/engine/bytecode.md`).
//!
//! Unknown ops and reads of undeclared variables are errors rather than
//! guesses.

use yuris_core::{Error, Result};
use yuris_value::{Value, VarRef, VarSpace, VariableStore};

use crate::insn::{BinOp, Cast, Insn, Instruction};

/// The display device exposed to scripts, distinct from the game canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayMetrics {
    width: u32,
    height: u32,
    bits_per_pixel: u32,
}
impl DisplayMetrics {
    /// Creates valid display metrics. Zero dimensions/depth are rejected.
    pub fn new(width: u32, height: u32, bits_per_pixel: u32) -> Option<Self> {
        (width > 0 && height > 0 && bits_per_pixel > 0)
            .then_some(Self { width, height, bits_per_pixel })
    }
}
impl Default for DisplayMetrics {
    /// Legacy standalone-VM fallback; game bootstrap supplies its own metrics.
    fn default() -> Self { Self { width: 1920, height: 1080, bits_per_pixel: 32 } }
}

/// Runtime values shared by expression and command variable reads.
#[derive(Debug, Clone, Copy, Default)]
pub struct VariableReadContext {
    /// Display device properties, not the drawable surface size.
    pub display: DisplayMetrics,
    /// Innermost loop iteration, or zero outside a loop.
    pub loop_counter: i64,
    /// Cursor in game logical coordinates.
    pub cursor: (i64, i64),
    /// Engine monotonic time in milliseconds.
    pub clock_ms: i64,
}

impl VariableReadContext {
    /// Reads computed system values, then frame-local and global storage.
    /// Empty indices mean a scalar read (element zero for arrays).
    /// Frame argument/return families never fall back to globals when a
    /// frame is present. Undeclared variables remain errors.
    pub fn read(
        &self,
        store: &VariableStore,
        locals: Option<&VariableStore>,
        r: &VarRef,
        indices: &[i64],
    ) -> Result<Value> {
        if r.space == VarSpace::At {
            let computed = match r.id {
                1 if indices == [1] => Some(self.clock_ms),
                48 => Some(self.loop_counter),
                133 => Some(self.cursor.0),
                138 => Some(self.cursor.1),
                114 => Some(i64::from(self.display.bits_per_pixel)),
                115 => Some(i64::from(self.display.width)),
                116 => Some(i64::from(self.display.height)),
                _ => None,
            };
            if let Some(v) = computed { return Ok(Value::Int(v)); }
        }
        if let Some(local) = locals {
            if indices.is_empty() {
                if let Some(v) = local.get_opt(r) { return Ok(v.clone()); }
            }
            if local.has_array(r) {
                return read_array(local, r, indices, is_frame_sysvar(r));
            }
            if is_frame_sysvar(r) {
                return Ok(frame_sysvar_default(frame_sysvar_elem(r)));
            }
        }
        if store.has_array(r) { return read_array(store, r, indices, true); }
        if indices.is_empty() { return store.get(r).cloned(); }
        store.get_elem(r, indices).cloned()
    }
}

fn read_array(store: &VariableStore, r: &VarRef, indices: &[i64], default_on_bounds: bool) -> Result<Value> {
    let zeros;
    let indices = if indices.is_empty() {
        zeros = vec![0; store.array_dims(r).unwrap_or_default().len()];
        &zeros[..]
    } else { indices };
    match store.get_elem(r, indices) {
        Ok(v) => Ok(v.clone()),
        Err(_) if default_on_bounds => Ok(type_default(store.array_elem_type(r))),
        Err(e) => Err(e),
    }
}

/// 栈式求值器。
pub struct Evaluator<'s> {
    store: &'s mut VariableStore,
    /// 当前 GOSUB 帧局部(可选):PushVar 优先查帧局部,再查全局。
    /// 帧局部 id 空间与全局分离(引擎:声明命令 id + 帧内槽)。
    locals: Option<&'s VariableStore>,
    /// @48 系统变量当前值 = 内层 LOOP 迭代计数(引擎 sysvar case 0x30,
    /// 00447ebc:读 obj+0x244 嵌套栈顶记录 +0x10;LOOP 置 1、LOOPEND +1、
    /// 无活动循环 = 0)。由 VM 在每次求值前注入。
    loop_counter: i64,
    /// 光标逻辑坐标(P9.2;引擎 sysvar case 0x85/0x8a = 窗口对象
    /// +0x2ac/+0x2b0,每帧 GetCursorPos→ScreenToClient→逻辑缩放,
    /// recon_00403f2c_input_poll.c)。由 VM 注入;(0,0) = 无输入面。
    cursor: (i64, i64),
    /// Engine clock in milliseconds, read as `@1[1]` (scripts time fades
    /// and waits with it: Erewhon's es.CGFADEEXEC computes
    /// `256 * (@1[1] - start) / duration`). Set by the VM.
    clock_ms: i64,
    display: DisplayMetrics,
    /// 调试:打印变量装载(默认 false)。
    pub verbose: bool,
}

impl<'s> Evaluator<'s> {
    /// 绑定变量存储(无帧局部)。
    pub fn new(store: &'s mut VariableStore) -> Self {
        Self { store, locals: None, loop_counter: 0, cursor: (0, 0), clock_ms: 0, display: DisplayMetrics::default(), verbose: false }
    }

    /// 绑定变量存储 + 帧局部。
    pub fn with_locals(
        store: &'s mut VariableStore,
        locals: Option<&'s VariableStore>,
    ) -> Self {
        Self { store, locals, loop_counter: 0, cursor: (0, 0), clock_ms: 0, display: DisplayMetrics::default(), verbose: false }
    }

    /// 注入 @48 当前值(内层 LOOP 迭代计数;无循环 = 0)。
    pub fn set_loop_counter(&mut self, v: i64) {
        self.loop_counter = v;
    }

    /// Sets the display device visible to system-variable reads.
    pub fn set_display_metrics(&mut self, display: DisplayMetrics) { self.display = display; }

    /// Sets the clock `@1[1]` reads.
    pub fn set_clock_ms(&mut self, ms: i64) {
        self.clock_ms = ms;
    }

    /// 注入光标逻辑坐标(@133/@138;引擎 obj+0x2ac/+0x2b0)。
    pub fn set_cursor(&mut self, x: i64, y: i64) {
        self.cursor = (x, y);
    }

    /// 解码并执行整个窗口,返回**剩余栈**(自底向上)。
    ///
    /// 正常的单值窗口余栈恰为 1 个元素;含组分隔的参数列表窗口可能为空。
    pub fn eval_window(&mut self, window: &[u8]) -> Result<Vec<Value>> {
        let instrs = crate::decode_window(window)?;
        self.eval_instructions(&instrs)
    }

    /// 执行已解码的指令序列,返回剩余栈。
    pub fn eval_instructions(&mut self, instrs: &[Instruction]) -> Result<Vec<Value>> {
        let slots = self.eval_slots(instrs)?;
        slots
            .into_iter()
            .map(|s| {
                s.into_value().ok_or_else(|| {
                    Error::Unimplemented(
                        "窗口以左值引用结尾(LET 语境需 eval_lvalue_window)",
                    )
                })
            })
            .collect()
    }

    /// 执行指令序列,返回完整槽位栈(LET / ArrayLoad 语境)。
    ///
    /// 槽位模型(引擎运行期变量处理器 0x48/0x56/0x76/0x29,`command-layer.md` §5b):
    /// - 0x56 → 平行**左值引用栈**(不装值);0x76 → 延迟占位(细节 Likely)
    /// - 0x29 → 从最近的左值引用点收集其上全部纯值项作下标表,行主序装载元素
    pub fn eval_slots(&mut self, instrs: &[Instruction]) -> Result<Vec<Slot>> {
        let mut stack: Vec<Slot> = Vec::new();
        for ins in instrs {
            match &ins.kind {
                Insn::PushInt(v) => stack.push(Slot::Val(Value::Int(*v))),
                Insn::PushFloat(v) => stack.push(Slot::Val(Value::Float(*v))),
                Insn::PushStr(p) => {
                    // 引擎 0x4d(成果 53):首字节为界定符,结果为界定符间
                    // 字节 + 转义解码;`22 22` → 空串(引擎空串字面量形态)
                    stack.push(Slot::Val(Value::Str(decode_pushstr(p))));
                }
                Insn::PushVar(r) => {
                    // 引擎 0x48:索引域清零后装载 → 标量,或数组的 0 号元素
                    let v = self.load_auto(r)?;
                    stack.push(Slot::Val(v));
                }
                Insn::PushVarRef(r) | Insn::PushVarIndexed(r) => {
                    stack.push(Slot::LValue(*r));
                }
                Insn::ArrayLoad { .. } => {
                    // 引擎 0x29:从引用点收集其上全部纯值项为下标表;
                    // 值栈 sp 重置到引用点、装载值压入(引用在平行引用栈单独弹)
                    let marker = stack
                        .iter()
                        .rposition(|s| matches!(s, Slot::LValue(_)))
                        .ok_or_else(|| {
                            Error::format("arrayload 无左值引用(引擎 0x1a5ea 同族)")
                        })?;
                    let indices = gather_indices(&stack[marker + 1..])?;
                    let r = match &stack[marker] {
                        Slot::LValue(r) => *r,
                        _ => unreachable!(),
                    };
                    let v = self.read_context().read(self.store, self.locals, &r, &indices)?;
                    stack.truncate(marker);
                    stack.push(Slot::Val(v));
                }
                Insn::Binary(op) => {
                    // 先弹右操作数,再弹左操作数(lhs op rhs)
                    let rhs = pop_val(&mut stack, ins)?;
                    let lhs = pop_val(&mut stack, ins)?;
                    stack.push(Slot::Val(apply_binary(*op, &lhs, &rhs)?));
                }
                Insn::Unary(_) => {
                    let v = pop_val(&mut stack, ins)?;
                    stack.push(Slot::Val(v.neg()?));
                }
                Insn::Cast(Cast::ToStr) => {
                    let v = pop_val(&mut stack, ins)?;
                    stack.push(Slot::Val(v.cast_to_str()?));
                }
                Insn::Cast(Cast::ToInt) => {
                    let v = pop_val(&mut stack, ins)?;
                    stack.push(Slot::Val(v.cast_to_int()?));
                }
                Insn::GroupSep => {}
                Insn::Unknown { raw_op, .. } => {
                    return Err(Error::UnresolvedOpcode {
                        code: *raw_op as u32,
                        offset: ins.offset,
                    });
                }
            }
        }
        Ok(stack)
    }

    /// LET 左值解析:窗口执行后,栈底为 LValue、其上全部为 Int 下标
    /// (引擎:引用点与栈顶之间的纯值项 = 下标表,推入序 = 维度序)。
    pub fn eval_lvalue_window(&mut self, window: &[u8]) -> Result<LValueRef> {
        let instrs = crate::decode_window(window)?;
        self.eval_lvalue_window_instrs(&instrs)
    }

    /// 已解码指令序列的左值解析。
    ///
    /// 引擎模型(VARH_00420ec4/004218b0/00421a4c 反编译,2026-09-03 勘误):
    /// - **0x48 运行期推值**(非引用)——旧实现把左值窗内全部 0x48 变换为引用
    ///   是过度概括,导致 `56 $a | 48 @i | 29` 的下标变量 @i 被误当数组基点;
    /// - 0x56 才推引用(记录标记位);0x29 从引用点上方收集纯值项作下标;
    /// - 0x29 命中**底层引用**且延迟模式时不装载、保留 {基点,下标} 交 LET
    ///   写回(VARH_00421a4c `iVar7==0 && db970!=0 → return 0`)。
    /// 语料佐证(全语料扫描):LET w0 含 aload 的窗口 **全部 0x56 起头**
    /// (6836 个;首指令 0x48 的多指令窗 0 个);单条 0x48/0x76 即标量左值。
    pub fn eval_lvalue_window_instrs(&mut self, instrs: &[Instruction]) -> Result<LValueRef> {
        // 单条 PushVar 族指令 → 直接作为赋值目标(不求值装载值)。
        if let [ins] = instrs {
            if let Insn::PushVar(r) | Insn::PushVarRef(r) | Insn::PushVarIndexed(r) = &ins.kind {
                return Ok(LValueRef { var: *r, indices: Vec::new() });
            }
        }
        // 多指令形态:基点 = **首条** var 类指令(语料全部 0x56);
        // 其后 0x48 保持取值语义(下标表达式);末条 0x29 在引擎延迟模式下
        // 不装载 → 从指令序列剥除,让基点引用与下标值留在栈上。
        let mut instrs: Vec<Instruction> = instrs.to_vec();
        if let Some(first) = instrs.first_mut() {
            if let Insn::PushVar(r) = first.kind {
                first.kind = Insn::PushVarRef(r);
            }
        }
        if matches!(instrs.last().map(|i| &i.kind), Some(Insn::ArrayLoad { .. })) {
            instrs.pop();
        } else if matches!(instrs.first().map(|i| &i.kind), Some(Insn::PushVarRef(_))) {
            // 无尾部 0x29 的多指令形态:除首条外若还有引用类指令,属未知形态
            if instrs.iter().skip(1).any(|i| {
                matches!(i.kind, Insn::PushVarRef(_) | Insn::PushVarIndexed(_))
            }) {
                return Err(Error::Unimplemented(
                    "LET 左值窗无 0x29 但含多条引用(语料外形态;不猜)",
                ));
            }
        }
        let slots = self.eval_slots(&instrs)?;
        let marker = slots
            .iter()
            .position(|s| matches!(s, Slot::LValue(_)))
            .ok_or_else(|| Error::Unimplemented("LET 左值窗口不含变量引用(不猜)"))?;
        let r = match &slots[marker] {
            Slot::LValue(r) => *r,
            _ => unreachable!(),
        };
        // 槽位形态(引擎 LET 左值窗):[LValue(0x56), 下标 Int×n]
        let indices = gather_indices(&slots[marker + 1..])?;
        if marker != 0 {
            return Err(Error::Unimplemented(
                "LET 左值引用前有多余栈项(引擎形态外;不猜)",
            ));
        }
        Ok(LValueRef { var: r, indices })
    }

    fn read_context(&self) -> VariableReadContext {
        VariableReadContext { display: self.display, loop_counter: self.loop_counter, cursor: self.cursor, clock_ms: self.clock_ms }
    }

    fn load_auto(&mut self, r: &VarRef) -> Result<Value> {
        self.read_context().read(self.store, self.locals, r, &[])
    }

}

/// 求值栈槽位:值或左值引用(引擎平行引用栈的忠实建模)。
#[derive(Debug, Clone, PartialEq)]
pub enum Slot {
    /// 普通值。
    Val(Value),
    /// 左值引用(0x56/0x76;LET 赋值目标 / ArrayLoad 的数组基点)。
    LValue(VarRef),
}

impl Slot {
    /// 转值;LValue → `None`。
    pub fn into_value(self) -> Option<Value> {
        match self {
            Slot::Val(v) => Some(v),
            Slot::LValue(_) => None,
        }
    }
}

/// LET 左值解析结果:变量 + 下标表(空 = 标量)。
#[derive(Debug, Clone, PartialEq)]
pub struct LValueRef {
    /// 变量引用。
    pub var: VarRef,
    /// 下标表(推入顺序 = 维度顺序;空 = 标量)。
    pub indices: Vec<i64>,
}

/// 帧局部系统变量族(@53/@60 INT、@54/@61 FLT、$55/$62 STR;成果 50/52)。
pub fn is_frame_sysvar(r: &VarRef) -> bool {
    match r.space {
        VarSpace::At => matches!(r.id, 53 | 54 | 60 | 61),
        VarSpace::Dollar => matches!(r.id, 55 | 62),
        _ => false,
    }
}

/// 帧局部系统族的元素类型(按 id 推断;gparam 未声明该型时读边界外默认)。
pub fn frame_sysvar_elem(r: &VarRef) -> Option<yuris_value::ElemType> {
    match (r.space, r.id) {
        (VarSpace::At, 53) | (VarSpace::At, 60) => Some(yuris_value::ElemType::Int),
        (VarSpace::At, 54) | (VarSpace::At, 61) => Some(yuris_value::ElemType::Float),
        (VarSpace::Dollar, 55) | (VarSpace::Dollar, 62) => Some(yuris_value::ElemType::Str),
        _ => None,
    }
}

/// 帧局部系统族越界读的默认值(引擎 sysvar case 0x35 边界外 → 0;
/// STR 族同理按类型默认 "" / 0.0)。
pub fn frame_sysvar_default(et: Option<yuris_value::ElemType>) -> Value {
    type_default(et)
}

/// 按元素类型的读取默认值(引擎全局数组读 FUN_00459490/00459418:
/// 越界 → 0 / 0.0 / "",不报错;成果 57)。
pub fn type_default(et: Option<yuris_value::ElemType>) -> Value {
    match et {
        Some(yuris_value::ElemType::Float) => Value::Float(0.0),
        Some(yuris_value::ElemType::Str) => Value::Str(Vec::new()),
        _ => Value::Int(0),
    }
}

/// 从槽位切片收集下标表:全部为 `Val(Int)`,否则报错
/// (引擎下标为 i64;Float/Str 下标语义 Unknown)。
fn gather_indices(slots: &[Slot]) -> Result<Vec<i64>> {
    slots
        .iter()
        .map(|s| match s {
            Slot::Val(Value::Int(i)) => Ok(*i),
            Slot::Val(_) => Err(Error::Unimplemented(
                "非整数下标语义 Unknown(引擎下标域为 i64)",
            )),
            Slot::LValue(_) => Err(Error::format(
                "下标表中混入左值引用(引擎 0x2710a 同族)",
            )),
        })
        .collect()
}

fn pop_val(stack: &mut Vec<Slot>, ins: &Instruction) -> Result<Value> {
    match stack.pop() {
        Some(Slot::Val(v)) => Ok(v),
        Some(Slot::LValue(_)) => Err(Error::format(format!(
            "lvalue 不能作运算数 at offset {}",
            ins.offset
        ))),
        None => Err(Error::format(format!(
            "stack underflow at offset {} (op {:#04x})",
            ins.offset, ins.raw_op
        ))),
    }
}

fn apply_binary(op: BinOp, lhs: &Value, rhs: &Value) -> Result<Value> {
    match op {
        BinOp::Add => lhs.add(rhs),
        BinOp::Sub => lhs.sub(rhs),
        BinOp::Mul => lhs.mul(rhs),
        BinOp::Div => lhs.div(rhs),
        BinOp::Mod => lhs.modulo(rhs),
        BinOp::Eq => Ok(Value::Int(lhs.eq(rhs)? as i64)),
        BinOp::Ne => Ok(Value::Int(lhs.ne(rhs)? as i64)),
        BinOp::Ge => Ok(Value::Int(lhs.ge(rhs)? as i64)),
        BinOp::Le => Ok(Value::Int(lhs.le(rhs)? as i64)),
        BinOp::Gt => Ok(Value::Int(lhs.gt(rhs)? as i64)),
        BinOp::Lt => Ok(Value::Int(lhs.lt(rhs)? as i64)),
        BinOp::BitAnd => lhs.bitand(rhs),
        BinOp::BitOr => lhs.bitor(rhs),
        BinOp::LogAnd => lhs.logand(rhs),
        BinOp::LogOr => lhs.logor(rhs),
        BinOp::Xor => lhs.xor(rhs),
    }
}

/// 引擎 0x4d 字符串字面量解码(成果 53,`00420cb8_expr_0x4d_pushstr.c` 反编译):
/// 载荷首字节 = 界定符(通常 `"`,也可以是 `'` 等);结果 = 界定符**之间**的字节,
/// 界定符本身不进入结果串;转义:`\\`→`\`、`\n`→LF(0x0a)、`\t`→TAB(0x09);
/// SJIS 双字节首字节(0x81-0x9F/0xE0-0xEF,引擎引导表 0x0059b0c0 语义)
/// 连同后续字节整体拷贝(避免把 SJIS 尾字节误当界定符/转义)。
/// 载荷为空/无界定符闭合 → 引擎循环自然终止于同界定符;畸形载荷(首字节
/// 即定界、无闭合)按引擎「读到闭合或截断」建模 —— 这里保守按无闭合处理,
/// 返回整个载荷(与旧行为兼容;语料无此形态)。
pub fn decode_pushstr(payload: &[u8]) -> Vec<u8> {
    let Some(&delim) = payload.first() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(payload.len());
    let mut i = 1usize;
    while i < payload.len() {
        let b = payload[i];
        if b == delim {
            // 引擎:遇到与首字节相同的界定符 → 结束(其后字节仍在窗口内,
            // 但 0x4d 是独立指令,操作数恰为字面量;这里到界定符即截断)
            return out;
        }
        if b == b'\\' {
            // 引擎转义分支:\\ → \ ;\n → 0x0a ;\t → 0x09;其余 → 丢弃反斜杠
            match payload.get(i + 1) {
                Some(0x5c) => {
                    out.push(0x5c);
                    i += 2;
                }
                Some(0x6e) => {
                    out.push(0x0a);
                    i += 2;
                }
                Some(0x74) => {
                    out.push(0x09);
                    i += 2;
                }
                // 反斜杠在末尾:引擎会继续读越界;保守按字面处理
                _ => {
                    out.push(b'\\');
                    i += 1;
                }
            }
            continue;
        }
        // SJIS 双字节:首字节 + 后续整体拷贝(引擎 0x0059b0c0 表引导)
        out.push(b);
        i += 1;
        if (0x81..=0x9F).contains(&b) || (0xE0..=0xEF).contains(&b) {
            if let Some(&b2) = payload.get(i) {
                out.push(b2);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode_window;
    use crate::insn::{VarRef, VarSpace};

    fn run(window: &[u8]) -> Result<Vec<Value>> {
        let mut store = VariableStore::new();
        Evaluator::new(&mut store).eval_window(window)
    }

    fn instrs(window: &[u8]) -> Vec<Instruction> {
        decode_window(window).unwrap()
    }

    #[test]
    fn constant_arithmetic() {
        // 2 + 3 = 5
        assert_eq!(
            run(&[0x42, 1, 0, 2, 0x42, 1, 0, 3, 0x2b, 0, 0]).unwrap(),
            vec![Value::Int(5)]
        );
        // 字符串拼接(手册 Confirmed;0x4d 载荷 = 界定符包裹,成果 53)
        assert_eq!(
            run(&[
                0x4d, 4, 0, b'"', b'1', b'2', b'"',
                0x4d, 4, 0, b'"', b'3', b'4', b'"',
                0x2b, 0, 0,
            ])
            .unwrap(),
            vec![Value::Str(b"1234".to_vec())]
        );
        // 混合精度: 5 * 0.5 = 2.5
        let mut w = vec![0x42, 1, 0, 5, 0x46, 8, 0];
        w.extend_from_slice(&0.5f64.to_le_bytes());
        w.extend_from_slice(&[0x2a, 0, 0]);
        assert_eq!(run(&w).unwrap(), vec![Value::Float(2.5)]);
        // 操作数顺序: 8 - 3 = 5(先弹 rhs)
        assert_eq!(
            run(&[0x42, 1, 0, 8, 0x42, 1, 0, 3, 0x2d, 0, 0]).unwrap(),
            vec![Value::Int(5)]
        );
    }

    #[test]
    fn comparisons_and_casts() {
        assert_eq!(
            run(&[0x42, 1, 0, 5, 0x42, 1, 0, 5, 0x3d, 0, 0]).unwrap(),
            vec![Value::Int(1)]
        );
        assert_eq!(
            run(&[0x42, 1, 0, 3, 0x42, 1, 0, 5, 0x3c, 0, 0]).unwrap(),
            vec![Value::Int(1)]
        );
        // $() 转字符串
        assert_eq!(
            run(&[0x42, 1, 0, 9, 0x73, 0, 0]).unwrap(),
            vec![Value::Str(b"9".to_vec())]
        );
        // @() 转整数(手册: @("123"+"45") == 12345;0x4d 带界定符,成果 53)
        let w = [
            0x4d, 5, 0, b'"', b'1', b'2', b'3', b'"',
            0x4d, 4, 0, b'"', b'4', b'5', b'"',
            0x2b, 0, 0,
            0x69, 0, 0,
        ];
        assert_eq!(run(&w).unwrap(), vec![Value::Int(12345)]);
    }

    #[test]
    fn pushstr_decodes_delimiters_and_escapes() {
        // 成果 53(引擎 00420cb8):首字节 = 界定符;结果 = 界定符间字节;
        // 空串字面量 22 22 → 空;转义 \\ \n \t;SJIS 双字节整体拷贝
        assert_eq!(decode_pushstr(b"\"\""), Vec::<u8>::new());
        assert_eq!(decode_pushstr(b"\"AB\""), b"AB".to_vec());
        assert_eq!(decode_pushstr(b"'ES'"), b"ES".to_vec());
        assert_eq!(decode_pushstr(b"\"a\\\\b\""), b"a\\b".to_vec());
        assert_eq!(decode_pushstr(b"\"a\\nb\""), b"a\nb".to_vec());
        assert_eq!(decode_pushstr(b"\"a\\tb\""), b"a\tb".to_vec());
        // SJIS 双字节(0x83 0x5c)不因尾字节 0x5c 误入转义
        assert_eq!(decode_pushstr(b"\"\x83\x5c\""), b"\x83\x5c".to_vec());
        // 空载荷 → 空串
        assert_eq!(decode_pushstr(b""), Vec::<u8>::new());
    }

    #[test]
    fn variables_and_errors() {
        let mut store = VariableStore::new();
        store.set(
            &VarRef { space: VarSpace::At, id: 100 },
            Value::Int(7),
        );
        let w = [0x48, 3, 0, 0x40, 100, 0, 0x42, 1, 0, 1, 0x2b, 0, 0];
        assert_eq!(
            Evaluator::new(&mut store).eval_window(&w).unwrap(),
            vec![Value::Int(8)]
        );
        // 未定义变量 → 报错
        let w_undef = [0x48, 3, 0, 0x40, 0xff, 0xff];
        let mut store2 = VariableStore::new();
        assert!(Evaluator::new(&mut store2).eval_window(&w_undef).is_err());
        // pushvarref → Unimplemented(语义未解,不猜)
        let w_ref = [0x56, 3, 0, 0x24, 0x04, 0x00];
        assert!(matches!(
            run(&w_ref),
            Err(Error::Unimplemented(_))
        ));
        // 未证实 opcode → UnresolvedOpcode
        assert!(matches!(
            run(&[0x00, 0, 0]),
            Err(Error::UnresolvedOpcode { code: 0x00, offset: 0 })
        ));
        // 栈下溢 → 报错
        assert!(run(&[0x2b, 0, 0]).is_err());
    }
}
