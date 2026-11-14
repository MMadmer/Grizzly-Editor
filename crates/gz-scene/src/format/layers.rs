//! The painted layers of a scene - the AI map, the detail objects, the wallmarks - as binary
//! files beside its Lua files (`ai_map.bin`, `details.bin`, `wallmarks.bin`).
//!
//! A layer file is a header - the magic `GZLAYER\0`, a format version and the layer's kind, each
//! a little-endian `u32` - and the layer's data as the scene holds it in memory
//! ([`BulkData::stream_of`]): written back bit for bit while nothing painted it.

use crate::class::ObjClass;
use crate::tools::BulkData;

pub const MAGIC: &[u8; 8] = b"GZLAYER\0";
/// The version of the header this editor writes and the newest it reads.
pub const VERSION: u32 = 1;

/// One painted layer: its file, its kind's number in the header, its class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layer {
    pub file: &'static str,
    /// The key of the `.scene`'s `layers` table.
    pub key: &'static str,
    pub kind: u32,
    pub class: ObjClass,
}

pub const LAYERS: [Layer; 3] = [
    Layer { file: "ai_map.bin", key: "ai_map", kind: 1, class: ObjClass::AiMap },
    Layer { file: "details.bin", key: "details", kind: 2, class: ObjClass::DetailObjects },
    Layer { file: "wallmarks.bin", key: "wallmarks", kind: 3, class: ObjClass::Wallmark },
];

/// The layer a `layers` key names.
#[must_use]
pub fn by_key(key: &str) -> Option<Layer> {
    LAYERS.iter().copied().find(|l| l.key == key)
}

/// A layer's file: the header and the data.
#[must_use]
pub fn write(layer: Layer, bulk: &BulkData) -> Vec<u8> {
    let data = bulk.stream_of(layer.class);
    let mut out = Vec::with_capacity(16 + data.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&layer.kind.to_le_bytes());
    out.extend_from_slice(&data);
    out
}

/// Whether the scene has anything of a layer to keep.
#[must_use]
pub fn has(layer: Layer, bulk: &BulkData) -> bool {
    bulk.needs_save(layer.class)
}

/// Reads a layer's file into `bulk`; a sentence when it is not one, or of a newer editor.
pub fn read(layer: Layer, bytes: &[u8], bulk: &mut BulkData) -> Result<Option<String>, String> {
    let word = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if bytes.get(..8) != Some(&MAGIC[..]) {
        return Err(format!(
            "{} is no layer file of the editor (it does not start with GZLAYER)",
            layer.file
        ));
    }
    let version = word(8).ok_or_else(|| format!("{} ends in its header", layer.file))?;
    if version > VERSION {
        return Err(format!(
            "{} was made by a newer Grizzly Editor (layer version {version}; this one reads up to {VERSION}): update the editor to open it",
            layer.file
        ));
    }
    if word(12) != Some(layer.kind) {
        return Err(format!("{} holds another layer than its name says", layer.file));
    }
    Ok(bulk.set_stream(layer.class, &bytes[16..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layer_reads_back_bit_for_bit() {
        let mut bulk = BulkData::default();
        for l in LAYERS {
            let bytes = write(l, &bulk);
            assert!(read(l, &bytes, &mut bulk).unwrap().is_none(), "{}", l.file);
            assert_eq!(write(l, &bulk), bytes);
        }
        let mut wrong = write(LAYERS[0], &bulk);
        wrong[8] = 9;
        assert!(read(LAYERS[0], &wrong, &mut bulk).unwrap_err().contains("newer"));
        assert!(read(LAYERS[1], &write(LAYERS[0], &bulk), &mut bulk).is_err());
    }
}
