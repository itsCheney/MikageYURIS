//! MATH: `MATH [LET=@A POW=1 SET=5 SET2=3]` and the other functions of
//! the manual (math_math): one flag picks the function, SET and SET2 are
//! its arguments, LET receives the result.
//!
//! Angles are radians. POW and ABS of integers stay integers (Erewhon
//! builds its bit table `@1157[n] = 2^n` up to 2^62 with POW); the other
//! functions give floats. A number stored into an INT variable is
//! truncated. RND gives an integer in [SET, SET2].

use yuris_value::{ElemType, Value};

use crate::{cmd, extract_var_target, value_summary, Error, GroupVm, Result, VarTarget, VmEvent};

/// Canonical MATH slots (YSCM order).
const LET: u8 = 0;
const SET: u8 = 1;
const SET2: u8 = 2;
const FIRST_FUNCTION: u8 = 4;
const NAMES: [&str; 19] = [
    "SIN", "ASIN", "SINH", "COS", "ACOS", "COSH", "TAN", "ATAN", "ATAN2", "TANH", "SQRT", "ABS",
    "EXP", "LOG", "LOG10", "POW", "RND", "HASH", "CALC",
];

fn num(v: &Value) -> f64 {
    match v {
        Value::Int(n) => *n as f64,
        Value::Float(f) => *f,
        _ => 0.0,
    }
}

impl GroupVm {
    fn math_random(&mut self, lo: i64, hi: i64) -> i64 {
        // xorshift64*; the engine's generator is not known.
        let mut x = self.math_rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.math_rng = x;
        let r = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
        let span = (hi as i128 - lo as i128 + 1) as u128;
        (lo as i128 + (r as u128 % span) as i128) as i64
    }

    /// Whether the variable behind `t` holds integers.
    fn target_is_int(&self, t: &VarTarget) -> bool {
        let r = match t {
            VarTarget::Scalar(r) | VarTarget::Indexed(r, _) => r,
        };
        match self.store.array_elem_type(r) {
            Some(e) => e == ElemType::Int,
            None => !matches!(self.store.get_opt(r), Some(Value::Float(_))),
        }
    }

    /// Runs MATH. Returns the reason when the function is not supported.
    pub(crate) fn exec_math(
        &mut self,
        windows: &[yuris_format::ystb::CommandSlot],
    ) -> Result<Option<String>> {
        let mut func: Option<u8> = None;
        let mut a = Value::Int(0);
        let mut b = Value::Int(0);
        let mut let_ref = None;
        let mut evaluated: Vec<(u8, String)> = Vec::new();
        for w in windows {
            if w.len == 0 {
                continue;
            }
            let slot = (w.tag & 0xff) as u8;
            if slot == LET {
                let_ref = Some(w.clone());
                continue;
            }
            let Some(v) = self.eval_window_condition(Some(w))? else { continue };
            if self.trace {
                evaluated.push((slot, value_summary(&v)));
            }
            match slot {
                SET => a = v,
                SET2 => b = v,
                s if s >= FIRST_FUNCTION && matches!(v, Value::Int(n) if n != 0) => func = Some(s),
                _ => {}
            }
        }
        let name = func.and_then(|f| NAMES.get(usize::from(f - FIRST_FUNCTION)).copied());
        let (x, y) = (num(&a), num(&b));
        let ints = matches!((&a, &b), (Value::Int(_), Value::Int(_)));
        let value = match name {
            Some("SIN") => Value::Float(x.sin()),
            Some("ASIN") => Value::Float(x.asin()),
            Some("SINH") => Value::Float(x.sinh()),
            Some("COS") => Value::Float(x.cos()),
            Some("ACOS") => Value::Float(x.acos()),
            Some("COSH") => Value::Float(x.cosh()),
            Some("TAN") => Value::Float(x.tan()),
            Some("ATAN") => Value::Float(x.atan()),
            Some("ATAN2") => Value::Float(x.atan2(y)),
            Some("TANH") => Value::Float(x.tanh()),
            Some("SQRT") => Value::Float(x.sqrt()),
            Some("ABS") => match a {
                Value::Int(n) => Value::Int(n.wrapping_abs()),
                _ => Value::Float(x.abs()),
            },
            Some("EXP") => Value::Float(x.exp()),
            Some("LOG") => Value::Float(x.ln()),
            Some("LOG10") => Value::Float(x.log10()),
            Some("POW") => match (&a, &b) {
                (Value::Int(n), Value::Int(e)) if ints && (0..64).contains(e) => {
                    Value::Int(n.wrapping_pow(*e as u32))
                }
                _ => Value::Float(x.powf(y)),
            },
            Some("RND") => {
                let (lo, hi) = (x as i64, y as i64);
                Value::Int(self.math_random(lo, hi))
            }
            _ => {
                let what = name.unwrap_or("no function");
                let reason =
                    format!("MATH {what} is not implemented s{} pc={}", self.ctx.script_id, self.pc);
                self.events.push(VmEvent::Unsupported {
                    pc: self.pc,
                    command: cmd::MATH,
                    reason: reason.clone(),
                });
                return Ok(Some(reason));
            }
        };
        let Some(w) = let_ref else {
            return Ok(None);
        };
        let bytes = self
            .ctx
            .script
            .window_bytes_pooled_copy(&w)
            .ok_or_else(|| Error::format("MATH LET window past the pool end"))?;
        let target = extract_var_target(&bytes)?;
        let value = match value {
            Value::Float(f) if self.target_is_int(&target) => Value::Int(f as i64),
            Value::Int(n) if !self.target_is_int(&target) => Value::Float(n as f64),
            v => v,
        };
        if self.trace {
            evaluated.push((255, format!("{} -> {}", value_summary(&value), target.summary())));
        }
        self.write_var_target(&target, value)?;
        self.events.push(VmEvent::Subsystem { pc: self.pc, command: cmd::MATH, evaluated });
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_names_follow_the_slots() {
        // Slot 19 is POW in the canonical MATH parameters.
        assert_eq!(NAMES[usize::from(19 - FIRST_FUNCTION)], "POW");
        assert_eq!(NAMES[usize::from(20 - FIRST_FUNCTION)], "RND");
    }
}
