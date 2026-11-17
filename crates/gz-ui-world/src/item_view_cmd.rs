//! `items.view`: the Items tab for whoever looks at it, and drives it the way a person does -
//! selects, makes a new item or a change from the + menu, edits the form, saves or drops the
//! edits - and answers everything the tab shows. The data stays the files themselves
//! (`items.list`, `items.info`, `asset.check`).

use std::sync::Arc;

use gz_app::{ShellState, on_ui};
use gz_base::Origin;
use gz_editor::{CmdError, Editor, Registry, Reply, Spec, Value, json};
use gz_items::SetValue;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::call::{args, error_text, ok, s};
use crate::item_model::{self, Own, Place, Row};
use crate::items::{self, ItemsDoc, Pick, SavePlan, Then, check, make_item, run_save};

pub(crate) fn items_doc(shell: &mut ShellState) -> Option<&mut ItemsDoc> {
    shell.document_mut(items::TAB).and_then(|d| d.as_any_mut()).and_then(|a| a.downcast_mut::<ItemsDoc>())
}

fn gone() -> CmdError {
    CmdError::new("refused", "the Items tab closed meanwhile")
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ItemsView {
    /// Selects an item: its file (project-relative) or the section it makes or changes.
    #[serde(default)]
    select: Option<String>,
    /// Makes a new item that starts from this section of the game, as the + menu's New Item...
    /// does (`items/<section>.itemasset`), and selects it.
    #[serde(default)]
    new: Option<String>,
    /// Makes a change of this section of the game, as Change a Game Item... does, and selects it.
    #[serde(default)]
    change: Option<String>,
    /// Sets keys of the selected item's section in the form, as its rows do: `{ "cost": 1500 }`;
    /// null takes the item's own value away (the Reset arrow); `hud.<key>` is a key of its HUD
    /// section. Nothing is written before `save`.
    #[serde(default)]
    set: Option<serde_json::Map<String, Value>>,
    /// Takes keys away (a change's `unset`); `hud.<key>` for the HUD section.
    #[serde(default)]
    unset: Option<Vec<String>>,
    /// Adds members to list keys: `{ "ammo_class": "ammo_x" }` or a list of members.
    #[serde(default)]
    add: Option<serde_json::Map<String, Value>>,
    /// Takes members out of list keys, the same way.
    #[serde(default)]
    drop: Option<serde_json::Map<String, Value>>,
    /// Sets texts: `{ "name.rus": "...", "description.eng": "..." }`; an empty text takes it away.
    #[serde(default)]
    text: Option<serde_json::Map<String, Value>>,
    /// Saves the edits into the item's file (after the edits of this call).
    #[serde(default)]
    save: bool,
    /// Drops the edits in progress (before anything else of this call).
    #[serde(default)]
    discard: bool,
    /// Puts a pick list or chooser of the tab up to be looked at: add (the + menu), new, change,
    /// base, or key:<key> (key:hud.<key>) for a key picked from a list.
    #[serde(default)]
    show: Option<String>,
    /// The row of the HUD section the In the Hands picture shows: a key of it. A motion line
    /// plays; any other row shows the idle, still (an aiming offset's row aimed).
    #[serde(default)]
    hud_row: Option<String>,
    /// Moves the In the Hands picture's motion to this many seconds (wrapped at its length).
    #[serde(default)]
    hands_time: Option<f32>,
    /// Plays (true) or stops (false) the In the Hands picture's motion where it is.
    #[serde(default)]
    hands_play: Option<bool>,
    /// Puts the Delete Item question up to be looked at; `ui.dialog action=close` takes it down.
    #[serde(default)]
    ask_delete: bool,
    /// Opens the tab without selecting anything.
    #[serde(default)]
    open: bool,
}

pub(crate) fn register(reg: &mut Registry) {
    // the area is gz-items' own: `items.list`, `items.info`
    reg.cmd(
        Spec::view(
            "items.view",
            "Puts the Items tab up on an item - selected, made from a game item, edited, saved - or a picker of it, and answers the list and the form it shows.",
        )
        .example("select=items/wpn_ak74.itemasset set={\"cost\": 1500} save=true")
        .destroys_with(&["discard"]),
        items_view,
    );
}

/// Brings the tab up through the shell's own opener; a window-less editor has none to bring.
async fn open_tab(ed: &Arc<Editor>, origin: Origin) -> Result<(), CmdError> {
    let reply = ed.call("ui.tab", args([("action", s("open")), ("kind", s(items::TAB))]), origin).await;
    if !ok(&reply) && reply.get("error").and_then(Value::as_str) != Some("the editor runs without a window") {
        return Err(CmdError::new("refused", error_text(&reply)));
    }
    Ok(())
}

async fn is_open(ed: &Arc<Editor>) -> Result<bool, CmdError> {
    match on_ui(ed, |shell, _| Ok(shell.document_mut(items::TAB).is_some())).await {
        Ok(v) => Ok(v),
        Err(e) if e.code == "no_window" => Ok(false),
        Err(e) => Err(e),
    }
}

fn set_value(v: &Value) -> Option<SetValue> {
    match v {
        Value::Bool(b) => Some(SetValue::Bool(*b)),
        Value::Number(n) => n.as_f64().map(SetValue::Number),
        Value::String(s) => Some(SetValue::Text(s.clone())),
        Value::Array(a) => a.iter().map(set_value).collect::<Option<Vec<_>>>().map(SetValue::List),
        _ => None,
    }
}

/// `hud.<key>` is a key of the HUD section.
fn scoped(key: &str) -> (&'static str, &str) {
    match key.strip_prefix("hud.") {
        Some(k) => (gz_items::catalog::HUD, k),
        None => (gz_items::catalog::ITEM, key),
    }
}

fn members(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect(),
        Value::String(s) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// The edits of a call, made in the form as its rows make them.
fn apply_edits(doc: &mut ItemsDoc, a: &ItemsView) -> Result<bool, CmdError> {
    let mut any = false;
    if let Some(set) = &a.set {
        for (key, v) in set {
            let (scope, k) = scoped(key);
            if v.is_null() {
                item_model::reset(doc.changes_mut(scope), k);
            } else {
                let value = set_value(v).ok_or_else(|| {
                    CmdError::invalid(format!(
                        "set.{key} takes a string, a number, a boolean or a list of them"
                    ))
                })?;
                item_model::set_own(doc.changes_mut(scope), k, value);
            }
            any = true;
        }
    }
    for key in a.unset.iter().flatten() {
        let (scope, k) = scoped(key);
        item_model::take_away(doc.changes_mut(scope), k);
        any = true;
    }
    let change = item_model::is_change(&doc.edit);
    for (list, adding) in [(&a.add, true), (&a.drop, false)] {
        for (key, v) in list.iter().flatten() {
            let (scope, k) = scoped(key);
            let row = form_row(doc, scope, k);
            let start = row.as_ref().and_then(|r| {
                if change { r.before.clone().or(r.resolved.clone()) } else { r.resolved.clone() }
            });
            for m in members(v) {
                let own = item_model::own_of(doc.changes_mut(scope), k);
                let base = if matches!(own, Own::Members { .. }) {
                    row.as_ref().and_then(|r| r.before.clone())
                } else {
                    start.clone()
                };
                let now = item_model::members_now(&own, base.as_deref());
                if adding {
                    item_model::add_member(doc.changes_mut(scope), k, &m, change, &now);
                } else {
                    item_model::drop_member(doc.changes_mut(scope), k, &m, change, &now);
                }
            }
            any = true;
        }
    }
    if let Some(texts) = &a.text {
        for (key, v) in texts {
            let at = items::TEXTS.iter().position(|(f, l)| format!("{f}.{l}") == *key).ok_or_else(|| {
                CmdError::invalid(format!(
                    "text.{key}: the texts are name, short_name and description, each .rus or .eng"
                ))
            })?;
            doc.texts[at] = v.as_str().unwrap_or_default().to_owned();
            any = true;
        }
    }
    if any {
        doc.changed();
    }
    Ok(any)
}

fn form_row(doc: &ItemsDoc, scope: &str, key: &str) -> Option<Row> {
    doc.form
        .groups
        .iter()
        .chain(doc.form.hud.iter().flat_map(|(_, g)| g.iter()))
        .flat_map(|g| g.rows.iter())
        .find(|r| r.scope == scope && r.key == key)
        .cloned()
}

fn parse_show(show: &str) -> Result<Option<Pick>, CmdError> {
    Ok(Some(match show {
        "add" => return Ok(None),
        "new" => Pick::New,
        "change" => Pick::Change,
        "base" => Pick::Base,
        other => match other.strip_prefix("key:") {
            Some(k) => {
                let (scope, key) = scoped(k);
                Pick::Key { scope: scope.to_owned(), key: key.to_owned() }
            }
            None => {
                return Err(CmdError::invalid(
                    "show must be add, new, change, base or key:<key> (key:hud.<key>)",
                ));
            }
        },
    }))
}

pub(crate) async fn items_view(ed: Arc<Editor>, a: ItemsView, origin: Origin) -> Reply {
    if ed.project_opt().is_none() {
        return Err(CmdError::no_project());
    }
    let show = a.show.clone().filter(|v| !v.is_empty());
    let shown = match &show {
        Some(v) => Some(parse_show(v)?),
        None => None,
    };
    let edits =
        a.set.is_some() || a.unset.is_some() || a.add.is_some() || a.drop.is_some() || a.text.is_some();
    let made = a.new.clone().map(|s| (false, s)).or_else(|| a.change.clone().map(|s| (true, s)));
    let wants = a.select.is_some()
        || made.is_some()
        || show.is_some()
        || a.open
        || edits
        || a.save
        || a.discard
        || a.ask_delete
        || a.hud_row.is_some()
        || a.hands_time.is_some()
        || a.hands_play.is_some();
    if wants {
        open_tab(&ed, origin).await?;
    }
    if !is_open(&ed).await? {
        return Ok(items::closed_json());
    }
    // the files as they are now: a write over MCP may have come after the tab last looked
    let loaded = items::load(&ed, true).await;
    let discard = a.discard;
    let dirty = on_ui(&ed, move |shell, _| {
        let doc = items_doc(shell).ok_or_else(gone)?;
        if let Some(l) = loaded {
            doc.take_loaded(l);
        }
        if discard {
            doc.discard_edits();
        }
        Ok(doc.is_dirty())
    })
    .await?;
    // what replaces the edited item: the caller decides what happens to the edits, MCP never asks
    let replaces = a.select.is_some() || made.is_some();
    if replaces && dirty && !a.save {
        let names =
            on_ui(&ed, |shell, _| Ok(items_doc(shell).map(|d| d.sel.clone().unwrap_or_default()))).await?;
        return Err(CmdError::unsaved(json!([names])).with_data(
            json!({ "dirty": [names], "note": "save=true or discard=true settles the edits first" }),
        ));
    }
    if replaces && dirty {
        save_now(&ed).await?;
    }
    let mut target = a.select.clone();
    if let Some((change, section)) = made {
        let file = make_item(&ed, change, &section).await.map_err(|e| CmdError::new("refused", e))?;
        target = Some(file);
        let loaded = items::load(&ed, true).await;
        on_ui(&ed, move |shell, _| {
            if let (Some(doc), Some(l)) = (items_doc(shell), loaded) {
                doc.take_loaded(l);
            }
            Ok(())
        })
        .await?;
    }
    if let Some(want) = target {
        let found = on_ui(&ed, move |shell, _| {
            let doc = items_doc(shell).ok_or_else(gone)?;
            let Some(file) = doc.find(&want) else { return Ok(Err(want)) };
            if doc.sel.as_deref() != Some(file.as_str()) {
                doc.select(Some(&file));
            }
            Ok(Ok(()))
        })
        .await?;
        if let Err(want) = found {
            return Err(CmdError::not_found(format!(
                "'{want}' is no item file of the module and no section one of its items makes or changes (items.list lists them)"
            )));
        }
    }
    // the form's model for the item as it is, so the edits find their rows
    settle(&ed).await?;
    if edits {
        let a2 = ItemsView {
            set: a.set.clone(),
            unset: a.unset.clone(),
            add: a.add.clone(),
            drop: a.drop.clone(),
            text: a.text.clone(),
            ..ItemsView::default()
        };
        on_ui(&ed, move |shell, _| {
            let doc = items_doc(shell).ok_or_else(gone)?;
            if doc.sel.is_none() {
                return Err(CmdError::new("refused", "no item is selected: select= names one"));
            }
            apply_edits(doc, &a2)?;
            Ok(())
        })
        .await?;
    }
    if a.save {
        save_now(&ed).await?;
    }
    settle(&ed).await?;
    let hud_row = a.hud_row.clone();
    let (hands_time, hands_play) = (a.hands_time, a.hands_play);
    let ask_delete = a.ask_delete;
    let answer = on_ui(&ed, move |shell, ctx| {
        let active = shell.active() == items::TAB;
        let doc = items_doc(shell).ok_or_else(gone)?;
        if let Some(r) = hud_row {
            doc.hud_row = (!r.is_empty()).then_some(r);
        }
        doc.refresh_hands();
        if let Some(view) = doc.hands.as_mut() {
            if let Some(t) = hands_time {
                view.set_time(t);
            }
            if let Some(on) = hands_play {
                view.set_playing(on);
            }
        }
        if ask_delete {
            if doc.sel.is_none() {
                return Err(CmdError::new("refused", "no item is selected: there is nothing to delete"));
            }
            doc.look_at_delete();
        }
        match shown {
            Some(None) => gz_ui::menu::show_popup(ctx, "items.add"),
            Some(Some(pick)) => {
                if matches!(pick, Pick::Base | Pick::Key { .. }) && doc.sel.is_none() {
                    return Err(CmdError::new("refused", "no item is selected: select= names one first"));
                }
                if let Pick::Key { scope, key } = &pick {
                    let row = form_row(doc, scope, key)
                        .ok_or_else(|| CmdError::not_found(format!("the form has no row {key}")))?;
                    if items::list_of(&row.ty).is_none() && row.ty != gz_items::KeyType::Section {
                        // a token's words: its own list under the row
                        gz_ui::menu::show_popup(ctx, &format!("item.key.{scope}.{key}"));
                    } else {
                        doc.open_chooser(pick);
                    }
                } else {
                    doc.open_chooser(pick);
                }
            }
            None => {}
        }
        ctx.request_repaint();
        Ok((doc.view_json(active), doc.chooser.is_some()))
    })
    .await?;
    let (answer, chooser) = answer;
    if !chooser {
        return Ok(answer);
    }
    // a chooser lists its rows in the frames after it came up: the answer says what it shows
    for _ in 0..40 {
        if ed.frames.after(2).await.is_err() {
            break;
        }
        let shown = on_ui(&ed, |shell, _| {
            Ok(items_doc(shell)
                .and_then(|d| d.chooser.as_ref().map(|(_, c)| c.state()["shown"].as_u64().unwrap_or(0))))
        })
        .await?;
        if shown.is_none_or(|n| n > 0) {
            break;
        }
    }
    on_ui(&ed, |shell, _| {
        let active = shell.active() == items::TAB;
        let doc = items_doc(shell).ok_or_else(gone)?;
        Ok(doc.view_json(active))
    })
    .await
}

/// Saves the edits of the tab and takes the answer in.
async fn save_now(ed: &Arc<Editor>) -> Result<(), CmdError> {
    let plan: Option<SavePlan> = on_ui(ed, |shell, _| {
        let doc = items_doc(shell).ok_or_else(gone)?;
        if !doc.is_dirty() {
            return Ok(None);
        }
        doc.save_plan()
            .map(Some)
            .map_err(|e| CmdError::new("refused", format!("the item was not saved: {e}")))
    })
    .await?;
    let Some(plan) = plan else { return Ok(()) };
    let file = run_save(ed, plan)
        .await
        .map_err(|e| CmdError::new("refused", format!("the item was not saved: {e}")))?;
    let loaded = items::load(ed, true).await;
    on_ui(ed, move |shell, _| {
        let doc = items_doc(shell).ok_or_else(gone)?;
        doc.saved(Ok(file), Then::Nothing);
        if let Some(l) = loaded {
            doc.take_loaded(l);
        }
        Ok(())
    })
    .await
}

/// Checks the item the tab shows now and makes its form: the answer says what it resolves to.
async fn settle(ed: &Arc<Editor>) -> Result<(), CmdError> {
    let input = on_ui(ed, |shell, _| {
        Ok(items_doc(shell).and_then(|d| d.sel.is_some().then(|| (d.edit.clone(), d.edit_gen))))
    })
    .await?;
    if let Some((edit, serial)) = input {
        let (res, issues, hints) = check(ed, &edit).await;
        on_ui(ed, move |shell, _| {
            if let Some(doc) = items_doc(shell) {
                doc.set_checked(serial, res, issues, hints);
                doc.refresh_form();
            }
            Ok(())
        })
        .await?;
    }
    Ok(())
}

fn row_json(doc: &ItemsDoc, r: &Row) -> Value {
    let place = Place::Key { scope: r.scope.clone(), key: r.key.clone() };
    json!({
        "key": r.key,
        "title": r.label(),
        "type": r.ty.word(),
        "unit": r.unit,
        "common": r.common,
        "required": r.required,
        "list": r.list,
        "value": r.shown(),
        "from": r.from,
        "base": r.base,
        "before": r.before,
        "own": r.own.word(),
        "sets": r.own.is_own(),
        "problems": problems_at(doc, &place),
    })
}

fn problems_at(doc: &ItemsDoc, at: &Place) -> Vec<Value> {
    doc.places
        .iter()
        .filter(|(p, _, _)| p == at)
        .map(|(_, tone, line)| json!({ "severity": if *tone == gz_ui::Tone::Error { "error" } else { "warning" }, "text": line }))
        .collect()
}

fn groups_json(doc: &ItemsDoc, groups: &[item_model::Group]) -> Vec<Value> {
    groups
        .iter()
        .map(|g| json!({ "group": g.id, "title": g.title, "rows": g.rows.iter().map(|r| row_json(doc, r)).collect::<Vec<_>>() }))
        .collect()
}

/// The form of the selected item, every section and row as the tab shows them.
pub(crate) fn form_json(doc: &ItemsDoc) -> Value {
    let e = &doc.edit;
    let field = |name: &str| problems_at(doc, &Place::Field(name.to_owned()));
    let part = |name: &str| -> Vec<Value> {
        doc.places
            .iter()
            .filter(|(p, _, _)| matches!(p, Place::Part { name: n, .. } if n == name))
            .map(|(_, tone, line)| json!({ "severity": if *tone == gz_ui::Tone::Error { "error" } else { "warning" }, "text": line }))
            .collect()
    };
    let texts: serde_json::Map<String, Value> = items::TEXTS
        .iter()
        .enumerate()
        .map(|(i, (f, l))| (format!("{f}.{l}"), json!({ "value": doc.texts[i], "game": doc.hints[i] })))
        .collect();
    let icon = match &e.icon {
        Some(gz_items::Icon::Picture { file, w, h, .. }) => {
            json!({ "form": "picture", "file": file, "w": w, "h": h })
        }
        Some(gz_items::Icon::Cell { texture, x, y, w, h }) => {
            json!({ "form": "cell", "texture": texture, "x": x, "y": y, "w": w, "h": h })
        }
        None => json!({ "form": "inherited" }),
    };
    let shown_icon = doc.edit_icon().map(|s| items::icon_json(&s));
    let other: Vec<Value> = doc
        .form
        .other
        .iter()
        .map(|o| json!({ "scope": o.scope, "key": o.key, "own": o.own.word(), "value": match &o.own { Own::Set(v) => Some(v.ltx()), _ => None } }))
        .collect();
    let scene = match &e.scene {
        Some(gz_items::Scene::Like(s)) => json!({ "like": s }),
        Some(gz_items::Scene::Table { like, keys, .. }) => {
            json!({ "like": like, "keys": keys.iter().map(|(k, v)| json!({ "key": k, "value": v.ltx() })).collect::<Vec<_>>() })
        }
        None => Value::Null,
    };
    let identity_problems = [field("section"), field("base"), field("id"), field("edit")].concat();
    let text_problems = [field("name"), field("short_name"), field("description")].concat();
    // who sells it, who carries it and what makes it: what the module's tables say of its section
    let found = match (&doc.loaded, doc.res.as_ref()) {
        (Some(l), Some(res)) => l.roster.found(&res.section),
        _ => Vec::new(),
    };
    let used = doc.form.family.as_ref().is_some_and(|f| doc.item_lists.scene_families.contains(f));
    json!({
        "file": e.file,
        "kind": if item_model::is_change(e) { "change" } else { "new" },
        "section": doc.res.as_ref().map(|r| r.section.clone()),
        "class": doc.form.class,
        "family": doc.form.family,
        "family_title": doc.form.family_title,
        "head": problems_at(doc, &Place::Head),
        "identity": {
            "section": e.section, "base": e.base, "id": e.id, "template": e.is_abstract(), "edit": e.edit,
            "problems": identity_problems,
        },
        "texts": texts,
        "text_problems": text_problems,
        "icon": { "own": icon, "shown": shown_icon, "drawn": doc.icon_drawn(), "problems": field("icon") },
        "groups": groups_json(doc, &doc.form.groups),
        "hud": doc.form.hud.as_ref().map(|(name, groups)| json!({ "section": name, "groups": groups_json(doc, groups), "problems": field("hud") })),
        "found": found,
        "scene": { "shown": e.scene.is_some() || used, "value": scene, "problems": part("scene") },
        "sections": e.sections.iter().map(|s| json!({
            "name": s.name, "edit": s.edit, "base": s.base,
            "set": s.changes.set.iter().map(|(k, v)| json!({ "key": k, "value": v.ltx() })).collect::<Vec<_>>(),
            "unset": s.changes.unset,
            "add": s.changes.add.iter().map(|(k, m)| json!({ "key": k, "members": m })).collect::<Vec<_>>(),
            "drop": s.changes.drop.iter().map(|(k, m)| json!({ "key": k, "members": m })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "other": other,
        "problems": doc.own.iter().map(|i| json!({ "code": i.code, "at": gz_luadata::path::show(&i.path), "text": i.message })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_show_names_a_pick_list_of_the_tab() {
        assert_eq!(parse_show("new").unwrap(), Some(Pick::New));
        assert_eq!(parse_show("add").unwrap(), None);
        assert_eq!(
            parse_show("key:hud.item_visual").unwrap(),
            Some(Pick::Key { scope: "hud".into(), key: "item_visual".into() })
        );
        assert!(parse_show("nowhere").is_err());
        assert_eq!(
            set_value(&json!([1, "a"])),
            Some(SetValue::List(vec![SetValue::Number(1.0), SetValue::Text("a".into())]))
        );
        assert_eq!(scoped("hud.anm_idle"), ("hud", "anm_idle"));
    }
}
