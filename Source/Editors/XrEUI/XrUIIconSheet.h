#pragma once

namespace XUI
{
// Every icon of XrUIIcons at 16, 24 and 48 pt with its name, in a grid that fills the current
// window's width - how the glyphs are checked by eye. The 48 pt one shows its 16-unit square and
// the 1-unit margin. `color` 0 draws in the text colour; the sheet starts at icon `first`.
XREUI_API void DrawIconSheet(ImU32 color = 0, int first = 1);
}
