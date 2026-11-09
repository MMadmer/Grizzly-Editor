#include "stdafx.h"
#include "../Public/ShortcutText.h"

UIKeyPressForm* UIKeyPressForm::Form = nullptr;
bool UIKeyPressForm::s_Closing = false;

namespace
{
constexpr const char* kTitle = "Assign Shortcut###dlg.assign_shortcut";

// the Windows key code the shortcut tables keep for a key ImGui reports; 0 for the modifiers and the rest
u32 VirtualKeyOf(ImGuiKey key)
{
	if (key >= ImGuiKey_A && key <= ImGuiKey_Z)
		return 'A' + u32(key - ImGuiKey_A);
	if (key >= ImGuiKey_0 && key <= ImGuiKey_9)
		return '0' + u32(key - ImGuiKey_0);
	if (key >= ImGuiKey_F1 && key <= ImGuiKey_F24)
		return VK_F1 + u32(key - ImGuiKey_F1);
	if (key >= ImGuiKey_Keypad0 && key <= ImGuiKey_Keypad9)
		return VK_NUMPAD0 + u32(key - ImGuiKey_Keypad0);
	switch (key)
	{
	case ImGuiKey_Tab:				return VK_TAB;
	case ImGuiKey_LeftArrow:		return VK_LEFT;
	case ImGuiKey_RightArrow:		return VK_RIGHT;
	case ImGuiKey_UpArrow:			return VK_UP;
	case ImGuiKey_DownArrow:		return VK_DOWN;
	case ImGuiKey_PageUp:			return VK_PRIOR;
	case ImGuiKey_PageDown:			return VK_NEXT;
	case ImGuiKey_Home:				return VK_HOME;
	case ImGuiKey_End:				return VK_END;
	case ImGuiKey_Insert:			return VK_INSERT;
	case ImGuiKey_Delete:			return VK_DELETE;
	case ImGuiKey_Backspace:		return VK_BACK;
	case ImGuiKey_Space:			return VK_SPACE;
	case ImGuiKey_Enter:			return VK_RETURN;
	case ImGuiKey_Escape:			return VK_ESCAPE;
	case ImGuiKey_Menu:				return VK_APPS;
	case ImGuiKey_Apostrophe:		return VK_OEM_7;
	case ImGuiKey_Comma:			return VK_OEM_COMMA;
	case ImGuiKey_Minus:			return VK_OEM_MINUS;
	case ImGuiKey_Period:			return VK_OEM_PERIOD;
	case ImGuiKey_Slash:			return VK_OEM_2;
	case ImGuiKey_Semicolon:		return VK_OEM_1;
	case ImGuiKey_Equal:			return VK_OEM_PLUS;
	case ImGuiKey_LeftBracket:		return VK_OEM_4;
	case ImGuiKey_Backslash:		return VK_OEM_5;
	case ImGuiKey_RightBracket:		return VK_OEM_6;
	case ImGuiKey_GraveAccent:		return VK_OEM_3;
	case ImGuiKey_CapsLock:			return VK_CAPITAL;
	case ImGuiKey_ScrollLock:		return VK_SCROLL;
	case ImGuiKey_PrintScreen:		return VK_SNAPSHOT;
	case ImGuiKey_Pause:			return VK_PAUSE;
	case ImGuiKey_KeypadDecimal:	return VK_DECIMAL;
	case ImGuiKey_KeypadDivide:		return VK_DIVIDE;
	case ImGuiKey_KeypadMultiply:	return VK_MULTIPLY;
	case ImGuiKey_KeypadSubtract:	return VK_SUBTRACT;
	case ImGuiKey_KeypadAdd:		return VK_ADD;
	case ImGuiKey_KeypadEnter:		return VK_RETURN;
	default:						return 0;
	}
}
}

UIKeyPressForm::UIKeyPressForm()
{
	m_Ok = false;
	m_Captured = false;
	m_Shown = false;
	m_TimeGlobal = 0.f;
}

UIKeyPressForm::~UIKeyPressForm()
{
}

void UIKeyPressForm::Capture(const xr_shortcut& shortcut)
{
	m_Resutl = shortcut;
	m_Captured = true;
	m_ConflictText = m_Conflict ? m_Conflict(shortcut) : xr_string();
}

void UIKeyPressForm::Draw()
{
	// no close cross and no Esc: Esc is a key a command can have
	if (!XUI::BeginDialog(kTitle, nullptr, XUI::DialogSize::S))
	{
		// taken down from outside (MCP ui_dialog close): as Cancel
		if (m_Shown)
		{
			m_Ok = false;
			bOpen = false;
		}
		return;
	}
	m_Shown = true;

	// the first key down after the dialog came up; a key held since before it does not count
	if (!ImGui::IsWindowAppearing())
	{
		const ImGuiIO& io = ImGui::GetIO();
		for (int key = ImGuiKey_NamedKey_BEGIN; key < ImGuiKey_NamedKey_END; ++key)
			if (ImGui::IsKeyPressed(ImGuiKey(key), false))
				if (const u32 vk = VirtualKeyOf(ImGuiKey(key)))
				{
					Capture(xr_shortcut(u8(vk), io.KeyAlt, io.KeyCtrl, io.KeyShift));
					break;
				}
	}

	XUI::BeginDialogBody();
	if (!m_What.empty())
	{
		ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::Vec4(GrizzlyTheme::ColorToken::TextSecondary));
		XUI::TextEllipsis(m_What.c_str());
		ImGui::PopStyleColor();
	}
	{
		XUI::FontScope title(XUI::Font::Title);
		ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::Vec4(m_Captured ? GrizzlyTheme::ColorToken::Text : GrizzlyTheme::ColorToken::TextMuted));
		const xr_string shown = m_Captured ? ShortcutText(m_Resutl) : xr_string("Press a key or a combination");
		XUI::TextEllipsis(shown.c_str());
		ImGui::PopStyleColor();
	}
	ImGui::Spacing();
	XUI::StatusText(XUI::Tone::Muted, "Every key counts, Esc and Enter included. Cancel keeps the shortcut as it is.");
	if (m_Captured && !m_ConflictText.empty())
	{
		const xr_string text = "Already the shortcut of " + m_ConflictText + ". Assigning it here takes it from there.";
		XUI::StatusText(XUI::Tone::Warning, text.c_str());
	}
	// the buttons take clicks only: Enter, Space and the arrows are keys to capture, not keyboard navigation
	ImGui::PushItemFlag(ImGuiItemFlags_NoNav, true);
	if (XUI::BeginDialogFooter())
	{
		if (XUI::DialogButton("Assign", XUI::Kind::Primary, false, m_Captured, "Press a key first"))
		{
			m_Ok = true;
			bOpen = false;
			ImGui::CloseCurrentPopup();
		}
		if (XUI::DialogButton("Cancel", XUI::Kind::Secondary))
		{
			m_Ok = false;
			bOpen = false;
			ImGui::CloseCurrentPopup();
		}
	}
	ImGui::PopItemFlag();
	XUI::EndDialog();
}

void UIKeyPressForm::Update(float timeGlobal)
{
	if (!Form)
	{
		// the form went away while its dialog was up (MCP, a closed document): the popup goes with it
		if (s_Closing)
		{
			s_Closing = false;
			bool open = false;
			if (XUI::BeginDialog(kTitle, &open, XUI::DialogSize::S))
				XUI::EndDialog();
		}
		return;
	}
	Form->m_TimeGlobal = timeGlobal;
	if (!Form->IsClosed())
		Form->Draw();
}

void UIKeyPressForm::Show(LPCSTR what, const TConflict& conflict)
{
	VERIFY(!Form);
	Form = xr_new<UIKeyPressForm>();
	Form->m_What = what ? what : "";
	Form->m_Conflict = conflict;
}

bool UIKeyPressForm::SetResult(const xr_shortcut& result)
{
	if (Form && !Form->IsClosed())
	{
		Form->Capture(result);
		return true;
	}
	return false;
}

bool UIKeyPressForm::GetResult(bool& Ok, xr_shortcut& result)
{
	if (Form && Form->IsClosed())
	{
		Ok = Form->m_Ok && Form->m_Captured;
		result = Form->m_Resutl;
		s_Closing = Form->m_Shown;
		xr_delete(Form);
		return true;
	}
	return false;
}

bool UIKeyPressForm::Parse(LPCSTR text, xr_shortcut& out)
{
	if (!text)
		return false;
	while (*text == ' ')
		++text;
	BOOL ctrl = FALSE, shift = FALSE, alt = FALSE;
	// the modifiers lead; the rest is the key's own name, which may hold a '+' itself ("Num +")
	for (;;)
	{
		if (!_strnicmp(text, "Ctrl+", 5))
			ctrl = TRUE, text += 5;
		else if (!_strnicmp(text, "Shift+", 6))
			shift = TRUE, text += 6;
		else if (!_strnicmp(text, "Alt+", 4))
			alt = TRUE, text += 4;
		else
			break;
	}
	if (!*text)
		return false;
	for (u32 vk = 1; vk < 255; ++vk)
	{
		const xr_shortcut candidate(u8(vk), FALSE, FALSE, FALSE);
		const xr_string name = ShortcutText(candidate);
		// a code without a name of its own ("Key 146") is no key the menus show
		if (!name.empty() && _strnicmp(name.c_str(), "Key ", 4) && !_stricmp(name.c_str(), text))
		{
			out = xr_shortcut(u8(vk), alt, ctrl, shift);
			return true;
		}
	}
	return false;
}
