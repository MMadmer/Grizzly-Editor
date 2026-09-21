#pragma once

// NQ - the details panel of a quest document (docs/NQ_ARCHITECTURE.md par. 13.9):
// upper section = the selected node (or the quest when nothing is selected),
// lower section = the selected on_enter/on_exit action. Widgets are generated
// from the catalog parameter types; edits are staged on a copy while a widget
// is active and committed to NqDoc (one undo step) when the interaction ends.

#include "../../../XrECore/Editor/Nq/NqDoc.h"
#include "../../../XrECore/Editor/Nq/NqCatalog.h"
#include "../../../XrECore/Editor/Nq/NqItemCaps.h"
#include "../../../XrECore/Editor/Nq/NqReferences.h"

class NqInspector
{
public:
	explicit		NqInspector		(NqDoc* doc);

	// draws both sections into the current window (child regions)
	void			Draw			(bool focus_rename, bool focus_action);

private:
	NqDoc*			m_Doc;
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
	// height of the variables pane as a fraction of the column, so the split
	// survives a resized window; kept in the editor preferences between sessions
	float			m_VarsFrac;
	float			m_ActionFrac;	// share of the node/action column the NODE keeps
	// the parameter table the current widget belongs to, so a `value` can ask what
	// type the `name` beside it was declared with
	SNqValue*		m_ParamsCtx;
	bool			m_OpenTaskRename;
	bool			m_OpenTaskReferences;
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

	void			DrawQuestSection();
	void			DrawNodeSection	();
	void			DrawActionSection();
	// the quest's variables, in their own pane under the node - they are read and
	// written from anywhere in the graph, so they are not a property of a node
	void			DrawVarsSection	();
	void			BeginTaskRename	(LPCSTR id);
	void			DrawTaskRename	();
	void			BeginTaskReferences(LPCSTR id);
	void			DrawTaskReferences();
	void			DrawHSplitter	(LPCSTR id, float& frac, float total, bool grows_down = false);

	// ---- variables -----------------------------------------------------------
	// A variable's type is the type of its declared default, so every value tied
	// to it is edited with the widget that type deserves instead of a free string.
	const SNqVar*	FindVar			(LPCSTR name) const;
	static LPCSTR	TypeName		(SNqValue::EType t);
	bool			DrawTypedValue	(LPCSTR label, SNqValue& v, SNqValue::EType as, float reserve = 0.f);
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
	bool			DrawObjectRef	(LPCSTR label, SNqValue& v);
	bool			DrawObjectiveId	(LPCSTR label, SNqValue& v);
	bool			DrawPlace		(LPCSTR label, SNqValue& v);
	// the role vocabulary of a behaviour graph (NQ_ARCHITECTURE par. 19)
	bool			DrawDestination	(LPCSTR label, SNqValue& v);
	bool			DrawWho			(LPCSTR label, SNqValue& v);
	bool			DrawRoleTarget	(LPCSTR label, SNqValue& v, bool attack);
	bool			DrawRoleBindings(LPCSTR label, SNqValue& v);
	void			DrawRolesSection(bool& changed);
	bool			DrawPosition	(SNqValue& v);
	bool			DrawSpawnSpec	(LPCSTR label, SNqValue& v);
	bool			DrawItemList	(LPCSTR label, SNqValue& v, bool objects_ok = true,
									 const NqCatalog::SKind* rows_of = 0);
	bool			DrawRowState	(const NqCatalog::SKind* k, SNqValue& e);
	bool			DrawUpgrades	(const NqItemCaps::SCaps& caps, SNqValue& v);
	bool			DrawFetchParams	(const NqCatalog::SKind* k, SNqValue& params);
	bool			DrawItemSpawnParams(const NqCatalog::SKind* k, SNqValue& params);
	bool			DrawItemRowsParams(const NqCatalog::SKind* k, SNqValue& params);

public:
	// Scroll the panel to something on the next frame: "top", "bottom", or
	// "task:<id>". The inspector redraws on demand, so the caller has to let a
	// frame happen before the move is visible.
	static void		RequestScroll	(LPCSTR where);
private:
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
