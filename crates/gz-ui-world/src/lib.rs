//! The Characters, Items and World Rules tabs (docs/UI_DESIGN.md, section 6, "Characters",
//! "Items" and "World Rules"; CLAUDE.md, "Characters" and "The mode's world").
//!
//!
//! - **Characters** ([`characters`]): the `characters` singleton tab - a list of 260 pt with a
//!   search, dirty dots and Template badges, a splitter, and a form of up to 760 pt: Identity,
//!   Name and Bio (a row per language), Appearance, Profile, Dialogs, the Starting Kit table and
//!   Trade. The form binds to the RAW stored values (`gz_characters::text_raw`), never the
//!   resolved ones; what the base gives shows as a hint or a muted row. Ctrl+S saves through
//!   `Characters::save` (only what changed reaches the file), Delete... asks through
//!   `dlg.delete_character` and runs `file.delete`; unsaved edits ask before any switch.
//!   Validation runs on change, on the pools.
//! - **Items** ([`items`]): the `items` singleton tab - the module's items under New and
//!   Changes, a form of up to 760 pt whose key rows are the linked game's item catalog's keys of
//!   the item's family (`item_model`), and the item as the game shows it. The form binds to what
//!   the file STORES; Ctrl+S saves through `Items::save` (the difference), the + menu's choosers
//!   make the file at once, Delete... asks through `dlg.delete_item`. What the item resolves to
//!   and its problems are worked out on the pools for the item as it is edited
//!   (`Items::check_edit`); the lists the Characters tab lacks are [`item_lists`].
//! - **World Rules** ([`world`]): the `world_rules` singleton tab, saved to `mod.ltx` as it is
//!   edited through `world.set`: Original Game, Player Start and Starting Kit on the surface,
//!   Applies On, Playable Area and Transitions under one folded More; a problems bar and drawer
//!   only when there are problems. Empty World... asks through `dlg.empty_world`.
//! - **The lists** ([`lists`]): what the pickers offer - the linked game's items, communities,
//!   dialogs, portraits and ranks (`gz_nq::Pickers`), its trade categories, the models of
//!   `meshes\actors` of the game and the project - read on the pools into a [`lists::ListsView`]
//!   the frames only look at; a list that is still loading shows busy.
//! - [`item_picker`] - the one item picker both tabs use (a kit row, a stock row, the mode's
//!   start kit).
//! - [`register`] - `characters.view`, `items.view`, `world.view`; [`install`] - the two tabs and their Mod menu
//!   rows (at `gz_ui_mod::mod_menu_slot`).
//!
//! What comes from a list is picked with the kit's `Combo` and `PickList` ([`picker::list_picker`],
//! [`picker::row_combo`]). Widgets the kit lacks, local to this crate:
//! [`picker::local_multi_combo`] (ticks over names, a name the list lacks kept),
//! [`portraits::local_portrait_gallery`] (the faces of every portrait sheet),
//! [`local::local_number_text`] (a number that may be empty, with a hint),
//! [`local::local_value_text`] (an own value or the inherited one as its hint),
//! [`local::local_add_field`], [`local::local_value_combo`], [`local::local_dirty_dot`], [`local::local_problems_drawer`], [`local::local_cell`] and
//! [`local::drawer_height`]. The levels World Rules picks are the kit's `PickList`.
//!
//! Nothing here writes a file or computes inside a frame: every edit is a command or a service
//! call on the pools (`Characters::save`, `file.delete`, `world.set`, `scene.place`,
//! `viewport.camera`, `preview.model`); what a frame shows comes from snapshots and from data read
//! on the pools.

#![forbid(unsafe_code)]

mod call;
mod char_form;
pub mod characters;
mod item_form;
mod item_hands;
pub mod item_lists;
pub mod item_model;
mod item_view_cmd;
pub mod items;
pub mod lists;
pub mod local;
pub mod picker;
pub mod portraits;
mod view_cmd;
pub mod world;

use gz_app::{Action, ShellBuilder, args};
use gz_editor::Registry;
use gz_ui::Icon;

pub use picker::item_picker;

/// Plugs the Characters and World Rules tabs and their Mod menu rows into the shell.
pub fn install(b: &mut ShellBuilder) {
    b.translation(include_str!("ru.lua"));
    b.translation(include_str!("items.ru.lua"));
    characters::install(b);
    items::install(b);
    world::install(b);
    b.action(
        Action::new(gz_ui_mod::menu::CHARACTERS, "Characters", "Mod", "ui.tab")
            .icon(Icon::Character)
            .args(|_| args(&[("action", "open"), ("kind", characters::TAB)]))
            .enabled(gz_ui_mod::menu::with_project),
    );
    b.action(
        Action::new(gz_ui_mod::menu::ITEMS, "Items", "Mod", "ui.tab")
            .icon(Icon::Item)
            .args(|_| args(&[("action", "open"), ("kind", items::TAB)]))
            .enabled(gz_ui_mod::menu::with_project),
    );
    b.action(
        Action::new(gz_ui_mod::menu::WORLD_RULES, "World Rules", "Mod", "ui.tab")
            .icon(Icon::World)
            .args(|_| args(&[("action", "open"), ("kind", world::TAB)]))
            .enabled(gz_ui_mod::menu::with_project),
    );
    gz_ui_mod::menu::row(b, gz_ui_mod::menu::CHARACTERS, None);
    gz_ui_mod::menu::row(b, gz_ui_mod::menu::ITEMS, None);
    gz_ui_mod::menu::row(b, gz_ui_mod::menu::WORLD_RULES, None);
}

/// The commands of this package: `characters.view`, `items.view`, `world.view`.
pub fn register(reg: &mut Registry) {
    view_cmd::register(reg);
    item_view_cmd::register(reg);
}

#[cfg(test)]
mod i18n_tests {
    #[test]
    fn the_catalog_reads_with_the_kits_and_the_shells() {
        let c = gz_ui::i18n::Catalog::parse(&[
            gz_ui::i18n::KIT_RU,
            gz_app::lang::SHELL_RU,
            include_str!("ru.lua"),
            include_str!("items.ru.lua"),
        ])
        .expect("the catalogs read");
        let p = gz_ui::i18n::problems(&c);
        assert!(p.is_empty(), "{}", p.join("\n"));
    }

    /// The window's catalogs as the Items tab reads them.
    fn catalog() -> gz_ui::i18n::Catalog {
        gz_ui::i18n::Catalog::parse(&[
            gz_ui::i18n::KIT_RU,
            gz_app::lang::SHELL_RU,
            include_str!("ru.lua"),
            include_str!("items.ru.lua"),
        ])
        .expect("the catalogs read")
    }

    /// Every title, unit and help of the bundled item catalog has its entry, under the context the
    /// tab looks it up by, and no entry of those contexts names a text the catalog lacks.
    #[test]
    fn every_text_of_the_item_catalog_has_its_entry() {
        let ltx = include_str!("../../../gamedata/configs/item_catalog.ltx");
        let mut wanted: Vec<(String, &str)> = Vec::new();
        for line in ltx.lines().map(str::trim) {
            let Some((key, value)) = line.split_once('=') else { continue };
            let ctx = match key.trim() {
                "title" | "unit" => "item",
                "help" => "item_help",
                _ => continue,
            };
            wanted.push((value.trim().to_owned(), ctx));
        }
        assert!(wanted.len() > 1000, "the catalog reads: {} texts", wanted.len());
        let c = catalog();
        let have: std::collections::HashSet<(&str, &str)> =
            c.entries().iter().filter_map(|e| e.ctx.as_deref().map(|ctx| (e.en.as_str(), ctx))).collect();
        let missing: Vec<String> = wanted
            .iter()
            .filter(|(en, ctx)| !have.contains(&(en.as_str(), *ctx)))
            .map(|(en, ctx)| format!("{ctx}: {en}"))
            .collect();
        assert!(missing.is_empty(), "{} texts without an entry:\n{}", missing.len(), missing.join("\n"));
        let stale: Vec<String> = have
            .iter()
            .filter(|(en, ctx)| {
                matches!(*ctx, "item" | "item_help") && !wanted.iter().any(|(w, wc)| w == en && wc == ctx)
            })
            .map(|(en, ctx)| format!("{ctx}: {en}"))
            .collect();
        assert!(stale.is_empty(), "entries the catalog no longer says:\n{}", stale.join("\n"));
    }

    /// A sample sentence of every check code, written as the check writes it, shows translated
    /// the way the tab shows a problem.
    #[test]
    fn every_sentence_of_the_item_check_translates() {
        use gz_items::model::Issue;
        let ctx = egui::Context::default();
        gz_ui::i18n::install(&ctx, Some(std::sync::Arc::new(catalog())));
        let (key, title, h, n) = ("cost", "Cost", "wpn_qa_hud", "wpn_qa");
        let loop_tail = ": the bases of the module's items lead back to [wpn_qa]";
        let messages: Vec<(&str, String)> = vec![
            ("I400", "items/a.itemasset: line 3: '=' expected".into()),
            ("I400", "items/a.itemasset: an item file returns one table".into()),
            ("I401", "a new item starts from a base: write base (or edit, to change an item that exists)".into()),
            ("I402", "a change names its section with edit alone: id, section, base and abstract make a new item".into()),
            ("I403", "the id '9qa' is not a-z, 0-9 and _, up to 48, not starting with a digit".into()),
            ("I404", "the section [Qa Ak] is not a-z, 0-9, _, . and -, under 64 characters".into()),
            ("I406", "another item makes the section 'qa_ak' too: items/b.itemasset, items/c.itemasset".into()),
            ("I407", format!("the section [{n}] exists already: write it as a change, with edit")),
            ("I407", format!("the section [{n}_hud] exists already")),
            ("I407", format!("the section [{n}_ammo] exists already: change it with {{ edit = \"{n}_ammo\" }}")),
            ("I408", "an unknown base [wpn_none]".into()),
            ("I408", "an unknown base [wpn_none_hud] of its hud".into()),
            ("I408", "an unknown base [wpn_none] of [qa_part]".into()),
            ("I409", "it changes [wpn_none], which is not there".into()),
            ("I409", "it changes the hud of [wpn_ak74], which names no hud section".into()),
            ("I410", format!("an unknown base [qa_b]{loop_tail}")),
            ("I410", format!("an unknown base [qa_b_hud] of its hud{loop_tail}")),
            ("I410", format!("an unknown base [qa_b] of [qa_part]{loop_tail}")),
            ("I410", format!("it changes [wpn_none], which is not there{loop_tail}")),
            ("I410", format!("the section [{n}] exists already: write it as a change, with edit{loop_tail}")),
            ("I410", format!("the section [{n}_hud] exists already{loop_tail}")),
            ("I410", format!("the section [{n}_ammo] exists already: change it with {{ edit = \"{n}_ammo\" }}{loop_tail}")),
            ("I411", format!("the game stops without '{key}' ({title}), and nothing the item starts from gives it: set it (`set = {{ {key} = ... }}`)")),
            ("I411", format!("the game stops without 'item_visual' (First-Person Model) in the HUD section '{h}': set it in `hud.set`")),
            ("I412", format!("'{key}' is \"x\": it takes true or false")),
            ("I412", format!("'{key}' is \"x\": it takes a whole number")),
            ("I412", format!("'{key}' is \"x\": it takes a number")),
            ("I412", "'fire_point' is \"1, 2\": it takes a vector: 3 numbers separated by commas".into()),
            ("I412", "'light_color' is \"red\": it takes a colour: r, g, b or r, g, b, a".into()),
            ("I412", "'hit_type' is \"x\": it takes one of fire_wound, wound, explosion".into()),
            ("I412", "'use1_functor' is \"x\": it takes a script function: module.function".into()),
            ("I412", "a chance of 1.5: a chance is from 0 to 1".into()),
            ("I413", "'visual' names the model 'weapons\\qa', which neither the game nor the project has".into()),
            ("I414", "'snd_shoot' names the sound 'weapons\\qa', which neither the game nor the project has".into()),
            ("I415", "'scope_texture' names the texture 'wpn\\qa', which neither the game nor the project has".into()),
            ("I415", "the icon's texture 'ui\\qa' is neither the game's nor the project's".into()),
            ("I416", "'flame_particles' names the particle effect 'qa\\fire', which neither the game nor the project has".into()),
            ("I417", "'ammo_class' names the section 'ammo_qa', which neither the game nor the project has".into()),
            ("I417", "'qa_bread' has no use scene in configs\\items\\items\\animations_settings.ltx: there is nothing to start from".into()),
            ("I418", "'inv_name' names the string id 'st_qa', which neither the game nor the project has".into()),
            ("I419", "'anm_idle' names the motion 'idle_qa', which the item's HUD model does not have".into()),
            ("I420", "the eng text of 'name' has characters the game's code page (cp1251) lacks: the player reads '?' there".into()),
            ("I420", "'qa_key' has characters the game's code page (cp1251) lacks: the game reads '?' there".into()),
            ("I421", "the icon picture lies outside the project: ../qa.png".into()),
            ("I421", "the icon picture is not there: qa.png (beside the asset)".into()),
            ("I421", "the icon picture cannot be read: qa.png: unreadable".into()),
            ("I422", "the rectangle 10, 2, 2 x 1 cells reaches past the texture 'ui\\qa' (512 x 512 px, 10 x 10 cells)".into()),
            ("I423", "the recipe's tool 'qa_tool' is no section of the game or of the module".into()),
            ("I423", "the recipe's part 'qa_part' is no section of the game or of the module".into()),
            ("I424", "the trader's config 'qa_trader' is neither the game's nor the project's (configs\\misc\\trade\\qa_trader.ltx)".into()),
            ("I424", "the trader's config configs\\misc\\trade\\qa.ltx has no section [supplies_qa]".into()),
            ("I424", "the config configs\\items\\qa.ltx is neither the game's nor the project's".into()),
            ("I425", "'qa_band' is no community of the game, nor a community and one of its ranks".into()),
            ("I426", "no game is linked: the models, sounds, textures, particle effects, string ids, communities and traders' files the item names were not looked up".into()),
            ("I427", format!("'{key}' takes a string")),
            ("I427", "'abstract' takes true or false".into()),
            ("I427", format!("'{key}' takes a number")),
            ("I427", "'w' is 0.5; it takes a whole number from 1 to 16".into()),
            ("I427", "'qa key' cannot be a key of a config".into()),
            ("I427", "base names a section no config can hold".into()),
            ("I427", "hud.base names a section no config can hold".into()),
            ("I427", "the base names a section no config can hold".into()),
            ("I427", "a recipe's at names a section no config can hold".into()),
            ("I427", "a part of a recipe names a section no config can hold".into()),
            ("I427", "a row of lists names a section no config can hold".into()),
            ("I427", "base is a string or a list of them".into()),
            ("I427", "the base is a string or a list of them".into()),
            ("I427", "a part of a recipe is a string or a list of them".into()),
            ("I427", "unset, a list of keys, is a string or a list of them".into()),
            ("I427", "name is a string or { language = text }".into()),
            ("I427", "set takes key = value".into()),
            ("I427", "the value of cost holds characters no config can carry".into()),
            ("I427", "the value of cost is no string, number, boolean or list of them".into()),
            ("I427", "add takes key = member or { members }".into()),
            ("I427", "add.ammo_class is a member or a list of them".into()),
            ("I427", "add.ammo_class holds a member no list can carry".into()),
            ("I427", "icon is { texture, x, y, w, h }, in cells, or a picture beside the asset".into()),
            ("I427", "icon.texture names a texture".into()),
            ("I427", "icon is { texture, x, y, w, h }, in cells: x and y are its place".into()),
            ("I427", "loot is { count, chance, by }".into()),
            ("I427", "loot.count is a number or { min, max }".into()),
            ("I427", "loot.by is { <community or community_rank> = chance }".into()),
            ("I427", "a row of sold names the trader's config: by".into()),
            ("I427", "sold.by is a trader's config, or a config's path under configs".into()),
            ("I427", "a row of sold is { by, supplies, count, chance }".into()),
            ("I427", "sold is a list of { by, supplies, count, chance }".into()),
            ("I427", "scene.like names the item whose scene it takes".into()),
            ("I427", "scene is the item whose scene it takes, or { like, sound, camera, time, helmet, anm, hud_anm, hud }".into()),
            ("I427", "scene.sound is a value no config can carry".into()),
            ("I427", "scene.hud is { base, set, unset, add, drop }".into()),
            ("I427", "hud of a change changes the section the item's hud names: base belongs to a new item's hud".into()),
            ("I427", "hud is { base, set, unset, add, drop }".into()),
            ("I427", "sections is a list of { name, base, set } and { edit, set }".into()),
            ("I427", "a row of sections has a name (a section it makes) or edit (one it changes)".into()),
            ("I427", "[wpn_ak74_hud] exists and keeps its parents: base belongs to a section a row makes".into()),
            ("I427", "craft is a list of { at, count, parts }".into()),
            ("I427", "a recipe is { at, count, parts }".into()),
            ("I427", "a recipe names the tool or workbench it is made at: at".into()),
            ("I427", "a recipe takes one to four parts".into()),
            ("I427", "lists is a list of { file, section, key, value } and { file, section, key, add | drop }".into()),
            ("I427", "a row of lists is { file, section, key, value }".into()),
            ("I427", "a row of lists holds a value no config can carry".into()),
            ("I427", "a row of lists names a config by its path under configs".into()),
            ("I427", "a row of lists names its section".into()),
            ("I427", "a row of lists names a key a config can hold".into()),
            ("I427", "a row of lists sets a value, adds to a list or drops from one: one of them".into()),
            ("I427", "a row of lists holds a member no list can carry".into()),
            ("I427", "made by a newer Grizzly Editor (item = 2; this one reads up to 1): update the editor to open it".into()),
            ("I428", "the module's config configs\\items\\qa.ltx does not compose, so what the items resolve to may differ: line 4: no section".into()),
            ("I429", format!("the HUD section [{h}] has no base, and the item's bases name no HUD section: it holds only what hud.set says (hud.base names one)")),
            ("I430", "the death loot has no count (loot.count): the death manager never spawns an item without one".into()),
        ];
        // three plain English words of the sentence in a row: a part of it left in English
        let english = |en: &str, tr: &str| {
            let words = |t: &str| -> Vec<String> { t.split_whitespace().map(str::to_owned).collect() };
            let (en, tr) = (words(en), words(tr));
            tr.windows(3).any(|w| {
                w.iter().all(|x| x.chars().all(|c| c.is_ascii_alphabetic())) && en.windows(3).any(|e| e == w)
            })
        };
        let mut untranslated = Vec::new();
        for &(code, ref message) in &messages {
            let shown = crate::item_model::sentence(&Issue::warning(code, Vec::new(), message.as_str()));
            let ru = gz_ui::Tx::ui(shown.as_str()).text(&ctx);
            if ru == shown || english(&shown, &ru) {
                untranslated.push(format!("{code}: {shown}\n  {ru}"));
            }
        }
        assert!(
            untranslated.is_empty(),
            "sentences the catalog does not translate:\n{}",
            untranslated.join("\n")
        );
    }
}
