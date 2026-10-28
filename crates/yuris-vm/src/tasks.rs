//! Tasks: cooperative threads of script execution.
//!
//! YU-RIS games run much of their flow as tasks. Erewhon's main script
//! (`ES.GAMEMAIN.LOOP`) only runs the per-frame effect and button tasks; the
//! scenario itself (`es.SCENARIO.FLOW`, which leads to the logo and title)
//! is started with `TASK ID=... #=label`.
//!
//! Model (from the scripts; the engine's scheduler is not reversed yet):
//! - Each task has its own position, GOSUB frames, IF stack and LOOP stack.
//!   Variables are shared.
//! - Tasks yield with WAIT. Within one frame every runnable task runs once,
//!   until it waits or ends, in order of Z then creation; the main script
//!   has Z 0. After the last one the VM returns one frame of `Wait` to the
//!   host.
//! - `TASK E=0` pauses a task (it is skipped until `E=1`), `TASKEND` ends
//!   one, and a task ends when it RETURNs from its entry frame.
//! - TINT/TFLT/TSTR become the entry frame's `@53`/`@54`/`$55`, like GOSUB
//!   arguments.
//!
//! With no task created the VM behaves as before: WAIT returns its own
//! frame count or time to the host.

use std::sync::Arc;

use yuris_core::{Error, Result};
use yuris_value::{Value, VariableStore};

use crate::host::ScriptCtx;
use crate::{GosubFrame, GroupVm, IfFrame, LoopFrame, VmSuspend};

/// A task's execution state while another task runs.
#[derive(Debug, Clone)]
pub(crate) struct ExecState {
    pub ctx: Arc<ScriptCtx>,
    pub pc: usize,
    pub frames: Vec<GosubFrame>,
    pub if_nest: Vec<IfFrame>,
    pub loops: Vec<LoopFrame>,
}

/// One task.
#[derive(Debug, Clone)]
pub(crate) struct Task {
    /// Task name (`ID`); empty for the main script.
    pub name: Vec<u8>,
    /// Creation order, for stable scheduling.
    pub seq: u64,
    /// Run order within a frame (`Z`); lower runs first.
    pub z: i64,
    /// `E`: a disabled task is skipped.
    pub enabled: bool,
    /// `A` as given (meaning unknown; reported by TASKINFO).
    pub a: i64,
    /// Frames left to wait.
    pub wait_frames: u64,
    /// Clock time to wait for (ms).
    pub wait_until_ms: u64,
    /// Stopped at the end of its text until `TASK RESTART=1` (see `text`).
    pub text_wait: bool,
    /// Number of the last text line it finished (RETURNCODE), for TASKINFO
    /// TEXTLINE.
    pub text_line: i64,
    /// Saved state; None while this task is the running one.
    pub saved: Option<ExecState>,
}

/// A task as [`GroupVm::task_stacks`] reports it.
#[derive(Debug, Clone)]
pub struct TaskReport {
    /// Task name; empty for the main script.
    pub name: String,
    /// Whether it is the task running now.
    pub running: bool,
    /// `E`: false while paused.
    pub enabled: bool,
    /// Run order within a frame.
    pub z: i64,
    /// Frames left to wait.
    pub wait_frames: u64,
    /// Clock time (ms) it waits for.
    pub wait_until_ms: u64,
    /// Stopped at the end of its text.
    pub text_wait: bool,
    /// (script, pc) from the entry frame's caller inward; the last one is
    /// where the task is.
    pub stack: Vec<(u16, usize)>,
}

/// Parameters of a TASK command (YSCM slots).
#[derive(Debug, Default)]
pub(crate) struct TaskParams {
    pub id: Option<Vec<u8>>,
    pub e: Option<i64>,
    pub a: Option<i64>,
    pub z: Option<i64>,
    pub tint: Vec<(usize, i64)>,
    pub tflt: Vec<(usize, f64)>,
    pub tstr: Vec<(usize, Vec<u8>)>,
    pub label: Option<Vec<u8>>,
    pub script_pos: Option<Vec<u8>>,
    pub restart: bool,
}

impl TaskParams {
    /// Collects one evaluated slot.
    pub fn set(&mut self, slot: u8, v: &Value) {
        let int = || match v {
            Value::Int(n) => *n,
            Value::Float(f) => *f as i64,
            Value::Str(_) => 0,
        };
        let s = slot as usize;
        match s {
            0 => self.id = Some(strip_quotes(v)),
            1 => self.e = Some(int()),
            2 => self.a = Some(int()),
            3 => self.z = Some(int()),
            4..=13 => self.tint.push((s - 3, int())),
            14..=23 => self.tflt.push((
                s - 13,
                match v {
                    Value::Float(f) => *f,
                    Value::Int(n) => *n as f64,
                    Value::Str(_) => 0.0,
                },
            )),
            24..=33 => self.tstr.push((s - 23, strip_quotes(v))),
            34 => self.label = Some(strip_quotes(v)),
            35 => self.script_pos = Some(strip_quotes(v)),
            37 => self.restart = int() != 0,
            _ => {}
        }
    }
}

/// String value of a slot, without the quotes string literals carry.
pub(crate) fn strip_quotes(v: &Value) -> Vec<u8> {
    match v {
        Value::Str(s) if s.len() >= 2 && s[0] == b'"' && s[s.len() - 1] == b'"' => {
            s[1..s.len() - 1].to_vec()
        }
        Value::Str(s) => s.clone(),
        Value::Int(n) => n.to_string().into_bytes(),
        Value::Float(f) => f.to_string().into_bytes(),
    }
}

/// `SCRIPTPOS` strings are ours: `script:pc` of the task's position, then
/// `;script:pc` for the return point of each GOSUB it is in, outermost
/// first. They round-trip through TASKINFO SCRIPTPOS and TASK SCRIPTPOS
/// within this runtime (Erewhon keeps one in its saves). GOSUB locals,
/// IF and LOOP state are not kept.
pub(crate) fn format_script_pos(script_id: u16, pc: usize, frames: &[GosubFrame]) -> Vec<u8> {
    let mut s = format!("{script_id}:{pc}");
    for f in frames.iter().skip(1) {
        s.push_str(&format!(";{}:{}", f.script_id, f.return_pc));
    }
    s.into_bytes()
}

/// The positions of a `SCRIPTPOS` string: the task's own first, then the
/// return points.
fn parse_script_pos(s: &[u8]) -> Option<Vec<(u16, usize)>> {
    let s = std::str::from_utf8(s).ok()?;
    s.split(';')
        .map(|p| {
            let (a, b) = p.split_once(':')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        })
        .collect()
}

impl GroupVm {
    /// Whether any task besides the main script exists.
    pub fn has_tasks(&self) -> bool {
        self.tasks.len() > 1
    }

    /// Names of the live tasks, in run order (the main script is "").
    pub fn task_names(&self) -> Vec<String> {
        let mut v: Vec<&Task> = self.tasks.iter().collect();
        v.sort_by_key(|t| (t.z, t.seq));
        v.iter().map(|t| String::from_utf8_lossy(&t.name).into_owned()).collect()
    }

    /// Every task with its call stack (return points, then the current
    /// position, innermost last), in run order. The main script is "".
    pub fn task_stacks(&self) -> Vec<TaskReport> {
        let mut v: Vec<(usize, &Task)> = self.tasks.iter().enumerate().collect();
        v.sort_by_key(|(_, t)| (t.z, t.seq));
        v.into_iter()
            .map(|(i, t)| {
                let (frames, sid, pc) = match &t.saved {
                    Some(s) => (&s.frames, s.ctx.script_id, s.pc),
                    None => (&self.frames, self.ctx.script_id, self.pc),
                };
                let mut stack: Vec<(u16, usize)> =
                    frames.iter().skip(1).map(|f| (f.script_id, f.return_pc.saturating_sub(1))).collect();
                stack.push((sid, pc));
                TaskReport {
                    name: String::from_utf8_lossy(&t.name).into_owned(),
                    running: i == self.current_task,
                    enabled: t.enabled,
                    z: t.z,
                    wait_frames: t.wait_frames,
                    wait_until_ms: t.wait_until_ms,
                    text_wait: t.text_wait,
                    stack,
                }
            })
            .collect()
    }

    fn task_index(&self, name: &[u8]) -> Option<usize> {
        self.tasks.iter().position(|t| t.name == name)
    }

    /// The task TASK/TASKINFO/TASKEND address: by name, or the running one.
    pub(crate) fn task_target(&self, id: Option<&[u8]>) -> Option<usize> {
        match id {
            Some(n) if !n.is_empty() => self.task_index(n),
            _ => (!self.tasks.is_empty()).then_some(self.current_task),
        }
    }

    fn ensure_main_task(&mut self) {
        if self.tasks.is_empty() {
            self.tasks.push(Task {
                name: Vec::new(),
                seq: 0,
                z: 0,
                enabled: true,
                a: 0,
                wait_frames: 0,
                wait_until_ms: 0,
                text_wait: false,
                text_line: 0,
                saved: None,
            });
            self.current_task = 0;
            self.task_seq = 1;
        }
    }

    fn take_live_state(&mut self) -> ExecState {
        ExecState {
            ctx: self.ctx.clone(),
            pc: self.pc,
            frames: std::mem::take(&mut self.frames),
            if_nest: std::mem::take(&mut self.if_nest),
            loops: std::mem::take(&mut self.loops),
        }
    }

    fn put_live_state(&mut self, s: ExecState) {
        self.ctx = s.ctx;
        self.pc = s.pc;
        self.frames = s.frames;
        self.if_nest = s.if_nest;
        self.loops = s.loops;
    }

    fn switch_task(&mut self, to: usize) {
        if to == self.current_task {
            return;
        }
        let live = self.take_live_state();
        if let Some(t) = self.tasks.get_mut(self.current_task) {
            t.saved = Some(live);
        }
        let next = self.tasks[to].saved.take().expect("idle task has a saved state");
        self.put_live_state(next);
        self.current_task = to;
    }

    fn runnable(&self, i: usize) -> bool {
        let t = &self.tasks[i];
        t.enabled && !t.text_wait && t.wait_frames == 0 && self.clock_ms >= t.wait_until_ms
    }

    /// Starts a frame: counts down frame waits and fixes this frame's order.
    fn begin_frame(&mut self) {
        for t in &mut self.tasks {
            t.wait_frames = t.wait_frames.saturating_sub(1);
        }
        let mut order: Vec<usize> = (0..self.tasks.len()).collect();
        order.sort_by_key(|&i| (self.tasks[i].z, self.tasks[i].seq));
        self.frame_order = order.into_iter().map(|i| self.tasks[i].seq).collect();
        self.frame_pos = 0;
    }

    /// Switches to the next runnable task of this frame. False when the
    /// frame is over.
    fn next_task_in_frame(&mut self) -> bool {
        while self.frame_pos < self.frame_order.len() {
            let seq = self.frame_order[self.frame_pos];
            self.frame_pos += 1;
            let Some(i) = self.tasks.iter().position(|t| t.seq == seq) else { continue };
            if self.runnable(i) {
                self.switch_task(i);
                return true;
            }
        }
        false
    }

    /// Called at the top of `run`: after a frame was handed to the host,
    /// start the next one with the first runnable task.
    pub(crate) fn resume_frame(&mut self) -> Option<VmSuspend> {
        if !self.frame_done {
            return None;
        }
        self.frame_done = false;
        self.begin_frame();
        if self.next_task_in_frame() {
            None
        } else {
            // Everyone is still waiting: this frame passes as well.
            self.frame_done = true;
            Some(VmSuspend::Wait {
                counter: Some(1),
                time_ms: None,
            })
        }
    }

    /// WAIT in the running task. Some = return this to the host; None = the
    /// VM switched to another task and keeps running.
    pub(crate) fn task_wait(&mut self, counter: Option<u64>, time_ms: Option<u64>) -> Option<VmSuspend> {
        if !self.has_tasks() {
            return Some(VmSuspend::Wait { counter, time_ms });
        }
        let clock = self.clock_ms;
        let t = &mut self.tasks[self.current_task];
        t.wait_frames = counter.unwrap_or(0);
        t.wait_until_ms = time_ms.map(|ms| clock + ms).unwrap_or(0);
        if t.wait_frames == 0 && t.wait_until_ms == 0 {
            t.wait_frames = 1;
        }
        self.end_turn()
    }

    /// The running task gave up the rest of this frame (waited, paused or
    /// ended): run the next one, or end the frame.
    pub(crate) fn end_turn(&mut self) -> Option<VmSuspend> {
        if self.next_task_in_frame() {
            return None;
        }
        self.frame_done = true;
        Some(VmSuspend::Wait {
            counter: Some(1),
            time_ms: None,
        })
    }

    /// The running task ended (RETURN from its entry frame, TASKEND, or the
    /// end of its script). Some = return this; None = keep running another.
    /// The main script ending ends the run.
    pub(crate) fn task_finished(&mut self) -> Option<VmSuspend> {
        if self.tasks.is_empty() || self.tasks[self.current_task].name.is_empty() {
            return Some(VmSuspend::Complete);
        }
        self.remove_task(self.current_task)
    }

    fn remove_task(&mut self, i: usize) -> Option<VmSuspend> {
        let was_current = i == self.current_task;
        if was_current {
            // Park the live state on the main task's slot logic: the
            // removed task's state is simply dropped.
            let _ = self.take_live_state();
        }
        self.tasks.remove(i);
        if self.current_task > i {
            self.current_task -= 1;
        }
        if !was_current {
            return None;
        }
        // Nothing is live now; the next task supplies the state.
        self.current_task = usize::MAX;
        if let Some(s) = self.pick_next_after_removal() {
            return s;
        }
        None
    }

    fn pick_next_after_removal(&mut self) -> Option<Option<VmSuspend>> {
        while self.frame_pos < self.frame_order.len() {
            let seq = self.frame_order[self.frame_pos];
            self.frame_pos += 1;
            let Some(i) = self.tasks.iter().position(|t| t.seq == seq) else { continue };
            if self.runnable(i) {
                let next = self.tasks[i].saved.take().expect("idle task has a saved state");
                self.put_live_state(next);
                self.current_task = i;
                return Some(None);
            }
        }
        // End of frame; keep the main task live so the VM has a state.
        let main = self.tasks.iter().position(|t| t.name.is_empty()).unwrap_or(0);
        let next = self.tasks[main].saved.take().expect("idle task has a saved state");
        self.put_live_state(next);
        self.current_task = main;
        self.frame_done = true;
        Some(Some(VmSuspend::Wait {
            counter: Some(1),
            time_ms: None,
        }))
    }

    /// TASK: create a task at `#` (or at SCRIPTPOS), or change E/A/Z of an
    /// existing one. Returns a description for the event, and Some(suspend)
    /// when the running task paused itself and the VM moved on.
    pub(crate) fn exec_task(&mut self, p: TaskParams) -> Result<(String, Option<Option<VmSuspend>>)> {
        // A bare `TASK RESTART=1` is how the text task lets the task that
        // wrote the text go on (see `text`).
        if p.restart && p.id.is_none() && p.label.is_none() && p.script_pos.is_none() {
            let n = self.text_restart();
            return Ok((format!("text restart: {n} task(s) go on"), None));
        }
        let existing = self.task_target(p.id.as_deref());
        let script_pos = match &p.script_pos {
            Some(pos) if !pos.is_empty() => Some(
                parse_script_pos(pos)
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| Error::format(format!("TASK SCRIPTPOS {:?}", String::from_utf8_lossy(pos))))?,
            ),
            _ => None,
        };
        // From the manual: SCRIPTPOS on an existing task overwrites its
        // position, "in effect a load of the script position".
        if let (Some(i), Some(stack)) = (existing, &script_pos) {
            if !p.restart {
                return self.move_task(i, stack).map(|d| (d, None));
            }
        }
        let start = match (&p.label, &script_pos) {
            (_, Some(stack)) => Some(stack[0]),
            (Some(label), _) if !label.is_empty() => {
                let (pc, sid) = self.labels.get(label.as_slice()).copied().ok_or_else(|| {
                    Error::format(format!("TASK label not found: {}", String::from_utf8_lossy(label)))
                })?;
                Some((sid, pc as usize))
            }
            _ => None,
        };
        let name = p.id.clone().unwrap_or_default();
        if let (Some(i), false) = (existing, p.restart && start.is_some()) {
            let t = &mut self.tasks[i];
            if let Some(e) = p.e {
                t.enabled = e != 0;
            }
            if let Some(a) = p.a {
                t.a = a;
            }
            if let Some(z) = p.z {
                t.z = z;
            }
            let desc = format!(
                "task {} updated (E={} Z={})",
                String::from_utf8_lossy(&self.tasks[i].name),
                self.tasks[i].enabled as u8,
                self.tasks[i].z
            );
            // A task that disables itself stops at once.
            if i == self.current_task && !self.tasks[i].enabled && self.has_tasks() {
                self.pc += 1;
                return Ok((desc, Some(self.end_turn())));
            }
            return Ok((desc, None));
        }
        let Some((sid, pc)) = start else {
            return Ok((
                format!("task {} not found and no # label", String::from_utf8_lossy(&name)),
                None,
            ));
        };
        if let Some(i) = existing {
            if i == self.current_task {
                return Err(Error::format("TASK RESTART of the running task is not supported"));
            }
            self.tasks.remove(i);
            if self.current_task > i {
                self.current_task -= 1;
            }
        }
        self.ensure_main_task();
        let ctx = self.prepared_script(sid)?;
        let mut locals = VariableStore::new();
        if let Some(ids) = self.system_vars.ids() {
            seed_task_args(&mut locals, &p, &ids);
        }
        let state = ExecState {
            ctx,
            pc,
            frames: vec![GosubFrame {
                return_pc: 0,
                script_id: sid,
                locals,
                if_nest_depth: 0,
                loop_depth: 0,
            }],
            if_nest: Vec::new(),
            loops: Vec::new(),
        };
        let seq = self.task_seq;
        self.task_seq += 1;
        self.tasks.push(Task {
            name: name.clone(),
            seq,
            z: p.z.unwrap_or(0),
            enabled: p.e.map(|e| e != 0).unwrap_or(true),
            a: p.a.unwrap_or(0),
            wait_frames: 0,
            wait_until_ms: 0,
            text_wait: false,
            text_line: 0,
            saved: Some(state),
        });
        // A new task runs later this frame if its place in the order has
        // not passed yet (it goes in order of Z after the running task).
        let cur = self.tasks.get(self.current_task).map(|t| (t.z, t.seq));
        let z = p.z.unwrap_or(0);
        if cur.map(|c| (z, seq) > c).unwrap_or(true) {
            let insert_at = self.frame_order[self.frame_pos.min(self.frame_order.len())..]
                .iter()
                .position(|s| {
                    self.tasks
                        .iter()
                        .find(|t| t.seq == *s)
                        .map(|t| (t.z, t.seq) > (z, seq))
                        .unwrap_or(false)
                })
                .map(|k| k + self.frame_pos)
                .unwrap_or(self.frame_order.len());
            self.frame_order.insert(insert_at, seq);
        }
        Ok((
            format!(
                "task {} created at yst{sid:05} pc {pc} (Z={z})",
                String::from_utf8_lossy(&name)
            ),
            None,
        ))
    }

    /// Puts task `i` at a SCRIPTPOS stack (see [`format_script_pos`]),
    /// with fresh GOSUB frames and no IF/LOOP state, and wakes it.
    fn move_task(&mut self, i: usize, stack: &[(u16, usize)]) -> Result<String> {
        if i == self.current_task {
            return Err(Error::format("TASK SCRIPTPOS on the running task is not supported"));
        }
        let (sid, pc) = stack[0];
        let ctx = self.prepared_script(sid)?;
        let entry_sid = stack.get(1).map(|f| f.0).unwrap_or(sid);
        let mut frames = vec![GosubFrame {
            return_pc: 0,
            script_id: entry_sid,
            locals: VariableStore::new(),
            if_nest_depth: 0,
            loop_depth: 0,
        }];
        for &(rsid, rpc) in &stack[1..] {
            self.prepared_script(rsid)?;
            frames.push(GosubFrame {
                return_pc: rpc,
                script_id: rsid,
                locals: VariableStore::new(),
                if_nest_depth: 0,
                loop_depth: 0,
            });
        }
        let t = &mut self.tasks[i];
        t.saved = Some(ExecState { ctx, pc, frames, if_nest: Vec::new(), loops: Vec::new() });
        t.wait_frames = 0;
        t.wait_until_ms = 0;
        t.text_wait = false;
        Ok(format!(
            "task {} moved to yst{sid:05} pc {pc}",
            String::from_utf8_lossy(&t.name)
        ))
    }

    /// TASKEND: ends the named task, or the running one.
    pub(crate) fn exec_taskend(&mut self, id: Option<&[u8]>) -> (String, Option<Option<VmSuspend>>) {
        let Some(i) = self.task_target(id) else {
            return ("TASKEND: no such task".into(), None);
        };
        let name = String::from_utf8_lossy(&self.tasks[i].name).into_owned();
        if self.tasks[i].name.is_empty() {
            return ("TASKEND on the main script ignored".into(), None);
        }
        if i == self.current_task {
            return (format!("task {name} ended itself"), Some(self.task_finished()));
        }
        self.remove_task(i);
        (format!("task {name} ended"), None)
    }

    /// TASKINFO answers: EXIST, E, A, Z, SCRIPTPOS, FILE/FILENO/LINE (of the
    /// task's position), FILE0/FILENO0/LINE0 (of its entry frame).
    /// TEXTLINE is the last text line the task finished; NEXTVOICE is 0.
    pub(crate) fn task_info(&self, id: Option<&[u8]>, slot: u8) -> Option<Value> {
        let Some(i) = self.task_target(id) else {
            return Some(match slot {
                6 | 9 | 19 | 20 => Value::Str(Vec::new()),
                _ => Value::Int(0),
            });
        };
        let t = &self.tasks[i];
        let (sid, pc, frames) = match &t.saved {
            Some(s) => (s.ctx.script_id, s.pc, &s.frames),
            None => (self.ctx.script_id, self.pc, &self.frames),
        };
        let base_sid = frames.first().map(|f| f.script_id);
        let file = |sid: u16| {
            self.script_list
                .as_deref()
                .and_then(|l| l.get(u32::from(sid)))
                .map(|e| e.source.clone())
                .unwrap_or_default()
        };
        Some(match slot {
            2 => Value::Int(1),
            3 => Value::Int(t.enabled as i64),
            4 => Value::Int(t.a),
            5 => Value::Int(t.z),
            6 => Value::Str(file(sid)),
            7 => Value::Int(i64::from(sid)),
            8 => Value::Int(pc as i64),
            9 => Value::Str(file(base_sid.unwrap_or(sid))),
            10 => Value::Int(i64::from(base_sid.unwrap_or(sid))),
            11 => Value::Int(0),
            12 => Value::Int(t.text_line),
            13 => Value::Int(0),
            19 => Value::Str(format_script_pos(sid, pc, frames)),
            _ => return None,
        })
    }

    /// The remapped context of a script, with its declarations and per-script
    /// YSVR values applied (what a switch to it does).
    pub(crate) fn prepared_script(&mut self, script_id: u16) -> Result<Arc<ScriptCtx>> {
        let ctx = self.load_ctx(script_id)?;
        if self.declared_scripts.insert(script_id) {
            crate::consume_declarations_into(&mut self.store, &ctx)?;
        }
        self.apply_ysvr_for_script(script_id)?;
        Ok(ctx)
    }
}

/// TINT/TFLT/TSTR into the entry frame's `@_PINT`/`@_PFLT`/`$_PSTR`
/// (1-based, like GOSUB).
fn seed_task_args(locals: &mut VariableStore, p: &TaskParams, ids: &yuris_core::system_vars::SystemVarIds) {
    use yuris_value::{ElemType, VarRef, VarSpace};
    let int = VarRef { space: VarSpace::At, id: ids.arg_int };
    let flt = VarRef { space: VarSpace::At, id: ids.arg_float };
    let s = VarRef { space: VarSpace::Dollar, id: ids.arg_str };
    locals.declare_array(&int, ElemType::Int, &[11]);
    locals.declare_array(&flt, ElemType::Float, &[11]);
    locals.declare_array(&s, ElemType::Str, &[11]);
    for (k, v) in &p.tint {
        let _ = locals.set_elem(&int, &[*k as i64], Value::Int(*v));
    }
    for (k, v) in &p.tflt {
        let _ = locals.set_elem(&flt, &[*k as i64], Value::Float(*v));
    }
    for (k, v) in &p.tstr {
        let _ = locals.set_elem(&s, &[*k as i64], Value::Str(v.clone()));
    }
}
