//! What a report says of the machine it comes from: nothing that names the person or the computer
//! (docs/REPORTS.md, "Privacy"). Every text of a report goes through [`Sanitizer::clean`]: the user's
//! and the computer's names, the folders of this machine (the profile, the application data, the
//! temp folder, the editor's, the project's, the game's) become markers, and so do e-mail addresses,
//! network addresses and what looks like a key, a token or a password. The structure of the text
//! stays: a path keeps what lies below the folder it names.

use std::path::Path;

/// The replacements of one machine, longest first.
#[derive(Clone, Debug, Default)]
pub struct Sanitizer {
    /// (lower-cased text, marker): folders, matched in either slash and any case.
    folders: Vec<(String, &'static str)>,
    /// (lower-cased word, marker): names, matched as whole words.
    words: Vec<(String, &'static str)>,
}

/// One folder of this machine and its marker.
pub struct Folder<'a> {
    pub path: &'a Path,
    pub marker: &'static str,
}

impl Sanitizer {
    /// The sanitizer of this machine: its user and computer names from the environment, its folders
    /// from the environment and the editor's own (`extra`: the editor, its home, the project, the game).
    #[must_use]
    pub fn of_machine(extra: &[Folder<'_>]) -> Self {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let mut s = Self::default();
        for f in extra {
            s.add_folder(f.path, f.marker);
        }
        for (key, marker) in [
            ("TEMP", "<temp>"),
            ("TMP", "<temp>"),
            ("LOCALAPPDATA", "<localappdata>"),
            ("APPDATA", "<appdata>"),
            ("OneDrive", "<onedrive>"),
            ("OneDriveCommercial", "<onedrive>"),
            ("OneDriveConsumer", "<onedrive>"),
            ("USERPROFILE", "<profile>"),
            ("HOMEPATH", "<profile>"),
        ] {
            if let Some(v) = env(key) {
                s.add_folder(Path::new(&v), marker);
            }
        }
        for (key, marker) in
            [("USERNAME", "<user>"), ("COMPUTERNAME", "<computer>"), ("USERDOMAIN", "<domain>")]
        {
            if let Some(v) = env(key) {
                s.add_word(&v, marker);
            }
        }
        // what the system says, whatever the environment says: the shell's folders of this user, the
        // profile's own name (the user's, mostly), the computer's names
        #[cfg(windows)]
        {
            use gz_win::KnownFolder as K;
            for (folder, marker) in [
                (K::Documents, "<documents>"),
                (K::Desktop, "<desktop>"),
                (K::Downloads, "<downloads>"),
                (K::LocalAppData, "<localappdata>"),
                (K::RoamingAppData, "<appdata>"),
                (K::Profile, "<profile>"),
            ] {
                if let Some(path) = gz_win::known_folder(folder) {
                    s.add_folder(&path, marker);
                    if folder == K::Profile
                        && let Some(name) = path.file_name()
                    {
                        s.add_word(&name.to_string_lossy(), "<user>");
                    }
                }
            }
            for name in gz_win::computer_names() {
                s.add_word(&name, "<computer>");
            }
        }
        s
    }

    /// A folder to replace by `marker`, below it as well.
    pub fn add_folder(&mut self, path: &Path, marker: &'static str) {
        let text = fold(path.to_string_lossy().trim_end_matches(['\\', '/']));
        // a drive root or a folder of a letter or two would take whole words of the text with it
        if text.len() < 4 {
            return;
        }
        for form in [text.replace('/', "\\"), text.replace('\\', "/")] {
            if !self.folders.iter().any(|(f, _)| *f == form) {
                self.folders.push((form, marker));
            }
        }
        self.folders.sort_by_key(|(f, _)| std::cmp::Reverse(f.len()));
    }

    /// A name to replace by `marker` wherever it stands as a word of its own.
    pub fn add_word(&mut self, word: &str, marker: &'static str) {
        let word = fold(word.trim());
        // a name of a letter or two is in every word
        if word.chars().count() < 3 || self.words.iter().any(|(w, _)| *w == word) {
            return;
        }
        self.words.push((word, marker));
        self.words.sort_by_key(|(w, _)| std::cmp::Reverse(w.len()));
    }

    /// `text` with everything that names this machine or a person replaced.
    #[must_use]
    pub fn clean(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for (folder, marker) in &self.folders {
            out = replace_folder(&out, folder, marker);
        }
        // what is left of a path goes before the names: a name inside one would end it early
        out = mask_paths(&out);
        out = mask_github_owners(&out);
        for (word, marker) in &self.words {
            out = replace_word(&out, word, marker);
        }
        let out = mask_secrets(&out);
        let out = mask_emails(&out);
        mask_addresses(&out)
    }

    /// Every string of `value`, the keys of its objects too, cleaned.
    pub fn clean_value(&self, value: &mut serde_json::Value) {
        use serde_json::Value;
        match value {
            Value::String(s) => *s = self.clean(s),
            Value::Array(items) => items.iter_mut().for_each(|v| self.clean_value(v)),
            Value::Object(map) => {
                let taken = std::mem::take(map);
                for (k, mut v) in taken {
                    self.clean_value(&mut v);
                    map.insert(self.clean(&k), v);
                }
            }
            _ => {}
        }
    }
}

/// An absolute path no folder above named: `<path>` and its last name (`D:\Mods\x\a.txt` becomes
/// `<path>\a.txt`), a server's share too. A path ends where no path goes on: a quote, a bracket, a
/// colon past the drive's, the end of the line; its last name ends at a space.
fn mask_paths(text: &str) -> String {
    let b = text.as_bytes();
    let sep = |c: u8| c == b'\\' || c == b'/';
    let boundary =
        |i: usize| i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_' || sep(b[i - 1]));
    let mut out = String::with_capacity(text.len());
    let (mut copied, mut i) = (0, 0);
    while i + 2 < b.len() {
        let drive = b[i].is_ascii_alphabetic() && b[i + 1] == b':' && sep(b[i + 2]);
        // a drive in the long form (`\\?\D:\`) starts at its prefix; one in a `file:///` URL keeps it
        let long = drive && i >= 4 && &b[i - 4..i] == b"\\\\?\\";
        let url = drive && b[..i].ends_with(b"file:///");
        // a share: two backslashes or more (a log may write them escaped) before a name
        let run = if b[i] == b'\\' && (i == 0 || b[i - 1] != b'\\') {
            b[i..].iter().take_while(|&&c| c == b'\\').count()
        } else {
            0
        };
        let share = run >= 2 && b.get(i + run).is_some_and(u8::is_ascii_alphanumeric);
        if !(drive || share) || !(boundary(i) || long || url) {
            i += 1;
            continue;
        }
        let start = if long { i - 4 } else { i };
        let end = (i + 3..b.len())
            .find(|&j| {
                matches!(
                    b[j],
                    b'"' | b'\''
                        | b'<'
                        | b'>'
                        | b'|'
                        | b'\n'
                        | b'\r'
                        | b'\t'
                        | b','
                        | b';'
                        | b':'
                        | b'('
                        | b')'
                        | b'['
                        | b']'
                        | b'{'
                        | b'}'
                        | b'*'
                        | b'?'
                        | b'`'
                )
            })
            .unwrap_or(b.len());
        let Some(last) = (i + 2..end).rev().find(|&j| sep(b[j]) && !(share && j < i + run)) else {
            i += 1;
            continue;
        };
        let name_end = (last + 1..end).find(|&j| b[j] == b' ').unwrap_or(end);
        out.push_str(&text[copied..start]);
        out.push_str("<path>");
        out.push_str(&text[last..name_end]);
        copied = name_end;
        i = name_end.max(i + 1);
    }
    out.push_str(&text[copied..]);
    out
}

/// A GitHub account in an address (`github.com/<owner>/`, `api.github.com/repos/<owner>/`,
/// `uploads.github.com/repos/<owner>/`) becomes `<owner>`: whoever published says who they are there.
fn mask_github_owners(text: &str) -> String {
    let lower = fold(text);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(found) = lower[at..].find("github.com/") {
        let mut start = at + found + "github.com/".len();
        if lower[start..].starts_with("repos/") {
            start += "repos/".len();
        }
        let end = lower[start..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .map_or(lower.len(), |e| start + e);
        out.push_str(&text[at..start]);
        if end > start {
            out.push_str("<owner>");
        }
        at = end.max(start);
    }
    out.push_str(&text[at..]);
    out
}

/// `text` in lower case letter by letter, every letter keeping its length in bytes: a place found in
/// the folded text is the same place in the text.
fn fold(text: &str) -> String {
    text.chars()
        .map(|c| {
            let mut lower = c.to_lowercase();
            match (lower.next(), lower.next()) {
                (Some(l), None) if l.len_utf8() == c.len_utf8() => l,
                _ => c,
            }
        })
        .collect()
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Every place `folder` (lower case) stands in `text`, any case, followed by the end of the text or by
/// something that is no letter of a longer name.
fn replace_folder(text: &str, folder: &str, marker: &str) -> String {
    let lower = fold(text);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(found) = lower[at..].find(folder) {
        let start = at + found;
        let end = start + folder.len();
        let next = lower[end..].chars().next();
        if next.is_some_and(|c| is_word_char(c) || c == '.' || c == '-') {
            out.push_str(&text[at..end]);
        } else {
            out.push_str(&text[at..start]);
            out.push_str(marker);
        }
        at = end;
    }
    out.push_str(&text[at..]);
    out
}

/// Every place `word` (lower case) stands in `text` as a word of its own, any case. An underscore ends a
/// word here: a file named after the user (`grizzly_ivan.log`) is the user's name too.
fn replace_word(text: &str, word: &str, marker: &str) -> String {
    let is_word_char = char::is_alphanumeric;
    let lower = fold(text);
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(found) = lower[at..].find(word) {
        let start = at + found;
        let end = start + word.len();
        let before = lower[..start].chars().next_back();
        let after = lower[end..].chars().next();
        if before.is_some_and(is_word_char) || after.is_some_and(is_word_char) {
            out.push_str(&text[at..end]);
        } else {
            out.push_str(&text[at..start]);
            out.push_str(marker);
        }
        at = end;
    }
    out.push_str(&text[at..]);
    out
}

/// The value after a word that names a secret (`token=`, `password: `, `Bearer `), and the keys of the
/// services the editor speaks to (`ghp_...`, `github_pat_...`).
fn mask_secrets(text: &str) -> String {
    const NAMES: [&str; 9] = [
        "token",
        "password",
        "passwd",
        "secret",
        "api_key",
        "apikey",
        "authorization",
        "bearer",
        "private_key",
    ];
    const PREFIXES: [&str; 5] = ["ghp_", "gho_", "ghs_", "ghu_", "github_pat_"];
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let mut line = line.to_owned();
        for prefix in PREFIXES {
            while let Some(at) = line.find(prefix) {
                let end = line[at..]
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .map_or(line.len(), |e| at + e);
                line.replace_range(at..end, "<secret>");
            }
        }
        for name in NAMES {
            let mut from = 0;
            loop {
                let lower = fold(&line);
                let Some(found) = lower[from..].find(name) else { break };
                let after = from + found + name.len();
                let rest = &line[after..];
                let skip = rest.len() - rest.trim_start_matches([' ', '=', ':', '"', '\'']).len();
                let end_of = |line: &str, from: usize| {
                    line[from..]
                        .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == ',' || c == ';')
                        .map_or(line.len(), |e| from + e)
                };
                let mut value_start = after + skip;
                let mut value_end = end_of(&line, value_start);
                // `Authorization: Bearer <key>`: the scheme is no secret, the word after it is
                let scheme = &line[value_start..value_end];
                if name == "authorization"
                    && ["bearer", "basic", "token", "digest"].iter().any(|w| scheme.eq_ignore_ascii_case(w))
                {
                    let rest = &line[value_end..];
                    value_start = value_end + (rest.len() - rest.trim_start_matches(' ').len());
                    value_end = end_of(&line, value_start);
                }
                // a value after `=` or `:` of four characters or more is a secret; after a space alone only one
                // of eight or more - "the token expired" is no key
                let strong = line[after..after + skip].contains(['=', ':']);
                let least = if strong { 4 } else { 8 };
                if skip > 0
                    && value_end >= value_start + least
                    && !line[value_start..value_end].starts_with('<')
                {
                    line.replace_range(value_start..value_end, "<secret>");
                    from = value_start + "<secret>".len();
                } else {
                    from = after;
                }
            }
        }
        out.push_str(&line);
    }
    out
}

/// `name@domain.tld` becomes `<email>`.
fn mask_emails(text: &str) -> String {
    let part = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+' | '%');
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        let start =
            rest[..at].char_indices().rev().take_while(|(_, c)| part(*c)).last().map_or(at, |(i, _)| i);
        let end = rest[at + 1..].find(|c: char| !part(c)).map_or(rest.len(), |e| at + 1 + e);
        let domain = &rest[at + 1..end];
        if start < at && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') {
            out.push_str(&rest[..start]);
            out.push_str("<email>");
        } else {
            out.push_str(&rest[..end.max(at + 1)]);
        }
        rest = &rest[end.max(at + 1)..];
    }
    out.push_str(rest);
    out
}

/// An IPv4 address (four numbers of 0 to 255 joined by dots, no fifth) becomes `<ip>`; loopback stays.
fn mask_addresses(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut copied = 0;
    while i < bytes.len() {
        let boundary = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'.');
        if boundary && bytes[i].is_ascii_digit() {
            let mut j = i;
            let mut parts = 0;
            let mut ok = true;
            loop {
                let s = j;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                let n: u32 = text[s..j].parse().unwrap_or(999);
                if j == s || j - s > 3 || n > 255 {
                    ok = false;
                    break;
                }
                parts += 1;
                if parts == 4 || j >= bytes.len() || bytes[j] != b'.' {
                    break;
                }
                j += 1;
            }
            let after = bytes.get(j).copied();
            let closed = !after.is_some_and(|b| {
                b.is_ascii_alphanumeric() || b == b'.' && bytes.get(j + 1).is_some_and(u8::is_ascii_digit)
            });
            if ok && parts == 4 && closed {
                let address = &text[i..j];
                // a version of four numbers is no address: a number written with a leading zero
                // (`23.20.11.01`), one that starts with 0, one named a version, a build or a driver
                let leading_zero = address.split('.').any(|p| p.len() > 1 && p.starts_with('0'));
                let named = fold(text[..i].trim_end())
                    .rsplit(|c: char| !c.is_alphanumeric())
                    .next()
                    .is_some_and(|w| matches!(w, "version" | "ver" | "build" | "driver" | "drivers"));
                if !address.starts_with("127.") && !address.starts_with("0.") && !leading_zero && !named {
                    out.push_str(&text[copied..i]);
                    out.push_str("<ip>");
                    copied = j;
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&text[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Sanitizer {
        let mut s = Sanitizer::default();
        s.add_folder(Path::new("C:\\Users\\Ivan"), "<profile>");
        s.add_folder(Path::new("D:\\Mods\\Secret Quest"), "<project>");
        s.add_word("Ivan", "<user>");
        s.add_word("DESKTOP-7Q1", "<computer>");
        s
    }

    #[test]
    fn folders_and_names_become_markers_and_keep_what_lies_below() {
        let s = sample();
        assert_eq!(
            s.clean("opened d:/mods/secret quest/quests/a.nqasset from C:\\Users\\IVAN\\Downloads"),
            "opened <project>/quests/a.nqasset from <profile>\\Downloads"
        );
        assert_eq!(s.clean("user ivan on desktop-7q1"), "user <user> on <computer>");
        // a longer name that only starts with the folder or the word is somebody else's
        assert_eq!(s.clean("C:\\Users\\Ivanova\\x and Ivanov"), "<path>\\x and Ivanov");
    }

    #[test]
    fn other_paths_keep_their_last_name() {
        let s = sample();
        assert_eq!(
            s.clean("read E:\\Ivan Petrov\\mods\\a.ltx: no such file"),
            "read <path>\\a.ltx: no such file"
        );
        assert_eq!(s.clean("from D:\\a to F:/b/c.dds"), "from <path>\\a to <path>/c.dds");
        assert_eq!(s.clean("copy \\\\nas\\share\\x.zip done"), "copy <path>\\x.zip done");
        assert_eq!(s.clean("{\"root\": \"G:\\\\Games\\\\Dead Air\"}"), "{\"root\": \"<path>\\Dead Air\"}");
        // a URL and a ratio are no paths; a project path is the project's
        assert_eq!(s.clean("https://example.org/a/b and 16:9"), "https://example.org/a/b and 16:9");
        assert_eq!(s.clean("D:/mods/secret quest/a.lua"), "<project>/a.lua");
    }

    #[test]
    fn long_forms_urls_escaped_shares_and_owners() {
        let s = sample();
        assert_eq!(s.clean("open \\\\?\\E:\\Work\\x.ltx now"), "open <path>\\x.ltx now");
        assert_eq!(s.clean("see file:///E:/Work/a.html"), "see file:///<path>/a.html");
        assert_eq!(s.clean("{\"p\": \"\\\\\\\\nas\\\\share\\\\b.zip\"}"), "{\"p\": \"<path>\\b.zip\"}");
        assert_eq!(
            s.clean("GET https://api.github.com/repos/IvanP/mod/releases and https://github.com/IvanP/mod"),
            "GET https://api.github.com/repos/<owner>/mod/releases and https://github.com/<owner>/mod"
        );
        assert_eq!(s.clean("log grizzly_ivan.log"), "log grizzly_<user>.log");
    }

    #[test]
    fn versions_are_no_addresses_and_short_secrets_go() {
        let s = Sanitizer::default();
        assert_eq!(s.clean("driver 23.20.11.01 and 0.7.0.0"), "driver 23.20.11.01 and 0.7.0.0");
        assert_eq!(s.clean("version 1.6.4.2"), "version 1.6.4.2");
        assert_eq!(s.clean("password: hunter"), "password: <secret>");
    }

    #[test]
    fn a_json_answer_is_cleaned_key_and_value() {
        let s = sample();
        let mut v = serde_json::json!({ "C:\\Users\\Ivan\\x": ["D:\\Mods\\Secret Quest\\mod.ltx", 3] });
        s.clean_value(&mut v);
        assert_eq!(v, serde_json::json!({ "<profile>\\x": ["<project>\\mod.ltx", 3] }));
    }

    #[test]
    fn keys_mail_and_addresses_are_masked() {
        let s = Sanitizer::default();
        assert_eq!(s.clean("token=abcdef1234567890 next"), "token=<secret> next");
        assert_eq!(s.clean("Authorization: Bearer abcdefghij123"), "Authorization: Bearer <secret>");
        assert_eq!(s.clean("key ghp_ABCDEFabcdef0123456789 used"), "key <secret> used");
        assert_eq!(s.clean("the token expired"), "the token expired");
        assert_eq!(s.clean("authorization: Basic dXNlcjpwYXNz"), "authorization: Basic <secret>");
        assert_eq!(s.clean("write to ivan.p@mail.example.org now"), "write to <email> now");
        assert_eq!(
            s.clean("from 192.168.1.20:8080 and 127.0.0.1:28016"),
            "from <ip>:8080 and 127.0.0.1:28016"
        );
        // versions and builds are no addresses
        assert_eq!(
            s.clean("Grizzly Editor 0.7.0, build 26100.2605, 1.6.4.0.1"),
            "Grizzly Editor 0.7.0, build 26100.2605, 1.6.4.0.1"
        );
    }
}
