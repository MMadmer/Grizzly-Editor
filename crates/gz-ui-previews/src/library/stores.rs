//! What the three libraries keep beside their tabs: the edits not yet saved, one store per
//! library, as an `Editor` service. A tab shows and changes them; `library.set` changes them
//! the same way with no tab, so what a person edits by hand and what a command edits are one
//! set of pending changes with one Save.
//!
//! Also what a tab says about itself each frame ([`LibraryFacts`]) so an action can be asked
//! whether it is on and what to run it with.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gz_editor::{Editor, Origin};
use gz_fmt_xr::LightAnims;
use gz_store::{Store, StoreOptions};
use parking_lot::Mutex;

use super::lanim::{LanimCmd, LanimState};
use super::sounds::SoundEdits;
use super::textures::TexEdits;
use crate::paths::EditorDirs;

/// The pending edits of the libraries. An `Editor` service.
pub struct LibraryStores {
    pub tex: Store<TexEdits>,
    pub sound: Store<SoundEdits>,
    pub lanim: Store<LanimState>,
    /// A texture save or import runs.
    pub tex_saving: Arc<AtomicBool>,
    /// A sound save or rebuild runs.
    pub sound_saving: Arc<AtomicBool>,
    /// A sound rebuild ended: the tab reads the new file.
    pub sound_rebuilt: Arc<AtomicBool>,
}

impl LibraryStores {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<LibraryStores> {
        editor.services.get_or_init(|| LibraryStores {
            tex: Store::spawn("texture library", TexEdits::default(), &editor.exec, StoreOptions::default()),
            sound: Store::spawn(
                "sound library",
                SoundEdits::default(),
                &editor.exec,
                StoreOptions::default(),
            ),
            lanim: Store::spawn(
                "light animations",
                LanimState::default(),
                &editor.exec,
                StoreOptions::default(),
            ),
            tex_saving: Arc::new(AtomicBool::new(false)),
            sound_saving: Arc::new(AtomicBool::new(false)),
            sound_rebuilt: Arc::new(AtomicBool::new(false)),
        })
    }

    /// The light animation file, read and parsed: the library and why it could not be read.
    pub async fn read_lanims(editor: &Editor) -> (LightAnims, Option<String>) {
        let file: PathBuf = EditorDirs::of(editor).lanims();
        let read = editor.exec.io(move || std::fs::read(&file)).await;
        match read {
            Ok(bytes) => match LightAnims::read(&bytes) {
                Ok(l) => (l, None),
                Err(e) => (LightAnims::default(), Some(format!("lanims.xr could not be read: {e}"))),
            },
            // no file yet: an empty library, written by the first save
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (LightAnims::default(), None),
            Err(e) => (LightAnims::default(), Some(format!("lanims.xr could not be read: {e}"))),
        }
    }

    /// Reads the light animation file into the store, dropping what was pending.
    pub async fn reload_lanims(editor: &Editor) {
        let (lib, error) = Self::read_lanims(editor).await;
        let _ = Self::of(editor).lanim.send(LanimCmd::Load { lib, error }, Origin::Ui).await;
    }

    /// The light animations as the store holds them, read first when nobody has.
    pub async fn lanims_ready(editor: &Editor) -> Arc<LanimState> {
        let stores = Self::of(editor);
        if !stores.lanim.snapshot().loaded {
            Self::reload_lanims(editor).await;
        }
        let snap = stores.lanim.snapshot();
        Arc::new(LanimState { lib: snap.lib.clone(), loaded: snap.loaded, error: snap.error.clone() })
    }
}

/// What the Texture Library tab says of itself.
#[derive(Clone, Debug, Default)]
pub struct TexFacts {
    pub read_only: bool,
    pub saving: bool,
    pub dirty: bool,
    /// New Textures: the tab shows the import folder.
    pub import: bool,
    /// Which texture types the list shows.
    pub types: [bool; 5],
}

/// What the Sound Library tab says of itself.
#[derive(Clone, Debug, Default)]
pub struct SoundFacts {
    pub read_only: bool,
    pub saving: bool,
    pub selected: bool,
    pub has_game_sound: bool,
    pub playing: bool,
    pub auto_play: bool,
}

/// What the Light Animations tab says of itself.
#[derive(Clone, Debug, Default)]
pub struct LanimFacts {
    pub selected: bool,
    pub pointer: i32,
    pub frames: i32,
    pub on_key: bool,
    pub keys: usize,
    pub playing: bool,
    pub alpha: bool,
    /// The colour at the pointer, `0xAARRGGBB`.
    pub color: u32,
    /// The nearest free frame before and after the pointer, where a key here could move.
    pub free_before: Option<i32>,
    pub free_after: Option<i32>,
}

/// The tabs' facts: written by each tab as it draws, read by the actions. An `Editor` service.
#[derive(Default)]
pub struct LibraryFacts {
    tex: Mutex<TexFacts>,
    sound: Mutex<SoundFacts>,
    lanim: Mutex<LanimFacts>,
}

impl LibraryFacts {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<LibraryFacts> {
        editor.services.get_or_init(LibraryFacts::default)
    }

    #[must_use]
    pub fn tex(&self) -> TexFacts {
        self.tex.lock().clone()
    }

    pub fn set_tex(&self, facts: TexFacts) {
        *self.tex.lock() = facts;
    }

    #[must_use]
    pub fn sound(&self) -> SoundFacts {
        self.sound.lock().clone()
    }

    pub fn set_sound(&self, facts: SoundFacts) {
        *self.sound.lock() = facts;
    }

    #[must_use]
    pub fn lanim(&self) -> LanimFacts {
        self.lanim.lock().clone()
    }

    pub fn set_lanim(&self, facts: LanimFacts) {
        *self.lanim.lock() = facts;
    }
}

/// Whether a flag a job holds is set.
#[must_use]
pub fn is_set(flag: &AtomicBool) -> bool {
    flag.load(Ordering::Acquire)
}
