//! What the characters area hangs on the editor: the template of a new character, the build's
//! check, the character kind of `gz-asset` (its positioned check and its reference), and the
//! roster made when a project opens. A character's file is the author's text: it is created,
//! read and changed with file tools and checked with `asset.check`.

use std::sync::Arc;

use futures::future::BoxFuture;
use gz_asset::{AssetFile, KindOps, Kinds};
use gz_base::Problem;
use gz_build::{BuildCheck, BuildChecks, BuildWalk};
use gz_editor::{Editor, ProjectSession};
use gz_luadata::Seg;
use gz_project::{AssetKind, AssetTemplates, ProjectHooks, TemplateKind};
use serde_json::{Value, json};

use crate::model::{self, Character, template, text_in};
use crate::service::{Characters, Checker};

/// The character format, every key with its type, its default and what the game does with it.
const CHARACTER_MD: &str = include_str!("../../../docs/assets/CHARACTER.md");

/// What the build says while the characters' check runs.
const STAGE: &str = "Checking the characters";

/// What the area hangs on the editor: the `file.create` template of a character, the
/// build's check, the kind of `gz-asset`, and the characters made when a project opens, so that
/// `gz_nq` knows them from the first validation on.
pub fn install(editor: &Editor) {
    AssetTemplates::of(editor).provide(TemplateKind::Character, |cx| Ok(template(cx.stem).into_bytes()));
    BuildChecks::of(editor).add(Arc::new(CharactersCheck));
    let kinds = Kinds::of(editor);
    kinds.add(kind());
    kinds.covers_build_stage(STAGE);
    ProjectHooks::of(editor).on_open(|ed, _session| {
        let ed = ed.clone();
        ed.exec.clone().spawn(async move {
            // a project without characters costs a look at its file tree
            let _ = Characters::of(&ed).await;
        });
    });
    // a character is a spawn section of the game: the scene places it like any other
    gz_game::OwnSections::of(editor).add(Arc::new(|ed| {
        Box::pin(async move {
            let Ok(svc) = Characters::of(ed).await else { return Vec::new() };
            let module = svc.manifest().snapshot().manifest().map(|m| m.id.clone()).unwrap_or_default();
            sections(&module, &svc.fresh_roster().await.resolved)
        })
    }));
}

/// The sections the game makes of a module's characters (the game's `xms_characters`): the
/// spawn section `<module>_<id>` over `stalker`, and its squad. A template is none.
#[must_use]
pub fn sections(module: &str, characters: &[Character]) -> Vec<gz_game::OwnSection> {
    let mut out = Vec::new();
    for c in characters.iter().filter(|c| !c.is_abstract) {
        let gid = crate::model::game_id(module, &c.id);
        let community = if c.community.is_empty() { "stalker" } else { c.community.as_str() };
        let mut lines = vec![
            ("$spawn".to_owned(), format!("\"npc\\{module}\\{}\"", c.id)),
            ("character_profile".to_owned(), gid.clone()),
            ("community".to_owned(), community.to_owned()),
        ];
        if !c.spec_rank.is_empty() {
            lines.push(("spec_rank".to_owned(), c.spec_rank.clone()));
        }
        // the look is the profile's in the game; the editor draws the model the character wears
        if !c.visual.is_empty() {
            lines.push(("visual".to_owned(), c.visual.clone()));
        }
        let origin = c.file.replace('\\', "/");
        out.push(gz_game::OwnSection {
            name: gid.clone(),
            parent: "stalker".to_owned(),
            lines,
            origin: origin.clone(),
        });
        if c.squad {
            out.push(gz_game::OwnSection {
                name: format!("{gid}_squad"),
                parent: "online_offline_group".to_owned(),
                lines: vec![("faction".to_owned(), community.to_owned()), ("npc".to_owned(), gid)],
                origin,
            });
        }
    }
    out
}

/// The character kind of `gz-asset`.
#[must_use]
pub fn kind() -> KindOps {
    let mut k = KindOps::of(AssetKind::Character);
    k.known = model::KNOWN;
    k.reference_doc = "docs/assets/CHARACTER.md";
    k.check = check;
    k.reference = |_| Box::pin(async { CHARACTER_MD.to_owned() });
    k.diff_rules = model::diff_rules;
    k
}

/// One character as the answers describe it.
#[must_use]
pub fn character_json(c: &Character) -> Value {
    json!({
        "id": c.id,
        "file": c.file.replace('\\', "/"),
        "name": text_in(&c.name, "rus"),
        "community": c.community,
        "visual": c.visual.replace('\\', "/"),
        "base": c.base,
        "abstract": c.is_abstract,
        "supplies": c.supplies.len(),
        "trader": c.trade.is_trader(),
        "squad": c.squad,
    })
}

/// The key of the file a validator's code is about.
fn field_of(code: &str) -> &'static [&'static str] {
    match code {
        "E101" | "E102" => &["id"],
        "E103" | "W107" => &["name"],
        "E104" | "W105" | "E123" => &["visual"],
        "W114" => &["community"],
        "W115" => &["icon"],
        "W116" => &["start_dialog"],
        "W117" | "E108" | "E109" => &["supplies"],
        "E110" => &["money"],
        "E118" | "W121" => &["trade", "stock"],
        "E119" | "W120" | "W122" => &["trade"],
        "E111" | "E112" => &["base"],
        _ => &[],
    }
}

/// A coded line of the validator (`E104 <file>: <message>`) as a problem at the key it is
/// about.
fn line_problem(line: &str, file: &AssetFile) -> Problem {
    let (code, rest) = line.split_once(' ').unwrap_or((line, ""));
    let message = rest.split_once(": ").map_or(rest, |(_, m)| m);
    let p =
        if code.starts_with('E') { Problem::error(code, message) } else { Problem::warning(code, message) };
    let path: Vec<Seg> = field_of(code).iter().map(|k| Seg::key(k)).collect();
    file.place(p, &path)
}

fn check(ed: Arc<Editor>, file: Arc<AssetFile>) -> BoxFuture<'static, Vec<Problem>> {
    Box::pin(async move {
        let wire = file.path.as_str().to_owned();
        let mut c = match model::from_value(&file.doc.value()) {
            Ok(c) => c,
            Err(e) => return vec![file.place_head(Problem::error("E100", e))],
        };
        c.file.clone_from(&wire);
        if c.id.is_empty() {
            c.id = model::id_from_name(model::base_name(&wire));
        }
        let Ok(svc) = Characters::of(&ed).await else { return Vec::new() };
        let roster = svc.fresh_roster().await;
        // what the character RESOLVES to: its base chain walked, the ids the others take
        // the roster's problems that name this file (an id taken twice names both)
        let mut lines: Vec<String> = roster
            .problems
            .iter()
            .filter(|p| {
                p.split_whitespace().any(|w| w.trim_end_matches([':', ',', '.']).eq_ignore_ascii_case(&wire))
            })
            .cloned()
            .collect();
        if let Some(r) = roster.get(&c.id).filter(|r| r.file.eq_ignore_ascii_case(&wire)) {
            c = r.clone();
        }
        if !c.is_abstract {
            let checker = svc.checker(&ed).await;
            let checked = ed
                .exec
                .io(move || {
                    let mut out = Vec::new();
                    checker.validate(&c, &mut out);
                    out
                })
                .await;
            lines.extend(checked);
        }
        lines.iter().map(|l| line_problem(l, &file)).collect()
    })
}

/// A coded line as a problem: its code, the rest its message.
fn as_problem(line: &str) -> Problem {
    let (code, message) = line.split_once(' ').unwrap_or((line, ""));
    if code.starts_with('E') { Problem::error(code, message) } else { Problem::warning(code, message) }
}

/// The build makes nothing of a character - the game reads the assets as they are; what is
/// left is saying no to a module whose characters the game could not use. A character that
/// does not parse stops the build: half a roster ships quests that point at NPCs that do not
/// exist. The refusal is the first error.
struct CharactersCheck;

impl BuildCheck for CharactersCheck {
    fn stage(&self) -> &'static str {
        STAGE
    }

    fn check(
        &self,
        editor: Arc<Editor>,
        _session: Arc<ProjectSession>,
        _walk: Arc<BuildWalk>,
    ) -> BoxFuture<'static, Vec<Problem>> {
        Box::pin(async move {
            // the roster reads only the files the build ships (`outside_build`)
            let Ok(svc) = Characters::of(&editor).await else { return Vec::new() };
            let roster = svc.fresh_roster().await;
            let all = roster.clone();
            let mut problems = roster.problems.clone();
            problems.extend(
                editor
                    .exec
                    .io(move || {
                        let checker = Checker::unlinked();
                        let shipped: Vec<_> =
                            all.resolved.iter().filter(|c| !c.is_abstract).cloned().collect();
                        checker.validate_all(&shipped)
                    })
                    .await,
            );
            problems.iter().find(|p| p.starts_with('E')).map(|p| vec![as_problem(p)]).unwrap_or_default()
        })
    }
}
