# Brief: the vector icon set

Port the editor's icon engine and every glyph to Rust + egui 0.37. Read
`docs/rust/CONVENTIONS.md`, `docs/rust/STATE.md` and `docs/UI_DESIGN.md` section 3
("Iconography") first.

You own: `crates/gz-ui/src/icons.rs` (a placeholder now: the enum and the names are there, the
drawing is a box - replace the file) and a new folder `crates/gz-ui/src/icons/` for
sub-modules. Nothing else in that crate.

## The C++ that is the spec

- `Source/Editors/XrEUI/XrUIIcons.h` (51 lines), `XrUIIcons.cpp` (1668 lines): the enum,
  `IconName`, the path-building mini language, every glyph's geometry on the 16-unit grid, the
  stroke engine.
- `Source/Editors/XrEUI/XrUIIconSheet.cpp`: the developer sheet (every glyph at 16, 24, 48 pt).

## What to keep exactly

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon { None, Minimize, .. }      // the C++ variants in the same order, without Count
impl Icon {
    pub const ALL: &'static [Icon];       // every variant except None
    pub fn name(self) -> &'static str;    // what the C++ IconName returns: port its table
    pub fn from_name(name: &str) -> Option<Icon>;
}
pub fn draw(painter: &egui::Painter, icon: Icon, rect: egui::Rect, color: egui::Color32);
pub fn draw_in(painter: &egui::Painter, icon: Icon, rect: egui::Rect, color: egui::Color32);
pub fn sheet(ui: &mut egui::Ui, first: usize);
```

The placeholder's `name()` was derived mechanically (snake_case of the variant) and may differ
from the C++ table in places (`zoom100`?): the C++ table wins - MCP answers and QA read these.

## How it draws (section 3 of the design, and the C++)

- Geometry is built ONCE per icon - arcs tessellated then - on a 16-unit grid with a 1-unit
  margin, and only scaled when drawn. Keep the glyph table as plain data and the built paths in
  a `std::sync::OnceLock` (an immutable cache computed from constants; say so in a comment).
  No other static.
- Stroke 1.5 units, at least 1 px. At 20 px and below, points snap to the pixel grid (and a
  backtrack pass drops the spikes snapping would make); above 2 px of an opaque stroke, joins
  and caps are round; dashed paths are stretched to whole dashes; fills only where the C++
  fills (play / stop, dots, the pupil, on-states). One colour, the caller's.
- egui feathers its own strokes. Reproduce the LOOK - crisp 1 px lines small, round joins
  large - rather than the C++ triangle math: snap in pixels using `painter.pixels_per_point()`,
  check what joins `epaint::PathShape` / `Shape::line` make and add round joins and caps
  yourself if they are not round. Signatures: `C:\Users\admin\.cargo\registry\src\
  index.crates.io-*\{egui,epaint}-0.37.0\src`.
- An icon is drawn hundreds of times per frame: no allocation per draw beyond the shape egui
  needs; scale the cached points into a reused buffer or emit one `Shape` per path.

## Done means

- Every variant has its geometry; `bash tools/rust/cg.sh check -p gz-ui --tests` is clean with
  zero warnings in your files (errors in other new modules of `gz-ui` are another package's).
- Tests written (not run): every `Icon::ALL` entry has geometry and a unique name;
  `from_name(name)` round trip; a dozen names against the C++ table; the snapping helper.

## Report

Under 200 words: how joins and snapping were done in egui, any glyph you are unsure of by
name. Then update the `gz-ui` row of `docs/rust/STATE.md`.
