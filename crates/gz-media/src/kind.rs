//! The three media kinds of `gz-asset`: their entries (extension, header, version, the keys
//! each level knows, the check, the reference) and the one read of a file through them - the
//! version checked, an older one migrated.

use std::path::PathBuf;
use std::sync::Arc;

use gz_asset::{KindOps, Migration};
use gz_base::RelPath;
use gz_luadata::{DiffRules, Limits, Value};
use gz_project::asset_kind::MediaKind;

const MODEL_TOP: &[&str] = &[
    "mesh",
    "source",
    "name",
    "kind",
    "scale",
    "rotate",
    "surfaces",
    "parts",
    "bones",
    "animations",
    "userdata",
];
const MODEL_SURFACE: &[&str] = &["name", "texture", "shader", "compile", "material", "two_sided"];
const MODEL_PART: &[&str] =
    &["shape", "size", "pos", "rot", "surface", "tile", "radius", "height", "sides", "outline"];
const MODEL_BONE: &[&str] = &["name", "shape", "mass", "material"];
const MODEL_ANIMATION: &[&str] = &["name", "clip", "speed", "loop"];

const TEXTURE_TOP: &[&str] = &[
    "tex",
    "source",
    "name",
    "use",
    "format",
    "mips",
    "mip_filter",
    "has_alpha",
    "binary_alpha",
    "dither",
    "dither_each_mip",
    "implicit_light",
    "alpha_border",
    "color_border",
    "border_color",
    "fade_to_color",
    "fade_to_alpha",
    "fade_color",
    "fade_amount",
    "fade_delay",
    "material",
    "weight",
    "normal_map",
    "bump",
    "detail",
    "extra_flags",
];
const TEXTURE_BUMP: &[&str] = &["texture", "source", "from", "depth", "parallax"];
const TEXTURE_DETAIL: &[&str] = &["texture", "scale", "on"];

const SOUND_TOP: &[&str] =
    &["snd", "source", "name", "min_distance", "max_distance", "volume", "ai_distance", "type", "quality"];

/// The keys each level of a kind knows: any other is the author's, kept and noted.
#[must_use]
pub fn known(kind: MediaKind) -> &'static [(&'static str, &'static [&'static str])] {
    match kind {
        MediaKind::Model => &[
            ("", MODEL_TOP),
            ("surfaces.*", MODEL_SURFACE),
            ("parts.*", MODEL_PART),
            ("bones.*", MODEL_BONE),
            ("animations.*", MODEL_ANIMATION),
        ],
        MediaKind::Texture => &[("", TEXTURE_TOP), ("bump", TEXTURE_BUMP), ("detail", TEXTURE_DETAIL)],
        MediaKind::Sound => &[("", SOUND_TOP)],
    }
}

/// Where each kind's reference is written down.
#[must_use]
pub fn reference_doc(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Model => "docs/assets/MODEL.md",
        MediaKind::Texture => "docs/assets/TEXTURE.md",
        MediaKind::Sound => "docs/assets/SOUND.md",
    }
}

/// The steps from version n to n + 1 of each kind: none yet (every kind is at 1).
const NO_STEPS: &[Migration] = &[];

fn model_rules() -> DiffRules {
    DiffRules::new()
        .order("", MODEL_TOP)
        .keyed("surfaces", "name")
        .keyed("bones", "name")
        .keyed("animations", "name")
}

fn texture_rules() -> DiffRules {
    DiffRules::new().order("", TEXTURE_TOP).order("bump", TEXTURE_BUMP).order("detail", TEXTURE_DETAIL)
}

fn sound_rules() -> DiffRules {
    DiffRules::new().order("", SOUND_TOP)
}

/// The entry of a kind, with `steps` as its migrations and `version` as its current version
/// (a test lifts both).
#[must_use]
pub fn ops_with(kind: MediaKind, version: u32, steps: &'static [Migration]) -> KindOps {
    let mut k = KindOps::of(gz_project::AssetKind::Quest);
    k.name = kind.name();
    k.ext = kind.extension();
    k.header = kind.header();
    k.version = version;
    k.limits = Limits::asset();
    k.migrations = steps;
    k.known = known(kind);
    k.reference_doc = reference_doc(kind);
    k.diff_rules = match kind {
        MediaKind::Model => model_rules,
        MediaKind::Texture => texture_rules,
        MediaKind::Sound => sound_rules,
    };
    k.check = match kind {
        MediaKind::Model => |ed, f| Box::pin(crate::check::check(ed, f, MediaKind::Model)),
        MediaKind::Texture => |ed, f| Box::pin(crate::check::check(ed, f, MediaKind::Texture)),
        MediaKind::Sound => |ed, f| Box::pin(crate::check::check(ed, f, MediaKind::Sound)),
    };
    k.reference = match kind {
        MediaKind::Model => |_| Box::pin(async { crate::doc::reference(MediaKind::Model) }),
        MediaKind::Texture => |_| Box::pin(async { crate::doc::reference(MediaKind::Texture) }),
        MediaKind::Sound => |_| Box::pin(async { crate::doc::reference(MediaKind::Sound) }),
    };
    k
}

/// The entry of a kind as the editor has it.
#[must_use]
pub fn ops(kind: MediaKind) -> KindOps {
    ops_with(kind, kind.version(), NO_STEPS)
}

/// The value of a file of `kind` through the kind's read: a newer version refused by name, an
/// older one migrated, a syntax error with its line. `rel` names the file in the sentences.
pub fn load_with(ops: &Arc<KindOps>, rel: &str, text: &str) -> Result<Value, String> {
    let path = RelPath::new(rel).map_err(|e| format!("{rel}: {e}"))?;
    match gz_asset::read_text(ops, path, PathBuf::new(), text, &[]) {
        Ok(f) => Ok(f.doc.value()),
        Err(e) => Err(match (e.problem.line, e.problem.column) {
            (Some(l), Some(c)) => format!("{rel}:{l}:{c}: {}", e.problem.message),
            _ => format!("{rel}: {}", e.problem.message),
        }),
    }
}

/// [`load_with`] with the kind's own entry.
pub fn load(kind: MediaKind, rel: &str, text: &str) -> Result<Value, String> {
    load_with(&Arc::new(ops(kind)), rel, text)
}
