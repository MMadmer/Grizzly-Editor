// Helper lines and triangles, label occlusion quads, the selection outline. Appended to
// common.wgsl.

struct LineIn {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_line(v: LineIn) -> LineOut {
    var out: LineOut;
    out.clip = frame.view_proj * vec4<f32>(v.position, 1.0);
    out.color = v.color;
    return out;
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    return in.color;
}

// A label's anchor as a quad of 3 x 3 pixels, drawn inside an occlusion query.
@vertex
fn vs_label(@location(0) position: vec3<f32>, @builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = frame.view_proj * vec4<f32>(position, 1.0);
    let o = corners[vi % 6u] * 1.5 * frame.viewport.zw * 2.0 * c.w;
    return vec4<f32>(c.xy + o, c.z, c.w);
}

@fragment
fn fs_label() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}

@group(1) @binding(0) var id_tex: texture_2d<u32>;

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

// A band two pixels wide round what is selected and visible, a light wash over it.
@fragment
fn fs_outline(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(id_tex));
    let p = vec2<i32>(pos.xy);
    let here = textureLoad(id_tex, p, 0).r;
    if ((here & SELECTED_BIT) != 0u) {
        return vec4<f32>(frame.selection.rgb, 0.12);
    }
    var hit = false;
    for (var dy = -2; dy <= 2; dy = dy + 1) {
        for (var dx = -2; dx <= 2; dx = dx + 1) {
            let q = clamp(p + vec2<i32>(dx, dy), vec2<i32>(0), dims - vec2<i32>(1));
            if ((textureLoad(id_tex, q, 0).r & SELECTED_BIT) != 0u) {
                hit = true;
            }
        }
    }
    if (!hit) {
        discard;
    }
    return vec4<f32>(frame.selection.rgb, 1.0);
}
