//! The Level tab's commands: `outliner_show`, `outliner_filter`, `scene_visibility`, and
//! `scene.choices` with the lists this crate reads.

use std::sync::Arc;

use gz_app::on_ui;
use gz_editor::{Args, ArgsExt, CmdError, Editor, JsonSchema, Origin, Registry, Reply, Spec, Value, json};
use gz_scene::props;
use gz_scene::{ObjClass, SceneCmd, SceneService};
use serde::Deserialize;

use crate::doc::with_level;
use crate::layout::PanelId;
use crate::outliner_model::{Filter, Folds, Group, Sort, Visibility, compute, types_to_hide};
use crate::{AREA, LEVEL, classes, outliner};

pub(crate) fn register(reg: &mut Registry) {
    reg.add(AREA, "outliner_show", outliner_show);
    reg.add(AREA, "outliner_filter", outliner_filter);
    reg.add(AREA, "scene_visibility", scene_visibility);
    // over the scene's own: with the lists the Level tab reads (textures, shaders, models, motions)
    reg.cmd(
        Spec::read("scene.choices", "What a property picks from: its words, or the items of its list.")
            .example("object=stalker_01 key=props.visual query=bandit"),
        scene_choices,
    );
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ChoicesParams {
    /// The object (left out: the first selected one).
    #[serde(default)]
    object: Option<String>,
    /// The property's key.
    key: String,
    /// A property of the level instead of an object's.
    #[serde(default)]
    world: bool,
    /// Only items that hold this text (any case).
    #[serde(default)]
    query: Option<String>,
    /// At most this many (default 500).
    #[serde(default)]
    limit: Option<usize>,
}

/// `scene.choices`: the scene's answer, with the items of the lists only this crate reads.
async fn scene_choices(ed: Arc<Editor>, p: ChoicesParams, _o: Origin) -> Reply {
    let (row, items) = gz_scene::choices(&ed, p.object.as_deref(), &p.key, p.world).await?;
    let list = match &row.kind {
        props::Kind::Choose(choice) => crate::lists::ListKey::of_choose(choice),
        _ => None,
    };
    let items = match (items, &list) {
        (Some(items), _) => items,
        (None, Some(list)) => crate::lists::Lists::wait(&ed, list).await.as_ref().clone(),
        (None, None) => Vec::new(),
    };
    let mut reply = gz_scene::choices_reply(&row, &items, p.query.as_deref(), p.limit);
    // an empty list says what is missing, of the thing it is missing from
    if items.is_empty()
        && let (Some(list), Value::Object(m)) = (&list, &mut reply)
    {
        m.insert("empty".into(), json!(list.empty_text()));
    }
    Ok(reply)
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
                .send(SceneCmd::ClassView { class: tool, visible: Some(visible), locked: None }, o)
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
            svc.store.send(SceneCmd::ClassView { class: c, visible: Some(true), locked: None }, o).await?;
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
