#pragma once

// NQ - the two side panels of a graph document (docs/NQ_ARCHITECTURE.md par. 13.9):
// the Graph panel (the quest's or the behaviour graph's own settings: its header, its
// tasks or roles, and its variables under a splitter) and the Details panel (the
// selected node, and its selected action under a splitter). Both are property grids of
// the UI kit; the widgets are generated from the catalog parameter types. Edits are
// staged on a copy while a widget is active and committed to NqDoc (one undo step)
// when the interaction ends.

#include "../../../XrECore/Editor/Nq/NqDoc.h"
#include "../../../XrECore/Editor/Nq/NqCatalog.h"
#include "../../../XrECore/Editor/Nq/NqItemCaps.h"
#include "../../../XrECore/Editor/Nq/NqReferences.h"

class NqCanvas;

class NqInspector
{
public:
	explicit		NqInspector		(NqDoc* doc, NqCanvas* canvas);

	// the Graph panel, into the current window (it fills what is left of it)
	void			DrawGraphPanel	();
	// the Details panel, into the current window
	void			DrawDetails		(bool focus_rename, bool focus_action);
	// the inspector's own dialogs (Rename Task, Task References, Edit Text); called from the
	// document's window so that they belong to it whichever panels are shown
	void			DrawDialogs		();
	// staged edits reach the document once no widget is being worked
	void			CommitIdle		();

	// Scroll a panel to something on the next frame: "top", "bottom" (the Details node
	// section), "action:top", "action:bottom", "tasks", "task:<id>", "task:<id>/steps",
	// "variables". The panels redraw on demand, so the caller lets a frame happen first.
	static void		RequestScroll	(LPCSTR where);
	// The two splits for a caller without a mouse, as shares of their column: `action_frac`
	// the Details node pane, `vars_frac` the Variables pane of the Graph panel; below zero
	// leaves that one alone. Stored in the layout like a drag (graph.action_split,
	// graph.vars_split, pt).
	void			SetSplit		(float action_frac, float vars_frac);
	// the shares the last frame drew
	float			ActionSplit		() const { return m_ActionShare; }
	float			VarsSplit		() const { return m_VarsShare; }
	// where the scrolling panes stood on the last frame: {y, max}. A caller that cannot see
	// the scrollbar learns from `max` that there is more below.
	const Fvector2&	NodeScroll		() const { return m_NodeScroll; }
	const Fvector2&	ActionScroll	() const { return m_ActionScroll; }
	const Fvector2&	GraphScroll		() const { return m_GraphScroll; }
	const Fvector2&	VarsScroll		() const { return m_VarsScroll; }
	// the panes as the last frame drew them (px of the screen, x y w h; zero when not drawn)
	ImVec4			NodePane		() const { return m_NodePane; }
	ImVec4			ActionPane		() const { return m_ActionPane; }
	ImVec4			GraphPane		() const { return m_GraphPane; }
	ImVec4			VarsPane		() const { return m_VarsPane; }
	// an action is open under the node
	bool			HasAction		() const;

private:
	NqDoc*			m_Doc;
	NqCanvas*		m_Canvas;
	// staged node copy
	SNqNode			m_Node;
	xr_string		m_NodeId;
	u32				m_NodeRev;
	bool			m_NodeDirty;
	// staged quest header copy
	SNqQuest		m_Quest;
	u32				m_QuestRev;
	bool			m_QuestDirty;
	xr_string		m_LuaCheck;		// last syntax check result
	bool			m_FocusRename, m_FocusAction;
	char			m_Search[64];	// picker popup filter
	float			m_ActionShare, m_VarsShare;
	float			m_DetailsColumn, m_GraphColumn;		// pt of the split columns, last frame
	float			m_PendingAction, m_PendingVars;		// SetSplit before the columns were measured
	Fvector2		m_NodeScroll, m_ActionScroll, m_GraphScroll, m_VarsScroll;
	ImVec4			m_NodePane, m_ActionPane, m_GraphPane, m_VarsPane;
	int				m_VarSel;		// the variable the Graph panel edits
	// the module's global variables (mod.ltx [vars]) as the Graph panel edits them: read again
	// when the manifest's generation moves, written back once an edit is let go of
	xr_vector<SNqVar>	m_Globals;
	u32				m_GlobalsGen;
	bool			m_GlobalsDirty;
	int				m_GlobalSel;
	// the node and action the Details panel last showed, the frames the node pane still tries to bring
	// the action's row into view, and whether its Actions section is still to be opened for it
	xr_string		m_RevealKey;
	int				m_RevealFrames;
	bool			m_RevealOpen;
	// the parameter table the current widget belongs to, so a `value` can ask what
	// type the `name` beside it was declared with
	SNqValue*		m_ParamsCtx;
	bool			m_TaskRenameOpen;
	bool			m_TaskReferencesOpen;
	xr_string		m_TaskRenameFrom;
	xr_string		m_TaskRenameTo;
	xr_string		m_TaskRenameError;
	xr_string		m_TaskReferencesId;
	xr_string		m_TaskReferencesError;
	xr_vector<NqReferences::SReference>	m_TaskReferences;
	xr_vector<NqReferences::SDiagnostic>	m_TaskReferenceDiagnostics;
	bool			m_TaskReferencesComplete;
	u32			m_TaskReferencesGeneration;
	xr_vector<xr_string>	m_ProjectQuestIds;
	xr_vector<xr_string>	m_ProjectBehaviourIds;	// what behaviour.run may name
	u32			m_ProjectQuestIdsSerial;

	void			SyncNode		();
	void			CommitNode		();
	void			SyncQuest		();
	void			CommitQuest		();

	bool			Behaviour		() const { return m_Doc && m_Doc->quest.IsBehaviour(); }
	void			DrawHeaderPane	();
	void			DrawQuestFields	(bool& changed);
	void			DrawTasks		(bool& changed, xr_string& rename, xr_string& references);
	void			DrawRolesSection(bool& changed);
	// the graph's variables, in a pane of their own: they are read and written from anywhere
	// in the graph, so they are not a property of a node
	void			DrawVarsSection	();
	// the module's global variables under the graph's own: every quest and behaviour graph of
	// the module shares them, so they are saved into mod.ltx, not into this file
	void			DrawGlobalsSection();
	void			SaveGlobals		();
	void			DrawNodeSection	();
	void			DrawActionSection();
	void			BeginTaskRename	(LPCSTR id);
	void			DrawTaskRename	();
	void			BeginTaskReferences(LPCSTR id);
	void			DrawTaskReferences();
	void			DrawTextEdit	();

	// ---- variables -----------------------------------------------------------
	// A variable's type is the type of its declared default, so every value tied
	// to it is edited with the widget that type deserves instead of a free string.
	const SNqVar*	FindVar			(LPCSTR name) const;
	static LPCSTR	TypeName		(SNqValue::EType t);
	bool			DrawTypedValue	(LPCSTR label, SNqValue& v, SNqValue::EType as);
	bool			DrawVarNameCombo(LPCSTR label, xr_string& name);
	// renames a declared variable and every var.set/var.add/var reference to it
	bool			RenameVar		(LPCSTR from, LPCSTR to);

	// generic parameter editors; every one returns true when the value changed
	bool			DrawParams		(const NqCatalog::SKind* k, SNqValue& params, LPCSTR id_prefix);
	bool			DrawParam		(const NqCatalog::SParam& p, SNqValue& params, LPCSTR id_prefix, LPCSTR kind);
	bool			DrawTyped		(LPCSTR type, const NqCatalog::SParam* p, LPCSTR label, SNqValue& v);
	bool			DrawText		(LPCSTR label, SNqValue& v, bool multiline);
	bool			DrawPicked		(LPCSTR label, LPCSTR type, xr_string& s);
	bool			DrawDuration	(LPCSTR label, SNqValue& v);
	bool			DrawNpcRef		(LPCSTR label, SNqValue& v, bool target, bool kill);
	bool			DrawObjectRef	(LPCSTR label, SNqValue& v, bool character = false);
	bool			DrawObjectiveId	(LPCSTR label, SNqValue& v);
	bool			DrawPlace		(LPCSTR label, SNqValue& v);
	// the role vocabulary of a behaviour graph (NQ_ARCHITECTURE par. 19)
	bool			DrawDestination	(LPCSTR label, SNqValue& v);
	bool			DrawWho			(LPCSTR label, SNqValue& v);
	bool			DrawRoleTarget	(LPCSTR label, SNqValue& v, bool attack);
	bool			DrawRoleBindings(LPCSTR label, SNqValue& v);
	// spawn.npc: one row per NPC - a character of the project or a section
	bool			DrawNpcList		(LPCSTR label, SNqValue& v);
	bool			DrawPosition	(SNqValue& v);
	bool			DrawSpawnSpec	(LPCSTR label, SNqValue& v);
	bool			DrawItemList	(LPCSTR label, SNqValue& v, bool objects_ok = true,
									 const NqCatalog::SKind* rows_of = 0);
	bool			DrawRowState	(const NqCatalog::SKind* k, SNqValue& e);
	bool			DrawUpgrades	(const NqItemCaps::SCaps& caps, SNqValue& v);
	bool			DrawFetchParams	(const NqCatalog::SKind* k, SNqValue& params);
	bool			DrawItemSpawnParams(const NqCatalog::SKind* k, SNqValue& params);
	bool			DrawItemRowsParams(const NqCatalog::SKind* k, SNqValue& params);
	bool			DrawLootSpawn	(LPCSTR label, SNqValue& v);
	bool			DrawCases		(LPCSTR label, SNqValue& v, bool weights);
	bool			DrawCondList	(xr_vector<SNqCond>& conds, LPCSTR id_prefix, int depth);
	bool			DrawCondValue	(LPCSTR label, SNqValue& v, int depth);		// cond_list inside params
	bool			DrawLua			(LPCSTR label, SNqValue& v);
	bool			DrawScalarValue	(LPCSTR label, SNqValue& v);
	bool			DrawRaw			(LPCSTR label, SNqValue& v);
	bool			DrawKindCombo	(LPCSTR label, u32 use_mask, xr_string& kind);
	bool			PickerPopup		(LPCSTR popup, LPCSTR type, xr_string& out);
	void			RefreshProjectQuestIds();
};
