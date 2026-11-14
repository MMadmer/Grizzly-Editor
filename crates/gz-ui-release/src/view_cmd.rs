//! `release.view`: the release windows for whoever looks at them - a release selected on the
//! Releases tab and its Edit Release dialog, the Submit to Mod Browser window and its withdrawal
//! question - to be looked at, never to act. What they show is `release.list` and
//! `release.catalog_status`.

use std::sync::Arc;

use gz_catalog::Catalog;
use gz_editor::Origin;
use gz_editor::{CmdError, Editor, Registry, Reply, Spec, json};
use gz_publish::ReleaseList;
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ViewParams {
    /// Selects a release on the Releases tab by its id or its tag, from the list as it was last
    /// read: nothing goes to GitHub.
    #[serde(default)]
    select: Option<String>,
    /// With select: puts that release's Edit Release dialog up.
    #[serde(default)]
    edit: bool,
    /// Puts the Submit to Mod Browser window up (true) or takes it down (false).
    #[serde(default)]
    submit: Option<bool>,
    /// Puts the withdrawal's question over the Submit window.
    #[serde(default)]
    withdraw_question: bool,
}

pub(crate) fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::view(
            "release.view",
            "Puts the release windows up to be looked at: a release and its edit dialog, the Submit window and its withdrawal question.",
        )
        .example("select=v1.2.0 edit=true"),
        release_view,
    );
}

async fn release_view(ed: Arc<Editor>, p: ViewParams, _origin: Origin) -> Reply {
    ed.project()?;
    let list = ReleaseList::of(&ed);
    if let Some(which) = p.select.as_deref().map(str::trim).filter(|w| !w.is_empty()) {
        let id = which.parse::<u64>().unwrap_or(0);
        let tag = if id == 0 { which } else { "" };
        list.select_release(id, tag, p.edit).map_err(|e| CmdError::new("refused", e))?;
    } else if p.edit {
        return Err(CmdError::invalid("edit needs select: the release whose dialog goes up"));
    }
    let catalog = Catalog::of(&ed);
    let mut want_submit = p.submit;
    if p.withdraw_question {
        want_submit = Some(true);
    }
    match want_submit {
        Some(true) if !catalog.window_open() => {
            let reply = ed
                .call("ui.dialog", gz_app::args(&[("action", "open"), ("id", crate::SUBMIT_ID)]), Origin::Mcp)
                .await;
            if reply.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
                let why = reply
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("the window has no Submit dialog");
                return Err(CmdError::new("refused", why.to_owned()));
            }
        }
        Some(false) => catalog.set_window(false),
        _ => {}
    }
    catalog.set_withdraw_question(p.withdraw_question);
    let (_, releases) = list.json();
    Ok(json!({
        "releases_tab": releases.get("tab"),
        "selected": list.selected(),
        "submit": catalog.window_open(),
        "withdraw_question": catalog.wants_withdraw_question(),
    }))
}
