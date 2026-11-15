//! The references of the media kinds, made from the same tables the reader uses:
//! `docs/assets/MODEL.md`, `TEXTURE.md` and `SOUND.md` are this text, and
//! `asset.reference kind=model|texture|sound` answers it.

use gz_project::asset_kind::MediaKind;

use crate::fields::{ANIMATION, BONE, BUMP, DETAIL, Field, MODEL, PART, SOUND, SURFACE, TEXTURE, Ty};

fn esc(s: &str) -> String {
    s.replace('|', "\\|")
}

fn table(out: &mut String, fields: &[Field]) {
    out.push_str("| Key | Type | Default | Meaning |\n|---|---|---|---|\n");
    for f in fields {
        let ty = match f.ty {
            Ty::Number { min, max } => format!("number, {min} to {max}"),
            Ty::Integer { min, max } => format!("integer, {min} to {max}"),
            other => other.describe(),
        };
        let default = if f.default.is_empty() { "-".to_owned() } else { f.default.to_owned() };
        out.push_str(&format!("| `{}` | {} | {} | {} |\n", f.key, esc(&ty), esc(&default), esc(f.help)));
    }
}

const COMMON: &str = "\
Every key but the header may be left out. A key the editor does not know is kept where it is through every \
save and listed once as a note. The asset may lie anywhere in the project: the module build publishes it - the \
data baked from its source and a copy of this text - where the game looks for its `name`; the asset where it \
lies and its source never ship as they are. What is baked is kept in the project's `temp\\media\\`. `asset.check` \
names every problem at its key (`M1xx`); `asset.import from=<files> to=<folder>` brings files in and writes \
their assets. The rules every kind keeps are `docs/ASSETS.md`.\n\n";

const CODES: &str = "\
## Problems\n\n\
| Code | Severity | What |\n|---|---|---|\n\
| `M101` | error | The source is not there. |\n\
| `M102` | error | The source cannot be read (the reader's sentence). |\n\
| `M103` | error | Two assets of the kind publish one name (on both). |\n\
| `M104` | warning | A texture nobody has: not a texture asset, not a file of the project, not the game's. |\n\
| `M105` | warning | An engine shader the linked game does not have. |\n\
| `M106` | warning | A compiler shader the linked game does not have. |\n\
| `M107` | error | A surface's game material the linked game does not have: the level build refuses it (a bone's: a warning). |\n\
| `M108` | note | A dynamic model whose source has no bones: it gets one bone, `root`. |\n\
| `M109` | error | A value outside what its key takes. |\n\
| `M110` | error | `parts` together with `source`. |\n\
| `M111` | warning | What the model cannot take: `rotate` on a model with bones, a part that makes no faces. |\n\
| `M112` | error | A surface that wears no texture: no level build and no model of the game takes it. |\n\
| `M113` | error | A source that lies outside the project: a project names nothing outside itself. |\n\
| `M114` | warning | A sound's `streamer` on a sound that is not `copyrighted`: the replacement never plays. |\n\
| `M115` | error | A sound's replacement that is copyrighted itself: the streamer mode keeps it from playing too. |\n\
| `M116` | error | A sound's replacement nobody has: no sound asset of the project, no sound of the linked game. |\n";

/// The reference of a kind, as Markdown.
#[must_use]
pub fn reference(kind: MediaKind) -> String {
    let mut out = String::new();
    let generated = "This page is generated from the editor's own tables of the kind's keys \
(`crates/gz-media/src/fields.rs`): the reader, the check and this reference are one list. A test of \
`gz-media` fails when this file is not what the tables say. ";
    match kind {
        MediaKind::Model => {
            out.push_str("# Models (`.meshasset`)\n\n");
            out.push_str(generated);
            out.push_str(
                "A model asset is a text file beside its geometry - a glTF 2.0 (`.glb`, `.gltf`), a Wavefront OBJ \
or an X-Ray SDK `.object` - or with its geometry written in it (`parts`). A scene object refers to it by its project \
path: `ref = \"models/bunker/wall_long.meshasset\"` (any slashes, any case); any other reference is a library \
object as before. A static model is compiled into the level by the level build; a dynamic one is published as \
`meshes\\<name>.ogf` (and `.omf` with its motions) beside this text, which the game reads over the \
model: each surface's texture and shader, each bone's physics, the user data, each motion's speed and flags. \
The game reads the rows of `surfaces` by position - row n is the nth part the model is drawn in -, so the \
text that ships has one row for each of them, in their order; here a row is found by its `name`, and rows may \
be left out or stand in any order.\n\n\
glTF and OBJ are right-handed and the game is left-handed: Z is negated as the source is read and every \
triangle turned to face the same way; hard edges of the source's normals are kept; a skin becomes bones and \
weights (the four heaviest), its clips motions sampled 30 times a second.\n\n",
            );
            out.push_str(COMMON);
            out.push_str(
                "```lua\nreturn {\n  mesh = 1,\n  source = \"wall_long.glb\",\n  surfaces = {\n    { name = \"brick\", texture = \"bunker\\\\wall_brick\", material = \"materials\\\\bricks\" },\n  },\n}\n```\n\n\
A blockout, no source:\n\n\
```lua\nreturn {\n  mesh = 1,\n  parts = {\n    { shape = \"box\", size = { 4, 3, 0.3 }, pos = { 0, 0, 0 }, surface = \"brick\", tile = 2 },\n    { shape = \"ramp\", size = { 2, 1, 4 }, pos = { 3, 0, 0 }, surface = \"concrete\" },\n    { shape = \"cylinder\", radius = 0.2, height = 3, sides = 12, pos = { 0, 0, 2 }, surface = \"metal\" },\n    { shape = \"plane\", size = { 10, 10 }, surface = \"concrete\", tile = 4 },\n    { shape = \"prism\", outline = { { 0, 0 }, { 6, 0 }, { 6, 4 }, { 0, 4 } }, height = 3, surface = \"brick\" },\n  },\n}\n```\n\n\
## The keys\n\n",
            );
            table(&mut out, MODEL);
            out.push_str("\n## A row of `surfaces`\n\n");
            table(&mut out, SURFACE);
            out.push_str(
                "\n## A row of `parts`\n\n\
Every face of a part looks outward and every edge is hard; the texture is laid by box mapping in the part's \
own frame, one texture every `tile` metres.\n\n",
            );
            table(&mut out, PART);
            out.push_str("\n## A row of `bones`\n\n");
            table(&mut out, BONE);
            out.push_str("\n## A row of `animations`\n\n");
            table(&mut out, ANIMATION);
        }
        MediaKind::Texture => {
            out.push_str("# Textures (`.texasset`)\n\n");
            out.push_str(generated);
            out.push_str(
                "A texture asset is a text file beside its picture (`.png`, `.jpg`, `.tga`, `.bmp`, `.dds`) that says \
what a texture description (`.thm`) says, as named keys: nothing a `.thm` can say is unsayable here. It is \
published as `textures\\<name>.dds` (with the bump pair `<name>_bump.dds` and `<name>_bump#.dds` when `bump` has a \
`source`) beside a copy of this text, which the game registers as the texture's description; a `.thm` of the \
same name loses to it. A key left out is what the texture's `use` needs.\n\n",
            );
            out.push_str(COMMON);
            out.push_str(
                "```lua\nreturn {\n  tex = 1,\n  source = \"wall_brick.png\",\n  name = \"bunker\\\\wall_brick\",\n  material = \"phong\",\n  weight = 0.3,\n  bump = { source = \"wall_brick_height.png\", depth = 0.03 },\n  detail = { texture = \"detail\\\\detail_brick\", scale = 4 },\n}\n```\n\n\
## The keys\n\n",
            );
            table(&mut out, TEXTURE);
            out.push_str("\n## `bump`\n\n");
            table(&mut out, BUMP);
            out.push_str("\n## `detail`\n\n");
            table(&mut out, DETAIL);
        }
        MediaKind::Sound => {
            out.push_str("# Sounds (`.sndasset`)\n\n");
            out.push_str(generated);
            out.push_str(
                "A sound asset is a text file beside its sound (`.wav`, `.ogg`) that says what a sound's `.thm` says: \
how far it carries, how loud it is, what NPCs make of it. It is published as `sounds\\<name>.ogg` - a `.wav` \
encoded at `quality`, an `.ogg` as it is with these parameters written into it - beside a copy of this text, \
which the game reads over what the `.ogg` carries.\n\n\
A recording the module may not have streamed says `copyrighted = true`: while the game's streamer mode is on, \
the game plays it nowhere - its engine refuses to start it, the radio passes over it - unless `streamer` names \
a sound that plays in its place. The game keeps such a list of its own sounds; a replacement is never a \
copyrighted sound.\n\n",
            );
            out.push_str(COMMON);
            out.push_str(
                "```lua\nreturn {\n  snd = 1,\n  source = \"generator_hum.wav\",\n  name = \"bunker\\\\generator_hum\",\n  max_distance = 40,\n  type = \"world_ambient\",\n}\n```\n\n\
## The keys\n\n",
            );
            table(&mut out, SOUND);
        }
    }
    out.push('\n');
    out.push_str(CODES);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(kind: MediaKind) -> std::path::PathBuf {
        let name = match kind {
            MediaKind::Model => "MODEL.md",
            MediaKind::Texture => "TEXTURE.md",
            MediaKind::Sound => "SOUND.md",
        };
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/assets").join(name)
    }

    /// `GRIZZLY_WRITE_REFERENCE=1 bash tools/rust/cg.sh test -p gz-media reference` writes them.
    #[test]
    fn the_reference_documents_are_what_the_tables_say() {
        for kind in MediaKind::ALL {
            let path = file(kind);
            if std::env::var("GRIZZLY_WRITE_REFERENCE").is_ok_and(|v| v == "1") {
                std::fs::write(&path, reference(kind)).unwrap_or_default();
            }
            let written = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
            assert!(
                written == reference(kind),
                "{} is not what the tables say: run the test with GRIZZLY_WRITE_REFERENCE=1",
                path.display()
            );
        }
    }

    #[test]
    fn every_key_has_its_meaning_and_its_row() {
        for (kind, lists) in [
            (MediaKind::Model, vec![MODEL, SURFACE, PART, BONE, ANIMATION]),
            (MediaKind::Texture, vec![TEXTURE, BUMP, DETAIL]),
            (MediaKind::Sound, vec![SOUND]),
        ] {
            let text = reference(kind);
            for list in lists {
                for f in list {
                    assert!(!f.help.is_empty() && text.contains(&format!("| `{}` |", f.key)), "{}", f.key);
                }
            }
        }
    }
}
