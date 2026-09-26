#pragma once

// NQ - deterministic top-down auto layout (docs/NQ_ARCHITECTURE.md par. 13.8):
// a pin-aware dominator tree keeps complete branch bounds apart and promotes
// shared descendants into merge blocks between their contributing branches.
// Exclusive trigger trees and root-level merge blocks use a larger gap.
//
// The node box is sized by its content here, and only here: the layout pass, the
// canvas (drawing and hit tests), the minimap and MCP quest_layout all read the
// same Box(), so a node can never be drawn taller than the layout made room for.

#include "NqAsset.h"

namespace NqLayout
{
	// canvas metrics in world units: pt at zoom 1.0 (the canvas multiplies by the zoom
	// level and the UI scale)
	const float kNodeWidth		= 240.f;
	const float kHeader			= 28.f;		// family strip, icon, title, id, badges
	const float kLine			= 20.f;		// one "key: value" line
	const int	kMaxLines		= 3;
	const float kActionRow		= 20.f;		// one on_enter / on_exit action
	const int	kMaxActions		= 4;		// then one "+N more" row
	const float kPinRow			= 18.f;		// the pin labels, inside the bottom edge
	const float kPad			= 6.f;		// above a block of lines or actions, and at the bottom
	const float kGapX			= 64.f;
	const float kTreeGapX		= 160.f;
	const float kGapY			= 72.f;
	const float kGrid			= 16.f;
	// world coordinates are bounded: a hand-written or corrupt .nqasset can carry
	// inf / NaN / 1e18, which neither this layout nor the canvas renderer survives
	const float kWorldMax		= 1.0e6f;

	// finite and inside +-kWorldMax (non-finite becomes 0)
	ECORE_API float		Sane		(float v);
	// clamps every node position; returns the number of nodes it had to fix
	ECORE_API int		SanePositions(SNqQuest& q);
	// clamps positions and runs the graph layout only when a node has no position
	ECORE_API int		EnsurePositions(SNqQuest& q);

	// One line of a node. A condition of the node (`cond` = its kind) reads "<title> <what it
	// checks>", a catalog parameter that is set "<name>: <value>", and when there are more than
	// fit the last line is "+N more" (no key).
	struct SLine
	{
		xr_string	key;		// the parameter's name, or the condition's title ("Not ..." when inverted)
		xr_string	value;
		xr_string	cond;		// a condition line: the condition's kind
		bool		negate = false;
	};

	// Where the parts of a node sit, in world units from its top-left corner.
	struct SBox
	{
		float	w = kNodeWidth, h = kHeader + kPad;
		int		lines = 0;			// "key: value" lines shown (<= kMaxLines)
		float	lines_y = 0.f;
		int		actions = 0;		// on_enter + on_exit
		int		action_rows = 0;	// rows shown: min(actions, kMaxActions)
		bool	more = false;		// a "+N more" row follows them
		float	actions_y = 0.f;
		bool	pin_labels = false;	// the pins are named inside the node
		float	pins_y = 0.f;
	};

	// the output pins in draw order: the catalog's (a case node's cases first), then the
	// ones only the file has; `declared` gets how many of them the catalog declares
	ECORE_API void		Pins		(const SNqNode& n, xr_vector<xr_string>& pins, int* declared = nullptr);
	// the lines a node shows (at most kMaxLines): its conditions first - what a wait waits
	// for, what a trigger fires on - then its parameters
	ECORE_API void		Lines		(const SNqNode& n, xr_vector<SLine>& out);
	// the key of a condition line: its title, "Not has money" when it is inverted
	ECORE_API xr_string	CondKey		(LPCSTR title, bool negate);
	// the content-sized box of a node
	ECORE_API void		Box			(const SNqNode& n, SBox& out);
	ECORE_API Fvector2	NodeSize	(const SNqNode& n);
	// positions every node (`only_missing` = keep nodes that already have pos
	// and place the rest below the existing layout); returns the number of
	// nodes moved
	ECORE_API int		Run			(SNqQuest& q, bool only_missing);
}
