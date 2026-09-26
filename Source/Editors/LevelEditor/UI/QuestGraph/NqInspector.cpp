#include "stdafx.h"
#include "NqInspector.h"
#include "NqCanvas.h"
#include "../../../XrECore/Editor/Nq/NqLua.h"
#include "../../../XrECore/Editor/Nq/NqPickers.h"
#include "../../../XrECore/Editor/Nq/NqItemCaps.h"
#include "../../../XrECore/Editor/Nq/NqUtil.h"
#include "../../../XrECore/Editor/EditorCharacters.h"

namespace
{
	using T = XFinedTheme::ColorToken;

	//--------------------------------------------------------------------------
	// Rows. Every field of both panels is a row of the kit's property grid: the
	// label in its column on the left, the value on the right, the help in the
	// label's tooltip and in the help footer - one layout for every field, where
	// the old form put some labels left, some right and some above.
	//--------------------------------------------------------------------------
	bool		s_InGrid = false;
	int			s_Depth = 0;			// nested rows (an objective of a task, a row of a list) indent
	xr_string	s_NextHelp;
	int			s_NextFlags = 0;

	struct SNest
	{
		SNest() { ++s_Depth; }
		~SNest() { --s_Depth; }
	};

	struct SGrid
	{
		bool open;
		explicit SGrid(LPCSTR role)
		{
			s_Depth = 0;
			s_NextHelp.clear();
			s_NextFlags = 0;
			open = XUI::BeginPropertyGrid(role);
			s_InGrid = open;
		}
		~SGrid()
		{
			if (open) XUI::EndPropertyGrid();
			s_InGrid = false;
		}
	};

	// what the next row is for: its label's tooltip and the help footer say it
	void Help(LPCSTR text)	{ s_NextHelp = text ? text : ""; }
	void Flags(int flags)	{ s_NextFlags |= flags; }
	// more to what the caller already said about the next row
	void MoreHelp(LPCSTR text)
	{
		if (!text || !*text) return;
		s_NextHelp = s_NextHelp.empty() ? xr_string(text) : s_NextHelp + ". " + text;
	}

	// ImGui hides everything after "##" in a widget label, plain text does not -
	// labels that carry an id suffix must be trimmed before they are printed
	LPCSTR Shown(LPCSTR label, xr_string& tmp)
	{
		LPCSTR h = label ? strstr(label, "##") : 0;
		if (!h) return label ? label : "";
		tmp.assign(label, h - label);
		return tmp.c_str();
	}

	// A labelled row: the value widget follows at the cursor with its width set. False when
	// the row is scrolled out of view - the caller draws no value then, and goes on with
	// the rows under it, which may well be in view.
	bool Row(LPCSTR label)
	{
		const xr_string help = s_NextHelp;
		const int flags = s_NextFlags;
		s_NextHelp.clear();
		s_NextFlags = 0;
		if (s_InGrid) return XUI::PropertyRow(label, help.empty() ? nullptr : help.c_str(), flags, s_Depth);
		// outside a grid (a dialog): the label above its field
		xr_string tmp;
		LPCSTR shown = Shown(label, tmp);
		if (shown[0]) ImGui::TextUnformatted(shown);
		ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
		return true;
	}

	// the id of a value widget: its row shows the label already
	xr_string Hidden(LPCSTR label) { return xr_string("##v_") + (label ? label : ""); }

	// "spot_task" -> "Spot task": a catalog name read as a label
	xr_string Pretty(LPCSTR name)
	{
		struct SWord { LPCSTR from, to; };
		static const SWord kWords[] = { { "id", "ID" }, { "npc", "NPC" }, { "npcs", "NPCs" }, { "pda", "PDA" }, { "lua", "Lua" }, { "ui", "UI" } };
		xr_string out;
		xr_vector<xr_string> words;
		NqUtil::Split(name ? name : "", '_', words);
		for (u32 i = 0; i < words.size(); ++i)
		{
			xr_string w = words[i];
			if (w.empty()) continue;
			for (const SWord& k : kWords)
				if (w == k.from) { w = k.to; break; }
			if (out.empty() && w[0] >= 'a' && w[0] <= 'z') w[0] = char(w[0] - 0x20);
			if (!out.empty()) out += " ";
			out += w;
		}
		return out.empty() ? xr_string(name ? name : "") : out;
	}

	// a text box over xr_string at the width the row set. The buffer is refilled from the
	// model every frame and grows with the value - a fixed one would silently truncate
	// long texts the moment they are edited.
	bool TextField(LPCSTR id, xr_string& s, ImGuiInputTextFlags flags = 0)
	{
		static xr_vector<char> buf;
		const size_t need = s.size() + 4096;
		if (buf.size() < need) buf.resize(need);
		memcpy(&buf[0], s.c_str(), s.size() + 1);
		if (!ImGui::InputText(id, &buf[0], buf.size(), flags)) return false;
		s = &buf[0];
		return true;
	}

	bool InputStr(LPCSTR label, xr_string& s)
	{
		if (!Row(label)) return false;
		return TextField(Hidden(label).c_str(), s);
	}

	bool ComboField(LPCSTR id, xr_string& cur, const xr_vector<xr_string>& items, LPCSTR empty_label = 0)
	{
		bool changed = false;
		LPCSTR preview = cur.empty() ? (empty_label ? empty_label : "") : cur.c_str();
		if (ImGui::BeginCombo(id, preview))
		{
			if (empty_label && ImGui::Selectable(empty_label, cur.empty())) { cur.clear(); changed = true; }
			for (u32 i = 0; i < items.size(); ++i)
				if (ImGui::Selectable(items[i].c_str(), cur == items[i])) { cur = items[i]; changed = true; }
			ImGui::EndCombo();
		}
		return changed;
	}

	bool ComboStr(LPCSTR label, xr_string& cur, const xr_vector<xr_string>& items, LPCSTR empty_label = 0)
	{
		if (!Row(label)) return false;
		return ComboField(Hidden(label).c_str(), cur, items, empty_label);
	}

	// a checkbox as a row's value
	bool CheckRow(LPCSTR label, bool& v)
	{
		if (!Row(label)) return false;
		return ImGui::Checkbox(Hidden(label).c_str(), &v);
	}

	// a number as a row's value, without steppers: dragged, or typed after a double-click
	bool NumberField(LPCSTR id, float& f, LPCSTR format = "%g", float speed = 0.1f)
	{
		return ImGui::DragFloat(id, &f, speed, 0.f, 0.f, format);
	}

	xr_vector<xr_string> Items(LPCSTR a, LPCSTR b = 0, LPCSTR c = 0, LPCSTR d = 0, LPCSTR e = 0, LPCSTR f = 0)
	{
		xr_vector<xr_string> v;
		if (a) v.push_back(a); if (b) v.push_back(b); if (c) v.push_back(c); if (d) v.push_back(d); if (e) v.push_back(e); if (f) v.push_back(f);
		return v;
	}

	// which single key of a "mode table" is set (story/ref/profile/community/...)
	xr_string ModeOf(const SNqValue& v, const xr_vector<xr_string>& modes)
	{
		if (!v.IsTable()) return "";
		for (u32 i = 0; i < modes.size(); ++i) if (v.Has(modes[i].c_str())) return modes[i];
		return "";
	}

	bool NodeEquals(const SNqNode& a, const SNqNode& b)
	{
		SNqValue va, vb;
		NqLua::NodeToValue(a, va);
		NqLua::NodeToValue(b, vb);
		return va.Equals(vb) && a.has_pos == b.has_pos;
	}

	// A variable is named from two shapes: typed structs at node level, and plain
	// tables once a condition nests inside another one's params (any{of=...},
	// cases). Renaming has to reach both or a rename silently orphans references.
	void RenameVarInValue(SNqValue& v, LPCSTR from, LPCSTR to)
	{
		if (!v.IsTable()) return;
		SNqValue* k = v.Get("kind");
		if (k && k->IsString() && (k->s == "var" || k->s == "var.set" || k->s == "var.add"))
		{
			SNqValue* nm = v.Get("name");
			if (nm && nm->IsString() && nm->s == from) *nm = SNqValue::String(to);
		}
		for (u32 i = 0; i < v.vals.size(); ++i) RenameVarInValue(v.vals[i], from, to);
		for (u32 i = 0; i < v.arr.size(); ++i)  RenameVarInValue(v.arr[i], from, to);
	}

	void RenameVarInConds(xr_vector<SNqCond>& conds, LPCSTR from, LPCSTR to)
	{
		for (u32 i = 0; i < conds.size(); ++i)
		{
			if (conds[i].kind == "var")
			{
				SNqValue* nm = conds[i].params.Get("name");
				if (nm && nm->IsString() && nm->s == from) *nm = SNqValue::String(to);
			}
			RenameVarInValue(conds[i].params, from, to);
		}
	}

	// The catalog states defaults as text, so the comparison is made in the type
	// the widget actually produced rather than by re-printing the value.
	bool EqualsDefault(const NqCatalog::SParam& p, const SNqValue& v)
	{
		if (!p.has_default || v.IsNil()) return false;
		if (v.IsBool())		return p.def == (v.b ? "true" : "false");
		if (v.IsNumber())	return !p.def.empty() && atof(p.def.c_str()) == v.n;
		if (v.IsString())	return p.def == v.s;
		return false;
	}

	void SplitSlot(const xr_string& sel, xr_string& slot, int& index)
	{
		slot.clear(); index = -1;
		LPCSTR c = strchr(sel.c_str(), ':');
		if (!c) return;
		slot.assign(sel.c_str(), c - sel.c_str());
		index = atoi(c + 1);
	}

	// A labelled line at the top of a pane: the icon and the name of what the pane shows
	void PaneHeader(XUI::Icon icon, ImU32 icon_col, LPCSTR title, LPCSTR sub, float right_pt = 0.f)
	{
		const ImVec2 p = ImGui::GetCursorScreenPos();
		const float w = ImGui::GetContentRegionAvail().x - XUI::Pt(right_pt);
		const float h = XUI::Pt(28.f);
		const float icon_px = XUI::Pt(16.f);
		XUI::DrawIconRect(ImGui::GetWindowDrawList(), icon, ImVec2(p.x, p.y + (h - icon_px) * 0.5f),
			ImVec2(p.x + icon_px, p.y + (h + icon_px) * 0.5f), icon_col);
		const float x = p.x + icon_px + XUI::Pt(8.f);
		float room = _max(0.f, p.x + w - x);
		{
			XUI::FontScope font(XUI::Font::Strong);
			ImGui::SetCursorScreenPos(ImVec2(x, p.y + (h - ImGui::GetFontSize()) * 0.5f));
			const float tw = _min(ImGui::CalcTextSize(title ? title : "").x, room * (sub && *sub ? 0.65f : 1.f));
			XUI::TextEllipsis(title ? title : "", tw);
			room -= tw + XUI::Pt(8.f);
		}
		if (sub && *sub && room > XUI::Pt(24.f))
		{
			XUI::FontScope font(XUI::Font::Small);
			ImGui::SameLine(0.f, XUI::Pt(8.f));
			ImGui::SetCursorScreenPos(ImVec2(ImGui::GetCursorScreenPos().x, p.y + (h - ImGui::GetFontSize()) * 0.5f));
			ImGui::PushStyleColor(ImGuiCol_Text, XFinedTheme::Vec4(T::TextMuted));
			XUI::TextEllipsis(sub, room);
			ImGui::PopStyleColor();
		}
		ImGui::SetCursorScreenPos(ImVec2(p.x, p.y + h));
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
	}

	// A button that sits in a grid under the rows it adds to (Add Objective, Add Row, ...).
	// One that takes something away (Delete Role) is the quiet destructive kind: error-coloured
	// text, never the Add look beside it.
	bool AddButton(LPCSTR label, XUI::Kind kind = XUI::Kind::Ghost)
	{
		if (s_InGrid) XUI::EndPropertySection();
		ImGui::SetCursorPosX(ImGui::GetCursorPosX() + XUI::Pt(12.f) * float(s_Depth) + XUI::Pt(6.f));
		return XUI::Button(label, kind);
	}

	//--------------------------------------------------------------------------
	// Edit Text: a text of more than one line (a briefing, a Lua body) is edited in a
	// dialog, not in a box squeezed into a row. The field names itself by its ImGui id;
	// Apply leaves the text for that field, which takes it on its next frame.
	//--------------------------------------------------------------------------
	struct STextEdit
	{
		bool		open = false;
		bool		code = false;
		ImGuiID		target = 0;
		bool		apply = false;
		xr_string	title;
		xr_string	text;
		xr_string	check;		// syntax check result of a Lua body
	};
	STextEdit s_TextEdit;

	void OpenTextEdit(ImGuiID target, LPCSTR title, const xr_string& text, bool code)
	{
		s_TextEdit.open = true;
		s_TextEdit.code = code;
		s_TextEdit.target = target;
		s_TextEdit.apply = false;
		s_TextEdit.title = title ? title : "";
		s_TextEdit.text = text;
		s_TextEdit.check.clear();
	}

	// true once, on the frame the field `target` gets the text Apply left for it
	bool TakeTextEdit(ImGuiID target, xr_string& out)
	{
		if (!s_TextEdit.apply || s_TextEdit.target != target) return false;
		out = s_TextEdit.text;
		s_TextEdit.apply = false;
		s_TextEdit.target = 0;
		return true;
	}

	xr_string FirstLine(const xr_string& s)
	{
		const size_t eol = s.find('\n');
		return eol == xr_string::npos ? s : s.substr(0, eol) + " \xE2\x80\xA6";
	}
}

// what the next frame should scroll to, consumed by the frame that honours it
static xr_string s_ScrollTo;

void NqInspector::RequestScroll(LPCSTR where) { s_ScrollTo = where ? where : ""; }

NqInspector::NqInspector(NqDoc* doc, NqCanvas* canvas) : m_Doc(doc), m_Canvas(canvas)
{
	m_NodeRev = m_QuestRev = u32(-1);
	m_NodeDirty = m_QuestDirty = false;
	m_FocusRename = m_FocusAction = false;
	m_Search[0] = 0;
	m_ActionShare = 0.5f;
	m_VarsShare = 0.38f;
	m_DetailsColumn = m_GraphColumn = 0.f;
	m_PendingAction = m_PendingVars = -1.f;
	m_NodeScroll.set(0.f, 0.f);
	m_ActionScroll.set(0.f, 0.f);
	m_GraphScroll.set(0.f, 0.f);
	m_VarsScroll.set(0.f, 0.f);
	m_NodePane = m_ActionPane = m_GraphPane = m_VarsPane = ImVec4(0, 0, 0, 0);
	m_VarSel = 0;
	m_RevealFrames = 0;
	m_RevealOpen = false;
	m_ParamsCtx = 0;
	m_TaskRenameOpen = false;
	m_TaskReferencesOpen = false;
	m_TaskReferencesComplete = true;
	m_TaskReferencesGeneration = 0;
	m_ProjectQuestIdsSerial = 0;
}

//------------------------------------------------------------------------------
// staging
//------------------------------------------------------------------------------
void NqInspector::SyncNode()
{
	xr_string sel = m_Doc->selection.size() == 1 ? m_Doc->selection[0] : "";
	if (sel != m_NodeId || m_NodeRev != m_Doc->revision)
	{
		// a pending edit reaches the document before the staged copy is
		// refilled - the document moving under us (MCP, layout, canvas drag)
		// must not swallow what the user typed
		if (m_NodeDirty) CommitNode();
		m_NodeId = sel;
		m_NodeRev = m_Doc->revision;
		m_NodeDirty = false;
		const SNqNode* n = sel.empty() ? 0 : m_Doc->quest.FindNode(sel.c_str());
		if (n) m_Node = *n; else m_Node = SNqNode();
	}
}

void NqInspector::CommitNode()
{
	if (!m_NodeDirty || m_NodeId.empty()) return;
	const SNqNode* cur = m_Doc->quest.FindNode(m_NodeId.c_str());
	m_NodeDirty = false;
	if (!cur || NodeEquals(*cur, m_Node)) return;
	SNqNode staged = m_Node;
	xr_string id = m_NodeId;
	m_Doc->Edit([&](SNqQuest& q)
	{
		SNqNode* n = q.FindNode(id.c_str());
		if (!n) return;
		// the canvas owns the coordinates; the inspector never edits them and
		// must not write a stale copy back over a drag or an auto layout
		const Fvector2 keep_pos = n->pos;
		const bool keep_has = n->has_pos;
		*n = staged;
		n->pos = keep_pos;
		n->has_pos = keep_has;
	});
	m_NodeRev = m_Doc->revision;
}

void NqInspector::SyncQuest()
{
	if (m_QuestRev != m_Doc->revision)
	{
		if (m_QuestDirty) CommitQuest();
		m_Quest = m_Doc->quest;
		m_Quest.nodes.clear();		// only the header is staged here
		m_QuestRev = m_Doc->revision;
		m_QuestDirty = false;
	}
}

void NqInspector::CommitQuest()
{
	if (!m_QuestDirty) return;
	m_QuestDirty = false;
	SNqQuest h = m_Quest;
	m_Doc->Edit([&](SNqQuest& q)
	{
		q.id = h.id; q.title = h.title; q.activation = h.activation;
		q.vars = h.vars; q.tasks = h.tasks; q.roles = h.roles;
	});
	m_QuestRev = m_Doc->revision;
}

void NqInspector::CommitIdle()
{
	// commit staged edits once the user lets go of the widget
	if (ImGui::IsAnyItemActive()) return;
	if (m_NodeDirty)  CommitNode();
	if (m_QuestDirty) CommitQuest();
}

bool NqInspector::HasAction() const
{
	xr_string slot; int index;
	SplitSlot(m_Doc->sel_slot, slot, index);
	return m_Doc->selection.size() == 1 && !slot.empty();
}

void NqInspector::SetSplit(float action_frac, float vars_frac)
{
	if (action_frac >= 0.f)
	{
		const float f = _max(0.1f, _min(action_frac, 0.9f));
		if (m_DetailsColumn > 0.f) XUI::Layout::Set("graph.action_split", f * m_DetailsColumn);
		else m_PendingAction = f;
		m_ActionShare = f;
	}
	if (vars_frac >= 0.f)
	{
		const float f = _max(0.1f, _min(vars_frac, 0.9f));
		// the stored size is the pane above the Variables
		if (m_GraphColumn > 0.f) XUI::Layout::Set("graph.vars_split", (1.f - f) * m_GraphColumn);
		else m_PendingVars = f;
		m_VarsShare = f;
	}
}

//------------------------------------------------------------------------------
// Graph panel
//------------------------------------------------------------------------------
void NqInspector::DrawGraphPanel()
{
	SyncQuest();
	const ImVec2 origin = ImGui::GetCursorScreenPos();
	const ImVec2 avail = ImGui::GetContentRegionAvail();
	const float spacing = ImGui::GetStyle().ItemSpacing.y;
	m_GraphColumn = _max(1.f, (avail.y - spacing) / XUI::Dpi());
	if (m_PendingVars >= 0.f) { XUI::Layout::Set("graph.vars_split", (1.f - m_PendingVars) * m_GraphColumn); m_PendingVars = -1.f; }

	// The Variables fold like the sections above them. Folded, their header sits at the bottom
	// and the pane above takes the room; the split waits for them to open again. The state is
	// the header's own (its id in this window), read before the panes are laid out.
	static LPCSTR kVarsTitle = "Variables##nq_vars";
	ImGuiStorage* storage = ImGui::GetStateStorage();
	const ImGuiID vars_id = ImGui::GetID(kVarsTitle);
	// scroll=variables brings them into view, so it opens them
	if (s_ScrollTo == "variables") storage->SetBool(vars_id, true);
	const bool vars_open = storage->GetBool(vars_id, true);
	const float head_px = XUI::Pt(28.f);

	// the header and the tasks (or roles) above, the variables below: the upper pane keeps its
	// size, the variables take what a taller window adds
	float top_px = _max(1.f, avail.y - head_px - spacing);
	if (vars_open)
	{
		float top = 0.f;
		XUI::Splitter("graph.vars_split", false, &top, 120.f, 90.f, _max(120.f, m_GraphColumn * 0.62f));
		top_px = XUI::Pt(top);
	}
	ImGui::BeginChild("##nq_graph_top", ImVec2(avail.x, top_px), ImGuiChildFlags_None);
	DrawHeaderPane();
	m_GraphScroll.set(ImGui::GetScrollY(), ImGui::GetScrollMaxY());
	ImGui::EndChild();
	m_GraphPane = ImVec4(origin.x, origin.y, avail.x, top_px);

	const float vars_y = origin.y + top_px + spacing;
	ImGui::SetCursorScreenPos(ImVec2(origin.x, vars_y));
	const xr_string count = m_Quest.vars.empty() ? xr_string() : NqUtil::Format("%d", (int)m_Quest.vars.size());
	float vars_h = head_px;
	if (XUI::Section(kVarsTitle, true, count.empty() ? nullptr : count.c_str()))
	{
		// the list and its form scroll under a header that stays
		const float body_y = ImGui::GetCursorScreenPos().y;
		ImGui::BeginChild("##nq_graph_vars", ImVec2(avail.x, _max(1.f, origin.y + avail.y - body_y)), ImGuiChildFlags_None);
		DrawVarsSection();
		m_VarsScroll.set(ImGui::GetScrollY(), ImGui::GetScrollMaxY());
		ImGui::EndChild();
		vars_h = _max(head_px, origin.y + avail.y - vars_y);
	}
	else m_VarsScroll.set(0.f, 0.f);
	m_VarsPane = ImVec4(origin.x, vars_y, avail.x, vars_h);
	m_VarsShare = vars_h / _max(1.f, avail.y);
}

void NqInspector::DrawHeaderPane()
{
	// top and bottom belong to the Details panel while a node is selected
	const bool mine = m_Doc->selection.size() != 1;
	if (s_ScrollTo == "top" && mine) { ImGui::SetScrollY(0.f); s_ScrollTo.clear(); }
	bool ch = false;
	xr_string rename, references;
	DrawQuestFields(ch);
	if (Behaviour())	DrawRolesSection(ch);
	else				DrawTasks(ch, rename, references);
	if (s_ScrollTo == "bottom" && mine) { ImGui::SetScrollY(ImGui::GetScrollMaxY()); s_ScrollTo.clear(); }
	if (ch) m_QuestDirty = true;
	if (!references.empty()) BeginTaskReferences(references.c_str());
	if (!rename.empty()) BeginTaskRename(rename.c_str());
}

void NqInspector::DrawQuestFields(bool& ch)
{
	const bool behaviour = Behaviour();
	if (!XUI::Section(behaviour ? "Behaviour Graph##nq_head" : "Quest##nq_head", true)) return;
	{
		SGrid grid("graph_header");
		Help(behaviour ? "The graph's id: what behaviour.run names. a-z, 0-9 and _" : "The quest's id: unique in the module. a-z, 0-9 and _");
		ch |= InputStr("ID##q", m_Quest.id);
		// never starts itself and owns no tasks: a caller runs it, and what it has instead is a cast
		Help(behaviour ? "For the author only - a behaviour graph shows the player nothing" : "What the player reads in the PDA");
		ch |= DrawText("Title##q", m_Quest.title, false);
		if (!behaviour)
		{
			Help("Auto: the quest starts with the game. Manual: another quest or a script starts it");
			if (Row("Activation##q"))
			{
				static const char* kModes[] = { "Auto", "Manual" };
				int cur = m_Quest.activation == "manual" ? 1 : 0;
				if (XUI::Segmented("##activation", &cur, kModes, 2)) { m_Quest.activation = cur ? "manual" : "auto"; ch = true; }
			}
		}
	}
	XUI::EndSection();
}

void NqInspector::DrawTasks(bool& ch, xr_string& rename, xr_string& references)
{
	if (s_ScrollTo == "tasks") { ImGui::SetScrollHereY(0.f); s_ScrollTo.clear(); }
	const xr_string count = NqUtil::Format("%d", (int)m_Quest.tasks.size());
	if (!XUI::Section("Tasks##nq_tasks", true, count.c_str())) return;
	{
		SGrid grid("graph_tasks");
		ImGui::PushID("tasks");
		for (u32 i = 0; i < m_Quest.tasks.size(); ++i)
		{
			SNqTask& t = m_Quest.tasks[i];
			if (s_ScrollTo.size() > 5 && 0 == strncmp(s_ScrollTo.c_str(), "task:", 5) && t.id == s_ScrollTo.c_str() + 5)
			{
				ImGui::SetScrollHereY(0.f);
				s_ScrollTo.clear();
			}
			ImGui::PushID((int)i);
			const xr_string head = NqUtil::Format("%s###task", t.id.empty() ? "(no id)" : t.id.c_str());
			const xr_string badge = t.objectives.empty() ? xr_string() : NqUtil::Format("%d objectives", (int)t.objectives.size());
			bool removed = false;
			if (XUI::PropertySection(head.c_str(), true, badge.empty() ? nullptr : badge.c_str()))
			{
				Help("The task's id in the PDA and in saves. Rename changes every reference with it");
				if (Row("ID##t"))
				{
					XUI::PropertyText(t.id.c_str());
					if (XUI::PropertyTrailing("##refs", XUI::Icon::FindInProject, "Find the task's references in the project")) references = t.id;
					if (XUI::PropertyTrailing("##rename", XUI::Icon::Rename, "Rename the task and every reference to it")) rename = t.id;
				}
				Help("The task's name in the PDA");
				ch |= DrawText("Title##t", t.title, false);
				Help("What the PDA says about the task");
				ch |= DrawText("Description##t", t.descr, true);
				Help("Storyline tasks are listed first in the PDA");
				ch |= ComboStr("Type##t", t.type, Items("additional", "storyline"));
				Help("Where the task's marker points");
				ch |= DrawNpcRef("Target##t", t.target, true, false);
				Help("The PDA icon of the task: a texture id");
				ch |= InputStr("Icon##t", t.icon);

				// The steps of the task. Each is its own line in the PDA with its own marker,
				// which is how one task covers "collect X, then Y, then report".
				{
					xr_string want = "task:" + t.id + "/steps";
					if (s_ScrollTo == want) { ImGui::SetScrollHereY(0.f); s_ScrollTo.clear(); }
				}
				// meaningless without steps to wait for, so it lives with them
				if (!t.objectives.empty())
				{
					Help("The last completed objective completes the task. Off when the task ends on something its objectives do not cover");
					if (CheckRow("Auto-complete##t", t.auto_complete)) ch = true;
				}
				int up = -1, down = -1, drop = -1;
				for (u32 j = 0; j < t.objectives.size(); ++j)
				{
					SNqObjective& o = t.objectives[j];
					ImGui::PushID((int)j);
					SNest nest;
					const xr_string label = NqUtil::Format("Objective %u###obj", j + 1);
					Help("The objective's id: what objective.* and task.* nodes name");
					if (Row(label.c_str()))
					{
						if (TextField("##oid", o.id)) ch = true;
						if (XUI::PropertyTrailing("##del", XUI::Icon::Delete, "Remove the objective")) drop = (int)j;
						if (j + 1 < t.objectives.size() && XUI::PropertyTrailing("##down", XUI::Icon::MoveDown, "Move down")) down = (int)j;
						if (j > 0 && XUI::PropertyTrailing("##up", XUI::Icon::MoveUp, "Move up")) up = (int)j;
					}
					{
						SNest inner;
						Help("The objective's line in the PDA");
						ch |= DrawText("Title##o", o.title, false);
						ch |= DrawText("Description##o", o.descr, true);
						Help("Where the objective's marker points; unset: no marker");
						ch |= DrawNpcRef("Target##o", o.target, true, false);
						Help("Off: the step runs but takes no room in the PDA. task.set_objective_visible turns it on later");
						if (CheckRow("Visible##o", o.visible)) ch = true;
					}
					ImGui::PopID();
				}
				if (drop >= 0)							{ t.objectives.erase(t.objectives.begin() + drop); ch = true; }
				else if (up > 0)						{ std::swap(t.objectives[up], t.objectives[up - 1]); ch = true; }
				else if (down >= 0 && down + 1 < (int)t.objectives.size()) { std::swap(t.objectives[down], t.objectives[down + 1]); ch = true; }

				if (AddButton("Add Objective"))
				{
					SNqObjective o;
					for (u32 number = 1;; ++number)
					{
						o.id = NqUtil::Format("step%u", number);
						if (!t.FindObjective(o.id.c_str())) break;
					}
					t.objectives.push_back(o);
					ch = true;
				}
				ImGui::SameLine();
				if (XUI::Button("Delete Task", XUI::Kind::GhostDanger)) removed = true;
				XUI::Tip("Removes the task from the quest. Ctrl+Z brings it back");
			}
			XUI::EndPropertySection();
			ImGui::PopID();
			if (removed) { m_Quest.tasks.erase(m_Quest.tasks.begin() + i); ch = true; break; }
		}
		if (AddButton("Add Task"))
		{
			SNqTask t;
			for (u32 number = 1;; ++number)
			{
				t.id = NqUtil::Format("task%u", number);
				bool used = false;
				for (u32 i = 0; i < m_Quest.tasks.size(); ++i)
					if (m_Quest.tasks[i].id == t.id) { used = true; break; }
				if (!used) break;
			}
			t.type = "additional";
			t.title = SNqValue::String(""); t.descr = SNqValue::String("");
			m_Quest.tasks.push_back(t);
			ch = true;
		}
		ImGui::PopID();
	}
	XUI::EndSection();
}

// The cast of a behaviour graph: named slots its caller fills. A step says
// who = "<role>", so the roles are the first thing an author sets up - which is
// why they sit where a quest keeps its tasks.
void NqInspector::DrawRolesSection(bool& ch)
{
	if (s_ScrollTo == "tasks") { ImGui::SetScrollHereY(0.f); s_ScrollTo.clear(); }
	const xr_string count = NqUtil::Format("%d", (int)m_Quest.roles.size());
	if (!XUI::Section("Roles##nq_roles", true, count.c_str())) return;
	{
		SGrid grid("graph_roles");
		ImGui::PushID("roles");
		for (u32 i = 0; i < m_Quest.roles.size(); ++i)
		{
			SNqRole& r = m_Quest.roles[i];
			ImGui::PushID((int)i);
			const xr_string head = NqUtil::Format("%s###role", r.name.empty() ? "(no name)" : r.name.c_str());
			bool removed = false;
			if (XUI::PropertySection(head.c_str(), true, r.many ? "group" : nullptr))
			{
				Help("What the steps call it: who = \"<name>\". a-z, 0-9 and _ only");
				ch |= InputStr("Name##r", r.name);
				Help("For the author: shown where the caller binds the role. Never seen by the player");
				ch |= DrawText("Title##r", r.title, false);
				// Two flags of one binding share a row: in the narrow panel a row puts its label
				// above the value, and a lone box under its own label is a row of nothing.
				Help("Group: the role may hold several NPCs - a squad, a list. Optional: bound to nobody is not an error");
				if (Row("Binding##r"))
				{
					if (ImGui::Checkbox("Group##r_many", &r.many)) ch = true;
					XUI::Tip("The role may hold several NPCs - a squad, a list. Off: exactly one");
					ImGui::SameLine(0.f, XUI::Pt(16.f));
					if (ImGui::Checkbox("Optional##r_optional", &r.optional)) ch = true;
					XUI::Tip("Bound to nobody is not an error: steps addressed to it do nothing");
				}
				Help("Who fills the role when the caller names nobody. Unset: the caller has to say");
				ch |= DrawNpcRef("Default##r", r.def, false, false);
				// with the role's other actions, not beside its name, where it read as "clear the name"
				if (AddButton("Delete Role", XUI::Kind::GhostDanger)) removed = true;
				XUI::Tip("Removes the role from the graph. Ctrl+Z brings it back");
			}
			XUI::EndPropertySection();
			ImGui::PopID();
			if (removed) { m_Quest.roles.erase(m_Quest.roles.begin() + i); ch = true; break; }
		}
		if (AddButton("Add Role"))
		{
			SNqRole r;
			for (u32 number = 1;; ++number)
			{
				r.name = NqUtil::Format("role%u", number);
				bool used = false;
				for (u32 i = 0; i < m_Quest.roles.size(); ++i)
					if (m_Quest.roles[i].name == r.name) { used = true; break; }
				if (!used) break;
			}
			r.title = SNqValue::String("");
			m_Quest.roles.push_back(r);
			ch = true;
		}
		ImGui::PopID();
	}
	XUI::EndSection();
}

//------------------------------------------------------------------------------
// variables
//------------------------------------------------------------------------------
LPCSTR NqInspector::TypeName(SNqValue::EType t)
{
	switch (t)
	{
	case SNqValue::tBool:	return "bool";
	case SNqValue::tNumber:	return "number";
	case SNqValue::tString:	return "text";
	default:				return "?";
	}
}

const SNqVar* NqInspector::FindVar(LPCSTR name) const
{
	if (!name || !name[0]) return 0;
	for (u32 i = 0; i < m_Quest.vars.size(); ++i)
		if (m_Quest.vars[i].name == name) return &m_Quest.vars[i];
	return 0;
}

bool NqInspector::DrawTypedValue(LPCSTR label, SNqValue& v, SNqValue::EType as)
{
	if (!Row(label)) return false;
	const xr_string id = Hidden(label);
	switch (as)
	{
	case SNqValue::tBool:
	{
		// a box, not a string: "true" is the only spelling the runtime reads, and typing it
		// by hand is how a condition silently stops matching
		bool b = v.IsBool() && v.b;
		if (ImGui::Checkbox(id.c_str(), &b)) { v = SNqValue::Bool(b); return true; }
		return false;
	}
	case SNqValue::tNumber:
	{
		// %g, not the stock %.3f: a counter reads as 0, not 0.000, and a value that
		// really is fractional still shows its digits
		float f = v.IsNumber() ? float(v.n) : 0.f;
		if (NumberField(id.c_str(), f)) { v = SNqValue::Number(double(f)); return true; }
		return false;
	}
	default:
	{
		xr_string s = v.IsString() ? v.s : "";
		if (TextField(id.c_str(), s)) { v = SNqValue::String(s); return true; }
		return false;
	}
	}
}

bool NqInspector::DrawVarNameCombo(LPCSTR label, xr_string& name)
{
	// The list is the graph's own declarations, so an action cannot point at a
	// variable that does not exist - which used to be a plain typed string.
	xr_vector<xr_string> names;
	for (u32 i = 0; i < m_Quest.vars.size(); ++i) names.push_back(m_Quest.vars[i].name);
	// a name from an older asset that is no longer declared stays selectable, or
	// switching nodes would quietly rewrite it
	if (!name.empty() && !FindVar(name.c_str())) names.push_back(name);
	if (names.empty())
	{
		if (Row(label)) XUI::PropertyText("No variables - add one in the Graph panel");
		return false;
	}
	return ComboStr(label, name, names, "(unset)");
}

bool NqInspector::RenameVar(LPCSTR from, LPCSTR to)
{
	if (!from || !to || !to[0] || 0 == xr_strcmp(from, to)) return false;
	if (FindVar(to)) return false;						// name already taken
	bool touched = false;
	for (u32 i = 0; i < m_Quest.vars.size(); ++i)
		if (m_Quest.vars[i].name == from) { m_Quest.vars[i].name = to; touched = true; }
	if (!touched) return false;

	// every reference follows: the document is edited directly because the staged
	// node copy only holds the node the author is looking at
	xr_string old_name = from, new_name = to;
	m_Doc->Edit([&](SNqQuest& q)
	{
		for (u32 i = 0; i < q.vars.size(); ++i)
			if (q.vars[i].name == old_name) q.vars[i].name = new_name;
		for (u32 n = 0; n < q.nodes.size(); ++n)
		{
			SNqNode& node = q.nodes[n];
			for (int pass = 0; pass < 2; ++pass)
			{
				xr_vector<SNqAction>& v = node.Slot(pass == 0 ? "enter" : "exit");
				for (u32 a = 0; a < v.size(); ++a)
				{
					if (v[a].kind == "var.set" || v[a].kind == "var.add")
					{
						SNqValue* nm = v[a].params.Get("name");
						if (nm && nm->IsString() && nm->s == old_name) *nm = SNqValue::String(new_name);
					}
					// an action can carry a nested cond list of its own
					RenameVarInValue(v[a].params, old_name.c_str(), new_name.c_str());
				}
			}
			RenameVarInConds(node.cond, old_name.c_str(), new_name.c_str());
		}
	});
	// the staged copy was filled before the rename; take the document's word for it
	m_NodeRev = u32(-1);
	return true;
}

// A list of the variables, and the one picked from it in a form under the list: three fields
// on one line of a 280 pt panel is three fields nobody can read. The header with the count is
// the Graph panel's (DrawGraphPanel): it stays while this scrolls.
void NqInspector::DrawVarsSection()
{
	if (s_ScrollTo == "variables") { ImGui::SetScrollY(0.f); s_ScrollTo.clear(); }
	bool ch = false;
	ImGui::PushID("qvars");

	auto add = [&]()
	{
		SNqVar v;
		for (int number = (int)m_Quest.vars.size() + 1;; ++number)
		{
			v.name = NqUtil::Format("var%d", number);
			if (!FindVar(v.name.c_str())) break;
		}
		v.value = SNqValue::Bool(false);
		m_Quest.vars.push_back(v);
		m_VarSel = (int)m_Quest.vars.size() - 1;
		ch = true;
	};

	if (m_Quest.vars.empty())
	{
		if (XUI::EmptyState(XUI::Icon::Variable, "No variables", "A variable's default value is also its type.", "Add Variable")) add();
	}
	else
	{
		m_VarSel = _max(0, _min(m_VarSel, (int)m_Quest.vars.size() - 1));
		const float rows = float(_min((int)m_Quest.vars.size(), 6));
		int drop = -1;
		if (XUI::BeginList("##vars", ImVec2(0.f, rows * 22.f + 10.f)))
		{
			for (u32 i = 0; i < m_Quest.vars.size(); ++i)
			{
				const SNqVar& var = m_Quest.vars[i];
				const xr_string row_id = NqUtil::Format("var%u", i);
				if (XUI::ListRow(row_id.c_str(), (int)i == m_VarSel)) m_VarSel = (int)i;
				xr_string value = var.value.IsBool() ? (var.value.b ? "true" : "false")
					: var.value.IsNumber() ? NqUtil::Format("%g", var.value.n) : "\"" + var.value.AsString("") + "\"";
				const xr_string text = var.name + "  \xC2\xB7  " + TypeName(var.value.type) + " " + value;
				XUI::TextEllipsis(text.c_str(), _max(0.f, ImGui::GetContentRegionAvail().x - XUI::Pt(28.f)));
				static const XUI::Icon kIcons[] = { XUI::Icon::Delete };
				static const char* kTips[] = { "Remove the variable" };
				if (XUI::RowActions(kIcons, kTips, 1) == 0) drop = (int)i;
			}
		}
		XUI::EndList();
		if (drop >= 0)
		{
			m_Quest.vars.erase(m_Quest.vars.begin() + drop);
			m_VarSel = _max(0, _min(m_VarSel, (int)m_Quest.vars.size() - 1));
			ch = true;
		}
		if (XUI::Button("Add Variable", XUI::Kind::Ghost)) add();

		if (m_VarSel < (int)m_Quest.vars.size())
		{
			SNqVar& var = m_Quest.vars[m_VarSel];
			ImGui::PushID(m_VarSel);
			{
				SGrid grid("graph_vars");
				Help("Renaming changes every var node and condition that names it");
				if (Row("Name##var"))
				{
					// committed when the field is left, so a half-typed name does not rewrite
					// every reference in the graph on the way through
					char namebuf[64];
					strncpy_s(namebuf, sizeof(namebuf), var.name.c_str(), _TRUNCATE);
					if (ImGui::InputText("##name", namebuf, sizeof(namebuf), ImGuiInputTextFlags_EnterReturnsTrue) || ImGui::IsItemDeactivatedAfterEdit())
					{
						const xr_string want = NqUtil::Trim(namebuf);
						if (!want.empty() && want != var.name)
						{
							if (RenameVar(var.name.c_str(), want.c_str())) ch = true;
							else Msg("! [nq] variable '%s' already exists", want.c_str());
						}
					}
				}
				// the declared default is what gives a variable its type, so changing the type
				// here is what converts the default
				Help("Bool, number or text: the type of the default, and of every value it takes");
				if (Row("Type##var"))
				{
					static const char* kTypes[] = { "Bool", "Number", "Text" };
					int cur = var.value.IsBool() ? 0 : var.value.IsNumber() ? 1 : 2;
					const int was = cur;
					if (XUI::Segmented("##type", &cur, kTypes, 3) && cur != was)
					{
						switch (cur)
						{
						case 0:  var.value = SNqValue::Bool(var.value.IsNumber() ? var.value.n != 0.0 : false); break;
						case 1:  var.value = SNqValue::Number(var.value.IsBool() ? (var.value.b ? 1.0 : 0.0) : 0.0); break;
						default: var.value = SNqValue::String(var.value.IsBool() ? (var.value.b ? "true" : "false")
									: var.value.IsNumber() ? NqUtil::Format("%g", var.value.n).c_str() : ""); break;
						}
						ch = true;
					}
				}
				Help("The value the variable has when the graph starts");
				ch |= DrawTypedValue("Default##var", var.value, var.value.type);
			}
			ImGui::PopID();
		}
	}
	ImGui::PopID();
	if (ch) m_QuestDirty = true;
}

//------------------------------------------------------------------------------
// Details panel
//------------------------------------------------------------------------------
void NqInspector::DrawDetails(bool focus_rename, bool focus_action)
{
	m_FocusRename |= focus_rename;
	m_FocusAction |= focus_action;
	SyncNode();
	SyncQuest();

	const ImVec2 origin = ImGui::GetCursorScreenPos();
	const ImVec2 avail = ImGui::GetContentRegionAvail();
	const float spacing = ImGui::GetStyle().ItemSpacing.y;
	// the help footer: two lines of the hovered row's help under both panes
	const float footer = XUI::Pt(18.f) + XUI::FontSize(XUI::Font::Small) * XUI::Dpi() * 2.f;
	const float column = _max(XUI::Pt(60.f), avail.y - footer - spacing);
	m_DetailsColumn = column / XUI::Dpi();
	m_NodePane = m_ActionPane = ImVec4(0, 0, 0, 0);

	// A newly selected action (a click on the canvas, quest_view slot=) is brought into view in the node
	// pane: an action open below with its row scrolled out gave no sign which one it was. Judged here, where
	// "nothing selected" is seen too - the same node and action selected again is new again. A few frames at
	// most (the split the action opens resizes the pane), so it never fights the author's own scrolling.
	const xr_string reveal = m_NodeId.empty() ? xr_string() : m_NodeId + "|" + m_Doc->sel_slot;
	if (reveal != m_RevealKey)
	{
		m_RevealKey = reveal;
		m_RevealFrames = m_NodeId.empty() || m_Doc->sel_slot.empty() ? 0 : 3;
		m_RevealOpen = m_RevealFrames > 0;
	}

	if (m_NodeId.empty())
	{
		m_NodeScroll.set(0.f, 0.f);
		m_ActionScroll.set(0.f, 0.f);
		const int n = (int)m_Doc->selection.size();
		const xr_string title = n > 1 ? NqUtil::Format("%d nodes selected", n) : xr_string("Nothing selected");
		const xr_string text = n > 1 ? xr_string("Select one node to edit it.")
			: NqUtil::Format("Select a node to edit it. The %s's own settings are in the Graph panel.", NqLook::DocNoun(Behaviour()));
		XUI::EmptyState(XUI::Icon::Info, title.c_str(), text.c_str());
		return;
	}

	const bool has_action = HasAction();
	float node_h = column / XUI::Dpi();
	if (has_action)
	{
		if (m_PendingAction >= 0.f) { XUI::Layout::Set("graph.action_split", m_PendingAction * m_DetailsColumn); m_PendingAction = -1.f; }
		// min_b keeps the action pane and the footer
		XUI::Splitter("graph.action_split", false, &node_h, 100.f, 100.f + (footer + spacing) / XUI::Dpi(), m_DetailsColumn * 0.5f);
	}
	const float node_px = XUI::Pt(node_h);
	const ImVec2 action_at(origin.x, origin.y + node_px + spacing);
	const float action_px = _max(1.f, column - node_px - spacing);
	m_NodePane = ImVec4(origin.x, origin.y, avail.x, has_action ? node_px : column);
	if (has_action) m_ActionPane = ImVec4(action_at.x, action_at.y, avail.x, action_px);
	m_ActionShare = has_action ? node_px / _max(1.f, column) : 1.f;

	auto node_pane = [&]()
	{
		ImGui::SetCursorScreenPos(origin);
		ImGui::BeginChild("##nq_node", ImVec2(avail.x, m_NodePane.w), ImGuiChildFlags_None);
		DrawNodeSection();
		m_NodeScroll.set(ImGui::GetScrollY(), ImGui::GetScrollMaxY());
		ImGui::EndChild();
	};
	auto action_pane = [&]()
	{
		ImGui::SetCursorScreenPos(action_at);
		ImGui::BeginChild("##nq_action", ImVec2(avail.x, action_px), ImGuiChildFlags_None);
		DrawActionSection();
		m_ActionScroll.set(ImGui::GetScrollY(), ImGui::GetScrollMaxY());
		ImGui::EndChild();
	};
	// The help footer shows what the grid drawn last had under the cursor, so the pane
	// under the cursor is drawn last.
	if (has_action && ImGui::IsMouseHoveringRect(origin, ImVec2(origin.x + avail.x, origin.y + node_px)))
	{
		action_pane();
		node_pane();
	}
	else
	{
		node_pane();
		if (has_action) action_pane();
		else m_ActionScroll.set(0.f, 0.f);
	}
	ImGui::SetCursorScreenPos(ImVec2(origin.x, origin.y + column + spacing));
	XUI::HelpFooter();
}

void NqInspector::DrawNodeSection()
{
	if (s_ScrollTo == "top")	{ ImGui::SetScrollY(0.f); s_ScrollTo.clear(); }
	const bool behaviour = Behaviour();
	const NqCatalog::SKind* k = NqCatalog::Find(m_Node.kind.c_str());
	const bool trig = NqText::IsTrigger(m_Node.kind.c_str());
	const xr_string title = NqLook::KindTitle(m_Node.kind.c_str(), behaviour);
	PaneHeader(NqLook::KindIcon(m_Node.kind.c_str()), NqLook::FamilyColor(m_Node.kind.c_str()), title.c_str(), m_Node.id.c_str());

	// what is wrong with the node, first: each line names a parameter and says what to do
	for (u32 i = 0; i < m_Doc->problems.size(); ++i)
		if (m_Doc->problems[i].node_id == m_NodeId)
			XUI::StatusText(m_Doc->problems[i].IsError() ? XUI::Tone::Error : XUI::Tone::Warning, m_Doc->problems[i].Text().c_str());

	bool ch = false;
	SGrid grid("graph_node");
	if (XUI::PropertySection("Node##nq_node", true))
	{
		// id: renamed through the document so references follow
		Help("The node's id: what links, bookmarks and other nodes name. F2 renames it");
		if (Row("ID##n"))
		{
			static char idbuf[128];
			strncpy_s(idbuf, sizeof(idbuf), m_Node.id.c_str(), _TRUNCATE);
			if (m_FocusRename) { ImGui::SetKeyboardFocusHere(); m_FocusRename = false; }
			if (ImGui::InputText("##id", idbuf, sizeof(idbuf), ImGuiInputTextFlags_EnterReturnsTrue) || ImGui::IsItemDeactivatedAfterEdit())
			{
				xr_string nid = NqUtil::Trim(idbuf);
				if (nid != m_Node.id && !nid.empty())
				{
					CommitNode();
					xr_string err;
					if (m_Doc->RenameNode(m_Node.id.c_str(), nid.c_str(), err))
					{
						m_Doc->selection.clear(); m_Doc->selection.push_back(nid);
						m_NodeId = nid; m_Node.id = nid; m_NodeRev = m_Doc->revision;
					}
					else if (!err.empty()) Msg("! [nq] rename: %s", err.c_str());
				}
			}
		}
		// kind
		{
			const xr_string desc = NqLook::KindDesc(m_Node.kind.c_str(), behaviour);
			Help(desc.empty() ? "What the node does. Changing it resets the parameters" : desc.c_str());
			xr_string kind = m_Node.kind;
			if (DrawKindCombo("Kind##n", trig ? NqCatalog::useTrigger : NqCatalog::useMain, kind) && kind != m_Node.kind)
			{
				m_Node.kind = kind;
				m_Node.params = SNqValue::Table();
				m_Node.once_set = false;
				k = NqCatalog::Find(kind.c_str());
				// par. 13.9 resets the parameters, not the wiring: an edge on a pin the
				// new kind still declares stays (next -> next, done -> done). Case pins
				// go with the cases the parameter reset just dropped.
				xr_vector<SNqPin> keep;
				for (u32 p = 0; p < m_Node.out.size(); ++p)
					if (k && k->HasPin(m_Node.out[p].first.c_str())) keep.push_back(m_Node.out[p]);
				m_Node.out = keep;
				ch = true;
			}
		}
		// once
		if (!trig && k)
		{
			bool once = m_Node.once_set ? m_Node.once : k->once_default;
			Help("Runs once: after it has run, the node is passed over. The kind decides when unset");
			Flags(m_Node.once_set ? XUI::PropModified : 0);
			if (Row("Once##n"))
			{
				// toggling back to what the kind already does drops the override, same as
				// the reset button - otherwise it stays declared and the button stays lit
				if (ImGui::Checkbox("##once", &once))
				{
					m_Node.once = once;
					m_Node.once_set = (once != k->once_default);
					ch = true;
				}
				if (m_Node.once_set && XUI::PropertyTrailing("##reset", XUI::Icon::Undo, "Back to what the kind does"))
				{
					m_Node.once_set = false;
					m_Node.once = k->once_default;
					ch = true;
				}
			}
		}
		if (k) ch |= DrawParams(k, m_Node.params, "np");
		else if (Row("Kind##unknown")) XUI::PropertyMessage(XUI::Tone::Error, "Unknown kind - not in the catalog");

		Help("For the author: the player never sees it");
		ch |= InputStr("Comment##n", m_Node.comment);
	}
	XUI::EndPropertySection();

	// conditions (all main/trigger kinds may carry them; the validator says where they matter)
	{
		const xr_string count = NqUtil::Format("%d", (int)m_Node.cond.size());
		if (XUI::PropertySection("Conditions##nq_conds", true, m_Node.cond.empty() ? nullptr : count.c_str()))
			ch |= DrawCondList(m_Node.cond, "nc", 0);
		XUI::EndPropertySection();
	}

	// the actions, in order: a row opens one under the node
	if (!trig)
	{
		const int total = int(m_Node.on_enter.size() + m_Node.on_exit.size());
		const xr_string count = NqUtil::Format("%d", total);
		// the row of a newly selected action is brought into view (DrawDetails): a collapsed section opens for it
		if (m_RevealOpen)
		{
			ImGui::GetStateStorage()->SetBool(ImGui::GetID("Actions##nq_acts"), true);
			m_RevealOpen = false;
		}
		if (XUI::PropertySection("Actions##nq_acts", true, total ? count.c_str() : nullptr))
		{
			// An action row leads to the action below, so it is a list row - the whole row one target: its
			// icon, its name, where it runs - and not a field. As a field its highlight covered the value
			// cell alone, and a name set where a value stands sat a frame's padding above its label.
			for (int pass = 0; pass < 2; ++pass)
			{
				LPCSTR slot = pass == 0 ? "enter" : "exit";
				const xr_vector<SNqAction>& v = m_Node.Slot(slot);
				for (u32 i = 0; i < v.size(); ++i)
				{
					const xr_string sel = xr_string(slot) + ":" + NqUtil::Format("%d", (int)i);
					const xr_string where = NqUtil::Format("%s %d", pass == 0 ? "On enter" : "On exit", (int)i + 1);
					const xr_string what = NqLook::KindTitle(v[i].kind.c_str(), behaviour);
					const bool selected = m_Doc->sel_slot == sel;
					XUI::NavRowSpec spec;
					spec.icon = NqLook::KindIcon(v[i].kind.c_str());
					spec.count = where.c_str();
					spec.tip = pass == 0 ? "Runs when the node is entered. Click to edit it below" : "Runs when the node is left. Click to edit it below";
					if (XUI::NavRow(("##act_" + sel).c_str(), what.c_str(), selected, spec)) m_Doc->sel_slot = sel;
					// the row's rect is known even while it is scrolled out and not drawn
					if (selected && m_RevealFrames > 0)
					{
						const float top = ImGui::GetWindowPos().y, bottom = top + ImGui::GetWindowHeight();
						const ImVec2 a = ImGui::GetItemRectMin(), b = ImGui::GetItemRectMax();
						if (a.y >= top && b.y <= bottom) m_RevealFrames = 0;
						else ImGui::SetScrollFromPosY((a.y + b.y) * 0.5f - top, 0.5f);
					}
				}
			}
			if (AddButton("Add Action"))
				XUI::Shell::OpenPopupAt("##nq_add_action_slot", ImVec2(ImGui::GetItemRectMin().x, ImGui::GetItemRectMax().y));
			if (ImGui::BeginPopup("##nq_add_action_slot"))
			{
				const ImVec2 at = ImGui::GetWindowPos();
				if (ImGui::MenuItem("On Enter...")) m_Canvas->RequestAddAction(m_NodeId.c_str(), "enter", at);
				if (ImGui::MenuItem("On Exit..."))	m_Canvas->RequestAddAction(m_NodeId.c_str(), "exit", at);
				ImGui::EndPopup();
			}
		}
		XUI::EndPropertySection();
		if (m_RevealFrames > 0) --m_RevealFrames;
	}

	// where the pins lead: read here, wired on the canvas
	if (!m_Node.out.empty())
	{
		if (XUI::PropertySection("Outputs##nq_outs", false))
			for (u32 p = 0; p < m_Node.out.size(); ++p)
			{
				xr_string tg;
				for (u32 t = 0; t < m_Node.out[p].second.size(); ++t) { if (t) tg += ", "; tg += m_Node.out[p].second[t]; }
				const xr_string label = m_Node.out[p].first + "##out";
				ImGui::PushID((int)p);
				if (Row(label.c_str())) XUI::PropertyText(tg.empty() ? "(nothing)" : tg.c_str());
				ImGui::PopID();
			}
		XUI::EndPropertySection();
	}

	if (ch) m_NodeDirty = true;
	if (s_ScrollTo == "bottom") { ImGui::SetScrollY(ImGui::GetScrollMaxY()); s_ScrollTo.clear(); }
}

void NqInspector::DrawActionSection()
{
	xr_string slot; int index;
	SplitSlot(m_Doc->sel_slot, slot, index);
	// SNqNode::Slot answers on_enter for every name it does not know, so an
	// unexpected slot would silently edit the wrong list
	if (!m_Node.SlotC(slot.c_str())) { m_Doc->sel_slot.clear(); return; }
	xr_vector<SNqAction>& v = m_Node.Slot(slot.c_str());
	if (index < 0 || index >= (int)v.size()) { m_Doc->sel_slot.clear(); return; }
	SNqAction& a = v[index];
	// the action pane scrolls on its own, so quest_view's scroll reaches it by name
	if (s_ScrollTo == "action:top") { ImGui::SetScrollY(0.f); s_ScrollTo.clear(); }
	const bool behaviour = Behaviour();

	// the header, and what can be done to the action as a whole at its right end
	const float tools = 3.f * 26.f;
	const xr_string where = NqUtil::Format("%s %d", slot == "exit" ? "On exit" : "On enter", index + 1);
	const xr_string title = NqLook::KindTitle(a.kind.c_str(), behaviour);
	const ImVec2 head = ImGui::GetCursorScreenPos();
	PaneHeader(NqLook::KindIcon(a.kind.c_str()), NqLook::FamilyColor(a.kind.c_str()), title.c_str(), where.c_str(), tools);
	const ImVec2 after = ImGui::GetCursorScreenPos();
	bool ch = false;
	ImGui::SetCursorScreenPos(ImVec2(head.x + ImGui::GetContentRegionAvail().x - XUI::Pt(tools), head.y + XUI::Pt(2.f)));
	if (XUI::IconButton("##up", XUI::Icon::MoveUp, "Move up", false, index > 0))
	{
		std::swap(v[index], v[index - 1]);
		m_Doc->sel_slot = slot + ":" + NqUtil::Format("%d", index - 1);
		ch = true;
	}
	ImGui::SameLine(0.f, XUI::Pt(2.f));
	if (XUI::IconButton("##down", XUI::Icon::MoveDown, "Move down", false, index + 1 < (int)v.size()))
	{
		std::swap(v[index], v[index + 1]);
		m_Doc->sel_slot = slot + ":" + NqUtil::Format("%d", index + 1);
		ch = true;
	}
	ImGui::SameLine(0.f, XUI::Pt(2.f));
	if (XUI::IconButton("##remove", XUI::Icon::Delete, "Remove the action (Ctrl+Z brings it back)"))
	{
		v.erase(v.begin() + index);
		m_Doc->sel_slot.clear();
		m_NodeDirty = true;
		return;
	}
	ImGui::SetCursorScreenPos(after);
	if (ch) { m_NodeDirty = true; return; }

	SGrid grid("graph_action");
	{
		const xr_string desc = NqLook::KindDesc(a.kind.c_str(), behaviour);
		Help(desc.empty() ? "What the action does. Changing it resets the parameters" : desc.c_str());
		if (m_FocusAction) { ImGui::SetKeyboardFocusHere(); m_FocusAction = false; }
		xr_string kind = a.kind;
		if (DrawKindCombo("Kind##a", NqCatalog::useExtra, kind) && kind != a.kind) { a.kind = kind; a.params = SNqValue::Table(); ch = true; }
	}
	if (const NqCatalog::SKind* k = NqCatalog::Find(a.kind.c_str()))
		ch |= DrawParams(k, a.params, "ap");
	else if (Row("Kind##unknown"))
		XUI::PropertyMessage(XUI::Tone::Error, "Unknown kind - not in the catalog");
	if (ch) m_NodeDirty = true;
	if (s_ScrollTo == "action:bottom") { ImGui::SetScrollY(ImGui::GetScrollMaxY()); s_ScrollTo.clear(); }
}

//------------------------------------------------------------------------------
// dialogs
//------------------------------------------------------------------------------
void NqInspector::DrawDialogs()
{
	DrawTaskRename();
	DrawTaskReferences();
	DrawTextEdit();
}

void NqInspector::BeginTaskRename(LPCSTR id)
{
	if (m_QuestDirty) CommitQuest();
	m_TaskRenameFrom = id ? id : "";
	m_TaskRenameTo = m_TaskRenameFrom;
	m_TaskRenameError.clear();
	m_TaskRenameOpen = true;
}

void NqInspector::DrawTaskRename()
{
	if (!XUI::BeginDialog("Rename Task###dlg.nq_task_rename", &m_TaskRenameOpen, XUI::DialogSize::S)) return;
	XUI::BeginDialogBody();
	const xr_string head = NqUtil::Format("Rename the task '%s'.", m_TaskRenameFrom.c_str());
	ImGui::TextUnformatted(head.c_str());
	ImGui::Spacing();
	ImGui::TextUnformatted("New ID");
	ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
	TextField("##new_id", m_TaskRenameTo);
	ImGui::Spacing();
	XUI::StatusText(XUI::Tone::Warning, "This changes the task's runtime and save identity.");
	const xr_string note = NqUtil::Format("Saves and running quest state recorded under '%s' are not migrated.", m_TaskRenameFrom.c_str());
	XUI::StatusText(XUI::Tone::Muted, note.c_str());
	if (!m_TaskRenameError.empty()) XUI::StatusText(XUI::Tone::Error, m_TaskRenameError.c_str());
	XUI::BeginDialogFooter();
	const xr_string to = NqUtil::Trim(m_TaskRenameTo);
	if (XUI::DialogButton("Rename", XUI::Kind::Primary, false, !to.empty() && to != m_TaskRenameFrom, "Type the new ID first"))
	{
		xr_string error;
		int updated = 0;
		if (m_Doc->RenameTask(m_TaskRenameFrom.c_str(), to.c_str(), error, updated))
		{
			m_QuestRev = u32(-1);
			m_TaskRenameFrom.clear();
			m_TaskRenameOpen = false;
		}
		else m_TaskRenameError = error;
	}
	// Cancel is the safe default: the rename reaches saves
	if (XUI::DialogButton("Cancel", XUI::Kind::Secondary, true))
	{
		m_TaskRenameFrom.clear();
		m_TaskRenameOpen = false;
	}
	XUI::EndDialog();
}

void NqInspector::BeginTaskReferences(LPCSTR id)
{
	if (m_QuestDirty) CommitQuest();
	m_TaskReferencesId = id ? id : "";
	m_TaskReferences.clear();
	m_TaskReferenceDiagnostics.clear();
	m_TaskReferencesError.clear();
	m_TaskReferencesComplete = false;
	m_TaskReferencesGeneration = 0;

	NqProjectIndex::SSnapshot snapshot;
	xr_string error;
	if (!NqProjectIndex::Snapshot(snapshot, error)) m_TaskReferencesError = error;
	else
	{
		NqReferences::SResult result;
		if (!NqReferences::Find(snapshot, "task_id", m_TaskReferencesId.c_str(), m_Doc->path.c_str(), result, error))
			m_TaskReferencesError = error;
		else
		{
			m_TaskReferences = result.references;
			m_TaskReferenceDiagnostics = result.diagnostics;
			m_TaskReferencesComplete = result.complete;
			m_TaskReferencesGeneration = result.generation;
		}
	}
	m_TaskReferencesOpen = true;
}

void NqInspector::DrawTaskReferences()
{
	if (!XUI::BeginDialog("Task References###dlg.nq_task_references", &m_TaskReferencesOpen, XUI::DialogSize::M)) return;
	XUI::BeginDialogBody();
	const xr_string head = NqUtil::Format("%d reference(s) to the task '%s'", (int)m_TaskReferences.size(), m_TaskReferencesId.c_str());
	ImGui::TextUnformatted(head.c_str());
	if (!m_TaskReferencesComplete) XUI::StatusText(XUI::Tone::Warning, "Incomplete: some content could not be proven safe.");
	if (!m_TaskReferencesError.empty()) XUI::StatusText(XUI::Tone::Error, m_TaskReferencesError.c_str());
	if (m_TaskReferences.empty())
	{
		if (m_TaskReferencesError.empty()) XUI::StatusText(XUI::Tone::Muted, "No typed references.");
	}
	else if (XUI::BeginList("##task_refs", ImVec2(0.f, 12.f * 22.f + 10.f)))
	{
		for (u32 i = 0; i < m_TaskReferences.size(); ++i)
		{
			const NqReferences::SReference& reference = m_TaskReferences[i];
			const xr_string row_id = NqUtil::Format("ref%u", i);
			const bool clicked = XUI::ListRow(row_id.c_str(), false);
			const xr_string text = NqUtil::Format("%s  \xC2\xB7  %s  \xC2\xB7  %s.%s", reference.node.c_str(), reference.slot.c_str(),
				reference.kind.c_str(), reference.param.c_str());
			XUI::TextEllipsis(text.c_str());
			if (clicked)
			{
				m_TaskReferencesOpen = false;
				m_Doc->selection.clear();
				m_Doc->selection.push_back(reference.node);
				const size_t slash = reference.slot.find('/');
				const xr_string base = slash == xr_string::npos ? reference.slot : reference.slot.substr(0, slash);
				if (base.find("enter:") == 0 || base.find("exit:") == 0) m_Doc->sel_slot = base;
				else m_Doc->sel_slot.clear();
			}
		}
	}
	if (!m_TaskReferences.empty()) XUI::EndList();
	for (u32 i = 0; i < m_TaskReferenceDiagnostics.size(); ++i)
		XUI::StatusText(XUI::Tone::Warning, m_TaskReferenceDiagnostics[i].Text().c_str());
	const xr_string status = m_TaskReferencesGeneration ? NqUtil::Format("Index generation %u", m_TaskReferencesGeneration) : xr_string();
	XUI::BeginDialogFooter(status.empty() ? nullptr : status.c_str());
	if (XUI::DialogButton("Close", XUI::Kind::Secondary, true)) m_TaskReferencesOpen = false;
	XUI::EndDialog();
}

void NqInspector::DrawTextEdit()
{
	if (!XUI::BeginDialog(s_TextEdit.code ? "Edit Lua###dlg.nq_text" : "Edit Text###dlg.nq_text", &s_TextEdit.open, XUI::DialogSize::M)) return;
	XUI::BeginDialogBody();
	{
		XUI::FontScope font(XUI::Font::Small);
		ImGui::PushStyleColor(ImGuiCol_Text, XFinedTheme::Vec4(T::TextSecondary));
		XUI::TextEllipsis(s_TextEdit.title.c_str());
		ImGui::PopStyleColor();
	}
	{
		XUI::FontScope font(s_TextEdit.code ? XUI::Font::Mono : XUI::Font::Body);
		static xr_vector<char> buf;
		const size_t need = s_TextEdit.text.size() + 16384;
		if (buf.size() < need) buf.resize(need);
		memcpy(&buf[0], s_TextEdit.text.c_str(), s_TextEdit.text.size() + 1);
		if (ImGui::IsWindowAppearing()) ImGui::SetKeyboardFocusHere();
		if (ImGui::InputTextMultiline("##text", &buf[0], buf.size(), ImVec2(ImGui::GetContentRegionAvail().x, XUI::Pt(s_TextEdit.code ? 340.f : 220.f)),
			s_TextEdit.code ? ImGuiInputTextFlags_AllowTabInput : 0))
			s_TextEdit.text = &buf[0];
	}
	const bool ok = s_TextEdit.check == "ok";
	XUI::BeginDialogFooter(s_TextEdit.check.empty() ? nullptr : (ok ? "Syntax is fine" : s_TextEdit.check.c_str()),
		s_TextEdit.check.empty() ? XUI::Tone::Muted : ok ? XUI::Tone::Success : XUI::Tone::Error);
	if (XUI::DialogButton("Apply", XUI::Kind::Primary, true))
	{
		s_TextEdit.apply = true;
		s_TextEdit.open = false;
	}
	if (s_TextEdit.code && XUI::DialogButton("Check Syntax", XUI::Kind::Secondary))
	{
		NqLua::SError err;
		s_TextEdit.check = NqLua::SyntaxCheck(s_TextEdit.text.c_str(), err) ? "ok" : NqUtil::Format("%s (%d:%d)", err.message.c_str(), err.line, err.col);
	}
	if (XUI::DialogButton("Cancel", XUI::Kind::Secondary)) s_TextEdit.open = false;
	XUI::EndDialog();
}

//------------------------------------------------------------------------------
// generic editors
//------------------------------------------------------------------------------
bool NqInspector::DrawKindCombo(LPCSTR label, u32 use_mask, xr_string& kind)
{
	if (!Row(label)) return false;
	const bool behaviour = Behaviour();
	bool changed = false;
	const xr_string preview = NqLook::KindTitle(kind.c_str(), behaviour);
	if (ImGui::BeginCombo(Hidden(label).c_str(), preview.c_str(), ImGuiComboFlags_HeightLarge))
	{
		xr_vector<const NqCatalog::SKind*> kinds;
		NqCatalog::KindsFor(use_mask, kinds);
		xr_string group;
		for (u32 i = 0; i < kinds.size(); ++i)
		{
			if (kinds[i]->group != group)
			{
				group = kinds[i]->group;
				XUI::FontScope font(XUI::Font::Small);
				ImGui::TextDisabled("%s", group.c_str());
			}
			const xr_string item = NqLook::KindTitle(kinds[i]->id.c_str(), behaviour) + "##" + kinds[i]->id;
			if (ImGui::Selectable(item.c_str(), kinds[i]->id == kind)) { kind = kinds[i]->id; changed = true; }
			const xr_string tip = kinds[i]->id + "\n" + NqLook::KindDesc(kinds[i]->id.c_str(), behaviour);
			XUI::Tip(tip.c_str());
		}
		ImGui::EndCombo();
	}
	return changed;
}

bool NqInspector::DrawParams(const NqCatalog::SKind* k, SNqValue& params, LPCSTR id_prefix)
{
	bool ch = false;
	if (!params.IsTable()) params = SNqValue::Table();
	ImGui::PushID(id_prefix);
	// fetch and the item actions draw one form of their own: a list of rows plus the
	// modifiers, instead of two interleaved spellings nobody could tell apart. Only
	// ITEM rows: a kind whose rows belong to another list (spawn.npc's NPCs) goes
	// through the typed editors like everything else.
	bool item_rows = false;
	if (k->HasRows())
		for (u32 i = 0; i < k->params.size(); ++i)
			if (k->params[i].type == "item_list") item_rows = true;
	if (k->id == "objective.fetch" || item_rows)
	{
		if (k->id == "objective.fetch")		ch |= DrawFetchParams(k, params);
		else if (k->id == "item.spawn")		ch |= DrawItemSpawnParams(k, params);
		else								ch |= DrawItemRowsParams(k, params);
	}
	else
		for (u32 i = 0; i < k->params.size(); ++i)
		{
			ImGui::PushID((int)i);
			ch |= DrawParam(k->params[i], params, id_prefix, k->id.c_str());
			ImGui::PopID();
		}
	// parameters present in the file but unknown to the catalog stay editable as raw Lua
	for (u32 i = 0; i < params.keys.size(); ++i)
	{
		if (k->Param(params.keys[i].c_str())) continue;
		ImGui::PushID(1000 + (int)i);
		Help("Not a parameter of this kind: kept as written, in Lua");
		Flags(XUI::PropInvalid);
		const xr_string label = params.keys[i] + " (unknown)";
		ch |= DrawRaw(label.c_str(), params.vals[i]);
		ImGui::PopID();
	}
	ImGui::PopID();
	return ch;
}

bool NqInspector::DrawParam(const NqCatalog::SParam& p, SNqValue& params, LPCSTR, LPCSTR kind)
{
	SNqValue* slot = params.Get(p.name.c_str());
	SNqValue v = slot ? *slot : SNqValue::Nil();
	const xr_string label = Pretty(p.name.c_str()) + "##" + p.name;
	// a parameter another form has ruled out stays visible but dead: hiding it would
	// make the alternative forms of the kind impossible to discover
	const bool ruled_out = NqCatalog::FormRuledOut(kind, p.name.c_str(), params);
	xr_string help = p.name + ": " + p.type;
	if (p.has_default) help += " (default " + p.def + ")";
	if (ruled_out) help += ". Not used: another form of " + xr_string(kind) + " is set";
	Help(help.c_str());
	if (p.required) Flags(XUI::PropRequired);
	if (ruled_out) ImGui::BeginDisabled();
	SNqValue* keep = m_ParamsCtx;
	m_ParamsCtx = &params;
	bool ch = DrawTyped(p.type.c_str(), &p, label.c_str(), v);
	m_ParamsCtx = keep;
	if (ruled_out) ImGui::EndDisabled();
	if (ch)
	{
		// Typing the catalog default back is the same statement as not writing the
		// parameter at all, so it stops being written: otherwise the asset carries
		// a redundant value and the reset icon - which means "there is an explicit
		// value here" - never goes away.
		if (v.IsNil() || EqualsDefault(p, v)) params.Erase(p.name.c_str());
		else params.Set(p.name.c_str(), v);
	}
	return ch;
}

bool NqInspector::DrawTyped(LPCSTR type, const NqCatalog::SParam* p, LPCSTR label, SNqValue& v)
{
	xr_string t = type;
	if (t == "text")				return DrawText(label, v, true);
	if (t == "lua")					return DrawLua(label, v);
	if (t == "bool")
	{
		const bool set = v.IsBool();
		bool b = set ? v.b : (p && p->has_default && p->def == "true");
		bool ch = false;
		if (set) Flags(XUI::PropModified);
		if (Row(label))
		{
			if (ImGui::Checkbox(Hidden(label).c_str(), &b)) { v = SNqValue::Bool(b); ch = true; }
			if (set && XUI::PropertyTrailing("##reset", XUI::Icon::Undo, "Back to the default")) { v = SNqValue::Nil(); ch = true; }
		}
		return ch;
	}
	if (t == "int" || t == "float")
	{
		const bool is_int = t == "int";
		double d = v.IsNumber() ? v.n : (p && p->has_default ? atof(p->def.c_str()) : 0.0);
		bool ch = false;
		if (!v.IsNil()) Flags(XUI::PropModified);
		if (Row(label))
		{
			if (is_int) { int i = (int)d; if (ImGui::DragInt(Hidden(label).c_str(), &i, 0.25f)) { d = i; ch = true; } }
			else { float f = (float)d; if (NumberField(Hidden(label).c_str(), f)) { d = f; ch = true; } }
			if (ch)
			{
				if (p && p->has_min && d < p->min) d = p->min;
				if (p && p->has_max && d > p->max) d = p->max;
				v = SNqValue::Number(d);
			}
			if (!v.IsNil() && XUI::PropertyTrailing("##reset", XUI::Icon::Undo, "Back to the default")) { v = SNqValue::Nil(); ch = true; }
		}
		return ch;
	}
	if (t == "enum" || t == "relation")
	{
		xr_vector<xr_string> items = (t == "relation") ? Items("enemy", "neutral", "friend") : (p ? p->enums : xr_vector<xr_string>());
		xr_string cur = v.IsString() ? v.s : "";
		const xr_string def = p && p->has_default ? "(default: " + p->def + ")" : "(unset)";
		if (ComboStr(label, cur, items, def.c_str())) { v = cur.empty() ? SNqValue::Nil() : SNqValue::String(cur); return true; }
		return false;
	}
	if (t == "duration")			return DrawDuration(label, v);
	if (t == "npc_ref")				return DrawNpcRef(label, v, false, false);
	if (t == "target_ref")			return DrawNpcRef(label, v, true, false);
	if (t == "object_ref" || t == "squad_ref")	return DrawObjectRef(label, v);
	if (t == "objective_id")		return DrawObjectiveId(label, v);
	if (t == "kill_target")			return DrawNpcRef(label, v, false, true);
	if (t == "place")				return DrawPlace(label, v);
	if (t == "destination")			return DrawDestination(label, v);
	if (t == "role")				return DrawWho(label, v);
	if (t == "role_or_actor")		return DrawRoleTarget(label, v, false);
	if (t == "attack_target")		return DrawRoleTarget(label, v, true);
	if (t == "role_bindings")		return DrawRoleBindings(label, v);
	if (t == "npc_list")			return DrawNpcList(label, v);
	if (t == "spawn_spec")			return DrawSpawnSpec(label, v);
	if (t == "item_list")			return DrawItemList(label, v);
	if (t == "loot_spawn")			return DrawLootSpawn(label, v);
	if (t == "cases_cond")			return DrawCases(label, v, false);
	if (t == "cases_weight")		return DrawCases(label, v, true);
	if (t == "cond_list")			return DrawCondValue(label, v, 1);
	if (t == "var_name")
	{
		// a dropdown of what the graph actually declares, not a typed-in name
		xr_string s = v.AsString("");
		if (DrawVarNameCombo(label, s)) { v = s.empty() ? SNqValue::Nil() : SNqValue::String(s); return true; }
		return false;
	}
	if (t == "value")
	{
		// `value` always sits next to the `name` it is assigned to or compared
		// with, so the variable's declared type decides the widget: a bool gets a
		// box, a number a numeric field, and nobody types 5 into a string
		if (m_ParamsCtx)
		{
			const SNqValue* nm = m_ParamsCtx->Get("name");
			if (nm && nm->IsString())
				if (const SNqVar* var = FindVar(nm->s.c_str()))
					return DrawTypedValue(label, v, var->value.type);
		}
		return DrawScalarValue(label, v);
	}
	if (t == "count_or_all")
	{
		xr_string s = v.IsNumber() ? NqUtil::Format("%d", (int)v.n) : v.AsString("");
		if (InputStr(label, s))
		{
			s = NqUtil::Trim(s);
			if (s.empty()) v = SNqValue::Nil();
			else if (s == "all") v = SNqValue::String("all");
			else v = SNqValue::Number(atof(s.c_str()));
			return true;
		}
		return false;
	}
	// strings with (or without) a picker
	xr_string s = v.AsString("");
	if (DrawPicked(label, type, s)) { v = s.empty() ? SNqValue::Nil() : SNqValue::String(s); return true; }
	return false;
}

bool NqInspector::PickerPopup(LPCSTR popup, LPCSTR type, xr_string& out)
{
	if (!ImGui::IsPopupOpen(popup)) return false;

	// The rows are gathered BEFORE the window opens. A popup auto-sizes to its
	// content, but the list lives in a child with an explicit size, so the width
	// has to be measured here - otherwise a long id with its caption is all the
	// author never gets to read.
	const xr_string t = type;
	const bool doc_scoped = (t == "task_id" || t == "var_name" || t == "ref_name" || t == "node_id" || t == "quest_id" ||
							 t == "behaviour_id" || t == "role" || t == "way_path" || t == "character_id");
	xr_vector<xr_string> labels, values, extras;
	LPCSTR note = 0;

	if (doc_scoped)
	{
		xr_vector<xr_string> ids;
		const SNqQuest& q = m_Doc->quest;
		if (t == "task_id")	for (u32 i = 0; i < q.tasks.size(); ++i) ids.push_back(q.tasks[i].id);
		if (t == "var_name") for (u32 i = 0; i < q.vars.size(); ++i) ids.push_back(q.vars[i].name);
		if (t == "node_id")	for (u32 i = 0; i < q.nodes.size(); ++i) ids.push_back(q.nodes[i].id);
		xr_vector<xr_string> ref_kinds;
		if (t == "ref_name") NqValidate::DeclaredRefsEx(q, ids, ref_kinds);
		if (t == "quest_id" || t == "behaviour_id")
		{
			if (m_ProjectQuestIdsSerial != NqProjectIndex::InvalidationSerial()) RefreshProjectQuestIds();
			ids = (t == "quest_id") ? m_ProjectQuestIds : m_ProjectBehaviourIds;
		}
		if (t == "role")	for (u32 i = 0; i < q.roles.size(); ++i) ids.push_back(q.roles[i].name);
		xr_vector<xr_string> char_notes;
		if (t == "character_id")
		{
			// the project's own NPCs, by the names the author gave them - a template
			// is shown and says why it cannot be picked
			const xr_vector<EditorCharacters::SBrief>& all = EditorCharacters::Briefs();
			for (u32 i = 0; i < all.size(); ++i)
			{
				ids.push_back(all[i].id);
				char_notes.push_back(all[i].abstract_ ? xr_string("template - cannot be spawned")
													  : (all[i].name.empty() ? all[i].file : all[i].name));
			}
		}
		if (t == "way_path")
		{
			// the way paths of the scene that is open, as they are this second
			xr_vector<NqPickers::SEntry> ways;
			NqPickers::SceneWayPaths(ways);
			for (u32 i = 0; i < ways.size(); ++i) ids.push_back(ways[i].id);
		}
		for (u32 i = 0; i < ids.size(); ++i)
		{
			if (m_Search[0] && !XUI::Match(m_Search, ids[i].c_str())) continue;
			labels.push_back(ids[i]);
			values.push_back(ids[i]);
			// a ref is just a name: without its category the list offers squads where an
			// item belongs, and nothing warns the author which is which. The category is
			// what the validator actually knows - a squad ref can come from spawn.squad
			// or from objective.kill{spawn}, so naming one node kind here would lie
			xr_string extra;
			if (i < char_notes.size()) extra = char_notes[i];
			if (i < ref_kinds.size())
			{
				if (ref_kinds[i] == "squad")		extra = "squad";
				else if (ref_kinds[i] == "object")	extra = "object";
				else if (ref_kinds[i] == "item")	extra = "item";
			}
			extras.push_back(extra);
		}
		if (labels.empty())
		{
			if (!ids.empty())				note = "Nothing matches";
			else if (t == "way_path")		note = "No way paths in the open scene - draw one with the Way tool";
			else if (t == "behaviour_id")	note = "No behaviour graphs in this project";
			else if (t == "role")			note = "This graph declares no roles";
			else if (t == "character_id")	note = "No characters in this project - Create > Character";
			else							note = Behaviour() ? "Nothing declared in this graph" : "Nothing declared in this quest";
		}
	}
	else
	{
		const NqPickers::EType pt = NqPickers::TypeFromName(type);
		if (pt == NqPickers::tCount)			note = "No index for this parameter";
		else if (!NqPickers::Available())		note = "Link a game to get suggestions";
		else
		{
			xr_vector<const NqPickers::SEntry*> found;
			NqPickers::Search(pt, m_Search, 200, found);
			for (u32 i = 0; i < found.size(); ++i)
			{
				xr_string label = found[i]->id;
				if (!found[i]->name.empty()) label += "  -  " + found[i]->name;
				labels.push_back(label);
				values.push_back(found[i]->id);
				// who they are and how many, where a smart stands, which level a
				// restrictor is on - the index knows, and a list of bare ids does not
				extras.push_back(found[i]->extra);
			}
			if (labels.empty()) note = "Nothing matches";
		}
	}

	const ImGuiStyle& st = ImGui::GetStyle();
	float w = ImGui::CalcTextSize("Nothing declared in this quest").x;
	for (u32 i = 0; i < labels.size(); ++i)
	{
		float row = ImGui::CalcTextSize(labels[i].c_str()).x;
		if (i < extras.size() && !extras[i].empty())
			row += ImGui::CalcTextSize("  ").x + ImGui::CalcTextSize(extras[i].c_str()).x;
		w = _max(w, row);
	}
	w += st.WindowPadding.x * 2.f + st.FramePadding.x * 2.f + st.ScrollbarSize;
	// wide enough to be worth opening, capped so a stray long id cannot span the
	// screen; anything past the cap stays reachable through the scrollbar
	w = _min(_max(w, XUI::Em(16.f)), XUI::Em(34.f));
	ImGui::SetNextWindowSizeConstraints(ImVec2(w, 0.f), ImVec2(w, FLT_MAX));

	bool picked = false;
	if (!ImGui::BeginPopup(popup)) return false;
	if (ImGui::IsWindowAppearing()) ImGui::SetKeyboardFocusHere();
	ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
	XUI::SearchField("##search", m_Search, sizeof(m_Search), "Search", 8.f);
	ImGui::Separator();

	const float rows = _min(labels.empty() ? 1.f : float(labels.size()), 14.f);
	ImGui::BeginChild("##list", ImVec2(-1.f, ImGui::GetTextLineHeightWithSpacing() * rows),
		false, ImGuiWindowFlags_HorizontalScrollbar);
	for (u32 i = 0; i < labels.size(); ++i)
	{
		if (ImGui::Selectable(labels[i].c_str())) { out = values[i]; picked = true; ImGui::CloseCurrentPopup(); }
		if (i < extras.size() && !extras[i].empty())
		{
			ImGui::SameLine();
			ImGui::TextDisabled("%s", extras[i].c_str());
		}
	}
	if (note) ImGui::TextDisabled("%s", note);
	ImGui::EndChild();

	ImGui::EndPopup();
	return picked;
}

bool NqInspector::DrawPicked(LPCSTR label, LPCSTR type, xr_string& s)
{
	bool ch = false;
	ImGui::PushID(label);
	// A name nothing can resolve looks exactly like a good one in a plain box. The index
	// already knows better, so the row says so while it is being typed rather than letting
	// the build ship a graph that points at an object nobody ever placed.
	const NqPickers::EType pt = NqPickers::TypeFromName(type);
	const bool unknown = !s.empty() && pt != NqPickers::tCount &&
						 !NqPickers::Index(pt).empty() && !NqPickers::Known(pt, s.c_str());
	if (unknown) Flags(XUI::PropInvalid);
	if (Row(label))
	{
		ch |= TextField("##value", s);
		// the filter is cleared here, not on the appearing frame: the list is gathered
		// before the popup opens, so a leftover query would decide its width
		if (XUI::PropertyTrailing("##pick", XUI::Icon::More, "Pick from the list"))
		{
			m_Search[0] = 0;
			if (0 == strcmp(type, "quest_id") || 0 == strcmp(type, "behaviour_id")) RefreshProjectQuestIds();
			XUI::Shell::OpenPopupAt("pick", ImVec2(ImGui::GetItemRectMin().x, ImGui::GetItemRectMax().y));
		}
		if (unknown)
		{
			const xr_string why = NqUtil::Format("Nothing named '%s' is in the %s list: place it first, or pick one that exists",
				s.c_str(), NqPickers::TypeName(pt));
			XUI::PropertyMessage(XUI::Tone::Error, why.c_str());
		}
	}
	if (PickerPopup("pick", type, s)) ch = true;
	ImGui::PopID();
	return ch;
}

void NqInspector::RefreshProjectQuestIds()
{
	// two lists, because a call names one kind: quest.activate a quest,
	// behaviour.run a behaviour graph - and neither may offer the other's
	NqDocs::ProjectIdsOfKind(false, m_ProjectQuestIds);
	NqDocs::ProjectIdsOfKind(true, m_ProjectBehaviourIds);
	m_ProjectQuestIdsSerial = NqProjectIndex::InvalidationSerial();
}

//------------------------------------------------------------------------------
// spawn.npc: who to create
//------------------------------------------------------------------------------
// One row per NPC. A row names a CHARACTER of the project - the asset, picked by
// the name its author gave it, no engine ids to look up - or a spawn section for
// anybody the game already has. Each row keeps a ref of its own, which is how
// the rest of the graph (and a behaviour it runs) points at that very NPC.
bool NqInspector::DrawNpcList(LPCSTR label, SNqValue& v)
{
	if (!v.IsTable()) v = SNqValue::Table();
	bool ch = false;
	ImGui::PushID(label);
	// said once for the list; what each field of a row does is on its help, or five
	// NPCs turn the panel into a page of prose
	MoreHelp("A row is one NPC: a project character or a spawn section");
	if (Row(label))
	{
		const xr_string count = NqUtil::Format("%d NPC%s", (int)v.arr.size(), v.arr.size() == 1 ? "" : "s");
		XUI::PropertyText(count.c_str());
	}
	int remove = -1, up = -1, down = -1;
	SNest nest;
	for (u32 i = 0; i < v.arr.size(); ++i)
	{
		SNqValue& row = v.arr[i];
		if (!row.IsTable()) row = SNqValue::Table();
		ImGui::PushID((int)i);
		const bool by_section = row.Has("section") && !row.Has("character");
		const xr_string mode = by_section ? "section" : "character";
		const xr_string head = NqUtil::Format("NPC %u###npc", i + 1);
		Help("character: an NPC of this project (Create > Character). section: any spawn section the game has");
		if (Row(head.c_str()))
		{
			// what the row names: an asset of this project, or a section of the game
			static const char* kModes[] = { "Character", "Section" };
			int cur = by_section ? 1 : 0;
			ImGui::SetNextItemWidth(ImGui::CalcItemWidth());
			if (XUI::Segmented("##kind", &cur, kModes, 2) && (cur == 1) != by_section)
			{
				const xr_string next = cur == 1 ? "section" : "character";
				// the other name goes: a row names exactly one
				row.Erase(next == "section" ? "character" : "section");
				if (!row.Has(next.c_str())) row.Set(next.c_str(), SNqValue::String(""));
				// the default of `unique` follows the mode, and only what differs from it is kept
				if (const SNqValue* u = row.Get("unique"))
					if (u->IsBool() && u->b == (next == "character")) row.Erase("unique");
				ch = true;
			}
			if (XUI::PropertyTrailing("##del", XUI::Icon::Delete, "Remove the NPC")) remove = (int)i;
			if (i + 1 < v.arr.size() && XUI::PropertyTrailing("##down", XUI::Icon::MoveDown, "Move down")) down = (int)i;
			if (i > 0 && XUI::PropertyTrailing("##up", XUI::Icon::MoveUp, "Move up")) up = (int)i;
		}
		{
			SNest inner;
			const bool section_now = row.Has("section") && !row.Has("character");
			const xr_string key = section_now ? "section" : "character";
			xr_string s = row.GetString(key.c_str());
			LPCSTR type = section_now ? "spawn_section" : "character_id";
			Help(section_now ? "The spawn section of the NPC" : "The project's character: its id");
			if (DrawPicked(section_now ? "Section##npc_who" : "Character##npc_who", type, s))
			{
				row.Erase(section_now ? "character" : "section");
				row.Set(key.c_str(), SNqValue::String(s));
				ch = true;
			}
			xr_string r = row.GetString("ref");
			Help("The name this NPC is known by from here on: { ref = ... } in a later node, a role of behaviour.run, a kill objective. Type a new name or pick one");
			if (DrawPicked("Ref##npc_ref", "ref_name", r))
			{
				if (r.empty()) row.Erase("ref"); else row.Set("ref", SNqValue::String(r));
				ch = true;
			}
			// On by default for a character - one person - and off for a section, which is
			// a kind half the zone may share. Only what differs from that default is written.
			const bool unique_def = !section_now;
			bool unique = row.GetBool("unique", unique_def);
			Help("Every NPC of this kind already in the world is removed first, so there is never a second one. On by default for a character, off for a section");
			if (CheckRow("Unique##npc", unique))
			{
				if (unique == unique_def) row.Erase("unique"); else row.Set("unique", SNqValue::Bool(unique));
				ch = true;
			}
			bool dead = row.GetBool("dead", false);
			Help("Spawned as a corpse");
			if (CheckRow("Dead##npc", dead))
			{
				if (dead) row.Set("dead", SNqValue::Bool(true)); else row.Erase("dead");
				ch = true;
			}
			float yaw = (float)row.GetNumber("yaw", 0.0);
			Help("Where it faces, in degrees");
			if (Row("Yaw##npc") && NumberField("##yaw", yaw, "%.0f", 1.f))
			{
				if (yaw == 0.f) row.Erase("yaw"); else row.Set("yaw", SNqValue::Number(yaw));
				ch = true;
			}
		}
		ImGui::PopID();
	}
	if (remove >= 0)	{ v.arr.erase(v.arr.begin() + remove); ch = true; }
	else if (up > 0)	{ std::swap(v.arr[up], v.arr[up - 1]); ch = true; }
	else if (down >= 0 && down + 1 < (int)v.arr.size()) { std::swap(v.arr[down], v.arr[down + 1]); ch = true; }
	if (AddButton("Add NPC"))
	{
		SNqValue row = SNqValue::Table();
		row.Set("character", SNqValue::String(""));
		v.Push(row);
		ch = true;
	}
	ImGui::PopID();
	return ch;
}

//------------------------------------------------------------------------------
// the role vocabulary of a behaviour graph
//------------------------------------------------------------------------------
namespace
{
	// the row of a value that takes one of several forms: the form is picked here, what it
	// needs follows on rows of its own under it
	bool ModeRow(LPCSTR label, xr_string& mode, const xr_vector<xr_string>& modes)
	{
		if (!Row(label)) return false;
		return ComboField("##mode", mode, modes, "(unset)");
	}

	LPCSTR PickTypeOf(const xr_string& mode)
	{
		if (mode == "ref")		return "ref_name";
		if (mode == "path")		return "way_path";
		if (mode == "story")	return "story_id";
		return mode.c_str();
	}
}

// Where a step sends people. A waypoint path first, because it is the best
// destination - the walker gets a route, its look points and its stops, not
// just a point. Everything else is a place the engine turns into a point.
bool NqInspector::DrawDestination(LPCSTR label, SNqValue& v)
{
	// a role is a behaviour graph's cast; a quest only sees one it has to correct
	xr_vector<xr_string> modes = Items("path", "ref", "story");
	if (Behaviour() || v.Has("role")) modes.push_back("role");
	modes.push_back("smart"); modes.push_back("restrictor"); modes.push_back("pos"); modes.push_back("actor");
	xr_string mode = ModeOf(v, modes);
	if (mode.empty() && v.IsTable() && v.Has("level")) mode = "pos";
	bool ch = false;
	ImGui::PushID(label);
	MoreHelp("path: a way path drawn with the Way tool - the best destination. ref: something the caller spawned; "
		"the step waits for it. story: an object of the level. role: wherever that role stands. smart / restrictor / pos / actor: a place");
	xr_string m = mode;
	if (ModeRow(label, m, modes))
	{
		ch = true;
		if (m.empty()) v = SNqValue::Nil();
		else
		{
			SNqValue t = SNqValue::Table();
			if (m == "pos") { t.Set("level", SNqValue::String(NqDocs::CurrentLevel())); SNqValue p = SNqValue::Table(); p.Push(SNqValue::Number(0)); p.Push(SNqValue::Number(0)); p.Push(SNqValue::Number(0)); t.Set("pos", p); }
			else if (m == "actor") t.Set("actor", SNqValue::Bool(true));
			else t.Set(m.c_str(), SNqValue::String(""));
			v = t;
		}
		mode = m;
	}
	{
		SNest nest;
		if (mode == "pos") ch |= DrawPosition(v);
		else if (!mode.empty() && mode != "actor")
		{
			xr_string s = v.GetString(mode.c_str());
			const xr_string field = Pretty(mode.c_str()) + "##dest_value";
			if (DrawPicked(field.c_str(), PickTypeOf(mode), s)) { v.Set(mode.c_str(), SNqValue::String(s)); ch = true; }
		}
	}
	ImGui::PopID();
	return ch;
}

// `who`: one role or several, picked from what the graph declares, so the author never
// types a name the graph does not have.
bool NqInspector::DrawWho(LPCSTR label, SNqValue& v)
{
	const xr_vector<SNqRole>& roles = m_Doc->quest.roles;
	xr_vector<xr_string> picked;
	if (v.IsString() && !v.s.empty())	picked.push_back(v.s);
	else if (v.IsTable())
		for (u32 i = 0; i < v.arr.size(); ++i)
			if (v.arr[i].IsString()) picked.push_back(v.arr[i].s);

	// a name the graph no longer declares stays, and says so
	xr_string stray;
	for (u32 i = 0; i < picked.size(); ++i)
	{
		bool known = false;
		for (u32 r = 0; r < roles.size(); ++r) if (roles[r].name == picked[i]) known = true;
		if (!known) { if (!stray.empty()) stray += ", "; stray += picked[i]; }
	}
	if (!stray.empty()) Flags(XUI::PropInvalid);
	bool ch = false;
	ImGui::PushID(label);
	if (Row(label))
	{
		xr_string preview;
		for (u32 i = 0; i < picked.size(); ++i) { if (i) preview += ", "; preview += picked[i]; }
		if (preview.empty()) preview = roles.empty() ? "(no roles declared)" : "(nobody)";
		if (ImGui::BeginCombo("##who", preview.c_str()))
		{
			for (u32 i = 0; i < roles.size(); ++i)
			{
				const bool on = std::find(picked.begin(), picked.end(), roles[i].name) != picked.end();
				const xr_string cap = roles[i].name + (roles[i].many ? "  (group)" : "");
				if (ImGui::MenuItem(cap.c_str(), nullptr, on, true))
				{
					if (!on)	picked.push_back(roles[i].name);
					else		picked.erase(std::remove(picked.begin(), picked.end(), roles[i].name), picked.end());
					ch = true;
				}
			}
			// a stray name can still be unticked
			for (u32 i = 0; i < picked.size(); ++i)
			{
				bool known = false;
				for (u32 r = 0; r < roles.size(); ++r) if (roles[r].name == picked[i]) known = true;
				if (known) continue;
				const xr_string cap = picked[i] + "  (not a role of this graph)";
				if (ImGui::MenuItem(cap.c_str(), nullptr, true, true)) { picked.erase(picked.begin() + i); ch = true; break; }
			}
			if (roles.empty()) ImGui::TextDisabled("Declare a role in the Graph panel first");
			ImGui::EndCombo();
		}
		if (!stray.empty())
		{
			const xr_string why = "Not a role of this graph: " + stray;
			XUI::PropertyMessage(XUI::Tone::Error, why.c_str());
		}
	}
	if (ch)
	{
		if (picked.empty())				v = SNqValue::Nil();
		else if (picked.size() == 1)	v = SNqValue::String(picked[0]);
		else
		{
			v = SNqValue::Table();
			for (u32 i = 0; i < picked.size(); ++i) v.Push(SNqValue::String(picked[i]));
		}
	}
	ImGui::PopID();
	return ch;
}

// Who a follow walks after, or who an attack is aimed at: the player, a role,
// an object - and for an attack a whole community or a profile as well.
bool NqInspector::DrawRoleTarget(LPCSTR label, SNqValue& v, bool attack)
{
	xr_vector<xr_string> modes = Items("actor", "ref", "story");
	if (Behaviour() || v.Has("role")) modes.insert(modes.begin() + 1, xr_string("role"));
	if (attack) { modes.push_back("community"); modes.push_back("profile"); }
	xr_string mode = ModeOf(v, modes);
	bool ch = false;
	ImGui::PushID(label);
	xr_string m = mode;
	if (ModeRow(label, m, modes))
	{
		ch = true;
		if (m.empty()) v = SNqValue::Nil();
		else
		{
			SNqValue t = SNqValue::Table();
			if (m == "actor")	t.Set("actor", SNqValue::Bool(true));
			else				t.Set(m.c_str(), SNqValue::String(""));
			v = t;
		}
		mode = m;
	}
	if (!mode.empty() && mode != "actor")
	{
		SNest nest;
		xr_string s = v.GetString(mode.c_str());
		const xr_string field = Pretty(mode.c_str()) + "##target_value";
		if (DrawPicked(field.c_str(), PickTypeOf(mode), s)) { v.Set(mode.c_str(), SNqValue::String(s)); ch = true; }
	}
	ImGui::PopID();
	return ch;
}

// What a caller hands the graph it runs: one row per role THAT graph declares,
// read from the asset the node names - so the author fills a form instead of
// remembering somebody else's role names.
bool NqInspector::DrawRoleBindings(LPCSTR label, SNqValue& v)
{
	bool ch = false;
	ImGui::PushID(label);

	xr_string asset = m_ParamsCtx ? m_ParamsCtx->GetString("asset", "") : xr_string();
	xr_vector<SNqRole> roles;
	xr_string found_error;
	if (!asset.empty() && asset.find('.') == xr_string::npos)
	{
		xr_vector<NqDocs::SProjectQuest> all;
		NqDocs::ProjectQuests(all);
		bool found = false;
		for (u32 i = 0; i < all.size() && !found; ++i)
			if (all[i].behaviour && all[i].readable && all[i].id == asset)
			{
				SNqQuest q;
				xr_string err;
				if (NqDocs::ReadQuest(all[i].path.c_str(), q, err)) roles = q.roles;
				else found_error = err;
				found = true;
			}
		if (!found) found_error = "No behaviour graph called '" + asset + "' in this project";
	}

	if (!found_error.empty()) Flags(XUI::PropInvalid);
	if (Row(label))
	{
		if (asset.empty())					XUI::PropertyText("Pick the asset first - its roles appear here");
		else if (!found_error.empty())		XUI::PropertyMessage(XUI::Tone::Error, found_error.c_str());
		else
		{
			const xr_string count = NqUtil::Format("%d role%s", (int)roles.size(), roles.size() == 1 ? "" : "s");
			XUI::PropertyText(count.c_str());
		}
	}
	SNest nest;
	for (u32 i = 0; i < roles.size(); ++i)
	{
		const SNqRole& r = roles[i];
		ImGui::PushID((int)i);
		SNqValue binding;
		if (v.IsTable()) if (const SNqValue* b = v.Get(r.name.c_str())) binding = *b;
		xr_string help = NqText::Preview(r.title);
		if (r.many)				help += help.empty() ? "A group" : ". A group";
		if (!r.def.IsNil())		help += help.empty() ? "Has a default" : ". Has a default";
		else if (r.optional)	help += help.empty() ? "Optional" : ". Optional";
		Help(help.c_str());
		const xr_string role_label = r.name + "##bind";
		// a group takes a squad by ref as happily as one stalker by story id
		if (DrawNpcRef(role_label.c_str(), binding, false, false))
		{
			if (!v.IsTable()) v = SNqValue::Table();
			if (binding.IsNil() || binding.IsEmpty())	v.Erase(r.name.c_str());
			else										v.Set(r.name.c_str(), binding);
			if (v.IsTable() && v.keys.empty() && v.arr.empty()) v = SNqValue::Nil();
			ch = true;
		}
		ImGui::PopID();
	}
	// a binding for a role the asset no longer declares is shown, not swallowed
	if (v.IsTable())
		for (u32 k = 0; k < v.keys.size(); ++k)
		{
			bool known = roles.empty() && !found_error.empty();
			for (u32 i = 0; i < roles.size(); ++i) if (roles[i].name == v.keys[k]) known = true;
			if (known) continue;
			if (roles.empty() && asset.empty()) continue;
			ImGui::PushID(500 + (int)k);
			const xr_string stray_label = v.keys[k] + "##stray";
			Flags(XUI::PropInvalid);
			bool drop = false;
			if (Row(stray_label.c_str()))
			{
				XUI::PropertyMessage(XUI::Tone::Warning, "The asset declares no such role");
				drop = XUI::PropertyTrailing("##drop", XUI::Icon::Delete, "Remove the binding");
			}
			ImGui::PopID();
			if (drop) { v.Erase(v.keys[k].c_str()); ch = true; break; }
		}
	ImGui::PopID();
	return ch;
}

// A text: one line is typed in its row; a longer one shows its first line and opens in a
// dialog. A text per language puts each language on a row of its own under it.
bool NqInspector::DrawText(LPCSTR label, SNqValue& v, bool multiline)
{
	bool ch = false;
	ImGui::PushID(label);
	// what the Edit Text dialog left for a field lands whether or not its row is in view
	auto taken = [&](xr_string& s) -> bool
	{
		xr_string applied;
		if (!TakeTextEdit(ImGui::GetID("##text"), applied)) return false;
		s = applied;
		return true;
	};
	auto line = [&](LPCSTR caption, xr_string& s) -> bool
	{
		bool changed = false;
		const bool lines = s.find('\n') != xr_string::npos;
		if (!lines) changed |= TextField("##text", s);
		else XUI::PropertyText(FirstLine(s).c_str());
		if ((multiline || lines) && XUI::PropertyTrailing("##edit", XUI::Icon::Rename, "Edit the whole text"))
			OpenTextEdit(ImGui::GetID("##text"), caption, s, false);
		return changed;
	};
	xr_string tmp;
	const xr_string caption = Shown(label, tmp);
	if (v.IsTable())
	{
		if (Row(label))
		{
			XUI::PropertyText("(per language)");
			if (!v.Has("eng") && XUI::PropertyTrailing("##add_eng", XUI::Icon::Plus, "Add an English text"))
			{
				v.Set("eng", SNqValue::String(""));
				ch = true;
			}
			if (XUI::PropertyTrailing("##single", XUI::Icon::Language, "One text for every language"))
			{
				const xr_string keep = NqText::Preview(v);
				v = SNqValue::String(keep);
				ch = true;
			}
		}
		if (v.IsTable())
		{
			SNest nest;
			for (u32 i = 0; i < v.keys.size(); ++i)
			{
				xr_string s = v.vals[i].AsString("");
				ImGui::PushID((int)i);
				bool edited = taken(s);
				const xr_string row_label = v.keys[i] + "##lang";
				if (Row(row_label.c_str()))
				{
					const xr_string what = caption + " (" + v.keys[i] + ")";
					edited |= line(what.c_str(), s);
				}
				if (edited) { v.vals[i] = SNqValue::String(s); ch = true; }
				ImGui::PopID();
			}
		}
	}
	else
	{
		xr_string s = v.AsString("");
		if (taken(s)) { v = SNqValue::String(s); ch = true; }
		if (Row(label))
		{
			if (line(caption.c_str(), s)) { v = SNqValue::String(s); ch = true; }
			if (XUI::PropertyTrailing("##per_lang", XUI::Icon::Language, "A text per language"))
			{
				SNqValue t = SNqValue::Table();
				t.Set("rus", SNqValue::String(s));
				v = t;
				ch = true;
			}
		}
	}
	ImGui::PopID();
	return ch;
}

bool NqInspector::DrawDuration(LPCSTR label, SNqValue& v)
{
	static const xr_vector<xr_string> units = Items("seconds", "game_minutes", "game_hours");
	xr_string unit = ModeOf(v, units);
	double num = unit.empty() ? 0.0 : v.GetNumber(unit.c_str());
	bool ch = false;
	ImGui::PushID(label);
	if (Row(label))
	{
		// the number and its unit share the value: the number takes what the unit leaves
		const float w = ImGui::CalcItemWidth();
		const float unit_w = _min(w * 0.55f, XUI::Em(8.f));
		float f = (float)num;
		ImGui::SetNextItemWidth(_max(XUI::Em(3.f), w - unit_w - ImGui::GetStyle().ItemInnerSpacing.x));
		if (NumberField("##n", f)) { num = f; ch = true; }
		ImGui::SameLine(0.f, ImGui::GetStyle().ItemInnerSpacing.x);
		ImGui::SetNextItemWidth(unit_w);
		xr_string u = unit.empty() ? "seconds" : unit;
		if (ComboField("##u", u, units)) { unit = u; ch = true; }
	}
	if (ch)
	{
		if (unit.empty()) unit = "seconds";
		SNqValue t = SNqValue::Table();
		t.Set(unit.c_str(), SNqValue::Number(num));
		v = t;
	}
	ImGui::PopID();
	return ch;
}

bool NqInspector::DrawNpcRef(LPCSTR label, SNqValue& v, bool target, bool kill)
{
	xr_vector<xr_string> modes = Items("story", "ref", "profile", "community");
	// a target names a spot as well as a creature: the runtime resolves a zone by
	// name and anchors a bare position with a restrictor it creates itself
	if (target) { modes.push_back("smart"); modes.push_back("restrictor"); modes.push_back("pos"); }
	if (kill) modes.push_back("spawn");
	xr_string mode = ModeOf(v, modes);
	if (mode.empty() && target && v.IsTable() && v.Has("pos")) mode = "pos";
	bool ch = false;
	ImGui::PushID(label);
	xr_string m = mode;
	if (ModeRow(label, m, modes))
	{
		ch = true;
		if (m.empty()) v = SNqValue::Nil();
		else
		{
			SNqValue t = SNqValue::Table();
			// no empty smart in a fresh spawn spec: it is optional, and "" is not a name
			if (m == "spawn") { SNqValue s = SNqValue::Table(); s.Set("section", SNqValue::String("")); t.Set("spawn", s); }
			else if (m == "pos")
			{
				t.Set("level", SNqValue::String(NqDocs::CurrentLevel()));
				SNqValue p = SNqValue::Table();
				for (int i = 0; i < 3; ++i) p.Push(SNqValue::Number(0));
				t.Set("pos", p);
				t.Set("radius", SNqValue::Number(5));
			}
			else t.Set(m.c_str(), SNqValue::String(""));
			v = t;
		}
		mode = m;
	}
	{
		SNest nest;
		if (mode == "pos") ch |= DrawPosition(v);
		else if (mode == "spawn")
		{
			SNqValue* sp = v.Get("spawn");
			if (sp && DrawSpawnSpec("spawn", *sp)) ch = true;
		}
		else if (!mode.empty())
		{
			xr_string s = v.GetString(mode.c_str());
			const xr_string field = Pretty(mode.c_str()) + "##value";
			if (DrawPicked(field.c_str(), PickTypeOf(mode), s)) { v.Set(mode.c_str(), SNqValue::String(s)); ch = true; }
			if (mode == "community")
			{
				xr_string lvl = v.GetString("level");
				Help("Only NPCs of the community on this level; unset: any level");
				if (DrawPicked("Level##community", "level", lvl)) { if (lvl.empty()) v.Erase("level"); else v.Set("level", SNqValue::String(lvl)); ch = true; }
			}
		}
	}
	ImGui::PopID();
	return ch;
}

// The steps of whichever task the sibling "task" parameter names. Typing an id by
// hand would be guessing, and the validator would only tell you afterwards.
bool NqInspector::DrawObjectiveId(LPCSTR label, SNqValue& v)
{
	const SNqValue* owner = m_ParamsCtx ? m_ParamsCtx->Get("task") : 0;
	// the objective nodes bind through spot_task; the task.* actions through task
	if (!owner && m_ParamsCtx) owner = m_ParamsCtx->Get("spot_task");
	const SNqTask* task = (owner && owner->IsString()) ? m_Quest.FindTask(owner->s.c_str()) : 0;
	xr_vector<xr_string> ids;
	if (task)
		for (u32 i = 0; i < task->objectives.size(); ++i) ids.push_back(task->objectives[i].id);

	xr_string cur = v.IsString() ? v.s : xr_string();
	bool ch = false;
	ImGui::PushID(label);
	if (Row(label))
	{
		if (!task)				XUI::PropertyText(owner && owner->IsString() ? "Pick a task that exists first" : "Pick the task first");
		else if (ids.empty())
		{
			const xr_string none = NqUtil::Format("The task '%s' has no objectives yet", task->id.c_str());
			XUI::PropertyText(none.c_str());
		}
		else if (ComboField("##objective", cur, ids, "(unset)"))
		{
			v = cur.empty() ? SNqValue::Nil() : SNqValue::String(cur);
			ch = true;
		}
	}
	ImGui::PopID();
	return ch;
}

// object_ref / squad_ref: one concrete object, so only the two ways to name one -
// a profile or a community would describe a kind of creature, not a thing.
bool NqInspector::DrawObjectRef(LPCSTR label, SNqValue& v)
{
	xr_vector<xr_string> modes = Items("story", "ref");
	xr_string mode = ModeOf(v, modes);
	bool ch = false;
	ImGui::PushID(label);
	xr_string m = mode;
	if (ModeRow(label, m, modes))
	{
		ch = true;
		if (m.empty()) v = SNqValue::Nil();
		else { SNqValue t = SNqValue::Table(); t.Set(m.c_str(), SNqValue::String("")); v = t; }
		mode = m;
	}
	if (!mode.empty())
	{
		SNest nest;
		xr_string s = v.GetString(mode.c_str());
		const xr_string field = Pretty(mode.c_str()) + "##value";
		if (DrawPicked(field.c_str(), PickTypeOf(mode), s))
		{
			v.Set(mode.c_str(), SNqValue::String(s));
			ch = true;
		}
	}
	ImGui::PopID();
	return ch;
}

bool NqInspector::DrawSpawnSpec(LPCSTR label, SNqValue& v)
{
	if (!v.IsTable()) v = SNqValue::Table();
	bool ch = false;
	ImGui::PushID(label);
	// Two of these rows can both name a restrictor - where they appear and where they
	// are penned in - so each says what it is for rather than what type it takes.
	xr_string s = v.GetString("section");
	Help("Which squad to create; its section decides how many and who they are");
	Flags(XUI::PropRequired);
	if (DrawPicked("Squad section##spawn", "squad_section", s)) { v.Set("section", SNqValue::String(s)); ch = true; }
	s = v.GetString("smart");
	Help("Optional: the smart that owns them. Empty and with a place set, the nearest one is used");
	// erased when empty: a written "" reads as "a smart was named" everywhere it lands
	if (DrawPicked("Smart##spawn", "smart", s)) { if (s.empty()) v.Erase("smart"); else v.Set("smart", SNqValue::String(s)); ch = true; }

	SNqValue* pl = v.Get("place");
	SNqValue place = pl ? *pl : SNqValue::Nil();
	Help("Where they appear. Empty = on the smart. A zone scatters them across it, a point puts them all on it");
	if (DrawPlace("Appear at##spawn", place))
	{
		if (place.IsNil()) v.Erase("place"); else v.Set("place", place);
		ch = true;
	}
	if (place.IsTable() && place.Has("restrictor"))
	{
		float spread = (float)v.GetNumber("spread", 1.0);
		Help("How much of the zone to spawn in: 0.5 keeps them off the edges. The zone itself does not change");
		if (Row("Spread##spawn") && ImGui::SliderFloat("##spread", &spread, 0.05f, 1.f, "%.2f"))
		{
			if (spread >= 0.999f) v.Erase("spread"); else v.Set("spread", SNqValue::Number(spread));
			ch = true;
		}
	}
	s = v.GetString("restrictor");
	Help("A zone they are penned into. They keep working, they just cannot wander off");
	if (DrawPicked("Penned in##spawn", "restrictor", s)) { if (s.empty()) v.Erase("restrictor"); else v.Set("restrictor", SNqValue::String(s)); ch = true; }
	s = v.GetString("ref");
	Help("Other nodes point at this squad by that name");
	if (DrawPicked("Ref##spawn", "ref_name", s)) { if (s.empty()) v.Erase("ref"); else v.Set("ref", SNqValue::String(s)); ch = true; }
	bool hold = v.GetBool("hold", true);
	Help("Pins the squad to its smart, so the simulation cannot send it elsewhere");
	if (CheckRow("Hold##spawn", hold)) { v.Set("hold", SNqValue::Bool(hold)); ch = true; }
	ImGui::PopID();
	return ch;
}

// The rows of "what to bring": a section with a count, or one concrete object.
// Does the row schema carry anything beyond what to make and how many? item.spawn's does;
// item.give's and item.take's rows are section and count and fold away nothing.
static bool RowsHaveState(const NqCatalog::SKind* k)
{
	if (!k) return false;
	for (u32 i = 0; i < k->row_params.size(); ++i)
		if (k->row_params[i].name != "section" && k->row_params[i].name != "count") return true;
	return false;
}

// One line for a row that carries state, so a three-row list stays readable folded.
static xr_string RowStateSummary(const SNqValue& e)
{
	xr_string s;
	const char* keys[] = { "condition", "ammo", "ammo_kind", "upgrades", "ref" };
	for (int i = 0; i < 5; ++i)
	{
		const SNqValue* v = e.Get(keys[i]);
		if (!v || v->IsNil()) continue;
		if (!s.empty()) s += ", ";
		if (0 == strcmp(keys[i], "upgrades"))
		{
			xr_vector<xr_string> list;
			NqUtil::Split(v->AsString("").c_str(), ',', list);
			s += NqUtil::Format("%d upgrade%s", (int)list.size(), list.size() == 1 ? "" : "s");
		}
		else if (0 == strcmp(keys[i], "ref"))		s += "ref " + v->AsString("");
		else if (0 == strcmp(keys[i], "ammo_kind"))	s += v->AsString("");
		else if (v->IsNumber())						s += NqUtil::Format("%s %g", keys[i], v->n);
		else										s += keys[i];
	}
	return s;
}

// The upgrades installed on the item. `upgrades` in a section names GROUPS; what
// add_upgrade takes is a group's `elements`, and an element's `effects` names further
// groups - so a plain text box asked the author for ids they had no way to look up. This
// picks from the set the section can really install, and the engine aborts on a duplicate,
// so the list is a set of checkboxes rather than free text.
bool NqInspector::DrawUpgrades(const NqItemCaps::SCaps& caps, SNqValue& v)
{
	bool ch = false;
	xr_vector<xr_string> chosen;
	NqUtil::Split(v.AsString("").c_str(), ',', chosen);
	for (u32 i = 0; i < chosen.size(); ++i) chosen[i] = NqUtil::Trim(chosen[i]);

	xr_string shown;
	for (u32 i = 0; i < chosen.size(); ++i)
	{
		if (chosen[i].empty()) continue;
		if (!shown.empty()) shown += ", ";
		shown += chosen[i];
	}
	Help("The upgrades installed on the item, picked from what its section can take");
	if (Row("Upgrades##row"))
	{
		xr_string edited = shown;
		if (TextField("##upgrades", edited))
		{
			v = NqUtil::Trim(edited).empty() ? SNqValue::Nil() : SNqValue::String(edited);
			ch = true;
		}
		if (XUI::PropertyTrailing("##pick", XUI::Icon::More, "Pick the upgrades"))
			XUI::Shell::OpenPopupAt("upgrade_pick", ImVec2(ImGui::GetItemRectMin().x, ImGui::GetItemRectMax().y));
	}
	if (ImGui::BeginPopup("upgrade_pick"))
	{
		if (caps.upgrades.empty())
			ImGui::TextDisabled(caps.known ? "This item has no upgrades" : "Link a game to list the upgrades");
		for (u32 i = 0; i < caps.upgrades.size(); ++i)
		{
			const xr_string& id = caps.upgrades[i];
			bool on = false;
			for (u32 j = 0; j < chosen.size(); ++j) if (chosen[j] == id) on = true;
			if (ImGui::Checkbox(id.c_str(), &on))
			{
				xr_string out;
				for (u32 j = 0; j < caps.upgrades.size(); ++j)
				{
					const xr_string& c = caps.upgrades[j];
					bool keep = (c == id) ? on : false;
					if (c != id)
						for (u32 kx = 0; kx < chosen.size(); ++kx) if (chosen[kx] == c) keep = true;
					if (!keep) continue;
					if (!out.empty()) out += ", ";
					out += c;
				}
				v = out.empty() ? SNqValue::Nil() : SNqValue::String(out);
				ch = true;
				break;		// `chosen` is stale now; the next frame redraws from the value
			}
		}
		ImGui::EndPopup();
	}
	return ch;
}

// The state of ONE row, filtered by what the section can actually carry. With no linked game
// or an unknown section every field is drawn: the editor must never hide a field it cannot
// judge, because a module ships sections the linked game has never heard of.
bool NqInspector::DrawRowState(const NqCatalog::SKind* k, SNqValue& e)
{
	bool ch = false;
	const xr_string sect = e.GetString("section");
	const NqItemCaps::SCaps& caps = NqItemCaps::Of(sect.c_str());
	const int count = (int)e.GetNumber("count", 1);
	for (u32 i = 0; i < k->row_params.size(); ++i)
	{
		const NqCatalog::SParam& rp = k->row_params[i];
		if (rp.name == "section" || rp.name == "count") continue;
		// a ref names one object, so it is offered only where it can mean something -
		// but a ref already written stays visible, with the error to explain it
		if (rp.name == "ref" && count > 1 && !e.Has("ref")) continue;
		const u32 need = NqItemCaps::CapOfKey(rp.name.c_str());
		if (need != NqItemCaps::capAll && caps.known && !caps.Has((NqItemCaps::ECap)need) && !e.Has(rp.name.c_str()))
			continue;
		ImGui::PushID(400 + (int)i);
		SNqValue val = e.Has(rp.name.c_str()) ? *e.Get(rp.name.c_str()) : SNqValue::Nil();
		bool one = false;
		// a field that is set but means nothing for this section stays visible - it has to
		// be deletable - and says so rather than looking like a working setting
		const bool stale = need != NqItemCaps::capAll && caps.known && !caps.Has((NqItemCaps::ECap)need);
		xr_string help = "Unset means full. condition: wear, 0 ruined. ammo: rounds loaded, or in one box. ref: one item only";
		if (stale) help = NqUtil::Format("%s does not apply to %s - %s", rp.name.c_str(), sect.c_str(), NqItemCaps::Why((NqItemCaps::ECap)need));
		Help(help.c_str());
		if (stale) Flags(XUI::PropInvalid);
		if (rp.name == "ammo_kind" && !caps.ammo_class.empty())
		{
			// the rounds this weapon takes, not every item section in the game
			xr_string cur = val.AsString("");
			if (ComboStr("Cartridge##row", cur, caps.ammo_class, "(first)"))
			{
				val = cur.empty() ? SNqValue::Nil() : SNqValue::String(cur);
				one = true;
			}
		}
		else if (rp.name == "upgrades")
			one = DrawUpgrades(caps, val);
		else
		{
			xr_string lbl = Pretty(rp.name.c_str());
			// An unset value means FULL, so that is what the field shows: the number in the
			// box is what the item will really have, not a zero that reads as "ruined".
			SNqValue shown = val;
			if (rp.name == "ammo" && val.IsNil() && caps.FullAmmo() > 0)
				shown = SNqValue::Number(caps.FullAmmo());
			if (rp.name == "ammo" && caps.known && caps.FullAmmo() > 0)
				lbl += NqUtil::Format(" / %d", caps.FullAmmo());
			lbl += "##" + rp.name;
			one = DrawTyped(rp.type.c_str(), &rp, lbl.c_str(), shown);
			if (one)
			{
				val = shown;
				// typing the full value back is the same statement as saying nothing
				if (rp.name == "ammo" && val.IsNumber() && (int)val.n == caps.FullAmmo()) val = SNqValue::Nil();
			}
		}
		if (one)
		{
			if (val.IsNil()) e.Erase(rp.name.c_str()); else e.Set(rp.name.c_str(), val);
			ch = true;
		}
		ImGui::PopID();
	}
	return ch;
}

bool NqInspector::DrawItemList(LPCSTR label, SNqValue& v, bool objects_ok, const NqCatalog::SKind* rows_of)
{
	bool ch = false;
	ImGui::PushID(label);
	const NqCatalog::SParam* row_count = rows_of ? rows_of->RowParam("count") : 0;
	const bool count_or_all = row_count && row_count->type == "count_or_all";
	const bool with_state = rows_of && RowsHaveState(rows_of);
	if (objects_ok)			Help("A row is either N items of a section, or one concrete object (ref/story id). Rows mix freely");
	else if (with_state)	Help("A row is N items of a section with its own state. An ammo section counts in boxes");
	else if (count_or_all)	Help("A row is N items of a section, or all of them. An ammo section counts in boxes");
	else					Help("A row is N items of a section. An ammo section counts in boxes");
	if (!v.IsTable()) { if (!v.IsNil()) { v = SNqValue::Table(); ch = true; } }
	if (Row(label))
	{
		const int n = v.IsTable() ? (int)v.arr.size() : 0;
		const xr_string count = NqUtil::Format("%d row%s", n, n == 1 ? "" : "s");
		XUI::PropertyText(count.c_str());
	}
	int kill = -1;
	SNest nest;
	for (u32 i = 0; v.IsTable() && i < v.arr.size(); ++i)
	{
		SNqValue& e = v.arr[i];
		if (!e.IsTable()) e = SNqValue::Table();
		ImGui::PushID((int)i);
		const xr_string head = NqUtil::Format("Row %u###row", i + 1);
		bool drop = false;
		if (e.Has("item"))
		{
			Help("One concrete object: a story id or a ref this graph created");
			ch |= DrawObjectRef(head.c_str(), e.Set("item"));
			// the row's own remove: on a row of its own, the object's rows sit under the head
			if (Row("Remove##row") && XUI::Button("Remove Row", XUI::Kind::GhostDanger)) drop = true;
		}
		else
		{
			xr_string sect = e.GetString("section");
			Help("The item section: what to bring or make");
			ImGui::PushID("sect");
			const NqPickers::EType pt = NqPickers::TypeFromName("item_section");
			const bool unknown = !sect.empty() && !NqPickers::Index(pt).empty() && !NqPickers::Known(pt, sect.c_str());
			if (unknown) Flags(XUI::PropInvalid);
			if (Row(head.c_str()))
			{
				if (TextField("##section", sect)) { e.Set("section", SNqValue::String(sect)); ch = true; }
				if (XUI::PropertyTrailing("##del", XUI::Icon::Delete, "Remove the row")) drop = true;
				if (XUI::PropertyTrailing("##pick", XUI::Icon::More, "Pick from the list"))
				{
					m_Search[0] = 0;
					XUI::Shell::OpenPopupAt("pick", ImVec2(ImGui::GetItemRectMin().x, ImGui::GetItemRectMax().y));
				}
				if (unknown) XUI::PropertyMessage(XUI::Tone::Error, "Not an item section of the linked game");
			}
			if (PickerPopup("pick", "item_section", sect)) { e.Set("section", SNqValue::String(sect)); ch = true; }
			ImGui::PopID();

			SNest inner;
			if (count_or_all)
			{
				// A take row may say "all". The default of 1 is not written.
				const SNqValue* cv = e.Get("count");
				xr_string s = !cv ? xr_string("1") : cv->IsNumber() ? NqUtil::Format("%d", (int)cv->n) : cv->AsString("");
				Help("How many, or \"all\"");
				if (InputStr("Count##il", s))
				{
					const xr_string c = NqUtil::Trim(s);
					bool digits = !c.empty();
					for (u32 ci = 0; ci < c.size(); ++ci) if (!isdigit((unsigned char)c[ci])) digits = false;
					if (c.empty() || c == "1")	e.Erase("count");
					else if (digits)			e.Set("count", SNqValue::Number(atof(c.c_str())));
					else						e.Set("count", SNqValue::String(c));	// "all", or a typo the validator names
					ch = true;
				}
			}
			else
			{
				int count = (int)e.GetNumber("count", 1);
				Help("How many");
				if (Row("Count##il") && ImGui::DragInt("##count", &count, 0.25f, 1, 100000))
				{
					if (count < 1) count = 1;
					if (count == 1) e.Erase("count"); else e.Set("count", SNqValue::Number(count));
					ch = true;
				}
			}
			// the state of this row, folded away unless it already carries some: three rows
			// with every field open is a wall nobody reads
			if (with_state)
			{
				ImGuiStorage* storage = ImGui::GetStateStorage();
				const ImGuiID key = ImGui::GetID("##state_open");
				const xr_string sum = RowStateSummary(e);
				bool open = storage->GetBool(key, !sum.empty());
				Help("The state every item of the row is made with; unset means full");
				if (Row("State##il"))
				{
					XUI::PropertyText(sum.empty() ? "full" : sum.c_str());
					if (XUI::PropertyTrailing("##state", open ? XUI::Icon::ChevronUp : XUI::Icon::ChevronDown, open ? "Hide the state" : "Edit the state"))
					{
						open = !open;
						storage->SetBool(key, open);
					}
				}
				if (open)
				{
					SNest state;
					ch |= DrawRowState(rows_of, e);
				}
			}
		}
		ImGui::PopID();
		if (drop) kill = (int)i;
	}
	if (kill >= 0) { v.arr.erase(v.arr.begin() + kill); if (v.arr.empty()) v = SNqValue::Nil(); ch = true; }
	if (AddButton("Add Section Row"))
	{
		if (!v.IsTable()) v = SNqValue::Table();
		SNqValue e = SNqValue::Table();
		e.Set("section", SNqValue::String(""));
		v.Push(e);
		ch = true;
	}
	if (objects_ok)
	{
		ImGui::SameLine();
		if (XUI::Button("Add Object", XUI::Kind::Ghost))
		{
			if (!v.IsTable()) v = SNqValue::Table();
			SNqValue e = SNqValue::Table();
			SNqValue r = SNqValue::Table();
			r.Set("story", SNqValue::String(""));
			e.Set("item", r);
			v.Push(e);
			ch = true;
		}
	}
	ImGui::PopID();
	return ch;
}

// item.spawn as ONE form: what (the rows), where (one destination), the state of
// every instance, and the name. Mirrors fetch so the two read alike.
// The list of rows of an item action. The one-line spelling (section + count, and for
// item.spawn its own state) is shown as a one-row list; the first edit rewrites it as
// `items`, untouched assets keep their spelling.
bool NqInspector::DrawItemRowsParams(const NqCatalog::SKind* k, SNqValue& params)
{
	bool ch = false;
	SNqValue rows = SNqValue::Nil();
	if (const SNqValue* it = params.Get("items")) rows = *it;
	else if (params.Has("section"))
	{
		rows = SNqValue::Table();
		SNqValue e = SNqValue::Table();
		e.Set("section", SNqValue::String(params.GetString("section")));
		if (const SNqValue* cnt = params.Get("count"))
			if (!(cnt->IsNumber() && cnt->n == 1)) e.Set("count", *cnt);
		for (u32 i = 0; k && i < k->row_params.size(); ++i)
		{
			const xr_string& rn = k->row_params[i].name;
			if (rn == "section" || rn == "count") continue;
			if (const SNqValue* sv = params.Get(rn.c_str())) e.Set(rn.c_str(), *sv);
		}
		rows.Push(e);
	}
	Flags(XUI::PropRequired);
	if (DrawItemList("Items##rows_items", rows, false, k))
	{
		ch = true;
		params.Erase("section");
		params.Erase("count");
		// whatever the rows carry moved into them; nothing of it stays on the action
		for (u32 i = 0; k && i < k->row_params.size(); ++i) params.Erase(k->row_params[i].name.c_str());
		if (rows.IsNil() || (rows.IsTable() && rows.arr.empty())) params.Erase("items");
		else params.Set("items", rows);
	}
	return ch;
}

bool NqInspector::DrawItemSpawnParams(const NqCatalog::SKind* k, SNqValue& params)
{
	bool ch = DrawItemRowsParams(k, params);
	SNqValue v = params.Has("where") ? *params.Get("where") : SNqValue::Nil();
	Help("Where they land: on the ground, inside a container or NPC, in a stash or a corpse, or in a fresh corpse");
	Flags(XUI::PropRequired);
	if (DrawLootSpawn("Where##spawn_where", v))
	{
		ch = true;
		if (v.IsNil() || v.IsEmpty()) params.Erase("where"); else params.Set("where", v);
	}
	// The state and the ref live on the rows: one list of two rifles and a medkit has three
	// different answers, and one set of fields on the action could only give it one.
	return ch;
}

// The whole fetch node as ONE form: the list of rows, then the modifiers. The three
// old spellings drew side by side and read as three different fields for the same thing.
bool NqInspector::DrawFetchParams(const NqCatalog::SKind* k, SNqValue& params)
{
	bool ch = false;

	// the one-line spelling (section+count) is shown as a one-row list; the first edit
	// rewrites it as `items`, untouched assets keep their spelling
	SNqValue rows = SNqValue::Nil();
	if (const SNqValue* it = params.Get("items")) rows = *it;
	else if (params.Has("section"))
	{
		rows = SNqValue::Table();
		SNqValue e = SNqValue::Table();
		e.Set("section", SNqValue::String(params.GetString("section")));
		const double n = params.GetNumber("count", 1);
		if (n > 1) e.Set("count", SNqValue::Number(n));
		rows.Push(e);
	}
	if (DrawItemList("Items##fetch_items", rows))
	{
		ch = true;
		params.Erase("section");
		params.Erase("count");
		if (rows.IsNil() || (rows.IsTable() && rows.arr.empty())) params.Erase("items");
		else params.Set("items", rows);
	}
	{
		SNqValue v = params.Has("from") ? *params.Get("from") : SNqValue::Nil();
		Help("Optional: credit only items taken from this container. An identical item from elsewhere does not count");
		if (DrawObjectRef("From container##fetch_from", v))
		{
			ch = true;
			if (v.IsNil() || v.IsEmpty()) params.Erase("from"); else params.Set("from", v);
		}
	}
	{
		SNqValue v = params.Has("spawn") ? *params.Get("spawn") : SNqValue::Nil();
		Help("Optional: the node creates the items on entry and credits only those instances");
		if (DrawLootSpawn("Spawn loot##fetch_spawn", v))
		{
			ch = true;
			if (v.IsNil() || v.IsEmpty()) params.Erase("spawn"); else params.Set("spawn", v);
		}
	}
	// the marker binding draws through the same generic rows as kill and reach -
	// ONE template everywhere: spot_task picker, spot_objective dropdown,
	// spot_done checkbox, all straight from the catalog
	{
		static const char* kSpot[] = { "spot_task", "spot_objective", "spot_done" };
		for (int i = 0; i < 3; ++i)
			if (const NqCatalog::SParam* sp = k ? k->Param(kSpot[i]) : 0)
			{
				ImGui::PushID(2000 + i);
				ch |= DrawParam(*sp, params, "np", k->id.c_str());
				ImGui::PopID();
			}
	}
	return ch;
}

// WHERE the node puts the loot it creates. One mode at a time, each with its own little form.
bool NqInspector::DrawLootSpawn(LPCSTR label, SNqValue& v)
{
	xr_vector<xr_string> modes = Items("place", "into", "stash", "npc", "corpse", "new_corpse");
	xr_string mode = ModeOf(v, modes);
	bool ch = false;
	ImGui::PushID(label);
	xr_string m = mode;
	if (ModeRow(label, m, modes))
	{
		ch = true;
		if (m.empty()) v = SNqValue::Nil();
		else
		{
			SNqValue t = SNqValue::Table();
			if (m == "place")
			{
				SNqValue pl = SNqValue::Table();
				pl.Set("level", SNqValue::String(NqDocs::CurrentLevel()));
				SNqValue pp = SNqValue::Table(); pp.Push(SNqValue::Number(0)); pp.Push(SNqValue::Number(0)); pp.Push(SNqValue::Number(0));
				pl.Set("pos", pp);
				t.Set("place", pl);
			}
			else if (m == "new_corpse")
			{
				SNqValue nc = SNqValue::Table();
				nc.Set("section", SNqValue::String(""));
				SNqValue pl = SNqValue::Table();
				pl.Set("level", SNqValue::String(NqDocs::CurrentLevel()));
				SNqValue pp = SNqValue::Table(); pp.Push(SNqValue::Number(0)); pp.Push(SNqValue::Number(0)); pp.Push(SNqValue::Number(0));
				pl.Set("pos", pp);
				nc.Set("place", pl);
				t.Set("new_corpse", nc);
			}
			else if (m == "stash" || m == "corpse")	t.Set(m.c_str(), SNqValue::Table());
			else									{ SNqValue r = SNqValue::Table(); r.Set("story", SNqValue::String("")); t.Set(m.c_str(), r); }
			v = t;
		}
		mode = m;
	}
	SNest nest;
	if (mode == "place")
	{
		Help("Drops the loot on the ground at the place");
		ch |= DrawPlace("Place##ls", v.Set("place"));
	}
	else if (mode == "into")
	{
		Help("A specific container: a story id (ALife\\Story ID (script) on the object) or a ref created by this graph");
		ch |= DrawObjectRef("Into object##ls", v.Set("into"));
	}
	else if (mode == "npc")
	{
		Help("Puts the loot into a live NPC inventory. profile/community match the first online one");
		ch |= DrawNpcRef("NPC##ls", v.Set("npc"), false, false);
	}
	else if (mode == "stash" || mode == "corpse")
	{
		SNqValue& sel = v.Set(mode.c_str());
		if (!sel.IsTable()) sel = SNqValue::Table();
		xr_vector<xr_string> picks = Items("random", "nearest", "farthest");
		xr_string pick = sel.GetString("pick");
		Help("Which of the matches gets the loot");
		if (ComboStr("Pick##ls_pick", pick, picks, "(default)"))
		{
			if (pick.empty()) sel.Erase("pick"); else sel.Set("pick", SNqValue::String(pick));
			ch = true;
		}
		if (mode == "stash")
		{
			xr_vector<xr_string> states = Items("any", "full", "empty");
			xr_string st = sel.GetString("state");
			Help("Full: has anything inside. Empty: holds nothing");
			if (ComboStr("State##ls_state", st, states, "(any)"))
			{
				if (st.empty()) sel.Erase("state"); else sel.Set("state", SNqValue::String(st));
				ch = true;
			}
		}
		float within = (float)sel.GetNumber("within", 0);
		Help("0 = unlimited. Nothing within it: the nearest match is used");
		if (Row("Within, m##ls_w") && NumberField("##within", within, "%.0f", 1.f))
		{
			if (within <= 0.f) sel.Erase("within"); else sel.Set("within", SNqValue::Number(within));
			ch = true;
		}
		float beyond = (float)sel.GetNumber("beyond", 0);
		Help("0 = unlimited. Nothing beyond it: the farthest match is used");
		if (Row("Beyond, m##ls_b") && NumberField("##beyond", beyond, "%.0f", 1.f))
		{
			if (beyond <= 0.f) sel.Erase("beyond"); else sel.Set("beyond", SNqValue::Number(beyond));
			ch = true;
		}
		if (mode == "stash")
		{
			bool scatter = sel.GetBool("scatter", false);
			Help("Each item goes to its own matching stash, otherwise all share one");
			if (CheckRow("Scatter##ls_sc", scatter))
			{
				if (scatter) sel.Set("scatter", SNqValue::Bool(true)); else sel.Erase("scatter");
				ch = true;
			}
		}
	}
	else if (mode == "new_corpse")
	{
		SNqValue& nc = v.Set("new_corpse");
		if (!nc.IsTable()) nc = SNqValue::Table();
		xr_string sect = nc.GetString("section");
		Help("The corpse's spawn section");
		if (DrawPicked("Corpse section##ls_ncs", "spawn_section", sect))
		{
			nc.Set("section", SNqValue::String(sect));
			ch = true;
		}
		Help("Where the fresh corpse drops. The loot goes into its inventory");
		ch |= DrawPlace("Place##ls_ncp", nc.Set("place"));
	}
	ImGui::PopID();
	return ch;
}

bool NqInspector::DrawPlace(LPCSTR label, SNqValue& v)
{
	// "ref" names an object this graph created - the only way to point at something
	// that did not exist when the graph was written; "actor" is a point relative to
	// the player, resolved at run time
	xr_vector<xr_string> modes = Items("pos", "restrictor", "smart", "ref", "actor");
	xr_string mode = ModeOf(v, modes);
	if (mode.empty() && v.IsTable() && v.Has("level")) mode = "pos";
	bool ch = false;
	ImGui::PushID(label);
	xr_string m = mode;
	if (ModeRow(label, m, modes))
	{
		ch = true;
		if (m.empty()) v = SNqValue::Nil();
		else
		{
			SNqValue t = SNqValue::Table();
			if (m == "pos") { t.Set("level", SNqValue::String(NqDocs::CurrentLevel())); SNqValue p = SNqValue::Table(); p.Push(SNqValue::Number(0)); p.Push(SNqValue::Number(0)); p.Push(SNqValue::Number(0)); t.Set("pos", p); t.Set("radius", SNqValue::Number(5)); }
			else if (m == "actor") { t.Set("actor", SNqValue::Bool(true)); t.Set("distance", SNqValue::Number(2)); }
			else t.Set(m.c_str(), SNqValue::String(""));
			v = t;
		}
		mode = m;
	}
	SNest nest;
	if (mode == "pos") ch |= DrawPosition(v);
	else if (mode == "actor")
	{
		float d = (float)v.GetNumber("distance", 0.0);
		Help("Resolved when the action runs: that many metres in front of the player, snapped to the navmesh. 0 = at their feet");
		if (Row("Ahead, m##place_actor_d") && NumberField("##distance", d, "%.1f"))
		{
			if (d <= 0.f) v.Erase("distance"); else v.Set("distance", SNqValue::Number(d));
			ch = true;
		}
	}
	else if (!mode.empty())
	{
		// The field is labelled with the mode: an unnamed box under a combo, next to a
		// second box that says "restrictor", is two blanks and a guess.
		xr_string s = v.GetString(mode.c_str());
		const xr_string field = Pretty(mode.c_str()) + "##place_value";
		if (DrawPicked(field.c_str(), PickTypeOf(mode), s)) { v.Set(mode.c_str(), SNqValue::String(s)); ch = true; }
	}
	ImGui::PopID();
	return ch;
}

// Shared by every variant that spells a spot out by hand: place{pos} and a task
// target{pos}, which the runtime anchors with a restrictor of its own.
bool NqInspector::DrawPosition(SNqValue& v)
{
	bool ch = false;
	xr_string lvl = v.GetString("level");
	Help("The level the point is on");
	if (DrawPicked("Level##pos", "level", lvl)) { v.Set("level", SNqValue::String(lvl)); ch = true; }
	SNqValue* pos = v.Get("pos");
	float xyz[3] = { 0, 0, 0 };
	if (pos && pos->IsTable()) for (u32 i = 0; i < 3 && i < pos->arr.size(); ++i) xyz[i] = (float)pos->arr[i].AsNumber();
	Help("The point, in the level's coordinates. The camera icon takes the viewport camera's position");
	if (Row("Position##pos"))
	{
		if (XUI::Vec3Field("##xyz", xyz, 0.1f))
		{
			SNqValue p = SNqValue::Table();
			for (int i = 0; i < 3; ++i) p.Push(SNqValue::Number(xyz[i]));
			v.Set("pos", p);
			ch = true;
		}
		if (XUI::PropertyTrailing("##camera", XUI::Icon::Perspective, "Use the viewport camera's position and level"))
		{
			SNqValue p = SNqValue::Table();
			p.Push(SNqValue::Number(EDevice->vCameraPosition.x));
			p.Push(SNqValue::Number(EDevice->vCameraPosition.y));
			p.Push(SNqValue::Number(EDevice->vCameraPosition.z));
			v.Set("pos", p);
			xr_string cl = NqDocs::CurrentLevel();
			if (!cl.empty()) v.Set("level", SNqValue::String(cl));
			ch = true;
		}
	}
	// The point is a sphere, not a pixel: "radius" under an x/y/z reads as decoration
	// and the author is left thinking they have to stand exactly on the spot.
	float r = (float)v.GetNumber("radius", 5.0);
	Help("How close the player has to get, in metres. 5 when not set");
	if (Row("Radius, m##pos") && NumberField("##radius", r)) { v.Set("radius", SNqValue::Number(r)); ch = true; }
	return ch;
}

bool NqInspector::DrawCases(LPCSTR label, SNqValue& v, bool weights)
{
	if (!v.IsTable()) v = SNqValue::Table();
	bool ch = false;
	ImGui::PushID(label);
	MoreHelp(weights ? "Each case is a pin; the weights decide how often each one fires" : "Each case is a pin; the first case whose conditions hold fires");
	if (Row(label))
	{
		const xr_string count = NqUtil::Format("%d case%s", (int)v.arr.size(), v.arr.size() == 1 ? "" : "s");
		XUI::PropertyText(count.c_str());
	}
	SNest nest;
	int drop = -1;
	for (u32 i = 0; i < v.arr.size(); ++i)
	{
		SNqValue& c = v.arr[i];
		if (!c.IsTable()) c = SNqValue::Table();
		ImGui::PushID((int)i);
		xr_string name = c.GetString("name");
		const xr_string head = NqUtil::Format("Case %u###case", i + 1);
		Help("The case's name: the pin it leaves the node by");
		if (Row(head.c_str()))
		{
			if (TextField("##name", name)) { c.Set("name", SNqValue::String(name)); ch = true; }
			if (XUI::PropertyTrailing("##del", XUI::Icon::Delete, "Remove the case")) drop = (int)i;
		}
		SNest inner;
		if (weights)
		{
			float w = (float)c.GetNumber("weight", 1.0);
			Help("How often this case fires against the others");
			if (Row("Weight##case") && NumberField("##weight", w)) { c.Set("weight", SNqValue::Number(w)); ch = true; }
		}
		else
		{
			// the case's cond list lives as generic values; edit through typed conds
			SNqValue* cl = c.Get("cond");
			SNqValue tmp = cl ? *cl : SNqValue::Table();
			if (DrawCondValue("cond", tmp, 1)) { c.Set("cond", tmp); ch = true; }
		}
		ImGui::PopID();
	}
	if (drop >= 0) { v.arr.erase(v.arr.begin() + drop); ch = true; }
	if (AddButton("Add Case"))
	{
		SNqValue c = SNqValue::Table();
		c.Set("name", SNqValue::String(NqUtil::Format("case%d", int(v.arr.size() + 1))));
		if (weights) c.Set("weight", SNqValue::Number(1));
		else c.Set("cond", SNqValue::Table());
		v.Push(c); ch = true;
	}
	ImGui::PopID();
	return ch;
}

bool NqInspector::DrawCondValue(LPCSTR label, SNqValue& v, int depth)
{
	// generic cond list <-> typed conds round trip
	xr_vector<SNqCond> conds;
	if (v.IsTable())
		for (u32 i = 0; i < v.arr.size(); ++i)
		{
			SNqCond c; xr_string err;
			if (NqLua::CondFromValue(v.arr[i], c, err)) conds.push_back(c);
		}
	if (!DrawCondList(conds, label, depth)) return false;
	SNqValue out = SNqValue::Table();
	for (u32 i = 0; i < conds.size(); ++i) { SNqValue cv; NqLua::CondToValue(conds[i], cv); out.Push(cv); }
	v = out;
	return true;
}

bool NqInspector::DrawCondList(xr_vector<SNqCond>& conds, LPCSTR id_prefix, int depth)
{
	bool ch = false;
	ImGui::PushID(id_prefix);
	const bool behaviour = Behaviour();
	int drop = -1;
	for (u32 i = 0; i < conds.size(); ++i)
	{
		SNqCond& c = conds[i];
		ImGui::PushID((int)i);
		const xr_string head = NqUtil::Format("%s###cond", i == 0 ? "If" : "And");
		const xr_string desc = NqLook::KindDesc(c.kind.c_str(), behaviour);
		Help(desc.empty() ? "A condition: all of them have to hold" : desc.c_str());
		if (Row(head.c_str()))
		{
			// "not" and the kind share the value: the box, then the kind in what it leaves
			const float w = ImGui::CalcItemWidth();
			if (ImGui::Checkbox("not##neg", &c.negate)) ch = true;
			XUI::Tip("Holds when the condition does not");
			ImGui::SameLine(0.f, ImGui::GetStyle().ItemInnerSpacing.x);
			const float used = ImGui::GetItemRectSize().x + ImGui::GetStyle().ItemInnerSpacing.x;
			ImGui::SetNextItemWidth(_max(XUI::Em(4.f), w - used));
			const xr_string preview = NqLook::KindTitle(c.kind.c_str(), behaviour);
			if (ImGui::BeginCombo("##kind", preview.c_str(), ImGuiComboFlags_HeightLarge))
			{
				xr_vector<const NqCatalog::SKind*> kinds;
				NqCatalog::KindsFor(NqCatalog::useCond, kinds);
				xr_string group;
				for (u32 k = 0; k < kinds.size(); ++k)
				{
					if (kinds[k]->group != group)
					{
						group = kinds[k]->group;
						XUI::FontScope font(XUI::Font::Small);
						ImGui::TextDisabled("%s", group.c_str());
					}
					const xr_string item = NqLook::KindTitle(kinds[k]->id.c_str(), behaviour) + "##" + kinds[k]->id;
					if (ImGui::Selectable(item.c_str(), kinds[k]->id == c.kind) && kinds[k]->id != c.kind)
					{
						c.kind = kinds[k]->id;
						c.params = SNqValue::Table();
						ch = true;
					}
					const xr_string tip = kinds[k]->id + "\n" + NqLook::KindDesc(kinds[k]->id.c_str(), behaviour);
					XUI::Tip(tip.c_str());
				}
				ImGui::EndCombo();
			}
			if (XUI::PropertyTrailing("##del", XUI::Icon::Delete, "Remove the condition")) drop = (int)i;
		}
		const NqCatalog::SKind* k = NqCatalog::Find(c.kind.c_str());
		if (k && !k->params.empty() && depth < 4)
		{
			SNest nest;
			ch |= DrawParams(k, c.params, "cp");
		}
		ImGui::PopID();
	}
	if (drop >= 0) { conds.erase(conds.begin() + drop); ch = true; }
	if (AddButton("Add Condition"))
	{
		SNqCond c; c.kind = "has_item"; c.params = SNqValue::Table();
		conds.push_back(c); ch = true;
	}
	ImGui::PopID();
	return ch;
}

// Lua in a row: its first line; the body is written in a dialog with a syntax check
bool NqInspector::DrawLua(LPCSTR label, SNqValue& v)
{
	bool ch = false;
	ImGui::PushID(label);
	xr_string code = v.AsString("");
	const ImGuiID target = ImGui::GetID("##code");
	xr_string applied;
	if (TakeTextEdit(target, applied)) { code = applied; v = SNqValue::String(code); ch = true; }
	if (Row(label))
	{
		XUI::PropertyText(code.empty() ? "(empty)" : FirstLine(code).c_str());
		if (XUI::PropertyTrailing("##edit", XUI::Icon::Rename, "Edit the Lua"))
		{
			xr_string tmp;
			OpenTextEdit(target, Shown(label, tmp), code, true);
		}
	}
	ImGui::PopID();
	return ch;
}

bool NqInspector::DrawScalarValue(LPCSTR label, SNqValue& v)
{
	// bool / number / string typed by content: true, false, 12.5, anything else
	xr_string s = v.IsBool() ? (v.b ? "true" : "false") : (v.IsNumber() ? NqUtil::Format("%g", v.n) : v.AsString(""));
	if (!InputStr(label, s)) return false;
	xr_string t = NqUtil::Trim(s);
	if (t == "true") v = SNqValue::Bool(true);
	else if (t == "false") v = SNqValue::Bool(false);
	else
	{
		char* end = 0;
		double d = strtod(t.c_str(), &end);
		if (!t.empty() && end && *end == 0) v = SNqValue::Number(d);
		else v = SNqValue::String(s);
	}
	return true;
}

bool NqInspector::DrawRaw(LPCSTR label, SNqValue& v)
{
	xr_string s = NqLua::WriteValue(v, 0);
	if (!InputStr(label, s)) return false;
	SNqValue nv; NqLua::SError err;
	xr_string chunk = "return " + s;
	if (NqLua::ParseValue(chunk.c_str(), (u32)chunk.size(), "raw", nv, err)) { v = nv; return true; }
	return false;
}
