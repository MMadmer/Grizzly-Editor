#!/usr/bin/env python3
"""The differential run: the same commands on the C++ editor (the oracle) and the Rust editor.

    python tools/qa/diff_editors.py                 scenarios that failed last time or changed
    python tools/qa/diff_editors.py --all           every scenario
    python tools/qa/diff_editors.py --scenario s3   one scenario
    python tools/qa/diff_editors.py --refresh-oracle   record the C++ side again
    python tools/qa/diff_editors.py --write-parity  refresh the ledger's section from the last results

A scenario is a list of steps {cmd, args} run on both editors in the same order, each on its own
copy of one source project (in the system temp folder). After every step the two answers are
compared, after the scenario the two project copies and the two output folders, file by file.
Normalisation is the table below and nothing else: the copy's path is `<project>`, the scenario's
temp folder `<temp>`; IGNORE names every answer field left out, per command, FILE_RULES every file
whose bytes are compared in another form - each with its reason. Anything else that differs is a
failure (exit code 1).

The oracle is recorded once: the C++ editor does not change on this branch, so its side of a
scenario - the steps it ran, its answers, the files it left (size, SHA-256, the bytes of the small
ones) - is kept in Intermediate/oracle/<scenario>-<key>/. The key hashes the scenario's steps,
LevelEditor.exe, the source project and the linked game. The C++ editor (the kept binary
Bin\\x64\\CppEditor\\LevelEditor.exe of the tag cpp-editor-last; no C++ is left to build it from) starts only
for a scenario without a record, never when the binary is not there, and never when something already
answers on its port (that is the author's own editor): in both cases the run says so and goes on with
what needs no new record.

The Rust side runs the scenarios side by side, each on its own port, copy and home (-home), headless.
Each scenario's last result is remembered with Grizzly.exe's fingerprint and the scenario's key, so
the default run takes only what failed or changed.
"""
from __future__ import annotations

import argparse
import collections
import concurrent.futures
import difflib
import fnmatch
import hashlib
import io
import json
import math
import os
import re
import shutil
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import threading
import time
import zipfile

import qa_target  # noqa: F401 - the binaries and the C++ editor's own files
import kept  # noqa: E402 - the root's files on disk while a run has them aside

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
INTER = os.path.join(REPO, "Intermediate")
ORACLE = os.path.join(INTER, "oracle")
RUNS = os.path.join(INTER, "diffrun")
STATE = os.path.join(RUNS, "state.json")
TOOLS_JSON = os.path.join(REPO, "crates", "gz-mcp", "tools.json")
PARITY = os.path.join(REPO, "docs", "rust", "PARITY.md")
PARITY_HEAD = "## Commands compared by the differential run"
PARITY_END = "<!-- end of the differential run's section -->"

# the kept C++ editor of the tag cpp-editor-last, a self-contained copy no build touches; records are made
# from it only while it is there
CPP_EXE = os.path.join(REPO, "Bin", "x64", "CppEditor", "LevelEditor.exe")
# the Rust editor: the debug build, or the binary GRIZZLY_QA_EXE names (Bin\x64\Release\Grizzly.exe, the release one)
RUST_EXE = os.environ.get("GRIZZLY_QA_EXE") or os.path.join(INTER, "cargo", "debug", "Grizzly.exe")
CPP_PORT = 28016
RUST_PORT0 = 28120
# The C++ editor's own files in the repository root; the Rust editor writes into its -home and
# must leave both its own names there untouched.
CPP_KEPT = ["_projects.ini", "level.ini", "level_imgui.ini"]
# Bumped when what a record holds or how the C++ side is run changes.
ORACLE_FORMAT = 3
RUST_KEPT = ["_projects.ini", "grizzly_editor.ltx"]

SOURCES = {
    "od": r"D:\XFinedProjects\DARF-Overnight-Delivery",
    "test": r"D:\XFinedProjects\Test",
}
EXAMPLES = os.path.join(REPO, "docs", "nq", "examples")
GAME = r"D:\Games\Dead Air"

POLL = 0.05
STEP_TIMEOUT = 120.0
START_TIMEOUT = 120.0
CLOSE_TIMEOUT = 60.0
BLOB_LIMIT = 16 << 20
LEVEL_BLOB_LIMIT = 1 << 30
NORMALISE_LIMIT = 8 << 20

# ------------------------------------------------------------------------------------------------
# What is left out of the comparison, and why. Paths: `a.b` a field, `a[]` every element of a list.

IGNORE: dict[str, dict[str, str]] = {
    "*": {},
    "state": {
        "fps": "a timing (section 10)",
        "ui_scale": "the monitor's scale (section 10: pixel rectangles and DPI)",
    },
    "ui_contract": {
        "dpi": "the monitor's scale (section 10: pixel rectangles and DPI)",
    },
    "quest_references": {
        "fingerprint": "hashes the project's absolute path and the path of every graph (NqProjectIndex::Snapshot): two copies never agree",
    },
    "quest_project_find": {
        "fingerprint": "hashes the project's absolute path and the path of every graph (NqProjectIndex::Snapshot): two copies never agree",
    },
    "list_assets category=Particles": {
        "total": "C++ bug: CPSLibrary::Load stops at the first effect it cannot read (Dead Air's short TargetColor) "
                 "and keeps one group; the Rust reader reads all 1428 (STATE.md, The first test run)",
    },
    "mod_export_spawn": {
        "missing": "added by the Rust editor: the bake dialog's missing-model count (D13: the wire adds)",
        "removed": "added by the Rust editor: the bake dialog's Base objects removed row (D13: the wire adds)",
    },
    "mod_package": {
        "index.size": "D4: the index states the package's bytes on disk",
        "index.sha256": "D4: the index states the package's bytes on disk",
        "packages[].size": "D4: package bytes differ (zlib-rs, own headers)",
        "packages[].sha256": "D4: package bytes differ (zlib-rs, own headers)",
        "packed": "D4: package bytes differ (zlib-rs, own headers)",
    },
    "property_inspector": {
        "open": "the panel's state: a window (the Rust editor runs this scenario headless)",
        "rows[].list": "added by the Rust editor: what a choose row's chooser lists (STATE.md, The last review, the "
                       "shell; D13: the wire adds)",
        "rows[].choices": "added by the Rust editor: the entries of a choose row whose list is the object's own, its "
                          "model's bones or motions (STATE.md, The last review, the shell; D13: the wire adds)",
    },
    "property_inspector target=world": {
        "visible": "counts the rows IGNORE_ROWS names for this command",
        "total": "counts the rows IGNORE_ROWS names for this command",
    },
    "object_info name=level_bbox": {
        "object.visible": "a level bound shape is hidden by CEditShape::OnFrame, which the minimised C++ editor "
                          "never ran before the step; the Rust editor hides it at once",
    },
    "scene_visibility class=glow": {
        "classes[].hidden_objects": "the level bound shape, as object_info name=level_bbox",
    },
    "scene_visibility all=True": {
        "changed": "the level bound shape, as object_info name=level_bbox",
    },
    "scene_tree limit=5": {
        "tree[].objects[].visible": "the level bound shape, as object_info name=level_bbox",
    },
    "shortcuts": {
        "shortcuts[].keys": "the Rust editor ships default keys (the repository's level.ini [shortcuts], STATE.md "
                            "The shell's rest); the C++ editor binds none without a level.ini (D3: not migrated)",
    },
    "list_commands": {
        "commands[].keys": "added by the Rust editor: the keys bound now (STATE.md, The shell's rest; D13: the wire adds)",
        "commands[].presets[].keys": "added by the Rust editor: the keys bound now (STATE.md, The shell's rest; D13: the wire adds)",
    },
}

# Fields brought to one form before they are compared: (command, field) -> (function, reason).
def _unsigned_on_its_line(v):
    if isinstance(v, str):
        return re.sub(r"(?<!\n)(the release is not signed)", "\n\\1", v)
    return v


NORMALISE: dict[tuple[str, str], tuple] = {
    ("mod_package", "warnings"): (
        _unsigned_on_its_line,
        "C++ bug: the unsigned-release warning is glued to the last build warning with no line break"),
}

# Rows of a list left out by name: (list path, the field that names a row, the names, reason).
_PARTS = ["Group", "Glow", "Scene Object", "Light", "Shape", "Sound Source", "Spawn", "Way", "Sector", "Portal",
          "Sound Environment", "Particle System", "Detail Objects", "AI Map", "Wallmark", "Scene fog volumes"]

IGNORE_ROWS: dict[str, list[tuple[str, str, set[str], str]]] = {
    "property_inspector": [
        ("rows", "key", {"smart_cover\\hold position time"},
         "a new smart cover's m_hold_position_time: the C++ CSE_SmartCover constructor never sets it (uninitialised "
         "memory, drawn clamped to its 60 s maximum); the Rust editor starts it at 0"),
        ("rows", "key", {"Detail Objects\\Objects per square", "Detail Objects\\Base Texture",
                         "Detail Objects\\Common\\Draw objects", "Detail Objects\\Common\\Draw base texture",
                         "Detail Objects\\Common\\Base texture blended", "Detail Objects\\Common\\Draw slot boxes"},
         "EDetailManager::FillProp in Level Settings: the Rust editor has these on the Detail Objects tool's panel "
         "and details_tool (STATE.md, Differences the differential left)"),
        ("rows", "key", {f"Scene\\Targets\\{k}\\{p}" for k in ("Enable", "Read Only") for p in _PARTS},
         "CLevelPreferences::FillProp keeps them in Preferences; the Rust editor shows them in Level Settings "
         "(STATE.md, The scene's rest)"),
    ],
    "shortcuts": [
        ("shortcuts", "id", {"COMMAND_MOVE_CAMERA_TO", "COMMAND_SIMULATE", "COMMAND_USE_SIMULATE_POSITIONS",
                             "COMMAND_FILE_MENU"},
         "C++ rows whose bodies do nothing: PARITY.md section 1 'dropped'"),
        ("shortcuts", "id", {
            "COMMAND_UNHIDE_ALL", "COMMAND_MOD_SCREENS", "COMMAND_MOD_BUILD_TO_FOLDER", "COMMAND_MOD_EXPORT_FLAT",
            "COMMAND_REWRITE_THUMBNAILS", "COMMAND_MOD_BAKE_SPAWN", "COMMAND_MOD_BAKE_XCFORM", "COMMAND_MOD_BAKE_OGF",
            "COMMAND_MOD_BAKE_CUT"},
         "Rust-only rows from id 123 for UI actions the C++ had no command for (STATE.md, The shell's rest; D13)"),
    ],
    "list_commands": [
        ("commands", "name", {
            "COMMAND_INITIALIZE", "COMMAND_DESTROY", "COMMAND_QUIT", "COMMAND_UPDATE_PROPERTIES",
            "COMMAND_REFRESH_PROPERTIES", "COMMAND_RENDER_FOCUS", "COMMAND_RENDER_RESIZE", "COMMAND_UPDATE_GRID",
            "COMMAND_CREATE_SOUND_LIB", "COMMAND_CHECK_MODIFIED", "COMMAND_SET_SETTINGS", "COMMAND_CLEAN_LIBRARY",
            "COMMAND_REFRESH_UI_BAR", "COMMAND_RESTORE_UI_BAR", "COMMAND_SAVE_UI_BAR", "COMMAND_UPDATE_TOOLBAR",
            "COMMAND_UPDATE_CAPTION", "COMMAND_UNLOAD_TEXTURES", "COMMAND_EVICT_OBJECTS", "COMMAND_EVICT_TEXTURES",
            "COMMAND_SAVE_BACKUP"},
         "the C++ frame's plumbing (start / stop, panel refresh, caption, eviction, the mailslot's backup): "
         "no user function, PARITY.md section 1 'same (no user function)'"),
        ("commands", "name", {
            "COMMAND_LIBRARY_EDITOR", "COMMAND_LANIM_EDITOR", "COMMAND_FILE_MENU", "COMMAND_SHOW_CLIP_EDITOR",
            "COMMAND_SHOWCONTEXTMENU", "COMMAND_REFRESH_SNAP_OBJECTS", "COMMAND_CHANGE_SNAP", "COMMAND_MOVE_CAMERA_TO",
            "COMMAND_SIMULATE", "COMMAND_USE_SIMULATE_POSITIONS", "COMMAND_EDIT_COMMAND_LIST",
            "COMMAND_SCENE_HIGHLIGHT_TEXTURE", "COMMAND_REFRESH_SOUND_ENV_GEOMETRY"},
         "C++ rows whose bodies do nothing (commented out, not_implemented, return FALSE): PARITY.md section 1 'dropped'"),
        ("commands", "name", {"COMMAND_CB_PLACE_ASSET"},
         "the Content Browser's Place in Scene and viewport drop (PARITY.md section 1 'same'); no table row of its own"),
        ("commands", "name", {
            "COMMAND_UNHIDE_ALL", "COMMAND_MOD_SCREENS", "COMMAND_MOD_BUILD_TO_FOLDER", "COMMAND_MOD_EXPORT_FLAT",
            "COMMAND_REWRITE_THUMBNAILS", "COMMAND_MOD_BAKE_SPAWN", "COMMAND_MOD_BAKE_XCFORM", "COMMAND_MOD_BAKE_OGF",
            "COMMAND_MOD_BAKE_CUT"},
         "Rust-only rows from id 123 for UI actions the C++ had no command for (STATE.md, The shell's rest; D13)"),
    ],
}

# Files whose bytes are compared in another form: (glob of the path under <project> or <temp>,
# what is compared, reason). A file that matches no rule is compared byte for byte (after the
# copy's and the temp folder's paths were put back as <project> / <temp>).
FILE_RULES: list[tuple[str, str, str]] = [
    ("project/temp/session.ltx", "skip",
     "the shell's open tabs: a UI state (the Rust editor runs headless here); test_ui_shell.py checks the tabs"),
    ("project/preview.png", "png",
     "D2: the picture is the Rust renderer's, the C++ one its minimised window's viewport; compared as a PNG"),
    ("temp/out/pkg/*.zip", "zip_entries",
     "D4: package bytes differ (zlib-rs, own headers); its entries - names, CRC-32 and sizes - are compared"),
    ("temp/out/pkg/*.files", "index_columns",
     "D4: the index states the package's bytes on disk (offset, packed length, method); every other column is compared"),
    ("temp/out/pkg/*.update.ltx", "descriptor_sizes",
     "D4: the descriptor pins the package and the index by size and SHA-256; every other line is compared"),
    # the level exports' models (s6_scene) and the level build (s7_levelbuild); the rules are the functions
    # below, STATE.md "For the plan" states them
    ("*levels/*/overlay.xcform", "cmp_xcform",
     "D14: the material table in the order a mesh's surface map (keyed by CSurface pointers) meets the surfaces: "
     "each triangle's material compared by name; header, vertices, corners and cut list byte for byte"),
    ("*levels/*/visuals/*.ogf", "cmp_ogf",
     "D14: a world-space model's children (one per surface) and the collision mesh after them come in the order "
     "of the mesh's surface map, keyed by CSurface pointers (CExportObjectOGF): the children compared as a "
     "multiset of their bytes, the collision mesh as a multiset of triangles; every other chunk byte for byte"),
    ("project/gamedata/levels/*/build.prj", "cmp_build_prj",
     "D14: faces in the order of each mesh's surface map (keyed by CSurface pointers), so the material, shader "
     "and texture tables in the order faces meet them - compared as sets of what they name; struct padding and "
     "the fields the C++ never clears (b_face's pad, an unset Flight's fields, the LOD material's reserved and "
     "area) left out; LOD corners within 2 micrometres, point-light colours within 4 ULP"),
    ("project/gamedata/levels/*/level.spawn", "cmp_level_spawn",
     "D10: the Rust build's breakable and climable objects left out (the C++ writes none); a smart cover's "
     "loopholes in Lua `pairs` order - compared as a set; every other entry byte for byte"),
    ("project/gamedata/levels/*/meshes/brkbl#*.ogf", "rust_only",
     "D10: the breakable objects' models, which the C++ build cannot make"),
    ("project/gamedata/levels/*/level_stat.txt", "cmp_level_stat_txt",
     "the author's statistic (no compiler and not the game reads it): a vertex a rounding away from a 1 m "
     "cell's border may land beside it; the maxima within 0.5 %, every other line equal"),
    ("project/gamedata/levels/*/level_stat.tga", "cmp_level_stat_tga",
     "the same statistic as a picture: at most one pixel in 10000 differs"),
    ("project/gamedata/levels/*/build.details", "cmp_build_details",
     "D14: the detail textures go into build_details.dds in the order of their shared-string addresses (and a "
     "sort whose comparator is no strict order), so a model's UVs are shifted by where its picture landed: "
     "every model's UVs equal up to one shift of its own, everything else byte for byte"),
    ("project/gamedata/levels/*/build_details.dds", "cmp_build_details_dds",
     "D14 places the pictures (as build.details) and the DXT5 encoder is another (the C++ xrDXT, here "
     "gz-fmt-tex): the header byte for byte, the first level decoded and its 128 px tiles matched one to one, each "
     "pair within DXT5's error (alpha mean 1, opaque colour mean 8 per channel); the smaller levels are filtered "
     "across the pictures' borders and follow the packing"),
]

# Commands of the catalogue this run never sends, and why (the ledger's "not compared").
RUST_ONLY = ("only the Rust editor has it (D13: jobs, the level build, the tools' and the scene's own commands); "
             "its crate's tests and the QA suites drive it")
CAMERA = ("the C++ ray pick skips every object its camera does not see (CSceneObject::RayPick, occ_visible): "
          "the answer follows a minimised window's camera")
PROJECTS = "works on the editor's project list and other projects' folders: test_project_browser.py drives it on throw-away projects"
NOT_COMPARED: dict[str, str] = {
    **{c: RUST_ONLY for c in (
        "jobs", "sector_tool", "portal_tool", "way_tool", "shape_tool", "light_tool", "fog_tool",
        "aimap_tool", "details_tool", "wallmark_tool", "particles_tool", "scene_edit", "selection_file",
        "rename_objects", "level_parts", "scene_new", "scene_summary", "compiler_errors", "scene_validate",
        "scene_export_obj", "object_motion", "object_tool", "spawn_attach", "group_tool")},
    **{c: CAMERA for c in ("probe_vertical", "scan_walls", "drop_objects")},
    **{c: PROJECTS for c in ("open_project", "project_close", "project_create", "project_delete", "project_forget",
                             "project_reveal")},
    "exec_command": "runs a row of the C++ command table in the window: test_ui_shell.py",
    "property_choose": "drives the chooser modal: a window (test_properties.py)",
    "quest_open": "brings a graph tab up: a window (test_graph_ui.py)",
    "quest_reload": "reloads an open graph tab from disk: a window (test_graph_ui.py)",
}
WINDOW_PREFIXES = ("ui_", "screenshot_")
WINDOW = {
    "titlebar_info", "window_control", "reset_layout", "theme", "command_palette", "quest_view",
    "characters_view", "world_view", "screens_view", "mod_settings_view", "pda_preview",
    "preview_model", "texture_preview", "asset_preview", "library", "outliner_show", "outliner_filter",
    "content_browser_selection", "content_browser_navigation", "content_browser_open",
    "content_browser_copy", "viewport_navigation", "viewport_controls", "view_mode", "camera_get",
    "camera_set", "focus_object", "frame_pacing", "game_lighting", "quest_minimap", "quest_bookmarks",
    "quest_history", "quest_find", "group_thumbnail", "project_browser_menu", "progress",
}
NETWORK = {
    "mod_publish": "reaches GitHub: test_mod_publish.py drives it against the mock",
    "mod_releases": "reaches GitHub: test_mod_publish.py drives it against the mock",
    "mod_catalog": "reaches GitHub and VirusTotal: test_mod_catalog.py drives it against the mock",
}


# ------------------------------------------------------------------------------------------------
# The scenarios. A step is {"cmd", "args"} (with "<project>" / "<temp>" in strings), plus
# "watch": files (project-relative) whose bytes are compared after the step, "timeout" in seconds.
# {"gen": f} is expanded on the C++ side from the answers so far and recorded with them.

def S(cmd: str, watch: list[str] | None = None, timeout: float | None = None, settle: str = "",
      expect: str = "ok", answer: str = "", cpp: dict | None = None, **args) -> dict:
    """One step. `expect` is what both editors must answer - "ok" (the work happens) or "refused" (the
    refusal is the point); anything else fails the scenario, also while the oracle is recorded. `answer`
    "skip: <why>" compares the files the step wrote and not its answer (the two editors' commands differ:
    `cpp` names the C++ editor's {cmd, args} for the step)."""
    step: dict = {"cmd": cmd, "args": args}
    if expect != "ok":
        step["expect"] = expect
    if answer:
        step["answer"] = answer
    if cpp:
        step["cpp"] = cpp
    if watch:
        step["watch"] = watch
    if settle:
        # the C++ editor draws after the step ("ui" a frame, "viewport" a pass of the scene too):
        # what its frame does (a level bound shape hidden by CEditShape::OnFrame) is done before
        # the next step reads it
        step["settle"] = settle
    if timeout:
        step["timeout"] = timeout
    return step


def last(answers: list[dict], cmd: str) -> dict:
    for step, answer in reversed(answers):
        if step["cmd"] == cmd:
            return answer
    return {}


def gen_quest_reads(answers: list) -> list[dict]:
    listed = last(answers, "quest_list")
    paths = [q.get("path") for q in listed.get("quests", []) if q.get("path")]
    steps = []
    for p in paths:
        steps += [S("quest_get", path=p), S("quest_validate", path=p), S("quest_layout", path=p)]
    return steps


def gen_scoped_lookups(answers: list) -> list[dict]:
    listed = last(answers, "quest_list").get("quests", [])
    quest = next((q["path"] for q in listed if q.get("path", "").endswith(".nqasset")), "")
    beh = next((q["path"] for q in listed if q.get("path", "").endswith(".behasset")), "")
    steps = []
    for t in ("task_id", "var_name", "ref_name", "node_id"):
        steps.append(S("quest_lookup", type=t, path=quest))
    if beh:
        steps.append(S("quest_lookup", type="role", path=beh))
    got = next((a for s, a in answers if s["cmd"] == "quest_get" and s["args"].get("path") == quest), {})
    tasks = got.get("tasks") or []
    task = tasks[0].get("id") if tasks and isinstance(tasks[0], dict) else ""
    if task:
        steps.append(S("quest_references", type="task_id", value=task, path=quest))
    qid = got.get("id", "")
    if qid:
        steps.append(S("quest_references", type="quest_id", value=qid))
    return steps


def gen_characters(answers: list) -> list[dict]:
    listed = last(answers, "characters_list")
    rows = listed.get("characters", [])
    return [S("characters_get", id=c["id"]) for c in rows if c.get("id")]


def gen_pda(answers: list) -> list[dict]:
    rows = last(answers, "pda_list").get("layouts", []) or last(answers, "pda_list").get("files", [])
    steps = []
    for r in rows:
        p = r.get("path") if isinstance(r, dict) else r
        if p:
            steps += [S("pda_get", path=p), S("pda_validate", path=p)]
    return steps


LOOKUP_TYPES = ["item_section", "squad_section", "level", "smart", "story_id", "profile", "community",
                "info", "spot_type", "global_name", "quest_id", "behaviour_id", "character_id", "dialog_id",
                "character_icon", "rank", "way_path", "level_changer"]
ASSET_CATEGORIES = ["Objects", "Groups", "Visuals", "Textures", "Textures (raw)", "Particles", "Sounds",
                    "Entities", "Light Anims"]

S1 = [
    S("ping"), S("mod_manifest"), S("game_link"), S("game_modes"), S("mod_commands"),
    S("quest_catalog"), S("quest_list"), S("quest_check_all"),
    {"gen": gen_quest_reads},
    *[S("quest_lookup", type=t) for t in LOOKUP_TYPES],
    {"gen": gen_scoped_lookups},
    S("characters_list"), {"gen": gen_characters},
    S("world_get"), S("world_transitions"), S("world_transitions", patrols="agr_", limit=20),
    S("world_transitions", level="l03_agroprom"),
    S("pda_list"), {"gen": gen_pda},
    S("screens_get"),
    *[S("list_assets", category=c) for c in ASSET_CATEGORIES],
    S("list_spawn_sections"), S("list_spawn_sections", filter="stalker", limit=50),
    S("ui_contract"), S("preferences"), S("list_projects"), S("progress"),
    S("quest_project_find", query="trader", limit=50), S("quest_project_find", query="spawn.npc -bandit", limit=50),
    S("darf_list"), S("darf_list", folder="configs\\gameplay"),
]

# What the shell answers: the Rust editor needs its window for these (minimised, never in front).
S1W = [S("list_commands"), S("shortcuts"), S("shortcuts", filter="scene"), S("state")]

M = ["mod.ltx"]
S2 = [
    S("mod_set_manifest", watch=M, name="Overnight Delivery QA", version="1.2.0", author="QA Author"),
    S("mod_set_manifest", watch=M, description="A line with \"quotes\" and ; a semicolon"),
    S("mod_set_manifest", watch=M, website="https://www.moddb.com/mods/qa-overnight"),
    S("mod_set_manifest", watch=M, requires="qa_base", after="qa_before", conflicts="qa_enemy"),
    S("mod_set_manifest", watch=M, vars="night=1, qa_flag=true, qa_text=\"hello\""),
    S("mod_set_manifest", watch=M, provides_mode_title="Overnight Delivery QA"),
    S("mod_set_manifest", watch=M, disable_game_updates=True),
    S("mod_set_manifest", watch=M, tags="story, horror", expect="refused"),
    S("mod_set_manifest", watch=M, tags="not_a_tag", expect="refused"),
    S("mod_set_manifest", watch=M, version="1.2.0.1"),
    S("mod_set_manifest", watch=M, version="not a version", expect="refused"),
    S("mod_exclude", watch=M, path="design", excluded=False),
    S("mod_exclude", watch=M, path="*.psd", excluded=True),
    S("mod_exclude", watch=M, path="docs/*.md", excluded=True),
    S("mod_set_manifest", watch=M, exclude="docs, design, *.psd"),
    S("mod_commands", watch=M, name="qa_cmd", function="qa_module.run", help="Runs the QA check"),
    S("mod_commands", watch=M, name="qa_cmd", function="od_cmd.status", help="Runs the QA check", args="[full]"),
    S("mod_commands", watch=M, name="qa_cmd", remove=True),
    S("world_set", watch=M, original_npcs=True),
    S("world_set", watch=M, player_choice="random_stalkers,mutants"),
    S("world_set", watch=M, player_choice="random_stalkers,mutants,npc_quests,story_quests", expect="refused"),
    S("world_set", watch=M, levels="l03_agroprom"),
    S("world_set", watch=M, lock="agr_level_changer_to_garbage", lock_level="l03u_agr_underground"),
    S("world_set", watch=M, unlock="agr_level_changer_to_garbage", unlock_level="l03u_agr_underground"),
    S("world_set", watch=M, start_kit="wpn_pm=1, ammo_9x18_fmj=2"),
    S("world_set", watch=M, start_money=500, start_perks=True),
    S("world_set", watch=M, start_kit_remove=True),
    S("world_set", watch=M, start_kit="wpn_pm=1", start_kit_remove=False),
    S("world_set", watch=M, preset="empty"),
    S("world_get"),
    S("screens_set", watch=M, action="policy", surface="loading", screens="only"),
    S("screens_set", watch=M, action="screen_add", id="qa_screen", surface="loading"),
    S("screens_set", watch=M, action="screen_set", id="qa_screen", levels="l03_agroprom", weight=2),
    S("screens_set", watch=M, action="track_add", id="qa_track", expect="refused"),
    S("screens_set", watch=M, action="screen_remove", id="qa_screen"),
    S("screens_get"),
    S("world_set", watch=M, preset="stock"),
    S("mod_manifest"),
]

QUEST = "quests/qa/qa_graph.nqasset"
BEH = "quests/qa/qa_beh.behasset"
QUEST_TEXT = """return {
  nq = 1,
  id = "qa_written",
  title = "QA written",
  activation = "auto",
  tasks = {
    t_main = { title = "Main task", descr = "The task the rename step renames.", type = "additional" },
  },
  nodes = {
    { id = "start", kind = "trigger.start", out = { next = "step" } },
    { id = "step", kind = "flow.step",
      on_enter = { { kind = "task.give", params = { task = "t_main" } } },
      out = { next = "close" } },
    { id = "close", kind = "flow.step",
      on_enter = { { kind = "task.complete", params = { task = "t_main" } } },
      out = { next = "done" } },
    { id = "done", kind = "flow.end", params = { status = "completed" } },
  },
}
"""
CHAR_TEXT = """return {
  npc = 1,
  id = "qa_char",
  name = { rus = "Проверка", eng = "Check" },
  community = "stalker",
  visual = "actors\\\\stalker_neutral\\\\stalker_neutral_1",
}
"""
OPS = """{
  { op = "add_node", node = { id = "wait", kind = "wait.time", params = { seconds = 5 } } },
  { op = "connect", from = "start", pin = "next", to = "wait" },
  { op = "set_pos", id = "wait", x = 300, y = 40 },
}"""


def S3_steps() -> list[dict]:
    q, b = [QUEST], [BEH]
    return [
        S("quest_new", watch=q, path=QUEST),
        S("quest_get", path=QUEST),
        S("quest_apply", path=QUEST, ops=OPS),
        S("quest_edit", path=QUEST, op="disconnect", **{"from": "start", "pin": "next"}),
        S("quest_undo", path=QUEST),
        S("quest_redo", path=QUEST),
        S("quest_undo", path=QUEST),
        S("quest_layout", path=QUEST, all=True),
        S("quest_save", watch=q, path=QUEST),
        S("quest_close", path=QUEST),
        S("quest_write", watch=q, path=QUEST, lua=QUEST_TEXT),
        S("quest_rename_task", watch=q, path=QUEST, to="t_renamed", ack_runtime_identity=True, **{"from": "t_main"}),
        S("quest_save", watch=q, path=QUEST),
        S("quest_get", path=QUEST),
        S("quest_close", path=QUEST),
        S("quest_new", watch=b, path=BEH),
        S("quest_get", path=BEH),
        S("quest_lookup", type="role", path=BEH),
        S("quest_apply", path=BEH, ops=OPS),
        S("quest_save", watch=b, path=BEH),
        S("quest_layout", path=BEH, all=True),
        S("quest_save", watch=b, path=BEH),
        S("quest_close", path=BEH),
        S("characters_write", watch=["characters/qa_char.npcasset"], text=CHAR_TEXT, id="qa_char"),
        S("characters_get", id="qa_char"),
        S("characters_list"),
        S("characters_delete", id="qa_char"),
        S("pda_write", watch=["pda/qa_layout.pdaasset"], path="pda/qa_layout.pdaasset"),
        S("pda_use", watch=M, path="pda/qa_layout.pdaasset"),
        S("pda_get", path="pda/qa_layout.pdaasset"),
        S("pda_delete", watch=M, path="pda/qa_layout.pdaasset"),
        S("content_create", kind="quest"),
        S("content_create", kind="behaviour"),
        S("content_create", kind="character"),
        S("content_create", kind="pda"),
        S("content_create", kind="quest"),
    ]


U_FOLDER = "qa_юникод"
S4 = [
    S("content_mkdir", path="qa_ops"),
    S("content_mkdir", path="qa_ops/" + U_FOLDER),
    S("content_mkdir", path="qa_ops/" + U_FOLDER),
    S("content_copy", src="characters/bandit_1.npcasset", dst="qa_ops/" + U_FOLDER + "/Ночная доставка.npcasset"),
    S("content_copy", src="LICENSE", dst="qa_ops/" + U_FOLDER + "/日本の地図.txt"),
    S("content_copy", src="LICENSE", dst="qa_ops/" + U_FOLDER + "/日本の地図.txt"),
    S("content_copy", src="LICENSE", dst="qa_ops/" + U_FOLDER + "/日本の地図.txt", overwrite=True),
    S("content_rename", path="qa_ops/" + U_FOLDER + "/日本の地図.txt", name="Сюжет ✓.txt"),
    S("content_rename", path="qa_ops/" + U_FOLDER + "/Ночная доставка.npcasset", name="Ночной.txt"),
    S("content_rename", path="qa_ops/" + U_FOLDER + "/Ночной.txt", name="Ночной"),
    S("content_copy", src="quests/side", dst="qa_ops/side_copy", recursive=True),
    S("content_copy", src="quests/side", dst="qa_ops/side_copy2"),
    S("content_move", src="qa_ops/side_copy/side/h1_hot_kitchen.nqasset", dst="qa_ops/" + U_FOLDER + "/Доставка.nqasset"),
    S("content_move", src="qa_ops/side_copy", dst="qa_ops/" + U_FOLDER + "/side", recursive=True),
    S("content_rename", path="qa_ops/" + U_FOLDER, name="qa_renamed"),
    S("quest_list"),
    S("darf_copy", path="ai/alife/birthpercentage.efd"),
    S("darf_copy", path="ai/alife/birthpercentage.efd", expect="refused"),
    S("darf_copy", path="ai/alife/birthpercentage.efd", overwrite=True),
    S("darf_copy", path="ai/alife/no_such_file.efd", expect="refused"),
    S("content_delete", path="qa_ops/qa_renamed/side/side_copy/side/h2_cold_chain.nqasset"),
    S("content_delete", path="qa_ops/qa_renamed/side", expect="refused"),
    S("content_delete", path="qa_ops/qa_renamed/side", recursive=True),
    S("content_delete", path="qa_ops/nothing_here.txt", expect="refused"),
    S("content_delete", path="", expect="refused"),
    S("content_move", src="qa_ops/qa_renamed/Сюжет ✓.txt", dst="../outside.txt", expect="refused"),
]

S5 = [
    # an open scene: every build bakes it into the module's spawn\<level>.xspawn first
    S("open_scene", scene="l03_agroprom.level", timeout=600, settle="viewport"),
    S("mod_export", target="<temp>\\out\\game", timeout=300),
    S("mod_export", target="<temp>\\out\\game", timeout=300),
    S("mod_export", target="<temp>\\out\\flat", flat=True, timeout=300),
    S("mod_package", out_dir="<temp>\\out\\pkg", timeout=600),
]


def gen_scene_sample(answers: list) -> list[dict]:
    tree = last(answers, "scene_tree")
    steps = []
    for group in tree.get("tree", []):
        objs = group.get("objects", [])
        for o in objs[:2] + objs[-1:]:
            if o.get("name"):
                steps.append(S("object_info", name=o["name"]))
        if group.get("class"):
            steps.append(S("list_objects", **{"class": group["class"], "limit": 20}))
    return steps


def gen_scene_props(answers: list) -> list[dict]:
    """A row set the way the Details panel sets it (the custom data's [story_object]), the spawn setters."""
    tree = next((a for s, a in answers if s["cmd"] == "scene_tree"), {}).get("tree", [])
    spawns = next((g.get("objects", []) for g in tree if g.get("class") == "spawn"), [])
    if not spawns:
        return []
    name = spawns[0]["name"]
    first = next((a for s, a in answers if s["cmd"] == "property_inspector"), {})
    story = [r["key"] for r in first.get("rows", []) if r.get("key", "").endswith("ALife\\Story ID (script)")]
    steps = [S("select_objects", names=name)]
    for k in story[:1]:
        steps += [S("property_set", key=k, value="qa story"), S("property_inspector", limit=2000),
                  S("property_set", key=k, value=""), S("object_info", name=name)]
    steps += [S("spawn_set_fixed_bones", name=name, bones=""),
              S("spawn_set_fixed_bones", name=name, bones="link"),
              S("spawn_set_visual", name=name, visual="dynamics\\door\\no_such_visual", expect="refused"),
              S("deselect_all")]
    return steps


def gen_scene_edits(answers: list) -> list[dict]:
    objs = last(answers, "list_assets").get("assets", [])
    ref = next((a.get("name") if isinstance(a, dict) else a for a in objs), "")
    stalker = ""
    for row in last(answers, "list_spawn_sections").get("sections", []):
        if row.get("class") == "AI_STL_S":
            stalker = row.get("section", "")
            break
    steps = []
    if ref:
        steps.append(S("place_object", ref=ref, name="qa_static", x=-220.0, y=-20.0, z=-140.0))
    steps += [
        S("place_spawn", section="player_start", x=-223.0, y=-20.0, z=-143.0),
        S("place_spawn", section=stalker or "stalker", x=-215.0, y=-20.0, z=-145.0),
        # not snapped: the C++ ray pick skips every object its camera does not see
        # (CSceneObject::RayPick, occ_visible) - STATE.md, Differences the differential left
        S("place_way", name="qa_patrol", points="-232,-20,-150; -228,-20,-156; -236,-20,-158",
          snap_to_ground=False),
        S("place_way", name="qa_patrol", points="-200,-20,-140", expect="refused"),
        S("selection"),
        S("scene_info"),
    ]
    return steps


def gen_scene_after_place(answers: list) -> list[dict]:
    def placed(i: int) -> str:
        found = [a for s, a in answers if s["cmd"] == "place_spawn"]
        return ((found[i].get("object") or {}).get("name", "")) if len(found) > i else ""
    start, stalker = placed(0), placed(1)
    steps = [
        S("object_set_position", name="qa_static", x=-221.5, y=-19.0, z=-141.25),
        S("object_set_rotation", name="qa_static", x=0.0, y=0.7853982, z=0.0),
        S("object_set_scale", name="qa_static", x=1.5, y=1.5, z=1.5),
        S("object_info", name="qa_static"),
        S("rename_object", name="qa_static", new_name="qa_static_renamed"),
        S("rename_object", name="qa_static_renamed", new_name="qa_patrol", expect="refused"),
    ]
    if stalker:
        steps += [
            S("spawn_order", name=stalker),
            S("spawn_order", name=stalker, order="patrol", expect="refused"),
            S("spawn_order", name=stalker, order="patrol", route="qa_patrol", run=True, hostile=True),
            S("object_info", name=stalker),
        ]
    if start:
        steps.append(S("spawn_order", name=start, expect="refused"))
    steps += [
        S("world_get"),
        S("select_objects", names="qa_static_renamed"),
        S("delete_selected"),
        S("undo"), S("redo"), S("undo"),
        S("selection"),
        S("scene_stats"),
        S("save_scene", timeout=600),
        S("mod_export_spawn", selected_only=False, timeout=300, watch=[XSPAWN]),
        # a small static object alone, turned and scaled: the golden files of crates/gz-export-level/tests/data
        S("place_object", ref=GOLDEN_REF, name="qa_golden", x=-226.0, y=-20.0, z=-146.0),
        S("object_set_rotation", name="qa_golden", x=0.0, y=0.5, z=0.0),
        S("object_set_scale", name="qa_golden", x=1.25, y=1.25, z=1.25),
        S("select_objects", names="qa_golden"),
        S("mod_export_xcform", timeout=300, watch=[XCFORM]),
        S("mod_export_cut", timeout=300, watch=[XCFORM, VISUALS_LTX]),
        S("mod_export_ogf_probe", timeout=300, watch=[VISUALS_LTX, LEVEL_DIR + "/visuals/qa_golden.ogf"]),
        # one object alone
        S("select_objects", names="qa_static_renamed"),
        S("mod_export_xcform", timeout=300, watch=[XCFORM]),
        S("mod_export_cut", timeout=300, watch=[XCFORM, VISUALS_LTX]),
        S("mod_export_ogf_probe", timeout=300, watch=[VISUALS_LTX, LEVEL_DIR + "/visuals/qa_static_renamed.ogf"]),
        S("list_objects", **{"class": "scene_object", "limit": 600}),
        {"gen": gen_scene_exports},
    ]
    return steps


XSPAWN = "spawn/l01_escape.xspawn"
GOLDEN_REF = "statics\\army_base\\prop_map_02"
LEVEL_DIR = "levels/l01_escape"
XCFORM = LEVEL_DIR + "/overlay.xcform"
VISUALS_LTX = LEVEL_DIR + "/overlay_visuals.ltx"
# how many of the scene's own static objects the level exports take: real geometry of Escape
EXPORT_OBJECTS = 40


def gen_scene_exports(answers: list) -> list[dict]:
    """The level exports over real geometry - the first static objects of Escape with the placed one -
    then the build with the scene baked inside, then a second Player Start that the build refuses (E201)."""
    listed = last(answers, "list_objects").get("objects", [])
    names = [o["name"] for o in listed if o.get("name") and not o["name"].lower().startswith("dynamics\\")]
    pick = ["qa_static_renamed"] + names[:EXPORT_OBJECTS]
    return [
        S("select_objects", names=",".join(pick)),
        S("selection"),
        S("mod_export_xcform", timeout=300, watch=[XCFORM]),
        S("mod_export_cut", timeout=300, grow=25, watch=[XCFORM, VISUALS_LTX]),
        S("mod_export_ogf_probe", timeout=300, watch=[VISUALS_LTX]),
        S("deselect_all"),
        S("mod_export", target="<temp>/out/game", timeout=600),
        S("place_spawn", section="player_start", x=-205.0, y=-20.0, z=-150.0),
        S("world_get"),
        S("mod_export", target="<temp>/out/game", timeout=600, expect="refused"),
    ]


def gen_scene_details(answers: list) -> list[dict]:
    """The Details rows of a sample of every spawn kind, the Level Settings rows, and the marks."""
    tree = last(answers, "scene_tree").get("tree", [])
    spawns = next((g.get("objects", []) for g in tree if g.get("class") == "spawn"), [])
    steps = []
    for o in spawns[:5]:
        steps += [S("select_objects", names=o["name"]), S("property_inspector", limit=2000)]
    steps += [
        S("property_inspector", target="world", limit=2000),
        S("deselect_all"), S("selection"),
    ]
    if spawns:
        steps += [
            S("select_objects", names=spawns[0]["name"]),
            S("mark_removed", removed=True), S("mark_authored", authored=True), S("object_info", name=spawns[0]["name"]),
            S("mark_authored", authored=False), S("mark_removed", removed=False),
        ]
    steps += [
        # as text: the C++ GetArg takes no JSON boolean here (STATE.md, Differences the differential left)
        S("scene_visibility", **{"class": "glow", "visible": "false"}), S("scene_tree", **{"class": "glow", "limit": 3}),
        S("scene_visibility", all=True, visible="true"),
        S("snap_objects", action="set_from_selection"), S("snap_objects"), S("snap_objects", action="clear"),
    ]
    return steps

# Classes the game's object factory has no server class for (neither the engine's table nor
# class_registrator.script): the Rust editor refuses them, the C++ editor places an invalid spawn
# point whose type is never set and stops in CSpawnPoint::SaveStream (THROW) at the next undo
# snapshot - undefined behaviour, no oracle.
NO_SERVER_CLASS = {"AI_SPGRP", "II_BTTCH", "NW_ATTCH", "W_MOUNTD"}
# Sections the C++ factory makes no entity of ("!Can't create entity: <section>" - a script class
# whose Lua server object fails in the editor): the same invalid spawn point, which a later Details
# pass (CSpawnPoint::FillProp) stops on with THROW. The Rust editor makes them; their class is
# compared through the next section of it.
NO_ENTITY_SECTION = {"holder_stul", "m_lesnik", "m_trader"}


def _class_sections(answers: list) -> list[tuple[str, str]]:
    rows = [a for s, a in answers if s["cmd"] == "list_spawn_sections" and not s["args"].get("filter")]
    seen: dict[str, str] = {}
    for row in (rows[-1] if rows else {}).get("sections", []):
        cls, sec = row.get("class", ""), row.get("section", "")
        # a "fake" section is no item of the game: the C++ editor stops on it (gl_fake_missile has
        # no inv_weight, CInifile::r_string)
        if cls in NO_SERVER_CLASS or sec in NO_ENTITY_SECTION:
            continue
        if cls and sec and cls not in seen and "fake" not in sec.lower():
            seen[cls] = sec
    return sorted(seen.items())


def gen_scene_classes(answers: list) -> list[dict]:
    """One spawn of every entity class the linked game's sections give (the first section of each
    class), placed side by side."""
    steps = []
    for i, (_cls, sec) in enumerate(_class_sections(answers)):
        x, z = -260.0 + (i % 12) * 3.0, -190.0 - (i // 12) * 3.0
        steps.append(S("place_spawn", section=sec, x=x, y=-20.0, z=z, snap_to_ground=False))
    return steps


def gen_scene_class_rows(answers: list) -> list[dict]:
    """The Details rows of each spawn gen_scene_classes placed."""
    n = len(_class_sections(answers))
    placed = [a for s, a in answers if s["cmd"] == "place_spawn"][-n:] if n else []
    steps = []
    for a in placed:
        name = (a.get("object") or {}).get("name", "")
        if name:
            steps += [S("select_objects", names=name), S("property_inspector", limit=2000)]
    return steps


S6 = [
    S("open_scene", scene="l01_escape.level", timeout=900, settle="viewport"),
    S("scene_info"), S("scene_stats"), S("scene_load_status"), S("recovery"),
    S("scene_tree", limit=5),
    {"gen": gen_scene_sample},
    {"gen": gen_scene_details},
    S("selection"),
    S("list_assets", category="Objects", limit=5),
    S("list_spawn_sections", filter="stalker", limit=400),
    {"gen": gen_scene_edits},
    {"gen": gen_scene_after_place},
    # last: the C++ undo stack after a Details edit is not the Rust one (D8), and the saved scene
    # above is compared without these edits
    {"gen": gen_scene_props},
    # every entity class's rows, on spawns placed after the scene was saved and compared
    # no undo snapshots from here: the C++ CSpawnPoint::SaveStream throws on a spawn point whose
    # entity the factory could not make (its type is never set)
    S("preferences", action="set", key="scene_undo_level", value=0),
    S("list_spawn_sections", limit=5000),
    {"gen": gen_scene_classes},
    {"gen": gen_scene_class_rows},
]

# The level build (Build > Make All, the C++ COMMAND_BUILD: SceneBuilder::Compile) of Escape: every file
# the editor writes into the level folder itself - build.prj, level.ltx, the game files, the HOM / SOM,
# build.aimap, the statistic - compared file by file. What the external compilers make of them is not
# the editor's (section 9: made by the C++ compilers until phase 8).
S7 = [
    S("open_scene", scene="l01_escape.level", timeout=900, settle="viewport"),
    # the one object of Escape that shows an .ogf (visual mode): the C++ SceneBuilder::BuildHOMModel
    # asserts on it (R_ASSERT(GetReference()) - no .object behind it) and the editor ends; the Rust build
    # leaves such an object out of every level file. Taken away so the C++ build runs to its end.
    S("select_objects", names="test_barrel"),
    S("delete_selected"),
    S("level_build", action="make_all", timeout=1800,
      cpp={"cmd": "exec_command", "args": {"name": "COMMAND_BUILD"}},
      answer="skip: the C++ editor runs the command table's row (exec_command answers its id and result 1), "
             "the Rust one level_build make_all; what both wrote is compared file by file"),
]

# A level that has everything Escape lacks: the smallest of the repository's 35 scenes whose parts hold
# sectors, portals, detail objects and wallmarks - l10_limansk (10.2 MB of parts: 79 sectors, 274 portals,
# 251001 detail slots of 18 objects, 127 wallmarks; l08_yantar, the only smaller scene with sectors,
# portals and details, has no wallmark). Copied from rawdata\levels into the copy of Test. Make All
# writes build.details, build_details.dds, level.wallmarks and the sector and portal tables of build.prj.
SCENE2 = "l10_limansk"
S8 = [
    S("open_scene", scene=SCENE2 + ".level", timeout=300, settle="viewport"),
    S("level_build", action="make_all", timeout=300,
      cpp={"cmd": "exec_command", "args": {"name": "COMMAND_BUILD"}},
      answer="skip: the C++ editor runs the command table's row (exec_command answers its id and result 1), "
             "the Rust one level_build make_all; what both wrote is compared file by file"),
]

# The Play path's steps both editors have: the rows of the C++ Build menu's command table
# (cpp-editor-last:Source/Editors/LevelEditor/Editor/UI_LevelMain.cpp: COMMAND_MAKE_GAME, _MAKE_AIMAP, _MAKE_DETAILS, _MAKE_HOM, _MAKE_SOM; Make All is
# s7 and s8), one at a time into an empty level folder, on the scene that gives each of them work.
# Collision, AI map, game graph, the editor spawn and the start of the game are the C++ top bar's
# buttons (UITopBarForm: ClickCForm, ClickAIMap, ClickGGraph, ClickPlayPC) with no command row and no
# MCP path: the Rust editor alone has them over MCP, and the game loading its Play is their proof
# (STATE.md, "The reviewer's verification, closed").
def _make(action: str, row: str) -> dict:
    return S("level_build", action=action, timeout=300,
             cpp={"cmd": "exec_command", "args": {"name": row}},
             answer="skip: the C++ editor runs the command table's row (exec_command answers its id and result 1), "
                    "the Rust one level_build " + action + "; what both wrote is compared file by file")


S9 = [
    S("open_scene", scene=SCENE2 + ".level", timeout=300, settle="viewport"),
    _make("make_game", "COMMAND_MAKE_GAME"),
    _make("make_aimap", "COMMAND_MAKE_AIMAP"),
    _make("make_details", "COMMAND_MAKE_DETAILS"),
    _make("make_hom", "COMMAND_MAKE_HOM"),
    # limansk has no sound occluder (both editors refuse its SOM): Escape's
    S("open_scene", scene="l01_escape.level", timeout=300, settle="viewport"),
    S("select_objects", names="test_barrel"),
    S("delete_selected"),
    _make("make_som", "COMMAND_MAKE_SOM"),
]

SCENARIOS: dict[str, dict] = {
    "s1_reads": {"source": "od", "steps": S1},
    "s1_shell": {"source": "od", "steps": S1W, "window": True},
    "s2_manifest": {"source": "od", "steps": S2},
    "s3_assets": {"source": "od", "steps": S3_steps()},
    "s4_content": {"source": "od", "steps": S4},
    "s5_builds": {"source": "od", "steps": S5, "qa_github": True},
    "s6_scene": {"source": "test", "steps": S6},
    "s7_levelbuild": {"source": "test", "steps": S7},
    "s8_levelbuild2": {"source": "test", "scenes": [SCENE2], "steps": S8},
    "s9_play": {"source": "test", "scenes": [SCENE2], "steps": S9},
}

# The repository's own scenes (the editor's data): a scenario's "scenes" are copied into its copy's
# rawdata\levels, and they count as part of its source.
REPO_SCENES = os.path.join(REPO, "rawdata", "levels")


def scene_files(name: str) -> list[tuple[str, str]]:
    """(path under the project, absolute source path) of every file of the scenario's scenes."""
    out = []
    for sc in SCENARIOS[name].get("scenes", []):
        out.append(("rawdata/levels/" + sc + ".level", os.path.join(REPO_SCENES, sc + ".level")))
        root = os.path.join(REPO_SCENES, sc)
        for dirpath, dirs, files in os.walk(root):
            dirs.sort()
            for n in sorted(files):
                p = os.path.join(dirpath, n)
                rel = "rawdata/levels/" + sc + "/" + os.path.relpath(p, root).replace("\\", "/")
                out.append((rel, p))
    return out


# ------------------------------------------------------------------------------------------------
# Plumbing

def rmtree(path: str) -> None:
    """Removes a tree for good, read-only entries too (git's objects are read-only)."""
    def unlock(func, p, _exc):
        try:
            os.chmod(p, stat.S_IWRITE)
            func(p)
        except OSError:
            pass
    if os.path.lexists(path):
        shutil.rmtree(path, onerror=unlock)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def stamp_file(path: str) -> str:
    try:
        st = os.stat(path)
        return f"{st.st_size}:{st.st_mtime_ns}"
    except OSError:
        return "missing"


def stamp_tree(root: str) -> str:
    h = hashlib.sha256()
    for dirpath, dirs, files in os.walk(root):
        dirs.sort()
        for name in sorted(files):
            p = os.path.join(dirpath, name)
            h.update(f"{os.path.relpath(p, root)}|{stamp_file(p)}\n".encode("utf-8", "surrogateescape"))
    return h.hexdigest()


def game_stamp() -> str:
    # the game's version is its engine binary; its data are the archives and the loose gamedata folder
    parts = [stamp_file(os.path.join(GAME, "xrEngine.exe")), stamp_file(os.path.join(GAME, "fsgame.ltx"))]
    for sub in ("db", "gamedata", "modules"):
        p = os.path.join(GAME, sub)
        if os.path.isdir(p):
            parts.append(sub + ":" + str(os.stat(p).st_mtime_ns))
            for name in sorted(os.listdir(p)):
                parts.append(name + ":" + stamp_file(os.path.join(p, name)))
    return sha256("\n".join(parts).encode())


# The C++ editor's stamp when it left the repository (the cutover, tag cpp-editor-last): LevelEditor.exe and every
# DLL beside it as the records were made. Its sources are gone, so it can no longer change, and the records stay
# valid wherever the binary lies (Bin\x64\CppEditor now).
CPP_STAMP = "4064068b014fe4f153ed2d7610bc32bc2f28664250e73073464902e295f161a4"


def cpp_stamp() -> str:
    """The C++ editor's part of a record's key: fixed since the cutover (CPP_STAMP)."""
    return CPP_STAMP


def plain_steps(steps: list[dict]) -> list:
    return [{"gen": s["gen"].__name__} if "gen" in s else s for s in steps]


def gen_sources(name: str) -> str:
    """The code that expands one scenario's steps: its own generators and their helpers."""
    import inspect
    names = sorted({s["gen"].__name__ for s in SCENARIOS[name]["steps"] if "gen" in s})
    if names:
        # the helpers, and the generators a generator's own steps name
        names += ["S", "last", "_class_sections", "gen_scene_exports"]
    # the data the generators read decides the steps as much as their code
    data = repr((sorted(NO_SERVER_CLASS), sorted(NO_ENTITY_SECTION), XSPAWN, LEVEL_DIR, EXPORT_OBJECTS, GOLDEN_REF))
    return "".join(inspect.getsource(globals()[n]) for n in names) + data


def scenario_key(name: str, cache: dict) -> str:
    sc = SCENARIOS[name]
    if "game" not in cache:
        cache["game"] = game_stamp()
        cache["cpp"] = cpp_stamp()
        cache["examples"] = stamp_tree(EXAMPLES)
    src = sc["source"]
    if src not in cache:
        cache[src] = stamp_tree(SOURCES[src])
    scenes = sha256("\n".join(f"{rel}|{stamp_file(p)}" for rel, p in scene_files(name)).encode()) if sc.get("scenes") else ""
    data = json.dumps({"format": ORACLE_FORMAT, "steps": plain_steps(sc["steps"]), "qa_github": sc.get("qa_github", False),
                       "window": sc.get("window", False), "gens": sha256(gen_sources(name).encode()),
                       "cpp": cache["cpp"], "src": cache[src], "game": cache["game"],
                       "examples": cache["examples"] if src == "od" else "", "scenes": scenes},
                      sort_keys=True, ensure_ascii=False)
    return sha256(data.encode())[:16]


def rust_fingerprint() -> str:
    return stamp_file(RUST_EXE)


def port_busy(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.3):
            return True
    except OSError:
        return False


def mcp(port: int, cmd: str, args: dict | None = None, timeout: float = STEP_TIMEOUT) -> dict:
    payload = {"cmd": cmd}
    payload.update(args or {})
    with socket.create_connection(("127.0.0.1", port), timeout=10) as s:
        s.settimeout(timeout)
        s.sendall((json.dumps(payload) + "\n").encode())
        buf = bytearray()
        while not buf.endswith(b"\n"):
            chunk = s.recv(1 << 20)
            if not chunk:
                break
            buf += chunk
    text = buf.decode("utf-8", "replace").strip()
    if not text:
        return {"ok": False, "error": "<no answer>"}
    return json.loads(text)


class Side:
    """One editor on one copy of the scenario's project."""

    def __init__(self, scenario: str, which: str, base: str, port: int, log) -> None:
        self.scenario, self.which, self.port, self.log = scenario, which, port, log
        self.base = base
        src = SOURCES[SCENARIOS[scenario]["source"]]
        self.project = os.path.join(base, "p", os.path.basename(src))
        self.home = os.path.join(base, "home")
        self.proc: subprocess.Popen | None = None
        self.watch_blobs: dict[str, bytes] = {}

    # -- the copy

    def prepare(self) -> None:
        sc = SCENARIOS[self.scenario]
        src = SOURCES[sc["source"]]
        if os.path.isdir(self.base):
            rmtree(self.base)
        # THE copy every QA script makes: under the system temp folder, without .git, releases, temp,
        # logs and backup, its project.ltx naming the copy (refused, exit code 2, anywhere else)
        qa_target.copy_project(src, self.project)
        # the copy leaves the source's scratch out; the C++ editor keeps its undo snapshots in the
        # project's temp folder and has no undo without one
        os.makedirs(os.path.join(self.project, "temp"), exist_ok=True)
        for rel, path in scene_files(self.scenario):
            dst = os.path.join(self.project, *rel.split("/"))
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            shutil.copy2(path, dst)
        if sc["source"] == "od":
            dst = os.path.join(self.project, "quests", "examples")
            os.makedirs(dst, exist_ok=True)
            for n in sorted(os.listdir(EXAMPLES)):
                shutil.copy2(os.path.join(EXAMPLES, n), os.path.join(dst, n))
        os.makedirs(os.path.join(self.base, "out"), exist_ok=True)
        os.makedirs(os.path.join(self.base, "catalog_home"), exist_ok=True)
        os.makedirs(self.home, exist_ok=True)
        # the copy's own project.ltx: no scene opens by itself, a build never reaches the game
        pl = os.path.join(self.project, "project.ltx")
        lines = []
        with open(pl, "r", encoding="utf-8", errors="surrogateescape", newline="") as f:
            for line in f.read().splitlines():
                key = line.split("=", 1)[0].strip().lower()
                if key == "last_scene":
                    continue
                if key == "export_target":
                    line = "export_target=" + os.path.join(self.base, "out", "game")
                lines.append(line)
        with open(pl, "w", encoding="utf-8", errors="surrogateescape", newline="") as f:
            f.write("\r\n".join(lines) + "\r\n")

    def subst(self, value):
        if isinstance(value, str):
            return value.replace("<project>", self.project).replace("<temp>", self.base)
        if isinstance(value, dict):
            return {k: self.subst(v) for k, v in value.items()}
        if isinstance(value, list):
            return [self.subst(v) for v in value]
        return value

    # -- the editor

    def start(self) -> None:
        env = dict(os.environ)
        env["GRIZZLY_CATALOG_HOME"] = os.path.join(self.base, "catalog_home")
        qa = SCENARIOS[self.scenario].get("qa_github", False)
        if self.which == "cpp":
            startup = subprocess.STARTUPINFO()
            startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
            startup.wShowWindow = 7  # SW_SHOWMINNOACTIVE
            qa_target.guard_project(self.project)
            cmd = [CPP_EXE, "-nodlg", "-project", self.project] + (["-qa_github"] if qa else [])
            self.proc = subprocess.Popen(cmd, cwd=os.path.dirname(CPP_EXE), startupinfo=startup, env=env)
        else:
            window = SCENARIOS[self.scenario].get("window", False)
            qa_target.guard_project(self.project)
            # the home again in the environment: an editor this one starts without -home keeps to it too
            env["GRIZZLY_HOME"] = self.home
            cmd = [RUST_EXE, "-mcp_port", str(self.port), "-home", self.home, "-nodlg", "-project", self.project]
            cmd += ([] if window else ["-headless"]) + (["-qa_github"] if qa else [])
            startup = subprocess.STARTUPINFO()
            startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
            startup.wShowWindow = 7  # SW_SHOWMINNOACTIVE
            self.proc = subprocess.Popen(cmd, cwd=os.path.dirname(RUST_EXE), env=env, startupinfo=startup,
                                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = time.monotonic() + START_TIMEOUT
        while time.monotonic() < deadline:
            if self.proc.poll() is not None:
                raise RuntimeError(f"{self.which}: the editor exited during startup ({self.proc.returncode})")
            try:
                a = mcp(self.port, "mod_manifest", timeout=30)
                if a.get("ok"):
                    break
            except (OSError, ValueError):
                pass
            time.sleep(POLL)
        else:
            raise RuntimeError(f"{self.which}: the editor never opened the project")
        self.settle()

    def settle(self, kind: str = "ui") -> None:
        if self.which == "cpp":
            if kind == "viewport":
                mcp(self.port, "screenshot_viewport", timeout=120)
            # a minimised C++ editor draws only when asked: one frame, then whatever it loads runs out
            mcp(self.port, "ui_list", timeout=60)
            deadline = time.monotonic() + 300
            while mcp(self.port, "progress").get("active") and time.monotonic() < deadline:
                time.sleep(POLL * 4)

    def close(self) -> None:
        if not self.proc:
            return
        try:
            mcp(self.port, "window_control", {"action": "close", "discard": True}, timeout=30)
        except (OSError, ValueError):
            pass
        try:
            self.proc.wait(timeout=CLOSE_TIMEOUT)
        except subprocess.TimeoutExpired:
            self.log(f"! {self.which} did not leave within {CLOSE_TIMEOUT:.0f} s: ended by force (pid {self.proc.pid})")
            self.proc.kill()
            self.proc.wait(timeout=30)
        self.proc = None

    # -- normalisation

    def norm_text(self, text: str) -> str:
        for path, name in ((self.project, "<project>"), (self.base, "<temp>")):
            for variant in {path, path.replace("\\", "/"), path.replace("\\", "\\\\")}:
                text = re.sub(re.escape(variant), name.replace("\\", "\\\\"), text, flags=re.IGNORECASE)
        return text

    def norm(self, value):
        if isinstance(value, str):
            return self.norm_text(value)
        if isinstance(value, dict):
            return {self.norm_text(k): self.norm(v) for k, v in value.items()}
        if isinstance(value, list):
            return [self.norm(v) for v in value]
        return value

    def norm_bytes(self, data: bytes) -> bytes:
        if len(data) > NORMALISE_LIMIT:
            return data
        for path, name in ((self.project, b"<project>"), (self.base, b"<temp>")):
            for variant in {path, path.replace("\\", "/"), path.replace("\\", "\\\\")}:
                for enc in ("utf-8",):
                    pat = re.escape(variant.encode(enc))
                    data = re.sub(pat, name, data, flags=re.IGNORECASE)
        return data

    def read_norm(self, path: str) -> bytes | None:
        try:
            with open(path, "rb") as f:
                return self.norm_bytes(f.read())
        except OSError:
            return None

    def manifest(self) -> dict[str, tuple[int, str]]:
        out = {}
        for top, tag in ((self.project, "project"), (os.path.join(self.base, "out"), "temp/out")):
            for dirpath, dirs, files in os.walk(top):
                dirs.sort()
                for n in files:
                    p = os.path.join(dirpath, n)
                    rel = tag + "/" + os.path.relpath(p, top).replace("\\", "/")
                    data = self.read_norm(p) or b""
                    out[rel] = (len(data), sha256(data), p)
        return out


# ------------------------------------------------------------------------------------------------
# Running one side of a scenario

def expectation(step: dict, answer: dict, row: bool = False) -> str:
    """Empty when the answer is what the step expects; else the sentence that fails the scenario. A row of
    the C++ command table (`row`) answers `ok` whatever it did: its `result` 0 is the refusal."""
    want = step.get("expect", "ok")
    got = "ok" if answer.get("ok") is True and not (row and answer.get("result") == 0) else "refused"
    if got == want:
        return ""
    why = answer.get("error") or answer.get("errors") or ""
    return f"expected {want}, answered {got}" + (f": {short(why)}" if why else "")


# The repository's own gamedata\levels: no level build of either editor may land there (an open project
# remounts $game_levels$ into the project, EditorProject's kMount).
REPO_LEVELS = os.path.join(REPO, "gamedata", "levels")


def run_steps(side: Side, steps: list[dict], expand: bool, log) -> tuple[list[dict], list[dict], list[dict], list[str]]:
    """Runs the steps; returns the steps run, their answers, their watches (normalised) and the steps
    whose answer was not what they expect."""
    done: list[dict] = []
    answers: list[dict] = []
    watches: list[dict] = []
    unmet: list[str] = []
    pairs: list = []
    queue = list(steps)
    while queue:
        step = queue.pop(0)
        if "gen" in step:
            if not expand:
                raise RuntimeError("a recorded scenario holds an unexpanded step")
            queue[0:0] = step["gen"](pairs)
            continue
        own = step["cpp"] if side.which == "cpp" and "cpp" in step else step
        args = side.subst(own["args"])
        t0 = time.monotonic()
        try:
            answer = mcp(side.port, own["cmd"], args, timeout=step.get("timeout", STEP_TIMEOUT))
        except socket.timeout:
            answer = {"ok": False, "error": "<timeout>"}
            log(f"  ! {side.which} {own['cmd']}: no answer within {step.get('timeout', STEP_TIMEOUT):.0f} s")
        except (OSError, ValueError) as e:
            answer = {"ok": False, "error": f"<transport: {e}>"}
        miss = expectation(step, answer, own["cmd"] == "exec_command")
        if miss:
            unmet.append(f"step {len(done) + 1} {step['cmd']}: {miss}")
            log(f"  ! {side.which} step {len(done) + 1} {step['cmd']}: {miss}")
        # an editor that died fails the run at once, naming the step: every later step would only
        # wait for a refused connection
        if side.proc and side.proc.poll() is not None:
            raise RuntimeError(f"{side.which}: the editor exited ({side.proc.returncode}) during step "
                               f"{len(done) + 1} {step['cmd']} {json.dumps(args, ensure_ascii=False)[:200]}")
        dt = time.monotonic() - t0
        if dt > 10:
            log(f"  . {side.which} {step['cmd']} took {dt:.1f} s")
        norm = side.norm(answer)
        w = {}
        for rel in step.get("watch", []):
            data = side.read_norm(os.path.join(side.project, rel))
            w[rel] = watch_form(side, data)
        done.append(step)
        answers.append(norm)
        watches.append(w)
        pairs.append((step, norm))
        if step.get("settle"):
            side.settle(step["settle"])
    return done, answers, watches, unmet


def watch_form(side: "Side", data: bytes | None) -> str | None:
    """A watched file as the record keeps it: its text, or for a binary file `sha256:<hex>:<size>` with the
    bytes kept beside the record (Side.watch_blobs)."""
    if data is None:
        return None
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError:
        digest = sha256(data)
        side.watch_blobs[digest] = data
        return f"sha256:{digest}:{len(data)}"


def check_kept(which: str, before: dict[str, bytes | None]) -> list[str]:
    bad = []
    for name, data in before.items():
        p = os.path.join(REPO, name)
        now = open(p, "rb").read() if os.path.isfile(p) else None
        if now != data:
            bad.append(name)
    return bad


def snapshot_kept(names: list[str]) -> dict[str, bytes | None]:
    out = {}
    for n in names:
        p = os.path.join(REPO, n)
        out[n] = open(p, "rb").read() if os.path.isfile(p) else None
    return out


def restore_kept(saved: dict[str, bytes | None]) -> None:
    for n, data in saved.items():
        p = os.path.join(REPO, n)
        if data is None:
            if os.path.exists(p):
                os.remove(p)
        else:
            with open(p, "wb") as f:
                f.write(data)


def source_manifest(name: str, cache: dict) -> dict[str, str]:
    src = SCENARIOS[name]["source"]
    if ("srcman", src) not in cache:
        m = {}
        root = SOURCES[src]
        for dirpath, dirs, files in os.walk(root):
            for n in files:
                p = os.path.join(dirpath, n)
                try:
                    if os.path.getsize(p) <= BLOB_LIMIT:
                        m["project/" + os.path.relpath(p, root).replace("\\", "/")] = sha256(open(p, "rb").read())
                except OSError:
                    pass
        cache[("srcman", src)] = m
    m = dict(cache[("srcman", src)])
    for rel, p in scene_files(name):
        if os.path.getsize(p) <= BLOB_LIMIT:
            m["project/" + rel] = sha256(open(p, "rb").read())
    return m


def record_oracle(name: str, key: str, base_root: str, log, cache: dict) -> str:
    """Runs the C++ side of one scenario and keeps it under Intermediate/oracle."""
    rec_dir = os.path.join(ORACLE, f"{name}-{key}")
    side = Side(name, "cpp", os.path.join(base_root, name, "cpp"), CPP_PORT, log)
    side.prepare()
    kept = snapshot_kept(CPP_KEPT)
    levels_there = os.path.isdir(REPO_LEVELS)
    t0 = time.monotonic()
    try:
        # the C++ editor starts as the Rust one does in its fresh home: no project list, no
        # settings, no layout of the author's (all put back byte for byte below)
        restore_kept({n: None for n in CPP_KEPT})
        side.start()
        log(f"  {name}: C++ editor up in {time.monotonic() - t0:.1f} s")
        steps, answers, watches, unmet = run_steps(side, SCENARIOS[name]["steps"], True, log)
    finally:
        side.close()
        restore_kept(kept)
    bad = check_kept("cpp", kept)
    if bad:
        raise RuntimeError("the C++ editor's own files did not come back: " + ", ".join(bad))
    if not levels_there and os.path.isdir(REPO_LEVELS):
        raise RuntimeError(f"the C++ editor wrote into {REPO_LEVELS}: a level build must land in the project")
    if unmet:
        # an oracle of steps that did not do their work proves nothing
        raise RuntimeError("the C++ side did not answer as the steps expect: " + "; ".join(unmet))
    man = side.manifest()
    src = source_manifest(name, cache)
    tmp = rec_dir + ".part"
    if os.path.isdir(tmp):
        rmtree(tmp)
    os.makedirs(os.path.join(tmp, "blobs"))
    files = {}
    for digest, data in side.watch_blobs.items():
        with open(os.path.join(tmp, "blobs", digest), "wb") as f:
            f.write(data)
    for rel, (size, digest, path) in sorted(man.items()):
        files[rel] = [size, digest]
        rule = file_rule(rel)
        # a level build's files are kept whole whatever their size: a difference is read from them
        limit = LEVEL_BLOB_LIMIT if rel.startswith("project/gamedata/levels/") else BLOB_LIMIT
        if rule or (size <= limit and src.get(rel) != digest):
            data = side.read_norm(path) if size <= NORMALISE_LIMIT else open(path, "rb").read()
            data = data or b""
            blob = os.path.join(tmp, "blobs", digest)
            if not os.path.exists(blob):
                with open(blob, "wb") as f:
                    f.write(file_form(rule[0], data) if rule else data)
    rec = {"scenario": name, "key": key, "steps": steps, "answers": answers, "watches": watches,
           "files": files, "seconds": round(time.monotonic() - t0, 1)}
    with open(os.path.join(tmp, "record.json"), "w", encoding="utf-8") as f:
        json.dump(rec, f, ensure_ascii=False, indent=1)
    # a scenario keeps one record: the one of its current key
    for old in os.listdir(ORACLE):
        if old.startswith(name + "-") and not old.endswith(".part"):
            rmtree(os.path.join(ORACLE, old))
    os.replace(tmp, rec_dir)
    rmtree(side.base)
    log(f"  {name}: oracle recorded in {time.monotonic() - t0:.1f} s ({len(steps)} steps)")
    return rec_dir


def load_record(name: str, key: str) -> dict | None:
    p = os.path.join(ORACLE, f"{name}-{key}", "record.json")
    if not os.path.isfile(p):
        return None
    with open(p, encoding="utf-8") as f:
        return json.load(f)


# ------------------------------------------------------------------------------------------------
# Comparing

def ignored(cmd: str, path: str) -> str | None:
    # a key "cmd" holds for every step of the command, "cmd arg=value" for the steps with that argument
    for table in (IGNORE.get(cmd, {}), *(IGNORE.get(f"{cmd} {q}", {}) for q in CURRENT_ARGS), IGNORE["*"]):
        if path in table:
            return table[path]
    return None


CURRENT_ARGS: list[str] = []


def short(v) -> str:
    t = json.dumps(v, ensure_ascii=False)
    return t if len(t) <= 160 else t[:157] + "..."


def diff_json(cmd: str, a, b, path: str, out: list[str], hits: set[str]) -> None:
    if path and ignored(cmd, path) is not None:
        hits.add(path)
        return
    norm = NORMALISE.get((cmd, path))
    if norm:
        a2, b2 = norm[0](a), norm[0](b)
        if (a2, b2) != (a, b) and a2 == b2:
            hits.add(path)
        a, b = a2, b2
    if isinstance(a, dict) and isinstance(b, dict):
        for k in list(a.keys()) + [k for k in b.keys() if k not in a]:
            sub = f"{path}.{k}" if path else k
            if k not in b:
                if ignored(cmd, sub) is None:
                    out.append(f"{sub}: {short(a[k])} | <absent>")
                else:
                    hits.add(sub)
            elif k not in a:
                if ignored(cmd, sub) is None:
                    out.append(f"{sub}: <absent> | {short(b[k])}")
                else:
                    hits.add(sub)
            else:
                diff_json(cmd, a[k], b[k], sub, out, hits)
        return
    if isinstance(a, list) and isinstance(b, list):
        for list_path, key, names, _why in IGNORE_ROWS.get(cmd, []):
            if path == list_path:
                keep_a = [x for x in a if not (isinstance(x, dict) and x.get(key) in names)]
                keep_b = [x for x in b if not (isinstance(x, dict) and x.get(key) in names)]
                if len(keep_a) != len(a) or len(keep_b) != len(b):
                    hits.add(f"{path}[{key}=...]")
                a, b = keep_a, keep_b
        sub = path + "[]"
        if len(a) != len(b):
            out.append(f"{path}: {len(a)} items | {len(b)} items; first: {short(a[:1])} | {short(b[:1])}")
            return
        start, bad = len(out), 0
        for x, y in zip(a, b):
            n = len(out)
            diff_json(cmd, x, y, sub, out, hits)
            if len(out) > n:
                bad += 1
            if len(out) - start > 4:
                del out[start + 4:]
        if bad > 1 and len(out) - start >= 4:
            out.append(f"{sub}: {bad} of {len(a)} elements differ")
        return
    if type(a) is bool or type(b) is bool:
        same = a is b
    elif isinstance(a, (int, float)) and isinstance(b, (int, float)):
        same = a == b
    else:
        same = a == b
    if not same:
        out.append(f"{path or '<answer>'}: {short(a)} | {short(b)}")


# ------------------------------------------------------------------------------------------------
# The level build's files compared by rule (STATE.md, "For the plan": one rule per file, each with
# the reason the bytes may differ and the game and the compilers do not care). A rule answers the
# differences it finds; none means the two files are equal under it.

def _chunks(data: bytes) -> dict[int, bytes]:
    out, off = {}, 0
    while off + 8 <= len(data):
        cid, size = struct.unpack_from("<II", data, off)
        out[cid] = data[off + 8:off + 8 + size]
        off += 8 + size
    return out


def _close(a: float, b: float) -> bool:
    """Equal within FLOAT_ULPS units in the last place of the larger of the two (and of 1.0: a small result
    of a sum of large terms keeps the terms' absolute rounding)."""
    scale = max(abs(a), abs(b), 1.0)
    ulp = 2.0 ** (math.frexp(scale)[1] - 24)
    return abs(a - b) <= FLOAT_ULPS * ulp


# a float the two compilers' code may round differently in its last bits (a point light's colour times
# its brightness): equal within this many units in the last place
FLOAT_ULPS = 4
# a LOD quad's corners and UVs come out of sums of products over the model's box that the two compilers'
# code rounds differently: within 2 micrometres (16 ULP of 1.0; seen: 8 ULP)
LOD_TOLERANCE = 2e-6
# the fields of `Flight` (b_light) that SceneBuilder sets, by D3DLIGHTTYPE; the others are the C++
# stack's leftovers (`Flight RL;` / `b_light L;` are never cleared): 0 type, 1-4 diffuse, 5-8 specular,
# 9-12 ambient, 13-15 position, 16-18 direction, 19 range, 20 falloff, 21-23 attenuation, 24 theta, 25 phi
_LIGHT_SET = {
    1: {0, 1, 2, 3, 13, 14, 15, 16, 17, 18, 19, 21, 22, 23, 25},  # BuildLight: diffuse.rgb, not .a
    3: {0, 1, 2, 3, 4, 13, 14, 15, 16, 17, 18},  # BuildSun / BuildHemiLights: type, diffuse, direction
}


def _light_diffs(a: bytes, b: bytes, size: int, label: str) -> list[str]:
    if len(a) != len(b):
        return [f"{label}: {len(a) // size} | {len(b) // size} lights"]
    out = []
    for n, i in enumerate(range(0, len(a), size)):
        x, y = a[i:i + size], b[i:i + size]
        if x[:4] != y[:4]:
            out.append(f"{label} {n}: controller differs")
            continue
        kind = struct.unpack_from("<I", x, 4)[0]
        fields = _LIGHT_SET.get(kind)
        if fields is None:
            if x[:108] != y[:108]:
                out.append(f"{label} {n}: type {kind} differs")
            continue
        for f in sorted(fields):
            at = 4 + 4 * f
            if f == 0:
                same = x[at:at + 4] == y[at:at + 4]
            else:
                same = _close(struct.unpack_from("<f", x, at)[0], struct.unpack_from("<f", y, at)[0])
            if not same:
                out.append(f"{label} {n}: field {f} differs")
                break
        if size == 144:
            # b_light_dynamic: the sectors in use; the svector's unused slots are not cleared
            cx, cy = struct.unpack_from("<I", x, 140)[0], struct.unpack_from("<I", y, 140)[0]
            if cx != cy or x[108:108 + 2 * min(cx, 16)] != y[108:108 + 2 * min(cy, 16)]:
                out.append(f"{label} {n}: sectors differ")
        if len(out) > 8:
            break
    return out


def _prj_tables(t: dict[int, bytes]):
    shaders = [t[5][i:i + 128].split(b"\0")[0] for i in range(0, len(t[5]), 128)]
    xrlc = [t[6][i:i + 128].split(b"\0")[0] for i in range(0, len(t[6]), 128)]
    tex = [(t[7][i:i + 128].split(b"\0")[0], t[7][i + 128:i + 144]) for i in range(0, len(t[7]), 144)]
    mats = []
    for i in range(0, len(t[4]), 16):
        surf, sh, shx, sector, res, _pad, area = struct.unpack_from("<6HI", t[4], i)
        name = tex[surf] if surf < len(tex) else surf
        # BuildObjectLOD's `b_material mtl;` is never cleared: reserved and the area are leftovers
        if name and name[0] == b"level_lods":
            res, area = 0, 0
        mats.append((name, shaders[sh] if sh < len(shaders) else sh, xrlc[shx] if shx < len(xrlc) else shx,
                     sector, res, area))
    return shaders, xrlc, tex, mats


def _face_key(f: bytes, mats, group: bytes):
    m = struct.unpack_from("<H", f, 36)[0]
    # the face's material by what it names; the two bytes after it are struct padding (b_face)
    return f[:36], mats[m] if m < len(mats) else m, f[40:44], group


# a portal's corners are projected onto its plane and back in float (Mgc::OrthogonalPlaneFit, build_camera)
# with the operations in another order: within this many metres (seen: 6.1e-5)
PORTAL_TOLERANCE = 1e-4
# a portal simplified from more than six corners drops the edge of the smallest triangle each round, and an
# arch's near-equal triangles fall to the rounding (limansk's portal_0047, eleven corners): such a portal
# passes when the two polygons' areas agree within 2 % (seen: 0.55 %)
PORTAL_AREA = 0.02


def _polygon_area(v: tuple, n: int) -> float:
    sx = sy = sz = 0.0
    for k in range(n):
        a, b = v[3 * k:3 * k + 3], v[3 * ((k + 1) % n):3 * ((k + 1) % n) + 3]
        sx += a[1] * b[2] - a[2] * b[1]
        sy += a[2] * b[0] - a[0] * b[2]
        sz += a[0] * b[1] - a[1] * b[0]
    return 0.5 * math.sqrt(sx * sx + sy * sy + sz * sz)


def _portal_diffs(a: bytes, b: bytes) -> list[str]:
    """build.prj's portals (b_portal: two sectors, six corners, the count in use)."""
    if len(a) != len(b):
        return [f"{len(a) // 80} | {len(b) // 80} portals"]
    out = []
    for n, i in enumerate(range(0, len(a), 80)):
        x, y = a[i:i + 80], b[i:i + 80]
        if x == y:
            continue
        if x[:4] != y[:4] or x[76:80] != y[76:80]:
            out.append(f"portal {n}: sectors or corner count differ")
            continue
        c = min(6, struct.unpack_from("<I", x, 76)[0])
        vx, vy = struct.unpack_from("<18f", x, 4), struct.unpack_from("<18f", y, 4)
        if all(abs(p - q) <= PORTAL_TOLERANCE for p, q in zip(vx[:3 * c], vy[:3 * c])):
            continue
        ax, ay = _polygon_area(vx, c), _polygon_area(vy, c)
        if ax > 0 and abs(ax - ay) <= PORTAL_AREA * ax:
            continue
        out.append(f"portal {n}: corners differ")
        if len(out) > 8:
            break
    return out


def cmp_build_details(a: bytes, b: bytes) -> list[str]:
    ta, tb = _chunks(a), _chunks(b)
    if sorted(ta) != sorted(tb):
        return [f"chunks {sorted(ta)} | {sorted(tb)}"]
    out = [f"chunk {c} differs" for c in sorted(ta) if c != 1 and ta[c] != tb[c]]

    def models(raw: bytes):
        res = []
        for _cid, m in sorted(_chunks(raw).items()):
            o = m.index(b"\0")
            o = m.index(b"\0", o + 1) + 1
            nv, ni = struct.unpack_from("<II", m, o + 12)
            head, o = m[:o + 20], o + 20
            verts = [struct.unpack_from("<5f", m, o + 20 * k) for k in range(nv)]
            res.append((head, verts, m[o + 20 * nv:]))
        return res
    ma, mb = models(ta[1]), models(tb[1])
    if len(ma) != len(mb):
        return out + [f"{len(ma)} | {len(mb)} models"]
    for n, ((ha, va, ia), (hb, vb, ib)) in enumerate(zip(ma, mb)):
        if ha != hb or ia != ib or len(va) != len(vb) or any(p[:3] != q[:3] for p, q in zip(va, vb)):
            out.append(f"model {n}: geometry differs")
            continue
        if va:
            du, dv = vb[0][3] - va[0][3], vb[0][4] - va[0][4]
            if not all(abs(q[3] - p[3] - du) <= 1e-6 and abs(q[4] - p[4] - dv) <= 1e-6 for p, q in zip(va, vb)):
                out.append(f"model {n}: UVs differ by more than one shift")
    return out


def cmp_build_details_dds(a: bytes, b: bytes) -> list[str]:
    import numpy as np
    from PIL import Image
    if len(a) != len(b) or a[:128] != b[:128]:
        return ["header or size differs"]
    pa = np.asarray(Image.open(io.BytesIO(a)).convert("RGBA"), dtype=np.int16)
    pb = np.asarray(Image.open(io.BytesIO(b)).convert("RGBA"), dtype=np.int16)
    t = 128
    h, w = pa.shape[:2]
    tiles = lambda p: [p[y:y + t, x:x + t] for y in range(0, h, t) for x in range(0, w, t)]  # noqa: E731
    left = tiles(pa)
    out = []
    for n, tile in enumerate(tiles(pb)):
        found = None
        for j, other in enumerate(left):
            alpha = float(np.abs(tile[..., 3] - other[..., 3]).mean())
            opaque = (tile[..., 3] >= 128) & (other[..., 3] >= 128)
            colour = float(np.abs(tile[..., :3] - other[..., :3])[opaque].mean()) if opaque.any() else 0.0
            if alpha <= 1.0 and colour <= 8.0:
                found = j
                break
        if found is None:
            out.append(f"tile {n} has no match")
            if len(out) > 8:
                break
        else:
            del left[found]
    return out


def cmp_build_prj(a: bytes, b: bytes) -> list[str]:
    ta, tb = _chunks(a), _chunks(b)
    if sorted(ta) != sorted(tb):
        return [f"chunks {sorted(ta)} | {sorted(tb)}"]
    out = []
    for cid, what in ((0, "version"), (1, "parameters"), (2, "vertices"), (8, "glows"),
                      (10, "light controls"), (15, "multiple-usage references")):
        if ta[cid] != tb[cid]:
            out.append(f"{what} differ")
    out += _portal_diffs(ta[9], tb[9])
    sa, xa, xta, ma = _prj_tables(ta)
    sb, xb, xtb, mb = _prj_tables(tb)
    # the tables are built in the order faces meet them, and the faces of a mesh come in the order of
    # its surface map, keyed by CSurface pointers (D14): compared as sets of what they name
    for name, x, y in (("render shaders", sa, sb), ("compiler shaders", xa, xb), ("textures", xta, xtb),
                       ("materials", ma, mb)):
        if collections.Counter(x) != collections.Counter(y):
            out.append(f"{name} differ as a set")
    ga = [ta[16][i:i + 4] for i in range(0, len(ta[16]), 4)]
    gb = [tb[16][i:i + 4] for i in range(0, len(tb[16]), 4)]
    fa = collections.Counter(_face_key(ta[3][i:i + 44], ma, ga[k] if k < len(ga) else b"")
                             for k, i in enumerate(range(0, len(ta[3]), 44)))
    fb = collections.Counter(_face_key(tb[3][i:i + 44], mb, gb[k] if k < len(gb) else b"")
                             for k, i in enumerate(range(0, len(tb[3]), 44)))
    if fa != fb:
        out.append(f"static faces differ as a multiset ({sum((fa - fb).values())} | {sum((fb - fa).values())} unmatched)")
    out += _light_diffs(ta[11], tb[11], 108, "static light")
    out += _light_diffs(ta[12], tb[12], 144, "dynamic light")
    # LOD models in order: the quads' corners and UVs within LOD_TOLERANCE, the material by what it names
    if len(ta[13]) != len(tb[13]):
        out.append("LOD model count differs")
    else:
        for n, i in enumerate(range(0, len(ta[13]), 644)):
            va, vb = struct.unpack_from("<160f", ta[13], i), struct.unpack_from("<160f", tb[13], i)
            la, lb = struct.unpack_from("<I", ta[13], i + 640)[0], struct.unpack_from("<I", tb[13], i + 640)[0]
            if not all(abs(p - q) <= LOD_TOLERANCE for p, q in zip(va, vb)) or ma[la] != mb[lb]:
                out.append(f"LOD model {n} differs")
                break
    # multiple-usage models in order: name, vertices, LOD, faces as a multiset (the surface map again)
    def models(raw: bytes, mats):
        res, o = [], 0
        while o < len(raw):
            end = raw.index(b"\0", o)
            name, o = raw[o:end], end + 1
            nv = struct.unpack_from("<I", raw, o)[0]
            verts = raw[o + 4:o + 4 + 12 * nv]
            o += 4 + 12 * nv
            nf = struct.unpack_from("<I", raw, o)[0]
            o += 4
            faces = [raw[o + 44 * k:o + 44 * k + 44] for k in range(nf)]
            o += 44 * nf
            lod = raw[o:o + 2]
            o += 2
            groups = [raw[o + 4 * k:o + 4 * k + 4] for k in range(nf)]
            o += 4 * nf
            res.append((name, verts, lod, collections.Counter(_face_key(f, mats, g) for f, g in zip(faces, groups))))
        return res
    mua, mub = models(ta[14], ma), models(tb[14], mb)
    if len(mua) != len(mub):
        out.append("multiple-usage model count differs")
    else:
        for x, y in zip(mua, mub):
            if x != y:
                out.append(f"multiple-usage model {x[0].decode('cp1251', 'replace')} differs")
                break
    return out


def _spawn_entries(data: bytes) -> list[tuple[bytes, bytes]]:
    out, off = [], 0
    while off + 8 <= len(data):
        _cid, size = struct.unpack_from("<II", data, off)
        p = data[off + 8:off + 8 + size]
        end = p.index(b"\0", 2) if b"\0" in p[2:] else 2
        out.append((p[2:end], p))
        off += 8 + size
    return out


def _loopholes(p: bytes, start: int):
    """se_smart_cover's tail from `start`: a count, then (id, on) pairs to the packet's end."""
    try:
        n, o, got = p[start], start + 1, []
        for _ in range(n):
            end = p.index(b"\0", o)
            got.append((p[o:end], p[end + 1]))
            o = end + 2
        return got if o == len(p) else None
    except (IndexError, ValueError):
        return None


def cmp_level_spawn(a: bytes, b: bytes) -> list[str]:
    ea = _spawn_entries(a)
    # D10: the breakable and climable objects the Rust build makes (the C++ one wrote errors instead)
    eb = [e for e in _spawn_entries(b) if e[0] not in (b"breakable_object", b"climable_object")]
    if len(ea) != len(eb):
        return [f"{len(ea)} | {len(eb)} entries (without the Rust build's breakable and climable objects)"]
    out = []
    for n, ((sa, x), (sb, y)) in enumerate(zip(ea, eb)):
        if x == y:
            continue
        if sa == sb == b"smart_cover" and len(x) == len(y):
            # se_smart_cover writes its loopholes in the order of Lua's `pairs` over a table keyed by
            # their ids; the game reads them back into such a table: compared as a set
            at = next(i for i in range(len(x)) if x[i] != y[i])
            if any(x[:s] == y[:s] and _loopholes(x, s) is not None
                   and sorted(_loopholes(x, s)) == sorted(_loopholes(y, s) or []) for s in range(at, max(0, at - 160), -1)):
                continue
        out.append(f"entry {n} ({sa.decode('cp1251', 'replace')}) differs")
        if len(out) > 8:
            break
    return out


def cmp_level_stat_txt(a: bytes, b: bytes) -> list[str]:
    # the author's statistic, read by neither the compilers nor the game: a vertex lands in its 1 m
    # cell by iFloor of a float, and a vertex a rounding away from a cell's border may land beside it
    la, lb = a.decode("ascii", "replace").splitlines(), b.decode("ascii", "replace").splitlines()
    if len(la) != len(lb):
        return ["line count differs"]
    out = []
    for x, y in zip(la, lb):
        if x == y:
            continue
        kx, _, vx = x.partition(":")
        ky, _, vy = y.partition(":")
        if kx == ky and kx.strip().startswith("Max ") and vx.strip().isdigit() and vy.strip().isdigit() \
                and abs(int(vx) - int(vy)) <= max(2, int(vx) // 200):
            continue
        out.append(f"{x!r} | {y!r}")
    return out


def cmp_level_stat_tga(a: bytes, b: bytes) -> list[str]:
    if len(a) != len(b) or a[:18] != b[:18]:
        return ["header or size differs"]
    pixels = (len(a) - 18) // 4
    differ = sum(1 for i in range(18, len(a), 4) if a[i:i + 4] != b[i:i + 4]) if a != b else 0
    # the same statistic as level_stat.txt: cells beside a vertex on a border
    return [] if differ <= max(1, pixels // 10000) else [f"{differ} of {pixels} pixels differ"]


def cmp_ogf(a: bytes, b: bytes) -> list[str]:
    """An exported world-space .ogf: its children - one per surface - come in the order of the editable
    object's surface map, keyed by CSurface pointers in the C++ (D14), and the collision mesh follows them.
    Every other chunk byte for byte, the children as a multiset of their bytes, the collision mesh as a
    multiset of triangles by their corners."""
    ta, tb = _chunks(a), _chunks(b)
    if sorted(ta) != sorted(tb):
        return [f"chunks {sorted(ta)} | {sorted(tb)}"]
    out = []
    for cid in sorted(ta):
        if cid in (9, 25, 26):
            continue
        if ta[cid] != tb[cid]:
            out.append(f"chunk {cid} differs")
    if 9 in ta:
        ca = collections.Counter(_chunks(ta[9]).values())
        cb = collections.Counter(_chunks(tb[9]).values())
        if ca != cb:
            out.append(f"children differ as a set ({sum((ca - cb).values())} | {sum((cb - ca).values())} unmatched)")
    if 25 in ta and 26 in ta:
        def tris(t):
            verts = [t[25][i:i + 12] for i in range(0, len(t[25]), 12)]
            # the indices are 32-bit, into the vertices of every child one after another
            idx = struct.unpack_from("<%dI" % (len(t[26]) // 4), t[26])
            return collections.Counter(tuple(verts[i] if i < len(verts) else i for i in idx[k:k + 3])
                                       for k in range(0, len(idx) - 2, 3))
        if tris(ta) != tris(tb):
            out.append("collision triangles differ as a set")
    return out


def _xcform(data: bytes):
    """overlay.xcform: the header, the material names, the vertices, the triangles (3 indices, a material, two
    bytes of padding), the cut boxes."""
    names, o = [], 16
    for _ in range(struct.unpack_from("<I", data, 12)[0]):
        end = data.index(b"\0", o)
        names.append(data[o:end])
        o = end + 1
    nv = struct.unpack_from("<I", data, o)[0]
    verts = data[o + 4:o + 4 + 12 * nv]
    o += 4 + 12 * nv
    nt = struct.unpack_from("<I", data, o)[0]
    tris = [data[o + 4 + 16 * k:o + 4 + 16 * k + 16] for k in range(nt)]
    o += 4 + 16 * nt
    return data[:12], names, verts, tris, data[o:]


def cmp_xcform(a: bytes, b: bytes) -> list[str]:
    """overlay.xcform: the material table is built in the order a mesh's surface map meets the surfaces - keyed by
    CSurface pointers in the C++ (D14, `// oracle:` in gz-export-level's overlay.rs) - so each triangle's material
    is compared by its name; the header, the vertices, the triangles' corners and the cut list byte for byte."""
    try:
        ha, na, va, ta, ca = _xcform(a)
        hb, nb, vb, tb, cb = _xcform(b)
    except (ValueError, struct.error) as e:
        return [f"does not read: {e}"]
    out = []
    if ha != hb:
        out.append("header differs")
    if sorted(na) != sorted(nb):
        out.append("material names differ as a set")
    if va != vb:
        out.append("vertices differ")
    if len(ta) != len(tb):
        out.append(f"{len(ta)} | {len(tb)} triangles")
    else:
        def named(t, names):
            m = struct.unpack_from("<H", t, 12)[0]
            return t[:12], names[m] if m < len(names) else m, t[14:16]
        bad = sum(1 for x, y in zip(ta, tb) if named(x, na) != named(y, nb))
        if bad:
            out.append(f"{bad} triangles differ")
    if ca != cb:
        out.append("cut list differs")
    return out


CMP = {
    "cmp_xcform": cmp_xcform,
    "cmp_ogf": cmp_ogf,
    "cmp_build_prj": cmp_build_prj,
    "cmp_level_spawn": cmp_level_spawn,
    "cmp_level_stat_txt": cmp_level_stat_txt,
    "cmp_level_stat_tga": cmp_level_stat_tga,
    "cmp_build_details": cmp_build_details,
    "cmp_build_details_dds": cmp_build_details_dds,
}


def file_form(rule: str, data: bytes) -> bytes:
    if rule == "skip":
        return b""
    if rule == "png":
        return b"PNG" if data[:8] == b"\x89PNG\r\n\x1a\n" else b"<not a PNG>"
    if rule == "zip_entries":
        try:
            z = zipfile.ZipFile(io.BytesIO(data))
            return "\n".join(f"{i.filename} {i.CRC:08x} {i.file_size}" for i in z.infolist()).encode()
        except zipfile.BadZipFile:
            return b"<not a zip>"
    if rule == "index_columns":
        lines = []
        for line in data.decode("utf-8", "replace").splitlines():
            parts = line.split(" ")
            if parts[0] == "file" and len(parts) >= 8:
                # file <sha> <size> <pack> <offset> <packed> <method> <path>
                parts = parts[:4] + parts[7:]
            lines.append(" ".join(parts))
        return "\n".join(lines).encode()
    if rule == "descriptor_sizes":
        lines = []
        for line in data.decode("utf-8", "replace").splitlines():
            k = line.split("=", 1)[0].strip()
            if k == "index" or k.endswith(".zip"):
                line = k + " = <bytes on disk>"
            lines.append(line)
        return "\n".join(lines).encode()
    return data


def file_rule(rel: str) -> tuple[str, str] | None:
    for pat, rule, why in FILE_RULES:
        if fnmatch.fnmatch(rel, pat):
            return rule, why
    return None


def text_diff(a: bytes | None, b: bytes | None) -> str:
    if a is None or b is None:
        return ""
    # a large or binary file: where it starts to differ (a line diff of megabytes runs for hours)
    if len(a) > (1 << 20) or len(b) > (1 << 20) or b"\0" in a[:4096] or b"\0" in b[:4096]:
        at, n = 0, min(len(a), len(b))
        while at < n and a[at:at + 65536] == b[at:at + 65536]:
            at += 65536
        while at < n and a[at] == b[at]:
            at += 1
        return f" (binary: first difference at byte {at} of {len(a)} | {len(b)})"
    la = a.decode("utf-8", "replace").splitlines()
    lb = b.decode("utf-8", "replace").splitlines()
    if la == lb:
        return " (line ends or encoding)"
    # the first hunks, so a line added above does not read as every line after it differing
    ops = [op for op in difflib.SequenceMatcher(None, la, lb, autojunk=False).get_opcodes() if op[0] != "equal"]
    parts = []
    for tag, i1, i2, j1, j2 in ops[:3]:
        parts.append(f"line {i1 + 1} {tag}: {short(la[i1:i2][:3])} | {short(lb[j1:j2][:3])}")
    more = f" (+{len(ops) - 3} more hunks)" if len(ops) > 3 else ""
    return " " + "; ".join(parts) + more


def compare_files(rec_dir: str, rec: dict, side: Side) -> tuple[list[str], set[str], list[str]]:
    """The differences, the files equal in their rule's form, and every file the oracle run wrote or
    changed (its bytes are in the record) that was compared."""
    out: list[str] = []
    hits: set[str] = set()
    compared: list[str] = []
    mine = side.manifest()
    theirs = rec["files"]
    for rel in sorted(set(theirs) | set(mine)):
        a, b = theirs.get(rel), mine.get(rel)
        rule = file_rule(rel)
        if rule and rule[0] == "skip":
            hits.add(rel)
            continue
        if a is None:
            if rule and rule[0] == "rust_only":
                hits.add(rel)
                continue
            out.append(f"{rel}: <absent> | {b[0]} bytes")
            continue
        if b is None:
            out.append(f"{rel}: {a[0]} bytes | <absent>")
            continue
        if file_rule(rel) or os.path.isfile(os.path.join(rec_dir, "blobs", a[1])):
            compared.append(rel)
        if a[1] == b[1]:
            continue
        blob = os.path.join(rec_dir, "blobs", a[1])
        a_data = open(blob, "rb").read() if os.path.isfile(blob) else None
        if rule and rule[0] in CMP:
            if a_data is None:
                out.append(f"{rel}: the oracle kept no bytes to compare it by its rule")
                continue
            found = CMP[rule[0]](a_data, open(b[2], "rb").read())
            if found:
                out.append(f"{rel}: by its rule: " + "; ".join(found[:6]))
            else:
                hits.add(rel)
            continue
        b_data = side.read_norm(b[2])
        if rule:
            # the record holds a ruled file in its rule's form
            b_data = file_form(rule[0], b_data or b"")
            if a_data is not None and a_data == b_data:
                hits.add(rel)
                continue
            if a_data is None:
                out.append(f"{rel}: the oracle kept no bytes to compare it in its rule's form")
                continue
        out.append(f"{rel}: {a[0]} bytes | {b[0]} bytes{text_diff(a_data, b_data)}")
    return out, hits, compared


# ------------------------------------------------------------------------------------------------
# One scenario on the Rust side, against its record

class Logger:
    def __init__(self, path: str | None, echo: bool) -> None:
        self.lock = threading.Lock()
        self.f = open(path, "w", encoding="utf-8") if path else None
        self.echo = echo

    def __call__(self, text: str) -> None:
        with self.lock:
            if self.echo:
                print(text, flush=True)
            if self.f:
                self.f.write(text + "\n")
                self.f.flush()


def run_rust(name: str, key: str, rec_dir: str, index: int, base_root: str, main_log) -> dict:
    with open(os.path.join(rec_dir, "record.json"), encoding="utf-8") as f:
        rec = json.load(f)
    log = Logger(os.path.join(RUNS, f"{name}.log"), False)
    side = Side(name, "rust", os.path.join(base_root, name, "rust"), RUST_PORT0 + index, log)
    t0 = time.monotonic()
    result = {"key": key, "rust": rust_fingerprint(), "ok": False, "steps": [], "files": [], "seconds": 0.0}
    try:
        side.prepare()
        side.start()
        log(f"{name}: Rust editor up in {time.monotonic() - t0:.1f} s")
        steps, answers, watches, unmet = run_steps(side, rec["steps"], False, log)
    except Exception as e:  # noqa: BLE001 - the report is the point
        side.close()
        result["error"] = repr(e)
        log(f"{name}: FAILED to run: {e!r}")
        main_log(f"{name}: FAILED to run: {e!r}")
        return result
    side.close()
    differed = 0
    for i, (step, a, b) in enumerate(zip(rec["steps"], rec["answers"], answers)):
        out: list[str] = []
        hits: set[str] = set()
        CURRENT_ARGS[:] = [f"{k}={v}" for k, v in step["args"].items()]
        # the step must have done what it expects on this side too, whatever the oracle answered
        miss = expectation(step, b)
        if miss:
            out.append(miss)
        if step.get("answer", "").startswith("skip"):
            hits.add("<answer: " + step["answer"][:60] + ">")
        else:
            diff_json(step["cmd"], a, b, "", out, hits)
        for rel, text in rec["watches"][i].items():
            mine = watches[i].get(rel)
            rule = file_rule("project/" + rel)
            if (text != mine and rule and rule[0] in CMP and isinstance(text, str) and isinstance(mine, str)
                    and text.startswith("sha256:") and mine.startswith("sha256:")):
                # a watched binary file with a rule: compared by it, from the bytes both sides kept
                theirs_blob = os.path.join(rec_dir, "blobs", text.split(":")[1])
                mine_bytes = side.watch_blobs.get(mine.split(":")[1])
                if os.path.isfile(theirs_blob) and mine_bytes is not None:
                    found = CMP[rule[0]](open(theirs_blob, "rb").read(), mine_bytes)
                    if not found:
                        hits.add(rel)
                        continue
                    out.append(f"file {rel}: by its rule: " + "; ".join(found[:4]))
                    continue
            if text != mine:
                out.append(f"file {rel}{text_diff(None if text is None else text.encode(), None if watches[i].get(rel) is None else watches[i][rel].encode())}"
                           if text is not None and watches[i].get(rel) is not None
                           else f"file {rel}: {'<absent>' if text is None else 'present'} | {'<absent>' if watches[i].get(rel) is None else 'present'}")
        label = f"{i + 1:3d} {step['cmd']} {short(step['args'])[:90]}"
        if out:
            differed += 1
            log(f"{label}\n" + "\n".join(f"      DIFF {d}" for d in out))
        else:
            log(f"{label}: same" + (f" (ignored: {', '.join(sorted(hits))})" if hits else ""))
        result["steps"].append({"cmd": step["cmd"], "same": not out, "ignored": sorted(hits), "diffs": out[:8]})
    with open(os.path.join(RUNS, f"{name}.rust.json"), "w", encoding="utf-8") as f:
        json.dump({"answers": answers, "watches": watches}, f, ensure_ascii=False, indent=1)
    fdiffs, fhits, compared = compare_files(rec_dir, rec, side)
    for d in fdiffs:
        log(f"    FILE DIFF {d}")
    for h in sorted(fhits):
        log(f"    file {h}: same in its rule's form")
    for c in compared:
        log(f"    compared {c}")
    result["files"] = fdiffs
    result["file_rules"] = sorted(fhits)
    result["compared"] = compared
    result["ok"] = differed == 0 and not fdiffs
    result["seconds"] = round(time.monotonic() - t0, 1)
    log(f"{name}: {len(rec['steps'])} steps, {differed} differ, {len(compared)} written files compared, "
        f"{len(fdiffs)} files differ, {result['seconds']} s")
    main_log(f"{name}: {len(rec['steps'])} steps, {differed} differ, {len(compared)} written files compared, "
             f"{len(fdiffs)} files differ ({result['seconds']} s) - details in Intermediate/diffrun/{name}.log")
    # a failed scenario leaves the Rust side's copy for a look, until its next run
    kept = os.path.join(RUNS, f"{name}.rust")
    rmtree(kept)
    if not result["ok"]:
        shutil.copytree(side.base, kept, symlinks=True)
        # the watched binary files as this side wrote them, by their SHA-256
        os.makedirs(os.path.join(kept, "watch_blobs"), exist_ok=True)
        for digest, data in side.watch_blobs.items():
            with open(os.path.join(kept, "watch_blobs", digest), "wb") as f:
                f.write(data)
    rmtree(side.base)
    return result


# ------------------------------------------------------------------------------------------------
# The ledger

def write_parity(state: dict) -> None:
    with open(TOOLS_JSON, encoding="utf-8") as f:
        tools = json.load(f)["tools"]
    cmds = sorted({t["cmd"] for t in tools})
    seen: dict[str, dict] = {}
    for name, res in sorted(state.get("scenarios", {}).items()):
        for st in res.get("steps", []):
            e = seen.setdefault(st["cmd"], {"scenarios": set(), "same": True, "ignored": set(), "steps": 0})
            e["scenarios"].add(name)
            e["steps"] += 1
            e["same"] = e["same"] and st["same"]
            e["ignored"].update(st["ignored"])
    lines = [PARITY_HEAD, "",
             "Written by `tools/qa/diff_editors.py --write-parity` from the last results of every scenario",
             "(`Intermediate/diffrun/state.json`); not edited by hand. `equal`: every step of the command",
             "answered the same on both editors; `equal, ignoring`: the same but for the fields named, each",
             "with its reason in the script's IGNORE table; `differs`: a step still differs; `not compared`:",
             "the run never sends it, and why.", "",
             "| Command | State | Steps | Scenarios |", "|---|---|---|---|"]
    counts = {"equal": 0, "ignoring": 0, "differs": 0, "not": 0}
    for c in cmds:
        e = seen.get(c)
        if e:
            if not e["same"]:
                state_text, k = "differs", "differs"
            elif e["ignored"]:
                state_text, k = "equal, ignoring " + ", ".join(f"`{x}`" for x in sorted(e["ignored"])), "ignoring"
            else:
                state_text, k = "equal", "equal"
            counts[k] += 1
            lines.append(f"| `{c}` | {state_text} | {e['steps']} | {', '.join(sorted(e['scenarios']))} |")
        else:
            counts["not"] += 1
            lines.append(f"| `{c}` | not compared: {why_not(c)} | 0 | - |")
    files = sorted({h for res in state.get("scenarios", {}).values() for h in res.get("file_rules", [])})
    lines += ["", f"{len(cmds)} commands: {counts['equal']} equal, {counts['ignoring']} equal with fields ignored, "
              f"{counts['differs']} differ, {counts['not']} not compared.", ""]
    if files:
        lines.append("Files compared in another form (FILE_RULES): " + ", ".join(f"`{x}`" for x in files) + ".")
        lines.append("")
    lines.append(PARITY_END)
    text = open(PARITY, encoding="utf-8").read()
    block = "\n".join(lines) + "\n"
    if PARITY_HEAD in text:
        start = text.index(PARITY_HEAD)
        end = text.index(PARITY_END, start) + len(PARITY_END)
        if end < len(text) and text[end] == "\n":
            end += 1
        text = text[:start] + block + text[end:]
    else:
        text = text.rstrip("\n") + "\n\n" + block
    with open(PARITY, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)


def why_not(cmd: str) -> str:
    if cmd in NOT_COMPARED:
        return NOT_COMPARED[cmd]
    if cmd in NETWORK:
        return NETWORK[cmd]
    if cmd in WINDOW or cmd.startswith(WINDOW_PREFIXES):
        return "window"
    return "no scenario sends it"


# ------------------------------------------------------------------------------------------------

def load_state() -> dict:
    try:
        with open(STATE, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {"scenarios": {}}


def save_state(state: dict) -> None:
    os.makedirs(RUNS, exist_ok=True)
    tmp = STATE + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(state, f, ensure_ascii=False, indent=1)
    os.replace(tmp, STATE)


def our_processes() -> list[str]:
    try:
        out = subprocess.run(["tasklist", "/FO", "CSV", "/NH"], capture_output=True, text=True, timeout=20).stdout
    except (OSError, subprocess.SubprocessError):
        return []
    return [line for line in out.splitlines() if line.startswith(('"Grizzly.exe"', '"LevelEditor.exe"'))]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--all", action="store_true", help="every scenario")
    ap.add_argument("--scenario", action="append", help="one scenario (repeatable)")
    ap.add_argument("--refresh-oracle", action="store_true", help="record the C++ side again")
    ap.add_argument("--write-parity", action="store_true", help="refresh the ledger's section and stop")
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) // 2))
    args = ap.parse_args()

    state = load_state()
    if args.write_parity:
        write_parity(state)
        print("docs/rust/PARITY.md: the differential run's section written")
        return 0

    t_start = time.monotonic()
    main_log = Logger(None, True)
    back = kept.recover()
    if back:
        print("put back what a run that died left aside: " + ", ".join(back))
    repo_files = snapshot_kept(sorted(set(CPP_KEPT) | set(RUST_KEPT)))
    # on disk too: a recording takes the author's project list and layout aside while the C++ editor runs
    kept.to_disk(repo_files)
    cache: dict = {}
    names = list(SCENARIOS)
    if args.scenario:
        unknown = [n for n in args.scenario if n not in SCENARIOS]
        if unknown:
            print("no such scenario: " + ", ".join(unknown) + " (have: " + ", ".join(names) + ")")
            return 2
        names = args.scenario
    keys = {n: scenario_key(n, cache) for n in names}
    fp = rust_fingerprint()
    if not (args.all or args.scenario):
        prev = state.get("scenarios", {})
        names = [n for n in names if not (prev.get(n, {}).get("ok") and prev[n].get("key") == keys[n]
                                          and prev[n].get("rust") == fp)]
    main_log(f"keys and fingerprints in {time.monotonic() - t_start:.1f} s")
    if not names:
        main_log("nothing changed: every scenario passed last time with this editor and these sources")
        return 0
    if not os.path.isfile(RUST_EXE):
        main_log("build the Rust editor first: bash tools/rust/cg.sh build -p grizzly")
        return 2

    os.makedirs(RUNS, exist_ok=True)
    # what a run that was stopped left in the temp folder
    for left in os.listdir(tempfile.gettempdir()):
        if left.startswith("grizzly_diff_"):
            rmtree(os.path.join(tempfile.gettempdir(), left))
    base_root = tempfile.mkdtemp(prefix="grizzly_diff_")
    need = [n for n in names if args.refresh_oracle or not load_record(n, keys[n])]
    cpp_busy = bool(need) and port_busy(CPP_PORT)
    if cpp_busy:
        main_log(f"something already answers on 127.0.0.1:{CPP_PORT} - the author's own editor; left alone. "
                 f"Without a record, not run: {', '.join(need)}")
        names = [n for n in names if n not in need]
        need = []
    if need and not os.path.isfile(CPP_EXE):
        main_log(f"no record and no oracle binary to make one ({CPP_EXE} is not there) - not run: {', '.join(need)}")
        names = [n for n in names if n not in need]
        need = []
    events = {n: threading.Event() for n in names}
    failed_oracle: dict[str, str] = {}
    for n in names:
        if n not in need:
            events[n].set()

    def recorder() -> None:
        for n in need:
            try:
                main_log(f"{n}: recording the C++ side")
                record_oracle(n, keys[n], base_root, main_log, cache)
            except Exception as e:  # noqa: BLE001
                failed_oracle[n] = repr(e)
                main_log(f"{n}: the oracle could not be recorded: {e!r}")
            events[n].set()

    rec_thread = threading.Thread(target=recorder, daemon=True)
    rec_thread.start()

    def one(i: int, n: str) -> tuple[str, dict]:
        events[n].wait()
        if n in failed_oracle:
            return n, {"key": keys[n], "rust": fp, "ok": False, "error": "no oracle: " + failed_oracle[n],
                       "steps": [], "files": []}
        rec_dir = os.path.join(ORACLE, f"{n}-{keys[n]}")
        return n, run_rust(n, keys[n], rec_dir, i, base_root, main_log)

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        futures = [pool.submit(one, i, n) for i, n in enumerate(names)]
        for fut in concurrent.futures.as_completed(futures):
            n, res = fut.result()
            state.setdefault("scenarios", {})[n] = res
            save_state(state)
    rec_thread.join()
    rmtree(base_root)
    # the C++ recorder put its own files back; nothing of the Rust editor's may have reached them
    changed = check_kept("repository", repo_files)
    if changed:
        main_log("! the repository's own files changed: " + ", ".join(changed) + " - put back")
        restore_kept(repo_files)
        for n in names:
            state["scenarios"][n]["ok"] = False
            state["scenarios"][n].setdefault("files", []).append("the repository's own files changed: " + ", ".join(changed))
        save_state(state)
    kept.done()

    bad = [n for n in names if not state["scenarios"][n].get("ok")]
    steps = sum(len(state["scenarios"][n].get("steps", [])) for n in names)
    differ = sum(sum(1 for s in state["scenarios"][n].get("steps", []) if not s["same"]) for n in names)
    files = sum(len(state["scenarios"][n].get("files", [])) for n in names)
    written = sum(len(state["scenarios"][n].get("compared", [])) for n in names)
    left = our_processes()
    if left:
        main_log("! editors still running: " + "; ".join(left))
    main_log(f"SUMMARY: {len(names)} scenarios, {steps} steps, {differ} differ, {written} written files compared, "
             f"{files} files differ; "
             f"{len(bad)} failed{(' (' + ', '.join(bad) + ')') if bad else ''}; {time.monotonic() - t_start:.1f} s")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
