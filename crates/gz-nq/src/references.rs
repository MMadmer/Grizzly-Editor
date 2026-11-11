//! Catalog-typed reference discovery (`NqReferences`): a value is a reference only when the
//! catalog declares its exact type - parameter names are never guessed. The walk reports where
//! it had to give up (`blocking` diagnostics), so a caller knows whether the answer is complete;
//! the task rename refuses on exactly those.

use gz_luadata::Value;
use serde_json::{Value as Json, json};

use crate::asset::Graph;
use crate::catalog::{Catalog, Kind, Param, USE_COND, USE_EXTRA, USE_MAIN, USE_TRIGGER};
use crate::index::IndexSnapshot;
use crate::text::valid_id;

/// One reference found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reference {
    /// Project-relative, forward slashes.
    pub path: String,
    pub node: String,
    pub slot: String,
    pub kind: String,
    pub param: String,
    pub ty: String,
    pub value: String,
    /// `disk` or `memory`.
    pub source: String,
}

/// Something the walk could not prove (`SDiagnostic`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagnostic {
    pub path: String,
    pub node: String,
    pub slot: String,
    pub kind: String,
    pub param: String,
    pub code: String,
    pub message: String,
    pub source: String,
    /// The walk gave up on a subtree: a reference may be hiding in it.
    pub blocking: bool,
}

impl Diagnostic {
    /// `path:node[slot] code: message`.
    #[must_use]
    pub fn text(&self) -> String {
        let mut wh = self.path.clone();
        if !self.node.is_empty() {
            wh.push(':');
            wh.push_str(&self.node);
        }
        if !self.slot.is_empty() {
            wh.push('[');
            wh.push_str(&self.slot);
            wh.push(']');
        }
        format!("{wh} {}: {}", self.code, self.message)
    }

    #[must_use]
    pub fn to_json(&self) -> Json {
        json!({
            "path": self.path, "node": self.node, "slot": self.slot, "kind": self.kind, "param": self.param,
            "code": self.code, "message": self.message, "source": self.source,
        })
    }
}

impl Reference {
    #[must_use]
    pub fn to_json(&self) -> Json {
        json!({ "path": self.path, "node": self.node, "slot": self.slot, "kind": self.kind, "param": self.param, "source": self.source })
    }
}

/// What a search found (`SResult`).
#[derive(Clone, Debug, Default)]
pub struct FindResult {
    pub references: Vec<Reference>,
    pub diagnostics: Vec<Diagnostic>,
    /// Every reference was seen: no blocking diagnostic.
    pub complete: bool,
    pub generation: u32,
    pub fingerprint: u64,
    pub catalog_generation: u32,
}

const KNOWN_TYPES: &[&str] = &[
    "string",
    "int",
    "float",
    "bool",
    "enum",
    "text",
    "duration",
    "npc_ref",
    "target_ref",
    "place",
    "spawn_spec",
    "item_section",
    "squad_section",
    "level",
    "smart",
    "story_id",
    "profile",
    "community",
    "info",
    "var_name",
    "task_id",
    "quest_id",
    "ref_name",
    "signal_name",
    "spot_type",
    "relation",
    "lua",
    "cases",
    "cases_cond",
    "cases_weight",
    "kill_target",
    "count_or_all",
    "value",
    "cond_list",
    "node_id",
    "restrictor",
    "squad_ref",
    "object_ref",
    "objective_id",
    "character_id",
    "global_name",
];

/// Whether references of a type can be looked for.
#[must_use]
pub fn is_known_type(t: &str) -> bool {
    KNOWN_TYPES.contains(&t)
}

fn is_string_type(t: &str) -> bool {
    matches!(
        t,
        "string"
            | "enum"
            | "item_section"
            | "squad_section"
            | "level"
            | "smart"
            | "story_id"
            | "profile"
            | "character_id"
            | "global_name"
            | "community"
            | "info"
            | "var_name"
            | "task_id"
            | "quest_id"
            | "ref_name"
            | "signal_name"
            | "spot_type"
            | "relation"
            | "cases"
            | "node_id"
    )
}

fn is_integer(v: f64) -> bool {
    v == v.floor() && v.abs() < 1e15
}

struct Walker<'a> {
    cat: &'a Catalog,
    path: String,
    source: &'static str,
    ty: String,
    value: String,
    /// The new value of every reference met, when renaming.
    rename: Option<String>,
    references: Vec<Reference>,
    diagnostics: Vec<Diagnostic>,
    edits: usize,
}

impl Walker<'_> {
    #[allow(clippy::too_many_arguments)]
    fn diag(
        &mut self,
        code: &str,
        message: String,
        node: &str,
        slot: &str,
        kind: &str,
        param: &str,
        blocking: bool,
    ) {
        self.diagnostics.push(Diagnostic {
            path: self.path.clone(),
            node: node.to_owned(),
            slot: slot.to_owned(),
            kind: kind.to_owned(),
            param: param.to_owned(),
            code: code.to_owned(),
            message,
            source: self.source.to_owned(),
            blocking,
        });
    }

    fn graph(&mut self, q: &mut Graph) {
        // task targets have a fixed schema rather than a catalog kind
        let task_kind = Kind { id: "quest.task".into(), ..Kind::default() };
        let target = Param { name: "target".into(), ty: "target_ref".into(), ..Param::default() };
        for t in q.tasks.iter_mut() {
            if t.target.is_nil() {
                continue;
            }
            let slot = format!("task:{}/target", t.id);
            self.npc_ref(&task_kind, &target, &mut t.target, "", &slot, true, false);
        }
        for n in q.nodes.iter_mut() {
            let id = n.id.to_string();
            self.walk_kind(&n.kind.clone(), &mut n.params, &id, "params", USE_TRIGGER | USE_MAIN);
            for (c, cond) in n.cond.iter_mut().enumerate() {
                let kind = cond.kind.clone();
                self.walk_kind(&kind, &mut cond.params, &id, &format!("cond:{c}"), USE_COND);
            }
            for (prefix, list) in [("enter", &mut n.on_enter), ("exit", &mut n.on_exit)] {
                for (i, a) in list.iter_mut().enumerate() {
                    let kind = a.kind.clone();
                    self.walk_kind(&kind, &mut a.params, &id, &format!("{prefix}:{i}"), USE_EXTRA);
                }
            }
        }
    }

    fn walk_kind(&mut self, kind_id: &str, params: &mut Value, node: &str, slot: &str, expected: u8) {
        let cat = self.cat;
        let Some(kind) = cat.find(kind_id) else {
            self.diag(
                "unknown_kind",
                format!("kind '{kind_id}' is absent from the active catalog"),
                node,
                slot,
                kind_id,
                "",
                true,
            );
            return;
        };
        if kind.uses & expected == 0 {
            self.diag(
                "wrong_kind_use",
                format!("kind '{kind_id}' is not valid in this slot"),
                node,
                slot,
                kind_id,
                "",
                true,
            );
            return;
        }
        if !params.is_nil() && !params.is_table() {
            self.diag("malformed", "params must be a table".into(), node, slot, kind_id, "", true);
            return;
        }
        if !params.arr().is_empty() {
            self.diag(
                "malformed",
                "params must not contain array entries".into(),
                node,
                slot,
                kind_id,
                "",
                false,
            );
        }
        let unknown: Vec<String> = params
            .fields()
            .iter()
            .filter(|(k, _)| kind.param(k).is_none())
            .map(|(k, _)| k.to_string())
            .collect();
        for k in unknown {
            self.diag(
                "unknown_param",
                format!("parameter '{k}' is absent from kind '{kind_id}' in the active catalog"),
                node,
                &format!("{slot}/param:{k}"),
                kind_id,
                &k,
                false,
            );
        }
        for param in &kind.params {
            let pslot = format!("{slot}/param:{}", param.name);
            let present = params.get(&param.name).is_some_and(|v| !v.is_nil());
            if !present {
                if param.required {
                    self.diag(
                        "malformed",
                        format!("required parameter '{}' ({}) is missing", param.name, param.ty),
                        node,
                        &pslot,
                        kind_id,
                        &param.name,
                        false,
                    );
                }
                continue;
            }
            if let Some(v) = params.table_mut().and_then(|t| t.get_mut(&param.name)) {
                self.walk_param(kind, param, v, node, &pslot);
            }
        }
    }

    fn reference(&mut self, kind: &Kind, param: &Param, ty: &str, value: &mut Value, node: &str, slot: &str) {
        let Value::String(s) = value else { return };
        if self.ty != ty || s.as_str() != self.value {
            return;
        }
        self.references.push(Reference {
            path: self.path.clone(),
            node: node.to_owned(),
            slot: slot.to_owned(),
            kind: kind.id.clone(),
            param: param.name.clone(),
            ty: ty.to_owned(),
            value: s.to_string(),
            source: self.source.to_owned(),
        });
        if let Some(to) = &self.rename {
            *value = Value::from(to.as_str());
            self.edits += 1;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn object(
        &mut self,
        kind: &Kind,
        param: &Param,
        value: &Value,
        node: &str,
        slot: &str,
        schema: &str,
        allowed: &[&str],
    ) -> bool {
        if !value.is_table() {
            self.diag(
                "malformed",
                format!("{schema} must be a table"),
                node,
                slot,
                &kind.id,
                &param.name,
                true,
            );
            return false;
        }
        if !value.arr().is_empty() {
            self.diag(
                "malformed",
                format!("{schema} must not contain array entries"),
                node,
                slot,
                &kind.id,
                &param.name,
                false,
            );
        }
        let unknown: Vec<String> = value
            .fields()
            .iter()
            .filter(|(k, _)| !allowed.contains(&k.as_str()))
            .map(|(k, _)| k.to_string())
            .collect();
        for k in unknown {
            self.diag(
                "unknown_field",
                format!("field '{k}' is not valid for {schema}"),
                node,
                &format!("{slot}/{k}"),
                &kind.id,
                &param.name,
                false,
            );
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn string_leaf(
        &mut self,
        kind: &Kind,
        param: &Param,
        owner: &mut Value,
        key: &str,
        ty: &str,
        required: bool,
        node: &str,
        slot: &str,
    ) -> bool {
        let leaf = format!("{slot}/{key}");
        let Some(value) = owner.table_mut().and_then(|t| t.get_mut(key)) else {
            if required {
                self.diag(
                    "malformed",
                    format!("required string field '{key}' is missing"),
                    node,
                    &leaf,
                    &kind.id,
                    &param.name,
                    false,
                );
            }
            return false;
        };
        if !value.is_string() {
            self.diag(
                "malformed",
                format!("field '{key}' must be a string"),
                node,
                &leaf,
                &kind.id,
                &param.name,
                false,
            );
            return false;
        }
        self.reference(kind, param, ty, value, node, &leaf);
        true
    }

    fn position(&mut self, kind: &Kind, param: &Param, owner: &Value, node: &str, slot: &str) {
        let leaf = format!("{slot}/pos");
        let pos = owner.get("pos");
        let ok = pos.is_some_and(|p| p.is_table() && p.arr().len() == 3 && p.fields().is_empty());
        if !ok {
            self.diag(
                "malformed",
                "pos must be an array of exactly three numbers".into(),
                node,
                &leaf,
                &kind.id,
                &param.name,
                false,
            );
            return;
        }
        if let Some((i, _)) =
            pos.map(Value::arr).unwrap_or(&[]).iter().enumerate().find(|(_, x)| !x.is_number())
        {
            self.diag(
                "malformed",
                "pos must be an array of exactly three numbers".into(),
                node,
                &format!("{leaf}/{i}"),
                &kind.id,
                &param.name,
                false,
            );
        }
    }

    fn radius(&mut self, kind: &Kind, param: &Param, owner: &Value, node: &str, slot: &str) {
        if let Some(r) = owner.get("radius")
            && !r.as_number().is_some_and(|n| n > 0.0)
        {
            self.diag(
                "malformed",
                "radius must be a positive number".into(),
                node,
                &format!("{slot}/radius"),
                &kind.id,
                &param.name,
                false,
            );
        }
    }

    fn spawn_spec(&mut self, kind: &Kind, param: &Param, value: &mut Value, node: &str, slot: &str) {
        if !self.object(
            kind,
            param,
            value,
            node,
            slot,
            "spawn_spec",
            &["section", "smart", "ref", "hold", "place", "restrictor"],
        ) {
            return;
        }
        self.string_leaf(kind, param, value, "section", "squad_section", true, node, slot);
        // optional now: a place can stand in for it
        if value.has("smart") {
            self.string_leaf(kind, param, value, "smart", "smart", false, node, slot);
        }
        if value.has("ref") {
            self.string_leaf(kind, param, value, "ref", "ref_name", false, node, slot);
        }
        if value.has("restrictor") {
            self.string_leaf(kind, param, value, "restrictor", "restrictor", false, node, slot);
        }
        if let Some(place) = value.table_mut().and_then(|t| t.get_mut("place")) {
            self.place(kind, param, place, node, slot);
        }
        if value.get("hold").is_some_and(|h| !h.is_bool()) {
            self.diag(
                "malformed",
                "spawn_spec field 'hold' must be a bool".into(),
                node,
                &format!("{slot}/hold"),
                &kind.id,
                &param.name,
                false,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn npc_ref(
        &mut self,
        kind: &Kind,
        param: &Param,
        value: &mut Value,
        node: &str,
        slot: &str,
        target: bool,
        kill: bool,
    ) {
        const NPC: &[&str] = &["story", "ref", "character", "profile", "community", "level"];
        const TARGET: &[&str] = &[
            "story",
            "ref",
            "character",
            "profile",
            "community",
            "level",
            "smart",
            "restrictor",
            "pos",
            "radius",
        ];
        const KILL: &[&str] = &["story", "ref", "character", "profile", "community", "level", "spawn"];
        let allowed = if target {
            TARGET
        } else if kill {
            KILL
        } else {
            NPC
        };
        if !self.object(kind, param, value, node, slot, &param.ty, allowed) {
            return;
        }
        let mut alts =
            ["story", "ref", "character", "profile", "community"].iter().filter(|k| value.has(k)).count();
        if target {
            alts += ["smart", "restrictor", "pos"].iter().filter(|k| value.has(k)).count();
        }
        if kill && value.has("spawn") {
            alts += 1;
        }
        if alts != 1 {
            self.diag(
                "malformed",
                format!("{} must select exactly one target variant", param.ty),
                node,
                slot,
                &kind.id,
                &param.name,
                false,
            );
        }
        for (key, ty) in [
            ("story", "story_id"),
            ("ref", "ref_name"),
            ("character", "character_id"),
            ("profile", "profile"),
            ("community", "community"),
        ] {
            if value.has(key) {
                self.string_leaf(kind, param, value, key, ty, false, node, slot);
            }
        }
        if value.has("level") {
            if !value.has("community") && !(target && value.has("pos")) {
                self.diag(
                    "malformed",
                    "level is valid only for community or pos variants".into(),
                    node,
                    &format!("{slot}/level"),
                    &kind.id,
                    &param.name,
                    false,
                );
            }
            self.string_leaf(kind, param, value, "level", "level", false, node, slot);
        }
        if target && value.has("smart") {
            self.string_leaf(kind, param, value, "smart", "smart", false, node, slot);
        }
        if target && value.has("restrictor") {
            self.string_leaf(kind, param, value, "restrictor", "restrictor", false, node, slot);
        }
        if target && value.has("pos") {
            self.position(kind, param, value, node, slot);
        }
        if target && value.has("radius") {
            if !value.has("pos") {
                self.diag(
                    "malformed",
                    "radius is valid only for the pos variant".into(),
                    node,
                    &format!("{slot}/radius"),
                    &kind.id,
                    &param.name,
                    false,
                );
            }
            self.radius(kind, param, value, node, slot);
        }
        if kill && let Some(spawn) = value.table_mut().and_then(|t| t.get_mut("spawn")) {
            self.spawn_spec(kind, param, spawn, node, &format!("{slot}/spawn"));
        }
    }

    fn object_ref(&mut self, kind: &Kind, param: &Param, value: &mut Value, node: &str, slot: &str) {
        let object = param.ty == "object_ref";
        let allowed: &[&str] = if object { &["story", "ref", "character"] } else { &["story", "ref"] };
        if !self.object(kind, param, value, node, slot, &param.ty, allowed) {
            return;
        }
        let alts = usize::from(value.has("story"))
            + usize::from(value.has("ref"))
            + usize::from(object && value.has("character"));
        if alts != 1 {
            self.diag(
                "malformed",
                format!(
                    "{} must select exactly one of story / ref{}",
                    param.ty,
                    if object { " / character" } else { "" }
                ),
                node,
                slot,
                &kind.id,
                &param.name,
                false,
            );
        }
        if value.has("story") {
            self.string_leaf(kind, param, value, "story", "story_id", false, node, slot);
        }
        if value.has("ref") {
            self.string_leaf(kind, param, value, "ref", "ref_name", false, node, slot);
        }
        if object && value.has("character") {
            self.string_leaf(kind, param, value, "character", "character_id", false, node, slot);
        }
    }

    fn place(&mut self, kind: &Kind, param: &Param, value: &mut Value, node: &str, slot: &str) {
        if !self.object(
            kind,
            param,
            value,
            node,
            slot,
            "place",
            &["level", "pos", "radius", "restrictor", "smart", "ref"],
        ) {
            return;
        }
        let position = value.has("pos") || value.has("level") || value.has("radius");
        let alts = usize::from(position)
            + usize::from(value.has("restrictor"))
            + usize::from(value.has("smart"))
            + usize::from(value.has("ref"));
        if alts != 1 {
            self.diag(
                "malformed",
                "place must select exactly one of level+pos, restrictor or smart".into(),
                node,
                slot,
                &kind.id,
                &param.name,
                false,
            );
        }
        if position {
            self.string_leaf(kind, param, value, "level", "level", true, node, slot);
            self.position(kind, param, value, node, slot);
            self.radius(kind, param, value, node, slot);
        }
        if value.has("ref") {
            self.string_leaf(kind, param, value, "ref", "ref_name", false, node, slot);
        }
        if value.has("restrictor") {
            self.string_leaf(kind, param, value, "restrictor", "restrictor", false, node, slot);
        }
        if value.has("smart") {
            self.string_leaf(kind, param, value, "smart", "smart", false, node, slot);
        }
    }

    fn duration(&mut self, kind: &Kind, param: &Param, value: &Value, node: &str, slot: &str) {
        if let Some(n) = value.as_number() {
            if n < 0.0 {
                self.diag(
                    "malformed",
                    "duration must be non-negative".into(),
                    node,
                    slot,
                    &kind.id,
                    &param.name,
                    false,
                );
            }
            return;
        }
        const UNITS: &[&str] = &["seconds", "game_hours", "game_minutes", "game_seconds"];
        if !self.object(kind, param, value, node, slot, "duration", UNITS) {
            return;
        }
        let mut alts = 0;
        for unit in UNITS {
            let Some(u) = value.get(unit) else { continue };
            alts += 1;
            if !u.as_number().is_some_and(|n| n >= 0.0) {
                self.diag(
                    "malformed",
                    format!("duration field '{unit}' must be a non-negative number"),
                    node,
                    &format!("{slot}/{unit}"),
                    &kind.id,
                    &param.name,
                    false,
                );
            }
        }
        if alts != 1 {
            self.diag(
                "malformed",
                "duration must select exactly one unit".into(),
                node,
                slot,
                &kind.id,
                &param.name,
                false,
            );
        }
    }

    fn walk_param(&mut self, kind: &Kind, param: &Param, value: &mut Value, node: &str, slot: &str) {
        let ty = param.ty.as_str();
        if !is_known_type(ty) {
            self.diag(
                "unknown_type",
                format!("parameter type '{ty}' is not inspectable"),
                node,
                slot,
                &kind.id,
                &param.name,
                false,
            );
            return;
        }
        if ty == "lua" {
            self.diag(
                "lua_uninspectable",
                "Lua may contain runtime references that cannot be proven statically".into(),
                node,
                slot,
                &kind.id,
                &param.name,
                false,
            );
            return;
        }
        if is_string_type(ty) {
            let Some(s) = value.as_str() else {
                self.diag(
                    "malformed",
                    format!("parameter '{}' ({ty}) must be a string", param.name),
                    node,
                    slot,
                    &kind.id,
                    &param.name,
                    false,
                );
                return;
            };
            if ty == "enum" || ty == "relation" {
                let allowed = if ty == "relation" && param.enums.is_empty() {
                    matches!(s, "enemy" | "neutral" | "friend")
                } else {
                    param.enums.is_empty() || param.enums.iter().any(|e| e == s)
                };
                if !allowed {
                    self.diag(
                        "malformed",
                        format!("'{s}' is not an allowed {ty} value"),
                        node,
                        slot,
                        &kind.id,
                        &param.name,
                        false,
                    );
                }
            }
            self.reference(kind, param, ty, value, node, slot);
            return;
        }
        match ty {
            "cond_list" => self.cond_list(value, node, slot),
            "cases_cond" => self.cases(value, node, slot, false),
            "cases_weight" => self.cases(value, node, slot, true),
            "npc_ref" => self.npc_ref(kind, param, value, node, slot, false, false),
            "target_ref" => self.npc_ref(kind, param, value, node, slot, true, false),
            "kill_target" => self.npc_ref(kind, param, value, node, slot, false, true),
            "squad_ref" | "object_ref" => self.object_ref(kind, param, value, node, slot),
            "place" => self.place(kind, param, value, node, slot),
            "spawn_spec" => self.spawn_spec(kind, param, value, node, slot),
            "duration" => self.duration(kind, param, value, node, slot),
            "int" | "float" => match value.as_number() {
                None => self.diag(
                    "malformed",
                    format!("parameter '{}' must be a number", param.name),
                    node,
                    slot,
                    &kind.id,
                    &param.name,
                    false,
                ),
                Some(n) => {
                    if ty == "int" && !is_integer(n) {
                        self.diag(
                            "malformed",
                            format!("parameter '{}' must be an integer", param.name),
                            node,
                            slot,
                            &kind.id,
                            &param.name,
                            false,
                        );
                    }
                    if param.min.is_some_and(|m| n < m) {
                        self.diag(
                            "malformed",
                            format!("parameter '{}' is below its catalog minimum", param.name),
                            node,
                            slot,
                            &kind.id,
                            &param.name,
                            false,
                        );
                    }
                    if param.max.is_some_and(|m| n > m) {
                        self.diag(
                            "malformed",
                            format!("parameter '{}' is above its catalog maximum", param.name),
                            node,
                            slot,
                            &kind.id,
                            &param.name,
                            false,
                        );
                    }
                }
            },
            "bool" => {
                if !value.is_bool() {
                    self.diag(
                        "malformed",
                        format!("parameter '{}' must be a bool", param.name),
                        node,
                        slot,
                        &kind.id,
                        &param.name,
                        false,
                    );
                }
            }
            "text" => {
                if value.is_string() {
                    return;
                }
                let ok = value.as_table().is_some_and(|t| !t.fields.is_empty() && t.arr.is_empty());
                if !ok {
                    self.diag(
                        "malformed",
                        "text must be a string or a non-empty language table".into(),
                        node,
                        slot,
                        &kind.id,
                        &param.name,
                        false,
                    );
                    return;
                }
                let bad: Vec<String> = value
                    .fields()
                    .iter()
                    .filter(|(_, x)| !x.is_string())
                    .map(|(k, _)| k.to_string())
                    .collect();
                for k in bad {
                    self.diag(
                        "malformed",
                        "localized text value must be a string".into(),
                        node,
                        &format!("{slot}/{k}"),
                        &kind.id,
                        &param.name,
                        false,
                    );
                }
            }
            "value" => {
                if value.is_table() || value.is_nil() {
                    self.diag(
                        "malformed",
                        "value must be a bool, number or string".into(),
                        node,
                        slot,
                        &kind.id,
                        &param.name,
                        false,
                    );
                }
            }
            "count_or_all" => {
                let ok = value.as_str() == Some("all")
                    || value.as_number().is_some_and(|n| is_integer(n) && n >= 1.0);
                if !ok {
                    self.diag(
                        "malformed",
                        "count_or_all must be a positive integer or 'all'".into(),
                        node,
                        slot,
                        &kind.id,
                        &param.name,
                        false,
                    );
                }
            }
            _ => self.diag(
                "unknown_type",
                format!("parameter type '{ty}' has no structural walker"),
                node,
                slot,
                &kind.id,
                &param.name,
                false,
            ),
        }
    }

    fn cond_list(&mut self, list: &mut Value, node: &str, slot: &str) {
        if !list.is_table() || list.arr().is_empty() || !list.fields().is_empty() {
            self.diag(
                "malformed",
                "condition list must be a non-empty array".into(),
                node,
                slot,
                "",
                "",
                true,
            );
            return;
        }
        if let Some(t) = list.table_mut() {
            for (i, c) in t.arr.iter_mut().enumerate() {
                self.cond_value(c, node, &format!("{slot}/cond:{i}"));
            }
        }
    }

    fn cond_container(&mut self, list: &mut Value, node: &str, slot: &str) {
        if !list.is_table() {
            self.diag("malformed", "condition container must be a table".into(), node, slot, "", "", true);
            return;
        }
        if list.arr().is_empty() && list.has("kind") {
            self.cond_value(list, node, &format!("{slot}/cond:0"));
            return;
        }
        if !list.fields().is_empty() {
            self.diag(
                "malformed",
                "condition list must contain only condition entries".into(),
                node,
                slot,
                "",
                "",
                false,
            );
            return;
        }
        if let Some(t) = list.table_mut() {
            for (i, c) in t.arr.iter_mut().enumerate() {
                self.cond_value(c, node, &format!("{slot}/cond:{i}"));
            }
        }
    }

    fn cond_value(&mut self, value: &mut Value, node: &str, slot: &str) {
        if !value.is_table() {
            self.diag("malformed", "condition must be a table".into(), node, slot, "", "", false);
            return;
        }
        if !value.arr().is_empty() {
            self.diag(
                "malformed",
                "condition must not contain array entries".into(),
                node,
                slot,
                "",
                "",
                false,
            );
        }
        let unknown: Vec<String> = value
            .fields()
            .iter()
            .filter(|(k, _)| !matches!(k.as_str(), "kind" | "params" | "not"))
            .map(|(k, _)| k.to_string())
            .collect();
        for k in unknown {
            self.diag(
                "unknown_field",
                format!("unknown condition field '{k}'"),
                node,
                &format!("{slot}/{k}"),
                "",
                "",
                false,
            );
        }
        let kind = match value.get("kind") {
            Some(Value::String(s)) if !s.is_empty() => s.to_string(),
            _ => {
                self.diag("malformed", "condition needs a string kind".into(), node, slot, "", "", false);
                return;
            }
        };
        if value.get("not").is_some_and(|n| !n.is_bool()) {
            self.diag(
                "malformed",
                "condition field 'not' must be a bool".into(),
                node,
                &format!("{slot}/not"),
                "",
                "",
                false,
            );
        }
        let mut empty = Value::Nil;
        let params = match value.table_mut().and_then(|t| t.get_mut("params")) {
            Some(p) => p,
            None => &mut empty,
        };
        self.walk_kind(&kind, params, node, slot, USE_COND);
    }

    fn cases(&mut self, cases: &mut Value, node: &str, slot: &str, weighted: bool) {
        if !cases.is_table() || cases.arr().is_empty() || !cases.fields().is_empty() {
            self.diag("malformed", "cases must be a non-empty array".into(), node, slot, "", "", false);
            return;
        }
        let names: Vec<Option<String>> =
            cases.arr().iter().map(|c| c.get("name").and_then(Value::as_str).map(str::to_owned)).collect();
        let Some(t) = cases.table_mut() else { return };
        for (i, item) in t.arr.iter_mut().enumerate() {
            let case_slot = format!("{slot}/case:{i}");
            if !item.is_table() {
                self.diag("malformed", "case must be a table".into(), node, &case_slot, "", "", false);
                continue;
            }
            if !item.arr().is_empty() {
                self.diag(
                    "malformed",
                    "case must not contain array entries".into(),
                    node,
                    &case_slot,
                    "",
                    "",
                    false,
                );
            }
            let want = if weighted { "weight" } else { "cond" };
            let unknown: Vec<String> = item
                .fields()
                .iter()
                .filter(|(k, _)| k != "name" && k != want)
                .map(|(k, _)| k.to_string())
                .collect();
            for k in unknown {
                self.diag(
                    "unknown_field",
                    format!("unknown case field '{k}'"),
                    node,
                    &format!("{case_slot}/{k}"),
                    "",
                    "",
                    false,
                );
            }
            match item.get("name") {
                Some(Value::String(name)) => {
                    if !valid_id(name) {
                        self.diag(
                            "malformed",
                            "case name must be [a-z0-9_]+".into(),
                            node,
                            &format!("{case_slot}/name"),
                            "",
                            "",
                            false,
                        );
                    }
                    if names.iter().take(i).any(|p| p.as_deref() == Some(name.as_str())) {
                        self.diag(
                            "malformed",
                            format!("duplicate case name '{name}'"),
                            node,
                            &format!("{case_slot}/name"),
                            "",
                            "",
                            false,
                        );
                    }
                }
                _ => self.diag(
                    "malformed",
                    "case needs a string name".into(),
                    node,
                    &format!("{case_slot}/name"),
                    "",
                    "",
                    false,
                ),
            }
            if weighted {
                if !item.get("weight").and_then(Value::as_number).is_some_and(|w| w > 0.0) {
                    self.diag(
                        "malformed",
                        "weighted case needs weight > 0".into(),
                        node,
                        &format!("{case_slot}/weight"),
                        "",
                        "",
                        false,
                    );
                }
            } else {
                match item.table_mut().and_then(|t| t.get_mut("cond")) {
                    None => self.diag(
                        "malformed",
                        "conditional case needs cond".into(),
                        node,
                        &format!("{case_slot}/cond"),
                        "",
                        "",
                        false,
                    ),
                    Some(c) => self.cond_container(c, node, &case_slot),
                }
            }
        }
    }
}

/// Every exact reference of `ty` = `value` in the snapshot, or in the one graph whose
/// project-relative path key is `scope` (`Find`).
pub fn find(
    snapshot: &IndexSnapshot,
    catalog: &Catalog,
    ty: &str,
    value: &str,
    scope: Option<&str>,
) -> Result<FindResult, String> {
    let mut out = FindResult {
        generation: snapshot.generation,
        fingerprint: snapshot.fingerprint,
        catalog_generation: catalog.generation,
        ..FindResult::default()
    };
    if ty.is_empty() {
        return Err("reference type is empty".into());
    }
    if value.is_empty() {
        return Err("reference value is empty".into());
    }
    if !is_known_type(ty) {
        return Err(format!("unknown catalog parameter type '{ty}'"));
    }
    if !catalog.loaded() {
        return Err(format!("catalog unavailable: {}", catalog.error));
    }
    let mut found_scope = scope.is_none();
    for e in &snapshot.entries {
        if let Some(s) = scope
            && !e.rel.eq_ignore_ascii_case(s)
        {
            continue;
        }
        found_scope = true;
        if !e.readable || !e.complete {
            out.diagnostics.push(Diagnostic {
                path: e.rel.clone(),
                code: (if e.readable { "incomplete_model" } else { "unreadable" }).to_owned(),
                message: if e.error.is_empty() {
                    "quest could not be inspected".to_owned()
                } else {
                    e.error.clone()
                },
                source: e.source().to_owned(),
                // unread content can hide anything
                blocking: true,
                ..Diagnostic::default()
            });
            continue;
        }
        let mut q = (*e.graph).clone();
        let mut w = Walker {
            cat: catalog,
            path: e.rel.clone(),
            source: e.source(),
            ty: ty.to_owned(),
            value: value.to_owned(),
            rename: None,
            references: Vec::new(),
            diagnostics: Vec::new(),
            edits: 0,
        };
        w.graph(&mut q);
        out.references.extend(w.references);
        out.diagnostics.extend(w.diagnostics);
    }
    if !found_scope {
        return Err("quest is not present in the project index".into());
    }
    // complete means "every reference was seen", not "the project is spotless"
    out.complete = !out.diagnostics.iter().any(|d| d.blocking);
    Ok(out)
}

/// Renames a task and every proven `task_id` reference to it (`RenameTask`): all or nothing.
/// `Err` carries the diagnostics that refused it.
pub fn rename_task(
    q: &Graph,
    catalog: &Catalog,
    path: &str,
    from: &str,
    to: &str,
) -> Result<(Graph, usize), Vec<Diagnostic>> {
    let memory = |code: &str, message: String| Diagnostic {
        path: path.to_owned(),
        code: code.to_owned(),
        message,
        source: "memory".into(),
        ..Diagnostic::default()
    };
    if !catalog.loaded() {
        return Err(vec![memory("catalog_unavailable", format!("catalog unavailable: {}", catalog.error))]);
    }
    let mut after = q.clone();
    let mut w = Walker {
        cat: catalog,
        path: path.to_owned(),
        source: "memory",
        ty: "task_id".into(),
        value: from.to_owned(),
        rename: Some(to.to_owned()),
        references: Vec::new(),
        diagnostics: Vec::new(),
        edits: 0,
    };
    w.graph(&mut after);
    // only a walk that gave up on a subtree can hide a task reference
    let blocking: Vec<Diagnostic> = w.diagnostics.into_iter().filter(|d| d.blocking).collect();
    if !blocking.is_empty() {
        return Err(blocking);
    }
    let Some(at) = after.tasks.iter().position(|t| t.id == from) else {
        return Err(vec![memory("missing_declaration", format!("task '{from}' is not declared"))]);
    };
    if let Some(t) = after.tasks.get_mut(at) {
        t.id = to.to_owned();
    }
    Ok((after, w.edits))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::parse;

    fn bundled() -> Catalog {
        let bytes = include_bytes!("../../../gamedata/configs/nq/catalog.ltx");
        let text = gz_base::text::cp1251_to_utf8(bytes).into_owned();
        Catalog::assemble(&[crate::catalog::SourceFile { label: "bundled".into(), text }], false, None, 1)
    }

    #[test]
    fn renames_every_proven_task_reference() {
        let cat = bundled();
        let text = include_str!("../../../docs/nq/examples/dialog_branching.nqasset");
        let g = match parse(&cat, text, "dialog_branching") {
            Ok(g) => g,
            Err(e) => panic!("{}", e.message),
        };
        let (after, n) = match rename_task(&g, &cat, "quests/dialog_branching.nqasset", "kill_boars", "cull")
        {
            Ok(r) => r,
            Err(d) => panic!("{:?}", d.iter().map(Diagnostic::text).collect::<Vec<_>>()),
        };
        assert_eq!(n, 2);
        assert!(after.find_task("cull").is_some());
        assert!(after.find_task("kill_boars").is_none());
        assert!(
            matches!(rename_task(&g, &cat, "p", "nope", "x"), Err(d) if d.first().is_some_and(|d| d.code == "missing_declaration"))
        );
    }
}
