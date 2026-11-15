//! `level.view`: the Level tab's left panel and World Outliner as they are drawn - the mode and
//! the panel it shows, what the Place panel lists and has armed, the Outliner's filter and rows,
//! the Paint and Zones panels - and the view changes a person makes there: the searches, the
//! filters, a category or folder opened, a thing armed for Place mode, the Colour Indices
//! dialog. Nothing an author keeps changes (the effect is `view`); arming puts Place mode up,
//! as a click on the row does.

use std::sync::Arc;

use gz_app::on_ui;
use gz_editor::{CmdError, Editor, JsonSchema, Origin, Registry, Reply, Spec, Value, json};
use gz_scene::SceneService;
use serde::Deserialize;

use crate::LEVEL;
use crate::doc::with_level;
use crate::layout::PanelId;
use crate::outliner_model::{Sort, Visibility, compute, types_to_hide};
use crate::place::{Category, Inputs, PlacePanel, What};
use gz_ui_viewport::place::{Kind, Placeable};

pub(crate) fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::view(
            "level.view",
            "The Level tab's Place, Paint and Zones panel and its World Outliner as drawn; sets their searches and filters and arms a thing to place.",
        )
        .example("place_search=barrel outliner_search=light kinds=light,glow"),
        level_view,
    );
}

/// The Outliner's visibility filter.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum VisibilityParam {
    All,
    Visible,
    Hidden,
}

/// The Outliner's order inside a kind.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum SortParam {
    Scene,
    Az,
    Za,
}

/// By Type or By Hierarchy.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum LayoutParam {
    Type,
    Hierarchy,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ViewParams {
    /// The Place panel's search; an empty text clears it.
    #[serde(default)]
    place_search: Option<String>,
    /// Arms a thing for Place mode, as a click on its row: a `scene.place` kind (object, group,
    /// light, glow, sound, sound_env, particles, shape, fog, spawn, way); `none` stops placing.
    #[serde(default)]
    place: Option<String>,
    /// With `place`: the library object (or a model asset by its project path,
    /// `models/wall.meshasset`), group, sound or effect.
    #[serde(default)]
    r#ref: Option<String>,
    /// With `place=spawn`: the spawn section.
    #[serde(default)]
    section: Option<String>,
    /// With `place=shape`: a box instead of a sphere.
    #[serde(default)]
    r#box: bool,
    /// With `place=shape`: what the shape is for (`ai_map_bounds`).
    #[serde(default)]
    usage: Option<String>,
    /// A Place category to open or fold with `open`: recent, objects, lights, spawn_points,
    /// shapes, sounds, particles, way_paths, groups.
    #[serde(default)]
    category: Option<String>,
    /// With `category`: a folder of it (`props\barrels`) to open or close instead.
    #[serde(default)]
    folder: Option<String>,
    /// Opens (true) or folds (false) the category or the folder.
    #[serde(default)]
    open: Option<bool>,
    /// The Outliner's search (every word has to match, `-word` leaves those out).
    #[serde(default)]
    outliner_search: Option<String>,
    /// The Outliner shows only the selected objects.
    #[serde(default)]
    selected_only: Option<bool>,
    /// The Outliner shows every object, the visible ones or the hidden ones.
    #[serde(default)]
    visibility: Option<VisibilityParam>,
    /// The order of the objects in a kind: as in the scene, A-Z or Z-A.
    #[serde(default)]
    sort: Option<SortParam>,
    /// The Outliner by type or by hierarchy (groups with their members, way paths with their
    /// points).
    #[serde(default)]
    layout: Option<LayoutParam>,
    /// The kinds the Outliner shows, by their keys (light, spawn, way, ...); an empty list shows
    /// every kind.
    #[serde(default)]
    kinds: Option<Vec<String>>,
    /// Outliner rows to unfold: kinds by their key (By Type), groups and way paths by their name
    /// (By Hierarchy, where a way path lists its points).
    #[serde(default)]
    unfold: Option<Vec<String>>,
    /// Outliner rows to fold, named as `unfold` names them.
    #[serde(default)]
    fold: Option<Vec<String>>,
    /// Puts the Colour Indices dialog up (true) or takes it down (false).
    #[serde(default)]
    colour_indices: Option<bool>,
    /// How many rows of each list the answer gives; left out: 40.
    #[serde(default)]
    limit: Option<usize>,
}

impl ViewParams {
    fn changes_place(&self) -> bool {
        self.place_search.is_some() || self.place.is_some() || self.category.is_some()
    }

    fn changes_outliner(&self) -> bool {
        self.outliner_search.is_some()
            || self.selected_only.is_some()
            || self.visibility.is_some()
            || self.sort.is_some()
            || self.layout.is_some()
            || self.kinds.is_some()
            || self.unfold.is_some()
            || self.fold.is_some()
    }
}

/// What a click on a Place row would pick, from the parameters: `None` stops placing.
fn pick_of(p: &ViewParams) -> Result<Option<Option<What>>, CmdError> {
    let Some(word) = p.place.as_deref().map(str::trim) else { return Ok(None) };
    if word.eq_ignore_ascii_case("none") || word.is_empty() {
        return Ok(Some(None));
    }
    let kind = Kind::parse(word).ok_or_else(|| {
        CmdError::invalid(format!(
            "There is no kind {word} to place: object, group, light, glow, sound, sound_env, particles, shape, fog, spawn or way."
        ))
    })?;
    if kind == Kind::Shape && p.usage.as_deref() == Some("ai_map_bounds") {
        return Ok(Some(Some(What::Place(Placeable::new(kind, Some("ai_map_bounds"))))));
    }
    if let Some(u) = &p.usage {
        return Err(CmdError::invalid(format!(
            "usage={u} is not a shape the Place panel places: ai_map_bounds is"
        )));
    }
    let reference = match kind {
        Kind::Spawn => p.section.clone(),
        Kind::Shape => p.r#box.then(|| "box".to_owned()),
        _ => p.r#ref.clone(),
    };
    if kind.needs_reference() && reference.is_none() {
        let what = if kind == Kind::Spawn { "section" } else { "ref" };
        return Err(CmdError::invalid(format!("place={word} needs the {what} of what to place")));
    }
    Ok(Some(Some(What::Place(Placeable::new(kind, reference.as_deref())))))
}

async fn level_view(ed: Arc<Editor>, p: ViewParams, _o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let snap = svc.store.snapshot();
    if !snap.valid {
        return Err(CmdError::not_found("No scene is open."));
    }
    let pick = pick_of(&p)?;
    let category = match &p.category {
        Some(c) => Some(Category::parse(c).ok_or_else(|| {
            CmdError::invalid(format!(
                "There is no category {c}: recent, objects, lights, spawn_points, shapes, sounds, particles, way_paths or groups."
            ))
        })?),
        None => None,
    };
    if category.is_some() && p.open.is_none() {
        return Err(CmdError::invalid("category takes open=true or open=false"));
    }
    let hide = match &p.kinds {
        Some(k) => {
            let groups = compute(&snap, &Default::default(), &Default::default()).groups;
            Some(types_to_hide(&groups, k).map_err(CmdError::invalid)?)
        }
        None => None,
    };
    let limit = p.limit.unwrap_or(40).clamp(1, 1000);
    let changes = (p.changes_place(), p.changes_outliner());
    let ed2 = ed.clone();
    let snap_ui = snap.clone();
    let (job, filter, folds) = on_ui(&ed, move |shell, ctx| {
        // a person changing these panels has them in front of them
        if changes.0 || changes.1 || p.colour_indices == Some(true) {
            shell.activate(ctx, LEVEL);
        }
        with_level(shell, |doc| {
            if changes.0 {
                doc.open_panel(ctx, PanelId::Place);
            }
            if changes.1 {
                doc.open_panel(ctx, PanelId::Outliner);
            }
            let place = &mut doc.tools.place;
            if let Some(s) = &p.place_search {
                place.search = s.chars().take(127).collect();
            }
            if let (Some(c), Some(open)) = (category, p.open) {
                match &p.folder {
                    Some(f) => place.set_folder(c, f, open),
                    None => PlacePanel::set_open(ctx, c, open),
                }
            }
            match pick {
                Some(Some(w)) => place.pick(ctx, &ed2, w),
                Some(None) => place.disarm(&ed2),
                None => {}
            }
            let f = &mut doc.outliner.filter;
            if let Some(t) = &p.outliner_search {
                f.text = t.chars().take(127).collect();
            }
            if let Some(s) = p.selected_only {
                f.selected_only = s;
            }
            if let Some(v) = p.visibility {
                f.visibility = match v {
                    VisibilityParam::All => Visibility::All,
                    VisibilityParam::Visible => Visibility::Visible,
                    VisibilityParam::Hidden => Visibility::Hidden,
                };
            }
            if let Some(s) = p.sort {
                f.sort = match s {
                    SortParam::Scene => Sort::Scene,
                    SortParam::Az => Sort::Az,
                    SortParam::Za => Sort::Za,
                };
            }
            if let Some(l) = p.layout {
                f.hierarchy = matches!(l, LayoutParam::Hierarchy);
            }
            if let Some(h) = hide {
                f.hidden = h;
            }
            let mut unknown = Vec::new();
            if let Some(list) = &p.unfold {
                unknown.extend(doc.outliner.set_unfolded(&snap_ui, list, true));
            }
            if let Some(list) = &p.fold {
                unknown.extend(doc.outliner.set_unfolded(&snap_ui, list, false));
            }
            if !unknown.is_empty() {
                return Err(CmdError::not_found(format!(
                    "No kind or object is called {}.",
                    unknown.join(", ")
                )));
            }
            if let Some(open) = p.colour_indices {
                doc.detail_objects.open = open;
            }
            Ok((doc.tools.place.job(ctx), doc.outliner.filter.clone(), doc.outliner.folds.clone()))
        })
        .ok_or_else(|| CmdError::invalid("the Level tab is not there"))?
    })
    .await?;
    // the lists the panels read, and their rows as the next frame draws them
    let inputs = Inputs::wait(&ed).await;
    let rows = job.rows(inputs).await;
    let snap = svc.store.snapshot();
    let s = snap.clone();
    let listing = ed.exec.cpu(move || compute(&s, &filter, &folds)).await;
    // the Outliner as drawn: the frames in which it lists the scene as it is and has brought a
    // selection made elsewhere into view come first, then one more for the scroll to show
    if ed.frames.has_window() {
        let rev = snap.rev;
        let mut shown = false;
        for _ in 0..120 {
            ed.frames.after(1).await?;
            let settled: Option<bool> = on_ui(&ed, move |shell, _ctx| {
                Ok(with_level(shell, |doc| doc.outliner.settled(rev)).flatten())
            })
            .await?;
            match settled {
                // not on the screen: nothing to wait for
                None => break,
                Some(true) if shown => break,
                Some(now) => shown = now,
            }
        }
    }
    let snap2 = snap.clone();
    let ed3 = ed.clone();
    let rows2 = rows.clone();
    let out = on_ui(&ed, move |shell, _ctx| {
        with_level(shell, |doc| {
            let mut outliner = doc.outliner.view_state();
            if let Value::Object(m) = &mut outliner {
                m.insert("shown".into(), json!(listing.shown));
                m.insert("total".into(), json!(listing.total));
                m.insert("selected".into(), json!(listing.selected));
                m.insert("rows".into(), Value::Array(doc.outliner.rows_of(&snap2, &listing, limit)));
            }
            json!({
                "mode": snap2.tools.mode.word(),
                "panel": crate::tools::tab_label_of(snap2.tools.mode),
                "place": doc.tools.place.view_state(&ed3, &snap2, &rows2, limit),
                "paint": doc.tools.paint_state(&ed3, &snap2),
                "zones": crate::tools_objects::state(&snap2),
                "outliner": outliner,
                "colour_indices": doc.detail_objects.open,
            })
        })
        .ok_or_else(|| CmdError::invalid("the Level tab is not there"))
    })
    .await?;
    Ok(out)
}
