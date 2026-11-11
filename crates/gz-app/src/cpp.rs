//! The C++ editor's command table (`UI_MainCommand.cpp` `TUI::RegisterCommands`,
//! `UI_LevelMain.cpp` `CLevelMain::RegisterCommands`), as `list_commands` and `exec_command`
//! answer it: every command under its C++ name and its C++ numeric id, with its description and
//! its sub-commands, so a script written against the C++ editor keeps working.
//!
//! A row is registered with [`crate::ShellBuilder::cpp_command`]: its id, name, description,
//! sub-commands with their default keys, and its action - the [`crate::UiCommand`]s its crate
//! already registered under the sub-commands' ids (`COMMAND_UNDO`, `COMMAND_ZOOM_EXTENTS."Selected"`),
//! an editor command with arguments made of the two parameters, a function of the window, or a
//! task that runs off the UI thread. A row whose action is not there is left out: the table never
//! lists a command that does not work. Every row's sub-commands are `UiCommand`s too, so the
//! palette lists them and the Keyboard page binds them (an `editable` row only, as in the C++).
//!
//! The C++ commands that are plumbing of its own shell are not rows: `COMMAND_INITIALIZE`,
//! `DESTROY`, `QUIT`, `UPDATE_PROPERTIES`, `REFRESH_PROPERTIES`, `RENDER_FOCUS`, `RENDER_RESIZE`,
//! `UPDATE_GRID`, `UPDATE_CAPTION`, `UPDATE_TOOLBAR`, `REFRESH_UI_BAR`, `RESTORE_UI_BAR`,
//! `SAVE_UI_BAR`, `CREATE_SOUND_LIB`, `CHECK_MODIFIED`, `SET_SETTINGS`, `CLEAN_LIBRARY`,
//! `UNLOAD_TEXTURES`, `EVICT_OBJECTS`, `EVICT_TEXTURES`, `SAVE_BACKUP`, `CHANGE_AXIS`. Neither are
//! the ones whose C++ bodies do nothing (`CHANGE_SNAP`, `MOVE_CAMERA_TO`, `EDIT_COMMAND_LIST`,
//! `SIMULATE`, `USE_SIMULATE_POSITIONS`, `LIBRARY_EDITOR`, `LANIM_EDITOR`, `FILE_MENU`,
//! `SHOWCONTEXTMENU`, `SHOW_CLIP_EDITOR`, `REFRESH_SNAP_OBJECTS`, `SCENE_HIGHLIGHT_TEXTURE`,
//! `REFRESH_SOUND_ENV_GEOMETRY` - its `RealRefreshEnvGeometry` is commented out).

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::future::BoxFuture;
use gz_editor::{Args, Editor, Origin, SmolStr, Value, json};
use gz_ui::Icon;

use crate::builder::{EnabledFn, UiCommand};
use crate::doc::DocCx;
use crate::keys::Chord;
use crate::settings::EditorSettings;

/// A `CCommandVar`: a number or a text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandVar {
    Int(u32),
    Str(String),
}

impl Default for CommandVar {
    fn default() -> Self {
        CommandVar::Int(0)
    }
}

impl CommandVar {
    #[must_use]
    pub fn int(&self) -> Option<u32> {
        match self {
            CommandVar::Int(v) => Some(*v),
            CommandVar::Str(_) => None,
        }
    }

    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            CommandVar::Str(s) => Some(s),
            CommandVar::Int(_) => None,
        }
    }

    /// Not zero, as `if (p1)` reads a number; a text is true.
    #[must_use]
    pub fn truthy(&self) -> bool {
        self.int().is_none_or(|v| v != 0)
    }

    /// How the C++ answers it: a number as a JSON number, a text as a string.
    #[must_use]
    pub fn json(&self) -> Value {
        match self {
            CommandVar::Int(v) => json!(v),
            CommandVar::Str(s) => json!(s),
        }
    }

    /// `ParseParam` of a command list: a number unless the text holds a quote or does not read as
    /// one (`%d`, leading digits), else what stands between the first two quotes. `None` for an
    /// empty parameter (it keeps what the sub-command gives).
    #[must_use]
    pub fn parse_param(text: &str) -> Option<Self> {
        let t = text.trim();
        if t.is_empty() {
            return None;
        }
        if !t.contains('"')
            && let Some(v) = scan_int(t)
        {
            return Some(CommandVar::Int(v as u32));
        }
        let inner = t.split('"').nth(1).unwrap_or("").trim();
        (!inner.is_empty()).then(|| CommandVar::Str(inner.to_owned()))
    }

    /// The C++ log line's form of a parameter: `"text"` or `12`.
    fn log_form(&self) -> String {
        match self {
            CommandVar::Int(v) => v.to_string(),
            CommandVar::Str(s) if s.contains('\n') || s.contains('\r') => "\"...\"".to_owned(),
            CommandVar::Str(s) => format!("\"{s}\""),
        }
    }
}

/// `sscanf("%d")`: an optional sign and the digits that follow; `None` without a digit.
fn scan_int(t: &str) -> Option<i64> {
    let b = t.as_bytes();
    let (neg, start) = match b.first() {
        Some(b'-') => (true, 1),
        Some(b'+') => (false, 1),
        _ => (false, 0),
    };
    let digits: String = t.get(start..)?.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    let v: i64 = digits.parse().unwrap_or(i64::from(i32::MAX)).min(i64::from(u32::MAX));
    Some(if neg { -v } else { v })
}

/// A sub-command (`SESubCommand`): its name ("" for the plain command), the parameters it runs
/// with, its key until the author binds another, and the face's name when it is not
/// "<command> <sub-command>".
#[derive(Clone, Debug)]
pub struct CppSub {
    pub desc: SmolStr,
    pub p1: CommandVar,
    pub p2: CommandVar,
    pub keys: Option<Chord>,
    pub label: Option<SmolStr>,
}

impl CppSub {
    #[must_use]
    pub fn plain() -> Self {
        Self::new("", CommandVar::Int(0), CommandVar::Int(0))
    }

    #[must_use]
    pub fn new(desc: &str, p1: CommandVar, p2: CommandVar) -> Self {
        Self { desc: SmolStr::new(desc), p1, p2, keys: None, label: None }
    }

    #[must_use]
    pub fn ints(desc: &str, p1: u32, p2: u32) -> Self {
        Self::new(desc, CommandVar::Int(p1), CommandVar::Int(p2))
    }

    #[must_use]
    pub fn keys(mut self, keys: &str) -> Self {
        self.keys = Chord::parse(keys);
        self
    }

    #[must_use]
    pub fn label(mut self, label: &str) -> Self {
        self.label = Some(SmolStr::new(label));
        self
    }
}

/// The id the Keyboard page and `shortcuts` know a sub-command by: `NAME` or `NAME."desc"`.
#[must_use]
pub fn sub_id(name: &str, desc: &str) -> SmolStr {
    if desc.is_empty() { SmolStr::new(name) } else { SmolStr::new(format!("{name}.\"{desc}\"")) }
}

/// Arguments of an editor command, made of the two parameters; `Err` says why they do not fit.
pub type ArgsFn = Rc<dyn Fn(&CommandVar, &CommandVar) -> Result<Args, String>>;
/// A function of the window: the answer, or why it refused.
pub type RunFn =
    Rc<std::cell::RefCell<dyn FnMut(&mut DocCx<'_>, &CommandVar, &CommandVar) -> Result<CommandVar, String>>>;
/// Work off the UI thread (a macro), told who asked for it: the answer, or why it refused.
pub type TaskFn = Arc<
    dyn Fn(Arc<Editor>, CommandVar, CommandVar, Origin) -> BoxFuture<'static, Result<CommandVar, String>>
        + Send
        + Sync,
>;

/// Whether a row may run now: `(editor, active tab)`.
pub type EnabledRc = Rc<dyn Fn(&Editor, &str) -> bool>;

/// What a row does.
#[derive(Clone)]
pub enum CppAction {
    /// The [`UiCommand`]s registered under the sub-commands' ids.
    Ui,
    /// An editor command (its registry name) with arguments of the parameters.
    Editor { command: &'static str, args: ArgsFn },
    /// A function of the window, given the parameters.
    Shell(RunFn),
    /// A task off the UI thread, given the parameters.
    Task(TaskFn),
}

impl CppAction {
    pub fn editor(
        command: &'static str,
        args: impl Fn(&CommandVar, &CommandVar) -> Result<Args, String> + 'static,
    ) -> Self {
        CppAction::Editor { command, args: Rc::new(args) }
    }

    pub fn shell(
        f: impl FnMut(&mut DocCx<'_>, &CommandVar, &CommandVar) -> Result<CommandVar, String> + 'static,
    ) -> Self {
        CppAction::Shell(Rc::new(std::cell::RefCell::new(f)))
    }

    pub fn task<F>(
        f: impl Fn(Arc<Editor>, CommandVar, CommandVar, Origin) -> F + Send + Sync + 'static,
    ) -> Self
    where
        F: std::future::Future<Output = Result<CommandVar, String>> + Send + 'static,
    {
        CppAction::Task(Arc::new(move |ed, p1, p2, origin| Box::pin(f(ed, p1, p2, origin))))
    }
}

/// A row of the C++ table.
pub struct CppCommand {
    /// The C++ enum value.
    pub id: u32,
    /// `COMMAND_*`.
    pub name: SmolStr,
    /// `Desc()`: "Edit\Undo"; empty for a command the C++ registered without one.
    pub desc: SmolStr,
    /// `REGISTER_CMD_SE` / `_CE`: it takes a shortcut and the palette lists it.
    pub editable: bool,
    /// The faces' group, name and icon (Keyboard page, palette).
    pub group: SmolStr,
    pub label: SmolStr,
    pub icon: Icon,
    pub subs: Vec<CppSub>,
    pub action: CppAction,
    /// Off: a face is greyed out and the row refuses. `(editor, active tab)`.
    pub enabled: Option<EnabledRc>,
}

impl CppCommand {
    /// A row of one plain sub-command.
    #[must_use]
    pub fn new(id: u32, name: &str, desc: &str, action: CppAction) -> Self {
        Self {
            id,
            name: SmolStr::new(name),
            desc: SmolStr::new(desc),
            editable: !desc.is_empty(),
            group: SmolStr::new(desc.split_once('\\').map_or("General", |(g, _)| g)),
            label: SmolStr::new(desc.rsplit('\\').next().filter(|l| !l.is_empty()).unwrap_or(name)),
            icon: Icon::None,
            subs: vec![CppSub::plain()],
            action,
            enabled: None,
        }
    }

    #[must_use]
    pub fn subs(mut self, subs: Vec<CppSub>) -> Self {
        self.subs = subs;
        self
    }

    #[must_use]
    pub fn face(mut self, group: &str, label: &str, icon: Icon) -> Self {
        self.group = SmolStr::new(group);
        self.label = SmolStr::new(label);
        self.icon = icon;
        self
    }

    #[must_use]
    pub fn editable(mut self, on: bool) -> Self {
        self.editable = on;
        self
    }

    #[must_use]
    pub fn keys(mut self, keys: &str) -> Self {
        if let Some(s) = self.subs.first_mut() {
            s.keys = Chord::parse(keys);
        }
        self
    }

    #[must_use]
    pub fn enabled(mut self, f: impl Fn(&Editor, &str) -> bool + 'static) -> Self {
        self.enabled = Some(Rc::new(f));
        self
    }

    fn face_label(&self, sub: &CppSub) -> SmolStr {
        match (&sub.label, sub.desc.is_empty()) {
            (Some(l), _) => l.clone(),
            (None, true) => self.label.clone(),
            (None, false) => SmolStr::new(format!("{} {}", self.label, sub.desc)),
        }
    }
}

/// A row as the shell keeps it: the C++ data and, per sub-command, the index of its face in the
/// shell's commands.
pub(crate) struct Row {
    pub cmd: CppCommand,
    pub faces: Vec<Option<usize>>,
}

/// The table the shell keeps.
#[derive(Default)]
pub(crate) struct Table {
    pub rows: Vec<Row>,
}

/// Rust's own commands that have no C++ row: their ids continue after the C++ table.
const RUST_ONLY: [(u32, &str); 4] = [
    (123, "COMMAND_UNHIDE_ALL"),
    (124, "COMMAND_MOD_SCREENS"),
    (125, "COMMAND_MOD_BUILD_TO_FOLDER"),
    (126, "COMMAND_MOD_EXPORT_FLAT"),
];

/// The first id the table hands out to a command it was not told about.
const FREE_IDS: u32 = 200;

impl Table {
    /// Builds the table: the rows registered (a later one of an id wins), their faces added to
    /// `commands` where the action is not a `UiCommand` of its own, rows whose action is missing
    /// dropped, and every `UiCommand` nobody named given a row of its own after the C++ ones.
    pub(crate) fn build(mut rows: Vec<CppCommand>, commands: &mut Vec<UiCommand>) -> Self {
        // a later registration of an id replaces the earlier one
        let mut kept: Vec<CppCommand> = Vec::with_capacity(rows.len());
        for r in rows.drain(..) {
            kept.retain(|k| k.id != r.id && k.name != r.name);
            kept.push(r);
        }
        let mut out = Vec::with_capacity(kept.len() + 8);
        for cmd in kept {
            let mut faces = Vec::with_capacity(cmd.subs.len());
            for sub in &cmd.subs {
                let id = sub_id(&cmd.name, &sub.desc);
                let have = commands.iter().position(|c| c.id.eq_ignore_ascii_case(&id));
                let face = match (&cmd.action, have) {
                    (_, Some(i)) => Some(i),
                    (CppAction::Ui, None) => None,
                    (_, None) => {
                        commands.push(make_face(&cmd, sub, id));
                        Some(commands.len() - 1)
                    }
                };
                faces.push(face);
            }
            // a sub-command whose command is not there is not listed as if it worked
            let mut cmd = cmd;
            if matches!(cmd.action, CppAction::Ui) {
                let mut subs = Vec::with_capacity(cmd.subs.len());
                let mut kept_faces = Vec::with_capacity(faces.len());
                for (s, f) in cmd.subs.drain(..).zip(faces) {
                    if f.is_some() {
                        subs.push(s);
                        kept_faces.push(f);
                    }
                }
                if subs.is_empty() {
                    continue;
                }
                cmd.subs = subs;
                faces = kept_faces;
            }
            out.push(Row { cmd, faces });
        }
        // the UiCommands no row names: rows of their own
        let mut next = FREE_IDS;
        let named: Vec<usize> = out.iter().flat_map(|r| r.faces.iter().flatten().copied()).collect();
        let mut extra: Vec<(String, Vec<(usize, String)>)> = Vec::new();
        for (i, c) in commands.iter().enumerate() {
            if named.contains(&i) {
                continue;
            }
            let (base, desc) = match c.id.split_once('.') {
                Some((b, d)) => (b.to_owned(), d.trim_matches('"').to_owned()),
                None => (c.id.to_string(), String::new()),
            };
            match extra.iter_mut().find(|(b, _)| *b == base) {
                Some(e) => e.1.push((i, desc)),
                None => extra.push((base, vec![(i, desc)])),
            }
        }
        for (base, subs) in extra {
            let id = RUST_ONLY.iter().find(|(_, n)| *n == base).map_or_else(
                || {
                    next += 1;
                    next - 1
                },
                |(id, _)| *id,
            );
            let first = subs.first().and_then(|(i, _)| commands.get(*i));
            let (group, label) =
                first.map_or((String::new(), base.clone()), |c| (c.group.to_string(), c.label.to_string()));
            let desc = format!("{group}\\{label}");
            let mut cmd = CppCommand::new(id, &base, &desc, CppAction::Ui);
            cmd.subs = subs
                .iter()
                .enumerate()
                .map(|(k, (_, d))| CppSub::ints(d, u32::try_from(k).unwrap_or(0), 0))
                .collect();
            let faces = subs.iter().map(|(i, _)| Some(*i)).collect();
            out.push(Row { cmd, faces });
        }
        out.sort_by_key(|r| r.cmd.id);
        Table { rows: out }
    }

    /// A row by its id, its name, its description, or a face's id (`NAME."desc"`).
    pub(crate) fn find(&self, id: Option<i64>, name: &str) -> Option<usize> {
        if let Some(id) = id.filter(|v| *v >= 0) {
            return self.rows.iter().position(|r| i64::from(r.cmd.id) == id);
        }
        if name.is_empty() {
            return None;
        }
        let base = name.split_once('.').map_or(name, |(b, _)| b);
        self.rows.iter().position(|r| {
            r.cmd.name.eq_ignore_ascii_case(name)
                || (!r.cmd.desc.is_empty() && r.cmd.desc.eq_ignore_ascii_case(name))
                || r.cmd.name.eq_ignore_ascii_case(base)
        })
    }
}

/// The face of a sub-command: a `UiCommand` that runs the row's action with its parameters.
fn make_face(cmd: &CppCommand, sub: &CppSub, id: SmolStr) -> UiCommand {
    let (p1, p2) = (sub.p1.clone(), sub.p2.clone());
    let name = cmd.name.clone();
    let action = cmd.action.clone();
    let run = move |cx: &mut DocCx<'_>| {
        log_command(cx.editor, &name, &p1, &p2, 0);
        match run_now(cx, &action, &p1, &p2) {
            Outcome::Done(_) => {}
            Outcome::Refused(why) => cx.editor.notices.error(why),
            Outcome::Call(command, args) => {
                let ed = cx.editor.clone();
                let origin = ed.ui_origin();
                cx.editor.exec.spawn(async move {
                    let reply = ed.call(command, args, origin).await;
                    if reply.get("ok").and_then(Value::as_bool) != Some(true) {
                        let text =
                            reply.get("error").and_then(Value::as_str).unwrap_or("The command failed.");
                        ed.notices.error(text.to_owned());
                    }
                });
            }
            Outcome::Task(task, a, b) => {
                let ed = cx.editor.clone();
                let origin = ed.ui_origin();
                cx.editor.exec.spawn(async move {
                    if let Err(why) = task(ed.clone(), a, b, origin).await {
                        ed.notices.error(why);
                    }
                });
            }
        }
    };
    let mut face = UiCommand::new(&id, &cmd.group, &cmd.face_label(sub), cmd.icon, run);
    face.default_keys = sub.keys;
    if !cmd.editable {
        face.bindable = false;
        face.in_palette = false;
    }
    if let Some(f) = cmd.enabled.clone() {
        let f: EnabledFn = Box::new(move |ed, tab| f(ed, tab));
        face.enabled = Some(f);
    }
    face
}

/// What running an action on the UI thread came to.
pub(crate) enum Outcome {
    Done(CommandVar),
    Refused(String),
    /// An editor command to await.
    Call(&'static str, Args),
    /// A task to run off the UI thread.
    Task(TaskFn, CommandVar, CommandVar),
}

/// Runs an action that is not a `UiCommand` of its own.
pub(crate) fn run_now(cx: &mut DocCx<'_>, action: &CppAction, p1: &CommandVar, p2: &CommandVar) -> Outcome {
    match action {
        CppAction::Ui => Outcome::Refused("the command has no function of its own".to_owned()),
        CppAction::Editor { command, args } => match args(p1, p2) {
            Ok(a) => Outcome::Call(command, a),
            Err(why) => Outcome::Refused(why),
        },
        CppAction::Shell(f) => match f.try_borrow_mut() {
            Ok(mut f) => match (*f)(cx, p1, p2) {
                Ok(v) => Outcome::Done(v),
                Err(why) => Outcome::Refused(why),
            },
            Err(_) => Outcome::Refused("the command is already running".to_owned()),
        },
        CppAction::Task(t) => Outcome::Task(t.clone(), p1.clone(), p2.clone()),
    }
}

/// The sub-command the parameters name: the one whose preset they are (a text also matches a
/// sub-command's name, "Selected"), else the plain one, else the first.
pub(crate) fn pick_sub(subs: &[CppSub], p1: Option<&CommandVar>, p2: Option<&CommandVar>) -> Option<usize> {
    let zero = CommandVar::Int(0);
    let (a, b) = (p1.unwrap_or(&zero), p2.unwrap_or(&zero));
    subs.iter()
        .position(|s| s.p1 == *a && s.p2 == *b)
        .or_else(|| subs.iter().position(|s| s.p1 == *a))
        .or_else(|| {
            a.text()
                .and_then(|t| subs.iter().position(|s| !s.desc.is_empty() && s.desc.eq_ignore_ascii_case(t)))
        })
        .or_else(|| subs.iter().position(|s| s.desc.is_empty()))
        .or(if subs.is_empty() { None } else { Some(0) })
}

/// `Log Commands`: whether every command run is written to the log.
#[derive(Default)]
pub struct CommandLog {
    on: AtomicBool,
}

impl CommandLog {
    pub fn of(editor: &Editor) -> Arc<CommandLog> {
        editor.services.get_or_init(CommandLog::default)
    }

    pub fn set(&self, on: bool) {
        self.on.store(on, Ordering::Release);
    }

    #[must_use]
    pub fn on(&self) -> bool {
        self.on.load(Ordering::Acquire)
    }
}

/// One line for a command run while Log Commands is on: `COMMAND_X ("a",1)`, a nested one after
/// `;` and a dot per level.
// oracle: the C++ keeps the flag and the depth but its Msg calls are commented out
pub(crate) fn log_command(editor: &Editor, name: &str, p1: &CommandVar, p2: &CommandVar, depth: usize) {
    if !CommandLog::of(editor).on() {
        return;
    }
    let lead = if depth == 0 { String::new() } else { format!(";{}", ".".repeat(depth)) };
    editor.log.plain(format!("{lead}{name} ({},{})", p1.log_form(), p2.log_form()));
}

/// One sub-command as `list_commands` answers it.
pub(crate) fn preset_json(sub: &CppSub, keys: &str) -> Value {
    json!({ "desc": sub.desc, "p1": sub.p1.json(), "p2": sub.p2.json(), "keys": keys })
}

/// The C++ rows whose function is a `UiCommand` an area registers, and the shell's own: their
/// ids, names, descriptions and sub-commands as `RegisterCommands` gives them. A row whose
/// `UiCommand`s are not registered in this build is left out by [`Table::build`].
pub(crate) fn known_rows() -> Vec<CppCommand> {
    use CppAction::Ui;
    let ui = |id: u32, name: &str, desc: &str| CppCommand::new(id, name, desc, Ui);
    let plain_s = |id: u32, name: &str| CppCommand::new(id, name, "", Ui).editable(false);
    vec![
        ui(3, "COMMAND_EDITOR_PREF", "Editor Preference"),
        ui(4, "COMMAND_CHANGE_ACTION", "Change Action")
            .subs(vec![CppSub::ints("Select", 0, 0), CppSub::ints("Add", 1, 0)]),
        plain_s(5, "COMMAND_IMAGE_EDITOR"),
        plain_s(6, "COMMAND_LIGHTANIM_EDITOR"),
        plain_s(8, "COMMAND_CHECK_TEXTURES"),
        plain_s(9, "COMMAND_REFRESH_TEXTURES"),
        ui(18, "COMMAND_EXIT", "Exit"),
        ui(23, "COMMAND_ZOOM_EXTENTS", "Zoom")
            .subs(vec![CppSub::ints("Extent", 0, 0), CppSub::ints("Selected", 1, 0)]),
        ui(26, "COMMAND_TOGGLE_RENDER_WIRE", "Toggle Wireframe"),
        ui(28, "COMMAND_BREAK_LAST_OPERATION", "Break Last Operation"),
        ui(30, "COMMAND_TOGGLE_SAFE_RECT", "Toggle Safe Rect"),
        ui(31, "COMMAND_TOGGLE_GRID", "Toggle Grid"),
        ui(34, "COMMAND_GRID_SLOT_SIZE", "Change Grid Size")
            .subs(vec![CppSub::ints("Decrease", 0, 0), CppSub::ints("Increase", 1, 0)]),
        plain_s(45, "COMMAND_SOUND_EDITOR"),
        plain_s(46, "COMMAND_SYNC_SOUNDS"),
        ui(47, "COMMAND_UNDO", "Edit\\Undo"),
        ui(48, "COMMAND_REDO", "Edit\\Redo"),
        ui(56, "COMMAND_COMMAND_PALETTE", "Windows\\Command Palette"),
        ui(61, "COMMAND_SHOW_OBJECTLIST", "Scene\\Show Object List"),
        ui(93, "COMMAND_INVERT_SELECTION_ALL", "Selection\\Invert"),
        ui(94, "COMMAND_SELECT_ALL", "Selection\\Select All"),
        ui(95, "COMMAND_DESELECT_ALL", "Selection\\Unselect All"),
        ui(96, "COMMAND_DELETE_SELECTION", "Edit\\Delete"),
        ui(97, "COMMAND_HIDE_UNSEL", "Visibility\\Hide Unselected"),
        ui(98, "COMMAND_HIDE_SEL", "Visibility\\Hide Selected"),
        ui(99, "COMMAND_HIDE_ALL", "Visibility\\Hide All"),
        ui(109, "COMMAND_GIZMO_MODE", "Gizmo").subs(vec![
            CppSub::ints("Select", 4, 0),
            CppSub::ints("Move", 0, 0),
            CppSub::ints("Rotate", 2, 0),
            CppSub::ints("Scale", 1, 0),
            CppSub::ints("Cycle", 3, 0),
        ]),
        ui(113, "COMMAND_DROP_TO_GROUND", "Object\\Drop To Ground"),
        ui(114, "COMMAND_MOD_BUILD", "Mod\\Build Into Game"),
        ui(115, "COMMAND_MOD_EDIT_MANIFEST", "Mod\\Edit Manifest"),
        ui(116, "COMMAND_MOD_PACKAGE", "Mod\\Package Release"),
        ui(117, "COMMAND_MOD_PUBLISH", "Mod\\Publish Release"),
        ui(118, "COMMAND_MOD_RELEASES", "Mod\\Manage Releases"),
        ui(119, "COMMAND_MOD_CHARACTERS", "Mod\\Characters"),
        ui(120, "COMMAND_MOD_WORLD", "Mod\\World"),
        ui(121, "COMMAND_MOD_PDA", "Mod\\PDA Interface"),
        ui(122, "COMMAND_MOD_CATALOG", "Mod\\Submit to Mod Browser"),
    ]
}

/// The rows whose function is the shell's: the panels, the grid's cell count, the scene's file
/// commands that exist, and the macros.
pub(crate) fn shell_rows() -> Vec<CppCommand> {
    use crate::doc::PanelOp;
    let panel = |id: &'static str, op: PanelOp| {
        CppAction::shell(move |cx, _p1, _p2| {
            cx.requests.panel(id, op);
            Ok(CommandVar::Int(1))
        })
    };
    let has_project = |ed: &Editor, _: &str| ed.project_opt().is_some();
    let mut rows = vec![
        CppCommand::new(19, "COMMAND_SHOW_PROPERTIES", "Show Properties", panel("details", PanelOp::Focus))
            .face("Window", "Show Details", Icon::None)
            .keys("Enter")
            .enabled(has_project),
        CppCommand::new(86, "COMMAND_OPTIONS", "Scene\\Options", panel("level_settings", PanelOp::Focus))
            .face("Window", "Show Level Settings", Icon::None)
            .keys("Ctrl+O")
            .enabled(has_project),
        CppCommand::new(
            111,
            "COMMAND_SHOW_CONTENT_BROWSER",
            "Windows\\Content Browser",
            panel("content_browser", PanelOp::Toggle),
        )
        .face("Window", "Toggle Content Browser", Icon::None)
        .enabled(has_project),
        CppCommand::new(
            112,
            "COMMAND_SHOW_WORLD_OUTLINER",
            "Windows\\World Outliner",
            panel("world_outliner", PanelOp::Toggle),
        )
        .face("Window", "Toggle World Outliner", Icon::None)
        .enabled(has_project),
        // `CommandGridNumberOfSlots`: two cells more, or two fewer
        CppCommand::new(
            33,
            "COMMAND_GRID_NUMBER_OF_SLOTS",
            "",
            CppAction::shell(|cx, p1, _p2| {
                let settings = EditorSettings::of(cx.editor);
                let now = settings.pref("grid_cell_count").unwrap_or(100.0);
                let next = if p1.truthy() { now + 2.0 } else { now - 2.0 };
                let def = crate::settings::pref_def("grid_cell_count");
                let (lo, hi) = def.map_or((10.0, 1000.0), |d| (d.lo, d.hi));
                settings.set_pref("grid_cell_count", next.clamp(lo, hi));
                settings.save_soon(cx.editor);
                cx.editor.exec.wake().ping();
                Ok(CommandVar::Int(1))
            }),
        )
        .face("View", "Grid Cell Count", Icon::None),
        // `CommandLoad` with a scene: the file dialog is not part of this build
        CppCommand::new(
            40,
            "COMMAND_LOAD",
            "File\\Load Level",
            CppAction::editor("open_scene", |p1, p2| {
                let scene = p1.text().ok_or_else(|| {
                    "p1s names the scene (the Open Scene dialog is not part of this build)".to_owned()
                })?;
                let mut a = Args::new();
                a.insert("scene".into(), json!(scene));
                // flSceneLoadDiscard
                if p2.int().is_some_and(|f| f & 1 != 0) {
                    a.insert("discard".into(), json!(true));
                }
                Ok(a)
            }),
        )
        .editable(false)
        .face("File", "Open Scene", Icon::None),
        // `CommandSave`: Save; Save As (p2 = 1) is not part of this build
        CppCommand::new(
            41,
            "COMMAND_SAVE",
            "File",
            CppAction::editor("save_scene", |p1, p2| {
                if p2.int() == Some(1) {
                    return Err("Save As is not part of this build".to_owned());
                }
                let mut a = Args::new();
                if let Some(file) = p1.text().filter(|f| !f.is_empty()) {
                    a.insert("file".into(), json!(file));
                }
                Ok(a)
            }),
        )
        .subs(vec![CppSub::ints("Save", 0, 0).label("Save Scene")])
        .face("File", "Save Scene", Icon::Save)
        .enabled(has_project),
    ];
    rows.extend(crate::macros::rows());
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameters_read_like_parse_param() {
        assert_eq!(CommandVar::parse_param(" 12 "), Some(CommandVar::Int(12)));
        assert_eq!(CommandVar::parse_param("\"a b\""), Some(CommandVar::Str("a b".into())));
        assert_eq!(CommandVar::parse_param("x"), None);
        assert_eq!(CommandVar::parse_param(""), None);
        assert_eq!(CommandVar::parse_param("-1"), Some(CommandVar::Int(u32::MAX)));
        assert_eq!(CommandVar::parse_param("3abc"), Some(CommandVar::Int(3)));
    }

    #[test]
    fn a_sub_command_is_picked_by_its_preset() {
        let subs = vec![CppSub::ints("Extent", 0, 0), CppSub::ints("Selected", 1, 0)];
        assert_eq!(pick_sub(&subs, None, None), Some(0));
        assert_eq!(pick_sub(&subs, Some(&CommandVar::Int(1)), None), Some(1));
        assert_eq!(pick_sub(&subs, Some(&CommandVar::Str("selected".into())), None), Some(1));
        let gizmo = vec![CppSub::ints("Select", 4, 0), CppSub::ints("Move", 0, 0)];
        assert_eq!(pick_sub(&gizmo, None, None), Some(1));
    }

    #[test]
    fn ids_of_sub_commands() {
        assert_eq!(sub_id("COMMAND_UNDO", ""), "COMMAND_UNDO");
        assert_eq!(sub_id("COMMAND_ZOOM_EXTENTS", "Selected"), "COMMAND_ZOOM_EXTENTS.\"Selected\"");
    }

    #[test]
    fn rows_without_their_commands_are_left_out() {
        let mut commands = vec![UiCommand::new("COMMAND_UNDO", "Edit", "Undo", Icon::Undo, |_| {})];
        let t = Table::build(known_rows(), &mut commands);
        assert!(t.rows.iter().any(|r| r.cmd.id == 47));
        assert!(!t.rows.iter().any(|r| r.cmd.id == 48));
        assert_eq!(t.find(Some(47), ""), Some(0));
        assert_eq!(t.find(None, "Edit\\Undo"), Some(0));
    }
}
