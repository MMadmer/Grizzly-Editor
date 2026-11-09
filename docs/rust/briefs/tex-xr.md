# Brief: textures, sounds and the `.xr` libraries

Finish `gz-fmt-tex` and `gz-fmt-sound`, write `gz-fmt-xr`. Read `docs/rust/CONVENTIONS.md` and
`docs/rust/STATE.md` first; then `crates/gz-chunk/src/{lib,reader,writer}.rs`.

You own: `crates/gz-fmt-tex/**`, `crates/gz-fmt-sound/**`, `crates/gz-fmt-xr/**`.

## Where it stands

An earlier pass wrote most of the first two crates and was stopped. Read its code first - the
comments say what was concluded from the C++ - and build on it where it is right.

- `gz-fmt-tex` (about 4k lines: `bc`, `bump`, `dds`, `load`, `make`, `mip`, `params`,
  `resample`, `tga`, `thm`) does not compile: `lib.rs` names a `tile` module that was never
  written, and `thm.rs` uses `gz-fmt-sound` without the crate depending on it. Decide the
  cleanest ownership of the sound thumbnail (it shares the THM container) and make it compile.
- `gz-fmt-sound` (`ogg`, `params`, `pcm`, `thm`) compiles; not reviewed.
- `gz-fmt-xr` is a placeholder.

## Scope

**`gz-fmt-tex`**
- DDS: read every layout the game and the editor use (DXT1/3/5, uncompressed A8R8G8B8 /
  X8R8G8B8 / R8G8B8 / A8 / L8 / R5G6B5, mip chains; cube and volume maps at least recognised)
  into RGBA8 mip levels; write BC1, BC1 with 1-bit alpha, BC3 and uncompressed BGRA with a full
  mip chain. Pure-Rust encoder (`texpresso` is in the workspace, with rayon); blocks and mip
  levels in parallel.
- TGA read / write (8 / 24 / 32-bit, RLE read); PNG / JPG / BMP through `image` into RGBA8.
- `.thm` (texture description): read and write byte-compatibly, every chunk version the C++
  reads: thumbnail pixels, type, `STextureParams` (format, flags, border and fade colour, fade
  amount and delay, mip filter, size, texture type, detail name and scale, material and weight,
  bump virtual height, bump mode, bump and normal-map names). Object / sound / group
  thumbnails share the container.
- `STextureParams` as pure functions: the DDS format and mip settings a params block asks for,
  so `make_game_texture(rgba, params) -> dds bytes` lives here; "has alpha"; the bump (`#`)
  texture maths when it is pure pixel work.
- `make_thumbnail_dds(rgba, w, h)`: the middle square, 256 x 256, DXT1 (DXT5 when any pixel is
  transparent), every mip level - the module thumbnail of the Mod Browser.
- The `tile` the earlier pass planned: a picture made for the size a browser tile draws it at
  (the file's own mip first, then an alpha-weighted average) - see `CLAUDE.md`, "Icons for the
  editor's own assets", and `docs/UI_DESIGN.md` section 6, Content Browser.
- Spec: `Source/Editors/XrECore/Editor/EThumbnail*.cpp/.h`, `ETextureParams.*`
  (`rg -l "STextureParams" Source`), `ImageManager.cpp` (pixel / format logic only),
  `Source/Editors/XrDXT/*` (what `DXTCompress` does with the params), `EditorCatalog.cpp` /
  `EditorModManifest.cpp` for `thumbnail.dds`.

**`gz-fmt-sound`**
- The OGG Vorbis comment block that carries the game parameters, read and rewritten without
  re-encoding the audio (re-page with correct CRCs); the sound thumbnail; PCM decoding through
  `lewton` (interleaved i16, rate, channels) and a peak envelope for a waveform.
- Spec: `Source/XrSound/SoundRender_Source_loader.cpp`,
  `Source/Editors/XrECore/Editor/SoundManager.cpp`, `EThumbnailSound.cpp`.

**`gz-fmt-xr`** - read all, write what the editor writes today:
- `lanims.xr` (`Source/XrEngine/LightAnimLibrary.cpp` and the editor's copy): read + write, and
  colour evaluation at a frame exactly.
- `gamemtl.xr` (`Source/XrEngine/GameMtlLib.*`, `Source/Editors/XrECore/Engine/GameMtlLib*.cpp`).
- `shaders.xr` (`Source/XrRender/Private/ResourceManager_Loader.cpp`, `Blender.*`): the blender
  list with name, class id, owner, timestamp, and the raw property stream per blender. Read.
- `shaders_xrlc.xr` (`rg Shader_xrLC Source`).
- `particles.xr` (`Source/XrRender/Private/PSLibrary.cpp`, `ParticleEffectDef.*`,
  `ParticleGroup.*`, the action stream of `Source/XrParticles`): effects and groups fully,
  actions decoded into typed values. Read.
- `senvironment.xr` (`Source/XrSound/SoundRender_Environment.cpp`): read + write.

## Rules

- Readers take `&[u8]` and return owned immutable models; writers return `Vec<u8>`; no file
  access. Hostile input is an `Err`: never a panic, never an allocation by an unchecked count
  (images are capped at 16384 x 16384).
- Pixel work is parallel (rayon) and light on allocation: it runs for thousands of textures.
- The API of each crate at the top of its `lib.rs`. Tests written, not run.
- Compile: `bash tools/rust/cg.sh check -p gz-fmt-tex -p gz-fmt-sound -p gz-fmt-xr --tests`,
  zero warnings.

## Report

Under 300 words: entry points per crate, what is not done or not certainly byte-compatible.
Then update the three rows in `docs/rust/STATE.md`.
