//! The FROZEN release contract with the game's module updater (game repo,
//! `docs/dead-air/MOD_UPDATES.md`, section 2): the asset names, the file index, the descriptor,
//! the split into packages. It changes by adding optional keys and records and in no other way -
//! a published release has to install forever. Built as text exactly as `EditorModManifest.cpp`
//! builds it (key order, spacing, line ends): the descriptor is signed byte for byte.

use std::path::Path;

use gz_mod::{Manifest, validate_github_repo, validate_id, validate_version, validate_website};
use gz_project::paths;

/// Input bytes per package. Deflate never grows data by more than a fraction of a percent, so a
/// package stays clear of VirusTotal's 650 MB - the Mod Browser lists only packages VirusTotal
/// knows - and of GitHub's asset limit alike; a single file over this still gets a package of
/// its own.
pub const PART_INPUT_LIMIT: u64 = 512 << 20;
/// GitHub refuses a release asset of 2 GiB.
pub const ASSET_LIMIT: u64 = 2048 << 20;
/// 2.2: the game reads an index of 64 MiB at most.
pub const INDEX_LIMIT: u64 = 64 << 20;
pub const MAX_PACKAGES: usize = 64;
pub const MAX_ASSET_NAME: usize = 128;

/// `<id>.update.ltx`.
#[must_use]
pub fn descriptor_name(id: &str) -> String {
    format!("{id}.update.ltx")
}

/// `<id>-<version>.files`.
#[must_use]
pub fn index_name(id: &str, version: &str) -> String {
    format!("{id}-{version}.files")
}

/// `<id>-<version>.zip`, or `<id>-<version>.part<n>.zip` (1-based) when the module splits.
#[must_use]
pub fn package_name(id: &str, version: &str, part: usize, parts: usize) -> String {
    if parts > 1 { format!("{id}-{version}.part{part}.zip") } else { format!("{id}-{version}.zip") }
}

/// `<stem>.zip` or `<stem>.part<N>.zip`, any case - matched whole, so `test-1.0` never claims a
/// package of `test-1.0.1` that shares the folder (`IsPackageName`).
#[must_use]
pub fn is_package_name(name: &str, stem: &str) -> bool {
    let Some(head) = name.get(..stem.len()) else { return false };
    if !head.eq_ignore_ascii_case(stem) {
        return false;
    }
    let tail = name.get(stem.len()..).unwrap_or("");
    if tail.eq_ignore_ascii_case(".zip") {
        return true;
    }
    let Some(rest) = tail.get(..5).filter(|p| p.eq_ignore_ascii_case(".part")).and_then(|_| tail.get(5..)) else { return false };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && rest.get(digits..).is_some_and(|z| z.eq_ignore_ascii_case(".zip"))
}

/// Whether a folder entry is a file an earlier packaging of this id and version left: the
/// descriptor, the file index or a package (`ListReleaseFiles`).
#[must_use]
pub fn is_release_file(name: &str, id: &str, version: &str) -> bool {
    let stem = format!("{id}-{version}");
    name.eq_ignore_ascii_case(&descriptor_name(id)) || name.eq_ignore_ascii_case(&index_name(id, version)) || is_package_name(name, &stem)
}

/// MOD_UPDATES 2.2: what a path below `modules/<id>/` may look like. The game refuses a whole
/// package over one bad name, so the editor refuses first (`PackagePathOk`).
#[must_use]
pub fn package_path_ok(path: &str) -> bool {
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' ']) {
            return false;
        }
        if part.bytes().any(|c| c < 0x20 || c == 0x7F || b"<>:\"|?*\\".contains(&c)) {
            return false;
        }
        // device names are reserved with an extension too: nul, nul.txt
        let stem = part.split('.').next().unwrap_or("").to_ascii_lowercase();
        if matches!(stem.as_str(), "con" | "prn" | "aux" | "nul") {
            return false;
        }
        let b = stem.as_bytes();
        if b.len() == 4 && (stem.starts_with("com") || stem.starts_with("lpt")) && b.get(3).is_some_and(|c| (b'1'..=b'9').contains(c)) {
            return false;
        }
    }
    true
}

/// Everything that decides whether this manifest can be released at all (`ReleaseGate`).
pub fn release_gate(m: &Manifest) -> Result<(), String> {
    if !validate_id(&m.id) {
        return Err("module id missing or invalid in mod.ltx (allowed: a-z 0-9 _ . -)".to_owned());
    }
    if !validate_version(&m.version) {
        return Err("a release needs an orderable version: 1-4 numbers separated by dots, e.g. 1.2.0".to_owned());
    }
    if !validate_website(&m.website) {
        return Err("invalid website in mod.ltx: only https://ap-pro.ru or https://moddb.com links are allowed".to_owned());
    }
    if !validate_github_repo(&m.update_github) {
        return Err("invalid [update] github in mod.ltx: owner/repo expected".to_owned());
    }
    // 2.1: an asset name - package or index - is 1-128 characters of a charset id and version
    // stay inside, and does not start with a dot. The longest name a release can need is the
    // last part's; "<id>-<version>.files" is shorter.
    if m.id.starts_with('.') {
        return Err("module id starts with a dot - a release asset name cannot".to_owned());
    }
    if m.id.len() + m.version.len() + "-.part64.zip".len() > MAX_ASSET_NAME {
        return Err("module id and version are too long for a release asset name (128 characters)".to_owned());
    }
    Ok(())
}

/// `requires_game`: MAJOR.MINOR.PATCH, or empty for no requirement.
pub fn check_requires_game(v: &str) -> Result<(), String> {
    if v.is_empty() || gz_mod::rules::dotted_number(v, 3, 3) {
        Ok(())
    } else {
        Err("invalid requires_game: MAJOR.MINOR.PATCH expected, e.g. 1.4.1".to_owned())
    }
}

/// `<project>\releases\<version>` by default; a relative folder is taken against the project,
/// never the process's working directory. Inside the project only `releases\` is allowed:
/// anywhere else the packages would ship inside the next build (`ResolveReleaseDir`).
pub fn resolve_release_dir(project_root: &Path, out_dir: &str, version: &str) -> Result<String, String> {
    let root_text = project_root.to_string_lossy().into_owned();
    let asked = gz_build::target::normalize_path(out_dir);
    let b = asked.as_bytes();
    let absolute = (b.len() > 1 && b.get(1) == Some(&b':')) || asked.starts_with("\\\\");
    let joined = if asked.is_empty() {
        format!("{root_text}\\releases\\{version}")
    } else if absolute {
        asked.clone()
    } else {
        format!("{root_text}\\{asked}")
    };
    let invalid = || "invalid output folder".to_owned();
    let dst = paths::full_path(&joined).ok_or_else(invalid)?;
    let mut root = paths::full_path(&root_text).ok_or_else(invalid)?;
    while root.ends_with('\\') {
        root.pop();
    }
    let n = root.len();
    let inside = dst.get(..n).is_some_and(|h| h.eq_ignore_ascii_case(&root)) && matches!(dst.as_bytes().get(n), None | Some(b'\\'));
    if !inside {
        return Ok(dst);
    }
    let rest = dst.get(n..).unwrap_or("");
    let releases = rest.get(..9).is_some_and(|r| r.eq_ignore_ascii_case("\\releases")) && matches!(rest.as_bytes().get(9), None | Some(b'\\'));
    if releases {
        return Ok(dst);
    }
    Err("output folder is inside the project content - use <project>\\releases or a folder outside the project".to_owned())
}

/// `[first, last)` of every package over files in package order; a file over the limit ends up
/// alone in its own.
#[must_use]
pub fn split_parts(sizes: &[u64]) -> Vec<(usize, usize)> {
    let mut parts = Vec::new();
    let (mut bytes, mut first) = (0u64, 0usize);
    for (i, &size) in sizes.iter().enumerate() {
        if i > first && bytes + size > PART_INPUT_LIMIT {
            parts.push((first, i));
            first = i;
            bytes = 0;
        }
        bytes += size;
    }
    parts.push((first, sizes.len()));
    parts
}

/// One record of the file index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IndexRecord {
    pub sha256: String,
    pub size: u64,
    /// 1-based package number.
    pub pack: u32,
    /// The first byte of the stored data inside the package.
    pub offset: u64,
    /// The length of that data.
    pub packed: u64,
    /// 0 stored, 8 raw deflate.
    pub method: u32,
    /// Module-relative, forward slashes.
    pub path: String,
}

/// The file index (2.2): `pack` lines in the descriptor's `[packages]` order, `file` lines in
/// package order; LF line ends. Nothing in it depends on when the packaging ran, so the same
/// module always yields the same bytes.
#[must_use]
pub fn index_text(packages: &[String], files: &[IndexRecord]) -> String {
    let mut out = String::from("xms-files 1\n");
    for p in packages {
        out.push_str("pack ");
        out.push_str(p);
        out.push('\n');
    }
    for f in files {
        out.push_str(&format!("file {} {} {} {} {} {} {}\n", f.sha256, f.size, f.pack, f.offset, f.packed, f.method, f.path));
    }
    out
}

/// An asset of the release: its name, size and SHA-256.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

fn key(out: &mut String, key: &str, value: &str) {
    out.push_str(&format!("{key:<13} = {value}\r\n"));
}

/// What the descriptor's `[release]` states.
#[derive(Clone, Debug, Default)]
pub struct ReleaseFacts<'a> {
    pub id: &'a str,
    pub version: &'a str,
    /// Empty: no requirement, no line.
    pub requires_game: &'a str,
    pub files: usize,
    pub unpacked: u64,
}

/// The descriptor up to the end of `[packages]` (2.1, `FinishRelease`); the card and the
/// signature follow.
#[must_use]
pub fn descriptor_head(r: &ReleaseFacts<'_>, index: &Asset, packages: &[Asset]) -> String {
    let mut text = String::from("; XMS module release descriptor. Generated by Grizzly Editor.\r\n[release]\r\n");
    key(&mut text, "schema", "1");
    key(&mut text, "id", r.id);
    key(&mut text, "version", r.version);
    if !r.requires_game.is_empty() {
        key(&mut text, "requires_game", r.requires_game);
    }
    key(&mut text, "files", &r.files.to_string());
    key(&mut text, "unpacked", &r.unpacked.to_string());
    key(&mut text, "index", &format!("{}, {}, {}", index.name, index.size, index.sha256));
    text.push_str("\r\n[packages]\r\n");
    for p in packages {
        text.push_str(&format!("{} = {}, {}\r\n", p.name, p.size, p.sha256));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_mod::ManifestDoc;

    #[test]
    fn names() {
        assert_eq!(package_name("od", "1.0.0", 1, 1), "od-1.0.0.zip");
        assert_eq!(package_name("od", "1.0.0", 2, 3), "od-1.0.0.part2.zip");
        assert!(is_package_name("OD-1.0.ZIP", "od-1.0"));
        assert!(is_package_name("od-1.0.part12.zip", "od-1.0"));
        assert!(!is_package_name("od-1.0.1.zip", "od-1.0"));
        assert!(!is_package_name("od-1.0.part.zip", "od-1.0"));
        assert!(!is_package_name("od-1.0.part1x.zip", "od-1.0"));
        assert!(is_release_file("od.update.ltx", "od", "1.0"));
        assert!(is_release_file("od-1.0.files", "od", "1.0"));
        assert!(!is_release_file("od-1.0.1.files", "od", "1.0"));
    }

    #[test]
    fn paths_the_updater_takes() {
        assert!(package_path_ok("gamedata/configs/x.ltx"));
        assert!(package_path_ok("\u{43c}\u{43e}\u{434}/\u{444}.ltx"));
        assert!(package_path_ok("console.txt"));
        assert!(package_path_ok("com10.txt"));
        for bad in ["a//b", "a/./b", "x/..", "a./b", "a /b", "nul", "NUL.txt", "com1", "lpt9.dds", "a:b", "a\\b", "a\u{1}b"] {
            assert!(!package_path_ok(bad), "{bad}");
        }
    }

    #[test]
    fn gate_and_parts() {
        let m = |t: &str| Manifest::parse(&ManifestDoc::parse(t));
        assert!(release_gate(&m("[module]\nid = od\nversion = 1.0.0\n")).is_ok());
        assert_eq!(release_gate(&m("[module]\nid = od\nversion = beta\n")).unwrap_err(), "a release needs an orderable version: 1-4 numbers separated by dots, e.g. 1.2.0");
        assert_eq!(release_gate(&m("[module]\nversion = 1\n")).unwrap_err(), "module id missing or invalid in mod.ltx (allowed: a-z 0-9 _ . -)");
        let long = format!("[module]\nid = {}\nversion = 1.0.0\n", "a".repeat(112));
        assert_eq!(release_gate(&m(&long)).unwrap_err(), "module id and version are too long for a release asset name (128 characters)");
        assert!(check_requires_game("").is_ok());
        assert!(check_requires_game("1.4.2").is_ok());
        assert!(check_requires_game("1.4").is_err());

        assert_eq!(split_parts(&[1, 2, 3]), [(0, 3)]);
        let half = PART_INPUT_LIMIT / 2 + 1;
        assert_eq!(split_parts(&[half, half, half]), [(0, 1), (1, 2), (2, 3)]);
        assert_eq!(split_parts(&[PART_INPUT_LIMIT * 3, 1]), [(0, 1), (1, 2)]);
        assert_eq!(split_parts(&[]), [(0, 0)]);
    }

    #[test]
    fn index_and_descriptor_text() {
        let files = [
            IndexRecord { sha256: "aa".into(), size: 394, pack: 1, offset: 66, packed: 214, method: 8, path: "mod.ltx".into() },
            IndexRecord { sha256: "bb".into(), size: 0, pack: 1, offset: 400, packed: 2, method: 8, path: "a b/c.txt".into() },
        ];
        assert_eq!(index_text(&["od-1.2.0.zip".to_owned()], &files), "xms-files 1\npack od-1.2.0.zip\nfile aa 394 1 66 214 8 mod.ltx\nfile bb 0 1 400 2 8 a b/c.txt\n");
        let facts = ReleaseFacts { id: "od", version: "1.2.0", requires_game: "1.4.2", files: 2, unpacked: 394 };
        let index = Asset { name: "od-1.2.0.files".into(), size: 99, sha256: "cc".into() };
        let pkgs = [Asset { name: "od-1.2.0.zip".into(), size: 777, sha256: "dd".into() }];
        assert_eq!(
            descriptor_head(&facts, &index, &pkgs),
            "; XMS module release descriptor. Generated by Grizzly Editor.\r\n\
             [release]\r\n\
             schema        = 1\r\n\
             id            = od\r\n\
             version       = 1.2.0\r\n\
             requires_game = 1.4.2\r\n\
             files         = 2\r\n\
             unpacked      = 394\r\n\
             index         = od-1.2.0.files, 99, cc\r\n\
             \r\n[packages]\r\n\
             od-1.2.0.zip = 777, dd\r\n"
        );
        let none = ReleaseFacts { requires_game: "", ..facts };
        assert!(!descriptor_head(&none, &index, &pkgs).contains("requires_game"));
    }

    #[test]
    fn the_release_folder() {
        let root = std::env::temp_dir().join("gz-release-dir").join("proj");
        let r = root.to_string_lossy().into_owned();
        assert_eq!(resolve_release_dir(&root, "", "1.0.0").unwrap(), format!("{r}\\releases\\1.0.0"));
        assert_eq!(resolve_release_dir(&root, "releases/x", "1.0.0").unwrap(), format!("{r}\\releases\\x"));
        assert!(resolve_release_dir(&root, "gamedata", "1.0.0").unwrap_err().starts_with("output folder is inside the project content"));
        assert!(resolve_release_dir(&root, "releases_old", "1.0.0").is_err());
        assert!(resolve_release_dir(&root, "..\\out", "1.0.0").unwrap().ends_with("gz-release-dir\\out"));
        assert_eq!(resolve_release_dir(&root, "D:/Out//", "1.0.0").unwrap(), "D:\\Out");
    }
}
