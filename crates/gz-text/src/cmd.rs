//! `script.lookup`: where a script's name is declared and what a prefix completes to.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{CmdError, Editor, Registry, Reply, Spec};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::index::{DeclKind, OfferKind};
use crate::service::TextService;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LookupParams {
    /// A name as a script writes it - `xr_logic.pick_section_from_condlist`, `xr_logic:name`, a
    /// global (`printf`) or a script (`xr_logic`) - or, with `complete`, what is typed so far
    /// (`xr_logic.pick`, `pri`).
    pub name: String,
    /// Answer what `name` completes to (`offers`) instead of where it is declared.
    #[serde(default)]
    pub complete: bool,
    /// How many offers to answer (default 50).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Registers the `script` area.
pub fn register(reg: &mut Registry) {
    reg.area(
        "script",
        "The scripts' declarations, the linked game's and the project's: where a name is declared and what a prefix completes to",
    );
    reg.cmd(
        Spec::read(
            "script.lookup",
            "Where a script's function, value, global or the script itself is declared (script, line, game or project), or with complete=true what a name typed so far completes to.",
        )
        .example("name=xr_logic.pick_section_from_condlist"),
        lookup,
    );
}

fn kind_word(kind: OfferKind) -> &'static str {
    match kind {
        OfferKind::Keyword => "keyword",
        OfferKind::Function => "function",
        OfferKind::Value => "value",
        OfferKind::Module => "script",
    }
}

async fn lookup(ed: Arc<Editor>, p: LookupParams, _o: Origin) -> Reply {
    let session = ed.project()?;
    let service = TextService::of_session(&ed, &session);
    let index = service.index(&ed).await;
    let name = p.name.trim();
    if p.complete {
        let limit = p.limit.unwrap_or(50).max(1);
        // what stands before the last dot or colon is the script the rest is a name of
        let (qualifier, prefix) = match name.rfind(['.', ':']) {
            Some(at) => (Some(name.get(..at).unwrap_or("")), name.get(at + 1..).unwrap_or("")),
            None => (None, name),
        };
        let offers: Vec<Value> = index
            .complete(qualifier, prefix, limit)
            .into_iter()
            .map(|o| json!({ "text": o.text, "detail": o.detail, "kind": kind_word(o.kind) }))
            .collect();
        return Ok(json!({ "offers": offers }));
    }
    if name.is_empty() {
        return Err(CmdError::invalid(
            "name is empty: give a name as a script writes it (xr_logic.pick_section_from_condlist)",
        ));
    }
    let declared: Vec<Value> = index
        .lookup(name)
        .into_iter()
        .map(|d| {
            json!({
                "script": d.script,
                "name": d.name,
                "kind": if d.kind == DeclKind::Function { "function" } else { "value" },
                "params": d.params,
                "line": d.line,
                "source": d.source.word(),
                "path": d.source.path(),
            })
        })
        .collect();
    Ok(json!({ "declared": declared, "scripts": index.scripts() }))
}
