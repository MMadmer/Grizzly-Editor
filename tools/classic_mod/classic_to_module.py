#!/usr/bin/env python3
"""Turns a classic gamedata mod into a module project of Grizzly Editor, for Dead Air Refined.

A classic mod replaces whole files of the game it was made over. A module adds to the game of today, so the
mod's work is the mod minus its BASE (the game's files it was made from), applied to today's game:

  * the base of a file the mod replaces is the archived version of that path it is closest to (`--base`
    narrows the candidates); a file the game never had is the mod's own;
  * the system.ltx tree is compared on resolved values, three ways (base, mod, today's game): inventory items
    become `items\\<family>\\<group>\\<section>.itemasset` (a change of a game item with its HUD section and
    the mixins it owns, or a new item), everything else `patch\\<id>_<topic>.ltxp`, written parents first and
    checked by composing the game with the output;
  * configs scripts open by name get `#file` directives of the mod's own lines; XML and scripts are not
    converted (their diffs go to `port\\` for the next step);
  * strings: the ids the mod adds or changes, one table per language; icons: the pictures that are the mod's
    own, cut out of its atlas; particles: the effects and groups it adds or changes, as loose `.pe` / `.pg`;
  * media: the files that are the mod's own bytes, and an older game version only where the mod's work needs it;
    its motion files against the game's of the same paths (`motion_files.py`): a copy of the game's file gets the
    motions the game has grown since, a file of another skeleton gets a name of its own.

    python tools/classic_mod/classic_to_module.py --mod <gamedata> --game <game root> --out <new project>
        --id <module id> [--name <title>] [--base <archive names, comma separated, or a folder>]

It never changes the mod or the game. Everything it writes is listed in <out>\\temp\\classic_import.files: a
rerun removes exactly that and nothing a person added. Needs python-lzo, Pillow, numpy and the game's
tools\\stalker-tools (LzHuf).
"""
from __future__ import annotations

import argparse
import collections
import difflib
import hashlib
import json
import os
import re
import shutil
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import compose  # noqa: E402
import icons  # noqa: E402
import items  # noqa: E402
import luadata  # noqa: E402
import media  # noqa: E402
import motion_files  # noqa: E402
import notes  # noqa: E402
import particles  # noqa: E402
import strings  # noqa: E402
from archives import Game, Version, View, crc_of_file, walk  # noqa: E402
from ltx import REMOVED, decode, encode, lines_of, load_file, load_tree, norm  # noqa: E402

SYSTEM = "configs\\system.ltx"
# the game's own overlays, merged into the system config before the module stage (MODDING.md, "Patches of any config")
OVERLAYS = ["configs\\dead_air_x64_ai.ltx", "configs\\dead_air_x64_pda3d_items.ltx", "configs\\dead_air_x64_animations.ltx"]
STATE_FILES = {"configs\\axr_options.ltx": "the player's options, the game writes them",
               "configs\\atmosfear_options.ltx": "the player's weather options, the game writes them",
               "localization.ltx": "the player's language choice; and the engine reads no file at that path"}
LEDGER = os.path.join("temp", "classic_import.files")
TEXT_EXT = (".ltx", ".xml", ".script")
ATLAS = "textures\\ui\\ui_icon_equipment.dds"


class Failure(Exception):
    pass


class Converter:
    def __init__(self, args):
        self.mod_root = os.path.abspath(args.mod)
        self.game_root = os.path.abspath(args.game)
        self.out = os.path.abspath(args.out)
        self.id = args.id
        self.name = args.name or args.id
        self.base_arg = args.base
        self.t0 = time.time()
        self.written: dict[str, tuple[str, int]] = {}
        self.conflicts: list[str] = []
        self.kept_changed: list[str] = []
        self.port = os.path.join(self.out, "port")
        self.rep: dict = collections.OrderedDict()
        self.problems: list[str] = []
        self.next_log = 0.0

    # ---- output -----------------------------------------------------------------------------------------
    def log(self, msg, force=True):
        now = time.time() - self.t0
        if force or now >= self.next_log:
            print(f"[{now:7.1f}s] {msg}", flush=True)
            self.next_log = now + 60

    def _target(self, rel):
        path = os.path.join(self.out, rel)
        if rel in self.written:
            raise Failure(f"written twice: {rel}")
        if os.path.exists(path):
            self.conflicts.append(rel)
            return None
        os.makedirs(os.path.dirname(path), exist_ok=True)
        return path

    def write(self, rel, data: bytes):
        path = self._target(rel)
        if path is None:
            return False
        with open(path, "wb") as f:
            f.write(data)
        self.written[rel] = (hashlib.sha256(data).hexdigest(), len(data))
        return True

    def write_text(self, rel, text: str):
        return self.write(rel, text.encode("utf-8"))

    def copy(self, rel, src):
        path = self._target(rel)
        if path is None:
            return False
        h = hashlib.sha256()
        with open(src, "rb") as fi, open(path, "wb") as fo:
            while True:
                b = fi.read(1 << 22)
                if not b:
                    break
                h.update(b)
                fo.write(b)
        self.written[rel] = (h.hexdigest(), os.path.getsize(path))
        return True

    def prepare_out(self):
        ledger = os.path.join(self.out, LEDGER)
        if not os.path.isfile(ledger):
            return
        removed = 0
        dirs = set()
        for line in open(ledger, encoding="utf-8"):
            parts = line.rstrip("\n").split("\t")
            if len(parts) != 3:
                continue
            digest, size, rel = parts
            path = os.path.join(self.out, rel)
            if not os.path.isfile(path):
                continue
            if os.path.getsize(path) == int(size) and self._sha(path) == digest:
                os.remove(path)
                removed += 1
                dirs.add(os.path.dirname(path))
            else:
                self.kept_changed.append(rel)
        os.remove(ledger)
        for d in sorted(dirs, key=len, reverse=True):
            while d.startswith(self.out) and d != self.out:
                try:
                    os.rmdir(d)
                except OSError:
                    break
                d = os.path.dirname(d)
        self.log(f"an earlier run's files removed: {removed}; changed by a person and kept: {len(self.kept_changed)}")

    @staticmethod
    def _sha(path):
        h = hashlib.sha256()
        with open(path, "rb") as f:
            while True:
                b = f.read(1 << 22)
                if not b:
                    return h.hexdigest()
                h.update(b)

    def finish_ledger(self):
        lines = [f"{d}\t{s}\t{rel}" for rel, (d, s) in sorted(self.written.items())]
        os.makedirs(os.path.join(self.out, "temp"), exist_ok=True)
        with open(os.path.join(self.out, LEDGER), "w", encoding="utf-8", newline="\n") as f:
            f.write("\n".join(lines) + "\n")

    # ---- inputs ---------------------------------------------------------------------------------------------
    def scan(self):
        self.log("reading the game's archive tables")
        self.game = Game(self.game_root)
        self.log(f"game: {len(self.game.archives)} archives, {len(self.game.versions)} paths")
        self.mod = walk(self.mod_root)
        self.mod_names = set(self.mod)
        self.status: dict[str, dict] = {}
        total = sum(os.path.getsize(p) for p in self.mod.values())
        done = 0
        for i, (rel, path) in enumerate(sorted(self.mod.items())):
            size = os.path.getsize(path)
            crc = crc_of_file(path)
            done += size
            eff = self.game.effective(rel)
            if eff is None:
                st = "new"
            elif eff.size == size and self.game.crc(eff) == crc:
                st = "identical"
            else:
                st = "replaces"
            older = [v.label for v in self.game.versions.get(rel, []) if v is not eff and v.size == size and self.game.crc(v) == crc] \
                if st == "replaces" else []
            self.status[rel] = {"status": st, "size": size, "crc": crc, "older": older}
            self.log(f"mod files hashed: {i + 1}/{len(self.mod)} ({done >> 20} of {total >> 20} MB)", force=False)
        c = collections.Counter(v["status"] for v in self.status.values())
        self.rep["mod_files"] = {"files": len(self.mod), "bytes": total, **c}
        self.log(f"mod: {len(self.mod)} files, {total >> 20} MB: {dict(c)}")

    def candidates(self, rel):
        vs = self.game.distinct(rel)
        if not self.base_arg:
            return vs, False
        if os.path.isdir(self.base_arg):
            p = os.path.join(self.base_arg, rel)
            if os.path.isfile(p):
                return [Version("base folder", -1, os.path.getsize(p), crc_of_file(p), path=p)], False
            return vs, True
        names = {x.strip().lower() for x in self.base_arg.split(",") if x.strip()}
        narrowed = [v for v in vs if v.label.lower() in names]
        return (narrowed, False) if narrowed else (vs, True)

    def pick_bases(self):
        """The base of every text file the mod replaces: the archived version it is closest to by lines."""
        self.log("choosing the base of every replaced text file")
        self.base: dict[str, object] = {}
        rows, fell_back = [], []
        first = {}
        todo = [r for r, s in self.status.items() if s["status"] == "replaces" and r.startswith(("configs\\", "scripts\\"))
                and r.endswith(TEXT_EXT)]
        for i, rel in enumerate(sorted(todo)):
            mine = lines_of(open(self.mod[rel], "rb").read())
            vs, fb = self.candidates(rel)
            if fb:
                fell_back.append(rel)
            scored = []
            for v in vs:
                theirs = lines_of(self.game.read(v))
                sm = difflib.SequenceMatcher(None, theirs, mine, autojunk=False)
                plus = minus = 0
                for tag, i1, i2, j1, j2 in sm.get_opcodes():
                    if tag != "equal":
                        plus += j2 - j1
                        minus += i2 - i1
                scored.append((plus + minus, v, plus, minus))
            first[rel] = scored
            self.log(f"bases: {i + 1}/{len(todo)}", force=False)
        winners = collections.Counter(min(s, key=lambda x: x[0])[1].label for s in first.values() if s)
        rank = {lab: -n for lab, n in winners.items()}
        for rel, scored in sorted(first.items()):
            if not scored:
                continue
            best = min(s[0] for s in scored)
            tied = [s for s in scored if s[0] == best]
            pick = min(tied, key=lambda s: (rank.get(s[1].label, 0), s[1].order))
            self.base[rel] = pick[1]
            rows.append(f"{rel}\t{pick[1].label}\t+{pick[2]}/-{pick[3]}\t" + "  ".join(f"{s[1].label}:+{s[2]}/-{s[3]}" for s in scored))
        self.base_rows = rows
        self.rep["bases"] = dict(collections.Counter(v.label for v in self.base.values()))
        self.rep["bases_fell_back"] = fell_back
        self.log(f"bases chosen: {self.rep['bases']}")

    def views(self):
        self.gview = View(self.game)
        self.bview = View(self.game)
        self.mview = View(self.game)
        for rel, s in self.status.items():
            if not rel.startswith(("configs\\", "scripts\\")):
                continue
            self.mview.put_file(rel, self.mod[rel])
            if rel in self.base:
                self.bview.put_version(rel, self.base[rel])

    # ---- the system.ltx tree ---------------------------------------------------------------------------------
    def config_tree(self):
        self.log("composing the three system.ltx trees")
        G = load_tree(self.gview, SYSTEM, overlays=OVERLAYS)
        B = load_tree(self.bview, SYSTEM, overlays=OVERLAYS)
        M = load_tree(self.mview, SYSTEM, overlays=OVERLAYS)
        self.G, self.B, self.M = G, B, M
        self.tree_files = set(G.files) | set(B.files) | set(M.files)
        self.rep["tree"] = {"game_files": len(set(G.files)), "game_sections": len(G.sections),
                            "base_sections": len(B.sections), "mod_sections": len(M.sections),
                            "mod_files_in_tree": len([f for f in self.tree_files if f in self.mod]),
                            "mod_duplicate_sections": len(M.dupes), "mod_problems": len(set(M.problems) - set(G.problems))}
        it = compose.three_way(B, M, G)
        junk = 0
        for s in list(it.pins):
            for k in [k for k in it.pins[s] if items.junk_key(k)]:
                del it.pins[s][k]
                junk += 1
            if not it.pins[s]:
                del it.pins[s]
        for s in it.new.values():
            for k in [k for k in s.own if items.junk_key(k)]:
                del s.own[k]
                junk += 1
        self.junk_keys = junk
        self.log(f"intent: {len(it.pins)} game sections changed, {len(it.new)} new, {len(it.conflicts)} conflicts, "
                 f"{len(it.rollbacks)} rolled-back slots not carried, {len(it.lost_game_additions)} game sections the mod's files lack (kept)")
        W0 = compose.wanted(G, it)
        own = items.Owners(W0, M, G)
        # the use-animation pseudo weapons and what only they use go to port\use_animations.json
        excluded = sorted(s for s in set(it.pins) | set(it.new) if own.excluded(s))
        self.excluded = {s: ("new" if s in it.new else "changed") for s in excluded}
        for s in excluded:
            it.pins.pop(s, None)
            it.new.pop(s, None)
        self.log(f"left to the next step (use animations): {len(excluded)} sections")
        self.intent = it
        self.icon_pass(it, own)
        W = compose.wanted(G, it)
        own = items.Owners(W, M, G)
        self.W, self.own = W, own
        self.strings_intent()
        p = compose.plan(G, W, it)
        self.plan = p
        nops = sum(len(v) for v in p.ops.values())
        self.log(f"plan: {nops} directives on {len(p.ops)} sections, {len(p.new)} new sections")
        self.script_read = self.files_scripts_read()
        self.distribute(p, own)
        self.write_items_and_patches()
        self.verify_tree()
        self.rep["class_changes"] = sorted((s, op[2]) for s, ops in p.ops.items() for op in ops if op[0] == "set" and op[1] == "class")

    def icon_pass(self, it, own):
        """The mod's own pictures become icon files; inv_grid_* pointing into the mod's atlas is never carried."""
        self.log("comparing icons in the mod's, the base's and the game's atlases")
        atl: dict[tuple, object] = {}

        def atlas(which, tex):
            key = (which, tex)
            if key in atl:
                return atl[key]
            rel = "textures\\" + tex + ".dds"
            data = None
            if which == "mod":
                data = open(self.mod[rel], "rb").read() if rel in self.mod else None
                if data is None and self.game.effective(rel):
                    data = self.game.read(self.game.effective(rel))
            elif which == "game":
                v = self.game.effective(rel)
                data = self.game.read(v) if v else None
            else:
                v = self.atlas_base.get(tex)
                data = self.game.read(v) if v else None
            atl[key] = icons.decode(data) if data else None
            return atl[key]

        # the base of an atlas: the archived version with the fewest cells differing from the mod's
        self.atlas_base = {}
        texs = set()
        for n in own.items:
            r = self.M.resolve(n) if n in self.M.sections else {}
            c = icons.coords(r)
            if c:
                texs.add(c[0])
        self.atlas_rows = []
        for tex in sorted(texs):
            rel = "textures\\" + tex + ".dds"
            if rel not in self.mod:
                self.atlas_base[tex] = self.game.effective(rel)
                continue
            mine = atlas("mod", tex)
            best = None
            for v in self.game.distinct(rel):
                n = icons.cells_differing(icons.decode(self.game.read(v)), mine)
                self.atlas_rows.append(f"{rel}\t{v.label}\t{n} cells differ from the mod's")
                if best is None or n < best[0]:
                    best = (n, v)
            self.atlas_base[tex] = best[1] if best else None
        self.icon_files = {}
        stats = collections.Counter()
        dropped_pins = 0
        for s in list(it.pins):
            for k in items.ICON_KEYS:
                if k in it.pins[s]:
                    del it.pins[s][k]
                    dropped_pins += 1
            if not it.pins[s]:
                del it.pins[s]
        for n in sorted(own.items):
            if n not in self.M.sections or own.excluded(n):
                continue
            mc = icons.coords(self.M.resolve(n))
            if not mc:
                continue
            tex, x, y, w, h = mc
            mpic = icons.cut(atlas("mod", tex), x, y, w, h) if atlas("mod", tex) is not None else None
            if mpic is None:
                stats["no picture"] += 1
                continue
            if n in self.G.sections or n in self.B.sections:
                diff_b = True
                if n in self.B.sections:
                    bc = icons.coords(self.B.resolve(n))
                    ba = atlas("base", bc[0]) if bc else None
                    diff_b = icons.differs(mpic, icons.cut(ba, *bc[1:]) if ba is not None else None)
                diff_g = True
                if n in self.G.sections:
                    gc = icons.coords(self.G.resolve(n))
                    ga = atlas("game", gc[0]) if gc else None
                    diff_g = icons.differs(mpic, icons.cut(ga, *gc[1:]) if ga is not None else None)
                if diff_b and diff_g:
                    self.icon_files[n] = (mpic, w, h, "own")
                    stats["own picture"] += 1
                elif not diff_g:
                    stats["the game's picture"] += 1
                else:
                    stats["the base's picture (old art, not carried)"] += 1
            else:
                ba, ga = atlas("base", tex), atlas("game", tex)
                same_g = ga is not None and not icons.differs(mpic, icons.cut(ga, x, y, w, h))
                same_b = ba is not None and not icons.differs(mpic, icons.cut(ba, x, y, w, h))
                if same_g:
                    stats["new item on a cell the game's atlas still shows"] += 1
                elif same_b:
                    self.icon_files[n] = (mpic, w, h, "old art the game's atlas no longer has there")
                    stats["new item showing old art (cut to a file)"] += 1
                else:
                    self.icon_files[n] = (mpic, w, h, "own")
                    stats["own picture"] += 1
        self.rep["icons"] = dict(stats)
        self.rep["icon_pins_dropped"] = dropped_pins
        self.log(f"icons: {dict(stats)}; inv_grid/icons_texture values not carried: {dropped_pins}")

    def strings_intent(self):
        self.log("comparing string tables")
        self.str_want, self.str_stats, self.str_tables = {}, {}, {}
        langs = sorted(set(strings.languages(self.mview)))
        self.game_tables = {lang: strings.merged(self.gview, lang) for lang in strings.languages(self.gview)}
        for lang in langs:
            want, rolled, same, mtable = strings.intent(self.bview, self.mview, self.gview, lang)
            if not any(rel.startswith(f"configs\\text\\{lang}\\") for rel in self.mod):
                continue
            self.str_want[lang] = want
            self.str_tables[lang] = mtable
            self.str_stats[lang] = {"wanted": len(want), "carried_from_base_only": rolled, "already_the_game_s": len(same)}
        self.log(f"strings: {self.str_stats}")

    def text_of(self, sid: str):
        """A string id's texts by language as the game with the module reads them: the game's tables, the mod's ids over them."""
        b = encode(sid)
        out = {}
        for lang in sorted(set(self.str_want) | set(self.game_tables)):
            text = self.str_want.get(lang, {}).get(b, self.game_tables.get(lang, {}).get(b))
            if text is not None:
                out[lang] = strings.plain_text(text)
        return out

    def inline_texts(self, spec, sect):
        """A new item's name and description inline, when the ids are the mod's own and nothing else names them.
        The game makes `inv_name_short` the name's string when `short_name` is left out, so a short name of its own
        comes along whenever the item's differs from its name."""
        rus = self.str_want.get("rus", {})

        def own_id(key):
            sid = norm(sect.own.get(key))
            if not sid or encode(sid) not in rus or (self.id_users.get(sid, set()) - {spec.section}):
                return None
            return sid

        name = own_id("inv_name")
        if name:
            short = norm(self.W.resolve(spec.section).get("inv_name_short"))
            short_texts = None
            if short and short != name:
                short_texts = self.text_of(short)
                if "rus" not in short_texts:
                    name = None
            if name:
                spec.texts["name"] = self.text_of(name)
                spec.drop_keys.add("inv_name")
                self.inlined.add(encode(name))
                if short_texts:
                    spec.texts["short_name"] = short_texts
                    spec.drop_keys.add("inv_name_short")
                    if own_id("inv_name_short") == short:
                        self.inlined.add(encode(short))
                else:
                    spec.drop_keys.add("inv_name_short")
        descr = own_id("description")
        if descr:
            spec.texts["description"] = self.text_of(descr)
            spec.drop_keys.add("description")
            self.inlined.add(encode(descr))

    def files_scripts_read(self) -> set:
        """Config files the game's scripts open by name, with every file those include."""
        names = set()
        for rel in self.gview.names:
            if rel.startswith("scripts\\") and rel.endswith(".script"):
                for m in re.finditer(r'ini_file(?:_ex)?\(\s*"([^"]+)"', decode(self.gview.read(rel) or b"")):
                    names.add(os.path.normpath("configs\\" + m.group(1).replace("\\\\", "\\")).lower())
        out = set()
        for n in sorted(names):
            if self.gview.exists(n):
                out.update(load_file(self.gview, n).files)
        return out

    def craft_recipes(self) -> dict:
        """Recipes a mod adds to Dead Air's craft script (`ui_itm_craft.script`: tables of
        `{"<result>", <count>, "<part>", ...}`, each table the recipes of the tool its `section ==` line names)."""
        rel = "scripts\\ui_itm_craft.script"
        out = collections.defaultdict(list)
        self.craft_rows, self.craft_whole = [], False
        if rel not in self.mod or rel not in self.base:
            return out
        mine = decode(open(self.mod[rel], "rb").read()).splitlines()
        base = decode(self.game.read(self.base[rel])).splitlines()
        tool_of = {}
        for m in re.finditer(r'section\s*==\s*"([^"]+)"\s*\)\s*then\s+Recepies_add\s*=\s*(\w+)', "\n".join(mine)):
            tool_of.setdefault(m.group(2), m.group(1))
        rx = re.compile(r'^\s*\{\s*"([^"]+)"\s*,\s*(\d+)\s*,\s*((?:"[^"]*"\s*,?\s*)+)\}\s*,?\s*(--.*)?$')
        added, removed, other = [], 0, 0
        sm = difflib.SequenceMatcher(None, base, mine, autojunk=False)
        for tag, i1, i2, j1, j2 in sm.get_opcodes():
            if tag in ("delete", "replace"):
                removed += sum(1 for x in base[i1:i2] if x.strip())
            if tag in ("insert", "replace"):
                for j in range(j1, j2):
                    m = rx.match(mine[j])
                    if m:
                        added.append((j, m))
                    elif mine[j].strip():
                        other += 1
        for j, m in added:
            table = None
            for k in range(j, -1, -1):
                t = re.match(r"^\s*local\s+(\w+)\s*=\s*\{", mine[k])
                if t:
                    table = t.group(1)
                    break
            tool = tool_of.get(table)
            parts = [x for x in re.findall(r'"([^"]*)"', m.group(3)) if x and x != "-"]
            if tool and 1 <= len(parts) <= 4:
                out[m.group(1).lower()].append((tool.lower(), int(m.group(2)), [x.lower() for x in parts]))
                self.craft_rows.append((m.group(1).lower(), tool, int(m.group(2)), parts))
            else:
                self.problems.append(f"a recipe line of {rel} with no tool or more than four parts: {mine[j].strip()}")
        self.craft_whole = bool(added) and not removed and not other
        return out

    def distribute(self, p, own):
        self.specs: dict[str, items.ItemSpec] = {}
        self.patches: dict[str, list] = collections.defaultdict(list)
        self.sysfile_blocks: dict[str, list] = collections.defaultdict(list)
        self.inlined = set()
        self.items_in_script_files = []
        # string ids named by section values, to keep a shared one in the tables
        self.id_users = collections.defaultdict(set)
        for n in self.W.sections:
            for v in self.W.resolve(n).values():
                if v and v is not REMOVED and len(v) < 80 and "," not in v:
                    self.id_users[v].add(n)

        def spec_of(item):
            sp = self.specs.get(item)
            if sp is None:
                fam = own.items.get(item, "item")
                sp = items.ItemSpec(item, item in p.new, fam, own.group(item, fam))
                self.specs[item] = sp
            return sp

        for s in self.W.topo():
            if s in p.new:
                entry = ("new", p.new[s])
            elif s in p.ops:
                entry = ("edit", s, p.ops[s])
            else:
                continue
            kind, item, role = own.owner(s)
            file = self.W.sections[s].file
            if kind == "item" and file in self.script_read and file in self.tree_files:
                self.items_in_script_files.append((s, file))
            if kind == "item":
                sp = spec_of(item)
                if role == "main":
                    if sp.new:
                        if not entry[1].parents:
                            self.patches["other"].append(entry)
                            del self.specs[item]
                            self.problems.append(f"[{item}] is a new item with no parent: written as a patch section")
                            continue
                        sp.main_new = entry[1]
                        self.inline_texts(sp, entry[1])
                    else:
                        sp.main_ops = entry[2]
                elif role == "hud" and sp.new and entry[0] == "new" and s == item + "_hud" and entry[1].parents \
                        and norm((sp.main_new.own if sp.main_new else {}).get("hud")) == s:
                    sp.hud_new = entry[1]
                    sp.drop_keys.add("hud")
                elif role == "hud" and not sp.new and entry[0] == "edit":
                    sp.hud_ops = entry[2]
                    sp.hud_name = s
                else:
                    sp.sections.append(entry)
            elif file in self.script_read and file in self.tree_files:
                # a file of the system config a script also reads: `#file` reaches both
                self.sysfile_blocks[file].append(entry)
            else:
                self.patches[items.topic_of(s, self.W.resolve(s))].append(entry)
        # an item whose only change is its picture
        for n, (pic, w, h, why) in self.icon_files.items():
            if n not in own.items:
                continue
            sp = spec_of(n)
            png = os.path.join("icons", n + ".png")
            rel_to_asset = os.path.relpath(png, os.path.dirname(sp.path)).replace("\\", "/")
            sp.icon = (rel_to_asset, w, h)
            sp.icon_png = png
            if sp.new:
                sp.drop_keys.update(items.ICON_KEYS)
        # the recipes the mod added to the craft script
        self.recipes = self.craft_recipes()
        for result, rows in sorted(self.recipes.items()):
            if result in own.items and not own.excluded(result):
                spec_of(result).craft = rows
            else:
                self.problems.append(f"a recipe makes [{result}], which is no item of the module")
        for item in [k for k, v in self.specs.items() if v.new and v.main_new is None]:
            self.problems.append(f"[{item}]: a new item's section was not planned; left out")
            del self.specs[item]
        self.settle()

    # ---- what the game makes first -----------------------------------------------------------------------------
    def settle(self):
        """Every parent an item names must be there before the game reads the item: the game's own sections, or
        those of a new item it waits for (`BuildItems`). A section whose parent cannot be there in time - one an item
        of its own makes, or a change makes - is written over that parent's own parents, with what it took from it."""
        self.flattened = []
        for _ in range(100):
            tables = [(sp.path, items.asset_table(sp)) for sp in self.specs.values() if not sp.empty()]
            refused, _ = items.apply_items(tables, compose.Composer(self.G), self.id)
            fixed = False
            for file, why, missing in refused:
                sp = next(s for s in self.specs.values() if s.path == file)
                for name, parent in missing:
                    sect = self._made_by(sp, name)
                    if sect is not None and parent not in self.G.sections:
                        self._flatten(sect)
                        fixed = True
            if not refused or not fixed:
                for file, why, missing in refused:
                    self.problems.append(f"{file}: the game would leave it out - {why}")
                return

    @staticmethod
    def _made_by(sp, name):
        if sp.main_new is not None and sp.main_new.name == name:
            return sp.main_new
        if sp.hud_new is not None and sp.section + "_hud" == name:
            return sp.hud_new
        for e in sp.sections:
            if e[0] == "new" and e[1].name == name:
                return e[1]
        return None

    def _flatten(self, sect):
        """The parents the game has instead of the module's own; the keys that differ from them written out."""
        def expand(parents):
            out = []
            for p in parents:
                if p in self.G.sections:
                    out.append(p)
                elif p in self.W.sections:
                    out += expand(self.W.sections[p].parents)
            return out
        was = list(sect.parents)
        parents = expand(sect.parents)
        base: dict = {}
        for p in parents:
            base.update(self.W.resolve(p))
        want = self.W.resolve(sect.name)
        order = list(sect.own) + sorted(k for k in want if k not in sect.own)
        sect.parents = parents
        sect.own = {k: want[k] for k in order if k in want and want[k] is not REMOVED and compose.val(base, k) != compose.val(want, k)}
        self.flattened.append((sect.name, was, parents))

    def write_items_and_patches(self):
        self.log("writing item assets and patches")
        self.asset_tables = {}
        for sp in sorted(self.specs.values(), key=lambda s: s.path):
            if sp.empty():
                continue
            t = items.asset_table(sp)
            self.write_text(sp.path, luadata.write(t))
            self.asset_tables[sp.path] = t
            if sp.icon_png:
                pic = self.icon_files[sp.section][0]
                path = self._target(sp.icon_png)
                if path:
                    icons.save_png(pic, path)
                    data = open(path, "rb").read()
                    self.written[sp.icon_png] = (hashlib.sha256(data).hexdigest(), len(data))
        self.patch_files = {}
        for topic in sorted(self.patches):
            lines = [f"; {PATCH_TOPICS.get(topic, topic)}"]
            self._patch_lines(lines, self.patches[topic])
            self.patch_files[os.path.join("patch", f"{self.id}_{topic}.ltxp")] = lines
        if self.sysfile_blocks:
            lines = ["; Files of the system config a script also reads by name (#file reaches both): the mod's own values."]
            for f in sorted(self.sysfile_blocks):
                lines += ["", "#file " + f[len("configs\\"):]]
                self._patch_lines(lines, self.sysfile_blocks[f])
                lines += ["", "#file"]
            self.patch_files[os.path.join("patch", f"{self.id}_system_files.ltxp")] = lines
        self.rep["items"] = {"new": sum(1 for t in self.asset_tables.values() if "edit" not in t),
                             "changed": sum(1 for t in self.asset_tables.values() if "edit" in t)}

    def _patch_lines(self, lines, entries):
        for e in entries:
            lines.append("")
            if e[0] == "new":
                s = e[1]
                lines.append(f"[{s.name}]" + (" : " + ", ".join(s.parents) if s.parents else ""))
                for k, v in s.own.items():
                    lines.append(self._ltxp_line("set", k, v))
            else:
                lines.append(f"[{e[1]}]")
                for op in e[2]:
                    lines.append(self._ltxp_line(*op))

    def _ltxp_line(self, kind, key, value=None):
        if kind == "unset":
            return "!" + key
        if kind in ("add", "drop"):
            return ("+" if kind == "add" else "-") + key + " = " + ",".join(value)
        v = "" if value is None else value
        if ";" in v or "\n" in v or "\r" in v:
            self.problems.append(f"a value .ltxp cannot hold (`;` or a line break): {key} = {v[:60]!r}")
        return f"{key} = {v}"

    # ---- the check: compose the game with what was written ------------------------------------------------
    def read_back_items(self):
        out = []
        for rel in sorted(self.asset_tables):
            text = open(os.path.join(self.out, rel), encoding="utf-8").read()
            t = luadata.read(text)
            self.asset_problems.extend(items.check_table(t, rel))
            out.append((rel, t))
        return out

    def verify_tree(self):
        self.log("checking: the game composed with the module's items and patches against the wanted state")
        self.asset_problems = []
        assets = self.read_back_items()
        # write the system patches now, then read them back like the game
        for rel in sorted(self.patch_files):
            self.write(rel, encode("\r\n".join(self.patch_files[rel]) + "\r\n"))
        system_files = {f[len("configs\\"):] for f in self.tree_files}
        comp = compose.Composer(self.G)
        refused, plans = items.apply_items(assets, comp, self.id)
        for rel in sorted(self.patch_files):
            apply_ltxp(open(os.path.join(self.out, rel), "rb").read(), comp, rel, system=True, system_files=system_files)
        # what the module build and the strings decide is checked apart: icon keys, the string ids, the recipes
        skip = {"xms_craft": None}
        text_problems = []
        for p in plans:
            drop = set()
            if p.icon_picture:
                drop.update(items.ICON_KEYS)
            for field, key in (("name", "inv_name"), ("short_name", "inv_name_short"), ("description", "description")):
                given = p.texts.get(field) or (p.texts.get("name") if field == "short_name" else None)
                if given is None:
                    continue
                drop.add(key)
                want = self.text_of(norm(self.W.resolve(p.section).get(key)) or "")
                got = {"rus": given} if isinstance(given, str) else given
                if want.get("rus") != got.get("rus"):
                    text_problems.append(f"{p.file}: {field} reads {got.get('rus')!r}, the mod's text is {want.get('rus')!r}")
            if drop:
                skip[p.section] = drop
        made = comp.c.sections.get("xms_craft")
        made_rows = sorted((made.own if made else {}).values())
        want_rows = sorted(norm(", ".join([at, r, str(n)] + parts)) for r, rows in self.recipes.items() for at, n, parts in rows)
        craft_ok = made_rows == want_rows
        diffs = [d for d in compose.compare(self.W, comp.c) if not (d[0] in skip and (skip[d[0]] is None or d[1] in skip[d[0]]))]
        self.tree_check = {"differences": len(diffs), "refused": refused, "asset_problems": self.asset_problems,
                           "keys_compared": sum(len(self.W.resolve(s)) for s in self.W.sections), "sections": len(self.W.sections),
                           "flattened": self.flattened, "texts": text_problems, "craft": craft_ok, "ltxp_refused": comp.refused}
        with open(os.path.join(self.port, "check_tree.txt"), "w", encoding="utf-8") as f:
            f.write("The game's system config composed with the module's items (new ones first, each waiting for its bases, "
                    "then the changes in path order) and its .ltxp in name order, against the wanted state.\n"
                    f"sections {self.tree_check['sections']}, keys {self.tree_check['keys_compared']}\n"
                    f"differences: {len(diffs)}\n")
            for d in diffs[:2000]:
                f.write(f"{d}\n")
            f.write(f"\nitems the game would leave out: {len(refused)}\n" + "\n".join(map(str, refused)) + "\n")
            f.write(f"\ndirectives the game would skip: {len(comp.refused)}\n" + "\n".join(comp.refused) + "\n")
            f.write(f"\nasset problems: {len(self.asset_problems)}\n" + "\n".join(self.asset_problems) + "\n")
            f.write(f"\nnames and descriptions that read otherwise than the mod's: {len(text_problems)}\n" + "\n".join(text_problems) + "\n")
            f.write(f"\n[xms_craft] the recipes made: {made_rows}; the mod's: {want_rows}\n")
            f.write(f"\nsections written over their parent's parents, because the parent cannot be there in time: {self.flattened}\n")
        self.log(f"check: {len(diffs)} differences, {len(refused)} items left out, {len(comp.refused)} directives skipped, "
                 f"{len(self.asset_problems)} asset problems, {len(text_problems)} texts off, recipes {'equal' if craft_ok else 'DIFFER'}")
        if diffs or refused or comp.refused or self.asset_problems or text_problems or not craft_ok:
            self.problems.append("the composition check of the system config failed (port\\check_tree.txt)")

    # ---- configs outside the tree ---------------------------------------------------------------------------
    def outside_configs(self):
        self.log("configs outside the system.ltx tree")
        self.file_blocks = collections.defaultdict(list)
        self.outside_rows = []
        self.outside_conflicts, self.outside_deleted = [], []
        shipped, state = [], []
        checks = {"targets": 0, "differences": 0, "refused": 0}
        for rel in sorted(r for r in self.mod if r.startswith("configs\\") and r.endswith(".ltx") and r not in self.tree_files):
            st = self.status[rel]["status"]
            if rel in STATE_FILES:
                state.append(rel)
                continue
            if st == "identical":
                continue
            if st == "new":
                self.copy(os.path.join("gamedata", rel), self.mod[rel])
                shipped.append(rel)
                continue
            Gf = load_file(self.gview, rel, fallback=self.G)
            Bf = load_file(self.bview, rel, fallback=self.G)
            Mf = load_file(self.mview, rel, fallback=self.G)
            scope = {n for c in (Gf, Bf, Mf) for n, s in c.sections.items() if s.file == rel}
            it = compose.three_way(Bf, Mf, Gf, scope)
            for s in list(it.pins):
                for k in [k for k in it.pins[s] if items.junk_key(k)]:
                    del it.pins[s][k]
                if not it.pins[s]:
                    del it.pins[s]
            Wf = compose.wanted(Gf, it, fallback=self.G)
            p = compose.plan(Gf, Wf, it)
            entries = []
            for s in Wf.topo():
                if s in p.new:
                    entries.append(("new", p.new[s]))
                elif s in p.ops:
                    entries.append(("edit", s, p.ops[s]))
            n_ops = sum(len(v) for v in p.ops.values()) + sum(len(s.own) for s in p.new.values())
            self.outside_rows.append((rel, self.base[rel].label if rel in self.base else "-", len(p.ops), len(p.new), n_ops,
                                      len(it.deleted_by_mod), len(it.lost_game_additions), len(it.conflicts)))
            if not entries:
                continue
            lines = []
            self._patch_lines(lines, entries)
            block = ["", "#file " + rel[len("configs\\"):]] + lines + ["", "#file"]
            topic = file_topic(rel)
            self.file_blocks[topic].append((rel, block, Gf, Wf, scope))
            self.outside_conflicts += [(rel,) + c for c in it.conflicts]
            self.outside_deleted += [(rel, s) for s in it.deleted_by_mod]
        self.log(f"outside the tree: {len(self.outside_rows)} files compared, {sum(len(v) for v in self.file_blocks.values())} "
                 f"#file targets, {len(shipped)} new files shipped, {len(state)} player-state files left out")
        self.rep["outside"] = {"compared": len(self.outside_rows), "targets": sum(len(v) for v in self.file_blocks.values()),
                               "shipped_new": shipped, "state_left_out": state}
        for topic, blocks in sorted(self.file_blocks.items()):
            rel = os.path.join("patch", f"{self.id}_{topic}.ltxp")
            lines = [f"; {FILE_TOPICS.get(topic, 'The mod own lines in configs scripts open by name')}"]
            for _, block, *_ in blocks:
                lines += block
            self.write(rel, encode("\r\n".join(lines) + "\r\n"))
            data = open(os.path.join(self.out, rel), "rb").read()
            for frel, _, Gf, Wf, scope in blocks:
                comp = compose.Composer(Gf, fallback=self.G)
                apply_ltxp(data, comp, rel, system=False, only_file=frel[len("configs\\"):])
                diffs = compose.compare(Wf, comp.c, names=scope)
                checks["targets"] += 1
                checks["differences"] += len(diffs)
                checks["refused"] += len(comp.refused)
                if diffs or comp.refused:
                    self.problems.append(f"#file {frel}: {len(diffs)} differences, {len(comp.refused)} refused")
                    with open(os.path.join(self.port, "check_files.txt"), "a", encoding="utf-8") as f:
                        f.write(f"{frel}: {diffs[:50]} {comp.refused[:20]}\n")
        self.file_check = checks
        self.log(f"#file check: {checks}")

    # ---- strings ------------------------------------------------------------------------------------------------
    def write_strings(self):
        self.str_written = {}
        for lang, want in sorted(self.str_want.items()):
            ids = {k: v for k, v in want.items() if k not in self.inlined}
            if not ids:
                continue
            rel = os.path.join("gamedata", "configs", "text", lang, f"{self.id}_strings.xml")
            self.write(rel, strings.write_table(ids))
            self.str_written[lang] = len(ids)
        self.rep["strings"] = {"written": self.str_written, "inlined_ids": len(self.inlined), "per_language": self.str_stats}
        self.log(f"strings written: {self.str_written}, inline in item assets: {len(self.inlined)} ids")

    # ---- XML and scripts for the next step --------------------------------------------------------------------
    def three_diff(self, rel):
        mine = decode(open(self.mod[rel], "rb").read()).splitlines()
        base = decode(self.game.read(self.base[rel])).splitlines() if rel in self.base else []
        eff = self.game.effective(rel)
        game = decode(self.game.read(eff)).splitlines() if eff else []
        lab = self.base[rel].label if rel in self.base else "-"
        out = [f"# {rel}", f"# base: {lab}; the game today: {eff.label if eff else '-'}", "",
               "# ---- the mod's change against its base (what the author did)"]
        out += list(difflib.unified_diff(base, mine, f"base ({lab})", "mod", lineterm="", n=3))
        out += ["", "# ---- the game's change against the same base (what the game did since)"]
        out += list(difflib.unified_diff(base, game, f"base ({lab})", "game", lineterm="", n=3))
        return "\n".join(out) + "\n"

    def xml_and_scripts(self):
        self.log("XML and scripts for the next step")
        xml_diffs, xml_new, scripts = [], [], []
        for rel in sorted(self.mod):
            st = self.status[rel]["status"]
            if rel.startswith("configs\\") and rel.endswith(".xml") and not rel.startswith("configs\\text\\"):
                if st == "replaces":
                    self.write(os.path.join("port", "xml", rel + ".diff"), encode(self.three_diff(rel)))
                    xml_diffs.append(rel)
                elif st == "new":
                    self.copy(os.path.join("gamedata", rel), self.mod[rel])
                    xml_new.append(rel)
            elif rel.startswith("scripts\\"):
                self.copy(os.path.join("port", rel), self.mod[rel])
                if st == "replaces":
                    self.write(os.path.join("port", rel + ".diff"), encode(self.three_diff(rel)))
                scripts.append((rel, st))
        self.rep["xml"] = {"diffs": xml_diffs, "shipped_new": xml_new}
        # a copy equal to its base only undoes what the game did since: nothing of the author's to carry
        self.rep["rollback_only"] = sorted(r for r in xml_diffs + [s for s, st in scripts if st == "replaces"]
                                           if r in self.base and self.game.read(self.base[r]) == open(self.mod[r], "rb").read())
        self.rep["scripts"] = scripts
        self.log(f"xml diffs {len(xml_diffs)}, new xml shipped {len(xml_new)}, scripts to port {len(scripts)}")

    # ---- particles -----------------------------------------------------------------------------------------------
    def particles(self):
        rel = "particles.xr"
        if rel not in self.mod:
            self.rep["particles"] = None
            return
        self.log("particles.xr: effects and groups")
        me, mg = particles.read_library(open(self.mod[rel], "rb").read())
        eff = self.game.effective(rel)
        ge, gg = particles.read_library(self.game.read(eff))
        best = None
        rows = []
        for v in self.game.distinct(rel):
            be, bg = particles.read_library(self.game.read(v))
            n = sum(1 for k, d in me.items() if k not in be or not particles.same(d, be[k])) + \
                sum(1 for k, d in mg.items() if k not in bg or not particles.same(d, bg[k]))
            rows.append(f"{v.label}: {n} definitions differ from the mod's")
            if best is None or n < best[0]:
                best = (n, v, be, bg)
        _, bv, be, bg = best
        stats = collections.Counter()
        written = []
        bad_names = []
        for kind, mine, base, game, ext, writer in (("effect", me, be, ge, ".pe", particles.write_effect),
                                                    ("group", mg, bg, gg, ".pg", particles.write_group)):
            for name, d in sorted(mine.items()):
                if name in base and particles.same(d, base[name]):
                    if name in game and not particles.same(d, game[name]):
                        stats[f"{kind}s carried from the base only"] += 1
                    continue
                if name in game and particles.same(d, game[name]):
                    stats[f"{kind}s the game already has"] += 1
                    continue
                stats[f"{kind}s {'changed' if name in base else 'new'}"] += 1
                if name != name.lower() or " " in name or "\t" in name:
                    bad_names.append(name)
                out = particles.loose_path(name.lower(), ext)
                self.write(out, writer(d))
                written.append((kind, name, out))
            stats[f"base {kind}s the mod lacks"] = sum(1 for k in base if k not in mine)
        # read every file back the way the game's loader does and compare it field by field with the binary
        mismatches = []
        for kind, name, out in written:
            data = open(os.path.join(self.out, out), "rb").read()
            src = (me if kind == "effect" else mg)[name]
            back = (particles.load2_effect if kind == "effect" else particles.load2_group)(data, name)
            diff = particles.equal_fields(src, back)
            if diff:
                mismatches.append((name, diff))
        no_actions = sum(1 for k, n, _ in written if k == "effect" and me[n]["actions"])
        self.rep["particles"] = {"base": bv.label, "base_rows": rows, **stats, "files": len(written),
                                 "read_back_mismatches": mismatches, "names_not_lower_case_or_with_blanks": bad_names,
                                 "effects_whose_actions_the_game_does_not_read": no_actions}
        self.log(f"particles: base {bv.label}; {dict(stats)}; read-back mismatches {len(mismatches)}")
        if mismatches:
            self.problems.append(f"particles: {len(mismatches)} files do not read back as their binary")

    # ---- media ---------------------------------------------------------------------------------------------------
    def media(self):
        self.log("media: deciding what is the mod's own")
        decided: dict[str, tuple[str, str]] = {}
        own, old = set(), set()
        for rel, s in self.status.items():
            if rel.startswith("scripts\\") or (rel.startswith("configs\\") and rel.endswith((".ltx", ".xml"))) \
                    or rel in STATE_FILES or ("\\" not in rel and rel.endswith(".xr")):
                continue
            why = media.junk(rel, self.mod_names)
            if why:
                decided[rel] = ("junk", why)
            elif rel == ATLAS or rel == ATLAS[:-4] + ".thm":
                decided[rel] = ("atlas", "the icon atlas: the items' own pictures are icon files instead")
            elif s["status"] == "identical":
                decided[rel] = ("identical", "the game's own bytes today")
            elif s["status"] == "new":
                own.add(rel)
            elif s["older"]:
                old.add(rel)
            else:
                own.add(rel)
        for rel in self.mod:
            if "\\" not in rel and rel.endswith(".xr"):
                decided[rel] = ("identical", "the game's own bytes today") if self.status[rel]["status"] == "identical" else \
                    ("particles", "never shipped: its effects are loose files") if rel == "particles.xr" else ("xr", "a root library")
        need = self.media_evidence(own, old)
        for r in old:
            if r in need:
                decided[r] = ("kept old", need[r])
            else:
                decided[r] = ("old game bytes", "equal to " + ", ".join(self.status[r]["older"]))
        for r in own:
            decided[r] = ("own", "new" if self.status[r]["status"] == "new" else "the mod's own bytes")
        self.media_decided = decided
        ship = sorted(r for r, d in decided.items() if d[0] in ("own", "kept old"))
        total = sum(self.status[r]["size"] for r in ship)
        free = shutil.disk_usage(self.out).free
        if free < total + (2 << 30):
            raise Failure(f"not enough free space: {free >> 20} MB free, {total >> 20} MB to copy")
        self.log(f"media: {len(ship)} files, {total >> 20} MB to copy ({free >> 30} GB free)")
        done = 0
        for i, r in enumerate(ship):
            self.copy(os.path.join("gamedata", r), self.mod[r])
            done += self.status[r]["size"]
            self.log(f"copied {i + 1}/{len(ship)} ({done >> 20} of {total >> 20} MB)", force=False)
        c = collections.Counter(d[0] for d in decided.values())
        by = collections.defaultdict(lambda: [0, 0])
        for r, d in decided.items():
            by[d[0]][0] += 1
            by[d[0]][1] += self.status[r]["size"]
        self.rep["media"] = {k: {"files": v[0], "mb": round(v[1] / 1048576, 1)} for k, v in sorted(by.items())}
        self.log(f"media done: {dict(c)}")

    def media_evidence(self, own: set, old: set) -> dict[str, str]:
        """Old game bytes the mod's own work needs, with the reason: named by a config key the mod sets, in a
        folder of the mod's own models and motions, named by a model that ships, a model that wears a texture of
        the mod's own, or the description and bump pair of a texture that ships."""
        need: dict[str, str] = {}
        refs_cache: dict[str, object] = {}

        def refs(rel):
            if rel not in refs_cache:
                try:
                    refs_cache[rel] = media.ogf_refs(open(self.mod[rel], "rb").read())
                except Exception as e:  # noqa: BLE001 - an unreadable model is reported and keeps nothing
                    refs_cache[rel] = None
                    self.problems.append(f"{rel}: not readable as an .ogf ({e})")
            return refs_cache[rel]

        def named_by(rel):
            got = refs(rel)
            if got is None:
                return []
            tex, mot, mdl = got
            return [f for t in tex for f in media.texture_files(t)] + [f"meshes\\{m}.omf" for m in mot] + \
                [f"meshes\\{m}.ogf" for m in mdl]

        values = []
        for s in set(self.plan.ops) | set(self.plan.new):
            values += [v for v in self.W.resolve(s).values() if isinstance(v, str)]
        for blocks in self.file_blocks.values():
            for _, _, _, Wf, scope in blocks:
                for s in scope:
                    values += [v for v in Wf.resolve(s).values() if isinstance(v, str)]
        for v in values:
            for cand in media.value_paths(v):
                if cand in old:
                    need.setdefault(cand, "a config key the mod sets names it")
        own_dirs = {os.path.dirname(r) for r in own}
        for r in old:
            if r.startswith(("meshes\\", "anims\\")) and os.path.dirname(r) in own_dirs:
                need.setdefault(r, "its folder holds files of the mod's own (models and their motions go together)")
        changed = True
        while changed:
            changed = False
            for r in sorted(own | set(need)):
                if r.endswith(".ogf"):
                    for f in named_by(r):
                        if f in old and f not in need:
                            need[f] = f"{r} names it"
                            changed = True
                for c in media.companions(r):
                    if c in old and c not in need:
                        need[c] = f"it goes with {r}"
                        changed = True
            for r in sorted(old - set(need)):
                if r.endswith(".ogf") and any(f in own for f in named_by(r) if f.startswith("textures\\")):
                    need[r] = "it wears a texture of the mod's own (painted for this model's mapping)"
                    changed = True
        return need

    def references(self):
        """Every model, sound, texture and particle a value of the module names, against the game and the module."""
        self.log("checking what the module's values name")
        shipped = {r[len("gamedata\\"):] for r in self.written if r.startswith("gamedata\\")}
        have = set(self.game.versions) | shipped
        pe, pg = set(), set()
        eff = self.game.effective("particles.xr")
        if eff:
            e, g = particles.read_library(self.game.read(eff))
            pe, pg = {k.lower() for k in e}, {k.lower() for k in g}
        for r in shipped:
            if r.startswith("particles\\"):
                (pe if r.endswith(".pe") else pg).add(r[len("particles\\"):-3])
        values = []  # (where, section, key, value)
        for s in set(self.plan.ops) | set(self.plan.new):
            keys = {op[1] for op in self.plan.ops.get(s, [])} | set(self.plan.new[s].own if s in self.plan.new else {})
            r = self.W.resolve(s)
            values += [("system", s, k, r[k]) for k in keys if isinstance(r.get(k), str)]
        for blocks in self.file_blocks.values():
            for frel, _, _, Wf, scope in blocks:
                for s in scope:
                    if s in Wf.sections:
                        values += [(frel, s, k, v) for k, v in Wf.resolve(s).items() if isinstance(v, str)]
        # a scope texture is an entry of the scope windows, not a file
        scopes = set()
        for x in ("configs\\ui\\scopes.xml", "configs\\ui\\scopes_16.xml"):
            if self.gview.exists(x):
                scopes |= {t.lower() for t in re.findall(rb"<([A-Za-z0-9_]+)[\s>]", self.gview.read(x)) for t in [t.decode()]}
        rus = strings.merged(self.gview, "rus")
        missing = []
        checked = collections.Counter()
        for where, s, k, v in values:
            first = v.split(",")[0].strip().strip('"').lower()
            if not first:
                continue
            if k in ("visual", "item_visual") or k.endswith("_visual"):
                checked["models"] += 1
                if f"meshes\\{first}" not in have and f"meshes\\{first}.ogf" not in have:
                    missing.append((where, s, k, v, "model"))
            elif k.startswith("snd_") and "\\" in first:
                # a path (a word without a folder is a sound theme of the NPC logic); `snd_x1..3` play when `snd_x` is missing
                checked["sounds"] += 1
                if f"sounds\\{first}.ogg" not in have and not any(f"sounds\\{first}{i}.ogg" in have for i in range(1, 4)):
                    missing.append((where, s, k, v, "sound"))
            elif k.endswith("particles") and "\\" in first:
                checked["particles"] += 1
                for p in (x.strip().lower() for x in v.split(",") if "\\" in x):
                    if p not in pe and p not in pg:
                        missing.append((where, s, k, v, "particle"))
                        break
            elif k == "scope_texture":
                checked["scope textures"] += 1
                if first not in scopes and f"textures\\{first}.dds" not in have:
                    missing.append((where, s, k, v, "scope texture"))
            elif k == "icons_texture" or k.endswith("_texture"):
                checked["textures"] += 1
                if f"textures\\{first}.dds" not in have:
                    missing.append((where, s, k, v, "texture"))
            elif k in ("inv_name", "inv_name_short", "description") and where == "system" and s in self.plan.new:
                checked["strings"] += 1
                b = encode(v)
                if b not in rus and not any(b in w for w in self.str_want.values()) and b not in self.inlined:
                    missing.append((where, s, k, v, "string"))
        # the same for every sound the mod's own tree names, so a miss that is no value of the module is listed too
        carried = {(s, k) for _, s, k, _ in values}
        mod_misses = set()
        for s in self.M.sections:
            for k, v in self.M.resolve(s).items():
                if k.startswith("snd_") and isinstance(v, str):
                    first = v.split(",")[0].strip().strip('"').lower()
                    if "\\" in first and f"sounds\\{first}.ogg" not in have and \
                            not any(f"sounds\\{first}{i}.ogg" in have for i in range(1, 4)) and f"sounds\\{first}" not in have:
                        mod_misses.add((first, "the module sets it" if (s, k) in carried else
                                        "the mod's files name it; the module does not carry it (its base's or the game's value)"))
        self.write_text(os.path.join("port", "missing_files.txt"),
                        "# values of the module that name a file neither the game nor the module has\n"
                        "# where\tsection\tkey\tvalue\tkind\n" + "\n".join("\t".join(m) for m in sorted(missing)) + "\n\n"
                        "# sounds the mod's own config tree names that neither the game nor the mod has\n" +
                        "\n".join(f"{n}\t{why}" for n, why in sorted(mod_misses)) + "\n")
        self.rep["references"] = {"checked": dict(checked), "missing": dict(collections.Counter(m[4] for m in missing)),
                                  "mod_tree_sound_misses": sorted(mod_misses)}
        self.log(f"references: {dict(checked)}; missing {self.rep['references']['missing']}")

    def lint_ltxp(self):
        """Every `.ltxp` line is a comment, a section header, a directive or a `#file` line."""
        bad = []
        rx = re.compile(r"^(;.*|\[!?[^\]]+\](\s*:\s*[^;]+)?|[+\-]?[^=;!\[#+\-][^=;]*=[^;]*|![^=;]+|#file(\s+[^;]+)?)$")
        for rel in sorted(r for r in self.written if r.startswith("patch\\") and r.endswith(".ltxp")):
            for i, line in enumerate(decode(open(os.path.join(self.out, rel), "rb").read()).split("\r\n"), 1):
                t = line.strip()
                if t and not rx.match(t):
                    bad.append(f"{rel}:{i}: {t[:80]}")
        self.rep["ltxp_lint"] = bad
        if bad:
            self.problems.append(f".ltxp lines no rule reads: {len(bad)} ({bad[:3]})")

    # ---- the project --------------------------------------------------------------------------------------------
    def project(self):
        manifest = "\r\n".join([
            "[module]",
            f"id          = {self.id}",
            f"name        = \"{self.name}\"" if any(c in self.name for c in ";\"") else f"name        = {self.name}",
            "version     = 1.0.0",
            "api         = 1",
            "mode        = *",
            "target      = any",
            "",
            "[exclude]",
            "PORT_NOTES.md",
            "port",
            "",
        ])
        self.write("mod.ltx", manifest.encode("utf-8"))

    def use_animations(self):
        """The pseudo weapons of the mod's use animations, what only they use, and the mod's item table."""
        tbl, lines = {}, []
        src = self.mod.get("scripts\\enhanced_animations.script")
        if src:
            text = decode(open(src, "rb").read())
            lines = text.splitlines()
            for m in re.finditer(r"^\s*([A-Za-z0-9_]+)\s*=\s*\{([^{}]*\banm\s*=[^{}]*)\}", text, re.M):
                fields = {}
                for k, v in re.findall(r"(\w+)\s*=\s*(\"(?:[^\"\\]|\\.)*\"|[^,}]+)", m.group(2)):
                    v = v.strip()
                    fields[k] = v[1:-1].replace("\\\\", "\\") if v.startswith('"') else v
                tbl[m.group(1).lower()] = fields
        out = []
        for s, status in sorted(self.excluded.items()):
            ms = self.M.sections.get(s)
            entry = {"section": s, "status": status, "kind": "pseudo_weapon" if s in self.own.use_anim else
                     ("hud" if s in self.own.hud_users else "helper"),
                     "parents": ms.parents if ms else [], "own": {k: v for k, v in (ms.own if ms else {}).items()},
                     "file": ms.file if ms else None}
            if status == "changed" and s in self.B.sections:
                bs = self.B.sections[s]
                entry["base_parents"] = bs.parents
                entry["base_own"] = dict(bs.own)
            if s in self.own.use_anim:
                entry["hud"] = norm(self.M.resolve(s).get("hud"))
                entry["used_by_item_table"] = sorted(k for k, f in tbl.items() if f.get("anm", "").lower() == s)
                entry["named_on_lines"] = [i + 1 for i, l in enumerate(lines) if f'"{s}"' in l]
                for pre in ("anm_harv_", "anm_fast_harv_"):
                    if s.startswith(pre):
                        entry["built_by_name"] = f"\"{pre}\" .. <knife section> in enhanced_animations: {s[len(pre):]}"
            out.append(entry)
        data = {"note": "Left out of the module: the mod's use-animation pseudo weapons (class WP_BINOC, `anm_*`), the "
                        "HUD sections only they use and helpers only they inherit. The game's own system plays a use "
                        "scene from data (`items\\items\\animations_settings.ltx`, `item_ea_*_hud`); the next step "
                        "turns these into scenes of that system.",
                "sections": out, "item_table": tbl}
        self.write_text(os.path.join("port", "use_animations.json"), json.dumps(data, ensure_ascii=False, indent=1))
        self.rep["use_animations"] = {"sections": len(out), "item_table_entries": len(tbl),
                                      "by_kind": dict(collections.Counter(e["kind"] for e in out))}

    def motion_files(self):
        """The module's motion files against the game's of the same paths: what `motion_files.py` does, with
        the files it changed written into the ledger as they are now."""
        self.log("motion files against the game's of the same paths")
        done = motion_files.process(self.out, self.game, self.id, say=lambda _line: None)
        by_lower = {rel.lower(): rel for rel in self.written}

        def refresh(rel, was=None):
            old = by_lower.get((was or rel).lower())
            if old is None:
                return
            del self.written[old]
            with open(os.path.join(self.out, rel), "rb") as f:
                data = f.read()
            self.written[rel if was else old] = (hashlib.sha256(data).hexdigest(), len(data))

        for rel in list(done.merged) + done.pointed:
            refresh(rel)
        for was, rel in done.renamed.items():
            refresh(rel, was)
        self.motion_done = done
        self.log(f"motion files: {len(done.merged)} got the game's motions, {len(done.renamed)} of another skeleton "
                 f"renamed, {len(done.pointed)} models pointed at them, {len(done.kept)} left as they are")

    def run(self):
        os.makedirs(self.out, exist_ok=True)
        self.prepare_out()
        os.makedirs(self.port, exist_ok=True)
        # the check reports are written as the checks run and listed at the end
        for f in ("check_tree.txt", "check_files.txt"):
            if os.path.exists(os.path.join(self.port, f)):
                os.remove(os.path.join(self.port, f))
        self.scan()
        self.pick_bases()
        self.views()
        self.config_tree()
        self.outside_configs()
        self.write_strings()
        self.xml_and_scripts()
        self.particles()
        self.use_animations()
        self.media()
        self.references()
        self.motion_files()
        self.lint_ltxp()
        self.project()
        notes.write(self)
        for f in ("check_tree.txt", "check_files.txt"):
            p = os.path.join(self.port, f)
            if os.path.exists(p):
                data = open(p, "rb").read()
                self.written[os.path.join("port", f)] = (hashlib.sha256(data).hexdigest(), len(data))
        self.finish_ledger()
        self.game.close()
        if self.conflicts:
            self.problems.append(f"{len(self.conflicts)} files were in the way and not overwritten: {self.conflicts[:10]}")
        self.log(f"done: {len(self.written)} files written; problems: {len(self.problems)}")
        for p in self.problems:
            print("  ! " + p)
        return 1 if any("check" in p or "in the way" in p or "read back" in p for p in self.problems) else 0


PATCH_TOPICS = {
    "creatures": "Monsters and stalkers: the mod's own values in their sections.",
    "squads": "Squads: the mod's own values.",
    "actor": "The player: the mod's own values.",
    "immunities": "Outfit immunity sections: new ones and the mod's own values.",
    "upgrades": "Upgrade sections: the mod's own values.",
    "hud": "HUD sections no item names: the mod's own values.",
    "sounds": "Sound sections no item owns: new ones and the mod's own values.",
    "other": "Sections that belong to no item: new ones and the mod's own values.",
}
FILE_TOPICS = {
    "trade": "Traders (#file): the mod's own lines in the game's trade configs.",
    "tasks": "Tasks (#file): the mod's own lines in the game's task configs.",
    "smart_logic": "Smart terrain and NPC logic (#file): the mod's own lines.",
    "loot": "Death loot (#file): the mod's own lines.",
    "plugins": "Plugins (#file): the mod's own lines.",
    "item_scenes": "Item use scenes of the game (#file): the mod's own lines.",
    "pda3d": "The 3D PDA (#file): the mod's own lines.",
    "files": "Other configs opened by name (#file): the mod's own lines.",
}


def file_topic(rel):
    r = rel[len("configs\\"):]
    if r.startswith("misc\\trade\\"):
        return "trade"
    if r.startswith("misc\\tm_") or r.startswith("misc\\task"):
        return "tasks"
    if r.startswith("scripts\\"):
        return "smart_logic"
    if r.startswith("misc\\death"):
        return "loot"
    if r.startswith("plugins\\"):
        return "plugins"
    if r.startswith("items\\items\\"):
        return "item_scenes"
    if r.startswith("dead_air_x64_pda3d"):
        return "pda3d"
    return "files"


def config_path(path: str) -> str:
    """`ConfigPath`: lower case, backslashes, `.` and `..` worked out."""
    parts = []
    for p in path.lower().replace("/", "\\").split("\\"):
        if p == "..":
            if not parts:
                return ""
            parts.pop()
        elif p and p != ".":
            parts.append(p)
    return "\\".join(parts)


def apply_ltxp(data: bytes, comp, where, system=True, only_file=None, system_files=()):
    """A `.ltxp` file the way `xms_ltxp.cpp` reads it: directives to the system config until a `#file <path>` sends
    them to that config (`only_file`, a path under configs); a `#file` naming a file the system config is read from
    (`system_files`) is a patch of the system config too."""
    target = None
    section, valid = None, False
    for raw in decode(data).split("\n"):
        line = raw.split(";", 1)[0].strip(" \t\r")
        if not line:
            continue
        if len(line) >= 5 and line[0] == "#" and line[1:5].lower() == "file" and (len(line) == 5 or line[5] in " \t"):
            rest = line[5:].strip(" \t")
            target = config_path(rest) if rest else None
            section, valid = None, False
            continue
        if system:
            mine = target is None or target in system_files
        else:
            mine = target == config_path(only_file or "")
        if line[0] == "[":
            close = line.find("]")
            if close < 0:
                continue
            name = line[1:close].strip(" \t")
            rest = line[close + 1:].strip(" \t")
            parents = [p.strip(" \t").lower() for p in rest[1:].split(",") if p.strip(" \t")] if rest.startswith(":") else []
            if name.startswith("!"):
                if mine:
                    comp.delete(name[1:].strip(" \t").lower())
                valid = False
                continue
            section = name.lower()
            valid = comp.create(section, parents, {}, where) if mine else False
            continue
        if not (mine and valid):
            continue
        eq = line.find("=")
        if eq >= 0:
            key, value = line[:eq].strip(" \t"), line[eq + 1:].strip(" \t")
            if key.startswith("+"):
                comp.listop("add", section, key[1:].strip(" \t"), [x.strip() for x in value.split(",") if x.strip()])
            elif key.startswith("-"):
                comp.listop("drop", section, key[1:].strip(" \t"), [x.strip() for x in value.split(",") if x.strip()])
            elif key:
                comp.set(section, key, value)
        elif line.startswith("!"):
            comp.unset(section, line[1:].strip(" \t"))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mod", required=True, help="the classic mod's gamedata folder")
    ap.add_argument("--game", required=True, help="the game's root (database\\, gamedata\\, tools\\stalker-tools)")
    ap.add_argument("--out", required=True, help="the project folder to write")
    ap.add_argument("--id", required=True, help="the module id")
    ap.add_argument("--name", help="the module's title")
    ap.add_argument("--base", help="what the mod was made from: archive names (comma separated) or a folder")
    args = ap.parse_args()
    if not re.match(r"^[a-z][a-z0-9_]*$", args.id):
        print("the module id is a-z 0-9 _ and starts with a letter")
        return 2
    out = os.path.join(os.path.abspath(args.out), "").lower()
    for p in (args.mod, args.game):
        inner = os.path.join(os.path.abspath(p), "").lower()
        if out.startswith(inner) or inner.startswith(out):
            print("the project folder may not lie inside the mod or the game")
            return 2
    try:
        return Converter(args).run()
    except Failure as e:
        print("! " + str(e))
        return 2


if __name__ == "__main__":
    sys.exit(main())
