use super::*;
use gz_chunk::{COMPRESSED, Writer, lzhuf, lzo};

struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("gz-vfs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn put(&self, rel: &str, bytes: &[u8]) {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An archive as the game's packer writes one: the stored files in chunk 0, the directory
/// LZHUF-packed in chunk 1. `pack` stores a file LZO-compressed.
fn archive(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
    let mut body = Vec::new();
    let mut table = Writer::new();
    for (name, bytes, pack) in files {
        let stored = if *pack { lzo::compress(bytes) } else { bytes.to_vec() };
        let offset = 8 + body.len() as u32;
        body.extend_from_slice(&stored);
        let name = gz_base::text::utf8_to_cp1251(name).0.into_owned();
        table.w_u16((16 + name.len()) as u16);
        table.w_u32(bytes.len() as u32);
        table.w_u32(stored.len() as u32);
        table.w_u32(0);
        table.w(&name);
        table.w_u32(offset);
    }
    let mut w = Writer::new();
    w.w_chunk(0, &body);
    w.w_chunk(1 | COMPRESSED, &lzhuf::compress(table.bytes()));
    w.into_bytes()
}

const FSGAME: &str =
    "$arch_dir$ = false| false| $fs_root$| database\\\r\n$game_data$ = true| true| $fs_root$| gamedata\\\r\n";

fn ogf() -> Vec<u8> {
    (0..4000u32).map(|i| (i % 13) as u8).collect()
}

fn game(t: &Temp) {
    t.put("fsgame.ltx", FSGAME.as_bytes());
    t.put(
        "database/a.db0",
        &archive(&[
            ("configs\\", b"", false),
            ("configs\\System.ltx", b"packed", false),
            ("meshes\\A.ogf", &ogf(), true),
            ("textures\\t.dds", b"old", false),
        ]),
    );
    t.put("database/b.db1", &archive(&[("textures\\t.dds", b"newer", false)]));
    t.put("database/empty.db2", b"");
    t.put("database/notes.txt", b"not an archive");
    t.put("gamedata/configs/system.ltx", b"loose");
    t.put("gamedata/.git/config", b"tooling");
    t.put("modules/MyMod/mod.ltx", b"[module]\r\n");
    t.put("modules/MyMod/configs/x.ltx", b"x");
    t.put("mods/old/mod.ltx", b"[module]\r\n");
}

#[test]
fn archive_index_round_trip() {
    let t = Temp::new("archive");
    let bytes: Vec<u8> = (0..20_000u32).map(|i| (i * 7 % 251) as u8).collect();
    t.put(
        "x.db0",
        &archive(&[
            ("a\\b.bin", &bytes, true),
            ("Кириллица.txt", b"text", false),
            ("a\\b.bin", b"later", false),
        ]),
    );
    let entries = archive::read_directory(&t.0.join("x.db0")).unwrap();
    // a later record of a path replaces the earlier one in place
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].path, "a/b.bin");
    assert_eq!(entries[1].path, "кириллица.txt");
    let mut open = archive::Open::new(&t.0.join("x.db0")).unwrap();
    let e = &entries[0];
    assert_eq!(open.read(e.offset, e.packed, e.size).unwrap(), b"later");
    let e = &entries[1];
    assert_eq!(open.read(e.offset, e.packed, e.size).unwrap(), b"text");
    assert!(open.read(u32::MAX - 2, 10, 10).is_err());

    t.put("y.db0", &archive(&[("big.bin", &bytes, true)]));
    let entries = archive::read_directory(&t.0.join("y.db0")).unwrap();
    let e = &entries[0];
    assert!(e.packed < e.size);
    let mut open = archive::Open::new(&t.0.join("y.db0")).unwrap();
    assert_eq!(open.read(e.offset, e.packed, e.size).unwrap(), bytes);
    // a size the packed bytes cannot hold is refused before anything is allocated for it
    assert!(open.read(e.offset, e.packed, u32::MAX).is_err());

    t.put("z.db0", b"no chunks here");
    assert!(archive::read_directory(&t.0.join("z.db0")).is_err());
}

#[test]
fn precedence() {
    let t = Temp::new("precedence");
    game(&t);
    let files = GameFiles::mount(&t.0, &MountOptions::default()).unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f.path).collect();
    // the game's own loose tree keeps a tooling name, as the C++ DARF content listed it
    assert_eq!(paths, [".git/config", "configs/system.ltx", "meshes/a.ogf", "textures/t.dds"]);
    assert_eq!(files.archives().len(), 2);
    assert!(files.skipped().is_empty());

    // loose over the archives, a later archive over an earlier one
    assert_eq!(files.get("Configs/System.LTX").unwrap().source, Source::Loose);
    assert_eq!(files.read("configs\\system.ltx").unwrap(), b"loose");
    assert_eq!(files.read("textures/t.dds").unwrap(), b"newer");
    assert!(matches!(files.get("textures/t.dds").unwrap().source, Source::Archive { archive: 1, .. }));
    assert_eq!(files.read("meshes/a.ogf").unwrap(), ogf());
    assert!(files.read("nothing").is_err());

    let many = files.read_many(&["textures/t.dds", "missing", "configs/system.ltx", "meshes/a.ogf"]);
    assert_eq!(many[0].as_deref().unwrap(), b"newer");
    assert!(many[1].is_err());
    assert_eq!(many[2].as_deref().unwrap(), b"loose");
    assert_eq!(many[3].as_deref().unwrap(), ogf().as_slice());

    let roots: Vec<(ModuleRoot, &str)> = files.modules().iter().map(|m| (m.root, m.id.as_str())).collect();
    assert_eq!(roots, [(ModuleRoot::Modules, "MyMod"), (ModuleRoot::Mods, "old")]);
    let m = files.module(ModuleRoot::Modules, "mymod").unwrap();
    assert!(m.exists("MOD.LTX"));
    assert_eq!(m.read("configs/x.ltx").unwrap(), b"x");
    assert_eq!(m.find("", "ltx").len(), 2);
}

#[test]
fn saved_index_is_checked_by_stamps() {
    let t = Temp::new("cache");
    game(&t);
    let cache = Temp::new("cache-dir");
    let opts = MountOptions { cache_dir: Some(cache.0.clone()) };

    let first = GameFiles::mount(&t.0, &opts).unwrap();
    assert!(!first.stats().from_cache);
    assert_eq!(first.stats().archives_read, 2);
    assert!(first.stats().cache_written);

    let second = GameFiles::mount(&t.0, &opts).unwrap();
    let s = second.stats();
    assert!(s.from_cache);
    assert_eq!((s.archives_read, s.archives_reused, s.dirs_read), (0, 2, 0));
    assert!(s.dirs_checked > 0);
    assert!(!s.cache_written);
    let keys = |f: &GameFiles| f.iter().map(|r| r.path.to_owned()).collect::<Vec<_>>();
    assert_eq!(keys(&second), keys(&first));

    // a new file in one folder and a rewritten archive: only those are read again
    t.put("gamedata/configs/new.ltx", b"n");
    t.put(
        "database/b.db1",
        &archive(&[("textures\\t.dds", b"newest!", false), ("textures\\u.dds", b"u", false)]),
    );
    let third = second.refresh(&opts).unwrap();
    let s = third.stats();
    assert_eq!((s.archives_read, s.archives_reused), (1, 1));
    assert!(s.dirs_read >= 1);
    assert!(s.cache_written);
    assert!(third.exists("configs/new.ltx"));
    assert_eq!(third.read("textures/t.dds").unwrap(), b"newest!");
    assert!(third.exists("textures/u.dds"));

    // a damaged saved index is no index
    let file = cache::file_for(&cache.0, &normalize(&t.0.to_string_lossy()));
    std::fs::write(&file, b"GZGI\x01\0\0\0garbage").unwrap();
    let fourth = GameFiles::mount(&t.0, &opts).unwrap();
    assert!(!fourth.stats().from_cache);
    assert_eq!(keys(&fourth), keys(&third));
}

#[test]
fn not_a_game() {
    let t = Temp::new("nogame");
    assert_eq!(
        GameFiles::mount(&t.0, &MountOptions::default()).unwrap_err(),
        MountError::NotAGame("no fsgame.ltx - not an X-Ray game folder")
    );
}
