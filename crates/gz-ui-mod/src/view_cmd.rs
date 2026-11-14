//! `mod.view`: the Mod Settings tab, and the dialogs of the builds and of the packaging, for
//! whoever looks at them. A `page` opens the tab on that page, `ask_game_updates` puts the Disable
//! Game Updates question up, `dialog` puts a build's or the packaging's dialog up the way the Mod
//! menu does, `show` puts one of the tab's pick lists up; without any of them this only reports. The data is `mod.info` / `mod.set`.

use std::sync::Arc;

use gz_app::on_ui;
use gz_base::Origin;
use gz_editor::{CmdError, Editor, Registry, Reply, Spec};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::call::{args, ok, s};
use crate::form::{self, PAGES};
use crate::settings::{ModSettingsDoc, RELATION_SHOWS, SHOW_FUNCTION, TAB, show_error, view_json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Dialog {
    /// Build to Folder.
    BuildToFolder,
    /// Export Flat Gamedata.
    ExportFlat,
    /// Package Release.
    PackageRelease,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ViewParams {
    /// Opens the tab on a page: general, mod_browser, game_mode, relations, build_contents, pda,
    /// commands.
    #[serde(default)]
    page: Option<String>,
    /// Puts the Disable Game Updates question up on the General page, to be looked at.
    #[serde(default)]
    ask_game_updates: bool,
    /// Puts a build's or the packaging's dialog up, as the Mod menu does.
    #[serde(default)]
    dialog: Option<Dialog>,
    /// Puts a list of Mod Settings up to be looked at: add_requires, add_load_after, add_load_before,
    /// add_conflicts (the Add Module lists of the Relations page) or function (the Function list
    /// of the first command; the module needs a command). `ui.audit` lists the rows it offers.
    #[serde(default)]
    show: Option<String>,
}

pub(crate) fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::view("mod.view", "Puts Mod Settings up on a page, or a build's or the packaging's dialog, and answers what it shows.")
            .example("page=commands"),
        mod_view,
    );
}

async fn mod_view(ed: Arc<Editor>, p: ViewParams, origin: Origin) -> Reply {
    ed.project()?;
    crate::cards::ensure(&ed);
    if let Some(dialog) = p.dialog {
        on_ui(&ed, move |shell, ctx| {
            shell.with_cx(ctx, |cx| match dialog {
                Dialog::BuildToFolder => crate::build::request_export(cx, false),
                Dialog::ExportFlat => crate::build::request_export(cx, true),
                Dialog::PackageRelease => crate::package::request(cx),
            });
            Ok(())
        })
        .await?;
        return Ok(serde_json::json!({ "dialog": match dialog {
            Dialog::BuildToFolder => "build_to_folder",
            Dialog::ExportFlat => "export_flat",
            Dialog::PackageRelease => "package_release",
        } }));
    }
    let page = match p.page.as_deref() {
        Some(id) => Some(form::page_index(id).ok_or_else(|| {
            let ids: Vec<&str> = PAGES.iter().map(|p| p.id).collect();
            CmdError::invalid(format!("page must be one of: {}", ids.join(", ")))
        })?),
        None => None,
    };
    let ask = p.ask_game_updates;
    let show = p.show.filter(|v| !v.is_empty());
    if let Some(name) = &show
        && name != SHOW_FUNCTION
        && !RELATION_SHOWS.contains(&name.as_str())
    {
        return Err(CmdError::invalid(show_error()));
    }
    if page.is_some() || ask || show.is_some() {
        let reply = ed.call("ui.tab", args([("action", s("open")), ("kind", s(TAB))]), origin).await;
        let windowless = reply.get("code").and_then(serde_json::Value::as_str) == Some("no_window")
            || reply.get("error").and_then(serde_json::Value::as_str)
                == Some("the editor runs without a window");
        if !ok(&reply) && !windowless {
            return Err(CmdError::new("refused", crate::call::error_text(&reply)));
        }
    }
    let answer = on_ui(&ed, move |shell, ctx| {
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
        if let Some(name) = &show {
            doc.show_list(ctx, name).map_err(|e| CmdError::new("refused", e))?;
        }
        doc.refresh();
        ctx.request_repaint();
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
