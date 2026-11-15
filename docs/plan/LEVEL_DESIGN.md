# Level design an agent can do: the working plan

A working document of one task. It leaves the tree when the task is closed; what stays is in the contracts
(`docs/ASSETS.md`, `docs/COMMANDS.md`, `docs/ARCHITECTURE.md`, `docs/assets/*.md`, `docs/LEVEL_DESIGN.md`,
`docs/STATE.md`, `CLAUDE.md`, and the game's `XMS_ARCHITECTURE.md` / `MODDING.md`).

## 0. The goal

An agent (and a person) builds a place in a level - models, changed ground, spawns, lights - by editing text
files, and at every step can ask the editor what the 3D scene is like and whether what it wrote is sound.
Four parts:

1. **Sense** - commands that answer the scene's geometry in numbers and in pictures (section 2.1, 2.2).
2. **Sound files** - the scene's files never lose work to a typo, every problem has a line (2.3).
3. **Media assets** - a model, a texture and a sound are text files (wrappers) over a source file of a
   common format; the editor reads them everywhere, the module ships the description and the game reads it (2.4, 2.6).
4. **Terrain** - changed ground is a text object of the scene; on a level of the game it replaces the ground
   under it: the picture, the collision and the AI map's heights (2.5, 2.6).

Principles that decide every doubt:

- **The editor describes, the game does.** What has a meaning is text the module ships and the game reads;
  the build bakes only data that is binary by nature (pixels, geometry, samples, a lit level) into the game's
  own long-lived containers (`.dds`, `.ogf`, `.omf`, `.ogg`, the level's files). The game names no editor.
- **Versions.** Every text kind has a header key with its version; a reader (the editor's and the game's) reads
  every version up to its own through a chain of small steps and refuses a newer one by name. Every binary
  file of ours starts with a magic and a version. Nothing shipped yet, so the versions stay 1 - but the chain
  must be wired and tested for every kind.
- **The original game stays whole**: old mods (XDB, loose gamedata, JSGME), saves (`.scop`/`.scoc`/`.scov`,
  `level.ai` vertex ids), every renderer. A file an older game does not know must be ignored by it.
- **Nothing derived is stored**, names instead of positions, one thing per line, missing is fine, unknown keys
  kept (ASSETS.md section 2).

## 1. Rules for every worker (read all of them)

1. Work in `D:\Games\XFined-Editor` (the game engine packets: `D:\Games\Dead Air\DeadAir-x64`), on the tree as
   it is. **No git command that changes anything** (no commit, add, stash, checkout, reset, restore, clean):
   the architect commits. Nothing is pushed, published, released, submitted or scanned anywhere.
2. **Foreign uncommitted edits** live in `crates/gz-app/src/settings.rs`, `crates/gz-render/src/{helpers,packet,
   view}.rs`, `crates/gz-ui-app/src/{prefs.rs,ru.lua}`, `crates/gz-ui-viewport/src/service.rs`,
   `docs/UI_DESIGN.md` (a grid that follows the camera). Never revert, reformat or rewrite them. If your
   packet must edit one of these files, add your own lines only and say so in the report.
3. You own the files your packet lists. Do not edit a file another packet owns; when you need something
   from it, say what in the report (or add a `pub` item in your own file).
4. Read `CLAUDE.md` of the repository first; it is binding. In short: every cargo command is `bash
   tools/rust/cg.sh <cargo args>`; every program a script starts runs under `python tools/rust/limited.py
   <seconds> <program> ...`; `unsafe` only in `gz-win`; no `static` state; the UI thread never waits; a
   command is a card plus a typed struct with `deny_unknown_fields` and a doc comment on every field;
   parameter names from `docs/COMMANDS.md` section 2; English only in `.rs` files; comments only where the
   "why" is not obvious; no test writes a program file (`.exe`, `.dll`, `.bat`, `.cmd`, `.ps1`, `MZ` bytes).
5. Build and test only what you changed: `bash tools/rust/cg.sh test -p <crate>` and `bash tools/rust/cg.sh
   clippy -p <crate> --all-targets -- -D warnings`. Do not run `check.sh`, `test.sh --all`, `run_all.py` or
   `Build.ps1`: the architect runs them once at the end. A QA suite only when your packet names it, one phase
   at a time. Another agent may hold cargo's lock: waiting for it is normal.
6. A QA script opens only a copy of a project under the system temp folder, on its own port (28917 and up)
   and home, through `tools/qa/qa.py`, and closes its editor with `app.quit`. The author's editor (port 28016),
   the author's projects (`D:\XFinedProjects\*`: a source to copy from, never opened, no git inside) and the
   installed game's files are never touched by an editor packet.
7. A text of the window is a `Tx` with its Russian in the crate's `src/ru.lua`; Russian appears nowhere else.
8. Tests come with the code: a unit test for every rule you implement, with a negative case. A fixture is
   text written by the test (Lua, glTF JSON, OBJ) - never a binary checked in, never a program file.
9. A contract changes in its document first. This plan states the contracts of the task; the documents each
   packet names are updated by that packet in the same change.
10. Report at the end, at most 400 words: the files you changed or made; what you ran and its result (the
    exact commands); what the plan asked that you did not do, and why; anything you found broken outside
    your packet. Do not paste code.

## 2. The contracts

### 2.1 Sense: what the scene answers

All positions are metres, rotations degrees, `[x, y, z]`. Every command below is `read` unless it says so and
runs without a window (`Grizzly-Cmd.exe run ...`).

**`scene.query`** - one more set of `action`s beside `walls` (unchanged):

| action | parameters | answer |
|---|---|---|
| `ground` | `position` or `positions` (a list); `top` (start above everything instead of at the point); `reach` | one point: `floor`, `ceiling` (as today) plus `normal`, `slope` (degrees), `object`, `surface {name, texture, material}`; a list: `points: [{position, floor, ceiling, normal, slope, object, surface}]` |
| `heights` | `area [x0, z0, x1, z1]`, `step` (1 m; at most 257 x 257 samples), `only` / `except` (object names) | `area`, `step`, `columns`, `rows`, `heights` (rows from z0 up, each from x0 to x1, `null` where nothing is), `min`, `max` |
| `ray` | `from`, `to` (or `direction` + `reach`), `all` | `hit {point, normal, distance, object, kind, surface}` or `null`; with `all`: `hits` |
| `box` | `min` + `max`, or `position` + `size` (+ `rotation`); `exact` | `objects: [{name, kind, ref, bounds {min, max}}]`, `total` - the objects whose world box (with `exact`: whose triangles) meet the box |
| `fit` | `ref` (a model) or `size`, or `object` (a scene object: its model, itself ignored); `position`, `rotation`, `scale`; `except` | `fits`, `blocked_by: [{object, depth}]`, `ground {min, max, centre}`, `gap` (its bottom above the ground under its centre), `rest` (the position at which it stands on the ground), `tilt` (degrees of the ground under it) |
| `free` | `ref` or `size`; `position` (near where), `radius` (at most 50), `count` (at most 16), `spacing`, `slope` (the steepest ground allowed) | `spots: [{position, distance}]` - places where it fits, standing on the ground, nearest first |
| `ai` | `positions`; or `from` + `to` | per point `on_map`, `node` (its position), `distance`; for a pair `reachable`, `length` |

A ray takes every object that has geometry: static and dynamic objects, terrain (2.5), and - by their
shapes - the kinds without a mesh; the ground a cutting terrain replaces is not there (2.5). A hit names the
object, and for a mesh the surface with its texture and game material.

**`scene.objects`** gains `bounds` (bool: each entry carries `bounds {min, max}` and `size`), `box` (`[minx,
miny, minz, maxx, maxy, maxz]`) or `near` + `radius` (only the objects there), `sort` (`name`, `distance`).
**`scene.get`** carries `bounds` and `size` for every object that has a box.

**`scene.align`** (write) keeps `drop` and `surface`, gains `offset` (metres above where it lands) and answers
`objects` (each brief with its new position) beside `requested` and `landed`.

**`scene.audit`** - the scene's spatial lint: parameters `objects` (left out: every object), `area`, `checks`
(codes or their words), `limit`; answer `problems` (each a Problem with `object`, positioned in the scene's
file at the object's line), `total`, `checked`. The codes are in 2.7.

**`model.info`** - `refs` (library references or `.meshasset` paths): per model `ref`, `source` (`project`,
`library`, `asset`), `file`, `kind` (`static`, `dynamic`), `bounds {min, max}` (model space), `size`,
`vertices`, `triangles`, `surfaces [{name, texture, shader, compile, material, two_sided}]`, `bones`,
`animations`. **`model.list`** - `query`, `folder`, `source`, `sizes` (bool), `limit`, `offset`: `models
[{ref, source, size?}]`, `total`.

### 2.2 Pictures

- **`shot.map`** - a top-down schematic made on the processor (no window, no graphics card): parameters
  `area [x0, z0, x1, z1]` or `position` + `size`; `px` (the longer side, 1024, at most 2048); `mode`
  (`objects`: every object in a colour of its own over the shaded ground; `height`: the heights as colours
  with contour lines; `ai`: the AI map's nodes over the ground); `grid` (metres between lines; left out: 1, 5,
  10 or 50 by the scale); `labels` (object names, true); `highlight` (objects drawn in the accent colour);
  `marks` (`[{position, text}]`: points with captions, to see a place before anything is put there); `only` /
  `except`. Answer: `png_base64`, `width`, `height`, `area`, `px_per_m`, `legend [{object, colour}]`. +X is
  right, +Z is up; the grid lines carry their coordinates at the picture's edge, so a pixel reads back to
  metres. A picture is always the same for the same scene.
- **`shot.viewport`** gains a camera of its own for one picture, the author's camera left where it is:
  `position` + `target` (or `rotation`), `fov`, or `view` (`top`, `front`, `back`, `left`, `right`, `iso`) with
  `objects` or `area` to frame; `width`, `height` (at most 2048); `clean` (no grid, no helpers, no selection);
  `labels` (object names drawn into the picture). Without parameters it answers what it answers today.
- **`shot.models`** - a sheet of library models and model assets: `refs` (at most 16), `px`: each model in a
  three-quarter view with its reference and its size in metres under it.

`gz-pix` is the one small crate that draws into a picture on the processor: an RGBA canvas, lines,
rectangles, filled polygons, a built-in 5 x 7 ASCII font (scaled by whole factors), PNG out.

### 2.3 The scene's files: nothing lost, everything positioned

- A part file of the scene (`objects.lua`, `spawns.lua`, ...) that does not parse **changes nothing**: the
  scene keeps the objects it had from that file, the problem is a `syntax` error with the file, line and
  column, and the scene comes back by itself when the file is fixed. The same for the `.scene` (its problem
  carries the line and column too). Today a broken part file drops its objects on a reload: that is the bug.
- Every problem of the scene's reader carries file, line and column: `S013` (a name used twice), `S020` (a
  group names a missing object), `S021`, `S030` included.
- A group written with a `ref` and without `members` is **placed when the scene is read**: its members are
  made from the group asset at the group's `pos` / `rot` / `scale`, named `<group>_<member>`, and the scene is
  unsaved until the next save writes them (a note says so). So a prefab is one line of `groups.lua`.
- `authored` and `removed` are set with `scene.set` (`set={"authored": true}`) like any other key.
- A library object's file that changes on the disk is read again: the library hears the project's watcher.
- **Versions**: the scene reads its version through the same chain every kind uses (`KindOps.migrations`,
  applied by `format::read` as well as by `gz-asset`), and a test migrates a version-0 fixture through a test
  step. The group asset the same.

### 2.4 Media assets: a model, a texture, a sound

A media asset is a text file (Lua data, ASSETS.md) beside a **source** file of a common format. The asset is
what everything refers to and what the content browser shows; the source is its companion. Three kinds:

| Kind | File | Header | Source | The game gets |
|---|---|---|---|---|
| Model | `*.meshasset` | `mesh` | `.glb`, `.gltf`, `.obj`, `.object`, or `parts` written in the asset | static: nothing (it is compiled into the level); dynamic: `meshes\<name>.ogf` (+ `.omf`) and the asset |
| Texture | `*.texasset` | `tex` | `.png`, `.jpg`, `.tga`, `.bmp`, `.dds` | `textures\<name>.dds` (+ the bump pair) and the asset |
| Sound | `*.sndasset` | `snd` | `.wav`, `.ogg` | `sounds\<name>.ogg` and the asset |

Rules of all three:

- `source` names the file, relative to the asset's folder; left out, it is the file with the asset's own name
  and one of the kind's extensions.
- `name` is the name the game knows the thing by (`bunker\wall_brick`: a texture name, a model name, a sound
  name - the game's own namespaces). Left out, it is `<module id>\<the asset's file name>`. Two assets of a kind
  that publish one name are an error on both.
- An asset may lie anywhere in the project. The module build **publishes** it: the baked data and a copy of
  the asset's text go where the game looks (`gamedata\textures\<name>.dds` and `.texasset`, ...); the asset
  where it lies and its source are sources and never ship. What is baked is cached in the project's `temp\`.
- The game reads the description and acts on it (2.6); the editor reads the same file for its own picture,
  its level compiler and its checks. A key the game reads means in the editor what it means in the game.
- The keys are declared in tables (`Field`-like: key, type, default, help), and the reference
  (`docs/assets/MODEL.md`, `TEXTURE.md`, `SOUND.md`, `asset.reference kind=`) is generated from them; a test
  fails on a hand edit, as for the scene.

**Model** (`mesh = 1`):

```lua
return {
  mesh = 1,
  source = "wall_long.glb",
  name = "bunker\\wall_long",       -- for a dynamic model; a static one needs none
  kind = "static",                   -- static: level geometry (default); dynamic: an object the game spawns
  scale = 1, rotate = { 0, 0, 0 },   -- applied to the source as it is read
  surfaces = {                       -- by the source's material names; a key left out comes from the source
    { name = "brick", texture = "bunker\\wall_brick", shader = "default", compile = "default",
      material = "materials\\bricks", two_sided = false },
  },
  parts = {                          -- geometry written here instead of a source: a blockout an agent types
    { shape = "box", size = { 4, 3, 0.3 }, pos = { 0, 1.5, 0 }, rot = { 0, 0, 0 }, surface = "brick", tile = 2 },
    { shape = "ramp", size = { 2, 1, 4 }, pos = { 3, 0, 0 }, surface = "concrete" },
    { shape = "cylinder", radius = 0.2, height = 3, sides = 12, pos = { 0, 0, 2 }, surface = "metal" },
    { shape = "plane", size = { 10, 10 }, pos = { 0, 0, 0 }, surface = "concrete", tile = 4 },
    { shape = "prism", outline = { { 0, 0 }, { 6, 0 }, { 6, 4 }, { 0, 4 } }, height = 3, pos = { 0, 0, 0 },
      surface = "brick" },
  },
  bones = { { name = "root", shape = "box", mass = 10, material = "default_object" } },   -- dynamic
  animations = { { name = "idle", clip = "Idle", speed = 1, loop = true } },              -- dynamic
  userdata = [[ ... ]],                                                                    -- dynamic
}
```

A scene object refers to a model asset by its project path: `ref = "models/bunker/wall_long.meshasset"`. A
reference that ends in `.meshasset` is a project path; anything else is a library reference as before
(`props\box_wood_01`). A source read from glTF or OBJ becomes the same in-memory object a library `.object`
is (`gz_fmt_object::Object`): everything downstream - the picture, the rays, the level compiler, the OGF
export - is one code path. glTF is right-handed, the game left-handed: Z is negated and the winding flipped;
hard edges of the source's normals are kept; skins become bones and weights (four heaviest), animation clips
become motions sampled at 30 a second.

**Texture** (`tex = 1`): `source`, `name`, and what `.thm` says today as named keys - `use` (`surface`, `ui`,
`cube`, `normal`, `terrain`), `format` (`auto`, `dxt1`, `dxt5`, `rgba`), `mips`, `bump = { source, from =
"normal" | "height", depth, parallax }`, `detail = { texture, scale, on = "diffuse" | "bump" | "both" }`,
`material` (`oren_nayar`, `blinn`, `phong`, `metal`) with `weight`, `implicit_light`, the fade and border keys.
The table is the whole of `TextureParams`: nothing a `.thm` can say is unsayable.

**Sound** (`snd = 1`): `source`, `name`, `min_distance`, `max_distance`, `volume`, `ai_distance`, `type` (the
game's sound types as words), `quality`.

The X-Ray SDK library (`rawdata\objects\*.object`, `.thm`, `.wav`) stays what it is and is read as before;
an asset may wrap an `.object` to restate its surfaces. `asset.import from=<files> to=<folder>` (write;
`overwrite` is the destroy door) brings files in: it copies each source (a `.gltf` with its `.bin` and
pictures, an `.obj` with its `.mtl`) and writes the asset beside it - and a texture asset for every picture a
model names.

### 2.5 Terrain

A terrain is an object of the scene (`terrain.lua`, the list `terrains`): a height grid over a rectangle.

```lua
return {
terrains = {
{ name = "yard", pos = { 120, 0, -40 }, rot = { 0, 30, 0 }, size = { 40, 40 }, cell = 1,
  base = "ground",         -- "ground": the level's ground under it; a number: flat at that height
  edge = 4,                -- with base = "ground": metres along the border over which it meets the ground around
  edits = {                -- in order; x and z are metres from the centre, along the terrain's own axes
    { op = "flatten", at = { 0, 0 }, radius = 10, falloff = 4, height = 12.5 },
    { op = "rect", min = { -8, -6 }, max = { 8, 6 }, falloff = 3, height = 12.5 },
    { op = "raise", at = { 12, 4 }, radius = 6, falloff = 6, amount = 2 },
    { op = "ramp", from = { -10, 0 }, to = { 10, 0 }, width = 4, falloff = 2, heights = { 11, 13 } },
    { op = "smooth", at = { 0, 0 }, radius = 12, strength = 0.5 },
    { op = "noise", amount = 0.3, scale = 8, seed = 1 },
  },
  rows = { { 12.1, 12.2, ... }, ... },   -- instead of edits: every height written out, rows from -z to +z
  surface = "ground",      -- "ground": the look of what is under it; or { texture, shader, compile, material, tile }
  cut = true,              -- on a level of the game: the ground under it is cut out of the level
  authored = true },
},
}
```

- `pos` is the centre (its `y` is the height `base` counts from when `base` is a number); `rot` uses the
  heading only; `size` is metres along its x and z; `cell` (0.5 to 4 m) is the grid step. `height` values are
  world heights, `amount` is relative. An edit's `falloff` is the width of its soft border.
- With `base = "ground"` every grid point starts at the height of the level's ground under it (every static
  object that is not `authored`), and within `edge` of the border the result is blended back to it, so the
  border meets the ground around. A skirt 0.3 m wide goes down 0.2 m under the border.
- `surface = "ground"`: every face takes the surface (texture, shaders, game material) and the texture
  coordinates of the ground under it; a surface the level lights into its own texture (terrain) keeps the
  level's light texture, so it needs no bake and has no seam. A table is a surface of its own, laid flat
  every `tile` metres and lit like any object.
- The terrain's mesh is made from the text (`gz-terrain`: pure, deterministic, tested) and is, for the rest
  of the editor, a library object that exists in memory (`@terrain/<name>`): drawn, hit by rays, compiled.
- **The cut.** With `cut` on, the ground the terrain replaces is gone wherever the editor looks: the
  picture, every ray (`scene.query`, `scene.align`, the AI map's generation, the audit) and the piece's
  shadow casters. One rule, the game's too (2.6): a triangle of a static object that is not `authored` is
  clipped by the terrain's rectangle; the part inside is removed when every vertex of that part lies within
  0.5 m of the old ground's height there; the rest stays. A wall standing on the ground stays; the ground
  under it goes.
- The build: on a level of the module's own a terrain is level geometry like a static object. On a level
  of the game it goes into the module's piece (visuals), `overlay.xcform` (collision), and the build writes
  `overlay.cut` (the regions) and `overlay.aiedit` (the AI map's nodes under it: new heights, closed where
  the new ground is too steep or something `authored` stands). `piece.ltx` of a piece with a cut says
  `stream = false`.
- `scene.place kind=terrain position= size= cell=` (write) places one; its keys are `scene.set`'s.

### 2.6 What the module ships and the game reads

New files; an older game ignores each of them (it never looks for the name):

**`levels\<level>\overlay.cut`** - exact cuts of the level's static geometry:

```text
"XCU1"  u32 version (1)  u32 region count
region:  u8 kind (0 everything, 1 ground)  u8[3] zero
         f32 centre x, y, z     f32 half x, y, z     f32 yaw (radians about +Y)
         kind 1:  f32 tolerance (m)  u32 columns, rows  f32[columns * rows] the old ground's heights
                  (sample i, j at local x = -half.x + i * 2 half.x / (columns - 1), z likewise; bilinear between)
```

A triangle of the level's static collision and of the level's own visuals (not a piece's, not an overlay
visual's) is clipped by the region's rectangle in XZ. The inside part is removed when - kind 0 - every vertex
of it is within the box's heights, or - kind 1 - within `tolerance` of the old ground there. The outside
parts stay as triangles, every vertex attribute interpolated linearly. The old `[cut_N]` boxes and the XCF1
v2 list keep doing exactly what they do.

**`levels\<level>\overlay.aiedit`** - changes of the level's AI nodes in place:

```text
"XAE1"  u32 version (1)  u32 count
edit:   f32 x, z   f32 old y   f32 new y   f32 normal x, y, z   u32 flags (1: closed)
```

The node whose cell holds x, z and whose height is nearest `old y` (within 2 m; none: the edit is skipped and
counted in the log) gets the new height and plane; `closed` unlinks it from its neighbours, both ways. No
node is added or removed: the vertex ids saves keep are untouched.

**Asset descriptions** - `gamedata\textures\**\*.texasset`, `gamedata\sounds\**\*.sndasset`,
`gamedata\meshes\**\*.meshasset` of a mounted module, read as plain Lua data (no code runs):

- a texture's description says what its `.thm` would (the bump texture `<name>_bump` and its mode, the
  detail texture and its scale, the material) - the game registers it in the texture descriptions; a `.thm`
  of the same name loses to it;
- a sound's description sets its distances, volume, AI distance and type over what the `.ogg` carries;
- a model's description sets, over what the `.ogf` holds: each surface's texture and shader (by order), each
  bone's physics (shape, mass, material), the user data, each animation's speed and flags.

Every reader checks the header: a version above its own is skipped with one log line naming the file and
"made for a newer game"; an older one goes through the reader's steps. The character and PDA readers get the
same rule (today one checks nothing and the other wants exactly its version).

The game states what it can in `EngineAbilities`: `level-cuts`, `ai-edits`, `asset-descriptions` (beside
`level-pieces`). The editor's build asks: a cutting terrain on a game without `level-cuts` is refused with a
sentence; a game without `asset-descriptions` gets a warning.

Log lines the QA reads (exact): `* [cuts] <level>: <n> region(s), <c> collision triangle(s) cut, <v>
visual(s) cut`, `* [ai edits] <level>: <n> node(s) moved, <c> closed, <s> skipped`, `* [assets] <t>
texture(s), <s> sound(s), <m> model(s) described by <k> module(s)`.

The catalog's content rules take `texasset`, `meshasset`, `sndasset` on the editor's and the game's side (the
live catalog follows after a game release, as it did for the screens).

### 2.7 The audit's codes

All warnings, positioned at the object's line. `S201` floating: its bottom is more than 0.25 m above
whatever is under it. `S202` buried: more than half of its box is under the ground. `S203` overlap: it and
another object overlap by more than 30 % of the smaller one's box and their triangles meet. `S204`
duplicate: the same model at the same place (1 cm, 0.1 degree). `S205` nothing under it within 1000 m.
`S206` a spawn point of a creature off the AI map (more than one node away). `S207` a spawn point inside
geometry. `S208` a way point off the AI map. `S209` a light inside geometry. `S210` on a level of the game:
an object the game's level does not have and that is not `authored` (it will not ship). `S211` a terrain
whose border stands more than 0.5 m off the ground around it.

## 3. The packets

Workers: Opus agents; the architect (this session) writes the contracts, takes B3, validates every packet
(reads the diff, runs its tests and clippy, runs the one QA phase that proves it) and commits. One critic
pass at the very end.

### Wave 1

**A1a - Scene sense** (editor; owns `crates/gz-scene/src/{cmd.rs, cmd_edit.rs, pick.rs, landing.rs,
library.rs, objects/mod.rs}`, new `crates/gz-scene/src/sense/*`)

- Read first: the files above, `objtools/geom.rs`, `service.rs` (`pick()`, `prefetch_library`), `math.rs`.
- `PickHit` carries the mesh and surface of a triangle hit (index into the library object's surfaces) so an
  answer can name `surface {name, texture, material}`; `ray_scene_objects` gets a way to take every kind with
  geometry and an `except` / `only` list (keep the old call's behaviour for its callers).
- A library object not yet in the cache must not be invisible to a query: every query prefetches what the
  objects in its reach need (`svc.prefetch_library`), then builds the pick tree.
- `Library` hears the project's watcher: a changed `.object` file is dropped from the cache and `rev` moves.
  Add `Library::provide(name, object)` / a provider hook (`Arc<dyn Fn(&str) -> Option<Arc<LibraryObject>>>`
  list, asked before the roots) - packets B1 and B2 register model assets and terrains through it. Document
  the hook in the type's doc comment.
- `scene.query`: the actions `ground` (list, `top`, normal, slope, object, surface), `heights`, `ray`, `box`,
  `fit`, `free`, `ai` exactly as 2.1; `walls` unchanged. Heavy loops run on `ed.exec.cpu`, check the caller's
  stop where there is one, and are bounded by the caps of 2.1 (a request over a cap is `invalid` with the cap).
- `scene.objects`: `bounds`, `box`, `near` + `radius`, `sort`; `scene.get`: `bounds`, `size`.
- `scene.align`: `offset`, the `objects` answer.
- Tests in `sense/tests.rs` on a scene built in the test (a ground quad, a box on it, a box floating): each
  action's answer, the caps, `only` / `except`, a hit's surface, the watcher dropping a cached object.
- Docs: nothing but the cards (the cards are the documentation; `grizzly`'s test `every_card_is_worth_reading`
  must pass: `bash tools/rust/cg.sh test -p grizzly every_card`).

**A1b - Scene files** (editor; owns `crates/gz-scene/src/format/*`, `kind.rs`, `service.rs`, `apply.rs`,
`props.rs`, `edit.rs`; `crates/gz-asset/src/*` only if the chain needs it)

- Everything of 2.3. A broken part file: `format::read` reports it and the caller (`outside_change`,
  `load_scene`, `check_scene`) keeps the objects of that file from the last good state; a first open of a
  scene with a broken part file opens the rest and names the file (error, positioned), and a save must not
  write the broken file's objects away - prove it with a test that breaks `objects.lua`, saves another
  change, fixes the file and finds every object.
- Positions for `S013`, `S020`, `S021`, `S030`, the `.scene`'s `syntax`.
- A group with `ref` and no `members` is placed at read (the code that places a group asset is
  `scene.place kind=group`'s; reuse it), with its note.
- `authored` / `removed` through `scene.set` (find why the Details row or the key is not settable, fix, test).
- The migration chain: `format::read` applies `KindOps.migrations` for the scene and the group; a test with a
  one-step chain. Fix the doc drift the survey found: `docs/ASSETS.md` section 7 says `asset.migrate` converts
  `.level` (it is `scene.convert`) and lists "bookmarks" the format does not have.
- Regenerate `docs/assets/SCENE.md` if the tables changed (the test tells how).

**A2 - Mesh sources** (editor; new crate `crates/gz-fmt-mesh`, formats layer: depends on `gz-fmt-object`,
`gz-base`, `serde`, `serde_json`, `base64` - no new third-party crate)

- `read_gltf(path) -> Result<MeshSource>` (`.gltf` with external or data-URI buffers, `.glb`), `read_obj(path)`
  (with its `.mtl`). `MeshSource { object: gz_fmt_object::Object, materials: Vec<SourceMaterial { name,
  picture: Option<PictureRef>, two_sided }>, pictures: Vec<SourcePicture { name, mime, bytes | path }>, notes }`.
- glTF: every mesh primitive of the scene's nodes with the node transforms applied, triangles only (strips
  and fans expanded, other modes a note), positions welded, one UV set (TEXCOORD_0), per-material surfaces,
  hard edges from the normals (study `gz-fmt-object/src/mesh.rs` for how smoothing is stored), all accessor
  types and sparse accessors, the handedness change of 2.4. Skins: joints to bones (parent first, bind pose
  from the inverse bind matrices), the four heaviest weights per vertex normalised, as the object's weight
  maps. Animations: every clip to a motion, keys sampled at 30 a second (study `skeleton.rs`, `motion.rs`,
  `bone.rs` and what `gz-fmt-ogf/src/export.rs` `export_skeleton` / `export_omf` want - a test must export the
  result through both and read the OGF back).
- OBJ: `v`, `vt` (v flipped), `vn` (hard edges), `f` with any polygon (fan), `usemtl`, `g` / `o` as meshes,
  `mtllib` with `map_Kd`.
- The surfaces come out valid for the rest of the editor: `shader = "default"`, `shader_xrlc = "default"`,
  `game_mtl = "materials\\default"`, the fvf with exactly one texture coordinate set (see what the OGF export
  refuses). Hostile files are errors, never panics: truncated buffers, indices out of range, cycles in nodes.
- Tests: fixtures written by the tests as text (a cube `.gltf` with a base64 buffer, a skinned two-bone strip
  with one clip, an `.obj` with a quad and an n-gon); round trips through `Object::write` / `read`; the
  static and the skeleton OGF export.
- Add the crate to the workspace `Cargo.toml`; `docs/ARCHITECTURE.md` section 8's crate list gets its line.

**A3 - The game: exact cuts and AI edits** (engine repository; C++)

- Read first: `PROJECT_RULES.md`, `docs/dead-air/XMS_ARCHITECTURE.md` sections 8.2 and 8.3 and the status
  table rows P6, `src/xrCDB/xr_area.cpp` (`xms_load_xcform_overlays`, `CObjectSpace::Load`),
  `src/Layers/xrRender_R2/r2_loader.cpp` (`LoadOverlayVisuals`, `HideOverlayGeometry`, `LoadPieces`, the
  buffers' load), `src/xrAICore/Navigation/level_graph.cpp` (`xms_collect_aimap_overlays`).
- Implement 2.6's `overlay.cut` and `overlay.aiedit`, one shared parser in `src/xrCore/XMS/` (new
  `xms_level_edits.h/.cpp`), read from every mounted module that has the file for the level, in module order.
  Collision: clip before the tree is built, in the same place the v2 boxes work; the cache key takes the
  cut file's hash. Visuals: for every level visual a region's box touches, read its vertices and indices
  (from the level's geometry file again if the loader no longer has them), clip, and give the visual buffers
  of its own with the parts that stay - a progressive visual keeps its most detailed form. Interpolate every
  vertex element by its declared type. R4, R5 and R6 share the loader: all three must work.
- `EngineAbilities` of the engine and its libraries gain `level-cuts` and `ai-edits` (find where
  `level-pieces` is stated; follow it). The three log lines of 2.6, exactly.
- Edit `XMS_ARCHITECTURE.md` (a row in the status table; the formats in 8.2 and a new 8.4) and `MODDING.md`
  (the author's view). The documents name "a module" and "the authoring tool", never the editor.
- Proof: a headless check of the clip itself (find how the repository tests engine code without the game -
  `tools/qa` - and add a case: a triangle straddling a rectangle, a triangle inside, one outside, one off the
  ground's band); build `tools\build\build_x64.ps1 -Configuration Release`; do NOT deploy and do not start
  the game - the architect deploys and runs the game QA when the editor's side exists.
- Nothing of saves, `level.ai` vertex ids, the old cut paths or the original x86 behaviour changes.

### Wave 2

**B1 - Media assets** (editor; new crate `crates/gz-media`, domain layer; owns `crates/gz-project/src/
asset_kind.rs` and `rules.rs` entries for the three kinds, the registration in `crates/grizzly/src/main.rs`)

- The three kinds of 2.4 in `gz-asset`'s list (extension, header, version 1, a migration chain that is
  empty but tested through a test step, check, reference, template for `file.create`), key tables with the
  generated references `docs/assets/MODEL.md`, `TEXTURE.md`, `SOUND.md` and their drift tests.
- The index: every media asset of the project by kind and published name, kept fresh by the project's
  watcher (`gz_project::ProjectWatch`), cached by revision - never a walk per call.
- Models: `resolve(path) -> Arc<LibraryObject>`: the source through `gz-fmt-mesh` (or `Object::read` for an
  `.object`, or the `parts`), `scale` / `rotate`, the `surfaces` laid over the source's; registered as a
  provider of `gz_scene::Library` (A1a's hook) for references ending in `.meshasset`. The `parts` generator
  (box, ramp, cylinder, plane, prism) with box-mapped texture coordinates every `tile` metres.
- Textures: `texture(name) -> pixels + TextureParams` for the editor's own consumers; a function for each
  consumer to call, and the consumers wired: the viewport's texture loader
  (`crates/gz-ui-viewport/src/loader.rs`), the content browser's thumbnails, the level build's textures
  (`gz-levelbuild`, where `BuildTexture` is filled), the `texture` chooser list. The project's asset wins,
  then the old order.
- Sounds: the description and the source for the scene's sound sources and the sound preview.
- `asset.check` rules: a missing source, an unreadable source (the reader's sentence), two assets with one
  name, a surface naming a texture nobody has (warning), an unknown shader or game material (warning; the
  lists are `gz-ui-level/src/lists.rs`' sources - read them through `gz-game`), a dynamic model without
  bones, a value out of its range. Coded `M1xx`, positioned.
- Commands: `asset.import` (2.4), `model.info`, `model.list` (2.1). `model.*` covers library objects and
  model assets alike.
- Baking for the build: `publish(project, sink)` - for every asset that ships: the `.dds` (and the bump
  pair) through `gz-fmt-tex`, the `.ogf` / `.omf` through `gz-fmt-ogf`, the `.ogg` through `gz-fmt-sound`,
  cached in `<project>\temp\media\` by a hash of the asset's text and its source's bytes, plus the asset's
  text; C1 calls it. A bake is deterministic: two runs give the same bytes.
- Docs: `docs/ASSETS.md` section 6 (the kinds table, and "binary by nature" rewritten to 2.4's rule),
  `docs/ARCHITECTURE.md` section 8.

**B2 - Terrain** (editor; new crate `crates/gz-terrain` (pure: the grid, the edits, the mesh, the clip of
2.5/2.6 over triangle soups, the `overlay.cut` and `overlay.aiedit` writers and readers); in `gz-scene`: a
new class with its `ObjData`, `format` table and file, `objects/terrain.rs`; owns those and the places the
survey lists for a new kind: `class.rs`, `format/mod.rs` `CLASS_FILES`, `kind.rs` `GROUP_KEYS`, `doc.rs`
`kind_text`, `objects::world_box`, `pick::ray_object`, `default_data`, `model.rs` render items)

- Starts when A1a and A1b are committed.
- Everything of 2.5 on the editor's side except the build: the object, its keys (table, Details rows,
  labels, Russian words), `scene.place kind=terrain`, the mesh as a provided library object, the cut ground
  as provided objects replacing the references of the objects it cuts (for the picture and for every ray),
  `surface = "ground"` sampling, the AI map's generation seeing it.
- `gz-terrain` is deterministic and has a test for every edit, the edge blend, the skirt, the clip rule
  (the same four cases A3 tests, with the same numbers: copy them from A3's test into yours).
- `docs/assets/SCENE.md` regenerated.

**B3 - Pictures** (the architect): `crates/gz-pix`; `shot.map` in `gz-scene/src/sense/map.rs`; the camera of
its own for `shot.viewport` (`gz-ui-viewport/src/cmd.rs`, a new `shots.rs` beside `service.rs`); `shot.models`
in `gz-ui-previews`.

### Wave 3

**C1 - The build** (editor; owns `crates/gz-build/*`, `crates/gz-levelbuild/src/{prj.rs, pipeline.rs,
fingerprint.rs, steps.rs, service.rs, play*.rs}`, `crates/gz-export-level/src/overlay.rs`,
`crates/gz-release/src/content.rs`, `crates/gz-game` abilities)

- The module build publishes media (B1's `publish`): the list of what a build writes gains them; an asset
  and its source are sources (never shipped where they lie; `left_out` does not list them as passed by);
  wherever a project's `gamedata` is staged for the game (Play's root, the release stage), the published
  media go with it.
- The level build takes terrains and model assets: `prj::build` (static geometry and occluders),
  `overlay::collision_of`, `has_authored_static`, the fingerprints (a terrain's data, a model asset's text
  and source). On a level of the game: `overlay.cut` and `overlay.aiedit` written beside `overlay.xcform`,
  removed when no terrain cuts any more, `piece.ltx` `stream = false` with a cut; `level.inspect` gains the
  parts `cuts` and `ai_edits`; `build.report` counts them. A game without `level-cuts` refuses a cutting
  terrain before anything is built; without `asset-descriptions` warns.
- The content rules take the three extensions.
- Tests: the fingerprint moves with a terrain edit and with a model asset's source; a build of a fixture
  project publishes a texture and a dynamic model; `tools/qa/test_level_compile.py` stays green (run only its
  phases your change touches).

**C2 - The game: asset descriptions** (engine repository; after A3 is committed)

- `src/xrCore/XMS/xms_characters.cpp` holds a reader of plain Lua data (`Value`): move it into a shared
  `xms_luadata.h/.cpp` and use it for the three descriptions of 2.6 in a new `xms_assets.h/.cpp`.
- Textures: `CTextureDescrMngr` (`src/Layers/xrRender/TextureDescrManager.cpp`, `LoadTHM`) learns the
  descriptions; sounds: where an `.ogg`'s comment is read; models: after a visual is loaded by name
  (the model pool), the surfaces, bones' physics, user data and animations of 2.6.
- The version rule for every reader of a module's assets, the character and PDA readers included.
- `EngineAbilities` gains `asset-descriptions`; the log line; `ModRelease.cpp`'s allow-list takes the three
  extensions; `docs/dead-air/MODDING.md` ("A module's models, textures and sounds") and `MOD_CATALOG.md`
  section 8.
- Build; do not deploy.

**C3 - The window** (editor; owns `crates/gz-ui-content/*`, `crates/gz-ui-previews/src/{model*.rs,
texture*.rs}`, `crates/gz-ui-level/src/{place.rs, lists.rs, view_cmd.rs}` and its `ru.lua`, `gz-ui`'s icons;
`docs/UI_DESIGN.md` - a file with foreign lines: add yours only)

- The content browser: the three kinds as tiles with their own icons (three new `gz_ui::Icon` glyphs on
  the 16-unit grid; `own_extension` knows them), the source a companion (moved, renamed, deleted with its
  asset), files dropped from Explorer on a folder and Add > Import... run `asset.import`; a tile of a
  model drags into the viewport and places it; `content.view` answers them.
- A double click opens a model asset in the model viewer (`preview.model path=`), which answers `bounds`
  and `surfaces` now; a texture asset in the texture preview; a sound asset plays.
- The Level tab: the Place panel lists the project's model assets with the library's objects, and Terrain
  among shapes and volumes; a terrain's Details rows come from its table; the chooser lists (`texture`,
  shaders, materials) offer the project's assets.
- `tools/qa/test_tour.py` and `test_asset_icons.py` gain the new kinds (run the phases you touch).

**C4 - The audit** (editor; owns `crates/gz-scene/src/sense/audit.rs`): `scene.audit` with the codes of
2.7, each with a test scene that trips it and one that does not; on the CPU pool, stoppable, bounded.

### Wave 4

**D1 - The guide and the suite** (editor; owns `docs/LEVEL_DESIGN.md`, `tools/mcp/AI_SETUP.md`,
`gz-project`'s `AGENTS.md` template, the `level-design` help topic, `tools/qa/test_level_design.py`,
`tools/qa/run_all.py`'s list, `docs/STATE.md`, `docs/MOD_QUICKSTART.md` section 5)

- `docs/LEVEL_DESIGN.md`: how an agent builds a place - look (map, heights, models), write (the files, the
  group as a prefab, a blockout model, a terrain), settle (`scene.align`), check (`asset.check`,
  `scene.audit`, a picture), build and play - with one worked example whose every command was run. The same
  text, shortened, is `help about=level-design`.
- `tools/qa/test_level_design.py`: on a copy, phases for every command of 2.1 and 2.2, the files' rules of
  2.3 (the broken part file above all), an import of a model written by the test as glTF text, a terrain,
  the audit's codes, a module build that publishes media, and - on a copy of a project with a level of the
  game - the piece with a terrain: `overlay.cut`, `overlay.aiedit`, `level.inspect`. It ends on `perf.stalls`.
- `tools/qa/test_play_game.py` gains the scene `l01_escape_terrain` (a terrain that cuts, a model asset
  with a texture asset) and wants the three log lines of 2.6 and no `!` line.

**Acceptance (the architect)**: build a small place on a copy of a level of the game with these tools alone
- a yard cut into a slope, a blockout building, props from the library, a spawn and a way - looking only
through `shot.map`, `shot.viewport` and the queries; fix what the tools lack. Deploy the game (backup,
SHA-256, the game closed) and run the game scene.

**The critic** (one pass, after everything): its brief lists every risk of this plan. The architect fixes
what it finds and proves each fix with a targeted test.

**Closing**: `bash tools/rust/check.sh`, `bash tools/rust/test.sh`, `python tools/qa/run_all.py` (and
`--with-game` for the game scenes), `pwsh -NoProfile -File .\Build.ps1`, the notices, `CLAUDE.md`, this plan
out of the tree.

## 4. Done means

- Every command of section 2 exists with its card, answers what the contract says, and has a test and a QA
  phase.
- A scene file with a typo never costs an object.
- A model, a texture and a sound come in as text assets, show in the editor, compile into a level, ship in a
  module, and the game reads their descriptions.
- A terrain written as text replaces the game's ground in the editor and in the game: picture, collision,
  AI heights.
- The three entry points are green, the release build is made, both repositories are committed locally and
  nothing is pushed.
