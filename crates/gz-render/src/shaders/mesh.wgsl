// The editor's materials. Appended to common.wgsl.

struct Material {
    color: vec4<f32>,
    // alpha reference, detail scale, 1 when there is a detail texture,
    // 1 when the base texture is a cube map + 2 when the detail one is
    params: vec4<f32>,
};

@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_tex: texture_2d<f32>;
@group(1) @binding(2) var detail_tex: texture_2d<f32>;
@group(1) @binding(3) var samp: sampler;
@group(1) @binding(4) var base_cube: texture_cube<f32>;
@group(1) @binding(5) var detail_cube: texture_cube<f32>;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) uv2: vec2<f32>,
    @location(4) color: vec4<f32>,
    @location(5) bones: vec4<u32>,
    @location(6) weights: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) @interpolate(flat) inst: u32,
};

fn skin_matrix(at: u32, v: VsIn) -> mat4x4<f32> {
    let w = v.weights;
    let total = w.x + w.y + w.z + w.w;
    if (total <= 0.0) {
        return bones[at + v.bones.x];
    }
    var m = bones[at + v.bones.x] * w.x;
    m = m + bones[at + v.bones.y] * w.y;
    m = m + bones[at + v.bones.z] * w.z;
    m = m + bones[at + v.bones.w] * w.w;
    return m * (1.0 / total);
}

@vertex
fn vs_main(v: VsIn, @builtin(instance_index) ii: u32) -> VsOut {
    let inst = instances[ii];
    var p = vec4<f32>(v.position, 1.0);
    var n = v.normal;
    if (inst.bones_at != NO_BONES) {
        let m = skin_matrix(inst.bones_at, v);
        p = m * p;
        n = (m * vec4<f32>(n, 0.0)).xyz;
    }
    let world = inst.model * p;
    let nm = mat3x3<f32>(inst.n0.xyz, inst.n1.xyz, inst.n2.xyz);
    var out: VsOut;
    out.clip = frame.view_proj * world;
    // a wallmark lies on its wall: a little nearer wins the depth test (depth is reversed)
    if ((inst.flags & INST_DECAL) != 0u) {
        out.clip.z = out.clip.z * 1.0005;
    }
    out.world = world.xyz;
    out.normal = nm * n;
    out.uv = v.uv;
    out.color = v.color * inst.color;
    out.inst = ii;
    return out;
}

// The sky follows the camera and lies on the far plane: it shows only where nothing was drawn.
@vertex
fn vs_sky(v: VsIn, @builtin(instance_index) ii: u32) -> VsOut {
    let inst = instances[ii];
    let rot = mat3x3<f32>(inst.model[0].xyz, inst.model[1].xyz, inst.model[2].xyz);
    let world = frame.camera_pos.xyz + rot * v.position;
    var out: VsOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.clip.z = 0.0;
    out.world = world;
    // the cube-map direction: the sky box's own coordinates (they do not turn with it, as in
    // dxEnvironmentRender), else the point's own direction
    out.normal = select(v.position, v.normal, (inst.flags & INST_SKY_TC) != 0u);
    out.uv = v.uv;
    out.color = v.color * inst.color;
    out.inst = ii;
    return out;
}

fn safe_normalize(v: vec3<f32>) -> vec3<f32> {
    let l = length(v);
    if (l < 1e-8) {
        return vec3<f32>(0.0, 1.0, 0.0);
    }
    return v / l;
}

// "Studio": a hemisphere ambient, a headlight along the view and a key from above.
fn studio(n: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    var to_eye = frame.camera_pos.xyz - world;
    if ((frame.mode.z & 4u) != 0u) {
        to_eye = -frame.camera_dir.xyz;
    }
    let e = safe_normalize(to_eye);
    let head = max(dot(n, e), 0.0);
    let hemi = mix(vec3<f32>(0.30, 0.29, 0.27), vec3<f32>(0.62, 0.64, 0.68), n.y * 0.5 + 0.5);
    let key = max(dot(n, normalize(vec3<f32>(-0.35, 0.85, -0.4))), 0.0);
    return hemi * 0.55 + vec3<f32>(head * 0.45 + key * 0.22);
}

// EGameLighting's glit_light: the weather's ambient, hemisphere and sun, the scene's lights with
// the game's falloff, a roof map telling indoors from outdoors, the tonemap's exposure.
fn game_light(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var outside = 1.0;
    if ((frame.mode.z & 1u) != 0u && frame.roof.w > 0.0) {
        let cell = vec2<i32>(floor((p.xz - frame.roof.xy) * frame.roof.z));
        let cells = i32(frame.roof.w);
        if (all(cell >= vec2<i32>(0)) && all(cell < vec2<i32>(cells))) {
            outside = select(0.0, 1.0, p.y > textureLoad(roof_map, cell, 0).r - 0.3);
        }
    }
    var l = frame.ambient.rgb;
    l = l + frame.hemi.rgb * (n.y * 0.5 + 0.5) * mix(frame.hemi.w, 1.0, outside);
    l = l + frame.sun_color.rgb * max(0.0, dot(n, -frame.sun_dir.xyz)) * outside;
    let count = frame.mode.y;
    for (var i = 0u; i < count; i = i + 1u) {
        let lp = lights[i].pos_range;
        let d = lp.xyz - p;
        let r2 = dot(d, d);
        var att = clamp(1.0 - r2 / max(lp.w * lp.w, 1e-6), 0.0, 1.0);
        if (att <= 0.0) {
            continue;
        }
        let lc = lights[i].color_cos;
        let dir = d * inverseSqrt(max(r2, 1e-6));
        if (lc.w > 0.0) {
            att = att * clamp((dot(-dir, lights[i].dir.xyz) - lc.w) / max(1.0 - lc.w, 1e-3) * 4.0, 0.0, 1.0);
        }
        l = l + lc.rgb * max(0.0, dot(n, dir)) * att;
    }
    return l * frame.ambient.w;
}

// calc_ffp_lighting of the C++ editor: the directional lights enabled for this point - the sun
// always (dir.w 2), a scene light within its range (dir.w 1, SelectLightsForObject) - over no
// ambient; with none of them the surface as it is.
fn editor_light(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var l = vec3<f32>(0.0);
    var enabled = 0u;
    let count = frame.mode.y;
    for (var i = 0u; i < count; i = i + 1u) {
        let lp = lights[i].pos_range;
        if (lights[i].dir.w < 1.5 && distance(p, lp.xyz) > lp.w) {
            continue;
        }
        enabled = enabled + 1u;
        l = l + lights[i].color_cos.rgb * max(0.0, dot(n, -lights[i].dir.xyz));
    }
    if (enabled == 0u) {
        return vec3<f32>(1.0);
    }
    return l;
}

fn shade(world: vec3<f32>, normal: vec3<f32>, front: bool) -> vec3<f32> {
    let mode = frame.mode.x;
    if (mode == MODE_UNLIT || mode == MODE_WIRE) {
        return vec3<f32>(1.0);
    }
    var n = safe_normalize(normal);
    // a two-sided surface seen from its back is lit on the side the camera sees
    if (!front) {
        n = -n;
    }
    if (mode == MODE_GAME) {
        return game_light(world, n);
    }
    if ((frame.mode.z & 8u) != 0u) {
        return editor_light(world, n);
    }
    return studio(n, world);
}

fn fogged(c: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    if (frame.mode.x != MODE_GAME || frame.fog.y <= frame.fog.x) {
        return c;
    }
    let d = distance(world, frame.camera_pos.xyz);
    let k = clamp((d - frame.fog.x) / (frame.fog.y - frame.fog.x), 0.0, 1.0);
    return mix(c, frame.fog_color.rgb, k);
}

fn surface(in: VsOut) -> vec4<f32> {
    return textureSample(base_tex, samp, in.uv) * material.color * in.color;
}

@fragment
fn fs_lit(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let c = surface(in);
    return vec4<f32>(fogged(c.rgb * shade(in.world, in.normal, front), in.world), 1.0);
}

@fragment
fn fs_alpha(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let c = surface(in);
    if (c.a < material.params.x) {
        discard;
    }
    return vec4<f32>(fogged(c.rgb * shade(in.world, in.normal, front), in.world), 1.0);
}

@fragment
fn fs_blend(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let c = surface(in);
    return vec4<f32>(fogged(c.rgb * shade(in.world, in.normal, front), in.world), c.a);
}

// D3DBLEND_ONE, D3DBLEND_ONE: the colour is added as it is, its alpha does nothing
@fragment
fn fs_add(in: VsOut) -> @location(0) vec4<f32> {
    let c = surface(in);
    return vec4<f32>(c.rgb, 0.0);
}

// D3DBLEND_DESTCOLOR with ZERO or SRCCOLOR: the blend multiplies what is behind by the colour
@fragment
fn fs_mul(in: VsOut) -> @location(0) vec4<f32> {
    let c = surface(in);
    return vec4<f32>(c.rgb, 1.0);
}

@fragment
fn fs_selflit(in: VsOut) -> @location(0) vec4<f32> {
    let c = surface(in);
    return vec4<f32>(fogged(c.rgb, in.world), 1.0);
}

@fragment
fn fs_terrain(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let c = surface(in);
    let d = textureSample(detail_tex, samp, in.uv * material.params.y);
    let k = select(vec3<f32>(1.0), d.rgb * 2.0, material.params.z > 0.5);
    return vec4<f32>(fogged(c.rgb * k * shade(in.world, in.normal, front), in.world), 1.0);
}

@fragment
fn fs_water(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let c = surface(in);
    return vec4<f32>(fogged(c.rgb * shade(in.world, in.normal, front), in.world), c.a * 0.75);
}

// sky2.ps: the factor's colour times the two cube maps blended by its alpha; a flat sky texture
// is drawn as it is
@fragment
fn fs_sky(in: VsOut) -> @location(0) vec4<f32> {
    let cubes = u32(material.params.w);
    if ((cubes & 1u) == 0u) {
        return vec4<f32>(surface(in).rgb, 1.0);
    }
    let dir = safe_normalize(in.normal);
    let s0 = textureSample(base_cube, samp, dir).rgb;
    var s1 = s0;
    if ((cubes & 2u) != 0u) {
        s1 = textureSample(detail_cube, samp, dir).rgb;
    }
    return vec4<f32>(material.color.rgb * in.color.rgb * mix(s0, s1, in.color.a), 1.0);
}

@fragment
fn fs_particle(in: VsOut) -> @location(0) vec4<f32> {
    return surface(in);
}

@fragment
fn fs_unlit(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(surface(in).rgb, 1.0);
}

@fragment
fn fs_wire(in: VsOut) -> @location(0) vec4<f32> {
    let inst = instances[in.inst];
    if (frame.mode.x != MODE_WIRE) {
        return vec4<f32>((material.color * in.color).rgb, 1.0);
    }
    let selected = (inst.flags & INST_SELECTED) != 0u && frame.mode.w != 0u;
    return vec4<f32>(select(frame.wire.rgb, frame.selection.rgb, selected), 1.0);
}

// an instance flagged NO_ID still hides what is behind it, as nothing
fn id_of(inst: u32) -> u32 {
    let i = instances[inst];
    if ((i.flags & INST_NO_ID) != 0u) {
        return 0u;
    }
    return i.id | select(0u, SELECTED_BIT, (i.flags & INST_SELECTED) != 0u);
}

@fragment
fn fs_id(in: VsOut) -> @location(0) u32 {
    return id_of(in.inst);
}

@fragment
fn fs_id_alpha(in: VsOut) -> @location(0) u32 {
    let c = surface(in);
    if (c.a < material.params.x) {
        discard;
    }
    return id_of(in.inst);
}

// what is seen through does not hide what is behind it from the outline
@fragment
fn fs_id_blend(in: VsOut) -> @location(0) u32 {
    let c = surface(in);
    let id = id_of(in.inst);
    if (c.a < material.params.x || (id & SELECTED_BIT) == 0u) {
        discard;
    }
    return id;
}
