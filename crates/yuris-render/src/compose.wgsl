// Layer quads. Image layers are straight alpha (premul = 0) and composed
// sets are premultiplied (premul = 1); either is drawn premultiplied and
// scaled by the layer's opacity k, as in yuris_host::compose. A quad in a
// rule transition (rule.y > 0) also scales by its coverage: rule.x is the
// progress and rule.y the edge width, both in rule levels.

struct VOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
    @location(1) duv: vec2f,
    @location(2) k: f32,
    @location(3) premul: f32,
    @location(4) mode: f32,
    @location(5) ruv: vec2f,
    @location(6) rule: vec2f,
}

@vertex
fn vs(
    @location(0) pos: vec2f,
    @location(1) uv: vec2f,
    @location(2) duv: vec2f,
    @location(3) k: f32,
    @location(4) premul: f32,
    @location(5) mode: f32,
    @location(6) ruv: vec2f,
    @location(7) rule: vec2f,
) -> VOut {
    var o: VOut;
    o.pos = vec4f(pos, 0.0, 1.0);
    o.uv = uv;
    o.duv = duv;
    o.k = k;
    o.premul = premul;
    o.mode = mode;
    o.ruv = ruv;
    o.rule = rule;
    return o;
}

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_smp: sampler;
// The destination copy (fs_dst) and the rule image; 1x1 stand-ins when
// unused.
@group(1) @binding(0) var dst_tex: texture_2d<f32>;
@group(2) @binding(0) var rule_tex: texture_2d<f32>;

fn source(v: VOut) -> vec4f {
    let c = textureSample(src_tex, src_smp, v.uv);
    var p = c;
    if v.premul < 0.5 {
        p = vec4f(c.rgb * c.a, c.a);
    }
    // Nearest texel, as the CPU compositor reads the rule.
    let size = vec2f(textureDimensions(rule_tex));
    let texel = vec2i(clamp(floor(v.ruv * size), vec2f(0.0), size - 1.0));
    let level = round(textureLoad(rule_tex, texel, 0).r * 255.0);
    var cover = 1.0;
    if v.rule.y > 0.0 {
        cover = clamp((v.rule.x - level) / v.rule.y, 0.0, 1.0);
    }
    return p * (v.k * cover);
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
