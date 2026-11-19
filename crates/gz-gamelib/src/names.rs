//! The names a scene may pick, as the game library and the open project hold them: what the
//! scene's check tells a sound, a particle effect, a light animation, a sound environment, a
//! texture and a compiled model against (`gz_scene::names`).

use std::path::PathBuf;
use std::sync::Arc;

use gz_editor::Editor;
use gz_project::asset_kind::MediaKind;
use gz_scene::names::{NameSources, Names};

use crate::index::Index;

/// The library's catalogs, with the open project's own files and media assets beside them.
struct LibraryNames {
    index: Arc<Index>,
    project: Option<PathBuf>,
    /// The project's media assets, by the name each publishes under.
    media: Option<Arc<gz_media::Index>>,
    /// The game's files, for what is named by its file: a fog volume's profile is a config.
    game: Option<Arc<gz_vfs::GameFiles>>,
}

impl LibraryNames {
    /// Whether the open project brings the thing itself: a file where the game looks for it, or
    /// a media asset that publishes under the name.
    fn own(&self, kind: &str, name: &str) -> bool {
        let name = name.trim().replace('/', "\\");
        let asset = |k: MediaKind| self.media.as_ref().is_some_and(|m| m.by_name(k, &name).is_some());
        let file = |under: &str, ext: &str| {
            self.project
                .as_ref()
                .is_some_and(|root| root.join("gamedata").join(under).join(format!("{name}{ext}")).is_file())
        };
        match kind {
            "sound" => asset(MediaKind::Sound) || file("sounds", ".ogg"),
            "texture" => asset(MediaKind::Texture) || file("textures", ".dds"),
            // the name ends in `.ogf` already
            "model" => file("meshes", ""),
            // a module's effects ship loose, an effect or a group of them
            "particle" => file("particles", ".pe") || file("particles", ".pg"),
            _ => false,
        }
    }

    /// Whether the project brings a library of the kind as one file of its own, which the
    /// game reads in place of its own: what that holds nobody here has read.
    fn own_library(&self, kind: &str) -> bool {
        let file = match kind {
            "particle" => "particles.xr",
            "lightanim" => "lanims.xr",
            "sound_env" => "senvironment.xr",
            _ => return false,
        };
        self.project.as_ref().is_some_and(|root| root.join("gamedata").join(file).is_file())
    }
}

impl LibraryNames {
    /// A file of the game's configs by its name from there, as a game with the module reads it:
    /// the project's own - under `gamedata\configs`, or what its `[vfs]` mounts under
    /// `configs\` - first, then the game's. `Some(None)` when nobody has it, `None` when nothing
    /// says (no game at hand).
    fn config_bytes(&self, name: &str) -> Option<Option<Vec<u8>>> {
        let name = name.trim().replace('\\', "/");
        let plain = !name.is_empty()
            && !name.contains(':')
            && name.split('/').all(|part| !part.is_empty() && part != "." && part != "..");
        if !plain {
            return Some(None);
        }
        if let Some(root) = &self.project {
            if let Ok(bytes) = std::fs::read(root.join("gamedata").join("configs").join(&name)) {
                return Some(Some(bytes));
            }
            if let Some(bytes) = mounted(root, &name) {
                return Some(Some(bytes));
            }
        }
        // without the game's files nothing says the file is missing
        let game = self.game.as_ref()?;
        Some(game.read(&format!("configs/{name}")).ok())
    }

    /// Whether a file of the game's configs is there, in the game or in the project.
    fn config(&self, name: &str) -> Option<bool> {
        self.config_bytes(name).map(|b| b.is_some())
    }
}

/// A config the project's manifest mounts under `configs\` from another place of the project
/// (`[vfs]`: a file, or a folder with the file in it). `name` is the file from `configs`, forward
/// slashes.
fn mounted(root: &std::path::Path, name: &str) -> Option<Vec<u8>> {
    let text = gz_mod::read_manifest_file(root).ok()??;
    let doc = gz_mod::ManifestDoc::parse(&text);
    let want = format!("configs\\{}", name.replace('/', "\\")).to_ascii_lowercase();
    for (virtual_path, inside) in doc.section_entries("vfs") {
        let v = virtual_path.trim().replace('/', "\\").to_ascii_lowercase();
        let v = v.trim_end_matches('\\');
        let inside = inside.trim().replace('\\', "/");
        let file = if want == v {
            Some(inside)
        } else {
            want.strip_prefix(v)
                .and_then(|rest| rest.strip_prefix('\\'))
                .map(|rest| format!("{}/{}", inside.trim_end_matches('/'), rest.replace('\\', "/")))
        };
        // a mount names a place inside the project: no step up, no drive
        let Some(file) = file.filter(|f| !f.contains(':') && !f.split('/').any(|p| p == "..")) else {
            continue;
        };
        if let Ok(bytes) = std::fs::read(root.join(file.replace('/', "\\"))) {
            return Some(bytes);
        }
    }
    None
}

/// The most emitters a fog volume's profile may name.
const MAX_FOG_EMITTERS: u32 = 64;

/// Why a config is no profile of a fog volume: what the game's loader reads of it without
/// asking - `[volume]` with `EmittersNum`, and a section `emitterNN` for each with its keys. The
/// game asks a module's profile the same before it reads it and leaves the volume out; the rule
/// is the game's (its renderer's fog volumes): change both or neither. `None` for a profile
/// that reads, and for one that takes lines from another file - what that holds nobody read.
#[must_use]
pub fn fog_profile_refusal(text: &str) -> Option<String> {
    use std::collections::{HashMap, HashSet};
    let mut sections: HashMap<String, HashSet<String>> = HashMap::new();
    let mut values: HashMap<(String, String), String> = HashMap::new();
    let mut at: Option<String> = None;
    for raw in text.lines() {
        let line = raw.split(';').next().unwrap_or_default().trim();
        if line.starts_with("#include") {
            return None;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let name = rest.split(']').next().unwrap_or_default().trim().to_ascii_lowercase();
            sections.entry(name.clone()).or_default();
            at = Some(name);
        } else if let (Some(section), Some((key, value))) = (&at, line.split_once('=')) {
            let key = key.trim().to_ascii_lowercase();
            values.insert((section.clone(), key.clone()), value.trim().to_owned());
            sections.entry(section.clone()).or_default().insert(key);
        }
    }
    let has = |section: &str, key: &str| sections.get(section).is_some_and(|keys| keys.contains(key));
    if !has("volume", "emittersnum") {
        return Some("it has no [volume] section that counts its emitters (EmittersNum)".to_owned());
    }
    let count: u32 = values
        .get(&("volume".to_owned(), "emittersnum".to_owned()))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if count > MAX_FOG_EMITTERS {
        return Some(format!("it names {count} emitters, and a volume takes {MAX_FOG_EMITTERS} at most"));
    }
    const ALWAYS: [&str; 8] =
        ["Type", "Radius", "Sigma", "FlowDirection", "FlowSpeed", "Density", "ApplyDensity", "ApplyImpulse"];
    const DRAUGHT: [&str; 3] = ["DraughtPeriod", "DraughtPhase", "DraughtAmp"];
    for i in 0..count {
        let section = format!("emitter{i:02}");
        if !sections.contains_key(&section) {
            return Some(format!("it counts {count} emitters and has no [{section}]"));
        }
        let lacks = |key: &str| {
            (!has(&section, &key.to_ascii_lowercase())).then(|| format!("[{section}] has no {key}"))
        };
        if let Some(why) = ALWAYS.iter().find_map(|k| lacks(k)) {
            return Some(why);
        }
        if !has(&section, "position") && !has(&section, "worldposition") {
            return Some(format!("[{section}] has no Position"));
        }
        let draught = values
            .get(&(section.clone(), "type".to_owned()))
            .is_some_and(|t| t.eq_ignore_ascii_case("SimpleDraught"));
        if draught && let Some(why) = DRAUGHT.iter().find_map(|k| lacks(k)) {
            return Some(why);
        }
    }
    None
}

impl Names for LibraryNames {
    fn has(&self, kind: &str, name: &str) -> Option<bool> {
        if kind == "fog_profile" {
            return self.config(name);
        }
        // a catalog the library has not made says nothing of a name
        if self.index.rows(kind).is_empty() {
            return None;
        }
        if self.index.row(kind, name).is_some() || self.own(kind, name) {
            return Some(true);
        }
        // not the game's, and the project has a library of its own nobody read: unknown
        if self.own_library(kind) { None } else { Some(false) }
    }

    fn nearest(&self, kind: &str, name: &str) -> Vec<String> {
        self.index.nearest(kind, name, 3)
    }

    fn refusal(&self, kind: &str, name: &str) -> Option<String> {
        if kind != "fog_profile" {
            return None;
        }
        let bytes = self.config_bytes(name)??;
        fog_profile_refusal(&gz_base::text::decode_guess(&bytes))
    }

    fn undrawn(&self, model: &str) -> Option<bool> {
        // a model of the project's own is described by nobody here
        if self.own("model", model) {
            return None;
        }
        let row = self.index.row("model", model)?;
        Some(row.get("invisible").and_then(serde_json::Value::as_bool) == Some(true))
    }
}

/// Registers the lists as what a scene's names are told against.
pub fn install(editor: &Arc<Editor>) {
    NameSources::of(editor).set(Arc::new(|ed: &Arc<Editor>| {
        Box::pin(async move {
            // the library as it is: a check does not start a reading of the game, and while one
            // runs its catalogs are half written - a name is told against none of them
            if crate::service::GameLibrary::running(ed).is_some() {
                return None;
            }
            let layout = crate::service::GameLibrary::layout(ed)?;
            let index = Index::of_layout(ed, &layout).await;
            let project = gz_project::Project::of(ed).ok().map(|(_, p)| PathBuf::from(p.root()));
            let media = match gz_media::Media::of(ed).await {
                Ok(m) => Some(m.index().await),
                Err(_) => None,
            };
            // the files the game is mounted with already: a check mounts nothing
            let game = match gz_game::GameLink::of(ed) {
                Ok(link) => link.files().await.ok(),
                Err(_) => None,
            };
            Some(Arc::new(LibraryNames { index, project, media, game }) as Arc<dyn Names>)
        })
    }));
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const EMITTER: &str = "Type = SimpleGaussian\nPosition = 0, 0, 0\nRadius = 1\nSigma = 1\nFlowDirection = 0, 1, 0\nFlowSpeed = 1\nDensity = 1\nApplyDensity = true\nApplyImpulse = true\n";

    #[test]
    fn a_fog_profile_is_what_the_loader_reads_without_asking() {
        let good =
            format!("[volume]\nEmittersNum = 2 ; two of them\n[emitter00]\n{EMITTER}[emitter01]\n{EMITTER}");
        assert_eq!(fog_profile_refusal(&good), None);
        // any config that is no profile, and a profile with a hole in it
        let why = fog_profile_refusal("[actor]\nhealth = 1\n").unwrap();
        assert!(why.contains("[volume]"), "{why}");
        let short = format!("[volume]\nEmittersNum = 2\n[emitter00]\n{EMITTER}");
        assert!(fog_profile_refusal(&short).unwrap().contains("[emitter01]"));
        let holed = good.replacen("Sigma = 1\n", "", 1);
        assert_eq!(fog_profile_refusal(&holed).as_deref(), Some("[emitter00] has no Sigma"));
        let many = "[volume]\nEmittersNum = 4000000\n";
        assert!(fog_profile_refusal(many).unwrap().contains("64 at most"));
        // a draught says its wave, in any case of the keys
        let draught = good.replacen("Type = SimpleGaussian", "type = simpledraught", 1);
        assert_eq!(fog_profile_refusal(&draught).as_deref(), Some("[emitter00] has no DraughtPeriod"));
        // a file that takes lines from another is not judged
        assert_eq!(fog_profile_refusal("#include \"base.ltx\"\n[x]\n"), None);
    }

    #[test]
    fn a_config_the_manifest_mounts_is_the_modules_own() {
        let dir = std::env::temp_dir().join(format!("gz-gamelib-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.as_path();
        std::fs::create_dir_all(root.join("tuning").join("fog")).unwrap();
        std::fs::write(root.join("tuning").join("fog").join("cellar.ltx"), "[volume]\n").unwrap();
        std::fs::write(root.join("one.ltx"), "[volume]\n").unwrap();
        std::fs::write(
            root.join("mod.ltx"),
            "[module]\nid = m\n[vfs]\nconfigs/environment = tuning\nconfigs/single.ltx = one.ltx\nconfigs/out = ../elsewhere\n",
        )
        .unwrap();
        assert!(mounted(root, "environment/fog/cellar.ltx").is_some());
        assert!(mounted(root, "single.ltx").is_some());
        assert!(mounted(root, "environment/fog/other.ltx").is_none());
        assert!(mounted(root, "out/x.ltx").is_none(), "a mount that steps out of the project names nothing");
        // through the lists: the module's own profile is there, and one of no folder is not
        let names = LibraryNames {
            index: Arc::new(Index::default()),
            project: Some(root.to_path_buf()),
            media: None,
            game: None,
        };
        assert_eq!(names.config("environment/fog/cellar.ltx"), Some(true));
        assert_eq!(names.config("../mod.ltx"), Some(false));
        assert_eq!(
            names.config("environment/fog/none.ltx"),
            None,
            "without the game nothing says it is missing"
        );
        let why = names.refusal("fog_profile", "environment/fog/cellar.ltx").unwrap();
        assert!(why.contains("EmittersNum"), "{why}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
