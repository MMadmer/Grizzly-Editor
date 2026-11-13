//! The PDA area's command: `pda_use` (which layout the module uses). A layout's file is the
//! author's: it is created, read and changed with file tools and checked with `asset.check`;
//! `pda_preview` is the preview tab's (it forwards to [`crate::Preview`]).

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, CmdError, Editor, Registry, Reply};
use gz_project::Project;
use serde_json::{Value, json};

use crate::service::PdaService;

const AREA: &str = "gz-pda";

pub fn register(reg: &mut Registry) {
    reg.add(AREA, "pda_use", pda_use);
}

async fn pda_use(ed: Arc<Editor>, args: Args, origin: Origin) -> Reply {
    let (session, _) = Project::of(&ed)?;
    let service = PdaService::of_session(&ed, &session);
    let path = match args.get("path") {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    service.use_layout(&ed, &path, origin).await.map_err(|e| CmdError::new("refused", e))?;
    Ok(json!({ "used": service.used_layout(&ed).await.replace('\\', "/") }))
}
