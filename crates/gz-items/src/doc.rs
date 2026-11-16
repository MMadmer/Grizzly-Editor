//! The reference of the item kind, made from the tables the reader uses: `docs/assets/ITEM.md`
//! is [`reference`], and `asset.reference kind=item` answers it with the linked game's catalog
//! after it ([`catalog_reference`]).

use crate::catalog::Catalog;
use crate::fields::{CRAFT, Field, HUD, ICON, LIST, LOOT, SCENE, SECTION, SOLD, TOP};

fn esc(s: &str) -> String {
    s.replace('|', "\\|")
}

fn table(out: &mut String, fields: &[Field]) {
    out.push_str("| Key | Takes | Left out | Means |\n|---|---|---|---|\n");
    for f in fields {
        let default = if f.default.is_empty() { "-".to_owned() } else { f.default.to_owned() };
        out.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            f.key,
            esc(&f.ty.describe()),
            esc(&default),
            esc(f.help)
        ));
    }
}

const INTRO: &str = "\
One file anywhere in the module is one item of the inventory - a weapon, a tool, a consumable, an outfit, a plain \
thing: a NEW one, or a CHANGE of one that exists (the game's, or another module's). The module ships the file as \
it is; the game reads it when its configs come up and makes the sections, the strings and the list entries itself. \
It is plain data like every asset (`docs/ASSETS.md`): `return { ... }` of literals, a key the game does not read \
is passed by, a key the editor does not know is kept through every save and noted once.\n\n";

const EXAMPLES: &str = "\
```lua\n\
-- a new item\n\
return {\n\
  item = 1,\n\
  id = \"sil_diy\",                 -- the file name when left out\n\
  section = \"wpn_sil_diy\",        -- \"<module>_<id>\" when left out\n\
  base = \"wpn_sil_pbs1\",          -- the section it starts from, or a list of them\n\
  name = { rus = \"...\", eng = \"DIY silencer\" },\n\
  description = { eng = \"...\" },\n\
  icon = \"icons/sil_diy.png\",     -- a picture beside the asset: the build bakes it into the module's atlas\n\
  set = { cost = 1200, inv_weight = 0.4, visual = [[dynamics\\weapons\\wpn_sil\\wpn_sil_diy]] },\n\
  sections = {\n\
    { name = \"wpn_sil_diy_sounds\", base = \"base_sounds\", set = { snd_shoot = [[weapons\\sil\\diy_shot]] } },\n\
  },\n\
  craft = { { at = \"itm_repairkit_tier_1\", parts = { \"jar\", \"textile_patch_m\", \"rope\", \"flashlight_broken\" } } },\n\
  loot  = { count = { 0, 1 }, chance = 0.01, by = { bandit = 0.03 } },\n\
  sold  = { { by = \"trade_military\", count = 1, chance = 0.5 } },\n\
}\n\
```\n\n\
```lua\n\
-- a change of an item that exists\n\
return {\n\
  item = 1,\n\
  edit = \"wpn_ak74\",\n\
  set   = { snd_shoot = [[weapons\\ak74\\ak74_shoot]], flame_particles = [[weapons\\new_effects\\ak74]] },\n\
  unset = { \"snd_shoot1\" },\n\
  add   = { ammo_class = { \"ammo_5.45x39_ap\" } },\n\
  drop  = { ammo_class = { \"ammo_5.45x39_fmj\" } },\n\
  hud   = { set = { hands_position = { 0.02, -0.01, 0.1 } }, unset = { \"anm_bore\" } },\n\
  icon  = \"icons/wpn_ak74.png\",   -- the game's item gets a new picture; the game's atlas is not touched\n\
}\n\
```\n\n";

const RULES: &str = "\
## New or change\n\n\
- **A new item** names what it starts from (`base`) and says what is different. Start from a section of the same \
kind - a pistol from a pistol, a medkit from a medkit - and write the difference: nothing is defaulted that the \
base gives. `id`, `section` and `abstract` make a file a new item too.\n\
- **A change** names the section it changes (`edit`) and says only what changes. It needs no `base`.\n\
- **Never both**: `edit` with `id`, `section`, `base` or `abstract` is refused. A new item whose section exists \
already is refused as well - write it as a change. A template (`abstract = true`) needs no base.\n\n\
## What the game makes\n\n\
Of a new item: `[<section>]:<base>` with `set` (a list joined by `, `), `unset`, `add` and `drop` (the list rewritten \
joined by `,`); the strings - `<section>_name`, `<section>_short` and `<section>_descr` with `inv_name`, \
`inv_name_short` (the name's id when `short_name` is left out) and `description`, over what `set` says; \
`icons_texture` and `inv_grid_*` of the icon; `<section>_hud` from `hud`, over the HUD section the bases name \
when `hud` names no base; the `sections`; the recipes (`[xms_craft] <n> = at, section, count, part, ...`, `n` a \
zero-padded number after the highest there); then the lines of other configs: the death loot, the traders' \
stock, the use scene (`items\\items\\animations_settings.ltx`: `sound` is `snd`, `camera` `cam`, `time` `tm`, \
`helmet` `helm`), `lists`. A template (`abstract = true`) is its section and nothing else. Of a change: the same \
keys on the section it edits, its HUD section changed through the section's own `hud` key. **Any malformed part \
refuses the whole item**: a key no config can hold, a value with a line break, a recipe of five parts.\n\n\
## Order\n\n\
A module's characters, then its items - the new sections first (an item made from another item of the module \
after that one, else in the order of their paths), then the changes in the order of their paths -, then its \
`configs\\xms\\*.ltx`, then its `patch\\*.ltxp`: what the module's own overlays and patches say of an item still \
counts. A section an item makes that the module's overlay defines again is merged, the later one winning - its \
own sections, and the ones made for the rows it adds (`[xms_craft]`, a scene's HUD section, a section of `lists`). \
Every change reaches the sections that inherit the changed key and do not set it themselves: \
`add = { ammo_class = ... }` on `wpn_ak74` reaches `wpn_ak74_kobra`. The order of paths is the game's: the paths \
compared byte by byte as the game reads them, ASCII letters lower-cased and `/` between folders - \
`items/weapons/ak` before `items/weapons2/ak`, `B` after `a`; of two changes of one key the later path wins.\n\n\
A refused item stops nothing: the game leaves it out with one line in its log (`! XMS: [<module>] <file>: left out \
- <why>`) - no base, an unknown base, a name that cannot be a section, an edit of a section that is not there.\n\n\
## Icons\n\n\
The game knows one form: a rectangle of a texture in cells of 50 px (`icons_texture`, `inv_grid_x`, `inv_grid_y`, \
`inv_grid_width`, `inv_grid_height`) - `icon = { texture, x, y, w, h }`. An author may give a picture instead \
(`icon = \"icons/ak74.png\"`, any size; or `{ file = \"icons/ak74.png\", w = 2, h = 1 }`): the module build packs \
every such picture into `gamedata\\textures\\ui\\<module>_icons.dds` - by the items' sections, each scaled to its \
cells; `w` and `h` left out are its size in cells, rounded up (100 x 50 px is 2 x 1) - and ships each asset with \
`icon = { texture = [[ui\\<module>_icons]], x, y, w, h }`, nothing else of its text changed. The pictures are \
sources and never ship. An icon on a change gives the game's item a new picture without touching the game's \
atlas.\n\n\
## The catalog\n\n\
What a key of an item's section means is the game's: its `configs\\item_catalog.ltx` (the editor's bundled copy \
when no game is linked) names the families of items by their engine class and, for every key of an item's or its \
HUD's section, its type, group, title, unit, whether the game stops without it and a sentence of help. The check \
reads it - a key the game stops without, a value of the wrong type, a model, sound, texture, particle effect, \
section, string id or motion nobody has - and `asset.reference kind=item` lists it after this page. \
`items.info item=<file or section>` answers what an item resolves to: every key of its section with its value and \
where the value comes from (the item, a base by name, the game, the module), its HUD section, for a change the \
game's values before it.\n\n";

const CODES: &str = "\
## Problems\n\n\
`asset.check` names every problem at its key; the build refuses a module whose items have an error, with the \
first one.\n\n\
| Code | Severity | What |\n|---|---|---|\n\
| `I400` | error | The file does not read. |\n\
| `I401` | error | Neither a new item nor a change: nothing says what the file is. |\n\
| `I402` | error | Both: `edit` with `id`, `section`, `base` or `abstract`. |\n\
| `I403` | error | An id that cannot be one (`a-z 0-9 _`, not starting with a digit, up to 48). |\n\
| `I404` | error | A section name that cannot be one (`a-z 0-9 _ . -`, under 64 characters). |\n\
| `I405` | error | A new item without a base. |\n\
| `I406` | error | Two items of the module make one section (on both). |\n\
| `I407` | error | A new item, or a section it makes, whose section the game or the module already has. |\n\
| `I408` | error | A base nothing has: not the game's, not a module's loaded before, not another item's. |\n\
| `I409` | error | An edit of a section that is not there (the item's, its HUD's, a row of `sections`). |\n\
| `I410` | error | The bases of the module's items lead back to themselves. |\n\
| `I411` | error | A key the catalog says the game stops without, which the section does not get (a base of the game that lacks it too shows the game does without it). |\n\
| `I412` | warning | A value that is no value of the key's type. |\n\
| `I413` | warning | A model nobody has. |\n\
| `I414` | warning | A sound nobody has. |\n\
| `I415` | warning | A texture nobody has. |\n\
| `I416` | warning | A particle effect nobody has. |\n\
| `I417` | warning | A section nobody has. |\n\
| `I418` | warning | A string id nobody has. |\n\
| `I419` | warning | A motion the HUD model does not have. |\n\
| `I420` | warning | A text with characters the game's code page (cp1251) lacks. |\n\
| `I421` | error | An icon picture that is not there, lies outside the project or is no picture. |\n\
| `I422` | warning | An icon rectangle outside its texture. |\n\
| `I423` | warning | A recipe's tool or part that is not there. |\n\
| `I424` | warning | A config a row names that is not there, or lacks the section. |\n\
| `I425` | warning | A death loot community that is not there. |\n\
| `I426` | note | No game is linked: what only the game knows was not looked up. |\n\
| `I427` | error | A value outside what its key takes, or a part the game cannot read: the whole item is left out. |\n\
| `I428` | warning | A config of the module that does not compose. |\n\
| `I429` | warning | A new item's HUD section without a base, whose bases name no HUD section either. |\n\
| `I430` | warning | A new item's death loot without a count: the death manager never spawns it. |\n\
| `I431` | warning | A use scene's `like` on an item that has a scene already: the game keeps the scene it has. |\n";

/// The reference of the item kind, as Markdown.
#[must_use]
pub fn reference() -> String {
    let mut out = String::new();
    out.push_str("# Items (`.itemasset`)\n\n");
    out.push_str(
        "This page is generated from the editor's own tables of the kind's keys (`crates/gz-items/src/fields.rs`): \
the reader, the check and this reference are one list. A test of `gz-items` fails when this file is not what the \
tables say.\n\n",
    );
    out.push_str(INTRO);
    out.push_str(EXAMPLES);
    out.push_str(RULES);
    out.push_str("## The keys\n\n");
    table(&mut out, TOP);
    out.push_str("\n## `icon`\n\n");
    table(&mut out, ICON);
    out.push_str("\n## `hud`\n\n");
    table(&mut out, HUD);
    out.push_str("\n## A row of `sections`\n\n");
    table(&mut out, SECTION);
    out.push_str("\n## A row of `craft`\n\n");
    table(&mut out, CRAFT);
    out.push_str("\n## `loot`\n\n");
    table(&mut out, LOOT);
    out.push_str("\n## A row of `sold`\n\n");
    table(&mut out, SOLD);
    out.push_str("\n## `scene` as a table\n\n");
    table(&mut out, SCENE);
    out.push_str("\n## A row of `lists`\n\n");
    table(&mut out, LIST);
    out.push('\n');
    out.push_str(CODES);
    out
}

/// The catalog as the reference lists it after the format: the families, then the keys of each
/// scope by group.
#[must_use]
pub fn catalog_reference(c: &Catalog, from: &str) -> String {
    let mut out = format!("\n## The catalog of {from}\n\n");
    if c.families.is_empty() && c.keys.is_empty() {
        out.push_str("No catalog: neither the linked game nor the editor has `configs\\item_catalog.ltx`.\n");
        return out;
    }
    out.push_str("| Family | Title | Classes | Starts from |\n|---|---|---|---|\n");
    for f in &c.families {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            f.id,
            esc(&f.title),
            esc(&f.classes.join(", ")),
            esc(&f.like.join(", "))
        ));
    }
    for scope in &c.scopes {
        let keys: Vec<_> = c.keys.iter().filter(|k| k.scope == scope.id).collect();
        if keys.is_empty() {
            continue;
        }
        out.push_str(&format!("\n### {} (`{}`)\n\n", scope.title, scope.id));
        if !scope.help.is_empty() {
            out.push_str(&format!("{}\n\n", scope.help));
        }
        out.push_str(
            "| Key | Type | Group | Families | Required | Default | Means |\n|---|---|---|---|---|---|---|\n",
        );
        for k in keys {
            let name = k.pattern.as_ref().map_or_else(|| k.key.clone(), |p| p.text.clone());
            let mut ty = k.ty.word().to_owned();
            if !k.values.is_empty() {
                ty.push_str(&format!(" ({})", k.values.join(", ")));
            }
            if !k.unit.is_empty() {
                ty.push_str(&format!(", {}", k.unit));
            }
            let group = c.group(&k.group).map_or(k.group.as_str(), |g| g.title.as_str());
            let fam = if k.families.is_empty() { "every".to_owned() } else { k.families.join(", ") };
            out.push_str(&format!(
                "| `{name}` | {} | {} | {} | {} | {} | {} |\n",
                esc(&ty),
                esc(group),
                esc(&fam),
                esc(&if !k.required {
                    String::new()
                } else if k.required_classes.is_empty() {
                    "yes".to_owned()
                } else {
                    k.required_classes.join(", ")
                }),
                esc(k.default.as_deref().unwrap_or("")),
                esc(if k.help.is_empty() { &k.title } else { &k.help })
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::LEVELS;

    fn file() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/assets/ITEM.md")
    }

    /// `GRIZZLY_WRITE_REFERENCE=1 bash tools/rust/cg.sh test -p gz-items reference` writes it.
    #[test]
    fn the_reference_document_is_what_the_tables_say() {
        let path = file();
        if std::env::var("GRIZZLY_WRITE_REFERENCE").is_ok_and(|v| v == "1") {
            std::fs::write(&path, reference()).unwrap_or_default();
        }
        let written = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        assert!(
            written == reference(),
            "{} is not what the tables say: run the test with GRIZZLY_WRITE_REFERENCE=1",
            path.display()
        );
    }

    #[test]
    fn every_key_has_its_meaning_and_its_row() {
        let text = reference();
        for (_, fields) in LEVELS {
            for f in *fields {
                assert!(!f.help.is_empty() && text.contains(&format!("| `{}` |", f.key)), "{}", f.key);
            }
        }
    }

    #[test]
    fn the_known_keys_are_the_tables() {
        for (pattern, fields) in LEVELS {
            let known =
                crate::model::KNOWN.iter().find(|(p, _)| p == pattern).map(|(_, k)| *k).unwrap_or_default();
            assert_eq!(known, crate::fields::keys(fields).as_slice(), "{pattern}");
        }
        assert_eq!(crate::model::KNOWN.len(), LEVELS.len());
    }
}
