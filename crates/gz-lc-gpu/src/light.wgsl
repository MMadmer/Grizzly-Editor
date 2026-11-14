// The light of sample points: `gz-lc-light`'s `light_point` with `RayModel::trace` as an any-hit
// walk of the lighter's 4-wide tree, in two passes - `trace_main` a ray a thread, `add_main` a
// sample a thread. The formulas are the CPU's, in its order.

struct Params {
    // the samples, the mode bits (1 cull, 2 linear opacity), the falloff, the grid on
    a: vec4<u32>,
    // the static directional lights (first, count), the static point lights (first, count)
    b: vec4<u32>,
    // the sun's lights, the hemisphere's
    c: vec4<u32>,
    // the grid's cells along x, y, z, where its entries start
    d: vec4<u32>,
    e: vec4<f32>,
    f: vec4<f32>,
    // -, the most lights a sample has
    g: vec4<u32>,
}

struct Light {
    // position, kind (a whole number)
    a: vec4<f32>,
    // direction, range
    b: vec4<f32>,
    // colour, squared range
    c: vec4<f32>,
    // attenuation 0, 1, 2, energy
    d: vec4<f32>,
    // falloff, and a hemisphere light's `-direction * 0.001` as the CPU rounds it
    e: vec4<f32>,
}

struct Sample {
    p: vec3<f32>,
    flags: u32,
    n: vec3<f32>,
    skip: u32,
    // where its rays start, `p + n * 0.01` as the CPU rounds it
    o: vec3<f32>,
    pad: u32,
}

@group(0) @binding(0) var<uniform> prm: Params;
@group(0) @binding(1) var<storage, read> nodes: array<vec4<u32>>;
@group(0) @binding(2) var<storage, read> tris: array<vec4<u32>>;
@group(0) @binding(3) var<storage, read> transp: array<vec4<u32>>;
@group(0) @binding(4) var<storage, read> alpha: array<u32>;
@group(0) @binding(5) var<storage, read> lights: array<Light>;
@group(0) @binding(6) var<storage, read> grid: array<u32>;
@group(0) @binding(7) var<storage, read> samples: array<Sample>;
@group(0) @binding(8) var<storage, read_write> outp: array<f32>;
// what gets through along each ray of a batch, a sample's light `q` at `q * samples + sample`
@group(0) @binding(9) var<storage, read_write> through: array<f32>;

const NONE: u32 = 0xffffffffu;
const LEAF: u32 = 0x80000000u;
const OPAQUE: u32 = 0xffffffffu;
const LT_DIRECT: u32 = 0u;
const LT_POINT: u32 = 1u;
const LT_SECONDARY: u32 = 2u;
const LP_DONT_RGB: u32 = 2u;
const LP_DONT_HEMI: u32 = 4u;
const LP_DONT_SUN: u32 = 8u;
const FALLOFF_LEVEL: u32 = 0u;
const FALLOFF_LEVEL_LINEAR: u32 = 1u;
// a box's entry and exit widened by the rounding of the card's reciprocal, so that a hit on the
// very border of a box is never lost
const WIDEN: f32 = 4e-6;
const STACK: u32 = 64u;
const BIG: f32 = 3.0e38;

// four children's planes of one axis, a byte each
fn planes(word: u32) -> vec4<f32> {
    return vec4<f32>((vec4<u32>(word) >> vec4<u32>(0u, 8u, 16u, 24u)) & vec4<u32>(0xffu));
}

// where the ray enters each of four boxes before `tmax`, or a negative number
fn enter4(
    lx: vec4<f32>,
    hx: vec4<f32>,
    ly: vec4<f32>,
    hy: vec4<f32>,
    lz: vec4<f32>,
    hz: vec4<f32>,
    o: vec3<f32>,
    inv: vec3<f32>,
    still: vec3<bool>,
    tmax: f32,
) -> vec4<f32> {
    let tx0 = (lx - o.x) * inv.x;
    let tx1 = (hx - o.x) * inv.x;
    let ty0 = (ly - o.y) * inv.y;
    let ty1 = (hy - o.y) * inv.y;
    let tz0 = (lz - o.z) * inv.z;
    let tz1 = (hz - o.z) * inv.z;
    if !any(still) {
        let near = max(max(min(tx0, tx1), min(ty0, ty1)), min(tz0, tz1));
        let far = min(min(max(tx0, tx1), max(ty0, ty1)), max(tz0, tz1));
        let n = max(near, vec4<f32>(0.0)) * (1.0 - WIDEN);
        let f = min(far * (1.0 + WIDEN), vec4<f32>(tmax));
        return select(vec4<f32>(-1.0), n, (far >= vec4<f32>(0.0)) & (n <= f));
    }
    let lo = vec4<f32>(-BIG);
    let hi = vec4<f32>(BIG);
    let near = max(
        max(select(min(tx0, tx1), lo, still.x), select(min(ty0, ty1), lo, still.y)),
        select(min(tz0, tz1), lo, still.z),
    );
    let far = min(
        min(select(max(tx0, tx1), hi, still.x), select(max(ty0, ty1), hi, still.y)),
        select(max(tz0, tz1), hi, still.z),
    );
    // along a still axis the ray is inside the slab for every distance when its origin is, faces
    // included, and never otherwise
    let none = vec4<bool>(false);
    let outside = select(none, (vec4<f32>(o.x) < lx) | (vec4<f32>(o.x) > hx), still.x)
        | select(none, (vec4<f32>(o.y) < ly) | (vec4<f32>(o.y) > hy), still.y)
        | select(none, (vec4<f32>(o.z) < lz) | (vec4<f32>(o.z) > hz), still.z);
    let n = max(near, vec4<f32>(0.0)) * (1.0 - WIDEN);
    let f = min(far * (1.0 + WIDEN), vec4<f32>(tmax));
    let hit = !outside & (far >= vec4<f32>(0.0)) & (n <= f);
    return select(vec4<f32>(-1.0), n, hit);
}

// how much of a transparent face's light gets through where the ray met it
fn transparency(row: u32, u: f32, v: f32) -> f32 {
    let q0 = bitcast<vec4<f32>>(transp[row * 2u]);
    let w1 = transp[row * 2u + 1u];
    let q1 = bitcast<vec4<f32>>(w1);
    let slot = w1.z;
    let off = alpha[slot * 4u];
    let w = alpha[slot * 4u + 1u];
    let h = alpha[slot * 4u + 2u];
    let bw = 1.0 - u - v;
    let ux = q0.x * bw + q0.z * u + q1.x * v;
    let uy = q0.y * bw + q0.w * u + q1.y * v;
    // `U %= dwWidth` takes the int as unsigned: a negative U wraps through 2^32
    let iu = bitcast<u32>(i32(floor(ux * f32(w) + 0.5))) % w;
    let iv = bitcast<u32>(i32(floor(uy * f32(h) + 0.5))) % h;
    let t = off + iv * w + iu;
    let a = f32((alpha[t >> 2u] >> ((t & 3u) * 8u)) & 0xffu) / 255.0;
    if (prm.a.y & 2u) != 0u {
        return 1.0 - a;
    }
    return 1.0 - a * a;
}

// Products rounded on their own, as the CPU rounds them. The card fuses a product into the sum
// that follows it, and a fused `a * b - c * d` is not 0 where the two products are equal: a ray
// through a corner of a floor would slip past it. A product passed through an OR with a zero the
// compiler cannot see is never fused.
fn exact(a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    return bitcast<vec3<f32>>(bitcast<vec3<u32>>(a * b) | vec3<u32>(prm.a.y >> 16u));
}

fn exact1(a: f32, b: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(a * b) | (prm.a.y >> 16u));
}

fn exact_cross(a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    return exact(a.yzx, b.zxy) - exact(a.zxy, b.yzx);
}

// left to right, as the CPU sums
fn exact_dot(a: vec3<f32>, b: vec3<f32>) -> f32 {
    let q = exact(a, b);
    return (q.x + q.y) + q.z;
}

// the product of the transparencies of a leaf's triangles the ray meets; 0 at an opaque one
fn leaf(first: u32, count: u32, o: vec3<f32>, d: vec3<f32>, tmax: f32, skip: u32) -> f32 {
    var scale = 1.0;
    let cull = (prm.a.y & 1u) != 0u;
    for (var k = first; k < first + count; k++) {
        let a = tris[k * 3u];
        let b = tris[k * 3u + 1u];
        let c = tris[k * 3u + 2u];
        if skip != NONE && a.w == skip {
            continue;
        }
        // Moeller-Trumbore, as the CPU's tree tests a triangle
        let e1 = bitcast<vec3<f32>>(b.xyz);
        let e2 = bitcast<vec3<f32>>(c.xyz);
        let p = exact_cross(d, e2);
        let det = exact_dot(e1, p);
        if cull {
            if det < 1e-12 {
                continue;
            }
        } else if abs(det) < 1e-12 {
            continue;
        }
        let inv = 1.0 / det;
        let s = o - bitcast<vec3<f32>>(a.xyz);
        let u = exact1(exact_dot(s, p), inv);
        if u < 0.0 || u > 1.0 {
            continue;
        }
        let q = exact_cross(s, e1);
        let v = exact1(exact_dot(d, q), inv);
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let t = exact1(exact_dot(e2, q), inv);
        if t < 0.0 || t > tmax {
            continue;
        }
        let row = b.w;
        if row == OPAQUE {
            return 0.0;
        }
        scale *= transparency(row, u, v);
        if scale == 0.0 {
            return 0.0;
        }
    }
    return scale;
}

fn normalize_safe(v: vec3<f32>) -> vec3<f32> {
    let m = v.x * v.x + v.y * v.y + v.z * v.z;
    if m > 1.17549435e-38 {
        let k = sqrt(1.0 / m);
        return v * k;
    }
    return v;
}

fn att(l: Light, r: f32, sq_d: f32) -> f32 {
    return l.d.x + l.d.y * r + l.d.z * sq_d;
}

// a point light's distance squared, direction and cosine from `p`; a cosine of 0 out of range
struct Towards {
    sq_d: f32,
    ldir: vec3<f32>,
    d: f32,
}

fn towards(l: Light, p: vec3<f32>, n: vec3<f32>) -> Towards {
    let e = p - l.a.xyz;
    let sq_d = e.x * e.x + e.y * e.y + e.z * e.z;
    if sq_d > l.c.w {
        return Towards(sq_d, vec3<f32>(0.0), 0.0);
    }
    let ldir = normalize_safe(l.a.xyz - p);
    return Towards(sq_d, ldir, dot(ldir, n));
}

// The phases of a sample's lights, in `light_point`'s order.
const PHASE_RGB_DIRECT: u32 = 0u;
const PHASE_RGB_POINT: u32 = 1u;
const PHASE_SUN: u32 = 2u;
const PHASE_HEMI: u32 = 3u;

// What a sample's lights are: the point, where its rays start, and where each phase of its
// lights begins in its sequence of them (the static directional ones, the point lights that may
// reach it, the sun's, the hemisphere's).
struct Point {
    p: vec3<f32>,
    n: vec3<f32>,
    // the origins come rounded from the CPU: the card would fuse the multiply and the add, and an
    // origin an ulp away changes rays that graze an edge
    pnew: vec3<f32>,
    skip: u32,
    pt_first: u32,
    to_pt: u32,
    to_sun: u32,
    to_hemi: u32,
    total: u32,
}

fn point_of(i: u32) -> Point {
    let s = samples[i];
    var pt_first = prm.b.z;
    var pt_last = prm.b.z + prm.b.w;
    if prm.a.w != 0u {
        // the sample's cell of the grid
        pt_first = 0u;
        pt_last = 0u;
        let c = floor((s.p - prm.e.xyz) * prm.f.xyz);
        let dims = vec3<f32>(prm.d.xyz);
        if all(c >= vec3<f32>(0.0)) && all(c < dims) {
            let cu = vec3<u32>(c);
            let cell = (cu.z * prm.d.y + cu.y) * prm.d.x + cu.x;
            pt_first = grid[cell];
            pt_last = grid[cell + 1u];
        }
    }
    let no_rgb = (s.flags & LP_DONT_RGB) != 0u;
    let len_dir = select(prm.b.y, 0u, no_rgb);
    let len_pt = select(pt_last - pt_first, 0u, no_rgb);
    let len_sun = select(prm.c.y, 0u, (s.flags & LP_DONT_SUN) != 0u);
    let len_hemi = select(prm.c.w, 0u, (s.flags & LP_DONT_HEMI) != 0u);
    let to_sun = len_dir + len_pt;
    let to_hemi = to_sun + len_sun;
    return Point(s.p, s.n, s.o, s.skip, pt_first, len_dir, to_sun, to_hemi, to_hemi + len_hemi);
}

// What one light of a sample does: nothing, a light added without a ray, or a ray whose light is
// `coef` times what gets through.
const RAY_NONE: u32 = 0u;
const RAY_ADD: u32 = 1u;
const RAY_TRACE: u32 = 2u;

struct Ray {
    what: u32,
    o: vec3<f32>,
    d: vec3<f32>,
    tmax: f32,
    coef: vec3<f32>,
    // 0 the static light, 1 the sun, 2 the hemisphere
    channel: u32,
}

// The light at `q` of a sample's sequence, as `light_point` treats it.
fn ray_of(pt: Point, q: u32) -> Ray {
    var ray = Ray(RAY_NONE, pt.pnew, vec3<f32>(0.0, 1.0, 0.0), 0.0, vec3<f32>(0.0), 0u);
    var phase = PHASE_HEMI;
    var index = prm.c.z + (q - pt.to_hemi);
    if q < pt.to_pt {
        phase = PHASE_RGB_DIRECT;
        index = prm.b.x + q;
    } else if q < pt.to_sun {
        phase = PHASE_RGB_POINT;
        index = pt.pt_first + (q - pt.to_pt);
        if prm.a.w != 0u {
            index = grid[index];
        }
    } else if q < pt.to_hemi {
        phase = PHASE_SUN;
        index = prm.c.x + (q - pt.to_sun);
    }
    let l = lights[index];
    let kind = u32(l.a.w);
    if phase == PHASE_RGB_DIRECT || (phase >= PHASE_SUN && kind == LT_DIRECT) {
        let ldir = -l.b.xyz;
        let cosine = exact_dot(ldir, pt.n);
        if cosine <= 0.0 {
            return ray;
        }
        ray.what = RAY_TRACE;
        ray.d = ldir;
        ray.tmax = 1000.0;
        if phase == PHASE_RGB_DIRECT {
            ray.coef = cosine * l.d.w * l.c.xyz;
            ray.channel = 0u;
        } else if phase == PHASE_SUN {
            ray.coef = vec3<f32>(l.d.w);
            ray.channel = 1u;
        } else {
            ray.o = pt.pnew + l.e.yzw;
            ray.coef = vec3<f32>(l.d.w);
            ray.channel = 2u;
        }
        return ray;
    }
    let tw = towards(l, pt.p, pt.n);
    if tw.d <= 0.0 {
        return ray;
    }
    let r = sqrt(tw.sq_d);
    ray.d = tw.ldir;
    ray.tmax = r;
    ray.what = RAY_TRACE;
    if phase == PHASE_RGB_POINT {
        ray.channel = 0u;
        if kind == LT_POINT {
            let falloff = prm.a.z;
            if falloff == FALLOFF_LEVEL_LINEAR {
                // the light of `-att`: its shadow is not read
                ray.what = RAY_ADD;
                ray.coef = (1.0 - r / l.b.w) * l.c.xyz;
            } else if falloff == FALLOFF_LEVEL {
                // the attenuation, made 0 at the light's range
                ray.coef = tw.d * l.d.w * (1.0 / att(l, r, tw.sq_d) - r * l.e.x) * l.c.xyz;
            } else {
                ray.coef = tw.d * l.d.w / att(l, r, tw.sq_d) * l.c.xyz;
            }
        } else {
            // a secondary light
            let cosine = tw.d * -dot(tw.ldir, l.b.xyz);
            if cosine <= 0.0 {
                ray.what = RAY_NONE;
                return ray;
            }
            ray.coef = pow(cosine, 1.0 / 8.0) * l.d.w * (1.0 - r / l.b.w) * l.c.xyz;
        }
    } else {
        ray.coef = vec3<f32>(tw.d * l.d.w / att(l, r, tw.sq_d));
        ray.channel = select(2u, 1u, phase == PHASE_SUN);
    }
    return ray;
}

// How much light gets from `o` along `d` (of unit length) over `tmax`: the 4-wide tree walked
// nearest first, the first opaque face ending it, the transparent ones multiplied.
fn trace(o: vec3<f32>, d: vec3<f32>, tmax: f32, skip: u32) -> f32 {
    // an axis the ray does not move along takes no reciprocal: 0 * inf would be NaN on a box face
    let still = abs(d) < vec3<f32>(1e-30);
    let inv = 1.0 / select(d, vec3<f32>(1.0), still);
    var stack: array<u32, STACK>;
    var sp = 0u;
    var node = 0u;
    var scale = 1.0;
    loop {
        let base = node * 4u;
        let w0 = nodes[base];
        let w1 = nodes[base + 1u];
        let w2 = nodes[base + 2u];
        let kid = nodes[base + 3u];
        // the planes: the grid's corner plus whole steps of a power of two, as the CPU rounds them
        let corner = bitcast<vec3<f32>>(w0.xyz);
        let sx = bitcast<f32>((w0.w & 0xffu) << 23u);
        let sy = bitcast<f32>(((w0.w >> 8u) & 0xffu) << 23u);
        let sz = bitcast<f32>(((w0.w >> 16u) & 0xffu) << 23u);
        let dist = enter4(
            corner.x + planes(w1.x) * sx,
            corner.x + planes(w1.y) * sx,
            corner.y + planes(w1.z) * sy,
            corner.y + planes(w1.w) * sy,
            corner.z + planes(w2.x) * sz,
            corner.z + planes(w2.y) * sz,
            o,
            inv,
            still,
            tmax,
        );
        // the lanes by hand: a vector indexed in a loop may live in memory, not in registers
        let hit = (dist >= vec4<f32>(0.0)) & (kid != vec4<u32>(NONE));
        let is_leaf = (kid & vec4<u32>(LEAF)) != vec4<u32>(0u);
        let leaves = hit & is_leaf;
        let first = kid & vec4<u32>(0x0fffffffu);
        let count = ((kid >> vec4<u32>(28u)) & vec4<u32>(7u)) + vec4<u32>(1u);
        if leaves.x {
            scale *= leaf(first.x, count.x, o, d, tmax, skip);
        }
        if leaves.y && scale != 0.0 {
            scale *= leaf(first.y, count.y, o, d, tmax, skip);
        }
        if leaves.z && scale != 0.0 {
            scale *= leaf(first.z, count.z, o, d, tmax, skip);
        }
        if leaves.w && scale != 0.0 {
            scale *= leaf(first.w, count.w, o, d, tmax, skip);
        }
        if scale == 0.0 {
            return 0.0;
        }
        let inner = hit & !is_leaf;
        var d0 = select(BIG, dist.x, inner.x);
        var d1 = select(BIG, dist.y, inner.y);
        var d2 = select(BIG, dist.z, inner.z);
        var d3 = select(BIG, dist.w, inner.w);
        var n0 = select(NONE, kid.x, inner.x);
        var n1 = select(NONE, kid.y, inner.y);
        var n2 = select(NONE, kid.z, inner.z);
        var n3 = select(NONE, kid.w, inner.w);
        // a sorting network of four: nearest first, the missed ones (BIG) last
        if d1 < d0 {
            let t = d0; d0 = d1; d1 = t;
            let m = n0; n0 = n1; n1 = m;
        }
        if d3 < d2 {
            let t = d2; d2 = d3; d3 = t;
            let m = n2; n2 = n3; n3 = m;
        }
        if d2 < d0 {
            let t = d0; d0 = d2; d2 = t;
            let m = n0; n0 = n2; n2 = m;
        }
        if d3 < d1 {
            let t = d1; d1 = d3; d3 = t;
            let m = n1; n1 = n3; n3 = m;
        }
        if d2 < d1 {
            let t = d1; d1 = d2; d2 = t;
            let m = n1; n1 = n2; n2 = m;
        }
        if n0 != NONE {
            node = n0;
            // the farther ones wait, the next nearest on top
            if n3 != NONE {
                stack[sp] = n3;
                sp = min(sp + 1u, STACK - 1u);
            }
            if n2 != NONE {
                stack[sp] = n2;
                sp = min(sp + 1u, STACK - 1u);
            }
            if n1 != NONE {
                stack[sp] = n1;
                sp = min(sp + 1u, STACK - 1u);
            }
        } else if sp == 0u {
            break;
        } else {
            sp -= 1u;
            node = stack[sp];
        }
    }
    return scale;
}

// One ray a thread: `x` the sample, `y` its light's place in the sample's sequence. The threads
// of a group are neighbouring samples lighting the same light, so their walks read the same
// nodes. A static light keeps what gets through, for the pass that adds the lights up to weigh
// with its colour; a light of the sun or the hemisphere keeps its whole light, 0 for none.
@compute @workgroup_size(64)
fn trace_main(@builtin(workgroup_id) wid: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let count = prm.a.x;
    let i = wid.y * 64u + lane;
    let q = wid.x;
    if i >= count {
        return;
    }
    let pt = point_of(i);
    if q >= pt.total {
        return;
    }
    let ray = ray_of(pt, q);
    if q < pt.to_sun {
        if ray.what == RAY_TRACE {
            through[q * count + i] = trace(ray.o, ray.d, ray.tmax, pt.skip);
        }
        return;
    }
    var light = 0.0;
    if ray.what == RAY_TRACE {
        light = ray.coef.x * trace(ray.o, ray.d, ray.tmax, pt.skip);
    }
    through[q * count + i] = light;
}

// One sample a thread: its lights added up in `light_point`'s order (a light that sends no ray
// adds an exact 0).
@compute @workgroup_size(64)
fn add_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let count = prm.a.x;
    let i = gid.x;
    if i >= count {
        return;
    }
    let pt = point_of(i);
    var rgb = vec3<f32>(0.0);
    var sun = 0.0;
    var hemi = 0.0;
    for (var q = 0u; q < pt.to_sun; q++) {
        let ray = ray_of(pt, q);
        if ray.what == RAY_TRACE {
            rgb += ray.coef * through[q * count + i];
        } else if ray.what == RAY_ADD {
            rgb += ray.coef;
        }
    }
    for (var q = pt.to_sun; q < pt.to_hemi; q++) {
        sun += through[q * count + i];
    }
    for (var q = pt.to_hemi; q < pt.total; q++) {
        hemi += through[q * count + i];
    }
    let at = i * 5u;
    outp[at] = rgb.x;
    outp[at + 1u] = rgb.y;
    outp[at + 2u] = rgb.z;
    outp[at + 3u] = hemi;
    outp[at + 4u] = sun;
}
