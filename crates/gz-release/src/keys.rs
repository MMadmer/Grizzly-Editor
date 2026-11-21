//! The author key of this Windows user (MOD_CATALOG 4.1): a FILE the author keeps, never a
//! secret of this computer. `Documents\Grizzly Editor\Keys\author.xkey` unless the author picks
//! another place, never inside a project; `Documents\Grizzly Editor\catalog.ltx` remembers where
//! it is. A `-qa_github` run keeps both in `GRIZZLY_CATALOG_HOME`, so a test never meets the
//! author's own key.
//!
//! [`AuthorKeys`] lives as long as the editor (`editor.services`): the key opened this session -
//! by its password once, or by itself when the file is unprotected - stays open until the
//! editor closes, and its scalar is wiped then. Nothing of it reaches a log or an answer but
//! the public half.

use std::path::Path;
use std::sync::Arc;

use gz_editor::{Editor, Log};
use parking_lot::Mutex;
use serde::Serialize;

use crate::crypto::EcKey;
use crate::keyfile::{self, KEY_FILE_LIMIT};
use crate::signed::sign_text;

/// What the key looks like now: the `key` object of `release.catalog`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct KeyState {
    pub exists: bool,
    /// Protected by a password not given yet this session.
    pub locked: bool,
    /// Where the key file is, or where a new one would go.
    pub path: String,
    /// 16 hex digits; empty when the file does not read.
    pub key_id: String,
    /// `p256:...`; empty when the file does not read.
    #[serde(rename = "public")]
    pub public_key: String,
    pub error: String,
}

impl KeyState {
    /// A key that signs now.
    #[must_use]
    pub fn usable(&self) -> bool {
        self.exists && self.error.is_empty() && !self.locked
    }
}

struct Open {
    key: EcKey,
    /// The file it came from.
    path: String,
}

/// The author key of the session. Its methods do file work: call them on the IO pool.
pub struct AuthorKeys {
    qa: bool,
    /// Where a QA run keeps its keys when its environment names no folder: inside the editor's home.
    qa_home: String,
    log: Log,
    open: Mutex<Option<Open>>,
    /// The home folder, found once.
    home: Mutex<Option<String>>,
}

fn read_limited(path: &str, limit: u64) -> Option<String> {
    let md = std::fs::metadata(path).ok()?;
    if !md.is_file() || md.len() > limit {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Creates a file that must not exist yet; a half-written one is removed.
fn write_new(path: &str, text: &str) -> bool {
    use std::io::Write;
    let Ok(mut f) = std::fs::OpenOptions::new().write(true).create_new(true).open(path) else { return false };
    if f.write_all(text.as_bytes()).and_then(|()| f.sync_all()).is_ok() {
        return true;
    }
    drop(f);
    let _ = std::fs::remove_file(path);
    false
}

fn make_folders(file: &str) {
    if let Some(dir) = Path::new(file).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
}

impl AuthorKeys {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(|| Self {
            qa: editor.flags.qa_github,
            qa_home: editor.paths.home.join("catalog_home").display().to_string(),
            log: editor.log.clone(),
            open: Mutex::new(None),
            home: Mutex::new(None),
        })
    }

    /// `Documents\Grizzly Editor`. A QA run never looks there: `GRIZZLY_CATALOG_HOME`, or a folder
    /// of the editor's own home when the environment names none.
    #[must_use]
    pub fn home(&self) -> String {
        if let Some(h) = self.home.lock().clone() {
            return h;
        }
        let home = if self.qa {
            std::env::var("GRIZZLY_CATALOG_HOME")
                .ok()
                .filter(|h| !h.trim().is_empty())
                .unwrap_or_else(|| self.qa_home.clone())
        } else {
            format!("{}\\Grizzly Editor", crate::documents::documents_folder())
        };
        *self.home.lock() = Some(home.clone());
        home
    }

    fn settings_path(&self) -> String {
        format!("{}\\catalog.ltx", self.home())
    }

    /// Where a new key goes unless the author picks another place.
    #[must_use]
    pub fn default_key_path(&self) -> String {
        format!("{}\\Keys\\author.xkey", self.home())
    }

    fn remembered_key_path(&self) -> String {
        let Some(text) = read_limited(&self.settings_path(), 64 << 10) else { return String::new() };
        let Some(at) = text.find("\nkey = ") else { return String::new() };
        let rest = text.get(at + 7..).unwrap_or("");
        rest.split(['\r', '\n']).next().unwrap_or("").to_owned()
    }

    fn remember_key_path(&self, path: &str) {
        let file = self.settings_path();
        make_folders(&file);
        let text = format!(
            "; Grizzly Editor: where this Windows user's Mod Browser author key file is\r\n[author]\r\nkey = {path}\r\n"
        );
        let _ = std::fs::remove_file(&file);
        write_new(&file, &text);
    }

    /// The key file in use: the remembered one, or the default place.
    #[must_use]
    pub fn current_key_path(&self) -> String {
        let remembered = self.remembered_key_path();
        if remembered.is_empty() { self.default_key_path() } else { remembered }
    }

    /// The key as it stands (`KeyState`): the file is read every time.
    #[must_use]
    pub fn state(&self) -> KeyState {
        let mut state = KeyState { path: self.current_key_path(), ..KeyState::default() };
        let Some(text) = read_limited(&state.path, KEY_FILE_LIMIT) else { return state };
        state.exists = true;
        let file = match keyfile::parse(&text) {
            Ok(f) => f,
            Err(e) => {
                state.error = e;
                return state;
            }
        };
        let Some(point) = file.public() else {
            state.error = "the key file's public key is damaged".to_owned();
            return state;
        };
        state.public_key.clone_from(&file.public_text);
        state.key_id = point.id();
        let open = self.open.lock();
        let opened =
            open.as_ref().is_some_and(|o| o.path == state.path && o.key.public.text() == file.public_text);
        state.locked = file.protected && !opened;
        state
    }

    /// A new key in a new file, and the path remembered. Never overwrites a file;
    /// never inside the open project (`project_root`).
    pub fn create(
        &self,
        path: Option<&str>,
        password: &str,
        project_root: Option<&Path>,
    ) -> Result<(), String> {
        let target = path.filter(|p| !p.is_empty()).map_or_else(|| self.default_key_path(), str::to_owned);
        inside_project(&target, project_root)?;
        if std::fs::symlink_metadata(&target).is_ok() {
            return Err(format!("{target} exists: a key file is never overwritten"));
        }
        let key = EcKey::generate().map_err(|_| "Windows could not make a P-256 key".to_owned())?;
        make_folders(&target);
        let text = keyfile::text(&key, password)?;
        if !write_new(&target, &text) {
            return Err(format!("can't write {target}"));
        }
        self.remember_key_path(&target);
        // the key's id names its author in the catalog: the log says a key was made, not which
        self.log.info(format!("[catalog] an author key was created in {target}"));
        *self.open.lock() = Some(Open { key, path: target });
        Ok(())
    }

    /// Another copy of the key - from another computer, a stick - is the one used from now on
    /// . An unprotected one opens at once; a protected one waits for [`Self::unlock`].
    pub fn use_file(&self, path: &str, project_root: Option<&Path>) -> Result<(), String> {
        let Some(text) = read_limited(path, KEY_FILE_LIMIT) else { return Err(format!("can't read {path}")) };
        inside_project(path, project_root)?;
        let file = keyfile::parse(&text)?;
        // the path is remembered before an unprotected file proves it opens
        self.remember_key_path(path);
        if !file.protected {
            let key = file.open("")?;
            *self.open.lock() = Some(Open { key, path: path.to_owned() });
        }
        self.log.info(format!("[catalog] author key file: {path}"));
        Ok(())
    }

    /// The password of a protected key file; the key stays open until the editor
    /// closes.
    pub fn unlock(&self, password: &str) -> Result<(), String> {
        let path = self.current_key_path();
        let Some(text) = read_limited(&path, KEY_FILE_LIMIT) else {
            return Err(format!("no author key file at {path}"));
        };
        let key = keyfile::parse(&text)?.open(password)?;
        *self.open.lock() = Some(Open { key, path });
        Ok(())
    }

    /// Forgets the opened key: a protected file asks for its password again.
    pub fn lock(&self) {
        *self.open.lock() = None;
    }

    /// `body` followed by its `[signature]` (`Sign`). An unprotected key file opens by itself.
    pub fn sign(&self, body: &str) -> Result<String, String> {
        let state = self.state();
        if !state.exists {
            return Err("no author key: Mod > Submit to Mod Browser makes one".to_owned());
        }
        if !state.error.is_empty() {
            return Err(state.error);
        }
        if state.locked {
            return Err("the author key is locked: give its password".to_owned());
        }
        let mut open = self.open.lock();
        if !open.as_ref().is_some_and(|o| o.path == state.path) {
            let text = read_limited(&state.path, KEY_FILE_LIMIT)
                .ok_or_else(|| format!("can't read {}", state.path))?;
            let key = keyfile::parse(&text)?.open("")?;
            *open = Some(Open { key, path: state.path.clone() });
        }
        let key =
            open.as_ref().map(|o| &o.key).ok_or_else(|| "the author key could not be opened".to_owned())?;
        sign_text(body, key).map_err(|_| "Windows could not sign".to_owned())
    }

    /// The key that signs, for a caller that verifies what it signed.
    #[must_use]
    pub fn open_public(&self) -> Option<String> {
        self.open.lock().as_ref().map(|o| o.key.public.text())
    }
}

/// A key file inside the open project would travel with it - into its git repository, into
/// every build a player downloads. The build leaves `*.xkey` out anyway; this says it first.
fn inside_project(path: &str, project_root: Option<&Path>) -> Result<(), String> {
    let Some(root) = project_root else { return Ok(()) };
    let (Some(full), Some(root)) =
        (gz_project::paths::full_path(path), root.to_str().and_then(gz_project::paths::full_path))
    else {
        return Ok(());
    };
    let root = root.trim_end_matches(['\\', '/']);
    let n = root.len();
    let below = full.len() > n
        && full.get(..n).is_some_and(|h| h.eq_ignore_ascii_case(root))
        && matches!(full.as_bytes().get(n), Some(b'\\' | b'/'));
    if below {
        return Err("a key file never goes inside a project - it would ship with the module; keep it in Documents or on a stick".to_owned());
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn keys(home: &Path) -> AuthorKeys {
        AuthorKeys {
            qa: true,
            qa_home: home.to_string_lossy().into_owned(),
            log: Log::memory(),
            open: Mutex::new(None),
            home: Mutex::new(Some(home.to_string_lossy().into_owned())),
        }
    }

    #[test]
    fn create_lock_unlock_sign() {
        let base = std::env::temp_dir().join(format!("gz-release-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let project = base.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let k = keys(&base.join("home"));
        let st = k.state();
        assert!(!st.exists);
        assert!(st.path.ends_with("\\Keys\\author.xkey"));
        assert_eq!(k.sign("x\n").unwrap_err(), "no author key: Mod > Submit to Mod Browser makes one");

        let inside = project.join("k.xkey").to_string_lossy().into_owned();
        assert!(k.create(Some(&inside), "", Some(&project)).unwrap_err().contains("inside a project"));
        assert!(!project.join("k.xkey").exists());

        k.create(None, "", Some(&project)).unwrap();
        let st = k.state();
        assert!(st.usable(), "{st:?}");
        assert!(std::fs::read_to_string(base.join("home").join("catalog.ltx")).unwrap().contains("\nkey = "));
        let before = std::fs::read(&st.path).unwrap();
        assert!(
            k.create(None, "other", None).unwrap_err().ends_with("exists: a key file is never overwritten")
        );
        assert_eq!(std::fs::read(&st.path).unwrap(), before);

        let signed = k.sign("[a]\r\nb = 1\r\n").unwrap();
        assert_eq!(crate::signed::verify_text(&signed, &st.public_key).unwrap(), st.key_id);

        // an unprotected key opens by itself after a lock
        k.lock();
        assert!(!k.state().locked);
        assert!(k.sign("x\n").is_ok());

        // another copy, protected (written with few rounds to keep the test short)
        let other = EcKey::generate().unwrap();
        let path = base.join("stick").join("second.xkey");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, keyfile::text_with(&other, "pw", &[3; 16], &[4; 16], 1000)).unwrap();
        k.use_file(&path.to_string_lossy(), Some(&project)).unwrap();
        let st = k.state();
        assert!(st.locked && st.key_id == other.public.id());
        assert_eq!(k.sign("x\n").unwrap_err(), "the author key is locked: give its password");
        assert_eq!(k.unlock("wrong").unwrap_err(), "wrong password");
        k.unlock("pw").unwrap();
        assert!(!k.state().locked);
        assert!(k.sign("x\n").is_ok());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
