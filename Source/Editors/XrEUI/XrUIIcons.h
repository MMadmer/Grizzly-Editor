#pragma once

// The editor's vector icon set (docs/UI_DESIGN.md, section 3). Every glyph is geometry on a 16-unit
// grid, built once and only scaled when drawn, so it is exact at any size and DPI - a bitmap is sharp
// at one size only. DrawIcon takes the size in pt; the colour is the caller's (the state tones live
// in the kit).
namespace XUI
{
enum class Icon : int
{
	None = 0,
	// chrome
	Minimize, Maximize, Restore, Close, ChevronUp, ChevronDown, ChevronLeft, ChevronRight, Caret, More,
	Overflow, Menu, Dot, Grip, Pin, External, SidebarLeft, SidebarRight, PanelBottom,
	// actions
	New, Open, Save, SaveAll, Reload, Undo, Redo, Cut, Copy, Paste, Duplicate, Rename, Delete, Plus, Minus,
	Clear, Check, Search, Filter, Sort, SortAsc, SortDesc, Settings, Eye, EyeOff, Lock, Unlock, Link, Unlink,
	Star, StarFilled, Bookmark, BookmarkFilled, BookmarkPrev, BookmarkNext, History, Back, Forward, Up, Home,
	Reveal, Info, Help, Keyboard, ExpandAll, CollapseAll, MoveUp, MoveDown, Globe, Key, Shield, Tag, Picture,
	Terminal, Refresh,
	// status
	Error, Warning, Success, Required, Spinner, Dirty,
	// build and run
	BuildCollision, BuildAIMap, BuildGameGraph, BuildAll, Play, PlayGame, PlayClean, Stop, ReloadConfigs,
	GameFolder, BuildMod, Package, Publish, Releases,
	// viewport
	Select, Place, Move, Rotate, Scale, SnapGrid, SnapAngle, SnapScale, SnapObject, FrameAll, FrameSelection,
	Perspective, Ortho, Lit, Unlit, Wireframe, Bones, Stats, ViewOptions,
	// scene object classes (the Tools panel, the Outliner)
	SceneObject, Light, SoundSource, SoundEnv, Glow, Shape, Spawn, Way, Sector, Portal, Group, Particles,
	DetailObjects, AIMap, Wallmark, FogVolume,
	// files and the editor's own asset kinds
	Folder, FolderOpen, File, FileTexture, FileModel, FileSound, FileScript, FileConfig, FileXml, FileText,
	FileArchive, Level, Quest, Behaviour, Character, CharacterTemplate, PdaInterface,
	// graph editor
	Flow, Wait, Condition, Variable, SpawnNpc, Dialog, Item, PdaMsg, World, Relation, Squad, Order, Timer,
	Trigger, End, Task, Role, Minimap, AutoLayout, Validate, FindInProject, ZoomIn, ZoomOut, ZoomFit, Zoom100,
	// other
	Person, Community, Rank, Money, Kit, Area, Transition, PlayerStart, Mode, Relations, Exclude, Aspect,
	Language, Tasks, Mcp, Log, Problems, Drawer,
	Count,
};

// min is the top-left corner in screen px, size_pt the square's side in pt (the DPI scale is applied
// inside). An icon without geometry draws nothing.
XREUI_API void DrawIcon(ImDrawList* draw_list, Icon icon, const ImVec2& min, float size_pt, ImU32 color);
// The same into a rectangle given in px: the glyph is centred in the largest square that fits.
XREUI_API void DrawIconRect(ImDrawList* draw_list, Icon icon, const ImVec2& min, const ImVec2& max, ImU32 color);
// Stable lower-case name ("chevron_down"), for MCP answers, tooltips of tests and the audit.
XREUI_API const char* IconName(Icon icon);
}
