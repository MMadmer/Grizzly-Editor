//! The commands. The wire is frozen by the C++ handlers reached from `LevelMcpMod.cpp`:
//! `mod_publish` (`EditorPublish::McpPublish`) and `mod_releases` (`EditorPublish::McpReleases`).
//! `mod_publish start` answers once the packaging has ended - the upload then runs and `status`
//! follows it - while every other command goes on meanwhile. The MCP path asks nothing: a caller
//! that wants a permission asks for it on its own side. A token goes in through `save_token` and
//! never comes out.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Registry, Reply};
use gz_github::{Github, Secret};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::job::{MAX_NOTES, PublishOptions, Publisher, reply};
use crate::manage::{ManageOp, ManageRequest, ReleaseList};

const AREA: &str = "gz-publish";

pub fn register(reg: &mut Registry) {
    reg.add(AREA, "mod_publish", mod_publish);
    reg.add(AREA, "mod_releases", mod_releases);
}

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

/// A string argument the way `GetArg` read it: present means given, even empty.
fn arg(args: &Args, key: &str) -> Option<String> {
    match args.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
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

/// `"id"` as a bare number or a quoted one; 0 when absent.
fn arg_u64(args: &Args, key: &str) -> u64 {
    match args.get(key) {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(s)) => {
            let digits = s.trim_start().bytes().take_while(u8::is_ascii_digit).count();
            s.trim_start().get(..digits).and_then(|d| d.parse().ok()).unwrap_or(0)
        }
        _ => 0,
    }
}

/// The source alone: a token goes in through MCP (`save_token`) and never comes out.
async fn account_json(ed: &Arc<Editor>) -> Reply {
    let gh = Github::of(ed);
    let account = gh.resolve_account().await;
    Ok(json!({
        "signed_in": account.signed_in(),
        "source": account.source.text(),
        "device_sign_in": gh.endpoints().device_sign_in(),
    }))
}

async fn mod_publish(ed: Arc<Editor>, mut args: Args, _origin: Origin) -> Reply {
    let action = args.text("action").unwrap_or("start").to_owned();
    let publisher = Publisher::of(&ed);
    match action.as_str() {
        "status" => {
            let (ok, v) = publisher.status_json();
            return reply(ok, v);
        }
        "cancel" => {
            publisher.cancel(&ed);
            let (ok, v) = publisher.status_json();
            return reply(ok, v);
        }
        "account" => return account_json(&ed).await,
        "sign_in_status" => {
            let (ok, v) = publisher.sign_json();
            return reply(ok, v);
        }
        "sign_in" => {
            if !Github::of(&ed).endpoints().device_sign_in() {
                return Err(refuse(
                    "the device sign-in is not set up in this build: use a token, GH_TOKEN or the GitHub CLI",
                ));
            }
            // the caller hands the code over; no clipboard, no browser behind its back
            publisher.sign_in(&ed);
            let (ok, v) = publisher.sign_json();
            return reply(ok, v);
        }
        "save_token" => {
            // The one request that carries a token. It goes into the Credential Manager and
            // nowhere else: not into the answer, not into the log.
            let Some(token) = take_secret(&mut args, "token") else {
                return Err(refuse("save_token needs 'token'"));
            };
            Github::of(&ed).save_token(Secret::from(token)).await.map_err(refuse)?;
            return account_json(&ed).await;
        }
        "sign_out" => {
            publisher.sign_stop();
            Github::of(&ed).forget_token().await;
            return account_json(&ed).await;
        }
        "start" => {}
        _ => {
            return Err(refuse(
                "unknown action: start, status, cancel, account, save_token, sign_in, sign_in_status or sign_out expected",
            ));
        }
    }
    if args.flag("answer_only") {
        let (ok, v) = publisher.started_json();
        return reply(ok, v);
    }

    // the author key's password, when the key file has one; never echoed or logged
    if let Some(password) = take_secret(&mut args, "password").filter(|p| !p.is_empty()) {
        let keys = gz_release::AuthorKeys::of(&ed);
        ed.exec.io(move || keys.unlock(&password)).await.map_err(|e| refuse(format!("author key: {e}")))?;
    }
    let options = PublishOptions {
        tag: arg(&args, "tag").unwrap_or_default(),
        title: arg(&args, "title").unwrap_or_default(),
        notes: arg(&args, "notes").unwrap_or_default(),
        // absent = the linked game's version, present but empty = no requirement
        requires_game: arg(&args, "requires_game"),
        prerelease: args.bool("prerelease").unwrap_or(false),
        latest: args.bool("latest").unwrap_or(true),
    };
    let id = publisher.start(&ed, options, false).await.map_err(refuse)?;
    // held until the packaging ended: the upload is on its way, or the release was refused
    ed.jobs.wait(id).await;
    let (ok, v) = publisher.started_json();
    reply(ok, v)
}

async fn mod_releases(ed: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let action = args.text("action").unwrap_or("list").to_owned();
    let list = ReleaseList::of(&ed);
    let answer = |list: &ReleaseList| {
        let (ok, v) = list.json();
        reply(ok, v)
    };
    let op = match action.as_str() {
        "status" => return answer(&list),
        "cancel" => {
            list.cancel();
            return answer(&list);
        }
        "delete" => ManageOp::Delete,
        "edit" => ManageOp::Edit,
        "list" | "select" => ManageOp::List,
        _ => return Err(refuse("unknown action: list, edit, delete, select, status or cancel expected")),
    };
    let select = action == "select";

    // MCP is the programmatic path: the caller has decided, nothing is asked
    let mut rq = ManageRequest { op, ..ManageRequest::default() };
    if op != ManageOp::List || select {
        rq.tag = arg(&args, "tag").unwrap_or_default();
        rq.id = arg_u64(&args, "id");
        if rq.id == 0 && rq.tag.is_empty() {
            return Err(refuse("name the release: 'id' or 'tag'"));
        }
        if op == ManageOp::Edit {
            // a field that is absent keeps its value; an empty one clears it
            rq.title = arg(&args, "title");
            rq.notes = arg(&args, "notes");
            if rq.title.is_none() && rq.notes.is_none() {
                return Err(refuse("edit needs 'title' or 'notes'"));
            }
            if rq.title.as_ref().is_some_and(|t| t.len() > 200)
                || rq.notes.as_ref().is_some_and(|n| n.len() > MAX_NOTES)
            {
                return Err(refuse("the title takes 200 characters, the notes 120 000"));
            }
        }
    }
    // the release the tab's details are about, from the list as it was last read: nothing goes
    // to GitHub; show=edit puts the edit dialog up for that release, to be looked at
    if select {
        let show = arg(&args, "show").unwrap_or_default();
        if !show.is_empty() && show != "edit" {
            return Err(refuse("show: edit expected"));
        }
        list.select_release(rq.id, &rq.tag, show == "edit").map_err(refuse)?;
        return answer(&list);
    }
    list.start(&ed, rq).await.map_err(|(e, _)| refuse(e))?;
    answer(&list)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn ids_bare_or_quoted() {
        let mut a = Args::new();
        a.insert("id".to_owned(), json!(42));
        assert_eq!(arg_u64(&a, "id"), 42);
        a.insert("id".to_owned(), json!("17"));
        assert_eq!(arg_u64(&a, "id"), 17);
        a.insert("id".to_owned(), json!("x"));
        assert_eq!(arg_u64(&a, "id"), 0);
        assert_eq!(arg_u64(&Args::new(), "id"), 0);
    }

    #[test]
    fn failed_answers_keep_their_fields() {
        let e = reply(false, json!({ "state": "failed", "error": "why", "pending": false })).unwrap_err();
        assert_eq!(e.message, "why");
        assert_eq!(e.data, Some(json!({ "state": "failed", "pending": false })));
        assert_eq!(reply(true, json!({ "state": "done" })).ok(), Some(json!({ "state": "done" })));
    }
}
