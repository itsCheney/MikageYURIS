//! Native commands: the commands that act on the engine's host side (CG
//! layers, fonts, sound, windows, mouse) rather than on script state.
//!
//! The VM evaluates a native command's parameters, names them with the
//! canonical YSCM parameter names ([`crate::commands::CANONICAL_PARAMS`]) and
//! hands them to a [`Native`] implementation. The `LET` parameter of the
//! `*INFO` commands is a variable reference: the VM keeps it and stores the
//! value the host returns. `yuris-host` holds the real implementation; the
//! VM's default, [`NullNative`], answers nothing.

use std::any::Any;

use yuris_core::Result;
use yuris_value::Value;

use crate::cmd;

/// Canonical indices of the commands routed to [`Native`].
pub const NATIVE_COMMANDS: &[u8] = &[
    cmd::CG,
    cmd::CGACT,
    cmd::CGEND,
    cmd::CGINFO,
    0x1a, // FONT
    0x1b, // FONTINFO
    cmd::SOUND,
    0x5a, // SOUNDEND
    0x5b, // SOUNDINFO
    0x6a, // WINDOWEND
    0x6b, // WINDOWINFO
    0x45, // MOUSE
];

/// Whether `command` (canonical index) goes to the native host.
pub fn is_native(command: u8) -> bool {
    NATIVE_COMMANDS.contains(&command)
}

/// One evaluated parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeArg {
    /// Canonical parameter slot.
    pub slot: u8,
    /// Canonical parameter name (`"?"` for a slot the canonical table lacks).
    pub param: &'static str,
    /// The evaluated value.
    pub value: Value,
    /// Assignment operator from the window tag, as in LET: 0 `=`, 1 `+=`,
    /// 2 `-=`, ... (Erewhon fades with `CG[ID=.. A-=8]`). See
    /// [`NativeArg::apply`].
    pub op: u8,
}

impl NativeArg {
    /// The parameter's new value for a current value `cur`: the value
    /// itself for `=`, otherwise `cur` combined with it.
    pub fn apply(&self, cur: i64) -> i64 {
        let n = match &self.value {
            Value::Int(n) => *n,
            Value::Float(f) => *f as i64,
            Value::Str(_) => 0,
        };
        if self.op == 0 {
            return n;
        }
        match yuris_value::compound_assign(self.op, &Value::Int(cur), &Value::Int(n)) {
            Ok(Value::Int(v)) => v,
            _ => n,
        }
    }
}

/// A native command call, as handed to [`Native::call`].
#[derive(Debug, Clone)]
pub struct NativeCall<'a> {
    /// Canonical command index.
    pub command: u8,
    /// Canonical command name.
    pub name: &'static str,
    /// Evaluated parameters, in script order. `LET` is not among them.
    pub args: &'a [NativeArg],
    /// Whether the call has a `LET` target (the `*INFO` commands).
    pub has_let: bool,
    /// Cursor position in screen pixels (@133/@138).
    pub cursor: (i64, i64),
    /// Engine clock in ms (@1[1]).
    pub clock_ms: i64,
}

impl NativeCall<'_> {
    /// The value of parameter `param`, if given.
    pub fn get(&self, param: &str) -> Option<&Value> {
        self.args.iter().rev().find(|a| a.param == param).map(|a| &a.value)
    }

    /// Whether parameter `param` was given at all.
    pub fn has(&self, param: &str) -> bool {
        self.get(param).is_some()
    }

    /// Parameter `param` as an integer (floats truncate).
    pub fn int(&self, param: &str) -> Option<i64> {
        match self.get(param)? {
            Value::Int(n) => Some(*n),
            Value::Float(f) => Some(*f as i64),
            Value::Str(_) => None,
        }
    }

    /// Parameter `param` as a float.
    pub fn float(&self, param: &str) -> Option<f64> {
        match self.get(param)? {
            Value::Int(n) => Some(*n as f64),
            Value::Float(f) => Some(*f),
            Value::Str(_) => None,
        }
    }

    /// Parameter `param` as raw string bytes (the game's encoding).
    pub fn bytes(&self, param: &str) -> Option<&[u8]> {
        match self.get(param)? {
            Value::Str(b) => Some(b),
            _ => None,
        }
    }

    /// A switch parameter (`COPY=1`): given and non-zero.
    pub fn flag(&self, param: &str) -> bool {
        self.int(param).is_some_and(|n| n != 0)
    }

    /// The first switch parameter of `params` that is set, in that order.
    pub fn first_flag(&self, params: &[&'static str]) -> Option<&'static str> {
        params.iter().copied().find(|p| self.flag(p))
    }
}

/// The host side of the native commands.
pub trait Native: Any {
    /// Runs one native command. For a call with a `LET` target, the
    /// returned value is stored there; `None` leaves the variable alone.
    fn call(&mut self, call: &NativeCall<'_>) -> Result<Option<Value>>;

    /// `SAVE ID=`: the layer's image in the host's own encoding, or `None`
    /// when there is no such layer.
    fn save_layer(&mut self, _id: &[u8]) -> Option<Vec<u8>> {
        None
    }

    /// `LOAD ID=`: puts an image from [`Native::save_layer`] into the
    /// layer, creating it if needed. Returns whether it was understood.
    fn load_layer(&mut self, _id: &[u8], _data: &[u8]) -> bool {
        false
    }

    /// For downcasting to the concrete host.
    fn as_any(&self) -> &dyn Any;

    /// For downcasting to the concrete host.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// A host that does nothing and answers nothing.
#[derive(Debug, Default)]
pub struct NullNative;

impl Native for NullNative {
    fn call(&mut self, _call: &NativeCall<'_>) -> Result<Option<Value>> {
        Ok(None)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
