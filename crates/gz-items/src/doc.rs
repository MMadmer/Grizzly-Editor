//! The references of the item kind and of the three table kinds, made from the tables the
//! readers use: `docs/assets/ITEM.md` is [`reference`], and `asset.reference kind=item` answers
//! it with the linked game's catalog after it ([`catalog_reference`]); `TRADER.md`, `LOOT.md`
//! and `RECIPES.md` are [`table_reference`].

use crate::catalog::Catalog;
use crate::fields::{DROP, Field, HUD, ICON, LOOT, RECIPE, RECIPES, SCENE, SECTION, STOCK, TOP, TRADE};
use crate::tables::TableKind;

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
it is; the game reads it when its configs come up and makes the sections, the strings and the use scene itself. \
It is plain data like every asset (`docs/ASSETS.md`): `return { ... }` of literals, a key the game does not read \
is passed by, a key the editor does not know is kept through every save and noted once.\n\n\
**An item says what the item is, and nothing of another thing** (`docs/ASSETS.md`, \"One thing, one place\"). Who \
sells it is the trader's to say (`TRADER.md`), who carries it the loot table's (`LOOT.md`), what makes it a \
recipe's (`RECIPES.md`): each is a file of its own, and the item shows it only as a fact - `items.info` answers \
`found`, the Items tab lists it under Where It Is Found with the file that says it. `sold`, `loot`, `craft` and \
`lists` are no keys of an item: the check refuses them by name (`I432`) and the game never reads them.\n\n";

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
  scene = { like = \"vodka\", time = 7200, fit = \"cut\" },\n\
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
when `hud` names no base; the `sections`; then the use scene, lines of `items\\items\\animations_settings.ltx`: \
`sound` is `snd`, `camera` `cam`, `time` `tm`, `helmet` `helm`, `fit` `fit`, and the keys of `scene.set` as they \
are. A template (`abstract = true`) is its section and nothing else. Of a change: the same keys on the section \
it edits, its HUD section changed through the section's own `hud` key. **Any malformed part refuses the whole \
item**: a key no config can hold, a value with a line break.\n\n\
## Order\n\n\
A module's characters, then its items - the new sections first (an item made from another item of the module \
after that one, else in the order of their paths), then the changes in the order of their paths -, then its \
traders, loot tables and recipes, then its `configs\\xms\\*.ltx`, then its `patch\\*.ltxp`: what the module's own \
overlays and patches say of an item still counts. A section an item makes that the module's overlay defines \
again is merged, the later one winning - its own sections and a scene's HUD section. **A section has one file**: \
a row of `sections` that makes or changes the section another item asset of the module is about is refused \
(`I433`) - what changes an item is written in that item's file. \
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
atlas. An icon is its texture and its rectangle together: a section that states its own rectangle - a variant \
with its own cells - keeps its texture when its base gets a picture.\n\n\
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
| `I426` | note | No game is linked: what only the game knows was not looked up. |\n\
| `I427` | error | A value outside what its key takes, or a part the game cannot read: the whole item is left out. |\n\
| `I428` | warning | A config of the module that does not compose. |\n\
| `I429` | warning | A new item's HUD section without a base, whose bases name no HUD section either. |\n\
| `I431` | warning | A use scene's `like` on an item that has a scene already: the game keeps the scene it has. |\n\
| `I432` | error | A key that is another file's to say: `sold` (a trader's), `loot` (a loot table's), `craft` (a recipe's), `lists` (a patch's). |\n\
| `I433` | error | A row of `sections` that makes or changes the section of another item asset of the module. |\n\
| `I434` | warning | A section the item makes (`section`, a row of `sections`) that is not named after the module: the module's id is no word of it. |\n";

const TABLE_CODES: &str = "\
## Problems\n\n\
`asset.check` names every problem at its row; the build refuses a module with an error, with the first one. The \
game takes a file whole or not at all: an error of its shape leaves the whole file out.\n\n\
| Code | Severity | What |\n|---|---|---|\n\
| `I400` | error | The file does not read. |\n\
| `I412` | warning | A chance outside 0 to 1. |\n\
| `I417` | warning | An item a row names that is no section of the game or of the module. |\n\
| `I423` | warning | A recipe's result, tool or part that is not there. |\n\
| `I424` | warning | A trader whose config is not there, or lacks the stock section a row names. |\n\
| `I425` | warning | A death loot community that is not there. |\n\
| `I426` | note | No game is linked: what only the game knows was not looked up. |\n\
| `I427` | error | A value outside what its key takes, or a row the game cannot read. |\n\
| `I430` | warning | A drop of an item the module makes with no count in any loot table: the death manager never spawns it. |\n";

const OWNERSHIP: &str = "\
**A file says what its thing is, and nothing of another thing** (`docs/ASSETS.md`, \"One thing, one place\"): the \
rows are written here, in the file of the side that owns them, and never in an item (`ITEM.md`). The item shows \
them as a fact - `items.info` answers `found`, the Items tab lists them under Where It Is Found with this file to \
open. Mod > Economy edits the three kinds side by side and saves as it is edited; `economy.list` answers every \
file with its rows and problems. The module ships the file as it is and the game reads it after the module's \
items, so a row may name an item the module makes.\n\n";

/// The reference of a table kind, as Markdown.
#[must_use]
pub fn table_reference(kind: TableKind) -> String {
    let mut out = String::new();
    let (title, what, top, row, row_title, example, more) = match kind {
        TableKind::Trade => (
            "# Traders (`.tradeasset`)",
            "One file anywhere in the module is what the module puts on the shelf of ONE trader of the game. Several \
files may speak of one trader; they are read in the order of their paths, a later row of the same item and stock \
section winning. A character of the module is no such trader: its shop is `trade` of its own `.npcasset` \
(`CHARACTER.md`).",
            TRADE,
            STOCK,
            "A row of `stock`",
            "\
```lua\n\
return {\n\
  trade = 1,\n\
  trader = \"trade_military\",       -- misc\\trade\\trade_military.ltx\n\
  stock = {\n\
    { item = \"wpn_sil_diy\", count = 1, chance = 0.5 },                         -- [supplies_1] wpn_sil_diy = 1, 0.5\n\
    { item = \"ammo_9x39_ap\", supplies = { \"supplies_2\", \"supplies_3\" }, count = 40 },\n\
    { item = \"vodka\", count = 0 },                                             -- taken off the shelf\n\
  },\n\
}\n\
```\n\n",
            "\
## What the game makes\n\n\
A row is a line of the trader's config: `[<supplies>] <item> = <count>, <chance>` for each stock section it names, \
`supplies_1` when it names none; a count of 0 takes the line away. They are patches of that config, as a `.ltxp` \
`#file` makes them: a line of a stock section reaches the sections that inherit it (the game's traders stock \
their later levels over the first), and a config the game writes is read as written. What a trader pays and asks \
for an item, and which goods category of `misc\\trade\\trade_presets.ltx` an item is in, no trader file says: \
that is a patch of those configs (`patch\\*.ltxp`).\n\n",
        ),
        TableKind::Loot => (
            "# Loot tables (`.lootasset`)",
            "One file anywhere in the module is a table of what the dead carry: an item, how many, and the chance - for \
everybody and for a community or a community and a rank. A module may keep one table or several.",
            LOOT,
            DROP,
            "A row of `drops`",
            "\
```lua\n\
return {\n\
  loot = 1,\n\
  drops = {\n\
    { item = \"wpn_sil_diy\", count = { 0, 1 }, chance = 0.01, by = { bandit = 0.03, stalker_veteran = 0.02 } },\n\
  },\n\
}\n\
```\n\n",
            "\
## What the game makes\n\n\
`count` is the item's line of `misc\\death_generic.ltx [item_count]` (`{ min, max }` as `min, max`); `chance` its \
line of `misc\\death_items_by_communities.ltx [base]`, which every community and rank inherits; each `by` entry \
its line of that community's own section. They are patches of those configs, as a `.ltxp` `#file` makes them. An \
item of the game has its count already; one the module makes needs a row that gives it one, or the death manager \
never spawns it (`I430`).\n\n",
        ),
        TableKind::Recipes => (
            "# Recipes (`.recipeasset`)",
            "One file anywhere in the module is a list of recipes: what is made, at which tool or workbench, from one \
to four parts. A module may keep one file or several.",
            RECIPES,
            RECIPE,
            "A row of `recipes`",
            "\
```lua\n\
return {\n\
  recipe = 1,\n\
  recipes = {\n\
    { makes = \"wpn_sil_diy\", at = \"itm_repairkit_tier_1\", parts = { \"jar\", \"textile_patch_m\", \"rope\", \"flashlight_broken\" } },\n\
    { makes = \"ammo_9x18_fmj\", count = 40, at = \"verstak_ammo\", parts = { \"ammo_bad\", \"ammo_bad\" } },\n\
  },\n\
}\n\
```\n\n",
            "\
## What the game makes\n\n\
One line a recipe in the system config's `[xms_craft]`: `<n> = <at>, <makes>, <count>, <part>, ...`, `n` a \
zero-padded number after the highest the section holds, so the recipes are listed in the order they are written, \
after the craft window's own. `at` is the section the craft window is opened with: a tool (`itm_repairkit_tier_1`, \
`itm_drugkit`) or a workbench. A result whose section holds `ammo_` is made as that many rounds, in boxes.\n\n",
        ),
    };
    out.push_str(title);
    out.push_str("\n\n");
    out.push_str(
        "This page is generated from the editor's own tables of the kind's keys (`crates/gz-items/src/fields.rs`): \
the reader, the check and this reference are one list. A test of `gz-items` fails when this file is not what the \
tables say.\n\n",
    );
    out.push_str(what);
    out.push_str(" It is plain data like every asset (`docs/ASSETS.md`).\n\n");
    out.push_str(OWNERSHIP);
    out.push_str(example);
    out.push_str(more);
    out.push_str("## The keys\n\n");
    table(&mut out, top);
    out.push_str(&format!("\n## {row_title}\n\n"));
    table(&mut out, row);
    out.push('\n');
    out.push_str(TABLE_CODES);
    out
}

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
    out.push_str("\n## `scene` as a table\n\n");
    table(&mut out, SCENE);
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

    /// `GRIZZLY_WRITE_REFERENCE=1 bash tools/rust/cg.sh test -p gz-items reference` writes them.
    #[test]
    fn the_reference_document_is_what_the_tables_say() {
        let docs = [
            (file(), reference()),
            (file().with_file_name("TRADER.md"), table_reference(TableKind::Trade)),
            (file().with_file_name("LOOT.md"), table_reference(TableKind::Loot)),
            (file().with_file_name("RECIPES.md"), table_reference(TableKind::Recipes)),
        ];
        for (path, text) in docs {
            if std::env::var("GRIZZLY_WRITE_REFERENCE").is_ok_and(|v| v == "1") {
                std::fs::write(&path, &text).unwrap_or_default();
            }
            let written = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
            assert!(
                written == text,
                "{} is not what the tables say: run the test with GRIZZLY_WRITE_REFERENCE=1",
                path.display()
            );
        }
    }

    #[test]
    fn every_key_has_its_meaning_and_its_row() {
        let text = reference();
        for (_, fields) in LEVELS {
            for f in *fields {
                assert!(!f.help.is_empty() && text.contains(&format!("| `{}` |", f.key)), "{}", f.key);
            }
        }
        for kind in TableKind::ALL {
            let text = table_reference(kind);
            for (_, fields) in crate::tables::levels(kind) {
                for f in *fields {
                    assert!(
                        !f.help.is_empty() && text.contains(&format!("| `{}` |", f.key)),
                        "{kind:?} {}",
                        f.key
                    );
                }
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
