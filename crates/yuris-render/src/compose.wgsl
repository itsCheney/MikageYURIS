// Layer quads. Image layers are straight alpha (premul = 0) and composed
// sets are premultiplied (premul = 1); either is drawn premultiplied and
// scaled by the layer's opacity k, as in yuris_host::compose.

struct VOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
    @location(1) duv: vec2f,
    @location(2) k: f32,
    @location(3) premul: f32,
    @location(4) mode: f32,
}

@vertex
fn vs(
    @location(0) pos: vec2f,
    @location(1) uv: vec2f,
    @location(2) duv: vec2f,
    @location(3) k: f32,
    @location(4) premul: f32,
    @location(5) mode: f32,
) -> VOut {
    var o: VOut;
    o.pos = vec4f(pos, 0.0, 1.0);
    o.uv = uv;
    o.duv = duv;
    o.k = k;
    o.premul = premul;
    o.mode = mode;
    return o;
}

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_smp: sampler;

fn source(v: VOut) -> vec4f {
    let c = textureSample(src_tex, src_smp, v.uv);
    var p = c;
    if v.premul < 0.5 {
        p = vec4f(c.rgb * c.a, c.a);
    }
    return p * v.k;
}

// Modes the fixed blend states do (normal, add, subtract, screen), and
// the final copy to the screen.
@fragment
fn fs(v: VOut) -> @location(0) vec4f {
    return source(v);
}

// Modes that need the destination (multiply, lighten, darken,
// difference): a copy of the target is read and the result replaces the
// pixels under the quad.
@group(1) @binding(0) var dst_tex: texture_2d<f32>;

@fragment
fn fs_dst(v: VOut) -> @location(0) vec4f {
    let s = source(v);
    let d = textureSample(dst_tex, src_smp, v.duv);
    let rest = s.rgb * (1.0 - d.a) + d.rgb * (1.0 - s.a);
    let m = i32(v.mode + 0.5);
    var c = s.rgb + d.rgb * (1.0 - s.a);
    if m == 3 {
        c = s.rgb * d.rgb + rest;
    } else if m == 6 {
        c = max(s.rgb * d.a, d.rgb * s.a) + rest;
    } else if m == 7 {
        c = min(s.rgb * d.a, d.rgb * s.a) + rest;
    } else if m == 8 {
        c = s.rgb + d.rgb - 2.0 * min(s.rgb * d.a, d.rgb * s.a);
    }
    let a = s.a + d.a * (1.0 - s.a);
    return clamp(vec4f(c, a), vec4f(0.0), vec4f(1.0));
}
