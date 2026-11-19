//! `text.view`: a text file in its Text tab for whoever looks at it - opened, the caret at a line,
//! a query found, the Markdown view picked, a pick list put up - and what the tab shows. The text
//! itself is the file's: an agent reads and writes it with its own tools.

use std::sync::Arc;

use gz_app::{ShellState, on_ui};
use gz_base::Origin;
use gz_editor::{CmdError, Editor, Registry, Reply, Spec, Value, json};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::doc::{KIND, TextDoc};
use crate::find::Query;
use crate::model::Source;
use crate::tabs::{Mode, Show, TextTabs};

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TextView {
    /// The file: project-relative, or a path of the linked game with source=game; left out: the
    /// Text tab that is up.
    #[serde(default)]
    path: Option<String>,
    /// `game` for a file of the linked game, which opens read-only; left out: the project's.
    #[serde(default)]
    source: Option<String>,
    /// The caret to this 1-based line, scrolled into view.
    #[serde(default)]
    line: Option<u32>,
    /// With line: the 1-based column, in characters.
    #[serde(default)]
    column: Option<u32>,
    /// Finds this text: the find bar opens with it and marks every match.
    #[serde(default)]
    find: Option<String>,
    /// With find: capitals and small letters told apart.
    #[serde(default)]
    case: bool,
    /// With find: only where it stands as a word of its own.
    #[serde(default)]
    whole_word: bool,
    /// A Markdown file: edit, split or preview.
    #[serde(default)]
    mode: Option<String>,
    /// Word Wrap on or off.
    #[serde(default)]
    wrap: Option<bool>,
    /// A part of the tab to put up to be looked at: find, symbols (Go to Symbol's list),
    /// complete (the editor with the keys, for completion), none (closes the find bar).
    #[serde(default)]
    show: Option<String>,
    /// Drops the tab's unsaved edits for the file's text, when the file changed on disk under
    /// them (the banner's Take Theirs).
    #[serde(default)]
    take_theirs: bool,
}

pub(crate) fn register(reg: &mut Registry) {
    reg.area("text", "A text file of the project in the Text tab: scripts, configs, XML, JSON, Markdown");
    reg.cmd(
        Spec::view(
            "text.view",
            "Opens a text file in its Text tab - the caret at a line, a query found, the Markdown view picked, a pick list up - and answers what the tab shows: language, encoding, line endings, problems, symbols, matches.",
        )
        .example("path=scripts/item_animations.script line=120 find=on_item_use")
        .destroys_with(&["take_theirs"]),
        text_view,
    );
}

fn text_doc<'a>(shell: &'a mut ShellState, id: &str) -> Option<&'a mut TextDoc> {
    shell.document_mut(id).and_then(|d| d.as_any_mut()).and_then(|a| a.downcast_mut::<TextDoc>())
}

fn parse_show(word: &str) -> Result<Show, CmdError> {
    match word.trim().to_ascii_lowercase().as_str() {
        "find" => Ok(Show::Find),
        "symbols" => Ok(Show::Symbols),
        "complete" => Ok(Show::Complete),
        "none" | "" => Ok(Show::None),
        w => Err(CmdError::invalid(format!("show is find, symbols, complete or none, not '{w}'"))),
    }
}

pub(crate) async fn text_view(ed: Arc<Editor>, a: TextView, origin: Origin) -> Reply {
    if ed.project_opt().is_none() {
        return Err(CmdError::no_project());
    }
    let mode = match a.mode.as_deref() {
        Some(m) => Some(
            Mode::from_id(m)
                .ok_or_else(|| CmdError::invalid(format!("mode is edit, split or preview, not '{m}'")))?,
        ),
        None => None,
    };
    let show = a.show.as_deref().map(parse_show).transpose()?;
    let game = match a.source.as_deref().map(str::trim) {
        None | Some("" | "project") => false,
        Some("game") => true,
        Some(s) => return Err(CmdError::invalid(format!("source is project or game, not '{s}'"))),
    };
    // the tab: the file named, or the Text tab that is up
    let id = match a.path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => {
            let source = Source::parse(p, game);
            let id = source.tab_id();
            let mut pairs = vec![("action", json!("open")), ("kind", json!(KIND)), ("path", json!(p))];
            if game {
                pairs.push(("source", json!("game")));
            }
            let mut args = gz_editor::Args::new();
            for (k, v) in pairs {
                args.insert(k.to_owned(), v);
            }
            let reply = ed.call("ui.tab", args, origin).await;
            if reply.get("ok").and_then(Value::as_bool) != Some(true) {
                let why =
                    reply.get("error").and_then(Value::as_str).unwrap_or("the tab did not open").to_owned();
                let code = reply.get("code").and_then(Value::as_str).unwrap_or("refused").to_owned();
                return Err(CmdError::new(&code, why));
            }
            id
        }
        None => {
            let active = on_ui(&ed, |shell, _| Ok(shell.active().to_owned())).await?;
            if !active.starts_with("text:") {
                return Err(CmdError::invalid("no Text tab is up: name the file with path"));
            }
            active
        }
    };
    let find = a.find.clone().map(|text| Query { text, case: a.case, whole_word: a.whole_word });
    let asks = a.line.is_some() || find.is_some() || mode.is_some() || a.wrap.is_some() || show.is_some();
    if asks {
        TextTabs::of(&ed).ask(&id, |ask| {
            if let Some(line) = a.line {
                ask.goto = Some((line.max(1), a.column.unwrap_or(1).max(1)));
            }
            ask.find.clone_from(&find);
            ask.mode = mode;
            ask.wrap = a.wrap;
            ask.show = show;
        });
    }
    if a.take_theirs {
        let wanted = id.clone();
        let took = on_ui(&ed, move |shell, _| Ok(text_doc(shell, &wanted).is_some_and(TextDoc::take_theirs)))
            .await?;
        if !took {
            return Err(CmdError::invalid("the file has no change on disk the tab keeps out"));
        }
    }
    // the tab takes the asks as it is drawn, and its first read lands in the frames after
    let _ = ed.frames.after(2).await;
    let mut waited = 0;
    loop {
        let wanted = id.clone();
        let state =
            on_ui(&ed, move |shell, _| Ok(text_doc(shell, &wanted).map(TextDoc::state_for_command))).await?;
        let Some(state) = state else {
            return Err(CmdError::new("refused", "the Text tab closed meanwhile"));
        };
        let busy = state.get("loading").and_then(Value::as_bool) == Some(true)
            || state.get("checking").and_then(Value::as_bool) == Some(true);
        if !busy || waited >= 100 {
            return Ok(state);
        }
        waited += 1;
        let _ = ed.frames.after(1).await;
    }
}
