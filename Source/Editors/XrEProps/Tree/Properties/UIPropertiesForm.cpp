#include "stdafx.h"
#include "../../UIPropsLocal.h"

xr_string PropEditText(PropItem* item);
const void* PropTarget(PropItem* item);
bool PropIsAngle(PropValue* value);

namespace
{
template <typename T>
bool Bounded(T mn, T mx)
{
	return mn < mx && double(mx) - double(mn) < 1e30;
}

bool ParseBool(LPCSTR text, bool& out)
{
	if (!_stricmp(text, "true") || !xr_strcmp(text, "1") || !_stricmp(text, "on") || !_stricmp(text, "yes"))
		out = true;
	else if (!_stricmp(text, "false") || !xr_strcmp(text, "0") || !_stricmp(text, "off") || !_stricmp(text, "no"))
		out = false;
	else
		return false;
	return true;
}

// "#RRGGBB" (opaque) or "0xAARRGGBB"
bool ParseColor(LPCSTR text, u32& argb, bool& has_alpha)
{
	const char* s = text;
	has_alpha = false;
	if (*s == '#')
		++s;
	else if (s[0] == '0' && (s[1] == 'x' || s[1] == 'X'))
	{
		s += 2;
		has_alpha = xr_strlen(s) == 8;
	}
	const size_t len = xr_strlen(s);
	if (len != 6 && len != 8)
		return false;
	char* end = nullptr;
	argb = u32(strtoul(s, &end, 16));
	if (end && *end)
		return false;
	has_alpha = len == 8;
	if (len == 6)
		argb |= 0xff000000u;
	return true;
}

// three numbers: "1, 2, 3", "1 2 3" or "{1, 2, 3}"
bool ParseVector(LPCSTR text, Fvector& out)
{
	string256 tmp;
	xr_strcpy(tmp, text);
	for (char* c = tmp; *c; ++c)
		if (*c == '{' || *c == '}' || *c == ',' || *c == ';')
			*c = ' ';
	float x, y, z;
	char extra[4];
	if (sscanf_s(tmp, "%f %f %f %1s", &x, &y, &z, extra, u32(sizeof(extra))) != 3)
		return false;
	out.set(x, y, z);
	return true;
}

template <typename T>
bool SetNumeric(PropItem* item, LPCSTR text, bool& changed, xr_string& err)
{
	NumericValue<T>* V = dynamic_cast<NumericValue<T>*>(item->GetFrontValue());
	if (!V)
		return false;
	T value = *V->value;
	item->BeforeEdit<NumericValue<T>, T>(value);
	char* end = nullptr;
	const double v = strtod(text, &end);
	if (!*text || (end && *end))
		err = "takes a number";
	else if (Bounded(V->lim_mn, V->lim_mx) && (v < double(V->lim_mn) || v > double(V->lim_mx)))
	{
		string128 range;
		xr_sprintf(range, "takes a number from %g to %g", double(V->lim_mn), double(V->lim_mx));
		err = range;
	}
	if (!err.empty())
	{
		item->AfterEdit<NumericValue<T>, T>(value);
		return true;
	}
	if constexpr (std::is_floating_point_v<T>)
		value = T(v);
	else
		value = T(std::is_unsigned_v<T> && v < 0.0 ? 0.0 : floor(v + .5));
	changed = item->AfterEdit<NumericValue<T>, T>(value) && item->ApplyValue<NumericValue<T>, T>(value);
	return true;
}

template <class T>
bool SetFlag(PropItem* item, bool on, bool& changed)
{
	FlagValue<_flags<T> >* V = dynamic_cast<FlagValue<_flags<T> >*>(item->GetFrontValue());
	if (!V)
		return false;
	_flags<T> value = V->GetValue();
	item->BeforeEdit<FlagValue<_flags<T> >, _flags<T> >(value);
	value.set(V->mask, on);
	changed = item->AfterEdit<FlagValue<_flags<T> >, _flags<T> >(value) && item->ApplyValue<FlagValue<_flags<T> >, _flags<T> >(value);
	return true;
}

// a name of the list (any case) or, failing that, its number; the names are what the error lists
template <typename Name>
int FindName(LPCSTR text, int count, Name name, xr_string& names)
{
	for (int i = 0; i < count; ++i)
		if (name(i) && !_stricmp(name(i), text))
			return i;
	for (int i = 0; i < count && i < 16; ++i)
	{
		names += i ? ", " : "takes one of ";
		names += name(i) ? name(i) : "";
	}
	if (count > 16)
		names += ", ...";
	return -1;
}

template <class T>
bool SetToken(PropItem* item, LPCSTR text, bool& changed, xr_string& err)
{
	TokenValue<T>* V = dynamic_cast<TokenValue<T>*>(item->GetFrontValue());
	if (!V)
		return false;
	int cnt = 0;
	while (V->token[cnt].name)
		++cnt;
	const int i = FindName(text, cnt, [V](int k) { return V->token[k].name; }, err);
	if (i < 0)
		return true;
	err.clear();
	T value = T(V->token[i].id);
	changed = item->AfterEdit<TokenValue<T>, T>(value) && item->ApplyValue<TokenValue<T>, T>(value);
	return true;
}

template <class T>
bool SetRToken(PropItem* item, LPCSTR text, bool& changed, xr_string& err)
{
	RTokenValue<T>* V = dynamic_cast<RTokenValue<T>*>(item->GetFrontValue());
	if (!V)
		return false;
	const int i = FindName(text, int(V->token_count), [V](int k) { return V->token[k].name.c_str(); }, err);
	if (i < 0)
		return true;
	err.clear();
	T value = T(V->token[i].id);
	changed = item->AfterEdit<RTokenValue<T>, T>(value) && item->ApplyValue<RTokenValue<T>, T>(value);
	return true;
}

int TextBufferResize(ImGuiInputTextCallbackData* data)
{
	if (data->EventFlag == ImGuiInputTextFlags_CallbackResize)
	{
		xr_vector<char>* buffer = static_cast<xr_vector<char>*>(data->UserData);
		buffer->resize(size_t(data->BufSize), 0);
		data->Buf = buffer->data();
	}
	return 0;
}
}

UIPropertiesForm::UIPropertiesForm():m_Root("",this)
{
	m_bModified = false;
	m_EditChooseValue = nullptr;
	m_EditShortcutValue = nullptr;
	m_EditTextureValue = nullptr;
	m_EditTextTarget = nullptr;
	m_EditTextOpen = false;
	m_InlineTarget = nullptr;
	m_EditGameTypeOpen = false;
	m_Flags.zero();
	m_Filter[0] = 0;
	m_VisiblePropertyCount = 0;
	m_PropertyCount = 0;
	m_ExpandByDefault = true;
	m_Chrome = chromeAuto;
	m_Role = "default";
	m_LabelFrac = .40f;
	m_ValueMaxEm = 0.f;
	m_FooterShown = false;
	m_RowsFrame = -1;
	m_Drawing = 0;
	m_HasDeferred = false;
	m_ChooserOwned = false;
}

UIPropertiesForm::~UIPropertiesForm()
{
	ClearProperties();
	for (PropItem* item : m_Deferred)
		xr_delete(item);
}

// what the choosers and the shortcut form a row opened hand back
void UIPropertiesForm::TakeResults()
{
	if (m_EditChooseValue)
	{
		shared_str result;
		bool is_result;
		if (UIChooseForm::GetResult(is_result, result))
		{
			if (is_result)
			{
				if (m_EditChooseValue->AfterEdit<ChooseValue, shared_str>(result))
					if (m_EditChooseValue->ApplyValue<ChooseValue, shared_str>(result))
					{
						Modified();
					}
			}
			m_EditChooseValue = nullptr;
			m_ChooserOwned = false;
		}
		else if (!UIChooseForm::IsActive())
		{
			// the form died without handing the result over - somebody else consumed it.
			// Keeping the edit pending would arm it for the NEXT chooser the author opens
			m_EditChooseValue = nullptr;
			m_ChooserOwned = false;
		}

		UIChooseForm::Update();
	}
	if (m_EditTextureValue)
	{
		shared_str result;
		bool is_result;
		if (UIChooseForm::GetResult(is_result, result))
		{
			if (is_result)
			{
				xr_string result_as_str = result.c_str() ? result.c_str() : "$null";
				if (m_EditTextureValue->AfterEdit<CTextValue, xr_string>(result_as_str))
					if (m_EditTextureValue->ApplyValue<CTextValue, LPCSTR>(result_as_str.c_str()))
					{
						Modified();
					}
			}
			m_EditTextureValue = nullptr;
			m_ChooserOwned = false;
		}
		else if (!UIChooseForm::IsActive())
		{
			m_EditTextureValue = nullptr;
			m_ChooserOwned = false;
		}
		UIChooseForm::Update();
	}
	if (m_EditShortcutValue)
	{
		xr_shortcut result;
		bool ok;
		if (UIKeyPressForm::GetResult(ok, result))
		{
			if (ok)
			{
				if (m_EditShortcutValue->AfterEdit<ShortcutValue, xr_shortcut>(result))
					if (m_EditShortcutValue->ApplyValue<ShortcutValue, xr_shortcut>(result))
					{
						Modified();
					}
			}
			m_EditShortcutValue = nullptr;
		}
	}
	// the row that opened the chooser went away (the selection changed under it): the chooser stays up until it is
	// answered, and the answer lands nowhere
	if (m_ChooserOwned && !m_EditChooseValue && !m_EditTextureValue && !m_PendingChooseKey.size() && !m_PendingTextureKey.size())
	{
		shared_str result;
		bool is_result = false;
		if (!UIChooseForm::IsActive() || UIChooseForm::GetResult(is_result, result))
			m_ChooserOwned = false;
		else
			UIChooseForm::Update();
	}
}

void UIPropertiesForm::Tick()
{
	ImGui::PushID(this);
	++m_Drawing;
	TakeResults();
	DrawDialogs();
	--m_Drawing;
	ApplyDeferred();
	ImGui::PopID();
}

void UIPropertiesForm::Draw()
{
	ImGui::PushID(this);
	++m_Drawing;
	TakeResults();
	const bool panel = m_Chrome == chromePanel;
	const bool rows_only = m_Chrome == chromeInline;
	// auto: a search once there is something to search, a grid that scrolls under its help when there is room
	const bool search = !rows_only && (panel || m_PropertyCount > 8 || IsFiltering());
	const bool roomy = panel || (!rows_only && ImGui::GetContentRegionAvail().y >= XUI::Pt(240.f));
	if (search)
		DrawSearchRow();
	if (roomy)
	{
		const float footer = HelpFooterHeight() + ImGui::GetStyle().ItemSpacing.y;
		const float height = _max(XUI::Pt(60.f), ImGui::GetContentRegionAvail().y - footer);
		if (ImGui::BeginChild("##grid", ImVec2(0.f, height), ImGuiChildFlags_None))
			DrawRows();
		ImGui::EndChild();
		DrawHelpFooter();
	}
	else
		DrawRows();
	DrawDialogs();
	--m_Drawing;
	ApplyDeferred();
	ImGui::PopID();
}

void UIPropertiesForm::DrawGrid()
{
	ImGui::PushID(this);
	++m_Drawing;
	TakeResults();
	DrawRows();
	DrawDialogs();
	--m_Drawing;
	ApplyDeferred();
	ImGui::PopID();
}

void UIPropertiesForm::DrawSearchRow()
{
	const float gap = ImGui::GetStyle().ItemInnerSpacing.x;
	const float more = XUI::Pt(24.f);
	string64 hint;
	if (m_PropertyCount == 1)
		xr_strcpy(hint, "Search 1 property");
	else
		xr_sprintf(hint, "Search %u properties", m_PropertyCount);
	ImGui::SetNextItemWidth(_max(XUI::Em(6.f), ImGui::GetContentRegionAvail().x - more - gap));
	if (XUI::SearchField("##property_search", m_Filter, sizeof(m_Filter), hint))
		RefreshFilter();
	ImGui::SameLine(0.f, gap);
	const ImVec2 at = ImGui::GetCursorScreenPos();
	if (XUI::IconButton("##property_menu", XUI::Icon::More, "More"))
		XUI::Shell::OpenPopupAt("##property_menu_popup", ImVec2(at.x, at.y + ImGui::GetFrameHeight() + XUI::Pt(2.f)));
	if (ImGui::BeginPopup("##property_menu_popup"))
	{
		if (IsFiltering())
		{
			string64 counts;
			xr_sprintf(counts, "%u of %u match", m_VisiblePropertyCount, m_PropertyCount);
			ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::Vec4(GrizzlyTheme::ColorToken::TextMuted));
			ImGui::TextUnformatted(counts);
			ImGui::PopStyleColor();
			ImGui::Separator();
		}
		if (ImGui::MenuItem("Expand All"))
			ExpandAll();
		if (ImGui::MenuItem("Collapse All", nullptr, false, !IsFiltering()))
			CollapseAll();
		if (IsFiltering())
			XUI::Tip("Everything that matches the search stays open while it is on");
		ImGui::EndPopup();
	}
}

void UIPropertiesForm::DrawRows()
{
	// what the rows drawn this frame tell the help footer
	m_HoverKey.clear();
	m_FooterShown = false;
	m_RowsFrame = ImGui::GetFrameCount();
	if (!m_VisiblePropertyCount)
	{
		// an embedded list with nothing to show (a chooser before a pick) draws nothing at all
		if (m_Chrome == chromeInline && !IsFiltering())
			return;
		if (IsFiltering())
		{
			xr_string text = "Nothing matches \"";
			text += m_Filter;
			text += "\".";
			XUI::EmptyState(XUI::Icon::Search, "No matches", text.c_str());
		}
		else
			XUI::EmptyState(XUI::Icon::Info, "No properties");
		return;
	}
	if (XUI::BeginPropertyGrid(m_Role.c_str(), m_LabelFrac))
	{
		m_Root.DrawRoot();
		XUI::EndPropertyGrid();
	}
	// the footer keeps speaking of the last row the mouse was over
	if (!m_HoverKey.empty())
	{
		m_FooterKey = m_HoverKey;
		m_FooterShown = true;
	}
}

float UIPropertiesForm::HelpFooterHeight()
{
	return XUI_Local::HelpFooterHeight();
}

void UIPropertiesForm::DrawHelpFooter(float width)
{
	// a footer drawn without its rows this frame (the host showed something else) has no row to speak of
	const bool shown = m_FooterShown && m_RowsFrame == ImGui::GetFrameCount() && !m_FooterKey.empty();
	PropItem* item = shown ? FindItem(m_FooterKey.c_str()) : nullptr;
	if (!item)
	{
		XUI_Local::HelpFooter(nullptr, "Point at a property to see what it does.", width);
		return;
	}
	const xr_string label = LabelOf(item->Key());
	xr_string help = GetHelp(item);
	// nothing to say about it: at least where it lives
	if (help.empty())
	{
		const char* key = item->Key();
		const char* leaf = strrchr(key, '\\');
		for (const char* c = key; leaf && c < leaf; ++c)
		{
			if (*c == '\\')
				help += " \xC2\xB7 ";
			else
				help += *c;
		}
	}
	XUI_Local::HelpFooter(label.c_str(), help.c_str(), width);
}

void UIPropertiesForm::DrawDialogs()
{
	DrawEditText();
	DrawEditGameType();
}

void UIPropertiesForm::AssignItems(PropItemVec& items)
{
	// a row's own callback lists the rows anew while they are drawn: the new list takes over when the frame is
	// through, so nothing the loop still walks is freed under it
	if (m_Drawing)
	{
		for (PropItem* item : m_Deferred)
			if (std::find(items.begin(), items.end(), item) == items.end())
				xr_delete(item);
		m_Deferred = items;
		m_HasDeferred = true;
		return;
	}
	// a list given over another one replaces it: the old rows would stay behind
	if (!m_Items.empty())
	{
		m_Items.erase(std::remove_if(m_Items.begin(), m_Items.end(),
			[&items](PropItem* item) { return std::find(items.begin(), items.end(), item) != items.end(); }), m_Items.end());
		ClearProperties();
	}
	m_Items = items;
	for (PropItem* item : items)
	{
		item->m_Owner = this;
		UIPropertiesItem* Item = static_cast<UIPropertiesItem*>(m_Root.AppendItem(item->Key()));
		VERIFY(Item);
		Item->PItem = item;
	}
	m_Root.Arrange();
	// gone from the new list means the edit has nothing to land on: the result is dropped when it arrives
	if (m_PendingChooseKey.size())
	{
		m_EditChooseValue = FindItem(m_PendingChooseKey.c_str());
		m_PendingChooseKey = nullptr;
	}
	if (m_PendingTextureKey.size())
	{
		m_EditTextureValue = FindItem(m_PendingTextureKey.c_str());
		m_PendingTextureKey = nullptr;
	}
	if (m_PendingShortcutKey.size())
	{
		m_EditShortcutValue = FindItem(m_PendingShortcutKey.c_str());
		m_PendingShortcutKey = nullptr;
	}
	RefreshFilter();
}

void UIPropertiesForm::ApplyDeferred()
{
	if (m_Drawing || !m_HasDeferred)
		return;
	m_HasDeferred = false;
	PropItemVec items;
	items.swap(m_Deferred);
	AssignItems(items);
}

void UIPropertiesForm::SetFilter(LPCSTR filter)
{
	strncpy_s(m_Filter, sizeof(m_Filter), filter ? filter : "", _TRUNCATE);
	RefreshFilter();
}

void UIPropertiesForm::ExpandAll()
{
	m_ExpandByDefault = true;
	m_Root.SetExpandedRecursive(true);
}

void UIPropertiesForm::CollapseAll()
{
	m_ExpandByDefault = false;
	m_Root.SetExpandedRecursive(false);
}

void UIPropertiesForm::RefreshFilter()
{
	m_VisiblePropertyCount = 0;
	m_PropertyCount = 0;
	xr_string rootPath;
	m_Root.RefreshFilter(m_Filter, rootPath, false, m_VisiblePropertyCount, m_PropertyCount);
}

bool UIPropertiesForm::RestoreExpansion(LPCSTR path, bool fallback) const
{
	const auto it = m_ExpansionState.find(path ? path : "");
	return it != m_ExpansionState.end() ? it->second : fallback;
}

void UIPropertiesForm::RememberExpansion(LPCSTR path, bool expanded)
{
	if (path && path[0])
		m_ExpansionState[path] = expanded;
}

void UIPropertiesForm::Folders(xr_vector<std::pair<xr_string, u32> >& out, bool matching) const
{
	out.clear();
	for (UITreeItem* item : m_Root.Items)
	{
		const UIPropertiesItem* folder = static_cast<const UIPropertiesItem*>(item);
		u32 total = 0, visible = 0;
		folder->CountRows(total, visible);
		out.emplace_back(folder->Name.c_str(), matching ? visible : total);
	}
}

void UIPropertiesForm::RowsInOrder(PropItemVec& out) const
{
	out.clear();
	out.reserve(m_Items.size());
	m_Root.CollectRows(out);
}

void UIPropertiesForm::SetHelp(LPCSTR key, LPCSTR help)
{
	if (!key || !key[0])
		return;
	if (help && help[0])
		m_Help[key] = help;
	else
		m_Help.erase(key);
}

namespace
{
// a unit a key may end in, "(m)", and how the value column writes it
struct SKeyUnit { const char* written; const char* unit; };
const SKeyUnit kKeyUnits[] =
{
	{ "m", "m" }, { "cm", "cm" }, { "km", "km" }, { "m/s", "m/s" }, { "s", "s" }, { "sec", "s" }, { "seconds", "s" },
	{ "ms", "ms" }, { "min", "min" }, { "minutes", "min" }, { "deg", "\xC2\xB0" }, { "degrees", "\xC2\xB0" },
	{ "%", "%" }, { "px", "px" }, { "fps", "fps" }, { "kb", "KB" }, { "mb", "MB" },
};

// the leaf of a key without its trailing " [range]" (a label of the old editors), and its trailing " (unit)" if it names one
void SplitLeaf(LPCSTR key, xr_string& label, const char** unit)
{
	*unit = nullptr;
	const char* leaf = key ? strrchr(key, '\\') : nullptr;
	label = leaf ? leaf + 1 : key ? key : "";
	const size_t open = label.rfind(" [");
	if (open != xr_string::npos && open > 0 && label.back() == ']')
		label.resize(open);
	const size_t paren = label.rfind(" (");
	if (paren == xr_string::npos || paren == 0 || label.back() != ')')
		return;
	const xr_string written = label.substr(paren + 2, label.size() - paren - 3);
	for (const SKeyUnit& u : kKeyUnits)
		if (!_stricmp(u.written, written.c_str()))
		{
			*unit = u.unit;
			label.resize(paren);
			return;
		}
}
}

xr_string UIPropertiesForm::LabelOf(LPCSTR key)
{
	// "Hemisphere quality [0-3]", "Weld distance (m)": the range is the help's to say, the unit the value's
	xr_string label;
	const char* unit;
	SplitLeaf(key, label, &unit);
	return label;
}

xr_string UIPropertiesForm::UnitOf(LPCSTR key)
{
	xr_string label;
	const char* unit;
	SplitLeaf(key, label, &unit);
	return unit ? unit : "";
}

void UIPropertiesForm::SetUnit(LPCSTR key, LPCSTR unit)
{
	if (!key || !key[0])
		return;
	if (unit && unit[0])
		m_Units[key] = unit;
	else
		m_Units.erase(key);
}

xr_string UIPropertiesForm::GetUnit(PropItem* item) const
{
	if (!item || item->Type() != PROP_NUMERIC)
		return {};
	const auto it = m_Units.find(item->Key());
	if (it != m_Units.end())
		return it->second;
	const xr_string unit = UnitOf(item->Key());
	if (!unit.empty())
		return unit;
	return PropIsAngle(item->GetFrontValue()) ? "\xC2\xB0" : "";
}

xr_string UIPropertiesForm::GetHelp(PropItem* item) const
{
	if (!item)
		return {};
	xr_string help;
	const auto it = m_Help.find(item->Key());
	if (it != m_Help.end())
		help = it->second;

	// the range a number takes, in its unit: what its label said in brackets, or what its limits say
	xr_string range;
	const xr_string key = item->Key();
	const xr_string unit = GetUnit(item);
	// a degree sign follows the number, a word is set apart from it
	const xr_string suffix = unit.empty() || unit == "\xC2\xB0" || unit == "%" ? unit : " " + unit;
	const size_t open = key.rfind(" [");
	if (open != xr_string::npos && key.back() == ']' && key.find('\\', open) == xr_string::npos)
		range = "Range: " + key.substr(open + 2, key.size() - open - 3) + suffix + ".";
	else if (item->Type() == PROP_NUMERIC)
	{
		PropValue* front = item->GetFrontValue();
		string128 text = "";
		const float to_display = PropIsAngle(front) ? 180.f / PI : 1.f;
		if (FloatValue* F = dynamic_cast<FloatValue*>(front))
		{
			if (Bounded(F->lim_mn, F->lim_mx))
				xr_sprintf(text, "Range: %g to %g%s.", double(F->lim_mn * to_display), double(F->lim_mx * to_display), suffix.c_str());
		}
		else if (U32Value* U = dynamic_cast<U32Value*>(front))
		{
			if (Bounded(U->lim_mn, U->lim_mx))
				xr_sprintf(text, "Range: %u to %u%s.", U->lim_mn, U->lim_mx, suffix.c_str());
		}
		else if (S32Value* S = dynamic_cast<S32Value*>(front))
		{
			if (Bounded(S->lim_mn, S->lim_mx))
				xr_sprintf(text, "Range: %d to %d%s.", S->lim_mn, S->lim_mx, suffix.c_str());
		}
		else if (U16Value* U16 = dynamic_cast<U16Value*>(front))
			xr_sprintf(text, "Range: %u to %u%s.", u32(U16->lim_mn), u32(U16->lim_mx), suffix.c_str());
		else if (S16Value* S16 = dynamic_cast<S16Value*>(front))
			xr_sprintf(text, "Range: %d to %d%s.", int(S16->lim_mn), int(S16->lim_mx), suffix.c_str());
		else if (U8Value* U8 = dynamic_cast<U8Value*>(front))
			xr_sprintf(text, "Range: %u to %u%s.", u32(U8->lim_mn), u32(U8->lim_mx), suffix.c_str());
		else if (S8Value* S8 = dynamic_cast<S8Value*>(front))
			xr_sprintf(text, "Range: %d to %d%s.", int(S8->lim_mn), int(S8->lim_mx), suffix.c_str());
		range = text;
	}
	if (!range.empty())
	{
		if (!help.empty())
			help += " ";
		help += range;
	}
	return help;
}

PropItem* UIPropertiesForm::FindItemOfName(shared_str name)
{
	for (PropItem* I : m_Items)
	{
		const char* key = I->Key();
		if (strrchr(key, '\\'))
		{
			key = strrchr(key, '\\') + 1;
		}
		if (name == key)
		{
			return I;
		}
	}
	// the label the grid shows, without the range or unit the key ends in ("Weld distance" of "Weld distance (m)")
	for (PropItem* I : m_Items)
		if (name.size() && LabelOf(I->Key()) == name.c_str())
			return I;
	return nullptr;
}

bool UIPropertiesForm::BeginChooseEdit(PropItem* item)
{
	if (!item || item->Type() != PROP_CHOOSE) return false;
	if (m_EditChooseValue || UIChooseForm::IsActive()) return false;
	ChooseValue* V = dynamic_cast<ChooseValue*>(item->GetFrontValue());
	if (!V) return false;

	shared_str edit_val = V->GetValue();
	if (!edit_val.size()) edit_val = V->m_StartPath;
	item->BeforeEdit<ChooseValue, shared_str>(edit_val);
	ChooseItemVec items;
	if (!V->OnChooseFillEvent.empty())
	{
		V->m_Items = &items;
		V->OnChooseFillEvent(V);
	}
	UIChooseForm::SelectItem(V->m_ChooseID, V->subitem, edit_val.c_str(), 0, V->m_FillParam, 0, items.size() ? &items : 0, V->m_ChooseFlags);
	m_EditChooseValue = item;
	m_ChooserOwned = true;
	return true;
}

void UIPropertiesForm::ClearProperties()
{
	if (m_Drawing)
	{
		PropItemVec none;
		AssignItems(none);
		return;
	}
	// The modals outlive the list: anything that rebuilds the properties while a
	// chooser is open would leave the pending edit pointing at a freed item, and
	// the pick would land in nothing. Remember the row, re-find it after the
	// rebuild - the key survives, the pointer does not.
	if (m_EditChooseValue)
	{
		m_PendingChooseKey = m_EditChooseValue->Key();
		m_EditChooseValue = nullptr;
	}
	if (m_EditTextureValue)
	{
		m_PendingTextureKey = m_EditTextureValue->Key();
		m_EditTextureValue = nullptr;
	}
	if (m_EditShortcutValue)
	{
		m_PendingShortcutKey = m_EditShortcutValue->Key();
		m_EditShortcutValue = nullptr;
	}
	for (PropItem* I : m_Items)
	{
		xr_delete(I);
	}
	m_Root.Reset(m_ExpandByDefault);
	m_Items.clear();
	RefreshFilter();
}

PropItem* UIPropertiesForm::FindItem(const char* name)
{
	UIPropertiesItem*Item = static_cast<UIPropertiesItem *>( m_Root.FindItem(name));
	if (Item)
	{
		return Item->PItem;
	}
	return nullptr;
}

bool UIPropertiesForm::ApplyText(PropItem* item, LPCSTR text)
{
	if (!item)
		return false;
	bool changed = false;
	switch (item->Type())
	{
	case PROP_RTEXT:
	{
		shared_str out = text;
		if (item->AfterEdit<RTextValue, shared_str>(out))
			changed = item->ApplyValue<RTextValue, shared_str>(out);
	}
	break;
	case PROP_STEXT:
	{
		xr_string out = text;
		if (item->AfterEdit<STextValue, xr_string>(out))
			changed = item->ApplyValue<STextValue, xr_string>(out);
	}
	break;
	case PROP_CTEXT:
	case PROP_TEXTURE2:
	{
		CTextValue* V = dynamic_cast<CTextValue*>(item->GetFrontValue());
		if (!V)
			break;
		xr_string out = text;
		if (item->AfterEdit<CTextValue, xr_string>(out))
		{
			// the value lives in a buffer of lim bytes: a longer text would run past it
			if (V->lim > 0 && out.size() >= size_t(V->lim))
				out.resize(size_t(V->lim) - 1);
			changed = item->ApplyValue<CTextValue, LPCSTR>(out.c_str());
		}
	}
	break;
	default:
		break;
	}
	if (changed)
		Modified();
	return changed;
}

bool UIPropertiesForm::SetValueText(PropItem* item, LPCSTR text, xr_string& err)
{
	err.clear();
	if (!item)
	{
		err = "is no row of this list";
		return false;
	}
	if (!text)
		text = "";
	if (IsReadOnly() || item->m_Flags.test(PropItem::flDisabled))
	{
		err = "is read-only";
		return false;
	}
	bool changed = false;
	switch (item->Type())
	{
	case PROP_NUMERIC:
	case PROP_TIME:
	{
		// the number as the row draws it is taken too: "1.5 m" is 1.5 on a row in metres
		xr_string number = text;
		const xr_string unit = GetUnit(item);
		if (!unit.empty() && number.size() > unit.size() && !number.compare(number.size() - unit.size(), unit.size(), unit))
			number.resize(number.size() - unit.size());
		while (!number.empty() && number.back() == ' ')
			number.pop_back();
		if (!SetNumeric<u32>(item, number.c_str(), changed, err) && !SetNumeric<float>(item, number.c_str(), changed, err) &&
			!SetNumeric<u8>(item, number.c_str(), changed, err) && !SetNumeric<s8>(item, number.c_str(), changed, err) &&
			!SetNumeric<s16>(item, number.c_str(), changed, err) && !SetNumeric<u16>(item, number.c_str(), changed, err) &&
			!SetNumeric<s32>(item, number.c_str(), changed, err))
			err = "cannot be set by text";
	}
	break;
	case PROP_BOOLEAN:
	{
		bool on = false;
		if (!ParseBool(text, on))
		{
			err = "takes true or false";
			break;
		}
		BOOLValue* V = dynamic_cast<BOOLValue*>(item->GetFrontValue());
		BOOL value = V ? V->GetValue() : FALSE;
		item->BeforeEdit<BOOLValue, BOOL>(value);
		value = on;
		changed = item->AfterEdit<BOOLValue, BOOL>(value) && item->ApplyValue<BOOLValue, BOOL>(value);
	}
	break;
	case PROP_FLAG:
	{
		bool on = false;
		if (!ParseBool(text, on))
		{
			err = "takes true or false";
			break;
		}
		if (!SetFlag<u8>(item, on, changed) && !SetFlag<u16>(item, on, changed) && !SetFlag<u32>(item, on, changed))
			err = "cannot be set by text";
	}
	break;
	case PROP_VECTOR:
	{
		VectorValue* V = dynamic_cast<VectorValue*>(item->GetFrontValue());
		Fvector parsed;
		if (!V || !ParseVector(text, parsed))
		{
			err = "takes three numbers: x, y, z";
			break;
		}
		Fvector value = V->GetValue();
		item->BeforeEdit<VectorValue, Fvector>(value);
		for (int i = 0; i < 3; ++i)
			value[i] = Bounded(V->lim_mn[i], V->lim_mx[i]) ? clampr(parsed[i], V->lim_mn[i], V->lim_mx[i]) : parsed[i];
		changed = item->AfterEdit<VectorValue, Fvector>(value) && item->ApplyValue<VectorValue, Fvector>(value);
	}
	break;
	case PROP_COLOR:
	case PROP_FCOLOR:
	case PROP_VCOLOR:
	{
		u32 argb = 0;
		bool has_alpha = false;
		if (!ParseColor(text, argb, has_alpha))
		{
			err = "takes a colour: #RRGGBB or 0xAARRGGBB";
			break;
		}
		const float r = float((argb >> 16) & 0xff) / 255.f, g = float((argb >> 8) & 0xff) / 255.f, b = float(argb & 0xff) / 255.f;
		if (item->Type() == PROP_COLOR)
		{
			U32Value* V = dynamic_cast<U32Value*>(item->GetFrontValue());
			u32 value = V ? V->GetValue() : 0;
			item->BeforeEdit<U32Value, u32>(value);
			value = has_alpha ? argb : subst_alpha(argb, color_get_A(value));
			changed = item->AfterEdit<U32Value, u32>(value) && item->ApplyValue<U32Value, u32>(value);
		}
		else if (item->Type() == PROP_FCOLOR)
		{
			ColorValue* V = dynamic_cast<ColorValue*>(item->GetFrontValue());
			Fcolor value = V ? V->GetValue() : Fcolor();
			item->BeforeEdit<ColorValue, Fcolor>(value);
			value.set(r, g, b, has_alpha ? float((argb >> 24) & 0xff) / 255.f : value.a);
			changed = item->AfterEdit<ColorValue, Fcolor>(value) && item->ApplyValue<ColorValue, Fcolor>(value);
		}
		else
		{
			Fvector value;
			value.set(r, g, b);
			item->BeforeEdit<VectorValue, Fvector>(value);
			value.set(r, g, b);
			changed = item->AfterEdit<VectorValue, Fvector>(value) && item->ApplyValue<VectorValue, Fvector>(value);
		}
	}
	break;
	case PROP_TOKEN:
		if (!SetToken<u8>(item, text, changed, err) && !SetToken<u16>(item, text, changed, err) && !SetToken<u32>(item, text, changed, err))
			err = "cannot be set by text";
		break;
	case PROP_RTOKEN:
		if (!SetRToken<u8>(item, text, changed, err) && !SetRToken<u16>(item, text, changed, err) && !SetRToken<u32>(item, text, changed, err))
			err = "cannot be set by text";
		break;
	case PROP_SH_TOKEN:
	{
		TokenValueSH* V = dynamic_cast<TokenValueSH*>(item->GetFrontValue());
		const int i = V ? FindName(text, int(V->cnt), [V](int k) { return V->items[k].str; }, err) : -1;
		if (i < 0)
			break;
		err.clear();
		u32 value = V->items[i].ID;
		changed = item->AfterEdit<TokenValueSH, u32>(value) && item->ApplyValue<TokenValueSH, u32>(value);
	}
	break;
	case PROP_CLIST:
	{
		CListValue* V = dynamic_cast<CListValue*>(item->GetFrontValue());
		const int i = V ? FindName(text, int(V->item_count), [V](int k) { return V->items[k].c_str(); }, err) : -1;
		if (i < 0)
			break;
		err.clear();
		if (item->AfterEdit<CListValue, xr_string>(V->items[i]))
			changed = item->ApplyValue<CListValue, LPCSTR>(V->items[i].c_str());
	}
	break;
	case PROP_RLIST:
	{
		RListValue* V = dynamic_cast<RListValue*>(item->GetFrontValue());
		const int i = V ? FindName(text, int(V->item_count), [V](int k) { return V->items[k].c_str(); }, err) : -1;
		if (i < 0)
			break;
		err.clear();
		if (item->AfterEdit<RListValue, shared_str>(V->items[i]))
			changed = item->ApplyValue<RListValue, shared_str>(V->items[i]);
	}
	break;
	case PROP_RTEXT:
	case PROP_STEXT:
	case PROP_CTEXT:
	case PROP_TEXTURE2:
		ApplyText(item, text);
		return true;
	case PROP_CHOOSE:
	{
		shared_str value = text[0] ? text : NONE_CAPTION;
		changed = item->AfterEdit<ChooseValue, shared_str>(value) && item->ApplyValue<ChooseValue, shared_str>(value);
	}
	break;
	case PROP_SHORTCUT:
	{
		ShortcutValue* V = dynamic_cast<ShortcutValue*>(item->GetFrontValue());
		xr_shortcut value;
		if (!V || (text[0] && !UIKeyPressForm::Parse(text, value)))
		{
			err = "takes a shortcut such as Ctrl+Shift+S, or nothing to clear it";
			break;
		}
		changed = V->ApplyValue(value);
	}
	break;
	case PROP_BUTTON:
	{
		ButtonValue* V = dynamic_cast<ButtonValue*>(item->GetFrontValue());
		// a playback control goes by the name its tooltip gives ("First Frame"), not by an engine glyph such as "|<<"
		auto caption = [V](int k)
		{
			const char* name = nullptr;
			XUI_Local::TransportOf(V->value[k].c_str(), &name);
			return name;
		};
		const int i = V ? FindName(text, int(V->value.size()), caption, err) : -1;
		if (i < 0)
			break;
		err.clear();
		V->btn_num = i;
		bool safe = false;
		V->OnBtnClick(safe);
		return true;
	}
	default:
		err = "cannot be set by text";
		break;
	}
	if (!err.empty())
		return false;
	if (changed)
		Modified();
	return true;
}

//---------------------------------------------------------------------------
// The whole text of a row in a dialog of its own: several lines, a long value
//---------------------------------------------------------------------------
void UIPropertiesForm::OpenEditText(PropItem* item)
{
	if (!item)
		return;
	const xr_string text = PropEditText(item);
	m_EditTextKey = item->Key();
	m_EditTextTarget = PropTarget(item);
	m_EditTextBuffer.assign(text.begin(), text.end());
	m_EditTextBuffer.resize(_max(text.size() + 1, size_t(1024)), 0);
	m_EditTextOpen = true;
}

void UIPropertiesForm::DrawEditText()
{
	if (!m_EditTextOpen)
		return;
	const xr_string title = "Edit " + LabelOf(m_EditTextKey.c_str()) + "###dlg.edit_text";
	if (!XUI::BeginDialog(title.c_str(), &m_EditTextOpen, XUI::DialogSize::M, true))
		return;
	XUI::BeginDialogBody();
	// a value kept in a fixed buffer takes no more than that
	int limit = 0;
	if (PropItem* item = FindItem(m_EditTextKey.c_str()))
		if (CTextValue* C = dynamic_cast<CTextValue*>(item->GetFrontValue()))
			limit = C->lim;
	{
		XUI::FontScope mono(XUI::Font::Mono);
		if (ImGui::IsWindowAppearing())
			ImGui::SetKeyboardFocusHere();
		const ImVec2 size(ImGui::GetContentRegionAvail().x, XUI::Pt(260.f));
		if (limit > 0)
		{
			m_EditTextBuffer.resize(size_t(limit), 0);
			ImGui::InputTextMultiline("##text", m_EditTextBuffer.data(), m_EditTextBuffer.size(), size);
		}
		else
			ImGui::InputTextMultiline("##text", m_EditTextBuffer.data(), m_EditTextBuffer.size(), size, ImGuiInputTextFlags_CallbackResize,
				TextBufferResize, &m_EditTextBuffer);
	}
	string64 status = "";
	const u32 length = xr_strlen(m_EditTextBuffer.data());
	if (limit > 0)
		xr_sprintf(status, "%u of %d characters", length, limit - 1);
	if (XUI::BeginDialogFooter(status))
	{
		if (XUI::DialogButton("OK", XUI::Kind::Primary, true))
		{
			PropItem* item = FindItem(m_EditTextKey.c_str());
			// the selection moved on while the dialog was up: its row now belongs to another object
			if (item && PropTarget(item) == m_EditTextTarget)
				ApplyText(item, m_EditTextBuffer.data());
			else
				Msg("! %s: the text was not applied, its row is gone", m_EditTextKey.c_str());
			m_EditTextOpen = false;
			ImGui::CloseCurrentPopup();
		}
		if (XUI::DialogButton("Cancel", XUI::Kind::Secondary))
		{
			m_EditTextOpen = false;
			ImGui::CloseCurrentPopup();
		}
	}
	XUI::EndDialog();
}

//---------------------------------------------------------------------------
// The game types a spawn takes part in
//---------------------------------------------------------------------------
void UIPropertiesForm::OpenEditGameType(PropItem* item)
{
	GameTypeValue* V = item ? dynamic_cast<GameTypeValue*>(item->GetFrontValue()) : nullptr;
	if (!V)
		return;
	m_EditGameTypeChooser = V->GetValue();
	item->BeforeEdit<GameTypeValue, GameTypeChooser>(m_EditGameTypeChooser);
	m_EditGameTypeKey = item->Key();
	m_EditGameTypeOpen = true;
}

void UIPropertiesForm::DrawEditGameType()
{
	if (!m_EditGameTypeOpen)
		return;
	if (!XUI::BeginDialog("Game Types###dlg.game_type", &m_EditGameTypeOpen, XUI::DialogSize::S))
		return;
	XUI::BeginDialogBody();
	static const struct { EGameIDs id; const char* name; } kTypes[] =
	{
		{ eGameIDSingle, "Single Player" },
		{ eGameIDDeathmatch, "Deathmatch" },
		{ eGameIDTeamDeathmatch, "Team Deathmatch" },
		{ eGameIDArtefactHunt, "Artefact Hunt" },
		{ eGameIDCaptureTheArtefact, "Capture the Artefact" },
	};
	for (const auto& type : kTypes)
	{
		bool on = m_EditGameTypeChooser.MatchType(type.id);
		if (ImGui::Checkbox(type.name, &on))
			m_EditGameTypeChooser.m_GameType.set(type.id, on);
	}
	if (XUI::BeginDialogFooter())
	{
		if (XUI::DialogButton("OK", XUI::Kind::Primary, true))
		{
			if (PropItem* item = FindItem(m_EditGameTypeKey.c_str()))
				if (item->AfterEdit<GameTypeValue, GameTypeChooser>(m_EditGameTypeChooser))
					if (item->ApplyValue<GameTypeValue, GameTypeChooser>(m_EditGameTypeChooser))
						Modified();
			m_EditGameTypeOpen = false;
			ImGui::CloseCurrentPopup();
		}
		if (XUI::DialogButton("Cancel", XUI::Kind::Secondary))
		{
			m_EditGameTypeOpen = false;
			ImGui::CloseCurrentPopup();
		}
	}
	XUI::EndDialog();
}
