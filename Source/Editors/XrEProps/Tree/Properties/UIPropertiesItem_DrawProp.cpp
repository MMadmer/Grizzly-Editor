#include "stdafx.h"
#include "../../UIPropsLocal.h"
//-----------------------------------------------------------------------------------------
// The value column of a property row (docs/UI_DESIGN.md, section 4 "Property grid"): numbers are drags without
// steppers, vectors have their axes tagged, colours are a swatch and a hex, choosers are pick fields, text is
// typed in place (a longer one in a dialog), and read-only values are selectable text.
//-----------------------------------------------------------------------------------------
#define TSTRING_COUNT 	10
const LPSTR TEXTUREString[TSTRING_COUNT] = { "Choose Texture...","$null","$base0", "$base1" ,"$base2" ,"$base3" ,"$base4","$base5" ,"$base6" ,"$base7" };

bool PropIsAngle(PropValue* value);

namespace
{
// a number the limits leave room for is a range; flt_min / flt_max and the like are no limits at all
template <typename T>
bool Bounded(T mn, T mx)
{
	return mn < mx && double(mx) - double(mn) < 1e30;
}

template <typename T> ImGuiDataType DataTypeOf();
template <> ImGuiDataType DataTypeOf<u8>() { return ImGuiDataType_U8; }
template <> ImGuiDataType DataTypeOf<s8>() { return ImGuiDataType_S8; }
template <> ImGuiDataType DataTypeOf<u16>() { return ImGuiDataType_U16; }
template <> ImGuiDataType DataTypeOf<s16>() { return ImGuiDataType_S16; }
template <> ImGuiDataType DataTypeOf<u32>() { return ImGuiDataType_U32; }
template <> ImGuiDataType DataTypeOf<s32>() { return ImGuiDataType_S32; }
template <> ImGuiDataType DataTypeOf<float>() { return ImGuiDataType_Float; }

// Before- and AfterEdit always come in pairs: an angle row turns its value and its limits into degrees in the
// first and back in the second. `unit` is written after the number (typing the value takes the number alone).
template<typename T>
bool DrawNumeric(PropItem* item, bool& change, const xr_string& unit)
{
	change = false;
	NumericValue<T>* V = dynamic_cast<NumericValue<T>*>(item->GetFrontValue());
	if (!V)					return false;
	T value = *V->value;
	item->BeforeEdit<NumericValue<T>, T>(value);
	T mn = V->lim_mn, mx = V->lim_mx;
	const bool bounded = Bounded(mn, mx);
	float speed = 0.f;
	string64 format = "%d";
	if constexpr (std::is_floating_point_v<T>)
	{
		speed = PropIsAngle(V) ? .5f : (V->inc > 0.f ? float(V->inc) : .01f);
		sprintf_s(format, "%%.%df", clampr(V->dec, 0, 8));
	}
	else
	{
		speed = .25f * _max(1.f, float(V->inc));
		if (std::is_unsigned_v<T>)
			xr_strcpy(format, "%u");
	}
	if (!unit.empty())
	{
		// a degree sign follows the number, a word is set apart from it; a % of the unit is no conversion
		if (unit != "\xC2\xB0" && unit != "%")
			xr_strcat(format, " ");
		for (char c : unit)
		{
			const char piece[3] = { c, c == '%' ? '%' : '\0', '\0' };
			xr_strcat(format, piece);
		}
	}
	change = ImGui::DragScalar("##value", DataTypeOf<T>(), &value, speed, bounded ? &mn : nullptr, bounded ? &mx : nullptr, format,
		bounded ? ImGuiSliderFlags_AlwaysClamp : ImGuiSliderFlags_None);
	if (change)
	{
		if (bounded)
			value = clampr(value, mn, mx);
		change = item->AfterEdit<NumericValue<T>, T>(value) && item->ApplyValue<NumericValue<T>, T>(value);
	}
	else
		item->AfterEdit<NumericValue<T>, T>(value);
	return true;
}

//-----------------------------------------------------------------------------------------
template <class T>
BOOL TokenOnEdit(PropItem* prop, bool& change, bool mixed)
{
	TokenValue<T>* V = dynamic_cast<TokenValue<T>*>(prop->GetFrontValue());
	if (!V)					return FALSE;
	T edit_value  = V->GetValue();
	prop->BeforeEdit<TokenValue<T>, T>(edit_value);
	int index = -1;
	xr_token* token_list = V->token;
	int cnt = 0;
	for (; token_list[cnt].name; cnt++)
		if (token_list[cnt].id == int(edit_value))
			index = cnt;
	if (XUI_Local::PickFromList(index >= 0 ? token_list[index].name : "Not in the list", cnt, &index, mixed,
		[token_list](int i) { return token_list[i].name; }))
	{
		T new_val = T(token_list[index].id);
		if (prop->AfterEdit<TokenValue<T>, T>(new_val))
			change = prop->ApplyValue<TokenValue<T>, T>(new_val);
	}
	return TRUE;
}

template <class T>
BOOL RTokenOnEdit(PropItem* prop, bool& change, bool mixed)
{
	RTokenValue<T>* V = dynamic_cast<RTokenValue<T>*>(prop->GetFrontValue());
	if (!V)					return FALSE;
	T edit_value = V->GetValue();
	prop->BeforeEdit<RTokenValue<T>, T>(edit_value);
	int index = -1;
	xr_rtoken* token_list = V->token;
	const int cnt = int(V->token_count);
	for (int i = 0; i < cnt; i++)
		if (T(token_list[i].id) == edit_value)
			index = i;
	if (XUI_Local::PickFromList(index >= 0 ? token_list[index].name.c_str() : "Not in the list", cnt, &index, mixed,
		[token_list](int i) { return token_list[i].name.c_str(); }))
	{
		T new_val = T(token_list[index].id);
		if (prop->AfterEdit<RTokenValue<T>, T>(new_val))
			change = prop->ApplyValue<RTokenValue<T>, T>(new_val);
	}
	return TRUE;
}

template <class T>
BOOL FlagOnEdit(PropItem* prop, bool& change)
{
	FlagValue<_flags<T> >* V = dynamic_cast<FlagValue<_flags<T> >*>(prop->GetFrontValue());
	if (!V)					return FALSE;
	_flags<T> new_val = V->GetValue();

	prop->BeforeEdit<FlagValue<_flags<T> >, _flags<T> >(new_val);
	u32 u = new_val.get();

	if (ImGui::CheckboxFlags("##value", &u, V->mask))
	{
		new_val.assign(T(u));
		if (prop->AfterEdit<FlagValue<_flags<T> >, _flags<T> >(new_val))
			change = prop->ApplyValue<FlagValue<_flags<T> >, _flags<T> >(new_val);
	}
	return TRUE;
}
}

//-----------------------------------------------------------------------------------------
// The text a row is edited as: its value through BeforeEdit (a library name shows its leaf)
xr_string PropEditText(PropItem* item)
{
	switch (item->Type())
	{
	case PROP_RTEXT:
	case PROP_CHOOSE:
		if (RTextValue* V = dynamic_cast<RTextValue*>(item->GetFrontValue()))
		{
			shared_str value = V->GetValue();
			item->BeforeEdit<RTextValue, shared_str>(value);
			return value.c_str() ? value.c_str() : "";
		}
		break;
	case PROP_STEXT:
		if (STextValue* V = dynamic_cast<STextValue*>(item->GetFrontValue()))
		{
			xr_string value = V->GetValue();
			item->BeforeEdit<STextValue, xr_string>(value);
			return value;
		}
		break;
	case PROP_CTEXT:
	case PROP_TEXTURE2:
	case PROP_CLIST:
		if (CTextValue* V = dynamic_cast<CTextValue*>(item->GetFrontValue()))
		{
			xr_string value = V->GetValue() ? V->GetValue() : "";
			item->BeforeEdit<CTextValue, xr_string>(value);
			return value;
		}
		break;
	default:
		break;
	}
	return item->GetDrawText();
}

// A text of several lines by nature (an ini text: a spawn's or the level's custom data, an object's user data) or one that
// holds several already. Only such a row offers the Edit window; a name or a path is typed in place, and that is all.
bool PropTakesLines(PropItem* item)
{
	if (PropEditText(item).find('\n') != xr_string::npos)
		return true;
	static const char* kLineTexts[] = { "Custom data", "User Data" };
	const xr_string label = UIPropertiesForm::LabelOf(item->Key());
	for (const char* name : kLineTexts)
		if (!_stricmp(label.c_str(), name))
			return true;
	return false;
}

namespace
{
// a row of playback controls (every caption names one): icon buttons instead of captions
bool IsTransportRow(ButtonValue* V)
{
	if (!V || V->value.empty())
		return false;
	for (const shared_str& caption : V->value)
		if (XUI_Local::TransportOf(caption.c_str()) == XUI_Local::Transport::None)
			return false;
	return true;
}
}

// What a text row writes into: the same for the same object across rebuilds, so an edit finished after the
// selection moved on is not written into another object's row of the same name
const void* PropTarget(PropItem* item)
{
	PropValue* front = item ? item->GetFrontValue() : nullptr;
	if (RTextValue* V = dynamic_cast<RTextValue*>(front))
		return V->value;
	if (STextValue* V = dynamic_cast<STextValue*>(front))
		return V->value;
	if (CTextValue* V = dynamic_cast<CTextValue*>(front))
		return V->value;
	return front;
}

void UIPropertiesItem::DrawTextValue()
{
	UIPropertiesForm* F = PropertiesFrom;
	const xr_string key = PItem->Key();
	const xr_string value = PropEditText(PItem);
	const size_t eol = value.find('\n');
	if (eol != xr_string::npos)
	{
		// several lines: the first one shows, the dialog has them all
		size_t lines = 1;
		for (char c : value)
			lines += c == '\n';
		string64 tip;
		xr_sprintf(tip, "%u lines. Click to edit them.", u32(lines));
		const xr_string first = value.substr(0, eol);
		if (XUI_Local::FrameText("##lines", first.c_str(), false, tip))
			F->OpenEditText(PItem);
	}
	else
	{
		// typed in place and written when the field is left (Enter or a click elsewhere); Esc takes it back
		size_t capacity = 4096;
		if (CTextValue* C = dynamic_cast<CTextValue*>(PItem->GetFrontValue()))
			if (C->lim > 0)
				capacity = size_t(C->lim);
		capacity = _max(capacity, value.size() + 1);
		const bool editing = F->m_InlineKey == key;
		static xr_vector<char> s_Scratch;
		char* buffer = nullptr;
		if (editing)
		{
			if (F->m_InlineBuffer.size() < capacity)
				F->m_InlineBuffer.resize(capacity, 0);
			buffer = F->m_InlineBuffer.data();
		}
		else
		{
			s_Scratch.assign(capacity, 0);
			memcpy(s_Scratch.data(), value.c_str(), value.size());
			buffer = s_Scratch.data();
		}
		ImGui::InputText("##value", buffer, capacity);
		if (ImGui::IsItemActivated())
		{
			F->m_InlineKey = key;
			F->m_InlineTarget = PropTarget(PItem);
			F->m_InlineBuffer.assign(buffer, buffer + capacity);
		}
		if (ImGui::IsItemDeactivated())
		{
			if (editing && ImGui::IsItemDeactivatedAfterEdit() && F->m_InlineTarget == PropTarget(PItem))
				F->ApplyText(PItem, F->m_InlineBuffer.data());
			F->m_InlineKey.clear();
			F->m_InlineTarget = nullptr;
		}
	}
	if ((eol != xr_string::npos || PropTakesLines(PItem)) &&
		XUI::PropertyTrailing("##edit_text", XUI::Icon::Rename, "Edit in a window, several lines included"))
		F->OpenEditText(PItem);
}

void UIPropertiesItem::DrawButtons()
{
	ButtonValue* V = dynamic_cast<ButtonValue*>(PItem->GetFrontValue()); R_ASSERT(V);
	if (V->value.empty())
		return;
	const ImGuiStyle& style = ImGui::GetStyle();
	const float w = _max(1.f, ImGui::CalcItemWidth());
	const float gap = style.ItemInnerSpacing.x;
	const int n = int(V->value.size());
	const ImVec2 start = ImGui::GetCursorScreenPos();
	const float step = ImGui::GetFrameHeight() + style.ItemSpacing.y;
	auto press = [V](int i)
	{
		V->btn_num = i;
		bool bSafe = false;
		V->OnBtnClick(bSafe);
	};
	if (IsTransportRow(V))
	{
		// playback controls: icon buttons from the left, a line after another when they do not fit
		const float each = clampr((w - gap * float(n - 1)) / float(n), XUI::Pt(22.f), XUI::Pt(30.f));
		const int per_line = _max(1, int((w + gap) / (each + gap)));
		for (int i = 0; i < n; ++i)
		{
			ImGui::SetCursorScreenPos(ImVec2(start.x + (each + gap) * float(i % per_line), start.y + step * float(i / per_line)));
			ImGui::PushID(i);
			const char* name = nullptr;
			const XUI_Local::Transport glyph = XUI_Local::TransportOf(V->value[i].c_str(), &name);
			if (XUI_Local::TransportButton("##transport", glyph, name, ImVec2(each, ImGui::GetFrameHeight())))
				press(i);
			ImGui::PopID();
		}
		return;
	}
	const float each = (w - gap * float(n - 1)) / float(n);
	// side by side while every caption fits, one under another otherwise: a caption is never cut
	bool fits = true;
	for (const shared_str& caption : V->value)
		if (ImGui::CalcTextSize(caption.c_str()).x + style.FramePadding.x * 2.f > each)
			fits = false;
	for (int i = 0; i < n; ++i)
	{
		ImGui::SetCursorScreenPos(fits ? ImVec2(start.x + (each + gap) * float(i), start.y) : ImVec2(start.x, start.y + step * float(i)));
		ImGui::PushID(i);
		if (ImGui::Button(V->value[i].c_str(), ImVec2(fits ? each : w, 0.f)))
			press(i);
		ImGui::PopID();
	}
}

float UIPropertiesItem::OneLineWidth() const
{
	if (!PItem)
		return 0.f;
	float width = 0.f;
	switch (PItem->Type())
	{
	case PROP_VECTOR:
	{
		// a read-only or mixed vector is one line of text; an edited one is XUI::Vec3Field, side by side from 15 em on
		const bool read_only = PropertiesFrom->IsReadOnly() || PItem->m_Flags.test(PropItem::flDisabled);
		const bool mixed = PItem->m_Flags.test(PropItem::flMixed) && !PItem->m_Flags.test(PropItem::flIgnoreMixed);
		if (!read_only && !mixed)
			width = XUI::Em(15.f);
	}
	break;
	case PROP_BUTTON:
		// its captions side by side, each as wide as the widest: the buttons, or a Segmented a host draws them as; playback
		// controls are icons of a comfortable width
		if (ButtonValue* V = dynamic_cast<ButtonValue*>(PItem->GetFrontValue()))
		{
			if (IsTransportRow(V))
			{
				const float n = float(V->value.size());
				width = XUI::Pt(28.f) * n + ImGui::GetStyle().ItemInnerSpacing.x * (n - 1.f);
				break;
			}
			float widest = 0.f;
			for (const shared_str& caption : V->value)
				widest = _max(widest, ImGui::CalcTextSize(caption.c_str()).x);
			if (!V->value.empty())
				width = (widest + XUI::Pt(20.f)) * float(V->value.size()) + XUI::Pt(4.f);
		}
		break;
	default:
		break;
	}
	// a switch of its own goes before the value
	if (width > 0.f && PItem->m_Flags.test(PropItem::flShowCB))
		width += ImGui::GetFrameHeight() + ImGui::GetStyle().ItemInnerSpacing.x;
	return width;
}

bool UIPropertiesItem::TakesTrailing() const
{
	if (!PItem)
		return false;
	switch (PItem->Type())
	{
	case PROP_CTEXT:
	case PROP_RTEXT:
	case PROP_STEXT:
		return PropTakesLines(PItem);
	case PROP_SHORTCUT:
		return true;
	default:
		return false;
	}
}

void UIPropertiesItem::DrawCanvas()
{
	CanvasValue* val = dynamic_cast<CanvasValue*>(PItem->GetFrontValue()); R_ASSERT(val);
	if (!val->OnDrawCanvasEvent.empty())
		val->OnDrawCanvasEvent(val);
}

void UIPropertiesItem::DrawValue()
{
	if (!PItem)
		return;
	UIPropertiesForm* F = PropertiesFrom;
	if (F->m_ValueDrawer && F->m_ValueDrawer(PItem))
		return;

	const EPropType type = PItem->Type();
	const bool read_only = F->IsReadOnly() || PItem->m_Flags.test(PropItem::flDisabled);
	const bool mixed = PItem->m_Flags.test(PropItem::flMixed) && !PItem->m_Flags.test(PropItem::flIgnoreMixed);

	// the row's own switch comes first, the value takes what is left
	if (PItem->m_Flags.test(PropItem::flShowCB))
	{
		const float w = ImGui::CalcItemWidth();
		if (ImGui::CheckboxFlags("##enabled", &PItem->m_Flags.flags, PropItem::flCBChecked))
		{
			PItem->OnChange();
			F->Modified();
		}
		ImGui::SameLine(0.f, ImGui::GetStyle().ItemInnerSpacing.x);
		ImGui::SetNextItemWidth(_max(1.f, w - ImGui::GetFrameHeight() - ImGui::GetStyle().ItemInnerSpacing.x));
	}

	switch (type)
	{
	case PROP_WAVE:
	case PROP_UNDEF:
		XUI::PropertyText(PItem->GetDrawText().c_str());
		return;
	case PROP_CAPTION:
	{
		const xr_string text = PItem->GetDrawText();
		if (text.empty())
			XUI_Local::MutedValue("None");
		else
			XUI::PropertyText(text.c_str());
		return;
	}
	case PROP_CANVAS:
		if (mixed)
			XUI_Local::MutedValue("Multiple values");
		else
			DrawCanvas();
		return;
	case PROP_BUTTON:
		ImGui::BeginDisabled(read_only);
		DrawButtons();
		ImGui::EndDisabled();
		return;
	default:
		break;
	}

	if (read_only)
	{
		// a switch stays a switch, dimmed; the rest is text that can be selected and copied
		if (type == PROP_BOOLEAN || type == PROP_FLAG)
		{
			bool value = false;
			if (FlagValueCustom* V = dynamic_cast<FlagValueCustom*>(PItem->GetFrontValue()))
				value = V->GetValueEx();
			else if (BOOLValue* B = dynamic_cast<BOOLValue*>(PItem->GetFrontValue()))
				value = !!B->GetValue();
			ImGui::BeginDisabled();
			ImGui::Checkbox("##value", &value);
			ImGui::EndDisabled();
		}
		else
			XUI::PropertyText(PItem->GetDrawText().c_str());
		return;
	}

	// several objects that differ: a switch, a list or a picker shows it itself; the rest offer to align them
	const bool shows_mixed = type == PROP_BOOLEAN || type == PROP_FLAG || type == PROP_CHOOSE || type == PROP_TOKEN ||
		type == PROP_RTOKEN || type == PROP_SH_TOKEN || type == PROP_CLIST || type == PROP_RLIST;
	if (mixed && !shows_mixed)
	{
		if (XUI_Local::MixedField())
			RemoveMixed();
		return;
	}

	switch (type)
	{
	case PROP_NUMERIC:
	case PROP_TIME:
	{
		bool change = false;
		const xr_string unit = F->GetUnit(PItem);
		if (!DrawNumeric<u32>(PItem, change, unit))
			if (!DrawNumeric<float>(PItem, change, unit))
				if (!DrawNumeric<u8>(PItem, change, unit))
					if (!DrawNumeric<s8>(PItem, change, unit))
						if (!DrawNumeric<s16>(PItem, change, unit))
							if (!DrawNumeric<u16>(PItem, change, unit))
								if (!DrawNumeric<s32>(PItem, change, unit))
									R_ASSERT(false);
		if (change)
			F->Modified();
	}
	break;
	case PROP_SHORTCUT:
	{
		ShortcutValue* V = dynamic_cast<ShortcutValue*>(PItem->GetFrontValue()); R_ASSERT(V);
		const xr_string text = V->GetDrawText(0);
		if (XUI_Local::FrameText("##shortcut", text.c_str(), !V->value->key, "Click to assign another shortcut"))
		{
			UIKeyPressForm::Show(UIPropertiesForm::LabelOf(PItem->Key()).c_str());
			F->m_EditShortcutValue = PItem;
		}
		if (XUI::PropertyTrailing("##clear_shortcut", XUI::Icon::Clear, "Remove the shortcut"))
		{
			xr_shortcut none;
			if (V->ApplyValue(none))
				F->Modified();
		}
	}
	break;
	case PROP_BOOLEAN:
	{
		BOOLValue* V = dynamic_cast<BOOLValue*>(PItem->GetFrontValue()); VERIFY(V);
		BOOL new_val_as_BOOL = V->GetValue();
		PItem->BeforeEdit<BOOLValue, BOOL>(new_val_as_BOOL);
		bool new_val = new_val_as_BOOL;
		if (ImGui::Checkbox("##value", &new_val))
		{
			new_val_as_BOOL = new_val;
			if (PItem->AfterEdit<BOOLValue, BOOL>(new_val_as_BOOL))
				if (PItem->ApplyValue<BOOLValue, BOOL>(new_val_as_BOOL))
					F->Modified();
		}
	}
	break;
	case PROP_FLAG:
	{
		bool change = false;
		if (!FlagOnEdit<u8>(PItem, change))
			if (!FlagOnEdit<u16>(PItem, change))
				if (!FlagOnEdit<u32>(PItem, change))
					R_ASSERT(false);
		if (change)F->Modified();
	}
	break;
	case PROP_VECTOR:
	{
		VectorValue* V = dynamic_cast<VectorValue*>(PItem->GetFrontValue()); R_ASSERT(V);
		Fvector edit_val = V->GetValue();
		PItem->BeforeEdit<VectorValue, Fvector>(edit_val);
		float vector[3] = { edit_val.x,edit_val.y,edit_val.z };
		const float speed = PropIsAngle(V) ? .5f : (V->inc.x > 0.f ? V->inc.x : .01f);
		if (XUI::Vec3Field("##value", vector, speed))
		{
			for (int i = 0; i < 3; i++)
			{
				if (Bounded(V->lim_mn[i], V->lim_mx[i]))
					vector[i] = clampr(vector[i], V->lim_mn[i], V->lim_mx[i]);
				edit_val[i] = vector[i];
			}
			if (PItem->AfterEdit<VectorValue, Fvector>(edit_val))
				if (PItem->ApplyValue<VectorValue, Fvector>(edit_val))
					F->Modified();
		}
		else
			PItem->AfterEdit<VectorValue, Fvector>(edit_val);
	}
	break;
	case PROP_COLOR:
	{
		U32Value* V = dynamic_cast<U32Value*>(PItem->GetFrontValue()); R_ASSERT(V);
		u32 edit_val = V->GetValue();
		PItem->BeforeEdit<U32Value, u32>(edit_val);
		const u32 a = color_get_A(edit_val);
		float color[3] = { color_get_R(edit_val) / 255.f, color_get_G(edit_val) / 255.f, color_get_B(edit_val) / 255.f };
		if (XUI_Local::ColorField("##value", color))
		{
			edit_val = subst_alpha(color_rgba_f(color[0], color[1], color[2], 1.f), a);
			if (PItem->AfterEdit<U32Value, u32>(edit_val))
				if (PItem->ApplyValue<U32Value, u32>(edit_val))
					F->Modified();
		}
	}
	break;
	case PROP_FCOLOR:
	{
		ColorValue* V = dynamic_cast<ColorValue*>(PItem->GetFrontValue()); R_ASSERT(V);
		Fcolor edit_val = V->GetValue();
		PItem->BeforeEdit<ColorValue, Fcolor>(edit_val);
		float color[3] = { edit_val.r,edit_val.g,edit_val.b };
		if (XUI_Local::ColorField("##value", color))
		{
			edit_val.r = color[0];
			edit_val.g = color[1];
			edit_val.b = color[2];
			if (PItem->AfterEdit<ColorValue, Fcolor>(edit_val))
				if (PItem->ApplyValue<ColorValue, Fcolor>(edit_val))
					F->Modified();
		}
	}
	break;
	case PROP_VCOLOR:
	{
		VectorValue* V = dynamic_cast<VectorValue*>(PItem->GetFrontValue()); R_ASSERT(V);
		Fvector edit_val = V->GetValue();
		PItem->BeforeEdit<VectorValue, Fvector>(edit_val);
		float color[3] = { edit_val[0],edit_val[1],edit_val[2] };
		if (XUI_Local::ColorField("##value", color))
		{
			edit_val.set(color[0], color[1], color[2]);
			if (PItem->AfterEdit<VectorValue, Fvector>(edit_val))
				if (PItem->ApplyValue<VectorValue, Fvector>(edit_val))
					F->Modified();
		}
		else
			PItem->AfterEdit<VectorValue, Fvector>(edit_val);
	}
	break;
	case PROP_CHOOSE:
	{
		ChooseValue* V = dynamic_cast<ChooseValue*>(PItem->GetFrontValue()); VERIFY(V);
		const xr_string text = PItem->GetDrawText();
		// the chooser's own None is no name to show
		const char* shown = text == NONE_CAPTION ? "" : text.c_str();
		const bool clearable = V && (V->m_ChooseFlags & cfAllowNone) && !(V->m_ChooseFlags & cfMultiSelect);
		const int picked = XUI::PickField("##value", shown, clearable);
		if (picked == XUI::PickBrowse)
			F->BeginChooseEdit(PItem);
		else if (picked == XUI::PickClear)
		{
			// exactly what the chooser's None row gives
			shared_str none = NONE_CAPTION;
			if (PItem->AfterEdit<ChooseValue, shared_str>(none))
				if (PItem->ApplyValue<ChooseValue, shared_str>(none))
					F->Modified();
		}
	}
	break;
	case PROP_TOKEN:
	{
		bool change = false;
		if (!TokenOnEdit<u8>(PItem, change, mixed))
			if (!TokenOnEdit<u16>(PItem, change, mixed))
				if (!TokenOnEdit<u32>(PItem, change, mixed))
					R_ASSERT(false);
		if (change)
			F->Modified();
	}
	break;
	case PROP_RTOKEN:
	{
		bool change = false;
		if (!RTokenOnEdit<u8>(PItem, change, mixed))
			if (!RTokenOnEdit<u16>(PItem, change, mixed))
				if (!RTokenOnEdit<u32>(PItem, change, mixed))
					R_ASSERT(false);
		if (change)
			F->Modified();
	}
	break;
	case PROP_SH_TOKEN:
	{
		TokenValueSH* V = dynamic_cast<TokenValueSH*>(PItem->GetFrontValue()); R_ASSERT(V);
		u32 edit_value = V->GetValue();
		PItem->BeforeEdit<TokenValueSH, u32>(edit_value);
		int index = -1;
		const int cnt = int(V->cnt);
		for (int i = 0; i < cnt; i++)
			if (V->items[i].ID == edit_value)
				index = i;
		const TokenValueSH::Item* items = V->items;
		if (XUI_Local::PickFromList(index >= 0 ? items[index].str : "Not in the list", cnt, &index, mixed, [items](int i) { return items[i].str; }))
		{
			u32 new_val = items[index].ID;
			if (PItem->AfterEdit<TokenValueSH, u32>(new_val))
				if (PItem->ApplyValue<TokenValueSH, u32>(new_val))
					F->Modified();
		}
	}
	break;
	case PROP_TEXTURE2:
	{
		CTextValue* T = dynamic_cast<CTextValue*>(PItem->GetFrontValue()); R_ASSERT(T);
		xr_string edit_val = T->GetValue();
		PItem->BeforeEdit<CTextValue, xr_string>(edit_val);
		int index = 0; int cnt = TSTRING_COUNT ;
		LPCSTR List[TSTRING_COUNT+1] = { TEXTUREString[0],TEXTUREString[1],TEXTUREString[2],TEXTUREString[3],TEXTUREString[4],TEXTUREString[5],TEXTUREString[6],TEXTUREString[7],TEXTUREString[8],TEXTUREString[9],0 };
		if(edit_val == List[1])index = 1;
		else if (edit_val == List[2])index = 2;
		else if (!edit_val.empty()) { List[3] = edit_val.c_str(); index = 3; cnt = 4; }
		else index = 1;
		if (XUI_Local::PickFromList(List[index], cnt, &index, false, [&List](int i) { return List[i]; }))
		{
			if (index == 0)
			{
				UIChooseForm::SelectItem(smTexture, 8, edit_val.c_str(),0,0,0,0,0);
				F->m_EditTextureValue = PItem;
				F->m_ChooserOwned = true;
			}
			else
			{
				edit_val = List[index];
				if (PItem->AfterEdit<CTextValue, xr_string>(edit_val))
					if (PItem->ApplyValue<CTextValue, LPCSTR>(edit_val.c_str()))
						F->Modified();
			}
		}
	}
	break;
	case PROP_CLIST:
	{
		CListValue* V = dynamic_cast<CListValue*>(PItem->GetFrontValue()); R_ASSERT(V);
		LPCSTR edit_value = V->value;
		int index = -1;
		const int cnt = int(V->item_count);
		for (int i = 0; i < cnt; i++)
			if (V->items[i] == edit_value)
				index = i;
		xr_string* items = V->items;
		if (XUI_Local::PickFromList(index >= 0 ? items[index].c_str() : edit_value, cnt, &index, mixed, [items](int i) { return items[i].c_str(); }))
		{
			if (PItem->AfterEdit<CListValue, xr_string>(V->items[index]))
				if (PItem->ApplyValue<CListValue, LPCSTR>(V->items[index].c_str()))
					F->Modified();
		}
	}
	break;
	case PROP_RLIST:
	{
		RListValue* V = dynamic_cast<RListValue*>(PItem->GetFrontValue()); R_ASSERT(V);
		LPCSTR edit_value = V->value? V->value->c_str():0;
		int index = -1;
		const int cnt = int(V->item_count);
		for (int i = 0; i < cnt; i++)
			if (V->items[i] == edit_value)
				index = i;
		shared_str* items = V->items;
		if (XUI_Local::PickFromList(index >= 0 ? items[index].c_str() : edit_value, cnt, &index, mixed, [items](int i) { return items[i].c_str(); }))
		{
			if (PItem->AfterEdit<RListValue, shared_str>(V->items[index]))
				if (PItem->ApplyValue<RListValue, shared_str>(V->items[index]))
					F->Modified();
		}
	}
	break;
	case PROP_CTEXT:
	case PROP_RTEXT:
	case PROP_STEXT:
		DrawTextValue();
		break;
	case PROP_GAMETYPE:
		if (XUI_Local::FrameText("##game_type", PItem->GetDrawText().c_str(), false, "Click to choose the game types"))
			F->OpenEditGameType(PItem);
		break;
	default:
		XUI::PropertyText(PItem->GetDrawText().c_str());
		break;
	}
}
