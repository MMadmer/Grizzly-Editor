//! The project file (`<Name>.grizzly`) as a kind of `gz-asset`: what `asset.check` finds in it -
//! an id that is no UUID, a scene to open that is not there, a library reference that resolves
//! nowhere, a second project file beside it - and its reference (`docs/assets/PROJECT.md`).

use std::path::Path;
use std::sync::Arc;

use futures::future::BoxFuture;
use gz_asset::{AssetFile, KindOps, at_path};
use gz_base::Problem;
use gz_editor::Editor;
use gz_luadata::{DiffRules, Seg, Value};
use gz_project::AssetKind;
use gz_project::library::{Resolves, resolve_object};

const PROJECT_MD: &str = include_str!("../../../docs/assets/PROJECT.md");
const TOP: &[&str] = &["grizzly", "id", "scene", "library"];
const KNOWN: &[(&str, &[&str])] = &[("", TOP)];

/// The kind.
#[must_use]
pub fn kind() -> KindOps {
    let mut k = KindOps::of(AssetKind::Project);
    // the editor alone reads a project file: the game knows modules, not projects
    k.limits.game = false;
    k.known = KNOWN;
    k.reference_doc = "docs/assets/PROJECT.md";
    k.check = check;
    k.reference = |_| Box::pin(async { PROJECT_MD.to_owned() });
    k.diff_rules = || DiffRules::new().order("", TOP);
    k
}

/// The problems of a project file's values, given the project's folder and the library's.
/// Blocking: a few attribute reads.
#[must_use]
pub fn problems_of(value: &Value, root: &Path, library: &Path, own_name: &str) -> Vec<(Problem, Vec<Seg>)> {
    let mut out = Vec::new();
    let id = value.get("id").and_then(Value::as_str).unwrap_or_default();
    if id.is_empty() {
        out.push((
            Problem::error(
                "E401",
                "The project file has no id: the editor gives it one when it opens the project.",
            ),
            vec![],
        ));
    } else if !gz_project::project::is_uuid(id) {
        out.push((
            Problem::error("E401", format!("The project's id '{id}' is no UUID: write one, or delete the line and the editor makes one.")),
            vec![Seg::key("id")],
        ));
    }
    if let Some(scene) = value.get("scene").and_then(Value::as_str).filter(|s| !s.is_empty())
        && std::fs::symlink_metadata(root.join(scene.replace('/', "\\"))).is_err()
    {
        out.push((
            Problem::warning("W402", format!("The scene '{scene}' the project opens with does not exist: the project opens without a scene.")),
            vec![Seg::key("scene")],
        ));
    }
    for (i, r) in value.get("library").map(Value::arr).unwrap_or_default().iter().enumerate() {
        let Some(r) = r.as_str() else { continue };
        if resolve_object(root, library, r) == Resolves::Nowhere {
            out.push((
                Problem::warning(
                    "W403",
                    format!("{r} is in no library: copy {r}.object into rawdata\\objects of the project, or set the library that holds it (Preferences > Library)."),
                ),
                vec![Seg::key("library"), Seg::Index(i)],
            ));
        }
    }
    let others: Vec<String> = gz_project::project::project_files(root)
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .filter(|n| !n.eq_ignore_ascii_case(own_name))
        .collect();
    if !others.is_empty() {
        out.push((
            Problem::error(
                "E404",
                format!(
                    "The folder holds another project file ({}): a project has exactly one.",
                    others.join(", ")
                ),
            ),
            vec![],
        ));
    }
    out
}

fn check(editor: Arc<Editor>, file: Arc<AssetFile>) -> BoxFuture<'static, Vec<Problem>> {
    Box::pin(async move {
        let Some(root) = editor.project_root() else { return Vec::new() };
        let library = gz_project::library_root(&editor);
        let value = file.doc.value();
        let own = file.path.file_name().to_owned();
        let f = file.clone();
        editor
            .exec
            .io(move || {
                problems_of(&value, &root, &library, &own)
                    .into_iter()
                    .map(|(p, path)| at_path(p, &f.path, &f.doc, &path))
                    .collect()
            })
            .await
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_wrong_with_a_project_file() {
        let t = tempfile::tempdir().unwrap();
        let (root, library) = (t.path().join("p"), t.path().join("lib"));
        std::fs::create_dir_all(library.join("rawdata/objects/props")).unwrap();
        std::fs::write(library.join("rawdata/objects/props/box.object"), "x").unwrap();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("P.grizzly"), "x").unwrap();
        let good = gz_luadata::parse("return { grizzly = 1, id = \"7c1e6c1e-5c0f-4f6e-9f43-2d5b1c9a0e11\", library = { \"props\\\\box\" } }").unwrap();
        assert!(problems_of(&good, &root, &library, "P.grizzly").is_empty());
        let bad = gz_luadata::parse(
            "return { grizzly = 1, id = \"nope\", scene = \"levels/gone.scene\", library = { \"props\\\\box\", \"props\\\\gone\" } }",
        )
        .unwrap();
        std::fs::write(root.join("Q.grizzly"), "x").unwrap();
        let codes: Vec<String> = problems_of(&bad, &root, &library, "P.grizzly")
            .into_iter()
            .map(|(p, _)| p.code.to_string())
            .collect();
        assert_eq!(codes, ["E401", "W402", "W403", "E404"]);
    }
}
