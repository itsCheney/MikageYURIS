//! The engine's input variables: mouse buttons, keys, `@_INP` and the
//! `@_CODE_*` button codes.
//!
//! System variable ids follow the order of the system variable table in
//! the v480 compiler dictionary, starting at id 10 (`@_TIME`). Erewhon
//! (v481) uses the same ids: `$_TEXT` = 122, `$_CHR` = 127, `@_INP` = 144,
//! `@_MOUSE_L` = 147, `@_CODE_MOUSE_L` = 415.
//!
//! From the manual (system variables): a button's state variable holds the
//! number of frames it has been held (1, 2, 3, ...) while it is down, and
//! the number of frames since its release as a negative number (-1, -2,
//! ...) while it is up. `@_MOUSENUM_*` / `@_KEYNUM_*` count the presses.
//! `@_INP(code)` is the same state, looked up by a button code from the
//! `@_CODE_*` variables (scripts keep key configs as codes).
//!
//! Pad codes are 384 + 36 * pad + n (n: 0-3 up/down/left/right, then
//! 4 + button): Erewhon's es._strtocode writes them as literals
//! (`PAD2_2` = 384 + 36 + 5). The mouse and key codes are not known;
//! ours are 1-20 and 32-139. Scripts keep key configs as names and
//! convert them through the `@_CODE_*` variables, so these are safe.

use yuris_value::{ElemType, Value, VarRef, VarSpace};

use crate::{GroupVm, Result};

/// `$_SYSTEM_PATH`, `$_PROJECT_PATH`, `$_SAVE_PATH`, `$_APPDATA_PATH`,
/// `$_PERSONAL_PATH`, `@_SAVE_PATH_TYPE`.
const SYSTEM_PATH: u16 = 103;
const PROJECT_PATH: u16 = 104;
const SAVE_PATH: u16 = 105;
const APPDATA_PATH: u16 = 106;
const PERSONAL_PATH: u16 = 107;
const SAVE_PATH_TYPE: u16 = 112;
pub(crate) const MOUSE_X: u16 = 133;
pub(crate) const MOUSE_Y: u16 = 138;
/// `@_INP[1024]`.
pub(crate) const INP: u16 = 144;
/// `@_MOUSE_L`; R, M, U and D follow, four ids apart (`L1`-`L3` sit in
/// between; their meaning is not documented, they stay 0).
const MOUSE_STATE: u16 = 147;
/// `@_KEY_A`, the first of the key states (`@_KEY_ANY` = 275 is last).
const KEY_STATE: u16 = 167;
const KEY_ANY: u16 = 275;
/// `@_MOUSENUM_L`, laid out like the states.
const MOUSE_COUNT: u16 = 281;
/// `@_KEYNUM_A`, in code order (see [`Key`]); `@_KEYNUM_ANY` = 409.
const KEY_COUNT: u16 = 301;
const KEY_COUNT_ANY: u16 = 409;
/// `@_CODE_MOUSE_L`, laid out like the states.
const MOUSE_CODE: u16 = 415;
/// `@_CODE_A`, in code order.
const KEY_CODE: u16 = 435;
/// `@_CODE_PAD_UP[16]`, DOWN, LEFT, RIGHT, then `@_CODE_PAD_BUTTON[16, 32]`.
const PAD_CODE: u16 = 543;

const MOUSE_CODE_BASE: i64 = 1;
const KEY_CODE_BASE: i64 = 32;
const PAD_CODE_BASE: i64 = 384;
const INP_LEN: u32 = 1024;

/// Mouse buttons, in the engine's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    WheelUp,
    WheelDown,
}

/// Keys, in the order of the `@_CODE_*` and `@_KEYNUM_*` variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Key {
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    D0, D1, D2, D3, D4, D5, D6, D7, D8, D9,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    F13, F14, F15, F16, F17, F18, F19, F20, F21, F22, F23, F24,
    Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9,
    NumAdd, NumSub, NumMul, NumDiv, NumDec,
    Backspace, Tab, Enter, Space, Shift, Ctrl, Alt,
    Left, Right, Up, Down, Insert, Delete, Help, Scroll, NumLock, Escape,
    PageUp, PageDown, Home, End, Hyphen,
    Circumflex, Yen, At, LeftBracket, Semicolon, Colon, RightBracket,
    Comma, Period, Slash, Backslash,
}

const KEY_COUNT_ALL: usize = Key::Backslash as usize + 1;

impl Key {
    /// Offset of the key's `@_KEY_*` state. That list has HYPHEN before
    /// HOME and END; the code and count lists have it after them.
    fn state_offset(i: usize) -> usize {
        const HOME: usize = Key::Home as usize;
        const END: usize = Key::End as usize;
        const HYPHEN: usize = Key::Hyphen as usize;
        match i {
            HOME => HOME + 1,
            END => END + 1,
            HYPHEN => HOME,
            _ => i,
        }
    }
}

/// Physical button states the host reports each frame.
#[derive(Debug, Clone)]
pub(crate) struct InputState {
    /// Whether the YSVR declares `@_INP[1024]`, i.e. the table above
    /// applies to this game.
    pub enabled: bool,
    pub mouse: [bool; 5],
    pub keys: [bool; KEY_COUNT_ALL],
}

impl Default for InputState {
    fn default() -> Self {
        Self { enabled: false, mouse: [false; 5], keys: [false; KEY_COUNT_ALL] }
    }
}

fn at(id: u16) -> VarRef {
    VarRef { space: VarSpace::At, id }
}

/// The next frame's value of a state variable (see the module doc).
/// A value the script wrote is counted on from.
fn next_state(prev: i64, down: bool) -> i64 {
    match (down, prev > 0) {
        (true, true) => prev.saturating_add(1),
        (true, false) => 1,
        (false, true) => -1,
        (false, false) => prev.min(0).saturating_sub(1).max(-1_000_000),
    }
}

impl GroupVm {
    /// Sets up the input variables if the YSVR declares `@_INP[1024]`:
    /// the button codes, and every state at "released".
    pub(crate) fn init_input(&mut self, ysvr: &yuris_format::ysvr::YsvrTable) -> Result<()> {
        self.input.enabled = ysvr
            .entries()
            .iter()
            .any(|e| e.var_id == INP && e.ty == 1 && e.bounds == [INP_LEN]);
        if !self.input.enabled {
            return Ok(());
        }
        if !self.store.has_array(&at(INP)) {
            self.store.declare_array(&at(INP), ElemType::Int, &[INP_LEN]);
        }
        for b in 0..5u16 {
            self.set_global_int(MOUSE_CODE + 4 * b, MOUSE_CODE_BASE + 4 * b as i64)?;
            // L1-L3 etc. get codes too, so no two variables share one.
            for k in 1..4u16 {
                self.set_global_int(MOUSE_CODE + 4 * b + k, MOUSE_CODE_BASE + (4 * b + k) as i64)?;
            }
        }
        for i in 0..KEY_COUNT_ALL as u16 {
            self.set_global_int(KEY_CODE + i, KEY_CODE_BASE + i as i64)?;
        }
        for dir in 0..4u16 {
            let r = at(PAD_CODE + dir);
            if !self.store.has_array(&r) {
                self.store.declare_array(&r, ElemType::Int, &[16]);
            }
            for pad in 0..16i64 {
                let code = PAD_CODE_BASE + pad * 36 + dir as i64;
                self.store.set_elem(&r, &[pad], Value::Int(code))?;
            }
        }
        let r = at(PAD_CODE + 4);
        if !self.store.has_array(&r) {
            self.store.declare_array(&r, ElemType::Int, &[16, 32]);
        }
        for pad in 0..16i64 {
            for b in 0..32i64 {
                let code = PAD_CODE_BASE + pad * 36 + 4 + b;
                self.store.set_elem(&r, &[pad, b], Value::Int(code))?;
            }
        }
        // The path variables of the same table. The engine puts absolute
        // folders here; scripts build save file names from `$_SAVE_PATH`
        // (Erewhon checks `FILEINFO EXIST FILE=$_SAVE_PATH+"config.sd"`
        // before loading), so it is `save\`, which the game file system
        // maps to the save folder. `@_SAVE_PATH_TYPE` 0 keeps saves there.
        let path = |id: u16| VarRef { space: VarSpace::Dollar, id };
        for id in [SYSTEM_PATH, PROJECT_PATH, APPDATA_PATH, PERSONAL_PATH] {
            self.store.set(&path(id), Value::Str(Vec::new()));
        }
        self.store.set(&path(SAVE_PATH), Value::Str(b"save\\".to_vec()));
        self.store.set(&at(SAVE_PATH_TYPE), Value::Int(0));
        Ok(())
    }

    /// Reports a mouse button as down or up (applied at the next frame).
    pub fn set_mouse_button(&mut self, b: MouseButton, down: bool) {
        self.input.mouse[b as usize] = down;
    }

    /// Reports a key as down or up (applied at the next frame).
    pub fn set_key(&mut self, k: Key, down: bool) {
        self.input.keys[k as usize] = down;
    }

    fn step_button(&mut self, state: u16, count: u16, code: i64, down: bool) -> Result<()> {
        let prev = self.global_int(state).unwrap_or(0);
        let next = next_state(prev, down);
        self.set_global_int(state, next)?;
        if next == 1 {
            let n = self.global_int(count).unwrap_or(0);
            self.set_global_int(count, n.saturating_add(1))?;
        }
        if (0..INP_LEN as i64).contains(&code) {
            self.store.set_elem(&at(INP), &[code], Value::Int(next))?;
        }
        Ok(())
    }

    /// The engine's per-frame input update, for the host to call before
    /// each frame: steps every button state from the reported buttons.
    pub fn update_input(&mut self) -> Result<()> {
        if !self.input.enabled {
            return Ok(());
        }
        for b in 0..5u16 {
            let down = self.input.mouse[b as usize];
            self.step_button(
                MOUSE_STATE + 4 * b,
                MOUSE_COUNT + 4 * b,
                MOUSE_CODE_BASE + 4 * b as i64,
                down,
            )?;
        }
        // Wheel "buttons" are one-frame events.
        self.input.mouse[MouseButton::WheelUp as usize] = false;
        self.input.mouse[MouseButton::WheelDown as usize] = false;
        for i in 0..KEY_COUNT_ALL {
            let down = self.input.keys[i];
            self.step_button(
                KEY_STATE + Key::state_offset(i) as u16,
                KEY_COUNT + i as u16,
                KEY_CODE_BASE + i as i64,
                down,
            )?;
        }
        let any = self.input.keys.iter().any(|&d| d);
        let prev = self.global_int(KEY_ANY).unwrap_or(0);
        let next = next_state(prev, any);
        self.set_global_int(KEY_ANY, next)?;
        if next == 1 {
            let n = self.global_int(KEY_COUNT_ANY).unwrap_or(0);
            self.set_global_int(KEY_COUNT_ANY, n.saturating_add(1))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_count_frames() {
        let mut v = 0;
        let seq: Vec<i64> = [true, true, true, false, false, true]
            .iter()
            .map(|&d| {
                v = next_state(v, d);
                v
            })
            .collect();
        assert_eq!(seq, [1, 2, 3, -1, -2, 1]);
        // A value the script wrote to consume a click is counted on.
        assert_eq!(next_state(5, true), 6);
        assert_eq!(next_state(5, false), -1);
    }

    #[test]
    fn key_state_order() {
        assert_eq!(KEY_STATE as usize + Key::state_offset(Key::Hyphen as usize), 261);
        assert_eq!(KEY_STATE as usize + Key::state_offset(Key::Home as usize), 262);
        assert_eq!(KEY_STATE as usize + Key::state_offset(Key::End as usize), 263);
        assert_eq!(KEY_STATE as usize + Key::state_offset(Key::Enter as usize), 244);
        assert_eq!(KEY_STATE as usize + KEY_COUNT_ALL, KEY_ANY as usize);
        assert_eq!(KEY_COUNT as usize + KEY_COUNT_ALL, KEY_COUNT_ANY as usize);
        assert_eq!(KEY_CODE as usize + KEY_COUNT_ALL, PAD_CODE as usize);
    }
}

/// `@_TIME(2,8)`. Scripts address it as `@0` (and `@_OS_TIME` as `@1`).
const TIME: u16 = 0;

impl GroupVm {
    /// Sets `@_TIME`, the local date and time: year, month, day, weekday
    /// (0 = Sunday), hour, minute, second, ms. From the manual, row 0 is
    /// now and row 1 the engine start; row 1 is filled on the first call.
    /// Erewhon shows row 0 as the save date of a slot.
    pub fn set_date_time(&mut self, now: [i64; 8]) -> Result<()> {
        let r = at(TIME);
        let first = !self.store.has_array(&r);
        if first {
            self.store.declare_array(&r, ElemType::Int, &[2, 8]);
        }
        for (i, v) in now.iter().enumerate() {
            self.store.set_elem(&r, &[0, i as i64], Value::Int(*v))?;
            if first {
                self.store.set_elem(&r, &[1, i as i64], Value::Int(*v))?;
            }
        }
        Ok(())
    }
}
