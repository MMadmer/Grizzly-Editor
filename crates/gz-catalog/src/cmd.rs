//! `mod_catalog`, frozen by the C++ handler (`EditorCatalogSubmit::McpCatalog`). Registered after
//! `gz-release`'s stand-in, which it replaces; the author key's actions are still `gz-release`'s
//! ([`gz_release::key_action`]). Like every MCP path it asks nothing and opens no window; its
//! secrets (a key password, a VirusTotal key) go in and never come out.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Registry, Reply};
use gz_github::{Github, Secret};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::service::{Catalog, Job};

const AREA: &str = "gz-catalog";

pub fn register(reg: &mut Registry) {
    reg.add(AREA, "mod_catalog", mod_catalog);
}

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// A secret argument taken out of the request, so that the only copy is wiped when it goes.
fn take_secret(args: &mut Args, key: &str) -> Option<Zeroizing<String>> {
    match args.remove(key)? {
        Value::String(s) => Some(Zeroizing::new(s)),
        Value::Number(n) => Some(Zeroizing::new(n.to_string())),
        Value::Bool(b) => Some(Zeroizing::new(b.to_string())),
        _ => None,
    }
}

async fn mod_catalog(ed: Arc<Editor>, mut args: Args, _origin: Origin) -> Reply {
    if ed.project_opt().is_none() {
        return Err(refuse("no active project"));
    }
    let action = args.text("action").unwrap_or("status").to_owned();
    let catalog = Catalog::of(&ed);
    match action.as_str() {
        "status" => {}
        "key_create" | "key_use" | "key_unlock" => {
            if let Some(done) = gz_release::key_action(&ed, &action, &mut args).await {
                done?;
            }
        }
        "vt_key" => {
            // the one request that carries a VirusTotal key: it goes into the Credential Manager
            let Some(key) = take_secret(&mut args, "key") else { return Err(refuse("vt_key needs 'key'")) };
            Github::of(&ed).save_vt_key(Secret::from(key)).await.map_err(refuse)?;
        }
        "vt_key_forget" => Github::of(&ed).forget_vt_key().await,
        "cancel" => catalog.cancel(&ed),
        // the window, for whoever has to look at it from outside: a screenshot, a person.
        // confirm=withdraw puts the withdrawal's question over it - only a click in it withdraws
        "show" => {
            if !catalog.window_open() {
                catalog.request(&ed).await;
            }
            let confirm = args.str("confirm").unwrap_or("");
            if !confirm.is_empty() && confirm != "withdraw" {
                return Err(refuse("confirm: withdraw expected"));
            }
            catalog.set_withdraw_question(confirm == "withdraw");
        }
        "hide" => catalog.set_window(false),
        _ => {
            // withdraw asks nothing here: the caller decided, and the bridge marks the call destructive
            let job = match action.as_str() {
                "check" => Job::Check,
                "vt" => Job::Vt,
                "submit" => Job::Submit,
                "submission" => Job::Submission,
                "withdraw" => Job::Withdraw,
                _ => {
                    return Err(refuse(
                        "unknown action: status, key_create, key_use, key_unlock, vt_key, vt_key_forget, check, vt, submit, submission, withdraw, cancel, show or hide expected",
                    ));
                }
            };
            catalog.start(&ed, job, false).await.map_err(refuse)?;
        }
    }
    Ok(catalog.status_json(&ed).await)
}
