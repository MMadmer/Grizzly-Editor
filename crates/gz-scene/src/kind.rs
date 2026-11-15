//! The scene and the object group as kinds of `gz-asset`: what `asset.check`, `asset.reference`
//! and `asset.kinds` ask of them.

use std::sync::Arc;

use futures::future::BoxFuture;
use gz_asset::{AssetFile, KindOps};
use gz_base::Problem;
use gz_editor::Editor;
use gz_luadata::Limits;

use crate::format::{self, group};

/// The keys of a `.scene` itself (its objects are in the files of its folder).
const SCENE_KEYS: &[&str] = &[
    format::HEADER,
    "level",
    "build",
    "sun",
    "light_controls",
    "camera",
    "classes",
    "view_flags",
    "random_placement",
    "layers",
];

/// The keys of a `.group`: its header and the lists of objects.
const GROUP_KEYS: &[&str] = &[
    group::HEADER,
    "objects",
    "lights",
    "glows",
    "sources",
    "environments",
    "particles",
    "shapes",
    "volumes",
    "spawns",
    "ways",
    "sectors",
    "portals",
    "groups",
];

/// The scene kind.
#[must_use]
pub fn scene() -> KindOps {
    KindOps {
        name: "scene",
        ext: ".scene",
        header: format::HEADER,
        version: format::VERSION,
        limits: Limits::scene(),
        migrations: format::MIGRATIONS,
        known: &[("", SCENE_KEYS)],
        reference_doc: "docs/assets/SCENE.md",
        check: check_scene,
        reference: |_| Box::pin(async { format::doc::reference() }),
        diff_rules: format::scene_rules,
        take_theirs: None,
    }
}

/// The object group kind.
#[must_use]
pub fn group() -> KindOps {
    KindOps {
        name: "group",
        ext: ".group",
        header: group::HEADER,
        version: group::VERSION,
        limits: Limits::scene(),
        migrations: group::MIGRATIONS,
        known: &[("", GROUP_KEYS)],
        reference_doc: "docs/assets/SCENE.md",
        check: check_group,
        reference: |_| Box::pin(async { format::doc::reference() }),
        diff_rules: gz_luadata::DiffRules::new,
        take_theirs: None,
    }
}

/// The problems of a scene: what reading it with its folder's files finds.
fn check_scene(ed: Arc<Editor>, file: Arc<AssetFile>) -> BoxFuture<'static, Vec<Problem>> {
    Box::pin(async move {
        let settings = crate::game::settings(&ed).await;
        let abs = file.abs.clone();
        let main = file.path.clone();
        let lib = gz_project::library_root(&ed);
        ed.exec
            .io(move || {
                let texts = match crate::service::read_texts(&abs) {
                    Ok(t) => t,
                    Err(e) => {
                        return vec![
                            Problem::error("io", format!("the scene could not be read: {e}")).in_file(main),
                        ];
                    }
                };
                // the folder beside the file, named as the scene
                let stem = main.as_str().rsplit_once('.').map_or(main.as_str(), |(s, _)| s);
                let folder = gz_base::RelPath::new(stem).unwrap_or_else(|_| main.clone());
                let st = settings.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync));
                match format::read(&texts, &main, &folder, st) {
                    Ok(mut read) => {
                        // the groups placed at the read: a group asset that cannot be read is an error
                        let depth = main.as_str().split('/').count();
                        if let Some(root) = abs.ancestors().nth(depth) {
                            let (notes, _) = crate::edit::place_unplaced(
                                &mut read.state,
                                &read.unplaced,
                                root,
                                Some(&lib),
                                st,
                            );
                            read.problems.extend(notes);
                        }
                        read.problems
                    }
                    Err(r) => vec![*r.problem],
                }
            })
            .await
    })
}

/// The problems of an object group.
fn check_group(ed: Arc<Editor>, file: Arc<AssetFile>) -> BoxFuture<'static, Vec<Problem>> {
    Box::pin(async move {
        let settings = crate::game::settings(&ed).await;
        let text = file.doc.text().to_owned();
        let rel = file.path.clone();
        ed.exec
            .cpu(move || {
                match group::read_group(
                    &text,
                    &rel,
                    settings.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync)),
                ) {
                    Ok((_, problems)) => problems,
                    Err(r) => vec![*r.problem],
                }
            })
            .await
    })
}
