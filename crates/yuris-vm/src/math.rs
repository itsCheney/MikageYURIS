//! MATH: `MATH [LET=@A POW=1 SET=5 SET2=3]` and the other functions of
//! the manual (math_math): one flag picks the function, SET and SET2 are
//! its arguments, LET receives the result.
//!
//! Angles are radians. POW and ABS of integers stay integers (Erewhon
//! builds its bit table `@1157[n] = 2^n` up to 2^62 with POW); the other
//! functions give floats. A number stored into an INT variable is
//! truncated. RND gives an integer in [SET, SET2].

use yuris_value::{ElemType, Value, VarRef};

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

/// MATH3D: `MATH3D [INT=@i() FLT=@f()]`, the sprite projection behind
/// ERIS's es.SP.XYZCALC. Not in the manual; read from the v481 engine's
/// handler (command 62 of its handler table). Inputs are elements 1-7 of
/// the INT array and 3-30 of the FLT array, outputs FLT 31-34.
///
/// INT: 1 the sprite, 2 its parent, 3 a third node, 4 a camera node (0 =
/// none), 5 and 6 the screen size, 7 nonzero for a flat (2D) sprite.
/// FLT: 3-5 the third node's x, y, z; 6-8 the parent's position; 9-10
/// and 11-12 two parent offsets; 13-15 the sprite's position; 16-17 its
/// zoom in percent; 18-19 its anchor; 20-21 its scale in percent; 22-24
/// the camera node's position and 25-26 its anchor; 27 the focal depth;
/// 28-30 the scene camera's offset.
///
/// Depth works in powers of two: a node at depth z is drawn at
/// 2^((focal - z + camera z) / 100) times its size, so every 100 units
/// toward the viewer double it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Math3dOut {
    pub x: f64,
    pub y: f64,
    pub zoom_x: f64,
    pub zoom_y: f64,
}

/// The MATH3D formula. `i[k]` / `f[k]` are the array elements (missing
/// ones are 0).
pub fn math3d(i: &[i64; 8], f: &[f64; 31]) -> Math3dOut {
    if i[7] != 0 {
        // Flat sprite: position minus its anchor scaled by its zoom.
        return Math3dOut {
            x: f[13] - f[18] * f[16] / 100.0,
            y: f[14] - f[19] * f[17] / 100.0,
            zoom_x: f[16],
            zoom_y: f[17],
        };
    }
    let depth = |z: f64, cam_z: f64| 2f64.powf((f[27] - z + cam_z) / 100.0);
    let (cx, cy, cz) = (f[3] + f[28], f[4] + f[29], f[5] + f[30]);
    let (sx, sy) = (f[20] / 100.0, f[21] / 100.0);
    let parent = depth(f[8], cz);
    let mut s = depth(f[15], cz);
    let mut x = i[5] as f64 * 0.5 + f[11] * sx + parent * ((f[6] - f[9]) - sx * cx);
    let mut y = i[6] as f64 * 0.5 + f[12] * sy + parent * ((f[7] - f[10]) - sy * cy);
    if i[1] != i[2] {
        x += parent * f[9];
        y += parent * f[10];
        if i[4] == 0 {
            x += s * (f[13] * sx - f[18]);
            y += s * (f[14] * sy - f[19]);
        } else {
            s = depth(f[24], cz);
            x += s * ((f[13] + f[22]) * sx - f[18] - f[25]);
            y += s * ((f[14] + f[23]) * sy - f[19] - f[26]);
        }
    }
    Math3dOut {
        x: x + (100.0 - f[16]) * f[18] * s / 100.0,
        y: y + (100.0 - f[17]) * f[19] * s / 100.0,
        zoom_x: f[16] * s,
        zoom_y: f[17] * s,
    }
}

impl GroupVm {
    /// The array a MATH3D window names (`@3231()`).
    fn math3d_array(&self, w: &yuris_format::ystb::CommandSlot) -> Result<VarRef> {
        let bytes = self
            .ctx
            .script
            .window_bytes_pooled_copy(w)
            .ok_or_else(|| Error::format("MATH3D window past the pool end"))?;
        match extract_var_target(&bytes)? {
            VarTarget::Scalar(r) | VarTarget::Indexed(r, _) => Ok(r),
        }
    }

    /// Runs MATH3D (see [`math3d`]).
    pub(crate) fn exec_math3d(
        &mut self,
        windows: &[yuris_format::ystb::CommandSlot],
    ) -> Result<()> {
        let mut int_ref = None;
        let mut flt_ref = None;
        for w in windows {
            if w.len == 0 {
                continue;
            }
            match (w.tag & 0xff) as u8 {
                0 => int_ref = Some(self.math3d_array(w)?),
                1 => flt_ref = Some(self.math3d_array(w)?),
                _ => {}
            }
        }
        let (Some(ir), Some(fr)) = (int_ref, flt_ref) else {
            return Err(Error::format("MATH3D needs INT and FLT arrays"));
        };
        let mut i = [0i64; 8];
        for (k, v) in i.iter_mut().enumerate().skip(1) {
            *v = match self.read_var_value(&ir, &[k as i64]) {
                Some(Value::Int(n)) => n,
                Some(Value::Float(x)) => x as i64,
                _ => 0,
            };
        }
        let mut f = [0f64; 31];
        for (k, v) in f.iter_mut().enumerate().skip(3) {
            *v = match self.read_var_value(&fr, &[k as i64]) {
                Some(v) => num(&v),
                None => 0.0,
            };
        }
        let out = math3d(&i, &f);
        for (k, v) in [(31, out.x), (32, out.y), (33, out.zoom_x), (34, out.zoom_y)] {
            self.write_var_value(&fr, &[k], Value::Float(v))?;
        }
        let mut evaluated = Vec::new();
        if self.trace {
            evaluated.push((255, format!("{out:?}")));
        }
        self.events.push(VmEvent::Subsystem { pc: self.pc, command: cmd::MATH3D, evaluated });
        Ok(())
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

    #[test]
    fn math3d_flat_and_depth() {
        let mut i = [0i64; 8];
        let mut f = [0f64; 31];
        // Flat: anchor scaled by zoom.
        i[7] = 1;
        f[13] = 500.0;
        f[14] = 300.0;
        f[16] = 50.0;
        f[17] = 50.0;
        f[18] = 200.0;
        f[19] = 100.0;
        let o = math3d(&i, &f);
        assert_eq!((o.x, o.y, o.zoom_x), (400.0, 250.0, 50.0));
        // 3D: a sprite 100 units nearer than the focal depth is twice as big.
        i = [0, 2, 1, 0, 0, 1152, 720, 0];
        f = [0f64; 31];
        f[16] = 100.0;
        f[17] = 100.0;
        f[20] = 100.0;
        f[21] = 100.0;
        f[27] = 200.0;
        f[8] = 200.0;
        f[15] = 100.0;
        let o = math3d(&i, &f);
        assert_eq!((o.x, o.y, o.zoom_x), (576.0, 360.0, 200.0));
    }
}
