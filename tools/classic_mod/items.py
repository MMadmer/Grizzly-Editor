"""Which sections are inventory items, who owns the others, and the `.itemasset` files (`docs/assets/ITEM.md`)."""
from __future__ import annotations

import os
import re

import luadata
from ltx import REMOVED, Config, Sect, norm

# Every `class =` value of an inventory item, by family (the item class table of the Dead Air engine:
# `object_factory_register.cpp` and `class_registrator.script`).
FAMILIES = {
    "ammo": "AMMO AMMO_S A_M209 A_OG7B A_VOG25 S_M209 S_OG7B S_VOG25",
    "artefact": "AF_BAST AF_BDROP AF_BGRAV AF_CTA AF_DUMMY AF_EBALL AF_FBALL AF_GALAN AF_GRAVI AF_MBALL AF_NEEDL "
                "AF_RHAIR AF_THORN AF_ZUDA ARTEFACT SCRPTART",
    "backpack": "EQ_BAKPK",
    "bolt": "II_BOLT",
    "detector": "DET_ADVA DET_ELIT DET_SCIE DET_SIMP D_ADVANC D_ELITE D_SCIENT D_SIMDET",
    "device": "D_FLARE WP_BINOC W_BINOC",
    "eatable": "II_BOTTL II_FOOD S_FOOD",
    "grenade": "G_F1 G_F1_S G_RGD5 G_RGD5_S",
    "grenade_launcher": "WP_GLAUN W_GLAUNC",
    "helmet": "EQ_HLMET E_HLMET",
    "item": "II_ATTCH II_DOC II_EXPLO S_EXPLO",
    "knife": "WP_KNIFE W_KNIFE",
    "launcher": "WP_RG6 WP_RPG7 W_RG6 W_RPG7",
    "medicine": "II_ANTIR II_BANDG II_MEDKI",
    "other": "G_FAKE G_RPG7 MP_PLBAG",
    "outfit": "EQU_EXO EQU_MLTR EQU_SCIE EQU_STLK E_STLK",
    "pda": "D_PDA S_PDA WP_PDA3D",
    "scope": "WP_SCOPE W_SCOPE",
    "shotgun": "WP_ASHTG WP_BM16 WP_SHOTG W_BM16 W_SHOTGN",
    "silencer": "WP_SILEN W_SILENC",
    "torch": "D_TORCH TORCH_S",
    "weapon": "WP_AK74 WP_GROZA WP_HPSA WP_LR300 WP_PM WP_SVD WP_SVU WP_USP45 WP_VAL WP_VINT WP_WALTH W_AK74 "
              "W_FN2000 W_FORT W_GROZA W_HPSA W_LR300 W_PM W_SVD W_SVU W_USP45 W_VAL W_VINT W_WALTHR W_WMAGAZ W_WMAGGL",
}
CLASS_FAMILY = {cls: fam for fam, names in FAMILIES.items() for cls in names.split()}
ICON_KEYS = ("inv_grid_x", "inv_grid_y", "inv_grid_width", "inv_grid_height", "icons_texture")
TEXT_KEYS = (("name", "inv_name"), ("short_name", "inv_name_short"), ("description", "description"))
_JUNK_KEY = re.compile(r"^[^A-Za-z0-9]*$")


def family_of(resolved: dict):
    cls = norm(resolved.get("class"))
    return CLASS_FAMILY.get(cls) if cls else None


def junk_key(k: str) -> bool:
    """A line the parser kept as a key that names nothing: `-----`, a bare `:`."""
    return bool(_JUNK_KEY.match(k))


class Owners:
    """Items of the wanted state, the use-animation sections left out, and who owns every other section."""

    def __init__(self, W: Config, M: Config, G: Config):
        self.W, self.M, self.G = W, M, G
        self.items: dict[str, str] = {}
        for n in W.sections:
            f = family_of(W.resolve(n))
            if f:
                self.items[n] = f
        # the use-animation pseudo weapons of the mod's own item-use system
        self.use_anim = {n for n in M.sections
                         if n.startswith("anm_") and norm(M.resolve(n).get("class")) in ("WP_BINOC", "W_BINOC")}
        kids: dict[str, set] = {}
        for c in (M, W):
            for p, ch in c.children().items():
                kids.setdefault(p, set()).update(ch)
        self.kids = kids
        self.hud_users: dict[str, set] = {}
        for c in (W, M):
            for n in c.sections:
                if n in self.items or n in self.use_anim:
                    h = norm(c.resolve(n).get("hud"))
                    if h:
                        self.hud_users.setdefault(h.lower(), set()).add(n)

    def descendants(self, n):
        out, stack = set(), list(self.kids.get(n, ()))
        while stack:
            c = stack.pop()
            if c not in out:
                out.add(c)
                stack.extend(self.kids.get(c, ()))
        return out

    def is_item(self, n):
        return n in self.items and n not in self.use_anim

    def hud_owner(self, h):
        users = sorted(u for u in self.hud_users.get(h, ()) if self.is_item(u))
        if not users:
            return None
        if h.endswith("_hud") and h[:-4] in users:
            return h[:-4]
        return users[0]

    def excluded(self, n) -> bool:
        """A use-animation section, a HUD section only they use, or a helper only they inherit."""
        if n in self.use_anim:
            return True
        if n in self.items:
            return False
        users = self.hud_users.get(n, set())
        if users and all(u in self.use_anim for u in users):
            return True
        if users:
            return False
        d = self.descendants(n)
        return bool(d) and all(self.excluded_leaf(x) for x in d)

    def excluded_leaf(self, n):
        if n in self.use_anim:
            return True
        users = self.hud_users.get(n, set())
        return bool(users) and all(u in self.use_anim for u in users)

    def owner(self, n):
        """("item", item, "main"|"hud"|"sections") or ("patch", None, None)."""
        if self.is_item(n):
            return ("item", n, "main")
        h = self.hud_owner(n)
        if h:
            return ("item", h, "hud")
        d = self.descendants(n)
        items, foreign = set(), False
        for x in d:
            if self.is_item(x):
                items.add(x)
            elif x in self.use_anim:
                continue
            elif self.hud_owner(x):
                items.add(self.hud_owner(x))
            elif x in self.hud_users or self.excluded_leaf(x):
                continue
            elif not self.kids.get(x):
                foreign = True
        if items and not foreign:
            by_name = sorted((i for i in items if n.startswith(i + "_")), key=len, reverse=True)
            return ("item", by_name[0] if by_name else sorted(items)[0], "sections")
        return ("patch", None, None)

    def group(self, n, fam):
        parts = n.split("_")
        for k in range(1, len(parts)):
            cand = "_".join(parts[:k])
            if cand in self.items and self.items[cand] == fam:
                return cand
        return n


def topic_of(name: str, resolved: dict) -> str:
    cls = norm(resolved.get("class")) or ""
    if cls.startswith(("SM_", "AI_")) or cls in ("C_HLCP_S",):
        return "creatures"
    if cls == "ON_OFF_S" or name.startswith("simulation_"):
        return "squads"
    if cls in ("S_ACTOR",) or name.startswith("actor"):
        return "actor"
    if "immunit" in name:
        return "immunities"
    if name.startswith(("up_", "upgr")) or "_up_" in name:
        return "upgrades"
    if name.endswith("_hud") or "_hud_" in name:
        return "hud"
    if "sound" in name or name.endswith("_snd"):
        return "sounds"
    return "other"


# ---- the asset ------------------------------------------------------------------------------------------
class ItemSpec:
    def __init__(self, section, new, family, group):
        self.section, self.new, self.family, self.group = section, new, family, group
        self.main_new: Sect | None = None
        self.main_ops: list = []
        self.texts: dict[str, dict[str, str]] = {}
        self.icon = None            # (file relative to the asset, w, h)
        self.icon_png = None        # project-relative path of the picture
        self.hud_new: Sect | None = None
        self.hud_ops: list = []
        self.hud_name = None
        self.sections: list = []    # ("new", Sect) | ("edit", name, ops)
        self.drop_keys: set = set()

    @property
    def path(self):
        return os.path.join("items", self.family, self.group, self.section + ".itemasset")

    def empty(self):
        return not (self.main_new or self.main_ops or self.icon or self.hud_new or self.hud_ops or self.sections or self.texts)


_ID = re.compile(r"^[a-z_][a-z0-9_]{0,47}$")


def ops_tables(ops) -> dict:
    out = {}
    sets = {k: luadata.typed(v) for kind, k, *rest in ops if kind == "set" for v in rest}
    unset = [k for kind, k, *rest in ops if kind == "unset"]
    add = {k: list(rest[0]) for kind, k, *rest in ops if kind == "add"}
    drop = {k: list(rest[0]) for kind, k, *rest in ops if kind == "drop"}
    if sets:
        out["set"] = sets
    if unset:
        out["unset"] = unset
    if add:
        out["add"] = add
    if drop:
        out["drop"] = drop
    return out


def base_value(parents):
    return parents[0] if len(parents) == 1 else list(parents)


def own_table(own: dict, drop=()) -> dict:
    return {k: luadata.typed(v) for k, v in own.items() if k not in drop and v is not REMOVED}


def asset_table(spec: ItemSpec) -> dict:
    t: dict = {"item": 1}
    if spec.new:
        stem = spec.section
        if not _ID.match(stem):
            t["id"] = re.sub(r"[^a-z0-9_]", "_", stem.lower()).lstrip("0123456789")[:48] or "item"
        t["section"] = spec.section
        if spec.main_new.parents:
            t["base"] = base_value(spec.main_new.parents)
    else:
        t["edit"] = spec.section
    for field, _key in TEXT_KEYS:
        texts = spec.texts.get(field)
        if texts:
            t[field] = texts["rus"] if list(texts) == ["rus"] else dict(texts)
    if spec.icon:
        t["icon"] = luadata.Inline(file=spec.icon[0], w=spec.icon[1], h=spec.icon[2])
    if spec.new:
        st = own_table(spec.main_new.own, spec.drop_keys)
        if st:
            t["set"] = st
    else:
        t.update(ops_tables(spec.main_ops))
    if spec.hud_new is not None:
        h = {}
        if spec.hud_new.parents:
            h["base"] = base_value(spec.hud_new.parents)
        st = own_table(spec.hud_new.own)
        if st:
            h["set"] = st
        t["hud"] = h
    elif spec.hud_ops:
        t["hud"] = ops_tables(spec.hud_ops)
    rows = []
    for entry in spec.sections:
        if entry[0] == "new":
            s = entry[1]
            r = {"name": s.name}
            if s.parents:
                r["base"] = base_value(s.parents)
            st = own_table(s.own)
            if st:
                r["set"] = st
        else:
            r = {"edit": entry[1]}
            r.update(ops_tables(entry[2]))
        rows.append(r)
    if rows:
        t["sections"] = rows
    return t




# ---- the game's reading of an asset (`xms_items.cpp`): what it refuses, and what it makes, in its order -----
SECTION_NAME = re.compile(r"^[a-z0-9_.\-]{1,63}$")
ALLOWED_TOP = {"item", "id", "section", "base", "abstract", "edit", "name", "short_name", "description", "icon",
               "set", "unset", "add", "drop", "hud", "sections", "scene", "editor"}
# what another file owns: an item that says it is refused by the editor's check, and the game never reads it
FOREIGN_TOP = {"sold": "a trader file (.tradeasset)", "loot": "a loot table (.lootasset)",
               "craft": "a recipes file (.recipeasset)", "lists": "a patch (.ltxp), or scene.set for a use scene's own keys"}


def plain_name(name) -> bool:
    """A section or a key a config line can hold."""
    return isinstance(name, str) and 0 < len(name) <= 255 and \
        not any(ord(c) < 0x21 or ord(c) == 0x7F or c in '[]=;,"' for c in name)


def plain_value(text: str) -> bool:
    """No line break, no other control character."""
    return not any((ord(c) < 0x20 and c != "\t") or ord(c) == 0x7F for c in text)


def value_text(v):
    """`ItemValueText`: a string as written, a number without needless decimals, a list joined by `, `."""
    if isinstance(v, str):
        return v
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, luadata.Raw):
        n = float(v.text)
        return str(int(n)) if n == int(n) and abs(n) < 1e15 else format(n, ".15g")
    if isinstance(v, list):
        parts = [value_text(x) for x in v]
        return None if any(p is None or isinstance(x, list) for p, x in zip(parts, v)) else ", ".join(parts)
    return None


class GamePlan:
    def __init__(self, file):
        self.file = file
        self.edit = False
        self.section = ""
        self.parents: list[str] = []
        self.abstract = False
        self.own: list[tuple] = []
        self.texts: dict[str, object] = {}
        self.icon_picture = False
        self.hud = False
        self.hud_parents: list[str] = []
        self.hud_parents_given = False
        self.hud_ops: list[tuple] = []
        self.sections: list[tuple] = []   # (make, name, parents, ops)


def _changes(t, where):
    ops = []
    st = t.get("set")
    if st is not None:
        if not isinstance(st, dict):
            raise ValueError(where + "set takes key = value")
        for k, v in st.items():
            text = value_text(v)
            if not plain_name(k):
                raise ValueError(f"{where}'{k}' cannot be a key of a config")
            if text is None:
                raise ValueError(f"{where}the value of {k} is no string, number, boolean or list of them")
            if not plain_value(text):
                raise ValueError(f"{where}the value of {k} holds characters no config can carry")
            ops.append(("set", k, text))
    un = t.get("unset")
    if un is not None:
        keys = [un] if isinstance(un, str) else un
        if not isinstance(keys, list) or not all(isinstance(k, str) for k in keys):
            raise ValueError(where + "unset is a list of keys")
        for k in keys:
            if not plain_name(k):
                raise ValueError(f"{where}'{k}' cannot be a key of a config")
            ops.append(("unset", k))
    for which in ("add", "drop"):
        lists = t.get(which)
        if lists is None:
            continue
        if not isinstance(lists, dict):
            raise ValueError(f"{where}{which} takes key = member or {{ members }}")
        for k, v in lists.items():
            members = [value_text(x) for x in (v if isinstance(v, list) else [v])]
            if not plain_name(k):
                raise ValueError(f"{where}'{k}' cannot be a key of a config")
            if any(m is None or not m or not plain_value(m) or "," in m for m in members):
                raise ValueError(f"{where}{which}.{k} holds a member no list can carry")
            ops.append((which, k, members))
    return ops


def _names(v, what):
    if v is None:
        return []
    names = [v] if isinstance(v, str) else v
    if not isinstance(names, list) or not all(isinstance(x, str) for x in names):
        raise ValueError(f"{what} is a section or a list of them")
    out = [x.lower() for x in names]
    if not all(plain_name(x) for x in out):
        raise ValueError(f"{what} names a section no config can hold")
    return out


def game_plan(t, module: str, file: str) -> GamePlan:
    """`PlanItem`: raises ValueError with the game's reason for a file it leaves out whole."""
    if not isinstance(t, dict):
        raise ValueError("an item file returns one table")
    p = GamePlan(file)
    if "edit" in t:
        if any(k in t for k in ("id", "section", "base", "abstract")):
            raise ValueError("a change names its section with edit alone")
        if not isinstance(t["edit"], str) or not plain_name(t["edit"].lower()):
            raise ValueError("edit names the section it changes")
        p.edit, p.section = True, t["edit"].lower()
    else:
        sid = t.get("id")
        if sid is None:
            name = re.sub(r"[^a-z0-9_]", "_", os.path.splitext(os.path.basename(file))[0].lower())
        else:
            name = sid if isinstance(sid, str) else ""
        if (sid is not None or "section" not in t) and not re.match(r"^[a-z_][a-z0-9_]{0,47}$", name):
            raise ValueError("the id is a-z, 0-9 and _, up to 48, not starting with a digit")
        sec = t.get("section")
        if sec is not None and not isinstance(sec, str):
            raise ValueError("the section is a-z, 0-9, _, . and -, under 64 characters")
        p.section = sec.lower() if sec else module + "_" + name
        if not SECTION_NAME.match(p.section):
            raise ValueError(f"the section [{p.section}] is not a-z, 0-9, _, . and -, under 64 characters")
        if "abstract" in t:
            if not isinstance(t["abstract"], bool):
                raise ValueError("abstract is true or false")
            p.abstract = t["abstract"]
        p.parents = _names(t.get("base"), "base")
        if not p.parents and not p.abstract:
            raise ValueError("a new item starts from a base")
    p.own = _changes(t, "")
    if p.abstract:
        return p
    for field, key, suffix in (("name", "inv_name", "_name"), ("short_name", "inv_name_short", "_short"),
                               ("description", "description", "_descr")):
        v = t.get(field)
        if v is None:
            continue
        if not (isinstance(v, str) or (isinstance(v, dict) and any(isinstance(x, str) for x in v.values()))):
            raise ValueError(f"{field} is a string or {{ language = text }}")
        p.texts[field] = v
    if "name" in p.texts:
        p.own.append(("set", "inv_name", p.section + "_name"))
    if "short_name" in p.texts:
        p.own.append(("set", "inv_name_short", p.section + "_short"))
    elif "name" in p.texts:
        p.own.append(("set", "inv_name_short", p.section + "_name"))
    if "description" in p.texts:
        p.own.append(("set", "description", p.section + "_descr"))
    icon = t.get("icon")
    if icon is not None:
        if isinstance(icon, str) or (isinstance(icon, dict) and "file" in icon):
            p.icon_picture = True
        elif isinstance(icon, dict) and isinstance(icon.get("x"), luadata.Raw) and isinstance(icon.get("y"), luadata.Raw):
            if "texture" in icon:
                p.own.append(("set", "icons_texture", icon["texture"]))
            else:
                p.own.append(("unset", "icons_texture"))
            for f, k in (("x", "inv_grid_x"), ("y", "inv_grid_y"), ("w", "inv_grid_width"), ("h", "inv_grid_height")):
                if f in icon:
                    p.own.append(("set", k, value_text(icon[f])))
        else:
            raise ValueError("icon is { texture, x, y, w, h }, in cells")
    hud = t.get("hud")
    if hud is not None:
        if not isinstance(hud, dict):
            raise ValueError("hud is { base, set, unset, add, drop }")
        if "base" in hud:
            if p.edit:
                raise ValueError("hud of a change changes the section the item's hud names: base belongs to a new item's hud")
            p.hud_parents = _names(hud["base"], "hud.base")
            p.hud_parents_given = True
        p.hud_ops = _changes(hud, "hud: ")
        p.hud = True
        if not p.edit and "hud" not in (t.get("set") or {}):
            p.own.append(("set", "hud", p.section + "_hud"))
    rows = t.get("sections")
    if rows is not None:
        if not isinstance(rows, list):
            raise ValueError("sections is a list of { name, base, set } and { edit, set }")
        for r in rows:
            if not isinstance(r, dict) or ("name" in r) == ("edit" in r):
                raise ValueError("a row of sections has a name (a section it makes) or edit (one it changes)")
            make = "name" in r
            name = (r["name"] if make else r["edit"])
            if not isinstance(name, str):
                raise ValueError("a row of sections has a name (a section it makes) or edit (one it changes)")
            name = name.lower()
            if not (SECTION_NAME.match(name) if make else plain_name(name)):
                raise ValueError(f"the section [{name}] is not a-z, 0-9, _, . and -, under 64 characters")
            if "base" in r and not make:
                raise ValueError(f"[{name}] exists and keeps its parents: base belongs to a section a row makes")
            p.sections.append((make, name, _names(r.get("base"), f"the base of [{name}]"), _changes(r, f"[{name}]: ")))
    return p


def _recipes(t) -> list[tuple]:
    """`PlanTable` of a recipes file: `(at, makes, count, parts)`, or what the game refuses the file for."""
    rows = t.get("recipes")
    if rows is None:
        return []
    if not isinstance(rows, list):
        raise ValueError("recipes is a list of { makes, count, at, parts }")
    out = []
    for r in rows:
        if not isinstance(r, dict) or not isinstance(r.get("makes"), str) or not plain_name(r["makes"].lower()):
            raise ValueError("a recipe names what it makes: makes")
        if not isinstance(r.get("at"), str) or not plain_name(r["at"].lower()):
            raise ValueError("a recipe names the tool or workbench it is made at: at")
        parts = _names(r.get("parts"), "parts")
        if not 1 <= len(parts) <= 4:
            raise ValueError("a recipe takes one to four parts")
        count = int(float(r["count"].text)) if isinstance(r.get("count"), luadata.Raw) else 1
        if not 1 <= count <= 1000:
            raise ValueError("a recipe's count is a whole number from 1 to 1000")
        out.append((r["at"].lower(), r["makes"].lower(), count, parts))
    return out


def check_recipes(t, path) -> list[str]:
    """What the game refuses a recipes file for (the whole file), and the contract's keys."""
    if not isinstance(t, dict):
        return [f"{path}: a recipes file returns one table"]
    bad = [f"{path}: '{k}' is no key of a recipes file" for k in t if k not in ("recipe", "recipes", "editor")]
    for r in t.get("recipes") or []:
        if isinstance(r, dict):
            bad += [f"{path}: '{k}' is no key of a recipe" for k in r if k not in ("makes", "count", "at", "parts")]
    try:
        _recipes(t)
    except ValueError as e:
        bad.append(f"{path}: {e}")
    return bad


def apply_recipes(t, comp):
    """`BuildTables` for a recipes file: its rows into `[xms_craft]`, numbered on from what the section holds."""
    try:
        rows = _recipes(t)
    except ValueError:
        return
    sect = comp.c.sections.get("xms_craft")
    nums = [int(k) for k in (sect.own if sect else {}) if k.isdigit()]
    n = max(nums) + 1 if nums else 1
    for at, makes, count, parts in rows:
        comp.set("xms_craft", "%06d" % n, ", ".join([at, makes, str(count)] + parts))
        n += 1


def check_table(t, path) -> list[str]:
    """What the game refuses (the whole item), and the contract's top-level keys."""
    bad = []
    if isinstance(t, dict):
        bad += [f"{path}: '{k}' is not an item's to say: it is {FOREIGN_TOP[k]}" for k in t if k in FOREIGN_TOP]
        bad += [f"{path}: unknown key {k}" for k in t if k not in ALLOWED_TOP and k not in FOREIGN_TOP]
        if not (isinstance(t.get("item"), luadata.Raw) and t["item"].text == "1"):
            bad.append(f"{path}: item = 1 missing")
    try:
        game_plan(t, "module", path)
    except ValueError as e:
        bad.append(f"{path}: left out by the game - {e}")
    return bad


def _ready(comp, p: GamePlan):
    have = comp.c.sections.__contains__
    if not p.edit:
        if have(p.section):
            return "refuse", f"the section [{p.section}] exists already: write it as a change, with edit"
        if p.hud and have(p.section + "_hud"):
            return "refuse", f"the section [{p.section}_hud] exists already"
    elif not have(p.section):
        return "refuse", f"it changes [{p.section}], which is not there"
    elif p.hud:
        hud = norm(comp.c.resolve(p.section).get("hud"))
        for op in p.own:
            if op[0] == "set" and op[1] == "hud":
                hud = op[2]
        if not hud or not have(hud):
            return "refuse", f"it changes the hud of [{p.section}], which names no hud section"
    for make, name, parents, ops in p.sections:
        if make and (have(name) or name == p.section):
            return "refuse", f"the section [{name}] exists already"
        if not make and not have(name):
            return "refuse", f"it changes [{name}], which is not there"
    missing = [(p.section, b) for b in p.parents if not have(b)] if not p.edit else []
    if p.hud and p.hud_parents_given:
        missing += [(p.section + "_hud", b) for b in p.hud_parents if not have(b)]
    for make, name, parents, ops in p.sections:
        if make:
            missing += [(name, b) for b in parents if not have(b)]
    if missing:
        return "wait", missing
    return "go", None


def _apply_to(comp, section, parents, ops, where):
    if section not in comp.c.sections:
        comp.c.add(Sect(section, parents, {}, where))
    for op in ops:
        if op[0] == "set":
            comp.set(section, op[1], op[2])
        elif op[0] == "unset":
            comp.unset(section, op[1])
        else:
            comp.listop(op[0], section, op[1], op[2])


def _apply(comp, p: GamePlan):
    if not p.edit:
        base_hud = ""
        for b in p.parents:
            h = norm(comp.c.resolve(b).get("hud"))
            if h:
                base_hud = h
        _apply_to(comp, p.section, p.parents, p.own, p.file)
        if p.hud:
            parents = p.hud_parents if p.hud_parents_given else ([base_hud] if base_hud in comp.c.sections else [])
            _apply_to(comp, p.section + "_hud", parents, p.hud_ops, p.file)
    else:
        _apply_to(comp, p.section, [], p.own, p.file)
        if p.hud:
            _apply_to(comp, norm(comp.c.resolve(p.section).get("hud")), [], p.hud_ops, p.file)
    for make, name, parents, ops in p.sections:
        _apply_to(comp, name, parents if make else [], ops, p.file)


def apply_items(assets, comp, module):
    """`BuildItems`: the new items, each waiting for its bases; then the changes in the order of their paths.
    Returns the refused items as (file, why, missing) - `missing` the (section, parent) pairs it waited for."""
    plans, refused = [], []
    for rel, t in sorted(assets, key=lambda a: a[0].lower()):
        try:
            plans.append(game_plan(t, module, rel))
        except ValueError as e:
            refused.append((rel, str(e), []))
    fresh = [p for p in plans if not p.edit]
    changes = [p for p in plans if p.edit]
    progress = True
    while progress and fresh:
        progress = False
        i = 0
        while i < len(fresh):
            state, why = _ready(comp, fresh[i])
            if state == "wait":
                i += 1
                continue
            if state == "refuse":
                refused.append((fresh[i].file, why, []))
            else:
                _apply(comp, fresh[i])
            fresh.pop(i)
            progress = True
    for p in fresh:
        state, why = _ready(comp, p)
        refused.append((p.file, "an unknown base", why if state == "wait" else []))
    for p in changes:
        state, why = _ready(comp, p)
        if state == "go":
            _apply(comp, p)
        else:
            refused.append((p.file, why if state == "refuse" else "an unknown base", why if state == "wait" else []))
    return refused, plans
