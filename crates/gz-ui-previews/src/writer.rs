//! The one way the libraries write: [`LibraryWriter`] is the library's folders plus the guard of
//! `gz_project::library_guard`, and every write of a library (a description, a game texture, a
//! game sound, `lanims.xr`, a picture taken in, a file renamed out of the way) is one of its
//! methods. Each checks the path first and writes only when the guard lets it through, so the
//! library may be any folder and a game is never written, whatever the folder is called.
//!
//! The methods block: they run on the IO pool, where the write happens.

use std::ops::Deref;
use std::path::Path;

use gz_editor::Editor;
use gz_project::LibraryGuard;

use crate::paths::{EditorDirs, set_file_age};

/// The library's folders and the guard its writes pass. Reads go through the folders (it derefs
/// to [`EditorDirs`]); writes only through its methods.
#[derive(Clone, Debug)]
pub struct LibraryWriter {
    dirs: EditorDirs,
    guard: LibraryGuard,
}

impl Deref for LibraryWriter {
    type Target = EditorDirs;

    fn deref(&self) -> &EditorDirs {
        &self.dirs
    }
}

impl LibraryWriter {
    /// The editor's library and the guard of the games this editor knows now. Touches no disk.
    #[must_use]
    pub fn of(editor: &Editor) -> Self {
        Self { dirs: EditorDirs::of(editor), guard: gz_project::library_guard(editor) }
    }

    /// Folders and a guard of their own.
    #[must_use]
    pub fn new(dirs: EditorDirs, guard: LibraryGuard) -> Self {
        Self { dirs, guard }
    }

    /// `Ok` when `path` may be written: the guard's sentence otherwise.
    pub fn check(&self, path: &Path) -> Result<(), String> {
        self.guard.check(path)
    }

    /// Writes a file whole, atomically (its folder is made as needed).
    pub fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        self.check(path)?;
        gz_editor::fsx::write_atomic(path, bytes)
            .map_err(|e| format!("{} could not be written: {}", path.display(), gz_base::io_reason(&e)))
    }

    /// Makes a folder and its parents.
    pub fn create_dir_all(&self, dir: &Path) -> Result<(), String> {
        self.check(dir)?;
        std::fs::create_dir_all(dir).map_err(|e| format!("{} could not be made: {e}", dir.display()))
    }

    /// Copies a file to `to`.
    pub fn copy(&self, from: &Path, to: &Path) -> Result<(), String> {
        self.check(to)?;
        std::fs::copy(from, to)
            .map(|_| ())
            .map_err(|e| format!("{} could not be copied: {e}", from.display()))
    }

    /// Renames a file: both places are checked, the one it leaves as well as the one it takes.
    pub fn rename(&self, from: &Path, to: &Path) -> Result<(), String> {
        self.check(from)?;
        self.check(to)?;
        std::fs::rename(from, to).map_err(|e| format!("{} could not be renamed: {e}", from.display()))
    }

    /// Removes a file; one that is not there is no error.
    pub fn remove_file(&self, path: &Path) -> Result<(), String> {
        self.check(path)?;
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("{} could not be removed: {e}", path.display()))
            }
            _ => Ok(()),
        }
    }

    /// Sets a file's age (seconds since 1970): the age a synchronised file is known by.
    pub fn set_age(&self, path: &Path, age: i64) -> Result<(), String> {
        self.check(path)?;
        set_file_age(path, age).map_err(|e| format!("{} could not be dated: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gz_fmt_tex::{Image, TgaDepth, write_tga};

    use super::*;
    use crate::library::{lanim, sounds, texsync};

    /// A library folder that is a game: the guard knows it, the files are the author's.
    struct FakeGame {
        _dir: tempfile::TempDir,
        root: PathBuf,
        writer: LibraryWriter,
    }

    fn put(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn tga() -> Vec<u8> {
        let image = Image::new(4, 4, vec![200; 4 * 4 * 4]).unwrap();
        write_tga(&image, TgaDepth::Bgra32).unwrap()
    }

    /// A PCM wav of a tenth of a second of silence.
    fn wav() -> Vec<u8> {
        let samples = 4410usize;
        let data_len = u32::try_from(samples * 2).unwrap();
        let mut b = b"RIFF".to_vec();
        b.extend((36 + data_len).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(44_100u32.to_le_bytes());
        b.extend(88_200u32.to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend(data_len.to_le_bytes());
        b.extend(vec![0u8; samples * 2]);
        b
    }

    fn fake_game() -> FakeGame {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("game");
        put(&root.join("fsgame.ltx"), b"x");
        put(&root.join("gamedata").join("lanims.xr"), b"not a light animation file");
        put(&root.join("rawdata").join("textures").join("qa").join("qa_swatch.tga"), &tga());
        put(&root.join("import").join("qa_new.tga"), &tga());
        put(&root.join("rawdata").join("sounds").join("qa").join("qa_tone.wav"), &wav());
        // a game sound with no source and a description with none: what a sound synchronisation
        // would move out of the way
        put(&root.join("gamedata").join("sounds").join("qa").join("qa_orphan.ogg"), b"ogg");
        put(&root.join("rawdata").join("sounds").join("qa").join("qa_orphan.thm"), b"thm");
        let writer =
            LibraryWriter::new(EditorDirs::at(&root), LibraryGuard::against([root.to_str().unwrap()]));
        FakeGame { _dir: dir, root, writer }
    }

    /// Every file under `root` with its bytes and its age: what "left as it was" means.
    fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>, Option<i64>)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(at) = stack.pop() {
            for e in std::fs::read_dir(&at).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let bytes = std::fs::read(&p).unwrap();
                    let age = crate::paths::file_age(&p);
                    out.push((p, bytes, age));
                }
            }
        }
        out.sort();
        out
    }

    fn refused(e: &str) -> bool {
        e.contains("a game is never written")
    }

    #[test]
    fn a_game_is_never_written_as_the_light_animation_library() {
        let g = fake_game();
        let before = snapshot(&g.root);
        let e = lanim::write_lanims(&g.writer, b"replaced").unwrap_err();
        assert!(refused(&e), "{e}");
        assert_eq!(snapshot(&g.root), before);
    }

    #[test]
    fn a_game_is_never_written_as_the_texture_library() {
        let g = fake_game();
        let before = snapshot(&g.root);
        let cancel = gz_task::Cancel::new();
        let report = texsync::synchronize_all(&g.writer, &cancel, |_, _, _| {}).unwrap();
        assert!(report.updated.is_empty());
        assert!(!report.failed.is_empty() && report.failed.iter().all(|f| refused(f)), "{:?}", report.failed);
        let one = texsync::sync_texture(&g.writer, "qa\\qa_swatch", None, true, true, 1).unwrap_err();
        assert!(refused(&one), "{one}");
        // taking a picture in: the copy into the library and the import file renamed
        let imported = texsync::import_picture(&g.writer, "qa_new.tga", false).unwrap_err();
        assert!(refused(&imported), "{imported}");
        let staged = texsync::import_picture(&g.writer, "qa_new.tga", true).unwrap_err();
        assert!(refused(&staged), "{staged}");
        assert_eq!(snapshot(&g.root), before);
    }

    #[test]
    fn a_game_is_never_written_as_the_sound_library() {
        let g = fake_game();
        let before = snapshot(&g.root);
        let cancel = gz_task::Cancel::new();
        let report = sounds::synchronize_sounds(&g.writer, &cancel, true).unwrap();
        assert!(report.updated.is_empty());
        assert!(!report.failed.is_empty() && report.failed.iter().all(|f| refused(f)), "{:?}", report.failed);
        let made =
            sounds::make_game_sound(&g.writer, "qa\\qa_tone", &gz_fmt_sound::SoundThm::default(), Some(1))
                .unwrap_err();
        assert!(refused(&made), "{made}");
        assert_eq!(snapshot(&g.root), before);
    }

    #[test]
    fn a_library_of_its_own_is_written() {
        let t = tempfile::tempdir().unwrap();
        let lib = t.path().join("library");
        let other = t.path().join("game");
        put(&other.join("fsgame.ltx"), b"x");
        let writer =
            LibraryWriter::new(EditorDirs::at(&lib), LibraryGuard::against([other.to_str().unwrap()]));
        lanim::write_lanims(&writer, b"mine").unwrap();
        assert_eq!(std::fs::read(lib.join("gamedata").join("lanims.xr")).unwrap(), b"mine");
        // a link into the game carries nothing across
        assert!(writer.write(&other.join("gamedata").join("lanims.xr"), b"x").is_err());
    }

    /// A picture the import staged leaves no copy; one the author put in the import folder is
    /// renamed out of the way, never deleted.
    #[test]
    fn a_staged_picture_leaves_no_copy_and_the_authors_is_kept() {
        let t = tempfile::tempdir().unwrap();
        let writer = LibraryWriter::new(EditorDirs::at(&t.path().join("library")), LibraryGuard::default());
        put(&writer.import.join("qa_staged.tga"), &tga());
        put(&writer.import.join("qa_mine.tga"), &tga());
        assert_eq!(texsync::import_picture(&writer, "qa_staged.tga", true).unwrap(), r"qa\qa_staged");
        assert_eq!(texsync::import_picture(&writer, "qa_mine.tga", false).unwrap(), r"qa\qa_mine");
        assert!(writer.textures.join("qa").join("qa_staged.tga").is_file());
        assert!(writer.textures.join("qa").join("qa_mine.tga").is_file());
        assert!(!writer.import.join("qa_staged.tga").exists());
        assert!(!writer.import.join("qa_staged.~tga").exists());
        assert!(!writer.import.join("qa_mine.tga").exists());
        assert!(writer.import.join("qa_mine.~tga").is_file());
    }
}
