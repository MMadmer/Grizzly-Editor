// What every pass of a frame reads: the frame block, the instances, the poses, the lights.

struct Frame {
    view_proj: mat4x4<f32>,
    view: mat4x4<f32>,
    camera_pos: vec4<f32>,
    camera_dir: vec4<f32>,
    // width, height, 1 / width, 1 / height
    viewport: vec4<f32>,
    // x: view mode (0 lit, 1 unlit, 2 wireframe, 3 game lighting), y: lights,
    // z: flags (1 roof map, 4 orthographic), w: 1 when the selection is shown
    mode: vec4<u32>,
    // rgb, w = exposure
    ambient: vec4<f32>,
    // rgb, w = the share of the hemisphere indoors
    hemi: vec4<f32>,
    // from the sun
    sun_dir: vec4<f32>,
    sun_color: vec4<f32>,
    fog_color: vec4<f32>,
    // near, far
    fog: vec4<f32>,
    // x0, z0, 1 / cell, cells
    roof: vec4<f32>,
    selection: vec4<f32>,
    wire: vec4<f32>,
};

struct Instance {
    model: mat4x4<f32>,
    // the normal matrix, column by column
    n0: vec4<f32>,
    n1: vec4<f32>,
    n2: vec4<f32>,
    color: vec4<f32>,
    id: u32,
    flags: u32,
    bones_at: u32,
    pad: u32,
};

struct Light {
    // position, range
    pos_range: vec4<f32>,
    // colour, cos of a spot's half angle (0: a point light)
    color_cos: vec4<f32>,
    // a spot's axis
    dir: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
@group(0) @binding(2) var<storage, read> bones: array<mat4x4<f32>>;
@group(0) @binding(3) var<storage, read> lights: array<Light>;
@group(0) @binding(4) var roof_map: texture_2d<f32>;

const MODE_UNLIT: u32 = 1u;
const MODE_WIRE: u32 = 2u;
const MODE_GAME: u32 = 3u;
// InstanceFlags
const INST_SELECTED: u32 = 1u;
const INST_NO_ID: u32 = 16u;
const SELECTED_BIT: u32 = 0x80000000u;
const NO_BONES: u32 = 0xffffffffu;
