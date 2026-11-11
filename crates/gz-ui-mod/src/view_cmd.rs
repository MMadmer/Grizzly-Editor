//! `mod_settings_view`: the Mod Settings tab for whoever looks at it (`EditorMod::McpSettingsView`).
//! A `page` opens the tab on that page, `ask_game_updates` puts the Disable Game Updates question
//! up on its next frame; without either this only reports. The data path stays `mod_manifest` /
//! `mod_set_manifest`.

use std::sync::Arc;

use gz_app::on_ui;
use gz_base::Origin;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Reply};

use crate::call::{args, ok, s};
use crate::form::{self, PAGES};
use crate::settings::{ModSettingsDoc, TAB, view_json};

pub(crate) async fn mod_settings_view(ed: Arc<Editor>, a: Args, origin: Origin) -> Reply {
    ed.project()?;
    crate::cards::ensure(&ed);
    let page = match a.text("page") {
        Some(id) => Some(form::page_index(id).ok_or_else(|| {
            let ids: Vec<&str> = PAGES.iter().map(|p| p.id).collect();
            CmdError::invalid(format!("page must be one of: {}", ids.join(", ")))
        })?),
        None => None,
    };
    let ask = a.flag("ask_game_updates");
    if page.is_some() || ask {
        let reply = ed.call("ui_tab", args([("action", s("open")), ("kind", s(TAB))]), origin).await;
        if !ok(&reply)
            && reply.get("error").and_then(serde_json::Value::as_str)
                != Some("the editor runs without a window")
        {
            return Err(CmdError::new("refused", crate::call::error_text(&reply)));
        }
    }
    let answer = on_ui(&ed, move |shell, _ctx| {
        let active = shell.active() == TAB;
        let doc = shell
            .document_mut(TAB)
            .and_then(|d| d.as_any_mut())
            .and_then(|any| any.downcast_mut::<ModSettingsDoc>());
        let Some(doc) = doc else { return Ok(view_json(false, false, form::GENERAL, false, &[])) };
        if let Some(page) = page {
            doc.set_page(page);
        }
        if ask {
            doc.ask_game_updates_later();
        }
        doc.refresh();
        Ok(doc.view_json(active))
    })
    .await;
    match answer {
        Ok(v) => Ok(v),
        // without a window there is no tab: it reports as closed
        Err(e) if e.code == "no_window" => {
            Ok(view_json(false, false, page.unwrap_or(form::GENERAL), false, &[]))
        }
        Err(e) => Err(e),
    }
}
