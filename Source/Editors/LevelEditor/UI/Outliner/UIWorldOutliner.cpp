#include "stdafx.h"
#include "..\..\..\XrECore\Editor\EditorUiContract.h"

UIWorldOutliner* UIWorldOutliner::Form = nullptr;

namespace
{
using Token = GrizzlyTheme::ColorToken;

// pt: rows, the indent of a level and the glyph of a row
constexpr float kRowH = 22.f;
constexpr float kIndent = 14.f;
constexpr float kRowIcon = 14.f;

float Round(float v)
{
	return floorf(v + .5f);
}

// the y at which a line of the current font sits centred in a row that starts at `top`
float TextY(float top)
{
	return Round(top + (XUI::Pt(kRowH) - ImGui::GetTextLineHeight()) * .5f);
}
}

UIWorldOutliner::UIWorldOutliner()
{
	m_Signature				= 0;
	m_TargetClass			= OBJCLASS_DUMMY;
	m_TotalObjects			= 0;
	m_Dirty					= true;
	m_HierarchyView			= false;
	m_HierarchyDirty		= true;
	m_HierarchyRowsDirty	= true;
	m_HierarchySort			= -1;
	m_SelectedOnly			= false;
	m_SelectedOnlyApplied	= false;
	m_Visibility				= 0;
	m_VisibilityApplied		= 0;
	m_Sort						= 0;
	m_SortApplied				= 0;
	m_AnchorClass			= OBJCLASS_DUMMY;
	m_HierarchyAnchor.cls	= OBJCLASS_DUMMY;
	m_SelSignature			= 0;
	m_ScrollTarget			= nullptr;
	m_SkipNextScroll		= false;
	m_RenameOpen			= false;
	m_RenameFocus			= false;
	m_FiltersAnchor			= ImVec2(0.f, 0.f);
	m_MoreAnchor			= ImVec2(0.f, 0.f);
	m_Filter[0]				= 0;
	m_RenameBuf[0]			= 0;
	m_RenameError[0]		= 0;
}

UIWorldOutliner::~UIWorldOutliner()
{
}

void UIWorldOutliner::Show()
{
	if (!Form) Form = xr_new<UIWorldOutliner>();
	Form->bOpen		= true;
	Form->m_Dirty	= true;
}

void UIWorldOutliner::Close()
{
	xr_delete(Form);
}

void UIWorldOutliner::Focus(bool search)
{
	Show();
	XUI::Shell::Activate("level");
	Form->m_FocusWindow = true;
	Form->m_FocusSearch |= search;
}

void UIWorldOutliner::Update()
{
	if (!Form) return;
	Form->Draw();
	// runs even when the window is collapsed, so nothing stays queued forever
	Form->RunPending();
	if (Form->IsClosed()) Close();
}

void UIWorldOutliner::Refresh()
{
	if (Form)
	{
		Form->m_Dirty = true;
		Form->InvalidateHierarchy();
	}
}

//------------------------------------------------------------------------------
// cache
//------------------------------------------------------------------------------
// Per-class object counts folded into one value. std::list::size() is O(1), so
// this is a handful of integer ops per frame instead of a scene walk.
u32 UIWorldOutliner::SceneSignature()
{
	u32 sig = 2166136261u;
	for (SceneToolsMapPairIt it = Scene->FirstTool(); it != Scene->LastTool(); ++it)
	{
		// ListObj VERIFYs on tools that hold no objects (ai_map and friends)
		if (!it->second || it->first == OBJCLASS_DUMMY || !Scene->GetOTool(it->first)) continue;
		sig = sig * 16777619u + u32(it->first);
		sig = sig * 16777619u + u32(Scene->ListObj(it->first).size());
	}
	return sig;
}

void UIWorldOutliner::Rebuild()
{
	InvalidateHierarchy();
	// collect the qualifying tools first, so the groups can be sorted and then
	// filled in place - no vector-of-vectors copying
	xr_vector<ESceneToolBase*> tools;
	for (SceneToolsMapPairIt it = Scene->FirstTool(); it != Scene->LastTool(); ++it)
	{
		if (!it->second || it->first == OBJCLASS_DUMMY || !Scene->GetOTool(it->first)) continue;
		if (Scene->ListObj(it->first).empty()) continue;
		tools.push_back(it->second);
	}

	std::sort(tools.begin(), tools.end(), [](ESceneToolBase* a, ESceneToolBase* b)
	{
		return xr_strcmp(a->ClassName(), b->ClassName()) < 0;
	});

	m_Groups.clear();
	m_Groups.resize(tools.size());
	m_TotalObjects = 0;

	for (u32 i = 0; i < tools.size(); ++i)
	{
		SGroup& g	= m_Groups[i];
		g.cls		= tools[i]->FClassID;
		g.name		= tools[i]->ClassName();
		g.objects.clear();
		ObjectList& lst = Scene->ListObj(g.cls);
		g.objects.reserve(lst.size());
		for (ObjectIt o = lst.begin(); o != lst.end(); ++o)
			g.objects.push_back(*o);
		m_TotalObjects += int(g.objects.size());
	}

	m_Signature		= SceneSignature();
	m_TargetClass	= LTools->CurrentClassID();
	m_Dirty			= false;
	// the anchor survives a rebuild on purpose - a plain click switches the
	// active target, which rebuilds right away; DrawFlatRow range-checks it
	ApplyFilter		();
}

void UIWorldOutliner::InvalidateHierarchy()
{
	m_Hierarchy.clear();
	m_HierarchyRoots.clear();
	m_HierarchyRows.clear();
	m_HierarchyDirty = true;
	m_HierarchyRowsDirty = true;
	m_HierarchySort = -1;
}

xr_string UIWorldOutliner::HierarchyIdentityKey(ObjClassID cls, LPCSTR name)
{
	xr_string key;
	key.sprintf("%d:%s", int(cls), name);
	return key;
}

void UIWorldOutliner::BuildHierarchy()
{
	std::map<CCustomObject*, int> by_object;
	std::map<xr_string, int> by_identity;
	InvalidateHierarchy();

	for (int group = 0; group < int(m_Groups.size()); ++group)
	{
		SGroup& g = m_Groups[group];
		g.hierarchy.assign(g.objects.size(), -1);
		for (int object_index = 0; object_index < int(g.objects.size()); ++object_index)
		{
			CCustomObject* object = g.objects[object_index];
			const auto found = by_object.find(object);
			if (found != by_object.end())
			{
				g.hierarchy[object_index] = found->second;
				continue;
			}

			SHierarchyNode node = {};
			node.object = object;
			node.group = group;
			node.parent = -1;
			const int node_index = int(m_Hierarchy.size());
			m_Hierarchy.push_back(node);
			by_object[object] = node_index;
			by_identity.emplace(HierarchyIdentityKey(object->FClassID, object->GetName()), node_index);
			g.hierarchy[object_index] = node_index;
		}
	}

	for (int node_index = 0; node_index < int(m_Hierarchy.size()); ++node_index)
	{
		SHierarchyNode& node = m_Hierarchy[node_index];
		if (CCustomObject* owner = node.object->GetOwner())
		{
			const auto found = by_object.find(owner);
			if (found != by_object.end() && found->second != node_index)
				node.parent = found->second;
		}
	}

	// Corrupt owner chains must degrade to roots instead of hanging the UI.
	xr_vector<u8> state(m_Hierarchy.size(), 0);
	for (int start = 0; start < int(m_Hierarchy.size()); ++start)
	{
		if (state[start]) continue;

		xr_vector<int> path;
		int current = start;
		while (current >= 0 && !state[current])
		{
			state[current] = 1;
			path.push_back(current);
			current = m_Hierarchy[current].parent;
		}

		if (current >= 0 && state[current] == 1)
		{
			int cycle_begin = 0;
			while (path[cycle_begin] != current) ++cycle_begin;
			int break_node = path[cycle_begin];
			for (int i = cycle_begin + 1; i < int(path.size()); ++i)
				break_node = _min(break_node, path[i]);
			m_Hierarchy[break_node].parent = -1;
		}

		for (int node_index : path)
			state[node_index] = 2;
	}

	for (int node_index = 0; node_index < int(m_Hierarchy.size()); ++node_index)
	{
		const int parent = m_Hierarchy[node_index].parent;
		if (parent >= 0)
			m_Hierarchy[parent].children.push_back(node_index);
		else
			m_HierarchyRoots.push_back(node_index);
	}

	for (auto expanded = m_HierarchyExpanded.begin(); expanded != m_HierarchyExpanded.end(); )
	{
		const auto object = by_identity.find(expanded->first);
		if (object == by_identity.end())
		{
			expanded = m_HierarchyExpanded.erase(expanded);
			continue;
		}
		m_Hierarchy[object->second].expanded = true;
		++expanded;
	}

	m_HierarchyRowsDirty = true;
	m_HierarchyDirty = false;
}

void UIWorldOutliner::ApplyHierarchyFilter()
{
	if (m_HierarchyDirty) BuildHierarchy();
	for (SHierarchyNode& node : m_Hierarchy)
	{
		node.direct = false;
		node.included = false;
	}

	for (int group = 0; group < int(m_Groups.size()); ++group)
	{
		SGroup& g = m_Groups[group];
		if (ClassHidden(g.cls)) continue;
		for (int row = 0; row < int(g.shown.size()); ++row)
		{
			const int object_index = g.shown[row];
			const int node_index = g.hierarchy[object_index];
			if (node_index < 0 || m_Hierarchy[node_index].group != group) continue;
			SHierarchyNode& node = m_Hierarchy[node_index];
			node.direct = true;
			node.included = true;
		}
	}

	for (const SHierarchyNode& match : m_Hierarchy)
	{
		if (!match.direct) continue;
		int parent = match.parent;
		while (parent >= 0)
		{
			if (m_Hierarchy[parent].included) break;
			m_Hierarchy[parent].included = true;
			parent = m_Hierarchy[parent].parent;
		}
	}

	if (m_HierarchySort != m_Sort)
	{
		const auto less = [this](int lhs, int rhs)
		{
			if (m_Sort)
			{
				const int cmp = NaturalCompare(m_Hierarchy[lhs].object->GetName(),
					m_Hierarchy[rhs].object->GetName());
				if (cmp) return 1 == m_Sort ? cmp < 0 : cmp > 0;
			}
			return lhs < rhs;
		};
		std::sort(m_HierarchyRoots.begin(), m_HierarchyRoots.end(), less);
		for (SHierarchyNode& node : m_Hierarchy)
			std::sort(node.children.begin(), node.children.end(), less);
		m_HierarchySort = m_Sort;
	}

	m_HierarchyRowsDirty = true;
	BuildHierarchyRows();
}

void UIWorldOutliner::SetHierarchyExpanded(int node_index, bool expanded)
{
	SHierarchyNode& node = m_Hierarchy[node_index];
	CCustomObject* obj = node.object;
	const xr_string key = HierarchyIdentityKey(obj->FClassID, obj->GetName());
	node.expanded = expanded;
	if (expanded)
	{
		SHierarchyIdentity identity;
		identity.cls = obj->FClassID;
		identity.name = obj->GetName();
		m_HierarchyExpanded[key] = identity;
	}
	else
		m_HierarchyExpanded.erase(key);
	m_HierarchyRowsDirty = true;
}

void UIWorldOutliner::RevealHierarchyPath(CCustomObject* obj)
{
	int node_index = -1;
	for (int i = 0; i < int(m_Hierarchy.size()); ++i)
		if (m_Hierarchy[i].object == obj) { node_index = i; break; }
	if (node_index < 0 || !m_Hierarchy[node_index].direct) return;

	for (int parent = m_Hierarchy[node_index].parent; parent >= 0;
		parent = m_Hierarchy[parent].parent)
		SetHierarchyExpanded(parent, true);
}

void UIWorldOutliner::UpdateHierarchyIdentity(CCustomObject* obj, LPCSTR new_name)
{
	const xr_string old_key = HierarchyIdentityKey(obj->FClassID, obj->GetName());
	const auto expanded = m_HierarchyExpanded.find(old_key);
	if (expanded != m_HierarchyExpanded.end())
	{
		SHierarchyIdentity identity = expanded->second;
		m_HierarchyExpanded.erase(expanded);
		identity.name = new_name;
		m_HierarchyExpanded[HierarchyIdentityKey(identity.cls, new_name)] = identity;
	}
	if (m_HierarchyAnchor.cls == obj->FClassID && m_HierarchyAnchor.name.equal(obj->GetName()))
		m_HierarchyAnchor.name = new_name;
}

void UIWorldOutliner::BuildHierarchyRows()
{
	m_HierarchyRows.clear();
	const bool filtering = HierarchyFiltering();

	struct SPendingRow
	{
		int node;
		int depth;
	};
	xr_vector<SPendingRow> pending;
	for (int i = int(m_HierarchyRoots.size()) - 1; i >= 0; --i)
		pending.push_back({ m_HierarchyRoots[i], 0 });

	while (!pending.empty())
	{
		const SPendingRow item = pending.back();
		pending.pop_back();
		const SHierarchyNode& node = m_Hierarchy[item.node];
		if (!node.included) continue;
		m_HierarchyRows.push_back({ item.node, item.depth });

		bool has_included_children = false;
		for (int child : node.children)
			if (m_Hierarchy[child].included) { has_included_children = true; break; }
		if (!has_included_children || (!filtering && !node.expanded)) continue;

		for (int i = int(node.children.size()) - 1; i >= 0; --i)
			if (m_Hierarchy[node.children[i]].included)
				pending.push_back({ node.children[i], item.depth + 1 });
	}

	m_HierarchyRowsDirty = false;
}

// Unreal's search box splits on spaces and ANDs the results, treats a leading
// '-' as "must not contain", and keeps "quoted words" together. Cheap to do and
// it is what anyone who has used the engine's outliner expects to work.
void UIWorldOutliner::ParseFilter()
{
	m_Terms.clear();

	char low[sizeof(m_Filter)];
	strncpy_s(low, sizeof(low), m_Filter, _TRUNCATE);
	strlwr(low);

	for (const char* p = low; *p; )
	{
		while (*p == ' ' || *p == '\t') ++p;
		if (!*p) break;

		STerm t;
		t.exclude = false;
		// a bare '-' is someone mid-typing, not an empty exclusion
		if (*p == '-' && p[1] && p[1] != ' ') { t.exclude = true; ++p; }

		if (*p == '"')
		{
			++p;
			const char* e = strchr(p, '"');
			if (!e) e = p + xr_strlen(p);		// unclosed quote: take the rest
			t.text.assign(p, e);
			p = *e ? e + 1 : e;
		}
		else
		{
			const char* e = p;
			while (*e && *e != ' ' && *e != '\t') ++e;
			t.text.assign(p, e);
			p = e;
		}

		if (!t.text.empty()) m_Terms.push_back(t);
	}
}

bool UIWorldOutliner::NameMatches(LPCSTR name) const
{
	if (m_Terms.empty()) return true;

	// SetName lowercases, but a hand-edited name may not be
	char low[256];
	strncpy_s(low, sizeof(low), name, _TRUNCATE);
	strlwr(low);

	for (u32 i = 0; i < m_Terms.size(); ++i)
	{
		// a plain term that misses, or an excluded term that hits, both reject
		const bool hit = !!strstr(low, m_Terms[i].text.c_str());
		if (hit == m_Terms[i].exclude) return false;
	}
	return true;
}

bool UIWorldOutliner::ClassHidden(ObjClassID cls) const
{
	for (u32 i = 0; i < m_HiddenClasses.size(); ++i)
		if (m_HiddenClasses[i] == cls) return true;
	return false;
}

bool UIWorldOutliner::AnyTypeHidden() const
{
	for (const SGroup& group : m_Groups)
		if (ClassHidden(group.cls)) return true;
	return false;
}

int UIWorldOutliner::NaturalCompare(LPCSTR lhs, LPCSTR rhs)
{
	while (*lhs && *rhs)
	{
		if (isdigit(static_cast<unsigned char>(*lhs)) && isdigit(static_cast<unsigned char>(*rhs)))
		{
			const char* lhs_begin = lhs;
			const char* rhs_begin = rhs;
			while (*lhs == '0') ++lhs;
			while (*rhs == '0') ++rhs;

			const char* lhs_end = lhs;
			const char* rhs_end = rhs;
			while (isdigit(static_cast<unsigned char>(*lhs_end))) ++lhs_end;
			while (isdigit(static_cast<unsigned char>(*rhs_end))) ++rhs_end;

			const size_t lhs_digits = lhs_end - lhs;
			const size_t rhs_digits = rhs_end - rhs;
			if (lhs_digits != rhs_digits) return lhs_digits < rhs_digits ? -1 : 1;
			const int number_cmp = strncmp(lhs, rhs, lhs_digits);
			if (number_cmp) return number_cmp;

			const size_t lhs_width = lhs_end - lhs_begin;
			const size_t rhs_width = rhs_end - rhs_begin;
			if (lhs_width != rhs_width) return lhs_width < rhs_width ? -1 : 1;
			lhs = lhs_end;
			rhs = rhs_end;
			continue;
		}

		const int lhs_char = tolower(static_cast<unsigned char>(*lhs));
		const int rhs_char = tolower(static_cast<unsigned char>(*rhs));
		if (lhs_char != rhs_char) return lhs_char < rhs_char ? -1 : 1;
		++lhs;
		++rhs;
	}
	return *lhs ? 1 : (*rhs ? -1 : 0);
}

void UIWorldOutliner::ApplyFilter()
{
	for (u32 i = 0; i < m_Groups.size(); ++i)
	{
		SGroup& g = m_Groups[i];
		g.shown.clear();	// keeps the capacity, so steady state allocates nothing
		for (int k = 0; k < int(g.objects.size()); ++k)
		{
			CCustomObject* o = g.objects[k];
			if (m_SelectedOnly && !o->Selected())	continue;
			if (1 == m_Visibility && !o->Visible())	continue;
			if (2 == m_Visibility && o->Visible())	continue;
			if (!NameMatches(o->GetName()))			continue;
			g.shown.push_back(k);
		}

		if (m_Sort)
		{
			std::stable_sort(g.shown.begin(), g.shown.end(), [this, &g](int lhs, int rhs)
			{
				const int cmp = NaturalCompare(g.objects[lhs]->GetName(), g.objects[rhs]->GetName());
				return 1 == m_Sort ? cmp < 0 : cmp > 0;
			});
		}
	}

	m_FilterApplied			= m_Filter;
	m_SelectedOnlyApplied	= m_SelectedOnly;
	m_VisibilityApplied		= m_Visibility;
	m_SortApplied				= m_Sort;
	if (m_HierarchyView)
		ApplyHierarchyFilter();
	else
		m_HierarchyRowsDirty = true;
}

int UIWorldOutliner::ScanSelection(u32& sig, CCustomObject*& last) const
{
	// selected pointers folded in scene order: any change of the selected set
	// - grow, shrink or swap - lands on a different value
	int n	= 0;
	sig		= 2166136261u;
	last	= nullptr;
	for (u32 i = 0; i < m_Groups.size(); ++i)
	{
		const SGroup& g = m_Groups[i];
		for (u32 k = 0; k < g.objects.size(); ++k)
		{
			CCustomObject* o = g.objects[k];
			if (!o->Selected()) continue;
			++n;
			last = o;
			sig = sig * 16777619u + u32(size_t(o));
			sig = sig * 16777619u + u32(size_t(o) >> 32);
		}
	}
	return n;
}

//------------------------------------------------------------------------------
// filters
//------------------------------------------------------------------------------
int UIWorldOutliner::ActiveFilters() const
{
	return (m_SelectedOnly ? 1 : 0) + (m_Visibility ? 1 : 0) + (AnyTypeHidden() ? 1 : 0);
}

void UIWorldOutliner::ClearFilters()
{
	m_Filter[0]		= 0;
	m_SelectedOnly	= false;
	m_Visibility	= 0;
	m_HiddenClasses.clear();
	ParseFilter();
	ApplyFilter();
}

void UIWorldOutliner::SetHierarchyView(bool hierarchy)
{
	if (m_HierarchyView == hierarchy) return;
	m_HierarchyView = hierarchy;
	if (hierarchy)
		ApplyHierarchyFilter();
	else
		m_HierarchyRowsDirty = true;
}

bool UIWorldOutliner::GroupCollapsed(ObjClassID cls) const
{
	return std::find(m_CollapsedClasses.begin(), m_CollapsedClasses.end(), cls) != m_CollapsedClasses.end();
}

void UIWorldOutliner::SetGroupCollapsed(ObjClassID cls, bool collapsed)
{
	const auto it = std::find(m_CollapsedClasses.begin(), m_CollapsedClasses.end(), cls);
	if (collapsed && it == m_CollapsedClasses.end())
		m_CollapsedClasses.push_back(cls);
	else if (!collapsed && it != m_CollapsedClasses.end())
		m_CollapsedClasses.erase(it);
}

int UIWorldOutliner::ShowObjects(bool selected_only, bool visible)
{
	if (!Scene) return 0;
	int changed = 0;
	for (SceneToolsMapPairIt it = Scene->FirstTool(); it != Scene->LastTool(); ++it)
	{
		if (!it->second || it->first == OBJCLASS_DUMMY || !Scene->GetOTool(it->first)) continue;
		// every object shown is shown in every class too
		if (!selected_only && visible && !it->second->IsVisible())
			it->second->m_EditFlags.set(ESceneToolBase::flVisible, TRUE);
		ObjectList& lst = Scene->ListObj(it->first);
		for (ObjectIt o = lst.begin(); o != lst.end(); ++o)
		{
			CCustomObject* obj = *o;
			if ((selected_only && !obj->Selected()) || !!obj->Visible() == visible) continue;
			obj->Show(visible ? TRUE : FALSE);
			++changed;
		}
	}
	UI->RedrawScene(true);
	return changed;
}

//------------------------------------------------------------------------------
// MCP surface
//------------------------------------------------------------------------------
bool UIWorldOutliner::McpSetFilter(LPCSTR text, int selected_only, LPCSTR types,
	LPCSTR visibility, LPCSTR layout, LPCSTR sort, xr_string& err)
{
	Show();									// filtering a closed panel helps nobody
	if (!Form)	{ err = "outliner unavailable"; return false; }
	if (!Scene)	{ err = "scene unavailable"; return false; }
	if (Form->m_Dirty) Form->Rebuild();

	int next_visibility = Form->m_Visibility;
	if (visibility)
	{
		if (0 == _stricmp(visibility, "all"))			next_visibility = 0;
		else if (0 == _stricmp(visibility, "visible"))	next_visibility = 1;
		else if (0 == _stricmp(visibility, "hidden"))	next_visibility = 2;
		else
		{
			err = "visibility must be all, visible, or hidden";
			return false;
		}
	}

	bool next_hierarchy = Form->m_HierarchyView;
	if (layout)
	{
		if (0 == _stricmp(layout, "type"))				next_hierarchy = false;
		else if (0 == _stricmp(layout, "hierarchy"))	next_hierarchy = true;
		else
		{
			err = "layout must be type or hierarchy";
			return false;
		}
	}

	int next_sort = Form->m_Sort;
	if (sort)
	{
		if (0 == _stricmp(sort, "scene"))		next_sort = 0;
		else if (0 == _stricmp(sort, "az"))		next_sort = 1;
		else if (0 == _stricmp(sort, "za"))		next_sort = 2;
		else
		{
			err = "sort must be scene, az or za";
			return false;
		}
	}

	xr_vector<ObjClassID> hide;
	if (types)
	{
		// class names are matched against the cache, which may not exist yet
		if (Form->m_Groups.empty()) Form->Rebuild();

		if (types[0])
		{
			xr_string unknown;
			// every listed name has to name a group, or the caller silently
			// gets an empty tree and no idea why
			for (const char* p = types; *p; )
			{
				const char* e = strchr(p, ';');
				if (!e) e = p + xr_strlen(p);
				xr_string one; one.assign(p, e);
				while (!one.empty() && one[0] == ' ')					one.erase(0, 1);
				while (!one.empty() && one[one.size()-1] == ' ')			one.erase(one.size()-1);
				if (!one.empty())
				{
					bool known = false;
					for (u32 i = 0; i < Form->m_Groups.size() && !known; ++i)
						known = (0 == _stricmp(Form->m_Groups[i].name.c_str(), one.c_str()));
					if (!known) { if (!unknown.empty()) unknown += ", "; unknown += one; }
				}
				p = *e ? e + 1 : e;
			}
			if (!unknown.empty())
			{
				err  = "unknown object type(s): " + unknown + "; scene has: ";
				for (u32 i = 0; i < Form->m_Groups.size(); ++i)
				{
					if (i) err += ", ";
					err += Form->m_Groups[i].name;
				}
				return false;
			}

			for (u32 i = 0; i < Form->m_Groups.size(); ++i)
			{
				bool shown = false;
				for (const char* p = types; *p && !shown; )
				{
					const char* e = strchr(p, ';');
					if (!e) e = p + xr_strlen(p);
					xr_string one; one.assign(p, e);
					while (!one.empty() && one[0] == ' ')				one.erase(0, 1);
					while (!one.empty() && one[one.size()-1] == ' ')		one.erase(one.size()-1);
					shown = (0 == _stricmp(Form->m_Groups[i].name.c_str(), one.c_str()));
					p = *e ? e + 1 : e;
				}
				if (!shown) hide.push_back(Form->m_Groups[i].cls);
			}
		}
	}

	// A rejected request must not leave half of its filter applied.
	if (text)					strncpy_s(Form->m_Filter, sizeof(Form->m_Filter), text, _TRUNCATE);
	if (selected_only >= 0)		Form->m_SelectedOnly = !!selected_only;
	if (types)					Form->m_HiddenClasses = hide;
	Form->m_Visibility = next_visibility;
	Form->m_Sort = next_sort;
	Form->ParseFilter();
	Form->ApplyFilter();
	Form->SetHierarchyView(next_hierarchy);
	return true;
}

void UIWorldOutliner::McpGetFilter(xr_string& text, bool& selected_only,
								   xr_string& hidden, xr_string& visibility,
								   xr_string& layout, xr_string& sort,
								   int& shown, int& total)
{
	text.clear(); hidden.clear(); visibility = "all"; layout = "type"; sort = "scene";
	selected_only	= false;
	shown = total	= 0;
	if (!Form) return;

	text			= Form->m_Filter;
	selected_only	= Form->m_SelectedOnly;
	visibility		= 1 == Form->m_Visibility ? "visible" : (2 == Form->m_Visibility ? "hidden" : "all");
	layout			= Form->m_HierarchyView ? "hierarchy" : "type";
	sort			= 1 == Form->m_Sort ? "az" : (2 == Form->m_Sort ? "za" : "scene");
	total			= Form->m_TotalObjects;
	for (u32 i = 0; i < Form->m_Groups.size(); ++i)
	{
		if (Form->ClassHidden(Form->m_Groups[i].cls))
		{
			if (!hidden.empty()) hidden += ";";
			hidden += Form->m_Groups[i].name;
			continue;
		}
		shown += int(Form->m_Groups[i].shown.size());
	}
}

//------------------------------------------------------------------------------
// selection / focus
//------------------------------------------------------------------------------
void UIWorldOutliner::PickObject(CCustomObject* obj)
{
	Scene->SelectObjects(false, OBJCLASS_DUMMY);
	obj->Select(TRUE);
	if (LTools->CurrentClassID() != obj->FClassID)
		LTools->SetTarget(obj->FClassID, 0);
	// the row is already under the cursor - recentring it would yank the list
	if (Form) Form->m_SkipNextScroll = true;
	UI->RedrawScene(true);
}

void UIWorldOutliner::FocusObject(CCustomObject* obj)
{
	Fbox box;
	if (!obj->GetBox(box))
	{
		const Fvector& p = obj->GetPosition();
		box.set(p, p);
		box.grow(2.f);
	}
	EDevice->m_Camera.ZoomExtents(box);
	UI->RedrawScene(true);
}

//------------------------------------------------------------------------------
// deferred scene edits
//------------------------------------------------------------------------------
void UIWorldOutliner::RunPending()
{
	if (!Scene || !m_PendingDeleteName.size()) return;

	const shared_str name = m_PendingDeleteName;
	m_PendingDeleteName = "";
	CCustomObject* obj = Scene->FindObjectByName(name.c_str(), static_cast<CCustomObject*>(nullptr));
	if (!obj) return;

	if (Scene->locked() || !obj->FParentTools || !obj->FParentTools->IsEditable() || !obj->Editable())
	{
		ELog.DlgMsg(mtError, "The object '%s' is read only.", obj->GetName());
		return;
	}

	Scene->SelectObjects(false, OBJCLASS_DUMMY);
	obj->Select(TRUE);
	const ObjClassID cls = obj->FClassID;
	if (LTools->CurrentClassID() != cls)
		LTools->SetTarget(cls, 0);

	// SetTarget is deferred to the next frame, so CurrentClassID() - the filter
	// COMMAND_DELETE_SELECTION uses - may still be the old class. Take the
	// command when it already matches, otherwise run its exact body.
	if (LTools->CurrentClassID() == cls)
	{
		ExecCommand(COMMAND_DELETE_SELECTION);
	}
	else
	{
		Scene->RemoveSelection(cls);
		Scene->UndoSave();
	}

	m_Dirty = true;
	UI->RedrawScene(true);
}

bool UIWorldOutliner::ApplyRename()
{
	m_RenameError[0] = 0;
	if (!m_RenameOriginal.size()) return true;
	CCustomObject* obj = Scene
		? Scene->FindObjectByName(m_RenameOriginal.c_str(), static_cast<CCustomObject*>(nullptr)) : nullptr;
	if (!obj)
	{
		strcpy_s(m_RenameError, "The object no longer exists.");
		return false;
	}
	if (Scene->locked() || !obj->FParentTools || !obj->FParentTools->IsEditable() || !obj->Editable())
	{
		strcpy_s(m_RenameError, "The object is read only.");
		return false;
	}

	char low[sizeof(m_RenameBuf)];
	strncpy_s(low, sizeof(low), m_RenameBuf, _TRUNCATE);
	strlwr(low);
	if (!low[0])
	{
		strcpy_s(m_RenameError, "Enter a name.");
		return false;
	}
	// the pass_object overload ignores the object being renamed
	if (Scene->FindObjectByName(low, obj))
	{
		strcpy_s(m_RenameError, "Another object has this name.");
		return false;
	}

	if (m_AnchorName.equal(m_RenameOriginal)) m_AnchorName = low;
	UpdateHierarchyIdentity(obj, low);
	obj->SetName(low);
	Scene->UndoSave();
	ExecCommand(COMMAND_UPDATE_PROPERTIES);
	m_Dirty = true;
	return true;
}

void UIWorldOutliner::AskDelete(CCustomObject* obj)
{
	const shared_str name = obj->GetName();
	XUI::ConfirmSpec spec;
	spec.id = "delete_object";
	spec.title = "Delete Object";
	spec.headline = "Delete this object from the scene?";
	spec.items.push_back(obj->GetName());
	spec.consequence = "Edit > Undo brings it back.";
	spec.action = "Delete";
	spec.destructive = true;
	XUI::Confirm(spec, [name](XUI::Answer answer)
	{
		// the panel may have closed while the question was up
		if (answer == XUI::Answer::Yes && Form)
			Form->m_PendingDeleteName = name;
	});
}

//------------------------------------------------------------------------------
// ui
//------------------------------------------------------------------------------
void UIWorldOutliner::HandleRowSelection(SGroup& g, int row, CCustomObject* obj)
{
	const ImGuiIO& io = ImGui::GetIO();
	if (ImGui::IsMouseDoubleClicked(ImGuiMouseButton_Left))
	{
		PickObject(obj);
		FocusObject(obj);
		m_AnchorClass = g.cls; m_AnchorName = obj->GetName();
	}
	else if (io.KeyShift && row >= 0 && m_AnchorClass == g.cls && m_AnchorName.size())
	{
		int anchor = -1;
		for (int i = 0; i < int(g.shown.size()); ++i)
			if (m_AnchorName.equal(g.objects[g.shown[i]]->GetName())) { anchor = i; break; }
		if (anchor >= 0)
		{
			const int a = _min(anchor, row);
			const int b = _max(anchor, row);
			for (int i = a; i <= b; ++i)
				g.objects[g.shown[i]]->Select(TRUE);
			m_SkipNextScroll = true;
			UI->RedrawScene(true);
		}
		else
		{
			PickObject(obj);
			m_AnchorName = obj->GetName();
		}
	}
	else if (io.KeyCtrl)
	{
		obj->Select(obj->Selected() ? FALSE : TRUE);
		m_AnchorClass = g.cls; m_AnchorName = obj->GetName();
		m_SkipNextScroll = true;
		UI->RedrawScene(true);
	}
	else
	{
		PickObject(obj);
		m_AnchorClass = g.cls; m_AnchorName = obj->GetName();
	}
}

void UIWorldOutliner::HandleHierarchySelection(int row, CCustomObject* obj)
{
	const auto set_anchor = [this](CCustomObject* anchor)
	{
		m_HierarchyAnchor.cls = anchor->FClassID;
		m_HierarchyAnchor.name = anchor->GetName();
	};
	const ImGuiIO& io = ImGui::GetIO();
	if (ImGui::IsMouseDoubleClicked(ImGuiMouseButton_Left))
	{
		PickObject(obj);
		FocusObject(obj);
		set_anchor(obj);
	}
	else if (io.KeyShift && m_HierarchyAnchor.name.size())
	{
		int anchor_row = -1;
		for (int i = 0; i < int(m_HierarchyRows.size()); ++i)
		{
			CCustomObject* candidate = m_Hierarchy[m_HierarchyRows[i].node].object;
			if (candidate->FClassID == m_HierarchyAnchor.cls &&
				m_HierarchyAnchor.name.equal(candidate->GetName()))
			{
				anchor_row = i;
				break;
			}
		}
		if (anchor_row >= 0)
		{
			const int first = _min(anchor_row, row);
			const int last = _max(anchor_row, row);
			Scene->SelectObjects(false, OBJCLASS_DUMMY);
			// Suppress group cascades without bypassing other object-specific selection contracts.
			for (int i = first; i <= last; ++i)
			{
				CCustomObject* candidate = m_Hierarchy[m_HierarchyRows[i].node].object;
				if (!candidate->Visible()) continue;
				if (candidate->FClassID == OBJCLASS_GROUP)
					candidate->CCustomObject::Select(TRUE);
				else
					candidate->Select(TRUE);
			}
			m_SkipNextScroll = true;
			UI->RedrawScene(true);
		}
		else
		{
			PickObject(obj);
			set_anchor(obj);
		}
	}
	else if (io.KeyCtrl)
	{
		obj->Select(obj->Selected() ? FALSE : TRUE);
		set_anchor(obj);
		m_SkipNextScroll = true;
		UI->RedrawScene(true);
	}
	else
	{
		PickObject(obj);
		set_anchor(obj);
	}
}

void UIWorldOutliner::DrawObjectContext(SGroup& g, CCustomObject* obj)
{
	if (!ImGui::BeginPopupContextItem("ctx")) return;
	const bool cur_vis = !!obj->Visible();
	if (XUI::MenuItem("Focus"))
	{
		PickObject(obj);
		FocusObject(obj);
	}
	if (XUI::MenuItem("Select All of This Class"))
	{
		Scene->SelectObjects(false, OBJCLASS_DUMMY);
		Scene->SelectObjects(true, g.cls);
		if (LTools->CurrentClassID() != g.cls)
			LTools->SetTarget(g.cls, 0);
		m_SkipNextScroll = true;
		UI->RedrawScene(true);
	}
	if (XUI::MenuItem(cur_vis ? "Hide" : "Show"))
	{
		obj->Show(cur_vis ? FALSE : TRUE);
		UI->RedrawScene(true);
	}
	ImGui::Separator();
	const bool can_edit = !Scene->locked() && obj->Editable() && obj->FParentTools &&
		!!obj->FParentTools->IsEditable();
	if (XUI::MenuItem("Rename...", nullptr, false, can_edit))
	{
		m_RenameOriginal = obj->GetName();
		strncpy_s(m_RenameBuf, sizeof(m_RenameBuf), obj->GetName(), _TRUNCATE);
		m_RenameError[0]	= 0;
		m_RenameOpen		= true;
		m_RenameFocus		= true;
	}
	if (XUI::MenuItem("Delete...", XUI::Icon::None, nullptr, can_edit, true))
		AskDelete(obj);
	ImGui::EndPopup();
}

void UIWorldOutliner::DrawObjectEye(CCustomObject* obj)
{
	const bool visible = !!obj->Visible();
	if (XUI_LocalEyeToggle("##vis", visible, visible ? "Hide in the viewport" : "Show in the viewport", kRowH))
	{
		obj->Show(visible ? FALSE : TRUE);
		UI->RedrawScene(true);
	}
}

void UIWorldOutliner::DrawObjectCells(CCustomObject* obj, float x, float top, float cell_right, bool dim)
{
	ImDrawList* dl = ImGui::GetWindowDrawList();
	XUI::DrawIcon(dl, ToolClassIcon(obj->FClassID), ImVec2(Round(x), Round(top + XUI::Pt(kRowH - kRowIcon) * .5f)), kRowIcon,
		GrizzlyTheme::U32(dim ? Token::TextDisabled : Token::TextSecondary));
	x += XUI::Pt(kRowIcon + 6.f);
	ImGui::SetCursorScreenPos(ImVec2(x, TextY(top)));
	ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(dim ? Token::TextSecondary : Token::Text));
	XUI::TextEllipsis(obj->GetName(), _max(0.f, cell_right - x));
	ImGui::PopStyleColor();

	ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(Token::TextSecondary));
	if (ImGui::TableSetColumnIndex(2))
	{
		ImGui::SetCursorScreenPos(ImVec2(ImGui::GetCursorScreenPos().x, TextY(top)));
		XUI::TextEllipsis(ToolClassTitle(obj->FClassID));
	}
	if (ImGui::TableSetColumnIndex(3))
		if (CCustomObject* owner = obj->GetOwner())
		{
			ImGui::SetCursorScreenPos(ImVec2(ImGui::GetCursorScreenPos().x, TextY(top)));
			XUI::TextEllipsis(owner->GetName());
		}
	ImGui::PopStyleColor();
}

void UIWorldOutliner::DrawFlatRow(SGroup& g, int row, CCustomObject* obj)
{
	const float row_h = XUI::Pt(kRowH);
	ImGui::TableNextRow(ImGuiTableRowFlags_None, row_h);
	ImGui::PushID(obj);

	ImGui::TableSetColumnIndex(0);
	DrawObjectEye(obj);

	ImGui::TableSetColumnIndex(1);
	const ImVec2 cell = ImGui::GetCursorScreenPos();
	const float right = cell.x + ImGui::GetContentRegionAvail().x;
	// the name drawn over the row keeps its own tooltip when it is cut
	ImGui::SetNextItemAllowOverlap();
	if (ImGui::Selectable("##row", !!obj->Selected(),
		ImGuiSelectableFlags_AllowDoubleClick | ImGuiSelectableFlags_SpanAllColumns, ImVec2(0.f, row_h)))
		HandleRowSelection(g, row, obj);
	DrawObjectContext(g, obj);

	ESceneToolBase* tool = Scene->GetTool(g.cls);
	// under the class's icon: the chevron's width in
	DrawObjectCells(obj, cell.x + XUI::Pt(16.f), cell.y, right, tool && !tool->IsVisible());
	ImGui::PopID();
}

void UIWorldOutliner::ScrollRowIntoView(float row_top, float row_h)
{
	// the frozen header row covers the top of the view
	const float view_top	= ImGui::GetScrollY() + row_h;
	const float view_h		= ImGui::GetWindowHeight() - row_h;

	// Already fully on screen: leave it EXACTLY where it is. Unreal checks the
	// same thing before scrolling (SListView::ScrollIntoView, "Only scroll the
	// item into view if it's not already in the visible range") and it is the
	// difference between a list that helps and one that runs away from the
	// cursor every time something is picked.
	if (row_top >= view_top && row_top + row_h <= view_top + view_h) return;

	// Off screen: centre it, which is what STreeView does - its default
	// alignment is CenterAligned, offset = index - NumVisible/2. ImGui clamps
	// the target to the scroll range for us.
	ImGui::SetScrollY(row_top + row_h * 0.5f - row_h - view_h * 0.5f);
}

bool UIWorldOutliner::DrawGroupRow(SGroup& g, bool open, float& row_top)
{
	const float row_h = XUI::Pt(kRowH);
	ImGui::TableNextRow(ImGuiTableRowFlags_None, row_h);

	// the class's own flag: the one the Tools panel's tile shows too
	ImGui::TableSetColumnIndex(0);
	row_top = ImGui::GetCursorPosY();
	ESceneToolBase* tool = Scene->GetTool(g.cls);
	const bool class_visible = !tool || !!tool->IsVisible();
	if (tool && XUI_LocalEyeToggle("##class_eye", class_visible,
		class_visible ? "Hide this class in the viewport" : "Show this class in the viewport", kRowH))
	{
		tool->m_EditFlags.set(ESceneToolBase::flVisible, !class_visible);
		UI->RedrawScene(true);
	}

	ImGui::TableSetColumnIndex(1);
	const ImVec2 cell = ImGui::GetCursorScreenPos();
	const float right = cell.x + ImGui::GetContentRegionAvail().x;
	ImGui::SetNextItemAllowOverlap();
	const bool clicked = ImGui::Selectable("##group", false, ImGuiSelectableFlags_SpanAllColumns, ImVec2(0.f, row_h));

	ImDrawList* dl = ImGui::GetWindowDrawList();
	const float cy = cell.y + row_h * .5f;
	XUI::DrawIcon(dl, open ? XUI::Icon::ChevronDown : XUI::Icon::ChevronRight, ImVec2(Round(cell.x), Round(cy - XUI::Pt(6.f))), 12.f,
		GrizzlyTheme::U32(Token::TextSecondary));
	float x = cell.x + XUI::Pt(16.f);
	XUI::DrawIcon(dl, ToolClassIcon(g.cls), ImVec2(Round(x), Round(cy - XUI::Pt(kRowIcon) * .5f)), kRowIcon,
		GrizzlyTheme::U32(class_visible ? Token::TextSecondary : Token::TextDisabled));
	x += XUI::Pt(kRowIcon + 6.f);

	char count[48];
	if (Filtering())
		sprintf_s(count, "%d of %d", int(g.shown.size()), int(g.objects.size()));
	else
		sprintf_s(count, "%d", int(g.objects.size()));
	const float count_w = ImGui::CalcTextSize(count).x;
	{
		XUI::FontScope strong(XUI::Font::Strong);
		ImGui::SetCursorScreenPos(ImVec2(x, TextY(cell.y)));
		ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(class_visible ? Token::Text : Token::TextSecondary));
		XUI::TextEllipsis(ToolClassTitle(g.cls), _max(0.f, right - x - count_w - XUI::Pt(6.f)));
		ImGui::PopStyleColor();
	}
	ImGui::SameLine(0.f, XUI::Pt(6.f));
	ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(Token::TextMuted));
	ImGui::TextUnformatted(count);
	ImGui::PopStyleColor();
	return clicked;
}

void UIWorldOutliner::DrawGroup(SGroup& g)
{
	if (ClassHidden(g.cls)) return;				// switched off in the Filters menu

	const bool filtering = Filtering();
	if (filtering && g.shown.empty()) return;	// empty groups vanish while filtering

	// a scroll target in a folded group would never come into view
	int target_row = -1;
	if (m_ScrollTarget)
		for (int i = 0; i < int(g.shown.size()); ++i)
			if (g.objects[g.shown[i]] == m_ScrollTarget) { target_row = i; break; }

	const bool open = filtering || target_row >= 0 || !GroupCollapsed(g.cls);
	ImGui::PushID(int(g.cls));
	float row_top = 0.f;
	// while a filter is on every class is open: a click would fold it only once the filter goes
	if (DrawGroupRow(g, open, row_top) && !filtering)
		SetGroupCollapsed(g.cls, open);

	if (open)
	{
		// Rows are a fixed height, so the target's position is arithmetic - it
		// never has to be drawn to be scrolled to. GetCursorPosY is content
		// space, the same space GetScrollY/SetScrollY speak.
		const float row_h		= XUI::Pt(kRowH);
		const float rows_top	= row_top + row_h;

		// thousands of rows per group - only the visible slice is submitted
		ImGuiListClipper clipper;
		clipper.Begin(int(g.shown.size()), row_h);
		while (clipper.Step())
			for (int row = clipper.DisplayStart; row < clipper.DisplayEnd; ++row)
				DrawFlatRow(g, row, g.objects[g.shown[row]]);

		if (target_row >= 0) ScrollRowIntoView(rows_top + target_row * row_h, row_h);
	}
	ImGui::PopID();
}

void UIWorldOutliner::DrawHierarchyRow(int row, const SHierarchyRow& item)
{
	SHierarchyNode& node = m_Hierarchy[item.node];
	SGroup& group = m_Groups[node.group];
	CCustomObject* obj = node.object;
	bool has_included_children = false;
	for (int child : node.children)
		if (m_Hierarchy[child].included) { has_included_children = true; break; }

	const float row_h = XUI::Pt(kRowH);
	ImGui::TableNextRow(ImGuiTableRowFlags_None, row_h);
	ImGui::PushID(obj);
	ImGui::TableSetColumnIndex(0);
	DrawObjectEye(obj);
	const bool visible = !!obj->Visible();

	ImGui::TableSetColumnIndex(1);
	const ImVec2 cell = ImGui::GetCursorScreenPos();
	const float right = cell.x + ImGui::GetContentRegionAvail().x;
	const float x = cell.x + _min(item.depth, 12) * XUI::Pt(kIndent);
	// the expander goes in before the row it sits on, so it wins the mouse
	if (has_included_children)
	{
		const bool filtering = HierarchyFiltering();
		const bool expanded = filtering || node.expanded;
		ImGui::SetCursorScreenPos(ImVec2(x, cell.y));
		ImGui::BeginDisabled(filtering);
		if (ImGui::InvisibleButton("##expand", ImVec2(XUI::Pt(16.f), row_h)))
			SetHierarchyExpanded(item.node, !expanded);
		ImGui::EndDisabled();
		XUI::DrawIcon(ImGui::GetWindowDrawList(), expanded ? XUI::Icon::ChevronDown : XUI::Icon::ChevronRight,
			ImVec2(Round(x + XUI::Pt(2.f)), Round(cell.y + (row_h - XUI::Pt(12.f)) * .5f)), 12.f,
			GrizzlyTheme::U32(filtering ? Token::TextDisabled : Token::TextSecondary));
		ImGui::SetCursorScreenPos(cell);
	}

	ImGuiSelectableFlags row_flags = ImGuiSelectableFlags_AllowDoubleClick | ImGuiSelectableFlags_SpanAllColumns;
	if (!visible) row_flags |= ImGuiSelectableFlags_Disabled;
	ImGui::SetNextItemAllowOverlap();
	const bool clicked = ImGui::Selectable("##row", !!obj->Selected(), row_flags, ImVec2(0.f, row_h));
	if (clicked) HandleHierarchySelection(row, obj);
	if (!visible)
		XUI::Tip("Hidden objects cannot be selected until shown.");
	else if (!node.direct)
		XUI::Tip("Shown because an object under it matches.");
	if (visible) DrawObjectContext(group, obj);

	DrawObjectCells(obj, x + XUI::Pt(16.f), cell.y, right, !node.direct);
	ImGui::PopID();
}

void UIWorldOutliner::DrawHierarchy()
{
	if (m_HierarchyDirty) ApplyHierarchyFilter();
	if (m_HierarchyRowsDirty) BuildHierarchyRows();

	int target_row = -1;
	if (m_ScrollTarget)
		for (int row = 0; row < int(m_HierarchyRows.size()); ++row)
			if (m_Hierarchy[m_HierarchyRows[row].node].object == m_ScrollTarget) { target_row = row; break; }

	const float row_h = XUI::Pt(kRowH);
	const float rows_top = ImGui::GetCursorPosY();
	ImGuiListClipper clipper;
	clipper.Begin(int(m_HierarchyRows.size()), row_h);
	while (clipper.Step())
		for (int row = clipper.DisplayStart; row < clipper.DisplayEnd; ++row)
			DrawHierarchyRow(row, m_HierarchyRows[row]);

	if (target_row >= 0) ScrollRowIntoView(rows_top + target_row * row_h, row_h);
}

// The Filters menu, Unreal's in miniature: what of the scene the tree shows. Types are hidden rather than shown so a
// scene growing a new one does not silently stay invisible.
void UIWorldOutliner::DrawFiltersMenu()
{
	ImGui::Checkbox("Selected Only", &m_SelectedOnly);
	XUI::Tip("Show only the objects selected in the viewport");
	if (XUI::BeginPropertyGrid("outliner_filters", .3f))
	{
		if (XUI::PropertyRow("Visibility", "Every object, only the visible ones or only the hidden ones"))
		{
			const char* modes[] = { "All", "Visible", "Hidden" };
			XUI::Segmented("##visibility", &m_Visibility, modes, 3);
		}
		if (XUI::PropertyRow("Sort", "The order of the objects in each class: as in the scene, or by name"))
		{
			const char* sorts[] = { "Scene", "A-Z", "Z-A" };
			XUI::Segmented("##sort", &m_Sort, sorts, 3);
		}
		XUI::EndPropertyGrid();
	}
	ImGui::Separator();

	bool hierarchy_filter_changed = false;
	ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(Token::TextSecondary));
	ImGui::AlignTextToFramePadding();
	ImGui::TextUnformatted("Object Types");
	ImGui::PopStyleColor();
	ImGui::SameLine();
	if (XUI::Button("All##types"))
	{
		hierarchy_filter_changed = !m_HiddenClasses.empty();
		m_HiddenClasses.clear();
	}
	ImGui::SameLine(0.f, XUI::Pt(4.f));
	if (XUI::Button("None##types"))
	{
		m_HiddenClasses.clear();
		for (u32 i = 0; i < m_Groups.size(); ++i)
			m_HiddenClasses.push_back(m_Groups[i].cls);
		hierarchy_filter_changed = true;
	}

	for (u32 i = 0; i < m_Groups.size(); ++i)
	{
		SGroup& g	= m_Groups[i];
		bool shown	= !ClassHidden(g.cls);
		ImGui::PushID(int(g.cls));
		if (ImGui::Checkbox(ToolClassTitle(g.cls), &shown))
		{
			if (shown)
			{
				for (u32 k = 0; k < m_HiddenClasses.size(); ++k)
					if (m_HiddenClasses[k] == g.cls) { m_HiddenClasses.erase(m_HiddenClasses.begin() + k); break; }
			}
			else m_HiddenClasses.push_back(g.cls);
			hierarchy_filter_changed = true;
		}
		char count[16];
		sprintf_s(count, "%d", int(g.objects.size()));
		ImGui::SameLine();
		ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(Token::TextMuted));
		ImGui::TextUnformatted(count);
		ImGui::PopStyleColor();
		ImGui::PopID();
	}
	if (hierarchy_filter_changed)
	{
		if (m_HierarchyView)
			ApplyHierarchyFilter();
		else
			m_HierarchyRowsDirty = true;
	}

	ImGui::Separator();
	XUI::StatusText(XUI::Tone::Muted, "Search: every word has to match, -word leaves those out, \"two words\" match together.");
}

void UIWorldOutliner::DrawMoreMenu()
{
	if (XUI::MenuItem("By Type", nullptr, !m_HierarchyView))
		SetHierarchyView(false);
	if (XUI::MenuItem("By Hierarchy", nullptr, m_HierarchyView))
		SetHierarchyView(true);
	XUI::Tip("Objects under the group that owns them. View only: ownership is never changed here.");
	ImGui::Separator();
	if (XUI::MenuItem("Show All"))
		ShowObjects(false, true);
	if (XUI::MenuItem("Show Selected"))
		ShowObjects(true, true);
	if (XUI::MenuItem("Hide Selected"))
		ShowObjects(true, false);
}

void UIWorldOutliner::DrawToolbar()
{
	XUI::Toolbar tb("outliner", XUI::ToolbarPanel);
	tb.Group(2);
	if (m_FocusSearch)
	{
		ImGui::SetKeyboardFocusHere();
		m_FocusSearch = false;
	}
	tb.Search(m_Filter, sizeof(m_Filter), "Search", 8.f);

	tb.Group(1);
	const int active = ActiveFilters();
	const bool filters = tb.Action("filters", XUI::Icon::Filter, "Filters", "Selected only, visibility, sort and object types",
		true, active > 0);
	// drawn in the row, not in the overflow menu: its menu opens under it, and the badge counts what is on
	if (ImGui::GetItemID() == ImGui::GetID("filters"))
	{
		const ImVec2 min = ImGui::GetItemRectMin(), max = ImGui::GetItemRectMax();
		m_FiltersAnchor = ImVec2(min.x, max.y + XUI::Pt(2.f));
		if (active)
		{
			char text[8];
			sprintf_s(text, "%d", active);
			ImFont* font = XUI::GetFont(XUI::Font::Small);
			if (!font) font = ImGui::GetFont();
			const float size = XUI::FontSize(XUI::Font::Small) * XUI::Dpi();
			const ImVec2 ts = font->CalcTextSizeA(size, FLT_MAX, 0.f, text);
			const ImVec2 c(max.x - XUI::Pt(3.f), min.y + XUI::Pt(3.f));
			ImDrawList* dl = ImGui::GetWindowDrawList();
			dl->AddCircleFilled(c, _max(XUI::Pt(7.f), ts.x * .5f + XUI::Pt(3.f)), GrizzlyTheme::U32(Token::Accent));
			dl->AddText(font, size, ImVec2(Round(c.x - ts.x * .5f), Round(c.y - ts.y * .5f)), GrizzlyTheme::U32(Token::Text), text);
		}
	}
	if (filters || XUI::Shell::TakePopup("outliner_filters"))
		XUI::Shell::OpenPopupAt("##filters_menu", m_FiltersAnchor);
	ImGui::SetNextWindowSize(ImVec2(XUI::Em(24.f), 0.f));
	if (ImGui::BeginPopup("##filters_menu"))
	{
		DrawFiltersMenu();
		ImGui::EndPopup();
	}

	const bool more = tb.Action("more", XUI::Icon::More, "", "Layout, and showing or hiding objects");
	if (ImGui::GetItemID() == ImGui::GetID("more"))
		m_MoreAnchor = ImVec2(ImGui::GetItemRectMin().x, ImGui::GetItemRectMax().y + XUI::Pt(2.f));
	if (more || XUI::Shell::TakePopup("outliner_more"))
		XUI::Shell::OpenPopupAt("##more_menu", m_MoreAnchor);
	if (ImGui::BeginPopup("##more_menu"))
	{
		DrawMoreMenu();
		ImGui::EndPopup();
	}
}

void UIWorldOutliner::DrawFooter(int shown, int selected)
{
	char text[96];
	if (shown != m_TotalObjects)
		sprintf_s(text, "%d of %d objects \xC2\xB7 %d selected", shown, m_TotalObjects, selected);
	else
		sprintf_s(text, "%d object%s \xC2\xB7 %d selected", m_TotalObjects, m_TotalObjects == 1 ? "" : "s", selected);
	XUI::FontScope small_font(XUI::Font::Small);
	// at the bottom of the panel whatever is above it
	const float bottom = ImGui::GetWindowPos().y + ImGui::GetWindowHeight() - ImGui::GetStyle().WindowPadding.y - ImGui::GetTextLineHeight();
	const ImVec2 at = ImGui::GetCursorScreenPos();
	ImGui::SetCursorScreenPos(ImVec2(at.x, _max(at.y, bottom)));
	ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(Token::TextMuted));
	XUI::TextEllipsis(text);
	ImGui::PopStyleColor();
}

void UIWorldOutliner::DrawRenameDialog()
{
	if (!m_RenameOpen && !m_RenameOriginal.size())
		return;
	if (!XUI::BeginDialog("Rename Object###dlg.rename_object", &m_RenameOpen, XUI::DialogSize::S))
	{
		if (!m_RenameOpen)
			m_RenameOriginal = "";
		return;
	}
	XUI::BeginDialogBody();
	if (m_RenameFocus)
	{
		ImGui::SetKeyboardFocusHere();
		m_RenameFocus = false;
	}
	ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
	bool apply = ImGui::InputText("##new_name", m_RenameBuf, sizeof(m_RenameBuf),
		ImGuiInputTextFlags_EnterReturnsTrue | ImGuiInputTextFlags_AutoSelectAll);
	if (m_RenameError[0])
		XUI::StatusText(XUI::Tone::Error, m_RenameError);
	else
		XUI::StatusText(XUI::Tone::Muted, "Names are lower case and unique in the scene.");
	if (XUI::BeginDialogFooter())
	{
		apply |= XUI::DialogButton("Rename", XUI::Kind::Primary, true);
		if (XUI::DialogButton("Cancel", XUI::Kind::Secondary))
			m_RenameOpen = false;
	}
	XUI::EndDialog();
	if (apply && m_RenameOpen && ApplyRename())
		m_RenameOpen = false;
}

void UIWorldOutliner::Draw()
{
	ImGui::PushStyleVar(ImGuiStyleVar_WindowMinSize, ImVec2(XUI::Pt(220.f), XUI::Pt(160.f)));
	// a fresh panel opens usable instead of a stub; the user's own size wins afterwards
	ImGui::SetNextWindowSize(ImVec2(XUI::Pt(340.f), XUI::Pt(480.f)), ImGuiCond_FirstUseEver);
	if (m_FocusWindow)
	{
		ImGui::SetNextWindowFocus();
		m_FocusWindow = false;
	}
	const bool visible = XUI::BeginPanel(EditorUiContract::kWindowOutliner, &bOpen);
	ImGui::PopStyleVar();
	if (!visible || !Scene)
	{
		if (visible)
			XUI::EmptyState(XUI::Icon::Level, "No scene", "Open or create a scene to see its objects here.");
		XUI::EndPanel();
		return;
	}

	// cache maintenance: an explicit dirty flag, a per-class count probe and the
	// active tool - anything else (renames aside) cannot change the tree
	if (m_Dirty || m_TargetClass != LTools->CurrentClassID() || m_Signature != SceneSignature())
		Rebuild();

	// the selected-only view depends on live selection, so it is redone each
	// frame; a plain text filter only when the text itself changed
	const bool text_changed = (0 != xr_strcmp(m_FilterApplied.c_str(), m_Filter));
	if (text_changed) ParseFilter();
	if (m_SelectedOnly || m_Visibility || m_SelectedOnlyApplied != m_SelectedOnly ||
		m_VisibilityApplied != m_Visibility || m_SortApplied != m_Sort || text_changed)
		ApplyFilter();

	// selection changes made OUTSIDE this panel (viewport picks, MCP) scroll
	// the last selected row into view; the panel's own clicks flag
	// m_SkipNextScroll, since their row is already visible
	u32 sel_sig;
	CCustomObject* sel_last;
	const int sel_count = ScanSelection(sel_sig, sel_last);
	if (sel_sig != m_SelSignature)
	{
		m_SelSignature = sel_sig;
		if (!m_SkipNextScroll && sel_last)
		{
			m_ScrollTarget = sel_last;
			if (m_HierarchyView) RevealHierarchyPath(sel_last);
		}
		m_SkipNextScroll = false;
	}

	const bool hierarchy_before = m_HierarchyView;
	DrawToolbar();
	if (m_HierarchyView && !hierarchy_before && sel_last)
	{
		RevealHierarchyPath(sel_last);
		m_ScrollTarget = sel_last;
	}

	// with a filter or a hidden type on, the footer says how much of the scene is showing
	int shown = 0;
	for (u32 i = 0; i < m_Groups.size(); ++i)
		if (!ClassHidden(m_Groups[i].cls)) shown += int(m_Groups[i].shown.size());

	// the footer's row is kept whatever sits above it, the way the table keeps it
	const float row_h = XUI::Pt(kRowH);
	if (!m_TotalObjects || !shown)
	{
		// the empty state centres in what is left above the footer, and scrolls where that is shorter than it
		ImGui::PushStyleColor(ImGuiCol_ChildBg, ImVec4(0.f, 0.f, 0.f, 0.f));
		const bool open = ImGui::BeginChild("##empty", ImVec2(0.f, -row_h));
		ImGui::PopStyleColor();
		if (open)
		{
			if (!m_TotalObjects)
				XUI::EmptyState(XUI::Icon::SceneObject, "The scene is empty",
					"Pick a tool in Tools and click in the viewport, or drag an asset in from the Content Browser.");
			else if (XUI::EmptyState(XUI::Icon::Search, "Nothing matches", "No object passes the search and the filters.", "Clear Filters"))
				ClearFilters();
		}
		ImGui::EndChild();
	}
	else
	{
		ImGui::PushStyleVar(ImGuiStyleVar_CellPadding, ImVec2(XUI::Pt(4.f), 0.f));
		const ImGuiTableFlags table_flags = ImGuiTableFlags_RowBg | ImGuiTableFlags_Resizable | ImGuiTableFlags_Reorderable |
			ImGuiTableFlags_Hideable | ImGuiTableFlags_ScrollY;
		if (ImGui::BeginTable("objects", 4, table_flags, ImVec2(0.f, -row_h)))
		{
			ImGui::TableSetupScrollFreeze(0, 1);
			ImGui::TableSetupColumn("Visibility", ImGuiTableColumnFlags_WidthFixed | ImGuiTableColumnFlags_NoHide |
				ImGuiTableColumnFlags_NoResize | ImGuiTableColumnFlags_NoReorder | ImGuiTableColumnFlags_NoHeaderLabel, row_h);
			ImGui::TableSetupColumn("Name", ImGuiTableColumnFlags_WidthStretch | ImGuiTableColumnFlags_NoHide, 1.f);
			ImGui::TableSetupColumn("Type", ImGuiTableColumnFlags_WidthFixed | ImGuiTableColumnFlags_DefaultHide, XUI::Em(7.f));
			ImGui::TableSetupColumn("Owner", ImGuiTableColumnFlags_WidthStretch | ImGuiTableColumnFlags_DefaultHide, .5f);
			{
				// the header's right-click menu shows Type and Owner
				XUI::FontScope small_font(XUI::Font::Small);
				ImGui::TableNextRow(ImGuiTableRowFlags_Headers, row_h);
				for (int column = 0; column < 4; ++column)
				{
					if (!ImGui::TableSetColumnIndex(column)) continue;
					ImGui::PushID(column);
					ImGui::TableHeader(column ? ImGui::TableGetColumnName(column) : "##visibility");
					if (!column)
					{
						const ImVec2 min = ImGui::GetItemRectMin(), max = ImGui::GetItemRectMax();
						const float s = XUI::Pt(kRowIcon);
						XUI::DrawIcon(ImGui::GetWindowDrawList(), XUI::Icon::Eye,
							ImVec2(Round((min.x + max.x - s) * .5f), Round((min.y + max.y - s) * .5f)), kRowIcon,
							GrizzlyTheme::U32(Token::TextMuted));
						XUI::Tip("Visibility in the viewport");
					}
					ImGui::PopID();
				}
			}

			if (m_HierarchyView)
				DrawHierarchy();
			else
				for (u32 i = 0; i < m_Groups.size(); ++i)
					DrawGroup(m_Groups[i]);
			ImGui::EndTable();
		}
		ImGui::PopStyleVar();
	}
	// A target the filter hid never reaches a row; one frame is its lifetime.
	m_ScrollTarget = nullptr;

	DrawFooter(shown, sel_count);
	DrawRenameDialog();
	XUI::EndPanel();
}
