//! Actions: what the author can trigger by name - a menu item, a toolbar button, a shortcut, a
//! palette row (docs/COMMANDS.md, section 4; docs/UI_DESIGN.md, section 5).
//!
//! An [`Action`] is a registry command with its arguments, worked out from what is in front of the
//! author ([`ActionCx`]), plus what the window shows of it: a title, an icon, default keys, a
//! group, where its keys are live, whether it is on now and why not, whether it is checked. The
//! window never does anything by itself: running an action is `Editor::call` with the window's
//! origin, so a question the command raises still opens a dialog for a person, and a failure is a
//! notice. An area adds its actions with `ShellBuilder::action`; menus, toolbars, shortcuts and the
//! palette name them by id.
//!
//! The Keyboard page rebinds every action ([`ActionTable::bind`]); what the author bound is kept in
//! the editor's settings (`[shortcuts]`, the action's id and its keys).

use std::sync::Arc;

use gz_editor::{Args, Editor, SmolStr, Value, json};
use gz_ui::Icon;
use rustc_hash::FxHashMap;

use crate::doc::{Drawer, EditAction};
use crate::keys::Chord;
use crate::settings::EditorSettings;

/// What is in front of the author when an action is asked whether it is on, or run.
pub struct ActionCx<'a> {
    pub editor: &'a Editor,
    /// The active tab's id (`level`, `quest:<path>`, `characters`) and kind.
    pub tab: &'a str,
    pub kind: &'a str,
    /// The active tab's file, project-relative; empty for a singleton.
    pub path: &'a str,
    /// A project is open and its game linked: the work area is not the Start page.
    pub project: bool,
    /// The active tab holds unsaved changes.
    pub dirty: bool,
    /// The Edit actions the active tab takes.
    pub edits: &'a [EditAction],
    /// The drawer that is up.
    pub drawer: Option<Drawer>,
}

impl ActionCx<'_> {
    /// The active tab takes this Edit action.
    #[must_use]
    pub fn takes(&self, a: EditAction) -> bool {
        self.edits.contains(&a)
    }
}

/// Whether an action is on now; off with the reason its tooltip and the palette show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enabled {
    On,
    Off(&'static str),
}

impl Enabled {
    #[must_use]
    pub fn on(self) -> bool {
        self == Enabled::On
    }

    #[must_use]
    pub fn why(self) -> &'static str {
        match self {
            Enabled::On => "",
            Enabled::Off(why) => why,
        }
    }

    /// On when `cond`, else off for `why`.
    #[must_use]
    pub fn when(cond: bool, why: &'static str) -> Self {
        if cond { Enabled::On } else { Enabled::Off(why) }
    }
}

/// Where an action's keys are live. Its menu row and palette row are there wherever it is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// On every tab.
    Global,
    /// On a tab of one of these kinds.
    Tab(&'static [&'static str]),
    /// On a tab of one of these kinds while the document says this part of it has the keys
    /// (`Document::key_part`: the graph's canvas).
    Part(&'static [&'static str], &'static str),
}

impl Scope {
    /// The keys are live here.
    #[must_use]
    pub fn live(self, kind: &str, part: Option<&str>) -> bool {
        match self {
            Scope::Global => true,
            Scope::Tab(kinds) => kinds.contains(&kind),
            Scope::Part(kinds, want) => kinds.contains(&kind) && part == Some(want),
        }
    }

    /// Two scopes where one chord would mean two things.
    #[must_use]
    pub fn overlaps(self, other: Scope) -> bool {
        let kinds = |s: Scope| match s {
            Scope::Global => None,
            Scope::Tab(k) | Scope::Part(k, _) => Some(k),
        };
        match (kinds(self), kinds(other)) {
            (None, _) | (_, None) => true,
            (Some(a), Some(b)) => {
                let shared = a.iter().any(|k| b.contains(k));
                match (self, other) {
                    (Scope::Part(_, x), Scope::Part(_, y)) => shared && x == y,
                    _ => shared,
                }
            }
        }
    }

    /// How the Keyboard page and `app.keys` say it.
    #[must_use]
    pub fn describe(self) -> String {
        match self {
            Scope::Global => "everywhere".to_owned(),
            Scope::Tab(kinds) => kinds.join(", "),
            Scope::Part(kinds, part) => format!("{} ({part})", kinds.join(", ")),
        }
    }
}

/// What the window is handed when an action's command answered.
pub struct ReplyCx<'a> {
    pub editor: &'a Arc<Editor>,
    pub ctx: &'a egui::Context,
    /// The whole answer: `{"ok": ..., ...}`.
    pub reply: &'a Value,
}

fn no_args(_: &ActionCx<'_>) -> Args {
    Args::new()
}

fn always(_: &ActionCx<'_>) -> Enabled {
    Enabled::On
}

/// Something the author can trigger by name.
#[derive(Clone, Copy)]
pub struct Action {
    /// Stable: what a key binding, a menu and the layout store name. `graph.frame_all`.
    pub id: &'static str,
    /// Title Case: what the menu and the palette print.
    pub title: &'static str,
    pub icon: Option<Icon>,
    /// The registry command it runs ...
    pub command: &'static str,
    /// ... with these arguments, from what is in front of the author.
    pub args: fn(&ActionCx<'_>) -> Args,
    /// Its keys until the author binds others.
    pub keys: &'static [Chord],
    /// The palette's category and the Keyboard page's section: "File", "Graph", "Mod".
    pub group: &'static str,
    pub scope: Scope,
    pub enabled: fn(&ActionCx<'_>) -> Enabled,
    pub checked: Option<fn(&ActionCx<'_>) -> bool>,
    /// What the window shows of the answer (a result dialog). Without one a failure is a notice.
    pub reply: Option<fn(&ReplyCx<'_>)>,
    /// Listed in the Command Palette.
    pub in_palette: bool,
    /// One of the frame's own keys (the palette, the tabs, the drawers, Save): it takes its key
    /// also while a panel that answers its own keys has the focus. Every other action leaves the
    /// keys to such a panel (the Content Browser's Delete deletes its files, not the scene's).
    pub frame: bool,
}

impl Action {
    /// An action that runs `command` with no arguments, on everywhere, with no keys.
    #[must_use]
    pub const fn new(
        id: &'static str,
        title: &'static str,
        group: &'static str,
        command: &'static str,
    ) -> Self {
        Self {
            id,
            title,
            icon: None,
            command,
            args: no_args,
            keys: &[],
            group,
            scope: Scope::Global,
            enabled: always,
            checked: None,
            reply: None,
            in_palette: true,
            frame: false,
        }
    }

    /// One of the frame's own keys (see [`Action::frame`]).
    #[must_use]
    pub const fn frame_key(mut self) -> Self {
        self.frame = true;
        self
    }

    #[must_use]
    pub const fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    #[must_use]
    pub const fn args(mut self, f: fn(&ActionCx<'_>) -> Args) -> Self {
        self.args = f;
        self
    }

    #[must_use]
    pub const fn keys(mut self, keys: &'static [Chord]) -> Self {
        self.keys = keys;
        self
    }

    #[must_use]
    pub const fn scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    #[must_use]
    pub const fn enabled(mut self, f: fn(&ActionCx<'_>) -> Enabled) -> Self {
        self.enabled = f;
        self
    }

    #[must_use]
    pub const fn checked(mut self, f: fn(&ActionCx<'_>) -> bool) -> Self {
        self.checked = Some(f);
        self
    }

    #[must_use]
    pub const fn reply(mut self, f: fn(&ReplyCx<'_>)) -> Self {
        self.reply = Some(f);
        self
    }

    /// Not a row of the palette (a toolbar's own button, a row of a menu that lists things).
    #[must_use]
    pub const fn hidden_from_palette(mut self) -> Self {
        self.in_palette = false;
        self
    }
}

/// Arguments from pairs: `args(&[("action", "open"), ("kind", "characters")])`.
#[must_use]
pub fn args(pairs: &[(&str, &str)]) -> Args {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), Value::String((*v).to_owned()))).collect()
}

/// The actions of the window and what the author bound them to. UI thread only.
#[derive(Default)]
pub struct ActionTable {
    actions: Vec<Action>,
    /// What the author bound: an action's id and its chords (empty: taken away).
    bound: FxHashMap<SmolStr, Vec<Chord>>,
}

/// Reads a list of chords as the settings keep it: "Ctrl+Y, Ctrl+Shift+Z"; empty is none.
#[must_use]
pub fn parse_keys(text: &str) -> Vec<Chord> {
    text.split(',').filter_map(|k| Chord::parse(k.trim())).collect()
}

/// Writes a list of chords as the settings keep it.
#[must_use]
pub fn keys_text(keys: &[Chord]) -> String {
    keys.iter().map(Chord::label).collect::<Vec<_>>().join(", ")
}

impl ActionTable {
    /// An action registered twice keeps the later one, in the earlier one's place.
    pub fn add(&mut self, action: Action) {
        match self.actions.iter_mut().find(|a| a.id == action.id) {
            Some(slot) => *slot = action,
            None => self.actions.push(action),
        }
    }

    /// Takes what the author bound, from the editor's settings: every key of `[shortcuts]` that
    /// names an action.
    pub fn load_bindings(&mut self, settings: &EditorSettings) {
        for (id, keys) in settings.shortcuts() {
            if self.get(&id).is_some() {
                self.bound.insert(id, parse_keys(&keys));
            }
        }
    }

    #[must_use]
    pub fn all(&self) -> &[Action] {
        &self.actions
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.id == id)
    }

    /// What an action is bound to now.
    #[must_use]
    pub fn keys_of(&self, id: &str) -> Vec<Chord> {
        match self.bound.get(id) {
            Some(keys) => keys.clone(),
            None => self.get(id).map(|a| a.keys.to_vec()).unwrap_or_default(),
        }
    }

    /// The keys a menu row shows: the first chord bound now, as text.
    #[must_use]
    pub fn keys_label(&self, id: &str) -> String {
        self.keys_of(id).first().map(Chord::label).unwrap_or_default()
    }

    /// The author bound it to other keys than its defaults.
    #[must_use]
    pub fn rebound(&self, id: &str) -> bool {
        self.bound.contains_key(id)
    }

    /// Binds an action to `keys` (empty takes its keys away); every other action of an
    /// overlapping scope that had one of the chords lets it go. Answers those actions' ids. The
    /// settings keep it (the caller saves them).
    pub fn bind(&mut self, id: &str, keys: Vec<Chord>, settings: &EditorSettings) -> Vec<String> {
        let Some(scope) = self.get(id).map(|a| a.scope) else { return Vec::new() };
        let mut moved = Vec::new();
        let others: Vec<(&'static str, Scope)> =
            self.actions.iter().filter(|a| a.id != id).map(|a| (a.id, a.scope)).collect();
        for (other, other_scope) in others {
            if !scope.overlaps(other_scope) {
                continue;
            }
            let mut have = self.keys_of(other);
            let before = have.len();
            have.retain(|c| !keys.contains(c));
            if have.len() != before {
                settings.set_shortcut(other, &keys_text(&have));
                self.bound.insert(SmolStr::new(other), have);
                moved.push(other.to_owned());
            }
        }
        settings.set_shortcut(id, &keys_text(&keys));
        self.bound.insert(SmolStr::new(id), keys);
        moved
    }

    /// Back to its defaults (`None`: every action's).
    pub fn reset(&mut self, id: Option<&str>, settings: &EditorSettings) {
        let ids: Vec<SmolStr> = match id {
            Some(one) => vec![SmolStr::new(one)],
            None => self.bound.keys().cloned().collect(),
        };
        for id in ids {
            self.bound.remove(&id);
            settings.remove_shortcut(&id);
        }
    }

    /// The other actions whose keys of an overlapping scope are one of this action's.
    #[must_use]
    pub fn conflicts(&self, id: &str) -> Vec<&'static str> {
        let Some(me) = self.get(id) else { return Vec::new() };
        let mine = self.keys_of(id);
        self.actions
            .iter()
            .filter(|a| a.id != id && me.scope.overlaps(a.scope))
            .filter(|a| self.keys_of(a.id).iter().any(|c| mine.contains(c)))
            .map(|a| a.id)
            .collect()
    }

    /// The action a chord runs here, first taker first: an action live on this tab (its part)
    /// before one live everywhere - a tab's own keys win over the window's - and only one that is
    /// on (an action that is off lets the key go on to the next taker).
    /// While a panel owns the keys (`owned`), only the frame's own keys are taken.
    #[must_use]
    pub fn for_chord(
        &self,
        chord: &Chord,
        cx: &ActionCx<'_>,
        part: Option<&str>,
        owned: bool,
    ) -> Option<&Action> {
        let bound = |a: &&Action| (!owned || a.frame) && self.keys_of(a.id).contains(chord);
        let on = |a: &&Action| (a.enabled)(cx).on();
        let local = self
            .actions
            .iter()
            .filter(|a| a.scope != Scope::Global && a.scope.live(cx.kind, part))
            .filter(bound)
            .find(on);
        local.or_else(|| self.actions.iter().filter(|a| a.scope == Scope::Global).filter(bound).find(on))
    }

    /// Whether a chord belongs to an action that is live everywhere: a binding of the level's own
    /// table on it is ignored.
    #[must_use]
    pub fn global_chord(&self, chord: &Chord) -> Option<&Action> {
        self.actions.iter().filter(|a| a.scope == Scope::Global).find(|a| self.keys_of(a.id).contains(chord))
    }
}

/// What the active tab was when the window last drew: what an action is asked against from a
/// surface (its own menus and toolbar) without the shell at hand.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub tab: String,
    pub kind: String,
    pub path: String,
    pub project: bool,
    pub dirty: bool,
    pub edits: Vec<EditAction>,
    pub drawer: Option<Drawer>,
}

/// The window's actions and what the active tab is, as an `Editor` service: a surface draws an
/// action's menu row or toolbar button from it ([`menu_row`], [`toolbar_button`]).
#[derive(Default)]
pub struct Actions {
    pub table: parking_lot::RwLock<ActionTable>,
    facts: parking_lot::Mutex<Facts>,
}

impl Actions {
    #[must_use]
    pub fn new(table: ActionTable) -> Self {
        Self { table: parking_lot::RwLock::new(table), facts: parking_lot::Mutex::default() }
    }

    /// The window's actions; `None` without a window.
    #[must_use]
    pub fn of(editor: &Editor) -> Option<Arc<Actions>> {
        editor.services.get::<Actions>()
    }

    pub(crate) fn set_facts(&self, facts: Facts) {
        *self.facts.lock() = facts;
    }

    /// Runs `f` with what is in front of the author now.
    pub fn with_cx<R>(&self, editor: &Editor, f: impl FnOnce(&ActionCx<'_>) -> R) -> R {
        let facts = self.facts.lock().clone();
        let cx = ActionCx {
            editor,
            tab: &facts.tab,
            kind: &facts.kind,
            path: &facts.path,
            project: facts.project,
            dirty: facts.dirty,
            edits: &facts.edits,
            drawer: facts.drawer,
        };
        f(&cx)
    }
}

/// What is bound to an action now, as a menu prints it ("Ctrl+Shift+F"); empty for none or
/// without a window. A surface's own menu row reads its keys here, never a literal.
#[must_use]
pub fn keys_label(editor: &Editor, id: &str) -> String {
    Actions::of(editor).map(|a| a.table.read().keys_label(id)).unwrap_or_default()
}

/// A menu row of an action, drawn by a surface in its own menu: the action's title (or `label`),
/// its icon, its keys as bound now, off with its reason, checked when it is. Picked, it runs.
pub fn menu_row(ui: &mut egui::Ui, cx: &mut crate::doc::DocCx<'_>, id: &str, label: Option<&str>) -> bool {
    let Some(actions) = Actions::of(cx.editor) else { return false };
    let Some(a) = actions.table.read().get(id).copied() else { return false };
    let keys = actions.table.read().keys_label(id);
    let (on, checked) =
        actions.with_cx(cx.editor, |acx| ((a.enabled)(acx), a.checked.is_some_and(|f| f(acx))));
    let item = gz_ui::MenuItem {
        icon: a.icon.unwrap_or(Icon::None),
        keys: &keys,
        enabled: on.on(),
        selected: checked,
        tip: on.why(),
        ..gz_ui::MenuItem::default()
    };
    let picked = gz_ui::menu_item(ui, label.unwrap_or(a.title), item);
    if picked {
        cx.requests.action(id);
    }
    picked
}

/// A toolbar button of an action: its icon and title (folded to the icon under pressure), its tip
/// with its keys, off with its reason, checked as a toggle. Pressed, it runs.
pub fn toolbar_button(
    tb: &mut gz_ui::Toolbar<'_, '_>,
    cx: &mut crate::doc::DocCx<'_>,
    id: &str,
    kind: gz_ui::Kind,
    label: Option<&str>,
) -> bool {
    let Some(actions) = Actions::of(cx.editor) else { return false };
    let Some(a) = actions.table.read().get(id).copied() else { return false };
    let keys = actions.table.read().keys_label(id);
    let (on, checked) =
        actions.with_cx(cx.editor, |acx| ((a.enabled)(acx), a.checked.is_some_and(|f| f(acx))));
    let title = label.unwrap_or(a.title).trim_end_matches("...");
    let tip = match (on, keys.is_empty()) {
        (Enabled::Off(why), _) => why.to_owned(),
        (Enabled::On, true) => title.to_owned(),
        (Enabled::On, false) => format!("{title} ({keys})"),
    };
    let pressed = tb.action(id, a.icon.unwrap_or(Icon::None), title, &tip, on.on(), checked, kind);
    if pressed {
        cx.requests.action(id);
    }
    pressed
}

/// Runs an action's command from the window: off the UI thread, with the window's origin; its
/// answer goes to the action's `reply`, or a failure becomes a notice.
pub fn run(editor: &Arc<Editor>, action: &Action, cx: &ActionCx<'_>) {
    let a = (action.args)(cx);
    let (command, reply) = (action.command, action.reply);
    let ed = editor.clone();
    let origin = editor.ui_origin();
    editor.exec.spawn(async move {
        let answer = ed.call(command, a, origin).await;
        let ok = answer.get("ok").and_then(Value::as_bool) == Some(true);
        match reply {
            Some(show) => {
                let ed2 = ed.clone();
                if let Ok(q) = crate::queue::UiQueue::of(&ed) {
                    q.post(move |_shell, ctx| show(&ReplyCx { editor: &ed2, ctx, reply: &answer }));
                }
            }
            None if !ok => {
                let text =
                    answer.get("error").and_then(Value::as_str).unwrap_or("The command failed.").to_owned();
                ed.notices.error(text);
            }
            None => {}
        }
    });
}

/// One action as `app.keys` and the Keyboard page list it.
#[must_use]
pub fn key_row(table: &ActionTable, a: &Action) -> Value {
    json!({
        "id": a.id,
        "title": a.title,
        "group": a.group,
        "keys": table.keys_of(a.id).iter().map(Chord::label).collect::<Vec<_>>(),
        "default": a.keys.iter().map(Chord::label).collect::<Vec<_>>(),
        "scope": a.scope.describe(),
        "command": a.command,
        "conflicts": table.conflicts(a.id),
    })
}

#[cfg(test)]
mod tests {
    use egui::Key;

    use super::{Action, ActionCx, ActionTable, Enabled, Scope};
    use crate::keys::Chord;

    const QUEST: &[&str] = &["quest", "beh"];

    fn cx<'a>(editor: &'a gz_editor::Editor, kind: &'a str) -> ActionCx<'a> {
        ActionCx { editor, tab: kind, kind, path: "", project: true, dirty: false, edits: &[], drawer: None }
    }

    fn table() -> ActionTable {
        let mut t = ActionTable::default();
        t.add(
            Action::new("mod.build", "Build Into Game", "Mod", "mod.build")
                .keys(const { &[Chord::ctrl(Key::B)] })
                .scope(Scope::Tab(&["level"])),
        );
        t.add(
            Action::new("graph.bookmark", "Bookmark", "Graph", "graph.action")
                .keys(const { &[Chord::ctrl(Key::B)] })
                .scope(Scope::Part(QUEST, "canvas")),
        );
        t.add(Action::new("edit.undo", "Undo", "Edit", "doc.undo").keys(const { &[Chord::ctrl(Key::Z)] }));
        t.add(
            Action::new("edit.off", "Off", "Edit", "x")
                .keys(const { &[Chord::new(Key::Delete)] })
                .enabled(|_| Enabled::Off("no")),
        );
        t
    }

    #[test]
    fn a_tabs_own_keys_win_and_an_action_that_is_off_lets_the_key_go() {
        let (rt, ed) = gz_editor_for_tests();
        let t = table();
        let b = Chord::ctrl(Key::B);
        assert_eq!(t.for_chord(&b, &cx(&ed, "level"), None, false).map(|a| a.id), Some("mod.build"));
        assert_eq!(
            t.for_chord(&b, &cx(&ed, "quest"), Some("canvas"), false).map(|a| a.id),
            Some("graph.bookmark")
        );
        // the canvas does not have the keys: Ctrl+B means nothing on the graph tab
        assert!(t.for_chord(&b, &cx(&ed, "quest"), None, false).is_none());
        assert_eq!(
            t.for_chord(&Chord::ctrl(Key::Z), &cx(&ed, "characters"), None, false).map(|a| a.id),
            Some("edit.undo")
        );
        assert!(t.for_chord(&Chord::new(Key::Delete), &cx(&ed, "level"), None, false).is_none());
        // a panel that answers its own keys keeps Ctrl+Z
        assert!(t.for_chord(&Chord::ctrl(Key::Z), &cx(&ed, "level"), None, true).is_none());
        drop(rt);
    }

    #[test]
    fn a_binding_takes_its_chord_from_an_overlapping_scope_only() {
        let (rt, ed) = gz_editor_for_tests();
        let settings =
            crate::settings::EditorSettings::load(std::path::Path::new("Z:/nowhere/grizzly_editor.ltx"));
        let mut t = table();
        // Ctrl+Z bound to Build Into Game: Undo (everywhere) overlaps the level and lets it go
        let moved = t.bind("mod.build", vec![Chord::ctrl(Key::Z)], &settings);
        assert_eq!(moved, vec!["edit.undo".to_owned()]);
        assert!(t.keys_of("edit.undo").is_empty());
        assert_eq!(t.keys_label("mod.build"), "Ctrl+Z");
        // a graph's Ctrl+B does not overlap the level's: nobody loses it
        let moved = t.bind("mod.build", vec![Chord::ctrl(Key::B)], &settings);
        assert!(moved.is_empty());
        assert!(t.conflicts("graph.bookmark").is_empty());
        t.reset(None, &settings);
        assert_eq!(t.keys_label("edit.undo"), "Ctrl+Z");
        assert!(Scope::Global.overlaps(Scope::Tab(&["level"])));
        assert!(!Scope::Tab(&["level"]).overlaps(Scope::Part(QUEST, "canvas")));
        let _ = ed;
        drop(rt);
    }

    fn gz_editor_for_tests() -> (gz_task::Runtime, std::sync::Arc<gz_editor::Editor>) {
        let rt =
            gz_task::Runtime::start(gz_task::ExecOptions { async_threads: 1, cpu_threads: 1, io_threads: 1 })
                .expect("runtime");
        let ed = gz_editor::Editor::new(
            gz_editor::EditorOptions::default(),
            rt.exec(),
            gz_log::Log::memory(),
            gz_editor::Registry::new(),
        );
        (rt, ed)
    }
}
