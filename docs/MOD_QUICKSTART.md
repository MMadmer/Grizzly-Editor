# Building and shipping a mod

Short version: press **Build Mod** (or `Ctrl+B`). Everything below is why that works
and what to do when it refuses.

---

## 1. Once per project

**Mod → Mod Settings** — this is the only mandatory setup. It opens as a tab of its
own (General, Mod Browser, Game Mode, Relations, Build Contents, PDA) and writes
`mod.ltx` when you press Save.

| Field | What it is |
|---|---|
| `id` | Folder name in the game (`a-z 0-9 _ . -`). The module lands in `<game>\modules\<id>`. |
| `name` | Shown to the player in the game's Mods menu. |
| `version` | One to four numbers separated by dots: `1`, `1.2`, `1.2.0`, `1.2.0.4`. The game offers an update only when the released version is higher than the installed one. |
| `author`, `description` | Optional. Shown in the Mods menu; the description may span lines. Both (and `name`) may be a string table id the module ships in its own `gamedata\configs\text\<language>\*.xml`. |
| `website` | Optional. The mod's page, opened from the Mods menu. Only `https://ap-pro.ru/...` and `https://moddb.com/...` (with or without `www.`) are accepted — the game ignores anything else, so the editor refuses to save it. |
| `GitHub Repository` | Optional, `owner/repo` (a pasted repository URL is reduced to it). Written as `[update] github`; the game checks that repository's latest release for updates — see §4. |
| **Target** (Game Mode page) | **Required.** Who gets the module's level work: **the ordinary game** = only when no campaign mode is active (Revolution II won't see it); **a mode already in the game** = only that campaign; **a NEW mode** = the campaign this module itself adds; **every game and every mode** = no gate at all — use only for things that belong everywhere. Without a choice the build refuses. |

If you declare a **new mode**, that declaration is all it takes: Dead Air: Refined
lists the mode among the module modes of the new-game screen by its title.

## Bringing in a classic mod

A mod made to be copied over `gamedata` can become a module. The mod changes for that, the game does not.

1. Create a project and copy the mod's `gamedata` folder into it.
2. Run `python tools/classic_mod/classic_to_module.py --project <project> --game <game>`, once. It removes what must not ship (the player's options, sources, files identical to the game's)
   and turns the mod's whole-file configs and string tables into a list of what it really changes, so the game's newer sections stay.
3. In Mod Settings, Game Mode, pick **Every game and mode**.
4. Build. Turn the module off in the game's Mods menu or delete its folder under `modules`, and the game is as it was.

A mod that changes the class of an existing item needs a new game while it is on.

## 2. Building

**Mod → Build Into Game** · toolbar **Build Mod** · `Ctrl+B`

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
visible per object and can be flipped: **Scene → Mark Selection as Mod Content /
as Base Level**. Objects placed in scenes saved before this flag existed count
as base — re-mark them once.

**Taking a stock object out:** a spawn object of the base level (a barrel, a box,
a lamp, a stock NPC) can be removed from the game without touching the level:
tick **Mod → Removed from the game** in its properties (MCP: `mark_removed` on the
selection). The object stays in the scene, and the build writes a remove op for it
(`[obj:base:<level>/<name>] op = remove`, gated by the module's mode), so a new game
of the mode starts without it. Level geometry is not a spawn object and cannot be
removed this way; a save already carries its own copy, so it applies to new games.

**The mod's own models:** an OGF in the project's `gamedata\meshes\` (with its DDS in
`gamedata\textures\`) is a visual like any game one - name it without `meshes\` and the
extension (`od\sign_pvz`) in Model\Visual. The viewport reads the project first, so the prop
shows as placed; the build ships both folders in the module, and the game mounts them over its
own. Fixed bones are checked against the model's bones: `all` becomes every bone, a name the
model lacks is dropped with a warning (the game would assert `wrong fixed bone` the moment the
object comes online).

**Furnishing by MCP:** `view_mode helpers=false` draws spawn objects as models only (no
restrictor shapes, cover gizmos, icons or labels) for clean screenshots; `scan_walls` is a
horizontal lidar slice of the level's static geometry (walls, columns, stock furniture) over an
area at one height; `probe_vertical` answers the ceiling and the floor over a point. Both pick
the whole level, not only what the camera sees.

**Lighting a place:** View > Game Lighting (MCP `game_lighting`, `view_mode preset=game`) lights
the viewport the way the game will at an hour and weather of the linked game - the first night of
a new game by default: the weather's ambient, sky and moon, every hanging lamp, burning campfire
and dynamic level light near the camera, the game's tonemap adapted to the frame. Place lamps by
looking at it. Two things it shows that the plain viewport hides: a lamp without Cast Shadow
lights through walls and slabs in the game too (keep its range short of the floor below), and a
stash box (`inventory_box`) has no physics at all - put an invisible collider physic object at
its place if people should not walk through it.

The build is a **mirror, not a merge**. Files the target holds and the project
no longer has are deleted, so a stale file from an old build can never haunt
you. If the target already holds files this project didn't put there (someone
else's module sharing your id), the build stops and asks before deleting
anything: the question names up to ten of those files, and Cancel is the default.
A build started from the editor reports in one result window - what was written,
what was left out, the warnings; over MCP the answer says the same.

**What ships.** The project folder is yours to work in: a git repository, design
notes, reference pictures can all sit beside the mod. The build leaves out on its
own everything version control and tools keep there - `.git`, `.gitignore`,
`.github`, `.vscode`, `Thumbs.db`, `desktop.ini`, any name starting with a dot -
at any depth, together with the editor's own folders (`releases`, `temp`,
`backup`, ...) and the sources under `rawdata`. What else stays behind is your
call: right-click a folder or a file in the Content Browser > **Exclude from
Build** (the tile greys out and says why on hover), or list paths and patterns
in **Mod → Mod Settings, Build Contents > Not Shipped**:

```
docs              a folder, with everything in it
design/story.md   one file
*.psd             that name at any depth
```

The list is kept in `mod.ltx` under `[exclude]`, so it travels with the
repository. Every build and every release says what it left out ("Not shipped:
..."). A quest or a character inside an excluded folder is no part of the build -
a half-written quest there cannot stop it. When the project is a git repository
without a `.gitignore`, the editor writes one for its own files (`project.ltx`,
`releases/`, the scratch folders) the first time it opens the project; yours is
never touched.

**File names.** Name files as you like, Cyrillic included: the editor shows,
renames and ships every name as it is. The game, though, opens module files
through the ANSI file functions of a Russian Windows (cp1251), and a character
that code page lacks - `✓`, `é`, an emoji, CJK - comes back as `?`: that file
never opens in the game. It still ships, and the build lists it under "file
names the game cannot open".

Other outputs, when you need them:

- **Mod → Build to Folder…** — same module layout, arbitrary target.
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

**Mod → Package Release…** (MCP: `grizzly_mod_package`) builds the module and
writes a release into `<project>\releases\<version>\`:

| File | What it is |
|---|---|
| `<id>-<version>.zip` | The package: the complete module, every entry under `modules/<id>/`. A bigger module is split into `<id>-<version>.part1.zip`, `.part2.zip`, … - every package stays under the 650 MB VirusTotal takes (the Mod Browser lists only packages VirusTotal knows) unless a single file alone is larger. |
| `<id>-<version>.files` | The file index: every file's SHA-256 and where its bytes sit inside the package. It is what lets the game download only the files that changed, however many versions the player skipped. |
| `<id>.update.ltx` | The release descriptor: id, version, the lowest game version the release runs on, and the name, size and SHA-256 of the index and of every package. The game reads it to decide whether an update exists. |

`releases\` is editor-only: it is never shown as content and never ships inside
the module. Packaging the same version again replaces its files (the old ones
are deleted).

**Manual install.** The package is an ordinary zip. Extracted into the game
folder it lands in `<game>\modules\<id>\`, which is where the game reads modules
from. Nothing is written into the game's own files, which is why uninstalling is
just deleting that folder.

**Updates through the game.** The first install is the zip above. After that the
game's Mods menu offers every later version, as long as the releases are on
GitHub. The editor publishes them itself: **Publish Release** on the toolbar,
next to **Build Mod** (also **Mod → Publish Release…**, MCP:
`grizzly_mod_publish`).

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

1. Set **GitHub Repository** and **Version** in **Mod Settings** (General): the
   repository once, the version for every release - it has to be higher than the
   last release, otherwise players are offered nothing. Then press **Publish
   Release**. Its Module section (Name, Repository, Version, Requires Game,
   Author Key) only shows them; the dialog reads the repository's releases as it
   opens and lists a version that is already out as a problem at its top, with
   **Open Mod Settings** and **Open Releases** to fix it, before you press
   anything.
2. Tag, title and notes are GitHub's; the game reads none of them. A
   **pre-release** never reaches players - that is what it is for.
3. **Publish** packages the module and uploads it as a *draft*, checks every
   stored asset against what was packaged (name, size, SHA-256), and only then
   publishes the draft as the latest release. Players never see a release that
   is half there. The last step asks GitHub for the descriptor at the very
   address the game uses and says whether the update is already on offer. The
   dialog closes as the job starts; a small progress window (**Stop
   Publishing**, **Hide**) follows it, and the result window says how it went.

A version that is already published is refused: a player who has it never looks
at it again, so changed files would reach nobody - raise the version. An
interrupted upload leaves its draft on GitHub, and the next attempt picks it up.

**A module in the Mod Browser.** The Mod Browser offers a release only once
VirusTotal knows its packages, and it never uploads anything itself. So for a
module the Mod Browser lists, Publish scans the packaged release on VirusTotal
with your own free key first, before anything reaches GitHub: every package is
looked up, one VirusTotal does not know is uploaded, and its report is awaited -
a few minutes. A listed module's release leaves only with a VirusTotal report of
every package: without a key, when VirusTotal blocks a package, is still analysing
it after the wait or does not answer, the release is not published and the
result says why and when to publish again. Add the key in **Mod → Submit to Mod
Browser…** (a free account at virustotal.com has one). A module the Mod Browser
does not list is published without a scan.

**The releases that are out.** **Releases** on the toolbar (**Mod → Releases**, a
tab of its own; MCP: `grizzly_mod_releases`, `grizzly_mod_release_edit`,
`grizzly_mod_release_delete`) lists what the repository holds in a sortable table -
Tag, Title, State, Published (local time), Files - drafts included, and marks the
one release the game reads; the selected release's details sit beside it.
**Edit…** changes the title and the notes of the selected release and nothing
else: not its files, not its tag, not which release is the latest. **Delete…**
(also the row's right-click menu, or Del) removes the selected release and then
its git tag, so the version number is free again. It names what goes and asks
first, Cancel the default, because GitHub keeps no copy:
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

**Requires Game** in the packaging window is the lowest Dead Air: Refined
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
| **Spawn Layer** | `spawn\<level>.xspawn` | Placed objects — both raw `.ogf` props and gameplay spawn elements, the latter with their config section and their logic (`custom_data`), plus what the section cannot hold: the model and its fixed bones, a restrictor's shapes, a smart cover's description (the animpoint an NPC takes), a hanging lamp's light (type, colour, brightness, range, cone, light bones, glow, ambient, animator) — so a lamp shines in game the way its properties say — and a remove op for every base object ticked **Mod → Removed from the game**. |
| **Collision Overlay** | `levels\<level>\overlay.xcform` | Extra collision geometry. |
| **Visual Overlay** | `levels\<level>\visuals\*.ogf` + `overlay_visuals.ltx` | Extra visible geometry. |
| **Level Cut** | cut boxes in `overlay.xcform` | The selection used as volumes that *remove* base collision and visuals — replace a chunk of a stock level without editing a single base file. |

These write into the project. The next build ships them.

## 5a. Characters — the mod's own NPCs

**Mod → Characters** (a tab) · content browser **Add → Character** ·
`grizzly_characters_write`

A story needs stalkers of its own. One `.npcasset` file per character:

```lua
return {
  npc       = 1,
  id        = "courier",
  name      = { rus = "Курьер", eng = "Courier" },
  community = "stalker",
  visual    = "actors\\stalker_neutral\\stalker_neutral_1",
  icon      = "ui_inGame2_neutral_2",
  rank      = 300,
  money     = { min = 500, max = 1500 },
  supplies  = {
    { section = "wpn_ak74u", count = 1 },
    { section = "ammo_5.45x39_fmj", count = 3 },
    { section = "medkit" },
  },
}
```

The `.npcasset` is all the module ships. The game reads it and makes the rest
itself - the character, its profile naming it outright, the name and the bio, the
spawn section, a one-man squad - and **adds** them to its lists: nothing of the game
and nothing of another module is touched. Delete the `.npcasset` and the character
is gone with the next build. The file holds plain values only: the game reads it as
data and runs nothing in it.

**The game knows the character as `<module>_<id>`**, and that one name is three
things at once:

- a **spawn section** — it shows up in the editor's spawn roster, so the
  character can be placed in a level by hand, and `spawn.object` creates it;
- a **squad section** `<module>_<id>_squad` — what `spawn.squad` takes;
- a **character profile** — what a stalker you placed yourself carries in its
  `character_profile` property.

**A quest needs none of those names.** `spawn.npc` takes the character itself:
`character = "valera"`, picked from the list of the project's characters, and the
runtime works the section out. One node holds as many NPCs as the scene needs,
every row with a `ref` of its own - that ref is how later nodes, a kill
objective or a behaviour graph point at exactly that NPC. A row may name a plain
spawn section of the game instead, for anybody who is not yours.

```lua
{ kind = "spawn.npc", params = {
    place = { smart = "esc_smart_terrain_5_7" },
    npcs  = {
      { character = "valera", ref = "valera" },
      { section = "sim_default_stalker_1", ref = "his_friend" },
    } } }
```

`unique` decides what happens to the ones already there. For a **character** it
is on unless you switch it off: every Valera in the world is removed before the
new one comes, so there is never a second one - and a ref that named the old one
is cleared, not left pointing at nobody. For a **section** it is off unless you
switch it on: a section is a kind half the zone may share, and clearing it out
is never assumed.

The Characters tab shows `name` and `bio` for BOTH languages at once, one
row each: fill `rus` and leave `eng` empty and an English install reads the
Russian text; fill both and each install reads its own. A character that only
has `rus` stays a plain string in the file.

Fields: `name` and `bio` are a string or a table keyed by language; `community`
is one of the linked game's - the tab offers them as a list, under the names
the player reads (`stalker`, `bandit`, `dolg`, `freedom`, `ecolog`, `killer`,
`army`, `monolith`, `zombied`, `csky`, `trader`, … - a global mod may change
the set); `base` is picked from the project's other characters; `visual` is
meshes-relative with the extension optional - the tab lists `meshes\actors` of
the game and the project, and **Preview** opens the model; `icon` is a portrait -
the tab shows it and opens a gallery; `rank` is a number on the linked game's
scale (Dead Air: novice from 0 to legend from 13999, the tab names the rank);
`money` is two amounts, *from* and *to* (empty while the character sets none - the
base's amount is the hint), **Never runs out** beside them and the Clear icon to
unset;
`start_dialog` is picked from the game's dialogs by their first line; `supplies`
is the starting kit, every row picked by the item's name; `squad = false` leaves
the squad section out.

Problems are coded: `E…` stops a build (no name, no model, an id that is not
`a-z0-9_`, two characters sharing an id), `W…` does not (a model the linked
game cannot find, text cp1251 cannot carry, `W114` - a community the linked
game does not list: the engine refuses a character of a community it does not
know, so this one is only safe when the module brings that community itself;
`W115` / `W116` / `W117` - a portrait, a dialog or a kit item neither the linked
game nor the project has; an item the game does not know stops the game when the
NPC spawns). `E123` is a model of the linked game on a skeleton of its own: an
NPC's damage section names bones of the stalker skeleton - the head and the face
among them - and the game stops the moment the NPC comes online if one is
missing (Clear Sky's Forester, `actors\stalker_lesnik\stalker_lesnik_1`, has no
eyes and no jaw). The Characters tab shows each problem as a sentence under its
field; `characters_write`, `characters_view` and the build keep the code.

### One kit, many characters

The tenth guard is the same twenty lines as the first, so a character can start
from another one. `base` names it; everything this character does not set comes
from there, and the **starting kits are merged**.

```lua
-- characters/kit_stalker.npcasset — a template, never a character in the game
return {
  npc      = 1,
  id       = "kit_stalker",
  abstract = true,
  supplies = {
    { section = "device_torch" },
    { section = "wpn_knife" },
    { section = "bandage", count = 2 },
    { section = "medkit", count = 2 },
    { section = "bread" },
  },
}
```

```lua
-- characters/guard.npcasset
return {
  npc       = 1,
  id        = "guard",
  base      = "kit_stalker",
  name      = "Охранник",
  community = "dolg",
  visual    = "actors\\stalker_dolg\\stalker_dolg_1",
  supplies  = {
    { section = "wpn_ak74u" },              -- added
    { section = "medkit", count = 4 },      -- the inherited 2 become 4
    { section = "bread", count = 0 },       -- the inherited bread is dropped
  },
}
```

| | |
|---|---|
| `base` | another character of this project. Chains are fine, a loop is `E112`, a base that does not exist is `E111` |
| `abstract = true` | a template only: no profile, no spawn section, no string table, and it needs neither a name nor a model |
| a repeated section | takes the new count, keeping its place in the list |
| `count = 0` | takes an inherited item out |

The **Characters** tab shows what the base already gives, greyed out above
your own rows, so you do not add the same torch twice.

**The model is fixed.** `visual` is what the NPC spawns with and keeps: in Dead
Air an outfit changes an NPC's protection, not its model — `ChangeVisual` is
applied to the player only. A character that should look different is a
different `visual` (a `base` with everything else shared is exactly the way to
write that).

### A trader

A character can keep a shop - a courier hub's cook sells food, its medic sells medicine:

```lua
return {
  npc   = 1,
  id    = "cook",
  name  = "Повар",
  -- ...
  trade = {
    sells   = { "food" },                -- categories of the game's trade presets
    buys    = { "food", "mutants" },     -- what it takes from the player
    markup  = 1.2,                       -- asks 120% of an item's cost
    pays    = 0.3,                       -- pays about a third of it
    restock = 24,                        -- game hours until the stock is back
    stock   = {
      { section = "conserva", count = 6 },
      { section = "bread", count = 4, chance = 0.8 },
    },
  },
}
```

The categories are the linked game's own (`misc\trade\trade_presets.ltx`: `weapon`, `ammo`,
`outfit`, `drugs`, `food`, `devices`, `mutants`, ...); the Characters tab offers them as ticks in
its **Trade** section, the stock as a table like the Starting Kit's. The stock is always for
sale; the categories add what the trader sells of whatever else it carries. The game makes its
trade config of this and uses it whatever the NPC is doing - standing at a post by an order, walking a behaviour
graph, placed in a level. `trade = false` takes a base's shop away from a character that starts
from it. Problems: `E118` a stock row without an item, with less than one, a chance outside 0..1
or twice; `E119` a markup that is not above zero, a price below nothing, a restock under an hour;
`W120` a category the linked game does not have; `W121` a stock item it does not have; `W122` a
trader with nothing to sell.

**Renaming at run time** is a different thing and belongs to the quest:
`npc.set_name` gives an NPC a name when the player learns it, and needs no
profile at all.

## 6. Quests (NQ)

A quest is a node graph in its own file. Make one in the Content Browser —
**Add → Quest** (the toolbar's [+ Add], or right-click the empty grid) — or over
MCP with `grizzly_quest_new`. Either way you get a working stub
(`trigger.start → flow.end`) and a tab of its own.

Edit it whichever way suits you. The tab is an infinite canvas between two
panels: nodes flow top to bottom, RMB pans, the wheel zooms through ten fixed
levels, and each node is a card listing its conditions, its key parameters and
its `on_enter` / `on_exit` actions. The **Graph** panel on the left holds the
quest's header, tasks and variables; the **Details** panel on the right edits the
selected node and action, its rows built from the kind catalog the linked game
ships. The
file itself is plain declarative Lua (`return { … }`), so writing or patching it
as text is a first-class path — `grizzly_quest_get` hands back the canonical text
plus a readable outline of the graph, `grizzly_quest_write` replaces it whole,
`grizzly_quest_apply` does surgical edits. Node coordinates are optional; leave
`pos` out and **Auto Layout** (or `grizzly_quest_layout`) places everything.

Validation runs as you edit; the toolbar's **Validate** opens the **Problems**
drawer under the canvas — click a problem to jump to the node. Errors (`E…`) block, warnings (`W…`) do
not. `grizzly_quest_check_all` runs exactly the gate the build runs.

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

### Global variables: what every quest of the module shares

A story mod is many quests - one per job, one per chain - and they have to know about each
other: which night it is, whether the cook's chain was finished, how many clues the player holds.
A quest's own `vars` are its alone; the module's **global variables** are everybody's:

```ini
; mod.ltx
[vars]
night       = 0
cook_done   = false
suspect     = "nobody"
```

In the graph tab they sit in the Variables pane under **Global** - add, rename, type and default,
saved into `mod.ltx` as you edit (MCP: `grizzly_mod_set_manifest vars=`). Any quest or behaviour
graph writes them with `global.set` / `global.add`, waits on them with the `global` condition
(`trigger.when`, `wait.when`, a branch), and shows one with `{global:suspect}` in a text. A save
keeps their values; a new game starts from the defaults. Naming one the manifest does not declare
is `E030`. Keep unrelated jobs in files of their own and let them talk through these.

### Staging a scene: put it there, take it away

A quest can create things and remove them again, which is how a scripted scene
works: the player crosses one trigger and the scene appears, crosses another and
it is gone.

| put it there | take it away |
|---|---|
| `spawn.squad` — a squad at a smart or inside a place, `ref` saves it | `squad.remove` |
| `spawn.npc` — one NPC or several at a place: a character of the module or a spawn section, a `ref` each | `npc.remove`, `object.remove` |
| `spawn.object` — anything with a spawn section, at a place, `ref` saves it | `object.remove` |
| `item.spawn` — items into a place, a stash, a corpse, a container or **an NPC's inventory** | `item.take` |
| | `npc.remove` (an NPC by story id, ref, profile or community) |

The trigger is whatever you already use: `trigger.when` with `actor_in_place`
(a point with a radius, a restrictor or a smart), `objective.reach`,
`event.level_entered`, an info portion, a variable — the same conditions the
rest of the graph uses.

```lua
{ id = "at_the_bridge", kind = "trigger.when",
  cond = { { kind = "actor_in_place", params = { place = { restrictor = "esc_bridge" } } } },
  on_enter = { { kind = "spawn.squad",
                 params = { section = "army_sim_squad_novice", place = { restrictor = "esc_bridge" },
                            ref = "patrol", hold = true } } },
  out = { next = "…" } }
```

**Removing something that is busy is safe.** An NPC being taken away is rarely
idle — it may be in a scheme a behaviour graph gave it, in a fight, or talking to the
player. Before anything is released the runtime tells whatever was driving it to
stop: a behaviour graph drops it from its role, a talk with the player is ended, and the
active scheme is switched off. Then the release itself goes through the engine's
own safe release, which defers it when the object cannot go yet. Nothing is left
pointing at an id that no longer exists.

A `ref` that named something removed is cleared, so a later node cannot resolve
a dead id.

## 6a. NPC behaviour graphs

**Content browser → Add → Behaviour Graph** · `grizzly_quest_new` with a
`.behasset` path · `grizzly_quest_write`

A quest tracks the player; a **behaviour graph** drives NPCs. It is its own
asset - a `.behasset`, its own icon, its own tab - on the node graph you
already know. It shows the player nothing (no tasks, no dialogs, no map spots)
and never starts itself: a quest runs it.

**Spawn Valera, make him walk his route, and turn him on the player at the
end** - the whole thing is one call in the quest:

```lua
{ id = "bring_valera", kind = "flow.step",
  on_enter = { { kind = "spawn.npc", params = {
                 place = { smart = "esc_smart_terrain_5_7" },
                 npcs  = { { character = "valera", ref = "valera" } } } } },
  out = { next = "his_route" } },

{ id = "his_route", kind = "behaviour.run",
  params = { asset = "valera_route", ref = "route",
             roles = { valera = { ref = "valera" } } },
  out = { started = "tell_the_player",     -- at once: the quest carries on
          finished = "valera_is_hostile",  -- the route played out
          aborted = "valera_never_made_it" } },  -- he died, or it was stopped
```

**Three exits, and the wiring is the mode.** Wire `started` alone and the quest
carries on while the NPC does his thing. Wire `finished` alone and the quest
waits for him. Wire both for both. `aborted` is the route that never played out.
There is no wait flag to remember.

The graph itself (`valera_route.behasset`) declares **roles**, not NPCs, so the
same file serves any cast - bind them at the call, or give a role a `default`
and bind nothing. A role holds a group as happily as one stalker: «отряд идёт в
область и закрепляется» is one role and one node. In the editor the roles sit in
the Graph panel where a quest keeps its tasks, a step's `who` is a tick per role, and the call's
bindings are a form: pick the asset and one row per role appears.

Steps: `npc.goto` (walk or run, wait for arrival), `npc.hold` (dig in and fight
from there), `npc.anim`, `npc.follow`, `npc.attack`, `npc.stance` (relation,
combat_ignore, invulnerable), `npc.release`. Conditions: `npc_at_place`,
`npc_in_combat`, `role_alive`. Branches, waits, variables, signals, sounds,
relations, spawns - the graph plumbing - are shared with quests; what belongs to
the player (tasks, dialogs, money, news, map spots) is not offered here at all.

**Where an NPC goes is a destination**, and it takes whatever you have:

| write | to walk to |
|---|---|
| `place = { path = "valera_gate" }` | a way path you drew with the WayPoint tool - the best one: he gets a route and looks around on it |
| `place = { ref = "dropped_case" }` | something the QUEST spawned - a behaviour graph sees the refs of the quest that runs it |
| `place = { story = "esc_wolf" }` | an object of the level |
| `place = { role = "leader" }` | wherever that role stands |
| `{ smart = }`, `{ restrictor = }`, `{ pos = }`, `{ actor = true }` | a place |

**A ref does not have to exist yet.** A step that names one not in the world
parks until it appears, follows it when it moves, and lets go - the graph moves
on - when it is removed. Spawn the case on one trigger, run the behaviour on
another, delete the case on a third: nothing breaks and nothing waits forever.

None of this is new AI. A step writes a logic section and hands it to the game's
own schemes, exactly the way a smart terrain does, so cover, danger, wounded and
death keep behaving like the base game; for a destination nobody drew a path to,
the ENGINE registers a one-point path on the fly and the same walker walks it. A
save in the middle of a step is safe - the NPC comes back into the section he
was in.

Full reference: `docs/nq/NQ_FORMAT.md` §10.

## 6b. The mode's world - an empty map of your own

**Mod → World Rules** (a tab) · `grizzly_world_get` / `grizzly_world_set` /
`grizzly_world_transitions` · Spawn tool → **Player Start** · a stalker's
**Order** properties · `grizzly_spawn_order`

A story mod on one map wants the map to itself. Say: only НИИ Агропром (and its
underground), the player starts at your spot with the kit you gave, nobody of the
game walks around, your own NPCs are placed and ordered about, and the story swaps
them as it goes. That is five settings and no code.

**1. Nothing of the original game.** Mod → World Rules, *Original Game*: untick
**Keep** where the mode does not keep it, or press **Empty World...** on the tab's
toolbar - the one-click empty world, which names the rules it turns off and asks
first. The tab saves `mod.ltx` as you edit. Five rules, each a row:

| row | `mod.ltx` key | off means |
|---|---|---|
| Original characters | `original_npcs` | traders, story characters and every squad the game placed by name are not in the world |
| Random stalkers | `random_stalkers` | the simulation's stalker squads neither start on the map nor respawn |
| Mutants | `mutants` | the same for monster squads |
| Jobs from NPCs | `npc_quests` | squad leaders offer no "got any work?" |
| Story quests | `story_quests` | named characters offer no tasks, the storylines stay shut |

*Applies On* narrows them to some levels (keep Cordon alive, empty Agroprom).
*Player Decides* puts a rule on the new-game screen as a checkbox, preset to
your choice - three at most, the screen has three rows.

**2. Where the player may go.** *Playable Area → Only These Levels*:
`l03_agroprom`, `l03u_agr_underground`. A transition leading anywhere else stays
closed - the game shows its own "the way is closed" message. *Transitions*
lists every level changer of the linked game by level: tick one to close it,
or close the whole level. A quest opens them later (`world.transition`,
`world.area`), when the story says so.

**3. Where the player starts.** Open the level's scene, Spawn tool →
**Player Start** (or **Place in Scene** in the World Rules tab), put it down and
turn it - the arrow is where the player faces. One per module. The new game
begins there, whatever faction the player picked.

**4. What the player starts with.** World Rules, *Starting Kit* → **Set a
Starting Kit**: it begins as the game's own base kit (the torch, the knife, the
binoculars) and you change it from there - items with a count, the money, and
whether the perks picked on the new-game screen stay. A new game of your mode then
begins with exactly that: the loadout and the extra kits the screen handed out are
taken away, while the PDA and the bolt always stay. `mod.ltx [start_kit]`,
`grizzly_world_set` `start_kit` / `start_money` / `start_perks`.

**5. Your NPCs.** Place stalkers of your characters with the Spawn tool. Each
one's Properties have an **Order** group: *Guard* (holds the spot, optionally
watching a way path), *Patrol* (walks a way path you drew with the WayPoint
tool), *Wander* (roams around its spawn), *Follow* (the player or a story id),
plus *Hostile to player*. That is what it does from its first frame. Your way
paths ship with the module under their own names; a name the game already uses
is refused at the bake, so nobody's path gets replaced.

**The story swaps them.** In a quest: `npc.remove` takes one out, `spawn.npc`
brings the next one in with a `ref`, and an `order.*` tells it what to do:

```lua
{ id = "shift_change", kind = "wait.timer", params = { duration = { game_hours = 2 } },
  on_exit = {
    { kind = "npc.remove", params = { npc = { ref = "guard_a" } } },
    { kind = "spawn.npc",  params = { place = { restrictor = "agr_gate" },
                                       npcs = { { character = "guard_b", ref = "guard_b" } } } },
    { kind = "order.patrol", params = { who = { ref = "guard_b" }, route = "agr_gate_walk" } },
  },
  out = { done = "night" } },
```

`order.guard`, `order.patrol` (a route, or an area to wander), `order.follow`
(the player or another NPC), `order.attack` (the player, an NPC, a squad, a
whole community), `order.clear`. An order belongs to the NPC: it survives
saves, outlives the quest, and a behaviour graph borrowing the NPC simply takes
precedence until it lets go.

**Nothing of the game is overwritten.** The editor only READS the linked
game's `all.spawn` to list levels and transitions; the rules, the start, the
paths and the orders are the module's own files, and the engine lays them over
the game in memory when a new game starts. Another module on the same map keeps
working; two modules that both place a Player Start are reported, not silently
merged. The rules take effect on a new game - a save keeps the world it began
with.

The build refuses a second Player Start (`E201`), more than three player
choices (`E202`) and a start kit row it cannot use (`E207`), and warns about a
level or a transition the linked game does not have, about an area the Player
Start is not in (`W203`-`W206`) and about a kit item the linked game lacks (`W208`).

## 6c. The PDA interface - your own app in the PDA

**Mod → PDA Interface** · Content browser → **Add → PDA Interface** ·
`grizzly_pda_*`

Your mod can put its own screens into the PDA: a courier app, a hacked military
terminal, a scanner - pages, buttons, lists, pop-up menus, and the live map
(with the player and every task marker) in a slot of your design. The device
stays the game's, and so do the tasks, contacts and the rest: a page of yours
can show any stock section.

1. **Mod → PDA Interface** makes `pda\<module>.pdaasset` from a template (home,
   map, journal), writes `[pda] layout` into `mod.ltx` and opens the **preview
   tab**: the 2D PDA with the interface in it, in the game's own textures and
   fonts. Click through it - buttons, tabs, lists and menus work as in the game;
   a quest signal or a script call shows as a note and goes into the action log.
2. Edit the file (the toolbar's **Source** pane → Apply (`Ctrl+Enter`) / Save, any
   text editor, or an AI agent with `grizzly_pda_write`): the tab follows the file
   unless the Source pane holds edits, and never drops those without asking. The format is
   `docs/pda/PDA_FORMAT.md` - a canvas of 960x680, `chrome` under every page,
   `pages`, an `overlay` on top, widgets `rect`, `image`, `text`, `button`,
   `list`, `group`, `map`, and actions like `page:map`, `show:menu`,
   `var:order=...`, `signal:order_taken`.
3. **Problems** are the first tab of the **Details** pane beside the device
   (`E3xx` stops the build, the game would not show it either; `W3xx` warns).
   **Aspect** 4:3 / 16:9 shows both screens; the canvas pans with the right or
   middle button and zooms at the mouse (`Ctrl`+wheel), `F` fits.
4. Build: the file ships as it is, and the game shows it while your module
   applies (in your mode, for a mode mod). Without `[pda]` - the stock PDA.

A quest hears the interface: `signal:order_taken` on a button is an
`event.signal` with `name = "order_taken"` in any quest, and a script changes a
live interface through `xms.pda` (`set`, `rows`, `page`, `show`, `open`, ...).

Things that bite: Dead Air's fonts draw `—` and `–` blank and have no `→` or
`★` - use `»`, `·`, `-` and the icons (`ui_xms_icon_star`, `_pin`, `_box`, ...);
`letterica25` is as small as `letterica16` in Dead Air - headers want
`graffiti22` or `graffiti32`.

## 6d. Console commands - your own commands in the game console

**Mod → Mod Settings → Commands** · `grizzly_mod_commands`

A mod can add console commands: a status line for the player, a switch for you
while you test, a cheat for your testers. A command is a name the player types and
a Lua function of your project's `gamedata\scripts` that the game calls:

```ini
; mod.ltx
[commands]
my_status      = my_mod.status

[command.my_status]
help        = The courier's night, money and debt
args        = [all]
```

```lua
-- gamedata\scripts\my_mod.script
function status(args, module, command)   -- args: what was typed after the name ("" when nothing)
	local words = xms.command_args(args)  -- 'a "b c"' -> { "a", "b c" }
	if not (db.actor) then return "No game is running." end
	return "Money: " .. db.actor:money()   -- a returned string is printed, line by line
end
```

1. **Add Command** on the Commands page: the **Name** (`a-z`, `0-9`, `_`), the
   **Function** (`<script>.<function>` - the chevron lists every function of your
   scripts), the **Help** line the console's `help` shows, the **Arguments** hint and
   **Cheat** (the command runs only when the game was started with `-dbg`).
2. Save, build, start the game: the command is in the console (`help`, Tab completes it).
   It exists in every game and in the main menu while the module is installed, so the
   function checks itself whether its world is there (`db.actor`, `xms.mode_active`).
3. A Lua error does not crash the game: the console prints it with the stack. Texts
   may be UTF-8 or cp1251.

A name the game already has, or that a module loading before yours took, is
refused by the game with a line in its log - pick names with your mod's prefix. The
page and the build check it: a bad name (`E401`), a name twice (`E402`), a function
that is not `<script>.<function>` (`E403`) or that your scripts do not define
(`E404` - the check looks for `function name(` or `name = function(`; a `local
function` is not reachable), a `[command.<name>]` without its line (`E405`) and a
command of the game (`E406`) stop the build; a name of one of the game's own families
(`g_`, `r2_`, `xms_` ...: `W406`), one another module of the linked game declares
(`W407`) and help text outside cp1251 (`W408`) are warnings.

## 7. When the build refuses

| Message | Fix |
|---|---|
| `no target game mode` | Mod → Mod Settings, Game Mode: pick a mode. |
| `Module id is missing or invalid` | Mod → Mod Settings, General: **Module ID** must be `a-z 0-9 _ . -`. |
| `No linked game` | Link a game — the build target is that install. |
| `target already holds a module: N file(s) there are not in this project` | Something else owns that folder. The question names up to ten of those files; confirm only if you're sure they are yours. |
| `quest graph errors: N (see log)` | Open the log, fix the listed `E…` codes — see [docs/nq/NQ_FORMAT.md](nq/NQ_FORMAT.md). |
| `E201 world: N Player Starts` | A module has one Player Start - delete the others (the message names them and their levels). |
| `E202 world: N rules offered to the player` | Mod → World Rules: the new-game screen has room for three *Player Decides* ticks. |
| `E207 world: a start kit row ...` | Mod → World Rules, *Starting Kit*: every row names one item with a count of at least 1, and an item has one row. |
| `E401`-`E406 commands: '<name>' ...` | Mod → Mod Settings, *Commands*: a name of `a-z 0-9 _` that the game does not have, declared once, calling a `<script>.<function>` your `gamedata\scripts` define. |
| `a release needs an orderable version` | Package Release only: set `version` to 1-4 numbers separated by dots (`1.2.0`). |
| `package … is N bytes - GitHub refuses a release asset of 2 GiB and more` | Package Release only: one file is too big to ship as a GitHub asset even alone. The message names it. |

## Not the same thing

The **Build** menu of the Level tab (Make ▸ Make All, Make Game, Make Details,
Make AI Map…; Compile ▸ Geometry & Light, AI Map, Spawn) is the X-Ray *level*
compiler — it bakes geometry, lighting and AI maps for a level you authored from
scratch. It has nothing to do with shipping a mod. If you're
adding things to existing levels, you never need it.
