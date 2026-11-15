//! The glTF 2.0 JSON document: the parts this reader looks at. Unknown keys are passed over.

use std::collections::BTreeMap;

use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Doc {
    pub asset: Option<Asset>,
    pub scene: Option<usize>,
    pub scenes: Vec<Scene>,
    pub nodes: Vec<Node>,
    pub meshes: Vec<GMesh>,
    pub accessors: Vec<Accessor>,
    pub buffer_views: Vec<BufferView>,
    pub buffers: Vec<Buffer>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub images: Vec<Image>,
    pub skins: Vec<Skin>,
    pub animations: Vec<Animation>,
    pub extensions_required: Vec<String>,
    pub extensions_used: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Asset {
    pub version: String,
    pub min_version: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Scene {
    pub nodes: Vec<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Node {
    pub name: Option<String>,
    pub children: Vec<usize>,
    pub matrix: Option<[f32; 16]>,
    pub translation: Option<[f32; 3]>,
    pub rotation: Option<[f32; 4]>,
    pub scale: Option<[f32; 3]>,
    pub mesh: Option<usize>,
    pub skin: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct GMesh {
    pub name: Option<String>,
    pub primitives: Vec<Primitive>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Primitive {
    pub attributes: BTreeMap<String, usize>,
    pub indices: Option<usize>,
    pub material: Option<usize>,
    pub mode: Option<u32>,
    pub targets: Vec<serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Accessor {
    pub buffer_view: Option<usize>,
    pub byte_offset: usize,
    pub component_type: u32,
    pub normalized: bool,
    pub count: usize,
    #[serde(rename = "type")]
    pub kind: String,
    pub sparse: Option<Sparse>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Sparse {
    pub count: usize,
    pub indices: SparseIndices,
    pub values: SparseValues,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct SparseIndices {
    pub buffer_view: usize,
    pub byte_offset: usize,
    pub component_type: u32,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct SparseValues {
    pub buffer_view: usize,
    pub byte_offset: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct BufferView {
    pub buffer: usize,
    pub byte_offset: usize,
    pub byte_length: usize,
    pub byte_stride: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Buffer {
    pub uri: Option<String>,
    pub byte_length: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Material {
    pub name: Option<String>,
    pub pbr_metallic_roughness: Option<Pbr>,
    pub double_sided: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Pbr {
    pub base_color_texture: Option<TextureRef>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct TextureRef {
    pub index: usize,
    pub tex_coord: usize,
    pub extensions: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Texture {
    pub source: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Image {
    pub name: Option<String>,
    pub uri: Option<String>,
    pub mime_type: Option<String>,
    pub buffer_view: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Skin {
    pub joints: Vec<usize>,
    pub inverse_bind_matrices: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Animation {
    pub name: Option<String>,
    pub channels: Vec<Channel>,
    pub samplers: Vec<Sampler>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Channel {
    pub sampler: usize,
    pub target: Target,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Target {
    pub node: Option<usize>,
    pub path: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Sampler {
    pub input: usize,
    pub output: usize,
    pub interpolation: Option<String>,
}
