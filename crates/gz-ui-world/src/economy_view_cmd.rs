//! `economy.view`: the Economy tab for whoever looks at it, and drives it the way a person does -
//! picks a page, selects a file, makes one, adds a row or takes one away - and answers what the
//! tab shows. The data stays the files themselves (`economy.list`, `asset.check`).

use std::sync::Arc;

use gz_app::{ShellState, on_ui};
use gz_base::Origin;
use gz_editor::{CmdError, Editor, Registry, Reply, Spec, Value};
use gz_items::{Items, Rows, TableFile, TableKind};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::call::{args, error_text, ok, s};
use crate::economy::{self, EconomyDoc, PAGES};

fn economy_doc(shell: &mut ShellState) -> Option<&mut EconomyDoc> {
    shell.document_mut(economy::TAB).and_then(|d| d.as_any_mut()).and_then(|a| a.downcast_mut::<EconomyDoc>())
}

fn gone() -> CmdError {
    CmdError::new("refused", "the Economy tab closed meanwhile")
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct EconomyView {
    /// The page to show: traders, loot or recipes.
    #[serde(default)]
    page: Option<String>,
    /// Selects a file of the module's traders, loot tables or recipes (project-relative); its page
    /// comes up.
    #[serde(default)]
    select: Option<String>,
    /// Makes a new file of the page's kind, as the page's Add control does, and selects it: for
    /// the traders page the trader it is about (`trade_military`), for the other two `true`.
    #[serde(default)]
    new: Option<Value>,
    /// Adds a row to the selected file, written as the file writes one: `{"item": "bread",
    /// "count": 2}` for a trader, `{"item": "bread", "chance": 0.1}` for a loot table,
    /// `{"makes": "bread", "at": "itm_repairkit_tier_1", "parts": ["jar"]}` for recipes.
    #[serde(default)]
    add: Option<serde_json::Map<String, Value>>,
    /// Takes the row with this number (from 0) out of the selected file.
    #[serde(default)]
    remove: Option<usize>,
    /// Opens the tab without changing what it shows.
    #[serde(default)]
    open: bool,
    /// Puts the question of the file's Delete... up to be looked at; nothing is deleted.
    #[serde(default)]
    ask_delete: bool,
}

pub(crate) fn register(reg: &mut Registry) {
    // the area is gz-items' own: `economy.list`
    reg.cmd(
        Spec::view(
            "economy.view",
            "Puts the Economy tab up on a page or a file of the module's traders, loot tables and recipes - selected, made, a row added or taken away - and answers what it shows.",
        )
        .example("select=trade/military.tradeasset add={\"item\": \"bread\", \"count\": 2}"),
        economy_view,
    );
}

/// Brings the tab up through the shell's own opener; a window-less editor has none to bring.
async fn open_tab(ed: &Arc<Editor>, origin: Origin) -> Result<(), CmdError> {
    let reply = ed.call("ui.tab", args([("action", s("open")), ("kind", s(economy::TAB))]), origin).await;
    if !ok(&reply) && reply.get("error").and_then(Value::as_str) != Some("the editor runs without a window") {
        return Err(CmdError::new("refused", error_text(&reply)));
    }
    Ok(())
}

async fn is_open(ed: &Arc<Editor>) -> Result<bool, CmdError> {
    match on_ui(ed, |shell, _| Ok(shell.document_mut(economy::TAB).is_some())).await {
        Ok(v) => Ok(v),
        Err(e) if e.code == "no_window" => Ok(false),
        Err(e) => Err(e),
    }
}

fn page_kind(word: &str) -> Result<TableKind, CmdError> {
    let w = word.trim();
    PAGES
        .iter()
        .find(|(label, kind)| {
            w.eq_ignore_ascii_case(label)
                || w.eq_ignore_ascii_case(kind.name())
                || w.eq_ignore_ascii_case(kind.header())
        })
        .map(|(_, kind)| *kind)
        .ok_or_else(|| CmdError::invalid(format!("page is traders, loot or recipes, not '{w}'")))
}

/// A row written as the file writes one, read by the kind's own reader: the file with that row
/// added.
fn with_row(table: &TableFile, row: &serde_json::Map<String, Value>) -> Result<TableFile, CmdError> {
    let kind = table.kind();
    let mut probe = gz_luadata::Table::new();
    probe.put(kind.header(), gz_items::tables::VERSION);
    if let Rows::Trade { trader, .. } = &table.rows {
        probe.put("trader", trader.as_str());
    }
    probe.put(kind.rows(), gz_luadata::Value::from(vec![gz_luadata::from_json(&Value::Object(row.clone()))]));
    let (read, issues) =
        gz_items::tables::from_value(kind, &gz_luadata::Value::Table(probe)).map_err(CmdError::invalid)?;
    if let Some(i) = issues.iter().find(|i| i.is_error()) {
        return Err(CmdError::invalid(format!("the row cannot be one of the file: {}", i.message)));
    }
    let mut out = table.clone();
    match (&mut out.rows, read.rows) {
        (Rows::Trade { stock, .. }, Rows::Trade { stock: new, .. }) => stock.extend(new),
        (Rows::Loot(drops), Rows::Loot(new)) => drops.extend(new),
        (Rows::Recipes(list), Rows::Recipes(new)) => list.extend(new),
        _ => return Err(CmdError::internal("the row is of another kind")),
    }
    Ok(out)
}

fn without_row(table: &TableFile, row: usize) -> Result<TableFile, CmdError> {
    let mut out = table.clone();
    let len = out.rows.len();
    if row >= len {
        return Err(CmdError::invalid(format!("the file has {len} rows: there is no row {row}")));
    }
    match &mut out.rows {
        Rows::Trade { stock, .. } => drop(stock.remove(row)),
        Rows::Loot(drops) => drop(drops.remove(row)),
        Rows::Recipes(list) => drop(list.remove(row)),
    }
    Ok(out)
}

/// Reads the module's files again and hands them to the tab, with a file to select.
async fn refresh(ed: &Arc<Editor>, select: Option<String>) -> Result<(), CmdError> {
    let loaded = economy::load(ed, true).await;
    on_ui(ed, move |shell, _| {
        let doc = economy_doc(shell).ok_or_else(gone)?;
        doc.take_loaded(loaded);
        if let Some(file) = &select {
            doc.open_file(file);
        }
        Ok(())
    })
    .await
}

pub(crate) async fn economy_view(ed: Arc<Editor>, a: EconomyView, origin: Origin) -> Reply {
    if ed.project_opt().is_none() {
        return Err(CmdError::no_project());
    }
    let page = match a.page.as_deref() {
        Some(p) => Some(page_kind(p)?),
        None => None,
    };
    let wants = page.is_some()
        || a.select.is_some()
        || a.new.is_some()
        || a.add.is_some()
        || a.remove.is_some()
        || a.open
        || a.ask_delete;
    if wants {
        open_tab(&ed, origin).await?;
    }
    if !is_open(&ed).await? {
        return Ok(economy::closed_json());
    }
    // the files as they are now: a write over MCP may have come after the tab last looked
    refresh(&ed, None).await?;
    if let Some(kind) = page {
        on_ui(&ed, move |shell, _| {
            economy_doc(shell).ok_or_else(gone)?.set_page(kind);
            Ok(())
        })
        .await?;
    }
    if let Some(file) = a.select.clone() {
        let found = on_ui(&ed, move |shell, _| {
            let doc = economy_doc(shell).ok_or_else(gone)?;
            doc.open_file(&file);
            Ok(doc.sel.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(&file.replace('\\', "/"))))
        })
        .await?;
        if !found {
            return Err(CmdError::new(
                "not_found",
                format!(
                    "'{}' is no trader, loot table or recipes file of the module (economy.list lists them)",
                    a.select.as_deref().unwrap_or_default()
                ),
            ));
        }
    }
    // what is written: a new file, or the selected one with a row added or taken away
    let mut written: Option<TableFile> = None;
    if let Some(new) = &a.new {
        let trader = match new {
            Value::String(t) => t.clone(),
            Value::Bool(true) => String::new(),
            _ => {
                return Err(CmdError::invalid(
                    "new is the trader the file is about (the traders page), or true (loot, recipes)",
                ));
            }
        };
        written = Some(
            on_ui(&ed, move |shell, _| {
                economy_doc(shell).ok_or_else(gone)?.plan_new(&trader).map_err(CmdError::invalid)
            })
            .await?,
        );
    }
    if a.add.is_some() || a.remove.is_some() {
        let shown = match &written {
            Some(t) => Some(t.clone()),
            None => on_ui(&ed, |shell, _| Ok(economy_doc(shell).ok_or_else(gone)?.shown.clone())).await?,
        };
        let Some(mut table) = shown else {
            return Err(CmdError::invalid("no file is selected: select one, or make one with new"));
        };
        if let Some(i) = a.remove {
            table = without_row(&table, i)?;
        }
        if let Some(row) = &a.add {
            table = with_row(&table, row)?;
        }
        written = Some(table);
    }
    if let Some(table) = written {
        let items = Items::of(&ed).await?;
        let file = items.save_table(&ed, None, &table).await?;
        refresh(&ed, Some(file)).await?;
    }
    let ask = a.ask_delete;
    on_ui(&ed, move |shell, _| {
        let active = shell.active() == economy::TAB;
        let doc = economy_doc(shell).ok_or_else(gone)?;
        if ask {
            doc.look_delete();
        }
        Ok(doc.view_json(active))
    })
    .await
}
