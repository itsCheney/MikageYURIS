//! Scenario text: WORD, `_` and RETURNCODE.
//!
//! The compiler turns each text line of a scenario into WORD groups (the
//! raw text, window tag 0), `_` groups (`_[expr]`, the value printed as
//! text) and a closing RETURNCODE whose window tag is the line's number.
//!
//! Model (from Erewhon's ERIS text task, yst00042 `es.TextTask`; the
//! engine's handlers are not reversed yet). In manual output mode
//! (`TEXT OUTPUTMODE=0`) the engine draws nothing itself. It keeps two
//! system strings that the script reads:
//! - `$_TEXT` (v481 id 122): the characters of the text so far;
//! - `$_TEXT_FLAG` (v481 id 123): one flag per unit: `1` a single-byte
//!   character, `2` a double-byte one, `E` the end of a line (RETURNCODE)
//!   and `Z` the end of the text (see below). The text task also knows
//!   `c`/`C` (click), `R` (new line) and `P` (new page), which the `\c`,
//!   `\r`, `\p` macros and its own auto settings produce.
//!
//! When the task that wrote text goes on to any other command, the engine
//! adds `Z` and stops that task. The text task shows the characters one by
//! one, waits for its click, and at `Z` runs `TASK RESTART=1`, which lets
//! the stopped task go on. Both strings start empty again then: the text
//! task resets its read positions to 1 just before.
//!
//! Between lines (a RETURNCODE followed by more text) the writing task
//! gives up the frame. The text task takes the text one line per cycle:
//! its size table routine (es.TX.BUFSET.SIZE, run twice per cycle) only
//! terminates if no `E` sits before the last character it has not sized
//! yet, i.e. if it never sees a later line in the same cycle. It pauses
//! the writing task (`TASK E=0`) while it shows a line.

use yuris_value::{Value, VarRef, VarSpace};

use crate::{cmd, GroupVm, VmEvent, VmSuspend};

/// `$_TEXT` in v481.
pub(crate) const TEXT_VAR: u16 = 122;
/// `$_TEXT_FLAG` in v481.
pub(crate) const TEXT_FLAG_VAR: u16 = 123;
/// A STR[256] the engine fills with the 256 one-byte strings: scripts use
/// `$127[13]+$127[10]` for CR LF, `$127[34]` for `"`, and the scenario
/// markers `$127[239]+$127[240]` (new line) and `$127[239]+$127[243]`
/// (click) that the text task turns into `R` and `C`.
pub(crate) const CHAR_TABLE_VAR: u16 = 127;

/// How the game's strings are encoded: decides which bytes start a
/// double-byte character (string lengths, VARACT positions, text flags).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Codepage {
    /// CP932. Lead bytes 0x81-0x9F and 0xE0-0xEF, the engine's own table.
    #[default]
    Sjis,
    /// GBK (Chinese patches run the engine on a GBK system code page).
    /// Lead bytes 0x81-0xFE.
    Gbk,
}

impl Codepage {
    /// Whether `b` starts a double-byte character.
    pub fn is_lead(self, b: u8) -> bool {
        match self {
            Codepage::Sjis => (0x81..=0x9F).contains(&b) || (0xE0..=0xEF).contains(&b),
            Codepage::Gbk => (0x81..=0xFE).contains(&b),
        }
    }

    /// Byte length of the character starting at `bytes[0]`.
    pub fn char_len(self, bytes: &[u8]) -> usize {
        match bytes.first() {
            Some(&b) if self.is_lead(b) && bytes.len() > 1 => 2,
            Some(_) => 1,
            None => 0,
        }
    }

    /// Number of characters in `bytes`.
    pub fn count_chars(self, bytes: &[u8]) -> usize {
        let mut off = 0;
        let mut n = 0;
        while off < bytes.len() {
            off += self.char_len(&bytes[off..]);
            n += 1;
        }
        n
    }
}

/// Text output state.
#[derive(Debug, Clone)]
pub(crate) struct TextState {
    /// `TEXT OUTPUTMODE`: true = automatic (the engine's default), false =
    /// manual (the script draws the text).
    pub auto: bool,
    /// The task (creation number) whose text is still open, if any.
    pub owner: Option<u64>,
    /// The open text's characters and flags, for the event.
    pub text: Vec<u8>,
    pub flags: Vec<u8>,
    /// The open text's last unit is a line end.
    pub line_ended: bool,
}

impl Default for TextState {
    fn default() -> Self {
        Self { auto: true, owner: None, text: Vec::new(), flags: Vec::new(), line_ended: false }
    }
}

/// Commands that continue the open text instead of closing it.
pub(crate) fn continues_text(command: u8) -> bool {
    matches!(command, cmd::WORD | cmd::PRINT | cmd::RETURNCODE | cmd::LABEL)
}

fn str_var(id: u16) -> VarRef {
    VarRef { space: VarSpace::Dollar, id }
}

impl GroupVm {
    /// Sets the code page of the game's strings.
    pub fn set_codepage(&mut self, cp: Codepage) {
        self.codepage = cp;
    }

    /// The code page of the game's strings.
    pub fn codepage(&self) -> Codepage {
        self.codepage
    }

    /// Fills the one-byte character table if the YSVR declares it.
    pub(crate) fn fill_char_table(&mut self, ysvr: &yuris_format::ysvr::YsvrTable) {
        let declared = ysvr
            .entries()
            .iter()
            .any(|e| e.var_id == CHAR_TABLE_VAR && e.ty == 3 && e.bounds == [256]);
        if !declared {
            return;
        }
        let r = str_var(CHAR_TABLE_VAR);
        for i in 1..256i64 {
            let _ = self.store.set_elem(&r, &[i], Value::Str(vec![i as u8]));
        }
    }

    /// Whether a task is stopped at the end of its text, waiting for the
    /// text task to let it go on (i.e. the game waits for a click).
    pub fn text_waiting(&self) -> bool {
        self.tasks.iter().any(|t| t.text_wait)
    }

    /// Creation number of the running task (0 without tasks).
    fn running_seq(&self) -> u64 {
        self.tasks.get(self.current_task).map(|t| t.seq).unwrap_or(0)
    }

    fn append_str_var(&mut self, id: u16, bytes: &[u8]) {
        let r = str_var(id);
        let mut v = match self.store.get_opt(&r) {
            Some(Value::Str(s)) => s.clone(),
            _ => Vec::new(),
        };
        v.extend_from_slice(bytes);
        self.store.set(&r, Value::Str(v));
    }

    /// Text from WORD or `_`: the characters go to `$_TEXT`, a `1` or `2`
    /// per character to `$_TEXT_FLAG`.
    pub(crate) fn text_append(&mut self, bytes: &[u8]) {
        let mut flags = Vec::new();
        let mut off = 0;
        while off < bytes.len() {
            let n = self.codepage.char_len(&bytes[off..]);
            flags.push(if n == 2 { b'2' } else { b'1' });
            off += n;
        }
        self.append_str_var(TEXT_VAR, bytes);
        self.append_str_var(TEXT_FLAG_VAR, &flags);
        self.text.text.extend_from_slice(bytes);
        self.text.flags.extend_from_slice(&flags);
        self.text.owner = Some(self.running_seq());
        self.text.line_ended = false;
    }

    /// RETURNCODE: the end of a line. `line` is its number, which TASKINFO
    /// TEXTLINE reports.
    pub(crate) fn text_line_end(&mut self, line: i64) {
        self.append_str_var(TEXT_FLAG_VAR, b"E");
        self.text.flags.push(b'E');
        self.text.owner = Some(self.running_seq());
        self.text.line_ended = true;
        if let Some(t) = self.tasks.get_mut(self.current_task) {
            t.text_line = line;
        }
    }

    /// Called before every group: when the running task has open text and
    /// goes on to another command, the text ends here. In manual mode the
    /// task then stops until `TASK RESTART=1`. Returns Some(outcome) when
    /// the task stopped (None inside = another task runs now).
    pub(crate) fn text_close_before(&mut self, command: u8) -> Option<Option<VmSuspend>> {
        let owner = self.text.owner?;
        if owner != self.running_seq() {
            return None;
        }
        if continues_text(command) {
            // The next line starts: let the text task take this one.
            let next_line = matches!(command, cmd::WORD | cmd::PRINT);
            if next_line && self.text.line_ended && !self.text.auto && self.has_tasks() {
                self.text.line_ended = false;
                return Some(self.task_wait(Some(1), None));
            }
            return None;
        }
        self.text.owner = None;
        self.text.line_ended = false;
        let text = std::mem::take(&mut self.text.text);
        let mut flags = std::mem::take(&mut self.text.flags);
        // Without tasks nothing could let the script go on.
        let stop = !self.text.auto && self.has_tasks();
        if stop {
            self.append_str_var(TEXT_FLAG_VAR, b"Z");
            flags.push(b'Z');
        }
        self.events.push(VmEvent::TextOut {
            pc: self.pc,
            script_id: self.ctx.script_id,
            text,
            flags,
        });
        if !stop {
            return None;
        }
        self.tasks[self.current_task].text_wait = true;
        Some(self.end_turn())
    }

    /// `TASK RESTART=1` (no ID, no label): the tasks stopped at the end of
    /// their text go on, and the text strings start empty.
    pub(crate) fn text_restart(&mut self) -> usize {
        let mut n = 0;
        for t in &mut self.tasks {
            if t.text_wait {
                t.text_wait = false;
                n += 1;
            }
        }
        self.store.set(&str_var(TEXT_VAR), Value::Str(Vec::new()));
        self.store.set(&str_var(TEXT_FLAG_VAR), Value::Str(Vec::new()));
        n
    }

    /// `TEXT CLEAR=1`: empties the text strings.
    pub(crate) fn text_clear(&mut self) {
        self.store.set(&str_var(TEXT_VAR), Value::Str(Vec::new()));
        self.store.set(&str_var(TEXT_FLAG_VAR), Value::Str(Vec::new()));
        self.text.text.clear();
        self.text.flags.clear();
        self.text.line_ended = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_pages_split_characters() {
        // GBK 「你好」 + "a". The SJIS table reads 0xC4 and 0xC3 as
        // single-byte kana and pairs 0xE3 with the next byte.
        let s = [0xC4, 0xE3, 0xBA, 0xC3, b'a'];
        assert_eq!(Codepage::Gbk.count_chars(&s), 3);
        assert_eq!(Codepage::Sjis.count_chars(&s), 4);
        // SJIS 「あ」 = 82 A0.
        assert_eq!(Codepage::Sjis.count_chars(&[0x82, 0xA0, b'x']), 2);
    }
}
