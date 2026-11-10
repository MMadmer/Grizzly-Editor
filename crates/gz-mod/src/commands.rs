//! Console commands (`[commands]`); the game half is `XmsGame::RegisterModuleCommands`.

use gz_base::{Problem, Severity};

use crate::model::Manifest;
use crate::text::outside_cp1251;

fn ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn ident_char(c: u8) -> bool {
    ident_start(c) || c.is_ascii_digit()
}

/// `a-z`, `0-9` and `_`, 64 at most: the console finds a command by its exact name and ends it
/// at a space.
#[must_use]
pub fn valid_command_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

/// `<script>.<function>`: two Lua names, 128 at most.
#[must_use]
pub fn valid_command_function(spec: &str) -> bool {
    if spec.len() > 128 {
        return false;
    }
    let b = spec.as_bytes();
    let mut p = 0;
    for part in 0..2 {
        if !b.get(p).is_some_and(|&c| ident_start(c)) {
            return false;
        }
        while b.get(p).is_some_and(|&c| ident_char(c)) {
            p += 1;
        }
        if part == 0 {
            if b.get(p) != Some(&b'.') {
                return false;
            }
            p += 1;
        }
    }
    p == b.len()
}

fn skip_blanks(b: &[u8], mut p: usize, e: usize) -> usize {
    while p < e && matches!(b.get(p), Some(b' ' | b'\t')) {
        p += 1;
    }
    p
}

/// `function name(` or `name = function(` at the start of a line: what the script's table
/// keeps, as `<script>.<name>`. `local function` stays inside its script; `function a.b(` and
/// `function a:b(` belong to a table.
pub fn scan_script_functions(text: &str, script: &str, out: &mut Vec<String>) {
    let b = text.as_bytes();
    let mut pos = 0;
    while pos < b.len() {
        let e = b.get(pos..).and_then(|rest| rest.iter().position(|&c| c == b'\n')).map_or(b.len(), |n| pos + n);
        let mut p = skip_blanks(b, pos, e);
        pos = e + 1;
        let keyword = e.saturating_sub(p) > 9 && b.get(p..p + 8) == Some(b"function") && matches!(b.get(p + 8), Some(b' ' | b'\t'));
        if keyword {
            p += 9;
        }
        p = skip_blanks(b, p, e);
        if p >= e || !b.get(p).is_some_and(|&c| ident_start(c)) {
            continue;
        }
        let name_start = p;
        while p < e && b.get(p).is_some_and(|&c| ident_char(c)) {
            p += 1;
        }
        let name = text.get(name_start..p).unwrap_or("");
        p = skip_blanks(b, p, e);
        if !keyword {
            if p >= e || b.get(p) != Some(&b'=') || (p + 1 < e && b.get(p + 1) == Some(&b'=')) {
                continue;
            }
            p = skip_blanks(b, p + 1, e);
            if e.saturating_sub(p) < 8 || b.get(p..p + 8) != Some(b"function") {
                continue;
            }
            p = skip_blanks(b, p + 8, e);
        }
        if p < e && b.get(p) == Some(&b'(') {
            out.push(format!("{script}.{name}"));
        }
    }
}

/// The game finds the script by its file name (any case), the function by its exact name.
#[must_use]
pub fn has_script_function(functions: &[String], spec: &str) -> bool {
    let Some(dot) = spec.find('.') else { return false };
    let (script, function) = (spec.get(..dot).unwrap_or(""), spec.get(dot..).unwrap_or(""));
    functions.iter().any(|f| f.find('.') == Some(dot) && f.get(..dot).is_some_and(|s| s.eq_ignore_ascii_case(script)) && f.get(dot..) == Some(function))
}

/// The game's own short words a module could pick: refused.
fn stock_command_word(name: &str) -> bool {
    const WORDS: [&str; 23] = [
        "bind", "chat", "disconnect", "flush", "fov", "help", "hide", "load", "name", "nq", "quit", "renderer", "save", "screenshot", "start", "unbind",
        "unbindall", "cfg_save", "cfg_load", "main_menu", "run_string", "time_factor", "default_controls",
    ];
    WORDS.contains(&name)
}

/// The game's own command families (the engine registers ~1000 commands): a warning.
fn stock_command_family(name: &str) -> Option<&'static str> {
    const FAMILIES: [&str; 32] = [
        "xms_", "r_", "r1_", "r2_", "r3_", "r4_", "r__", "rs_", "g_", "sv_", "cl_", "net_", "dbg_", "debug_", "ai_", "al_", "ph_", "hud_", "snd_", "vid_",
        "mm_", "mt_", "dar_", "cam_", "ui_", "inv_", "lua_", "gamepad_", "demo_", "stat_", "bind_", "unbind_",
    ];
    FAMILIES.into_iter().find(|f| name.starts_with(f))
}

fn problem(code: &str, text: String, name: &str) -> Problem {
    let severity = if code.starts_with('E') { Severity::Error } else { Severity::Warning };
    Problem::new(code, severity, text).about(name).in_field("commands")
}

/// The build's check of `[commands]`, with the C++ codes and texts: `E401` a bad name, `E402` a
/// name twice, `E403` a function that is not `<script>.<function>`, `E404` one the project's
/// scripts lack, `E405` a `[command.<name>]` with no `[commands]` line, `E406` a name the game
/// has; `W406` a name of a family of the game's own, `W407` a name another module of the linked
/// game declares, `W408` text outside cp1251. `functions` are [`crate::script_functions`],
/// `taken` the `(command, module id)` pairs of [`crate::installed_commands`].
#[must_use]
pub fn validate_commands(m: &Manifest, functions: &[String], taken: &[(String, String)]) -> Vec<Problem> {
    let mut out = Vec::new();
    for (i, c) in m.commands.iter().enumerate() {
        let name = c.name.as_str();
        let at = format!("commands: '{name}'");
        if !c.declared {
            out.push(problem("E405", format!("{at}: [command.{name}] describes a command [commands] does not declare - add its line or remove the section"), name));
            continue;
        }
        if !valid_command_name(name) {
            out.push(problem("E401", format!("{at} is not a console command name - a-z, 0-9 and _ only, 64 at most"), name));
        } else if stock_command_word(name) {
            out.push(problem("E406", format!("{at} is a command of the game - it keeps its own and refuses this one"), name));
        } else if let Some(family) = stock_command_family(name) {
            out.push(problem("W406", format!("{at} is named like the game's own {family}* commands - if the game has one of that name, it keeps it and refuses this one"), name));
        }
        if m.commands.iter().take(i).any(|k| k.declared && k.name.eq_ignore_ascii_case(name)) {
            out.push(problem("E402", format!("{at} is declared twice - the game takes the first line and refuses this one"), name));
        }
        if !valid_command_function(&c.function) {
            out.push(problem("E403", format!("{at} calls '{}' - it takes <script>.<function>, e.g. my_mod.status", c.function), name));
        } else if !has_script_function(functions, &c.function) {
            let (script, function) = c.function.split_once('.').unwrap_or((c.function.as_str(), ""));
            out.push(problem("E404", format!("{at}: gamedata\\scripts\\{script}.script has no function {function}( - the game would find nothing to call"), name));
        }
        if let Some((_, module)) = taken.iter().find(|(command, _)| command.eq_ignore_ascii_case(name)) {
            out.push(problem("W407", format!("{at} is a command of module '{module}' of the linked game too - the one that loads first keeps it"), name));
        }
        if let Some(bad) = outside_cp1251(&format!("{}{}", c.help, c.args)) {
            out.push(problem("W408", format!("{at}: the help or the arguments hold {bad}, which the console shows as '?'"), name));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Command;

    #[test]
    fn names_and_functions() {
        assert!(valid_command_name("od_status2"));
        assert!(!valid_command_name("Od"));
        assert!(!valid_command_name(""));
        assert!(valid_command_function("od_mod.status"));
        assert!(!valid_command_function("od.status.x"));
        assert!(!valid_command_function("1od.status"));
        assert!(!valid_command_function("od"));
    }

    #[test]
    fn scans_what_the_table_keeps() {
        let script = "function status(args)\n  local function inner() end\nlocal function hidden() end\nreport = function (x)\nfunction t.member()\nx == function()\n\tfunction  spaced  (a)\n";
        let mut out = Vec::new();
        scan_script_functions(script, "od", &mut out);
        assert_eq!(out, ["od.status", "od.report", "od.spaced"]);
        assert!(has_script_function(&out, "OD.status"));
        assert!(!has_script_function(&out, "od.Status"));
    }

    #[test]
    fn checks_with_the_cpp_codes() {
        let cmd = |name: &str, function: &str| Command { name: name.into(), function: function.into(), ..Command::default() };
        let mut m = Manifest::default();
        m.commands = vec![
            cmd("status", "od.status"),
            cmd("Status", "od.status"),
            cmd("save", "od.status"),
            cmd("g_fly", "od"),
            cmd("night", "od.nope"),
            Command { declared: false, ..cmd("lost", "") },
            Command { help: "★".into(), ..cmd("busy", "od.status") },
        ];
        let functions = vec!["od.status".to_owned()];
        let taken = vec![("BUSY".to_owned(), "other".to_owned())];
        let codes: Vec<String> = validate_commands(&m, &functions, &taken).iter().map(|p| p.code.to_string()).collect();
        assert_eq!(codes, ["E401", "E402", "E406", "W406", "E403", "E404", "E405", "W407", "W408"]);
        let texts = validate_commands(&m, &functions, &taken);
        assert_eq!(texts[5].message, "commands: 'night': gamedata\\scripts\\od.script has no function nope( - the game would find nothing to call");
    }
}
