# Textures (`.texasset`)

This page is generated from the editor's own tables of the kind's keys (`crates/gz-media/src/fields.rs`): the reader, the check and this reference are one list. A test of `gz-media` fails when this file is not what the tables say. A texture asset is a text file beside its picture (`.png`, `.jpg`, `.tga`, `.bmp`, `.dds`) that says what a texture description (`.thm`) says, as named keys: nothing a `.thm` can say is unsayable here. It is published as `textures\<name>.dds` (with the bump pair `<name>_bump.dds` and `<name>_bump#.dds` when `bump` has a `source`) beside a copy of this text, which the game registers as the texture's description; a `.thm` of the same name loses to it. A key left out is what the texture's `use` needs.

Every key but the header may be left out. A key the editor does not know is kept where it is through every save and listed once as a note. The asset may lie anywhere in the project: the module build publishes it - the data baked from its source and a copy of this text - where the game looks for its `name`; the asset where it lies and its source never ship as they are. What is baked is kept in the project's `temp\media\`. `asset.check` names every problem at its key (`M1xx`); `asset.import from=<files> to=<folder>` brings files in and writes their assets. The rules every kind keeps are `docs/ASSETS.md`.

```lua
return {
  tex = 1,
  source = "wall_brick.png",
  name = "bunker\\wall_brick",
  material = "phong",
  weight = 0.3,
  bump = { source = "wall_brick_height.png", depth = 0.03 },
  detail = { texture = "detail\\detail_brick", scale = 4 },
}
```

## The keys

| Key | Type | Default | Meaning |
|---|---|---|---|
| `tex` | integer | - | The header: the format version (1). A file of a newer version is refused by name. |
| `source` | string (a file beside the asset) | the file with the asset's own name and `.png`, `.jpg`, `.tga`, `.bmp` or `.dds` | The picture. |
| `name` | string (a game name) | `<module id>\<file name>` | The name the game knows the texture by: it is published as `textures\<name>.dds`. |
| `use` | `"surface"` `"ui"` `"cube"` `"bump"` `"normal"` `"terrain"` | "surface" | What the texture is: a surface's picture, a picture of the interface (no mip levels), a cube map, a bump map, a normal map or a terrain's picture. Picking one sets what that use needs; the keys below set the rest. |
| `format` | `"auto"` `"dxt1"` `"dxt1_alpha"` `"dxt3"` `"dxt5"` `"rgba4444"` `"rgba1555"` `"rgb565"` `"rgb"` `"rgba"` `"nvhs"` `"nvhu"` `"a8"` `"l8"` `"a8l8"` `"bc4"` `"bc5"` `"bc6"` `"bc7"` | "auto" | How the game texture is stored; `auto`: DXT1, or DXT5 when the picture has alpha (what `use` needs otherwise). |
| `mips` | boolean | true (false for `ui`, `cube`, `bump`, `terrain`) | The texture keeps its mip levels. |
| `mip_filter` | `"box"` `"cubic"` `"point"` `"triangle"` `"quadratic"` `"advanced"` `"catrom"` `"mitchell"` `"gaussian"` `"sinc"` `"bessel"` `"hanning"` `"hamming"` `"blackman"` `"kaiser"` | "box" (`kaiser` for `normal`) | How the mip levels are made. |
| `has_alpha` | boolean | the picture's | The picture has an alpha channel. |
| `binary_alpha` | boolean | false | Alpha is either on or off. |
| `dither` | boolean | true | The colours are dithered when they are reduced. |
| `dither_each_mip` | boolean | false | Every mip level is dithered on its own. |
| `implicit_light` | boolean | false (true for `terrain`) | The level build lights the texture by its own alpha. |
| `alpha_border` | boolean | false | The mip levels get a transparent border. |
| `color_border` | boolean | false | The mip levels get a border of `border_color`. |
| `border_color` | { r, g, b, a } (0-255) | { 0, 0, 0, 0 } | The border's colour. |
| `fade_to_color` | boolean | false | The smaller mip levels fade to `fade_color`. |
| `fade_to_alpha` | boolean | false | The smaller mip levels fade to transparent. |
| `fade_color` | { r, g, b, a } (0-255) | { 0, 0, 0, 0 } | The colour the mip levels fade to. |
| `fade_amount` | integer, 0 to 100 | 0 | How much of the colour the mip levels fade to, per cent. |
| `fade_delay` | integer, 0 to 255 | 0 | The mip level the fade starts at. |
| `material` | `"oren_nayar"` `"blinn"` `"phong"` `"metal"` | "blinn" | How the game lights the surface: each model blends towards the next one by `weight` (Oren-Nayar, Blinn, Phong, metal, back to Oren-Nayar). |
| `weight` | number, 0 to 1 | 0 | How far `material` is blended towards the next model. |
| `normal_map` | string (a game name) | none | A bump map's own normal map, by its game name. |
| `bump` | the bump map (below) | none | The texture has a bump map: the game draws the surface's relief with it. |
| `detail` | the detail texture (below) | none | A finer texture laid over this one close up. |
| `extra_flags` | integer, 0 to 4294967295 | 0 | Flag bits of a texture description no key names, kept as they are. |

## `bump`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `texture` | string (a game name) | `<name>_bump` | The bump map by its game name, when it is a texture of its own. |
| `source` | string (a file beside the asset) | none | A picture the bump map is made of: the build publishes it as `textures\<name>_bump.dds` with its `<name>_bump#.dds`. |
| `from` | `"height"` `"normal"` | "height" | What `source` holds: a height map (gloss in alpha) or a normal map. |
| `depth` | number, 0 to 1 | 0.05 | How deep the relief is, metres. |
| `parallax` | boolean | false | The relief is drawn with parallax. |

## `detail`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `texture` | string (a game name) | - | The detail texture by its game name. |
| `scale` | number, 0 to 10000 | 1 | How many times it repeats over this texture. |
| `on` | `"none"` `"diffuse"` `"bump"` `"both"` | "diffuse" | What it is laid over: the colour, the bump map, both, or neither. |

## Problems

| Code | Severity | What |
|---|---|---|
| `M101` | error | The source is not there. |
| `M102` | error | The source cannot be read (the reader's sentence). |
| `M103` | error | Two assets of the kind publish one name (on both). |
| `M104` | warning | A texture nobody has: not a texture asset, not a file of the project, not the game's. |
| `M105` | warning | An engine shader the linked game does not have. |
| `M106` | warning | A compiler shader the linked game does not have. |
| `M107` | error | A surface's game material the linked game does not have: the level build refuses it (a bone's: a warning). |
| `M108` | note | A dynamic model whose source has no bones: it gets one bone, `root`. |
| `M109` | error | A value outside what its key takes. |
| `M110` | error | `parts` together with `source`. |
| `M111` | warning | What the model cannot take: `rotate` on a model with bones, a part that makes no faces. |
| `M112` | error | A surface that wears no texture: no level build and no model of the game takes it. |
