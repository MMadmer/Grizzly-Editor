"""The mod's intent on resolved config values, the wanted state, the directives that reach it, and the check.

Three configs: `base` (the files the mod was made from), `mod` (the mod's files over that game) and `game`
(today's game). The intent is where mod != base; the wanted state is today's game with that intent pinned on
it and the mod's new sections added; the plan is the fewest directives that take the game there under the
composition the game's config engine does (a section's keys come from its own lines, else from the last parent that has them), written parents first so a change of a mixin is one line.
"""
from __future__ import annotations

import re

from ltx import REMOVED, Config, Sect, items_of, norm

ABSENT = object()


def val(resolved: dict, key: str):
    return norm(resolved[key]) if key in resolved else ABSENT


class Intent:
    def __init__(self):
        self.pins: dict[str, dict[str, object]] = {}   # existing section -> key -> value | REMOVED
        self.new: dict[str, Sect] = {}                  # sections the module makes
        self.conflicts: list[tuple] = []                # (section, key, base, game, mod)
        self.rollbacks: list[tuple] = []                # (section, key) the mod only carries from its base
        self.deleted_by_mod: list[str] = []
        self.game_deleted_mod_changed: list[str] = []
        self.game_deleted_unchanged: list[str] = []
        self.both_added: list[str] = []
        self.lost_game_additions: list[str] = []         # sections the game added since the base (kept)
        self.removed_keys: list[tuple] = []             # (section, key) the mod takes away

    def pin(self, sect, key, value):
        self.pins.setdefault(sect, {})[key] = value


def three_way(B: Config, M: Config, G: Config, scope=None) -> Intent:
    """`scope`: the section names to judge (all of the three when None)."""
    it = Intent()
    names = set(M.sections) | set(B.sections) | set(G.sections)
    if scope is not None:
        names &= set(scope)
    for s in sorted(names):
        in_m, in_b, in_g = s in M.sections, s in B.sections, s in G.sections
        if in_m and in_g and in_b:
            mr, br, gr = M.resolve(s), B.resolve(s), G.resolve(s)
            for k in sorted(set(mr) | set(br)):
                mv, bv = val(mr, k), val(br, k)
                gv = val(gr, k)
                if mv != bv:
                    it.pin(s, k, REMOVED if mv is ABSENT else mv)
                    if mv is ABSENT:
                        it.removed_keys.append((s, k))
                    if gv != bv and gv != mv:
                        it.conflicts.append((s, k, bv, gv, mv))
                elif gv != mv:
                    it.rollbacks.append((s, k))
        elif in_m and in_g:
            it.both_added.append(s)
            mr, gr = M.resolve(s), G.resolve(s)
            for k in sorted(set(mr) | set(gr)):
                mv = val(mr, k)
                it.pin(s, k, REMOVED if mv is ABSENT else mv)
        elif in_m:
            if in_b and M.resolve(s) == B.resolve(s):
                it.game_deleted_unchanged.append(s)
                continue
            if in_b:
                it.game_deleted_mod_changed.append(s)
            src = M.sections[s]
            it.new[s] = Sect(s, src.parents, {k: norm(v) for k, v in src.own.items()}, src.file)
        elif in_b and in_g:
            it.deleted_by_mod.append(s)
        elif in_g and not in_b:
            it.lost_game_additions.append(s)
    return it


def wanted(G: Config, it: Intent, fallback: Config | None = None) -> Config:
    """Today's game with the intent pinned (a pin is an own line of the section) and the new sections."""
    W = G.copy()
    W.fallback = fallback if fallback is not None else G.fallback
    for s, keys in it.pins.items():
        sect = W.sections.get(s)
        if sect is None:
            continue
        for k, v in keys.items():
            sect.own[k] = v
    for s, sect in it.new.items():
        W.add(sect.copy())
    W.invalidate()
    return W


# ---- the directives -----------------------------------------------------------------------------------
_NUMERIC = re.compile(r"^-?(\d+\.?\d*|\.\d+)$")


def list_op(cur, want, is_section):
    """`+key` / `-key` when the wanted list is the current one with members appended, or with members taken out,
    and the list is one of sections (`ammo_class`, `scopes`, `upgrades`): a motion with its speed is no list."""
    if cur is ABSENT or cur is None or want is None or want is ABSENT:
        return None
    if "," not in cur and "," not in want:
        return None
    a, b = items_of(cur), items_of(want)
    if not a or not b or len(set(a)) != len(a) or len(set(b)) != len(b):
        return None
    if any(_NUMERIC.match(x) or '"' in x or not is_section(x.lower()) for x in a + b):
        return None
    if len(b) > len(a) and b[:len(a)] == a and not set(b[len(a):]) & set(a):
        return ("add", b[len(a):])
    if len(b) < len(a) and set(b) <= set(a) and [x for x in a if x in b] == b:
        return ("drop", [x for x in a if x not in b])
    return None


def joined(op_kind, cur, members):
    a = items_of(cur if cur is not ABSENT else None)
    if op_kind == "add":
        for m in members:
            if m not in a:
                a.append(m)
    else:
        a = [x for x in a if x not in members]
    return ",".join(a)


class Plan:
    def __init__(self):
        self.ops: dict[str, list[tuple]] = {}     # existing section -> [("set", k, v) | ("unset", k) | ("add"|"drop", k, [..])]
        self.new: dict[str, Sect] = {}             # sections to make, with their own lines as the mod wrote them


def plan(G: Config, W: Config, it: Intent, use_lists=True) -> Plan:
    """Parents first: a section gets a directive only where the game composed with the directives so far still
    differs from the wanted state. A removed key falls back to an earlier parent or goes, as in the game."""
    p = Plan()
    sim = G.copy()
    sim.fallback = W.fallback
    for s, sect in it.new.items():
        sim.add(sect.copy())
        p.new[s] = sect
    for s in W.topo():
        if s in it.new or s not in sim.sections:
            continue
        ws, ss = W.resolve(s), sim.resolve(s)
        ops = []
        sect = sim.sections[s]
        for k in sorted(set(ws) | set(ss)):
            wv, sv = val(ws, k), val(ss, k)
            if wv == sv:
                continue
            if wv is ABSENT:
                ops.append(("unset", k))
                sect.own[k] = REMOVED
                continue
            lo = list_op(sv, wv, W.has) if use_lists else None
            if lo:
                ops.append((lo[0], k, lo[1]))
                sect.own[k] = joined(lo[0], sv, lo[1])
            else:
                ops.append(("set", k, wv))
                sect.own[k] = wv
        if ops:
            p.ops[s] = ops
            sim._memo.pop(s, None)
    return p


# ---- composing a module's directives onto a game ------------------------------------------------------
class Composer:
    """Applies directives in the game's order onto a copy of a config; records what the game would refuse."""

    def __init__(self, G: Config, fallback=None):
        self.c = G.copy()
        self.c.fallback = fallback if fallback is not None else G.fallback
        self.refused: list[str] = []

    def create(self, name, parents, own, where, item=False):
        """A `.ltxp` header makes a missing section (its parents copied in) and leaves one that exists as it is;
        an item makes only what does not exist (its check refuses the item before this)."""
        if name in self.c.sections:
            if item:
                self.refused.append(f"{where}: [{name}] exists already")
                return False
            for k, v in own.items():
                self.set(name, k, v)
            return True
        for p in parents:
            if not self.c.has(p):
                self.refused.append(f"{where}: [{name}] cannot inherit missing section [{p}]")
                return False
        self.c.add(Sect(name, parents, {k: norm(v) for k, v in own.items()}, where))
        return True

    def _sect(self, name):
        s = self.c.sections.get(name)
        if s is None:
            s = Sect(name, [], {}, "(made by a directive)")
            self.c.add(s)
        return s

    def set(self, name, key, value):
        self._sect(name).own[key] = norm(value)
        self.c.invalidate()

    def unset(self, name, key):
        s = self.c.sections.get(name)
        if s is None or key not in self.c.resolve(name):
            self.refused.append(f"!{key} in [{name}]: no such key")
            return
        s.own[key] = REMOVED
        self.c.invalidate()

    def listop(self, kind, name, key, members):
        cur = self.c.resolve(name).get(key) if name in self.c.sections else None
        self._sect(name).own[key] = joined(kind, norm(cur) if cur is not None else ABSENT, members)
        self.c.invalidate()

    def delete(self, name):
        self.c.remove(name)


def compare(W: Config, C: Config, names=None, skip=None) -> list[tuple]:
    """Every (section, key) where the composed config differs from the wanted one."""
    out = []
    skip = skip or {}
    for s in sorted(names if names is not None else set(W.sections) | set(C.sections)):
        if (s in W.sections) != (s in C.sections):
            out.append((s, None, "wanted" if s in W.sections else "absent", "made" if s in C.sections else "missing"))
            continue
        a, b = W.resolve(s), C.resolve(s)
        drop = skip.get(s, ())
        for k in set(a) | set(b):
            if k in drop:
                continue
            if val(a, k) != val(b, k):
                out.append((s, k, val(a, k), val(b, k)))
    return out
