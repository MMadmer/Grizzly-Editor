#pragma once

// NQ - the kind catalog (docs/NQ_ARCHITECTURE.md par. 6, par. 13.2).
//
// The game owns the catalog: configs\nq\catalog.ltx (+ configs\nq\kinds\*.ltx)
// of the linked install, plus the kinds\*.ltx of every installed module. The
// editor's bundled copy under <sdk>\gamedata\configs\nq\ is only the fallback
// when no linked game provides one. Cached until Invalidate().

struct SNqValue;

class ECORE_API NqCatalog
{
public:
	enum EUse
	{
		useTrigger	= 1 << 0,
		useMain		= 1 << 1,
		useExtra	= 1 << 2,
		useCond		= 1 << 3,
	};

	struct SParam
	{
		xr_string			name;
		xr_string			type;		// catalog type name (item_section, text, enum, ...)
		bool				required;
		bool				has_default;
		xr_string			def;		// default as written in the catalog
		bool				has_min, has_max;
		double				min, max;
		xr_vector<xr_string> enums;		// enum=a|b|c
		SParam() : required(false), has_default(false), has_min(false), has_max(false), min(0), max(0) {}
	};

	// nested types are not covered by the class export
	struct ECORE_API SKind
	{
		xr_string			id;			// "dialog.topic"
		xr_string			group;
		xr_string			title;		// UTF-8
		xr_string			desc;		// UTF-8
		u32					use;		// EUse mask
		xr_vector<SParam>	params;		// catalog order
		// The schema of ONE ROW of a list parameter, declared as `rows` + `row.<name>`.
		// A list type (item_list) is shared by the kind that CREATES the things and the one
		// that asks the player to bring them; only the former declares row state, so the
		// permission travels with the kind instead of being wired into the type.
		xr_vector<SParam>	row_params;
		xr_vector<xr_string> pins;		// declared pins; "cases" = pins come from params.cases
		bool				pins_from_cases;
		bool				wait;
		bool				once_default;
		bool				event;		// cond: edge-triggered event condition
		xr_string			impl;
		// Which document may carry the kind (NQ_ARCHITECTURE par. 19.8). A kind
		// that says nothing works in both: only the ones that speak to the
		// player or to the PDA are quest-only, and only the ones that drive an
		// NPC are scenario-only.
		bool				in_quest;
		bool				in_scenario;
		int					since;
		xr_vector<xr_string> alias;		// old kind names this one replaces (CSV in the file)
		xr_string			source;		// file the section came from (for the UI badge)

		SKind() : use(0), pins_from_cases(false), wait(false), once_default(false), event(false),
				  in_quest(true), in_scenario(true), since(1) {}

		bool				InDoc		(bool scenario) const { return scenario ? in_scenario : in_quest; }

		bool				Is			(EUse u) const { return 0 != (use & u); }
		const SParam*		Param		(LPCSTR name) const;
		const SParam*		RowParam	(LPCSTR name) const;
		bool				HasRows		() const { return !row_params.empty(); }
		// declared (static) pin?
		bool				HasPin		(LPCSTR pin) const;
	};

	// forces a reload on next use (link change, rescan)
	static void						Invalidate	();
	// loads when needed; false only when not even the bundled file is readable
	static bool						Ensure		();
	static LPCSTR					Source		();		// "game" | "bundled" | "" (not loaded)
	static int						Version		();
	static int						Api			();
	// changes after every catalog reload, including a project-link switch
	static u32						Generation	();
	static LPCSTR					LoadError	();
	static const xr_vector<SKind>&	Kinds		();
	static const SKind*				Find		(LPCSTR id);
	// kinds usable at a position, in catalog order (UI dropdowns)
	// Only what the document being edited may carry: a quest is never offered
	// npc.goto, a scenario is never offered a task action. The editor edits one
	// document at a time, so the filter is set when one becomes active rather
	// than threaded through every picker (NQ_ARCHITECTURE par. 19.8).
	static void						SetDocScenario(bool scenario);
	static bool						DocScenario	();
	static void						KindsFor	(u32 use_mask, xr_vector<const SKind*>& out);
	// files that made up the catalog (diagnostics)
	static const xr_vector<xr_string>& Files	();

	// Groups of parameters a kind takes only one of at a time. The catalog line has
	// no syntax for that yet, so the shape lives here - read by the validator (E022)
	// and by the inspector, which greys out the losing groups. Each entry is a comma
	// separated list of parameter names; empty when the kind has no such rule.
	static void						ExclusiveForms(LPCSTR kind, xr_vector<xr_string>& out);
	// index of the form `param` belongs to, -1 when the kind has no rule about it
	static int						FormIndex	(LPCSTR kind, LPCSTR param);
	// true when `param` belongs to a form that a value already present has ruled out
	static bool						FormRuledOut(LPCSTR kind, LPCSTR param, const SNqValue& params);

	// MCP quest_catalog: full JSON response
	static void						McpCatalog	(xr_string& out);
};
