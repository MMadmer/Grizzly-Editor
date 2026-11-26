//! A static object that is a stash, and the box the bake puts over it.
//!
//! In the game a stash is two things at one place: the box one sees is the static model (level
//! geometry, or the module's piece of a level of the game), and the stash is an `inventory_box`
//! whose model is that box's unseen case - the model's `_case` twin, a few faces nobody sees with
//! the same pivot (`dynamics\equipment_cache\equipment_box_02_case`). A static object that says
//! `stash` gets that box from the bake, under the object's own name: the game names a module's
//! object `<module>.<name>`, and a quest reaches it by the name (`{ name = "<name>" }`). Whether
//! the game puts a random stash's loot in it and whether the NPCs who search stashes go to it is
//! the box's `[xms_stash]` (the game's MODDING.md, "A placed stash"): a key left out is `true`.

use crate::class::ObjClass;
use crate::model::{ObjData, ObjId, Object, SceneState};

/// The section of a stash's box.
pub const SECTION: &str = "inventory_box";
/// The size of the case `inventory_box`'s own model is, metres: what a stash opens by when its
/// model has no case of its own.
pub const OWN_CASE: [f32; 3] = [0.94, 0.43, 0.48];

/// What a stash says of itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stash {
    /// The game may put a random stash's loot in it.
    pub random: bool,
    /// The NPCs who search stashes may search it and take what is in it.
    pub loot: bool,
}

impl Default for Stash {
    fn default() -> Self {
        Self { random: true, loot: true }
    }
}

impl Stash {
    /// The box's custom data: `[xms_stash]` with the keys that are `false`; empty when both are
    /// `true`, a box like any other.
    #[must_use]
    pub fn section(&self) -> String {
        if self.random && self.loot {
            return String::new();
        }
        let mut out = String::from("[xms_stash]\r\n");
        if !self.random {
            out.push_str("random = false\r\n");
        }
        if !self.loot {
            out.push_str("loot = false\r\n");
        }
        out
    }
}

/// The stash a static object is, when it is one.
#[must_use]
pub fn of(o: &Object) -> Option<Stash> {
    match &o.data {
        ObjData::SceneObject(d) => d.stash,
        _ => None,
    }
}

/// The stashes the bake makes a box for: what the module adds (`authored`) and says `stash`.
pub fn baked(state: &SceneState) -> impl Iterator<Item = (&Object, Stash)> + '_ {
    state
        .of_class(ObjClass::SceneObject)
        .filter(|o| o.marks.authored && !o.marks.base)
        .filter_map(|o| of(o).map(|s| (&**o, s)))
}

/// The unseen case of a model in the configs' form: `<model>_case` without `meshes\` and the
/// extension, with back slashes (`dynamics\box\box_02` -> `dynamics\box\box_02_case`).
#[must_use]
pub fn case_model(reference: &str) -> String {
    let r = reference.trim().replace('/', "\\");
    let r = match r.get(..7) {
        Some(head) if head.eq_ignore_ascii_case("meshes\\") => r[7..].to_owned(),
        _ => r,
    };
    // the extension of the file's name, never a dot of a folder
    let stem = match r.rsplit_once('.') {
        Some((stem, ext)) if !ext.contains('\\') && !stem.is_empty() => stem,
        _ => r.as_str(),
    };
    format!("{stem}_case")
}

/// What is wrong with the name of each stash the bake makes: a quest reaches the box by the
/// name, so the spawn layer must write it as it is - letters it can say, and no other object of
/// the layer under it. The stash and the sentence.
#[must_use]
pub fn clashes(state: &SceneState) -> Vec<(ObjId, String)> {
    let mut out = Vec::new();
    for (o, _) in baked(state) {
        let name = o.name.as_str();
        if super::op_name(name) != name {
            out.push((
                o.id,
                format!(
                    "{name} is a stash, and a quest reaches it by its name: the game names it as the spawn layer writes it, which takes a-z, A-Z, 0-9, _ - . only (it would be '{}') - rename it",
                    super::op_name(name)
                ),
            ));
            continue;
        }
        let other = [ObjClass::SceneObject, ObjClass::SpawnPoint]
            .into_iter()
            .flat_map(|c| state.of_class(c))
            .filter(|x| x.id != o.id && x.marks.authored && !x.marks.base)
            .find(|x| super::op_name(&x.name).eq_ignore_ascii_case(name));
        if let Some(x) = other {
            out.push((
                o.id,
                format!(
                    "{name} is a stash, and a quest reaches it by its name: {} is written into the spawn layer under the same name - rename one of them",
                    x.name
                ),
            ));
        }
    }
    out
}

/// What the reader says of a stash whose name another object of the scene has already: it is
/// renamed, and a quest that names it finds the other.
#[must_use]
pub fn taken_text(name: &str, fresh: &str) -> String {
    format!(
        "the name '{name}' is used twice: this stash is '{fresh}' now, and a quest that names '{name}' finds the other object - a stash is reached by its name; rename one of them"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::SceneObjectData;

    #[test]
    fn the_case_is_the_models_twin_in_the_configs_form() {
        assert_eq!(
            case_model("dynamics\\equipment_cache\\equipment_box_02"),
            "dynamics\\equipment_cache\\equipment_box_02_case"
        );
        assert_eq!(case_model("meshes/dynamics/box/box_wood.ogf"), "dynamics\\box\\box_wood_case");
        assert_eq!(case_model("models/yard/crate.meshasset"), "models\\yard\\crate_case");
        // a dot of a folder is no extension
        assert_eq!(case_model("props.v2\\box"), "props.v2\\box_case");
    }

    #[test]
    fn the_custom_data_says_only_a_false_key() {
        assert_eq!(Stash::default().section(), "");
        assert_eq!(Stash { random: false, loot: true }.section(), "[xms_stash]\r\nrandom = false\r\n");
        assert_eq!(Stash { random: true, loot: false }.section(), "[xms_stash]\r\nloot = false\r\n");
        assert_eq!(
            Stash { random: false, loot: false }.section(),
            "[xms_stash]\r\nrandom = false\r\nloot = false\r\n"
        );
    }

    #[test]
    fn a_stash_is_reached_by_a_name_the_layer_writes_as_it_is() {
        let object = |class: ObjClass, name: &str, stash: bool| {
            let data = match class {
                ObjClass::SceneObject => ObjData::SceneObject(SceneObjectData {
                    stash: stash.then(Stash::default),
                    ..SceneObjectData::library("dynamics\\box")
                }),
                _ => crate::objects::default_data(class),
            };
            let mut o = Object::new(class, name, data);
            o.marks.authored = true;
            o
        };
        let mut s = SceneState::default();
        let cache = s.insert(object(ObjClass::SceneObject, "cache_1", true));
        s.insert(object(ObjClass::SceneObject, "crate 1", false));
        assert!(clashes(&s).is_empty());
        // a name the layer cannot write as it is
        let spaced = s.insert(object(ObjClass::SceneObject, "my stash", true));
        // another object of the layer under the stash's name
        s.insert(object(ObjClass::SpawnPoint, "cache 1", false));
        let found = clashes(&s);
        assert_eq!(found.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [cache, spaced]);
        assert!(
            found[0].1.contains("cache 1 is written into the spawn layer under the same name"),
            "{}",
            found[0].1
        );
        assert!(found[1].1.contains("(it would be 'my_stash')"), "{}", found[1].1);
        // what the module does not add is no box of the layer
        let mut kept = (**s.get(spaced).unwrap()).clone();
        kept.marks.authored = false;
        s.replace(kept);
        assert_eq!(clashes(&s).len(), 1);
    }
}
