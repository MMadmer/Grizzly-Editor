# Building and shipping a mod

Short version: press **Build Mod** (or `Ctrl+B`). Everything below is why that works
and what to do when it refuses.

---

## 1. Once per project

**Mod → Edit Manifest…** — this is the only mandatory setup.

| Field | What it is |
|---|---|
| `id` | Folder name in the game (`a-z 0-9 _ . -`). The module lands in `<game>\modules\<id>`. |
| `name` | Shown to the player in the game's Mods menu. |
| `version` | One to four numbers separated by dots: `1`, `1.2`, `1.2.0`, `1.2.0.4`. The game offers an update only when the released version is higher than the installed one. |
| `author`, `description` | Optional. Shown in the Mods menu; the description may span lines. Both (and `name`) may be a string table id the module ships in its own `gamedata\configs\text\<language>\*.xml`. |
| `website` | Optional. The mod's page, opened from the Mods menu. Only `https://ap-pro.ru/...` and `https://moddb.com/...` (with or without `www.`) are accepted — the game ignores anything else, so the editor refuses to save it. |
| `GitHub repository` | Optional, `owner/repo` (a pasted repository URL is reduced to it). Written as `[update] github`; the game checks that repository's latest release for updates — see §4. |
| **Target mode** | **Required.** Who gets the module's level work: **the ordinary game** = only when no campaign mode is active (Revolution II won't see it); **a mode already in the game** = only that campaign; **a NEW mode** = the campaign this module itself adds; **every game and every mode** = no gate at all — use only for things that belong everywhere. Without a choice the build refuses. |

If you declare a **new mode**, the export also generates the new-game-screen
checkbox for it (the xml patch, the text entry and `mode_register.script`).
Those files are machinery, not content — the content browser hides them so you
can't delete one by accident, and the export regenerates them every time.

## 2. Building

**Mod → Build Mod into Game** · toolbar **Build Mod** · `Ctrl+B`

Writes straight into `<linked game>\modules\<id>`. No packing step, no copying
by hand — the game reads modules from that folder as-is.

The build **bakes the open scene by itself**: every placed object goes into
`spawn\<level>.xspawn` for the scene's level, and a model that lives in the
project is staged into `gamedata\meshes\` — the one module subtree the game
mounts into its file system. Save the scene, press Build, done. (The scene is
the source of truth: objects you deleted from it disappear from the layer too,
and an object whose model exists nowhere is refused with a warning instead of
shipping a crash.)

**What counts as "yours":** only objects you placed — tool add, drag&drop from
the content browser, paste. Everything an imported base level brought (all the
stock props, NPCs and logic of, say, Escape) is the game's, not the mod's, and
never ships; baking it would double the whole level in game. The label is
visible per object and can be flipped: **Mod → Mark Selection as Mod Content /
as Base Level**. Objects placed in scenes saved before this flag existed count
as base — re-mark them once.

The build is a **mirror, not a merge**. Files the target holds and the project
no longer has are deleted, so a stale file from an old build can never haunt
you. If the target already holds files this project didn't put there (someone
else's module sharing your id), the build stops and asks before deleting
anything.

Other outputs, when you need them:

- **Mod → Build to Another Folder…** — same module layout, arbitrary target.
  Use it to build into a second install.
- **Mod → Package Release…** — the zip and the update descriptor for other
  people, see §4.
- **Mod → Export Flat Gamedata…** — a plain `gamedata_<id>` folder for people
  running a JSGME-style setup instead of XMS.

## 3. Testing it

The game picks the module up on the next start — **and added objects reach
existing saves too**: loading a save composes the module layers the same way
a new game does, and anything this playthrough has never seen is spawned in
once (the game remembers what it already applied, so a prop the player
destroyed stays destroyed, and nothing ever doubles). Mind the mode gate:
a module built for one campaign contributes nothing in another.

What still needs a NEW game: *changing or removing* things that already exist
in the playthrough — a save carries its own copy of every object, so edits to
base objects and deletions apply to fresh games only. Level overlays
(collision, visuals, AI map) rebuild on every level load and always reflect
the current module.

Console helpers:

```
xms_enable test
```

```
xms_disable test
```

```
xms_list
```

`xms_why <file>` answers "which module gave me this file"; the log line
`* XMS: spawn layers composed: +N added ...` confirms your ops applied on the
new game. Disabled modules are listed in `<game>\modules\disabled.ltx`;
enabling and disabling never touches the module's own files.

## 4. Shipping it to other people

**Mod → Package Release…** (MCP: `xfined_mod_package`) builds the module and
writes a release into `<project>\releases\<version>\`:

| File | What it is |
|---|---|
| `<id>-<version>.zip` | The package: the complete module, every entry under `modules/<id>/`. A module too big for one GitHub asset (2 GiB) is split into `<id>-<version>.part1.zip`, `.part2.zip`, … |
| `<id>-<version>.files` | The file index: every file's SHA-256 and where its bytes sit inside the package. It is what lets the game download only the files that changed, however many versions the player skipped. |
| `<id>.update.ltx` | The release descriptor: id, version, the lowest game version the release runs on, and the name, size and SHA-256 of the index and of every package. The game reads it to decide whether an update exists. |

`releases\` is editor-only: it is never shown as content and never ships inside
the module. Packaging the same version again replaces its files (the old ones
go to the Recycle Bin).

**Manual install.** The package is an ordinary zip. Extracted into the game
folder it lands in `<game>\modules\<id>\`, which is where the game reads modules
from. Nothing is written into the game's own files, which is why uninstalling is
just deleting that folder.

**Updates through the game.** The first install is the zip above. After that the
game's Mods menu offers every later version, as long as the releases are on
GitHub. The editor publishes them itself: **Publish Release** on the toolbar,
next to **Build Mod** (also **Mod → Publish Release…**, MCP:
`xfined_mod_publish`).

Once:

1. Create a **public** repository on GitHub with at least one commit (a README
   or a license is enough). One repository per module: the game reads the latest
   release alone, so two modules in one repository would hide each other.
2. Sign in. The window shows where the account comes from: the GitHub CLI when
   `gh auth login` was done on this machine, a personal access token pasted into
   the window (fine-grained, this repository, **Contents: Read and write**), or
   `GH_TOKEN`. A pasted token is kept in the Windows Credential Manager - never
   in the project, a log or an MCP answer.

For every release:

1. Press **Publish Release**. **Repository** takes `owner/repo` or the pasted
   address the first time and stays in `mod.ltx`; **Version** has to be higher
   than the last release, otherwise players are offered nothing.
2. Tag, title and notes are GitHub's; the game reads none of them. A
   **pre-release** never reaches players - that is what it is for.
3. **Publish** packages the module and uploads it as a *draft*, checks every
   stored asset against what was packaged (name, size, SHA-256), and only then
   publishes the draft as the latest release. Players never see a release that
   is half there. The last step asks GitHub for the descriptor at the very
   address the game uses and says whether the update is already on offer.

A version that is already published is refused: a player who has it never looks
at it again, so changed files would reach nobody - raise the version. An
interrupted upload leaves its draft on GitHub, and the next attempt picks it up.

**The releases that are out.** **Releases** on the toolbar (**Mod → Manage
Releases…**, MCP: `xfined_mod_releases`, `xfined_mod_release_edit`,
`xfined_mod_release_delete`) lists what the repository holds, newest first,
drafts included, and marks the one release the game reads. **Edit…** changes the
title and the notes of the selected release and nothing else: not its files, not
its tag, not which release is the latest. **Delete…** removes the selected
release and then its git tag, so the version number is free again. The window names what goes and asks first, because GitHub keeps no copy:
the release, its files and the tag are gone for good. Two things to know before
pressing it:

- Deleting the **latest** release makes GitHub serve the newest remaining one.
  Players who have not updated yet are offered that one instead; with nothing
  left they are offered nothing.
- A player who already installed the deleted version keeps it. Publish the
  same version again and the game notices: for a release of the installed
  version it compares content, not numbers, and offers the new files as a fix
  by the author - the changed files only, like any update. A published release
  itself is never overwritten, so "delete, then publish again" is the way to
  put other files out under one number; a higher version is still the plain way
  to ship a change. Right after a deletion the editor offers the tag
  `v<version>-r2` instead of `v<version>`: under a tag that is used again GitHub
  keeps answering the download address with the deleted files for several
  minutes, and the game would see the old release until that cache lets go.

A draft that a cancelled or failed publication left behind can be deleted here
too; publishing the same version again would have reused it anyway.

Doing it by hand still works - attach **all** the files from
`releases\<version>\` under their own names, the zip byte for byte as written
(re-zipping moves the data and breaks the index), and publish the release as the
**latest**; a draft or a pre-release is invisible to the game.

There is nothing extra to do for a small update. Every release is complete; the
game compares the index with the files the player already has and downloads
only what differs, so a one-file fix costs a one-file download — from the last
version or from one ten releases back.

**Requires game** in the packaging window is the lowest Dead Air: Refined
version the release runs on. It defaults to the version of the linked game;
leave it empty when any version will do.

`website` takes only AP-PRO and ModDB links. The allow-list is enforced by the
game as well, so a link to anywhere else would simply never open.

The release format is a frozen contract between the editor and the game. Its
normative text lives in the game repository, `docs/dead-air/MOD_UPDATES.md` —
manifest metadata in section 1, the descriptor, the file index and the package
layout in section 2.

Tell players the target mode you picked — a module gated to `survival` does
nothing in a `metro` playthrough, and that looks like a broken mod if nobody
said so.

## 5. Baking scene work into the mod

Scene edits don't ship by themselves; they get baked into module files first.
**Mod → Bake Scene Layer**:

| Layer | Output | What it carries |
|---|---|---|
| **Spawn Layer** | `spawn\<level>.xspawn` | Placed objects — both raw `.ogf` props and gameplay spawn elements, the latter with their config section and their logic (`custom_data`). |
| **Collision Overlay** | `levels\<level>\overlay.xcform` | Extra collision geometry. |
| **Visual Overlay** | `levels\<level>\visuals\*.ogf` + `overlay_visuals.ltx` | Extra visible geometry. |
| **Level Cut** | cut boxes in `overlay.xcform` | The selection used as volumes that *remove* base collision and visuals — replace a chunk of a stock level without editing a single base file. |

These write into the project. The next build ships them.

## 5a. Characters — the mod's own NPCs

**Mod → Characters…** · content browser **Create → Character** ·
`xfined_characters_write`

A story needs stalkers of its own. One `.npcasset` file per character:

```lua
return {
  npc       = 1,
  id        = "courier",
  name      = { rus = "Курьер", eng = "Courier" },
  community = "stalker",
  visual    = "actors\\stalker_neutral\\stalker_neutral_1",
  icon      = "ui_npc_u_stalker_neutral_1",
  rank      = 300,
  money     = { min = 500, max = 1500 },
  supplies  = {
    { section = "wpn_ak74u", count = 1 },
    { section = "ammo_5.45x39_fmj", count = 3 },
    { section = "medkit" },
  },
}
```

The build turns every character into the five files X-Ray wants and **appends**
them to the game's lists instead of overwriting anything:

| generated | what it is |
|---|---|
| `gamedata\configs\gameplay\character_desc_<module>.xml` | the character itself |
| `gamedata\configs\gameplay\npc_profile_<module>.xml` | its profile, naming it outright so the engine cannot pick someone else |
| `gamedata\configs\text\<lang>\<module>_npc.xml` | the name and the bio |
| `gamedata\configs\xms\<module>_npc.ltx` | the spawn section and a one-man squad |
| `patch\<module>_characters.ltxp` | `+files` / `+specific_characters_files` — `+` APPENDS |

Nothing of the game and nothing of another module is touched. Delete the
`.npcasset` and the next build takes all of that back out.

**The game knows the character as `<module>_<id>`**, and that one name is three
things at once:

- a **spawn section** — it shows up in the editor's spawn roster, so the
  character can be placed in a level by hand, and `spawn.object` creates it;
- a **squad section** `<module>_<id>_squad` — what `spawn.squad` takes;
- a **character profile** — what a stalker you placed yourself carries in its
  `character_profile` property.

Fields: `name` and `bio` are a string or a table keyed by language; `community`
is one of the game's (`stalker`, `bandit`, `dolg`, `freedom`, `ecolog`,
`killer`, `army`, `monolith`, `zombied`); `visual` is meshes-relative with the
extension optional; `rank` runs 0-1000 (novice to master); `supplies` is the
starting kit; `squad = false` leaves the squad section out.

Problems are coded: `E…` stops a build (no name, no model, an id that is not
`a-z0-9_`, two characters sharing an id), `W…` does not (a model the linked
game cannot find, text cp1251 cannot carry).

**Renaming at run time** is a different thing and belongs to the quest:
`npc.set_name` gives an NPC a name when the player learns it, and needs no
profile at all.

## 6. Quests (NQ)

A quest is a node graph in its own file. Make one in the Content Browser —
**RMB → Create → Quest Graph** — or over MCP with `xfined_quest_new`. Either way
you get a working stub (`trigger.start → flow.end`) and a document tab.

Edit it whichever way suits you. The tab is an infinite canvas: nodes flow top
to bottom, RMB pans, the wheel zooms through ten fixed levels, the strips above
and below a node are its `on_enter` / `on_exit` actions, and the inspector on
the right builds its widgets from the kind catalog the linked game ships. The
file itself is plain declarative Lua (`return { … }`), so writing or patching it
as text is a first-class path — `xfined_quest_get` hands back the canonical text
plus a readable outline of the graph, `xfined_quest_write` replaces it whole,
`xfined_quest_apply` does surgical edits. Node coordinates are optional; leave
`pos` out and **Layout** (or `xfined_quest_layout`) places everything.

Validation runs as you edit and shows up in the problems bar under the canvas —
click a problem to jump to the node. Errors (`E…`) block, warnings (`W…`) do
not. `xfined_quest_check_all` runs exactly the gate the build runs.

Then press **Build** (`Ctrl+B`). If any quest in the project has an error the
build **refuses** with `quest graph errors: N (see log)` and the log lists them
one per line — same codes the game prints. A clean build copies the `.nqasset`
files straight into `<game>\modules\<id>\`, keeping the folder they sit in
inside your project. Nothing is generated: no Lua, no XML, no string tables.

Test it in the game's console:

| Command | What you get |
|---|---|
| `nq list` | every quest of every module: uid, status, token count, problems |
| `nq state <uid>` | tokens, variables, refs, tasks, errors and the full outline |
| `nq jump <uid> <node>` / `nq fire <uid> <node>` | force the quest onto a node / finish a waiting one |
| `nq reload` | re-read the assets without restarting; state survives |
| `nq debug 1` | mirror every transition into PDA news |

Format reference, kind catalog and the meaning of every problem code:
[docs/nq/NQ_FORMAT.md](nq/NQ_FORMAT.md). The game-side contract lives in the
engine repo as `docs/dead-air/NQ_RUNTIME.md`.

## 6a. NPC scenarios

**Content browser → Create → NPC Scenario** · `xfined_quest_write`

A quest tracks the player; a **scenario** drives NPCs. Same file, same editor,
`type = "scenario"`, and a quest node starts it:

```lua
{ id = "spring_the_trap", kind = "scenario.run",
  params = { asset = "bridge_ambush", wait_end = true, ref = "ambush",
             roles = { leader = { story = "esc_wolf" }, attackers = { ref = "ambush_squad" } } },
  out = { done = "after_the_fight" } }
```

The scenario declares **roles**, not NPCs, so one file serves any cast — bind
them at the call site, or give a role a `default` and bind nothing. A role holds
a group as happily as one stalker: «отряд идёт в область и закрепляется» is one
role and one node.

Steps: `npc.goto` (a waypoint path, walk or run, wait for arrival), `npc.hold`
(dig in and fight from there), `npc.anim`, `npc.follow`, `npc.attack`,
`npc.stance` (relation, combat_ignore, invulnerable), `npc.release`. Conditions:
`npc_at_place`, `npc_in_combat`, `role_alive`. Everything else — branches,
waits, variables, signals, sounds, relations, items — is the quest vocabulary,
shared.

Where an NPC goes is a **waypoint path** you place with the WayPoint tool, not a
coordinate: X-Ray walks patrol paths, and the path ships with the module.

None of this is new AI. A step writes a logic section and hands it to the game's
own schemes, exactly the way a smart terrain does, so cover, danger, wounded and
death keep behaving like the base game.

Full reference: `docs/nq/NQ_FORMAT.md` §10.

## 7. When the build refuses

| Message | Fix |
|---|---|
| `no target game mode` | Mod → Edit Manifest, pick a mode. |
| `Module id is missing or invalid` | Mod → Edit Manifest, `id` must be `a-z 0-9 _ . -`. |
| `No linked game` | Link a game — the build target is that install. |
| `target already holds a module: N file(s) there are not in this project` | Something else owns that folder. Confirm only if you're sure it's yours. |
| `quest graph errors: N (see log)` | Open the log, fix the listed `E…` codes — see [docs/nq/NQ_FORMAT.md](nq/NQ_FORMAT.md). |
| `a release needs an orderable version` | Package Release only: set `version` to 1-4 numbers separated by dots (`1.2.0`). |
| `package … is N bytes - GitHub refuses a release asset of 2 GiB and more` | Package Release only: one file is too big to ship as a GitHub asset even alone. The message names it. |

## Not the same thing

The **Compile** menu (Build, Make Game, Make Details, Make AI Map…) is the
X-Ray *level* compiler — it bakes geometry, lighting and AI maps for a level
you authored from scratch. It has nothing to do with shipping a mod. If you're
adding things to existing levels, you never need it.
