//! The Level tab's commands; the wire is `LevelMcpScene.cpp` (`outliner_show`,
//! `outliner_filter`, `scene_visibility`) and `LevelMcpProps.cpp` (`property_inspector`).

use std::sync::Arc;

use gz_app::on_ui;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Origin, Registry, Reply, Value, json};
use gz_scene::props;
use gz_scene::{ObjClass, SceneCmd, SceneService};

use crate::details::{self, SELECTION, WORLD};
use crate::doc::with_level;
use crate::layout::PanelId;
use crate::outliner_model::{Filter, Folds, Group, Sort, Visibility, compute, types_to_hide};
use crate::{AREA, LEVEL, classes, outliner};

pub(crate) fn register(reg: &mut Registry) {
    reg.add(AREA, "outliner_show", outliner_show);
    reg.add(AREA, "outliner_filter", outliner_filter);
    reg.add(AREA, "scene_visibility", scene_visibility);
    reg.add(AREA, "property_inspector", property_inspector);
}

/// `GetArgBool`: true / false, 1 / 0, the words, or the fallback.
fn arg_bool(a: &Args, key: &str) -> Option<bool> {
    match a.get(key)? {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => Some(n.as_f64().is_some_and(|v| v != 0.0)),
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn scene_of(ed: &Arc<Editor>) -> Result<Arc<SceneService>, CmdError> {
    let svc = SceneService::of(ed)?;
    if !svc.store.snapshot().valid {
        return Err(CmdError::new("no_scene", "no scene"));
    }
    Ok(svc)
}

async fn outliner_show(ed: Arc<Editor>, a: Args, _o: Origin) -> Reply {
    let open = arg_bool(&a, "open").unwrap_or(true);
    let now = on_ui(&ed, move |shell, ctx| {
        if open {
            // an opener: the Level tab comes up with the panel in front
            shell.activate(ctx, LEVEL);
        }
        with_level(shell, |doc| {
            if open {
                doc.open_panel(ctx, PanelId::Outliner);
            } else {
                doc.close_panel(ctx, PanelId::Outliner);
            }
            doc.panel_open(PanelId::Outliner)
        })
        .ok_or_else(|| CmdError::invalid("outliner unavailable"))
    })
    .await?;
    Ok(json!({ "open": now }))
}

/// The classes of the scene as the Outliner groups them (sorted by class name).
fn groups(state: &gz_scene::SceneState) -> Vec<Group> {
    let mut out: Vec<Group> = ObjClass::ALL
        .iter()
        .copied()
        .filter(|c| c.is_object_tool() && state.count(*c) > 0)
        .map(|c| Group { class: c, total: state.count(c), shown: 0 })
        .collect();
    out.sort_by_key(|g| g.class.name());
    out
}

async fn outliner_filter(ed: Arc<Editor>, a: Args, _o: Origin) -> Reply {
    let text = a.get("text").map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned));
    let types = a.get("types").map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned));
    let visibility = a.str("visibility").map(str::to_owned);
    let layout = a.str("layout").filter(|s| !s.is_empty()).map(str::to_owned);
    let sort = a.str("sort").filter(|s| !s.is_empty()).map(str::to_owned);
    let selected_only = arg_bool(&a, "selected_only");
    let svc = SceneService::of(&ed).map_err(|_| CmdError::invalid("scene unavailable"))?;
    let snap = svc.store.snapshot();
    let groups = groups(&snap);

    // a rejected request leaves none of its filter applied
    let next_visibility = match &visibility {
        Some(v) => Some(
            Visibility::parse(v)
                .ok_or_else(|| CmdError::invalid("visibility must be all, visible, or hidden"))?,
        ),
        None => None,
    };
    let next_hierarchy = match layout.as_deref() {
        Some(l) if l.eq_ignore_ascii_case("type") => Some(false),
        Some(l) if l.eq_ignore_ascii_case("hierarchy") => Some(true),
        Some(_) => return Err(CmdError::invalid("layout must be type or hierarchy")),
        None => None,
    };
    let next_sort = match &sort {
        Some(s) => Some(Sort::parse(s).ok_or_else(|| CmdError::invalid("sort must be scene, az or za"))?),
        None => None,
    };
    let hide = match &types {
        Some(t) => Some(types_to_hide(&groups, t).map_err(CmdError::invalid)?),
        None => None,
    };
    let (filter, folds): (Filter, Folds) = on_ui(&ed, move |shell, ctx| {
        with_level(shell, |doc| {
            // filtering a closed panel helps nobody; the tab stays
            if !doc.panel_open(PanelId::Outliner) {
                doc.open_panel(ctx, PanelId::Outliner);
            }
            let f = &mut doc.outliner.filter;
            if let Some(t) = text {
                f.text = t.chars().take(127).collect();
            }
            if let Some(s) = selected_only {
                f.selected_only = s;
            }
            if let Some(h) = hide {
                f.hidden = h;
            }
            if let Some(v) = next_visibility {
                f.visibility = v;
            }
            if let Some(s) = next_sort {
                f.sort = s;
            }
            if let Some(h) = next_hierarchy {
                f.hierarchy = h;
            }
            (doc.outliner.filter.clone(), doc.outliner.folds.clone())
        })
        .ok_or_else(|| CmdError::invalid("outliner unavailable"))
    })
    .await?;
    let s = snap.clone();
    let f = filter.clone();
    let listing = ed.exec.cpu(move || compute(&s, &f, &folds)).await;
    let hidden: Vec<&str> =
        listing.groups.iter().filter(|g| filter.class_hidden(g.class)).map(|g| g.class.name()).collect();
    Ok(json!({
        "text": filter.text,
        "selected_only": filter.selected_only,
        "hidden_types": hidden.join(";"),
        "visibility": filter.visibility.key(),
        "layout": if filter.hierarchy { "hierarchy" } else { "type" },
        "sort": filter.sort.key(),
        "shown": listing.shown,
        "total": listing.total,
    }))
}

async fn scene_visibility(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = scene_of(&ed)?;
    let class = a.str("class").filter(|s| !s.is_empty()).map(str::to_owned);
    let name = a.str("name").filter(|s| !s.is_empty()).map(str::to_owned);
    let selection = arg_bool(&a, "selection").unwrap_or(false);
    let all = arg_bool(&a, "all").unwrap_or(false);
    let targets = usize::from(class.is_some())
        + usize::from(name.is_some())
        + usize::from(selection)
        + usize::from(all);
    let has_visible = a.get("visible").is_some_and(|v| !matches!(v, Value::Null) && v.as_str() != Some(""));
    let visible = arg_bool(&a, "visible").unwrap_or(true);
    if targets > 1 {
        return Err(CmdError::invalid("pass one of class, name, selection=true or all=true"));
    }
    if targets > 0 && !has_visible {
        return Err(CmdError::invalid("visible=true or visible=false says what to do"));
    }
    let snap = svc.store.snapshot();
    let mut changed = 0usize;
    if let Some(c) = &class {
        let tool = ObjClass::ALL.iter().copied().find(|x| x.name().eq_ignore_ascii_case(c));
        let Some(tool) = tool else {
            return Err(CmdError::invalid(format!(
                "no class '{c}': the classes are listed by scene_visibility without arguments"
            )));
        };
        if snap.tools.part(tool).visible != visible {
            svc.store
                .send(SceneCmd::ToolFlags { class: tool, visible: Some(visible), readonly: None }, o)
                .await?;
            changed = 1;
        }
    } else if let Some(n) = &name {
        let Some(obj) = snap.find_relaxed(n) else {
            return Err(CmdError::invalid(format!("no object '{n}'")));
        };
        if obj.flags.visible != visible {
            svc.store.send(SceneCmd::Show { ids: vec![obj.id], visible }, o).await?;
            changed = 1;
        }
    } else if targets > 0 {
        let (ids, classes_) = outliner::plan_show(&snap, selection, visible);
        changed = ids.len();
        for c in classes_ {
            svc.store.send(SceneCmd::ToolFlags { class: c, visible: Some(true), readonly: None }, o).await?;
        }
        if !ids.is_empty() {
            svc.store.send(SceneCmd::Show { ids, visible }, o).await?;
        }
    }
    let s = svc.store.snapshot();
    let list: Vec<Value> = ObjClass::ALL
        .iter()
        .map(|c| {
            let (objects, hidden) = if c.is_object_tool() { (s.count(*c), s.of_class(*c).filter(|x| !x.flags.visible).count()) } else { (0, 0) };
            json!({ "class": c.name(), "title": classes::title(*c), "visible": s.tools.part(*c).visible, "objects": objects, "hidden_objects": hidden })
        })
        .collect();
    Ok(json!({ "changed": changed, "classes": list }))
}

fn target_index(t: &str) -> Result<usize, CmdError> {
    match t {
        "selection" => Ok(SELECTION),
        "world" => Ok(WORLD),
        _ => Err(CmdError::invalid("target must be selection or world")),
    }
}

/// `property_inspector`: `gz-scene`'s answer, with the panels' side - `open` brings the Level
/// tab up with the panel in front, `close` closes it, `expand` / `collapse` fold its grid.
async fn property_inspector(ed: Arc<Editor>, a: Args, _o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let filter_arg = a.str("filter").unwrap_or("").to_owned();
    let action = a
        .text("action")
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| if filter_arg.is_empty() { "get".into() } else { "filter".into() });
    let target_name = a.text("target").unwrap_or("selection").to_ascii_lowercase();
    let t = target_index(&target_name)?;
    if !matches!(action.as_str(), "get" | "filter" | "expand" | "collapse" | "open" | "close") {
        return Err(CmdError::invalid("action must be get, filter, expand, collapse, open, or close"));
    }
    if action == "filter"
        && let Some(f) = svc.view.lock().filter.get_mut(t)
    {
        f.clone_from(&filter_arg);
    }
    let panel = if t == WORLD { PanelId::LevelSettings } else { PanelId::Details };
    if matches!(action.as_str(), "open" | "close" | "expand" | "collapse") {
        let act = action.clone();
        let shown = on_ui(&ed, move |shell, ctx| {
            if act == "open" {
                shell.activate(ctx, LEVEL);
            }
            Ok(with_level(shell, |doc| {
                match act.as_str() {
                    "open" => doc.open_panel(ctx, panel),
                    "close" => doc.close_panel(ctx, panel),
                    "expand" => doc.fold(t, true),
                    _ => doc.fold(t, false),
                }
                doc.panel_open(panel)
            }))
        })
        .await;
        let open = match shown {
            Ok(Some(o)) => o,
            // without a window the view keeps the flag, as the C++ forms did
            _ => {
                action == "open"
                    || (action != "close" && svc.view.lock().open.get(t).copied().unwrap_or(false))
            }
        };
        if let Some(o) = svc.view.lock().open.get_mut(t) {
            *o = open;
        }
    }
    let (filter, open) = {
        let v = svc.view.lock();
        (v.filter.get(t).cloned().unwrap_or_default(), v.open.get(t).copied().unwrap_or(false))
    };
    let snap = svc.store.snapshot();
    let lib = svc.library.clone();
    let rows = ed.exec.cpu(move || details::rows(&snap, &lib, t)).await;
    let f = filter.to_lowercase();
    let visible = rows.iter().filter(|r| f.is_empty() || r.row.key.to_lowercase().contains(&f)).count();
    let limit = a.i64("limit").unwrap_or(200).max(0) as usize;
    let out: Vec<Value> = rows
        .iter()
        .take(limit)
        .map(|m| {
            let mut o = serde_json::Map::new();
            o.insert("key".into(), json!(m.row.key));
            o.insert("label".into(), json!(props::label_of(&m.row.key)));
            o.insert("value".into(), json!(if m.mixed { "(mixed)".to_owned() } else { m.row.text() }));
            o.insert("enabled".into(), json!(m.row.enabled));
            o.insert("mixed".into(), json!(m.mixed));
            o.insert("help".into(), json!(props::help_of(&m.row)));
            let unit = props::unit_of(&m.row);
            if !unit.is_empty() {
                o.insert("unit".into(), json!(unit));
            }
            Value::Object(o)
        })
        .collect();
    Ok(
        json!({ "target": target_name, "filter": filter, "visible": visible, "total": rows.len(), "open": open, "rows": out }),
    )
}
