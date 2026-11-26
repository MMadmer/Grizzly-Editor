//! The names a scene's objects pick - a sound, a particle effect, a light animation, a sound
//! environment, a texture, a compiled model - told against what there is.
//!
//! The scene's crate knows no list of the game. Whoever holds them (the game library)
//! registers a source here; the scene's check asks it, and a name nobody knows is a problem at
//! its key, with the nearest names there are.

use std::sync::Arc;

use futures::future::BoxFuture;
use gz_editor::Editor;
use parking_lot::RwLock;

/// The lists of names as they are now.
pub trait Names: Send + Sync {
    /// Whether a kind (`sound`, `particle`, `lightanim`, `sound_env`, `texture`, `model`,
    /// `fog_profile`) has the name; `None` when the kind's list is not at hand (nothing is said
    /// then).
    fn has(&self, kind: &str, name: &str) -> Option<bool>;

    /// The names of the kind nearest to one it has not, best first.
    fn nearest(&self, kind: &str, name: &str) -> Vec<String>;

    /// Why the game would not take a thing of this name though it is there: a fog profile that is
    /// a file and no profile. `None` when nothing is wrong with it, or nothing is known.
    fn refusal(&self, _kind: &str, _name: &str) -> Option<String> {
        None
    }

    /// Whether the game draws nothing for a compiled model (`<name>.ogf`): it is there - it
    /// stops a body, it is a container - and nobody sees it. `None` when nothing describes the
    /// model.
    fn undrawn(&self, _model: &str) -> Option<bool> {
        None
    }
}

/// The names, with the compiled models of the editor's own data (`gamedata\meshes` of its
/// library) that a build stages into the module - a stash's case among them: what the build finds,
/// the audit finds too.
pub struct WithEditorModels<'a> {
    pub names: &'a dyn Names,
    pub meshes: std::path::PathBuf,
}

impl Names for WithEditorModels<'_> {
    fn has(&self, kind: &str, name: &str) -> Option<bool> {
        match self.names.has(kind, name) {
            Some(false) if kind == "model" && self.meshes.join(name.replace('/', "\\")).is_file() => {
                Some(true)
            }
            said => said,
        }
    }

    fn nearest(&self, kind: &str, name: &str) -> Vec<String> {
        self.names.nearest(kind, name)
    }

    fn refusal(&self, kind: &str, name: &str) -> Option<String> {
        self.names.refusal(kind, name)
    }

    fn undrawn(&self, model: &str) -> Option<bool> {
        self.names.undrawn(model)
    }
}

/// What gives the lists: asked once per check, off the UI thread.
pub type NamesSource =
    Arc<dyn for<'a> Fn(&'a Arc<Editor>) -> BoxFuture<'a, Option<Arc<dyn Names>>> + Send + Sync>;

/// The editor's source of the names a scene may pick.
#[derive(Default)]
pub struct NameSources {
    source: RwLock<Option<NamesSource>>,
}

impl NameSources {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(Self::default)
    }

    /// Registers the source: the game library does, once, as the editor starts.
    pub fn set(&self, source: NamesSource) {
        *self.source.write() = Some(source);
    }

    /// The lists as they are now, when a source is registered and has them.
    pub async fn names(editor: &Arc<Editor>) -> Option<Arc<dyn Names>> {
        let source = Self::of(editor).source.read().clone()?;
        source(editor).await
    }
}

/// The kind of the lists a picked key of the scene's files names, for the keys that name
/// something of the game by its name.
#[must_use]
pub fn kind_of_pick(list: &str, value: &str) -> Option<&'static str> {
    match list {
        "sound_source" => Some("sound"),
        "particles" => Some("particle"),
        "lightanim" => Some("lightanim"),
        "sound_env" => Some("sound_env"),
        "fog_profile" => Some("fog_profile"),
        "texture" => Some("texture"),
        // a library object of the editor is another list: here the game's compiled models
        "object" if value.to_ascii_lowercase().ends_with(".ogf") => Some("model"),
        _ => None,
    }
}

/// What a kind is called in a sentence.
#[must_use]
pub fn kind_word(kind: &str) -> &'static str {
    match kind {
        "sound" => "sound",
        "particle" => "particle effect",
        "lightanim" => "light animation",
        "sound_env" => "sound environment",
        "fog_profile" => "fog profile",
        "texture" => "texture",
        "model" => "model",
        _ => "name",
    }
}
