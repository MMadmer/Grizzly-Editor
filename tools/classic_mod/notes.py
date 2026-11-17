"""PORT_NOTES.md for the mod's author and the long lists under port\\."""
from __future__ import annotations

import collections
import os
import re

KEY_FAMILIES = [
    ("price", r"^(cost|trade_.*|price.*)$"),
    ("upgrades", r"^(upgrades|installed_upgrades|upgrade_scheme)$"),
    ("icon/inventory", r"^(inv_grid_.*|icons_texture|inv_name.*|description|inv_weight|weight)$"),
    ("ammo lists", r"^(ammo_class|grenade_class|ammo_mag_size|ammo_elapsed|launch_speed)$"),
    ("addons", r"^(scope.*|silencer.*|grenade_launcher.*|addon.*|scopes_sect|.*_x$|.*_y$)$"),
    ("sounds", r"^snd_"),
    ("animations", r"^(anm_|anim_|lanim)"),
    ("visual/hud link", r"^(visual|hud|item_visual|player_hud_section|attach.*visual)$"),
    ("hud geometry", r"^(hands?_position.*|hands?_orientation.*|aim_.*|item_position|item_orientation|.*_hud_offset.*|"
                     r"fire_point.*|shell_point.*|fire_bone.*|shell_bone.*|zoom_.*|gl_.*|lowered_.*|.*position_16x9|"
                     r".*orientation_16x9|hud_fov|base_.*|strafe_.*|.*bone.*|freelook.*|lean.*)$"),
    ("damage/ballistics", r"^(hit_.*|fire_distance|bullet_speed|rpm.*|k_.*|buck_shot|wm_size|tracer.*|fire_dispersion.*|"
                          r"air_resistance.*|impair|explode.*|blast.*|frags.*|fire_modes|condition_.*|misfire_.*|cam_.*|"
                          r"control_inertion_factor|crosshair_inertion|PDM_.*|pdm_.*)$"),
    ("particles/light", r"(particles|light_|flame_|smoke_|shell_particles)"),
    ("eat/use", r"^(eat_.*|boost_.*|use.*|max_uses|remove_after_use|wounds_heal_perc|satiety.*|radiation.*|power_loss|"
                r"health.*|bleeding.*|alcohol|psy.*|thirst.*|sleep.*|tier|kind|repair_type|.*_restore.*)$"),
    ("protection", r"(protection|immunit|hit_fraction|bones_koeff|_restore_speed|artefact_count)"),
]


def key_family(k):
    for name, rx in KEY_FAMILIES:
        if re.search(rx, k, re.I):
            return name
    return "other"


def _show(v):
    from compose import ABSENT
    from ltx import REMOVED
    if v is ABSENT or v is REMOVED:
        return "(none)"
    return "(bare)" if v is None else v


def _table(rows, head):
    out = ["| " + " | ".join(head) + " |", "|" + "---|" * len(head)]
    out += ["| " + " | ".join(str(c) for c in r) + " |" for r in rows]
    return out


def write(cv):
    it = cv.intent
    port = lambda name, lines: cv.write_text(os.path.join("port", name), "\n".join(lines) + "\n")  # noqa: E731

    # ---- the long lists
    port("bases.txt", ["# file\tbase\tdistance\tevery version"] + cv.base_rows)
    port("conflicts.txt", ["# keys the mod and the game both changed since the base: the mod's value is in the module",
                           "# section\tkey\tbase\tgame\tmod"] +
         [f"{s}\t{k}\t{_show(b)}\t{_show(g)}\t{_show(m)}" for s, k, b, g, m in sorted(it.conflicts, key=lambda c: (c[0], c[1]))] +
         ["", "# configs opened by name (#file)"] +
         [f"{f}\t{s}\t{k}\t{_show(b)}\t{_show(g)}\t{_show(m)}" for f, s, k, b, g, m in cv.outside_conflicts])
    port("deleted_kept.txt", ["# sections the mod's files dropped from their base: the game keeps them"] + sorted(it.deleted_by_mod) +
         ["", "# configs opened by name"] + [f"{f}\t{s}" for f, s in cv.outside_deleted])
    roll = collections.Counter(key_family(k) for _, k in it.rollbacks)
    port("rollbacks.txt", ["# values the mod only carries from its base (today's game changed them since): not carried",
                           "# section\tkey"] + [f"{s}\t{k}" for s, k in sorted(it.rollbacks)])
    port("lost_sections.txt", ["# sections the game added since the mod's base, which the mod's whole-file copies lack: kept"] +
         sorted(it.lost_game_additions))
    port("game_deleted.txt", ["# sections of the base the game no longer has; the mod changed them, so the module makes them"] +
         sorted(it.game_deleted_mod_changed) + ["", "# the same, unchanged by the mod: not made"] + sorted(it.game_deleted_unchanged))
    port("media.txt", ["# path\tdecision\twhy"] + [f"{r}\t{d}\t{w}" for r, (d, w) in sorted(cv.media_decided.items())])
    port("items.txt", ["# asset\tnew or change\thud\tsections\ticon"] +
         [f"{p}\t{'change' if 'edit' in t else 'new'}\t{'yes' if 'hud' in t else ''}\t{len(t.get('sections') or [])}\t"
          f"{'yes' if 'icon' in t else ''}" for p, t in sorted(cv.asset_tables.items())])
    port("outside.txt", ["# file\tbase\tsections with directives\tsections new\tdirectives\tdeleted by the mod (kept)\tadded by the game (kept)\tconflicts"] +
         ["\t".join(str(x) for x in r) for r in cv.outside_rows])
    port("atlas.txt", cv.atlas_rows)

    # ---- the notes
    rep = cv.rep
    m = rep["media"]
    nitems = rep["items"]
    patch_dirs = sum(sum(len(e[2]) if e[0] == "edit" else len(e[1].own) + 1 for e in v) for v in cv.patches.values())
    file_targets = rep["outside"]["targets"]
    L = [f"# {cv.name}: the port", "",
         f"This project is the classic mod made into a module of Dead Air Refined (`{cv.id}`). It holds what the mod",
         "changes in the game of today and nothing it only copied from the old game it was made over.", "",
         "## What the module holds", ""]
    rows = [("`items\\`", f"{nitems['new']} new items and {nitems['changed']} changes of the game's items, one `.itemasset` each, "
                          "with the HUD section and the mixins each owns"),
            ("`icons\\`", f"{sum(1 for t in cv.asset_tables.values() if 'icon' in t)} pictures the mod painted itself, named by the items"),
            ("`craft\\`", (f"{len(cv.craft_rows)} recipes the mod added to the game's craft script, in one recipes file "
                           f"(`{os.path.basename(cv.recipe_file)}`)") if cv.recipe_file else "no recipe of the mod's own"),
            ("`patch\\*.ltxp`", f"{len(cv.patches)} files of sections no item owns ({patch_dirs} lines), "
                                f"{len(cv.file_blocks)} files of `#file` lines for {file_targets} configs scripts open by name" +
                                (f", one of `#file` lines for {len(cv.sysfile_blocks)} files of the system config a script "
                                 "also reads" if cv.sysfile_blocks else "")),
            ("`gamedata\\configs\\text\\`", ", ".join(f"{lang}: {n} strings" for lang, n in sorted(cv.str_written.items())) or "none"),
            ("`gamedata\\particles\\`", f"{rep['particles']['files'] if rep.get('particles') else 0} effects and groups (`.pe`, `.pg`)"),
            ("`gamedata\\` (media)", f"{m.get('own', {}).get('files', 0)} files of the mod's own ({m.get('own', {}).get('mb', 0)} MB), "
                                     f"{m.get('kept old', {}).get('files', 0)} older game files its work needs"),
            ("`port\\`", "what the next step needs: scripts, XML diffs, use animations, the lists below (not shipped)")]
    L += _table(rows, ["Where", "What"])
    L += ["", "## How it was made", "",
          "Every file the mod replaces was compared with every archived version of it; the closest one is its base.",
          "The mod's work is where the mod differs from its base. Where the mod equals its base, today's game stays.", ""]
    L += _table(sorted(rep["bases"].items(), key=lambda x: -x[1]), ["Base", "Files"])
    L += ["", "The list per file is `port\\bases.txt`."]
    L += ["", "## What was not carried, and why", ""]
    lost = collections.Counter()
    for s in it.lost_game_additions:
        lost[_family(cv.G, s)] += 1
    rows = [("Values the mod only carries from its base", len(it.rollbacks), "today's game changed them since; the game's stay (`port\\rollbacks.txt`)"),
            ("Game sections the mod's whole-file copies lack", len(it.lost_game_additions), "the game added them after the base; they stay (`port\\lost_sections.txt`)"),
            ("Media equal to an older game version", m.get("old game bytes", {}).get("files", 0),
             f"{m.get('old game bytes', {}).get('mb', 0)} MB of the old game, not the mod's work (`port\\media.txt`)"),
            ("Media equal to the game's today", m.get("identical", {}).get("files", 0), "nothing to ship"),
            ("Junk", m.get("junk", {}).get("files", 0), "sources, backups, archives, folder settings (`port\\media.txt`)"),
            ("The icon atlas", m.get("atlas", {}).get("files", 0), "a whole-file copy shows today's items with old art; the mod's own pictures are icon files"),
            ("`particles.xr`", 1 if rep.get("particles") else 0, "its effects of the mod's own are loose files"),
            ("Player state", len(rep["outside"]["state_left_out"]) + sum(1 for r in cv.mod if r in ("localization.ltx",)),
             "the game writes these files: " + ", ".join(rep["outside"]["state_left_out"] + [r for r in cv.mod if r == "localization.ltx"])),
            ("Lines that name nothing", cv.junk_keys, "a dashed line or a bare `:` the parser keeps as a key")]
    L += _table(rows, ["What", "Count", "Why"])
    L += ["", "Rolled-back values by kind:", ""]
    L += _table(roll.most_common(), ["Kind", "Values"])
    L += ["", "Game sections the mod's copies lack, by family:", ""]
    L += _table(lost.most_common(), ["Family", "Sections"])
    L += ["", "## Keys both the mod and the game changed", "",
          f"{len(it.conflicts)} keys in {len({c[0] for c in it.conflicts})} sections (and {len(cv.outside_conflicts)} in configs opened "
          "by name): the game changed them since the base and so did the mod. The module takes the mod's value. Check them: "
          "`port\\conflicts.txt` names the base's, the game's and the mod's value of each.", ""]
    bysec = collections.defaultdict(list)
    for c in it.conflicts:
        bysec[c[0]].append(c[1])
    rows = [(f"`{s}`", ", ".join(f"`{k}`" for k in ks[:8]) + (f" and {len(ks) - 8} more" if len(ks) > 8 else ""))
            for s, ks in sorted(bysec.items())]
    for f, s, k, *_ in cv.outside_conflicts:
        rows.append((f"`{s}` in `{f}`", f"`{k}`"))
    L += _table(rows, ["Section", "Keys"])
    L += ["", "## Sections the mod deleted that stay", "",
          f"{len(it.deleted_by_mod)} sections of the base are missing from the mod's files: "
          + ", ".join(f"`{s}`" for s in sorted(it.deleted_by_mod)) + ". A module does not delete what other mods and saves may use.", ""]
    L += ["## Things to know", ""]
    cls = rep.get("class_changes", [])
    if cls:
        L.append(f"- The module changes the class of {len(cls)} items: " + ", ".join(f"`{s}` ({c})" for s, c in cls) +
                 ". A save holds such an object in its old class's format: the module needs a new game.")
    silent = sorted(s for s, ops in cv.plan.ops.items() for op in ops if op[0] == "set" and op[1] == "use_sound" and op[2] == "none")
    if silent:
        L.append(f"- {len(silent)} consumables get `use_sound = none`: the mod's `item_animations.script` plays their sound. "
                 "Until the next step brings that script, they are used silently.")
    if cv.items_in_script_files:
        L.append("- Items whose section lies in a file a script also opens by name: " +
                 ", ".join(f"`{s}` (`{f}`)" for s, f in cv.items_in_script_files) +
                 ". An item asset changes the system config only; the script reads the file as the game ships it.")
    if cv.sysfile_blocks:
        L.append("- Sections of files the system config is read from and a script opens by name are written as `#file` "
                 "lines (`patch\\" + cv.id + "_system_files.ltxp`): the patch reaches both.")
    if cv.flattened:
        L.append("- The game makes an item's sections only when every parent they name is there before it (a new item waits "
                 "for another new item, never for a change). These were written over their parent's own "
                 "parents, with what they took from it: " +
                 ", ".join(f"`{s}` (over {', '.join(f'`{x}`' for x in now)} instead of {', '.join(f'`{x}`' for x in was)})"
                           for s, was, now in cv.flattened) + ".")
    md = getattr(cv, "motion_done", None)
    if md and md.merged:
        L.append(f"- {len(md.merged)} motion files of the mod are copies of the game's from the game it was made over. "
                 f"Mounted over today's game they would take away the motions the game has put into those files since, "
                 f"and the game stops when one is played: each got the motions it lacks ({sum(md.merged.values())} in "
                 "all), the mod's own motion winning where both have one name.")
    if md and md.renamed:
        L.append(f"- {len(md.renamed)} motion files of the mod are written for other bones than the game's file of the "
                 f"same path. Each has a name of its own (`<name>_{cv.id}.omf`) and the module's {len(md.pointed)} models "
                 "that use them name it: the game's file stays the game's, for every model that names that path.")
    if md and md.kept:
        L.append(f"- {len(md.kept)} motion files on a game path were left as they are: " +
                 ", ".join(f"`{f}` ({why})" for f, why in sorted(md.kept.items())) + ".")
    if cv.recipes:
        L.append(f"- Craft recipes the mod added to `ui_itm_craft.script` are the module's recipes file `{cv.recipe_file}`: " +
                 ", ".join(f"`{r}` at `{at}`" for r, rows in sorted(cv.recipes.items()) for at, _, _ in rows) + ".")
    L += ["- An item asset sits in `items\\<family>\\<group>\\`: the family is the item class's (Dead Air's scopes are of "
          "the food class, so they are under `eatable`); the group is the shortest start of the section's name that is "
          "itself an item of the same family (`wpn_ak74_kobra` is under `wpn_ak74`), else the section itself.",
          "- A list key changes by `add` / `drop` only when it is a list of sections (`ammo_class`, `scopes`); every other "
          "value is set whole.", ""]
    L += ["## What the next step does by hand", ""]
    rows = []
    for rel, st in rep["scripts"]:
        if rel == "scripts\\ui_itm_craft.script" and cv.craft_whole:
            rows.append((f"`{rel}`", "replaces a game script", "nothing left: its change is the recipes, now the module's recipes file"))
        elif st == "new":
            rows.append((f"`{rel}`", "new", "a module script (`port\\" + rel + "`)"))
        elif rel in rep["rollback_only"]:
            rows.append((f"`{rel}`", "replaces a game script", "equal to its base: a rollback, nothing to carry"))
        else:
            rows.append((f"`{rel}`", "replaces a game script", "port the change against its base (`port\\" + rel + ".diff`)"))
    for rel in rep["xml"]["diffs"]:
        if rel in rep["rollback_only"]:
            rows.append((f"`{rel}`", "replaced XML", "equal to its base: a rollback, nothing to carry"))
        else:
            rows.append((f"`{rel}`", "replaced XML", f"an `.xmlp` of the change (`port\\xml\\{rel}.diff`)"))
    ua = rep["use_animations"]
    rows.append(("use animations", f"{ua['sections']} sections, {ua['item_table_entries']} table rows",
                 "scenes of the game's own system (`port\\use_animations.json`)"))
    if rep.get("particles"):
        rows.append(("particle actions", f"{rep['particles']['effects_whose_actions_the_game_does_not_read']} effects",
                     "the game's loose loader reads no action list; the `.pe` files carry it in `[actions]`"))
    L += _table(rows, ["What", "State", "To do"])
    L += ["", "## What was checked", ""]
    tc, fc = cv.tree_check, cv.file_check
    L += [f"- The game's system config composed with every item asset and patch of the module, in the game's order and by "
          f"its inheritance rule, equals the wanted state in every section and key ({tc['sections']} sections, "
          f"{tc['keys_compared']} keys): {tc['differences']} differences, {len(tc['refused'])} items the game would leave out, "
          f"{len(tc['ltxp_refused'])} directives it would skip; the names read as the mod's ({len(tc['texts'])} off), the recipes "
          f"are the mod's ({'equal' if tc['craft'] else 'they differ'}) (`port\\check_tree.txt`).",
          f"- The same for every config opened by name: {fc['targets']} files, {fc['differences']} differences, {fc['refused']} refused.",
          f"- Every `.itemasset` reads back as plain Lua data and follows the contract's table: {len(tc['asset_problems'])} problems.",
          f"- Every particle file reads back the way the game's loader reads it and equals its binary: "
          f"{len(rep['particles']['read_back_mismatches']) if rep.get('particles') else 0} mismatches.",
          "", "Not proven here: the game itself loading the module, and anything that needs the game running; the icons "
          "become the module's atlas only in the editor's module build.", ""]
    if cv.problems:
        L += ["## Problems", ""] + [f"- {p}" for p in cv.problems] + [""]
    cv.write_text("PORT_NOTES.md", "\n".join(L))


def _family(c, s):
    from items import family_of
    r = c.resolve(s)
    f = family_of(r)
    if f:
        return f
    cls = r.get("class") or ""
    if cls.startswith(("SM_", "AI_")):
        return "creature"
    if cls == "ON_OFF_S":
        return "squad"
    if s.startswith(("up_", "upgr")):
        return "upgrade"
    if "immunit" in s:
        return "immunities"
    if s.endswith("_hud"):
        return "hud"
    return "other"
