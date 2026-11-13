//! PDA interfaces as a kind of `gz-asset`: what `asset.check` and `asset.reference` ask of
//! them. A layout's text is the author's and is written as sent; what it means is the game's
//! reader script, which the check runs.

use std::sync::Arc;

use futures::future::BoxFuture;
use gz_asset::{AssetFile, KindOps};
use gz_base::Problem;
use gz_editor::Editor;
use gz_luadata::Seg;
use gz_mod::ManifestService;
use gz_project::{AssetKind, Project};

use crate::service::PdaService;

/// The format reference.
const PDA_FORMAT: &str = include_str!("../../../docs/pda/PDA_FORMAT.md");

/// What the build says while the PDA's check runs.
pub(crate) const STAGE: &str = "Checking the PDA";

/// The PDA kind of `gz-asset`.
#[must_use]
pub fn kind() -> KindOps {
    let mut k = KindOps::of(AssetKind::Pda);
    k.reference_doc = "docs/pda/PDA_FORMAT.md";
    k.check = check;
    k.reference = |_| Box::pin(async { PDA_FORMAT.to_owned() });
    k
}

/// A path of the reader (`pages[2].widgets[1].action`, 1-based like Lua) as a document path.
#[must_use]
pub fn reader_path(path: &str) -> Vec<Seg> {
    let mut out = Vec::new();
    for part in path.split('.').filter(|p| !p.is_empty()) {
        let (name, rest) = part.split_once('[').map_or((part, ""), |(n, r)| (n, r));
        if !name.is_empty() {
            out.push(Seg::key(name));
        }
        for idx in rest.split('[').map(|r| r.trim_end_matches(']')) {
            if let Ok(i) = idx.parse::<usize>() {
                out.push(Seg::Index(i.saturating_sub(1)));
            }
        }
    }
    out
}

/// The texture a W320 names, as the document writes it: where it stands.
fn texture_path(doc: &gz_luadata::LuaDoc, name: &str) -> Vec<Seg> {
    fn find(v: &gz_luadata::Value, name: &str, at: &mut Vec<Seg>) -> bool {
        match v {
            gz_luadata::Value::String(s) => s.eq_ignore_ascii_case(name),
            gz_luadata::Value::Table(t) => {
                for (i, x) in t.arr.iter().enumerate() {
                    at.push(Seg::Index(i));
                    if find(x, name, at) {
                        return true;
                    }
                    at.pop();
                }
                for (k, x) in &t.fields {
                    at.push(Seg::Key(k.clone()));
                    if find(x, name, at) {
                        return true;
                    }
                    at.pop();
                }
                false
            }
            _ => false,
        }
    }
    let mut at = Vec::new();
    if find(&doc.value(), name, &mut at) { at } else { Vec::new() }
}

fn check(ed: Arc<Editor>, file: Arc<AssetFile>) -> BoxFuture<'static, Vec<Problem>> {
    Box::pin(async move {
        let Ok((session, _)) = Project::of(&ed) else { return Vec::new() };
        let service = PdaService::of_session(&ed, &session);
        let name = crate::read::base_name(file.path.as_str());
        let layout = match service.evaluate(&ed, Arc::from(file.doc.text()), &name, "rus").await {
            Ok(l) => l,
            Err(e) => {
                let p = Problem::error("E300", e.message);
                return vec![if e.line > 0 {
                    p.in_file(file.path.clone()).at(e.line, 1, e.line, 2)
                } else {
                    file.place_head(p)
                }];
            }
        };
        let mut out: Vec<Problem> = layout
            .problems
            .iter()
            // the header is the read path's (gz-asset): a version it reads is the game's too
            .filter(|p| p.code != "E301")
            .map(|p| {
                let base = if p.is_error() {
                    Problem::error(p.code.as_str(), p.message.as_str())
                } else {
                    Problem::warning(p.code.as_str(), p.message.as_str())
                };
                file.place(base, &reader_path(&p.path))
            })
            .collect();
        for t in service.missing_textures(&ed, &session, &layout).await {
            let at = texture_path(file.place_doc(), &t);
            out.push(file.place(
                Problem::warning(
                    "W320",
                    format!("texture '{t}' is neither in the linked game nor in the project"),
                ),
                &at,
            ));
        }
        if let Ok(m) = ManifestService::of_session(&ed, &session).await {
            let used = m.snapshot().manifest().map(|x| x.pda_layout.clone()).unwrap_or_default();
            if crate::service::same_path(&used, file.path.as_str()) && m.outside_build(file.path.as_str()) {
                out.push(file.place_head(Problem::error(
                    "pda",
                    "the module uses this layout, but the build leaves the file out",
                )));
            }
        }
        out
    })
}

/// What the manifest says about the PDA: `[pda] layout` naming a file that is not there.
pub(crate) fn manifest_check(ed: Arc<Editor>) -> BoxFuture<'static, Vec<Problem>> {
    Box::pin(async move {
        let Ok((session, _)) = Project::of(&ed) else { return Vec::new() };
        let Ok(m) = ManifestService::of_session(&ed, &session).await else { return Vec::new() };
        let _ = m.sync().await;
        let used = m.snapshot().manifest().map(|x| x.pda_layout.clone()).unwrap_or_default();
        if used.is_empty() {
            return Vec::new();
        }
        let abs = session.root.join(used.replace('/', "\\"));
        if ed.exec.io(move || abs.is_file()).await {
            return Vec::new();
        }
        vec![
            Problem::error("pda", format!("[pda] layout = {used}: the file does not exist"))
                .in_field("pda.layout"),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_luadata::path::show;

    #[test]
    fn reader_paths_are_document_paths() {
        assert_eq!(show(&reader_path("pages[2].widgets[1].action")), "pages[1].widgets[0].action");
        assert_eq!(show(&reader_path("map_spots.player.size")), "map_spots.player.size");
        assert_eq!(show(&reader_path("")), "");
    }
}
