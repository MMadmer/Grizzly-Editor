#pragma once

// A shortcut the way the menus, the property grid and the command palette print it. The key names
// are fixed English ones, never the active keyboard layout's: a Russian layout turned '[' into a
// Cyrillic letter. Every key has a name - an unknown one is 'Key <code>', not an assertion (the
// Preferences window used to stop the editor on Esc and Num Del).
inline xr_string ShortcutText(const xr_shortcut& shortcut)
{
	if (!shortcut.key)
		return {};

	xr_string text;
	if (shortcut.ext.test(xr_shortcut::flCtrl))		text += "Ctrl+";
	if (shortcut.ext.test(xr_shortcut::flShift))	text += "Shift+";
	if (shortcut.ext.test(xr_shortcut::flAlt))		text += "Alt+";

	const u32 key = shortcut.key;
	char buf[16];
	if ((key >= 'A' && key <= 'Z') || (key >= '0' && key <= '9'))
	{
		text += char(key);
		return text;
	}
	if (key >= VK_F1 && key <= VK_F24)
	{
		sprintf_s(buf, "F%u", key - VK_F1 + 1);
		return text + buf;
	}
	if (key >= VK_NUMPAD0 && key <= VK_NUMPAD9)
	{
		sprintf_s(buf, "Num %u", key - VK_NUMPAD0);
		return text + buf;
	}

	const char* name = nullptr;
	switch (key)
	{
	case VK_ADD:		name = "Num +";			break;
	case VK_SUBTRACT:	name = "Num -";			break;
	case VK_MULTIPLY:	name = "Num *";			break;
	case VK_DIVIDE:		name = "Num /";			break;
	case VK_DECIMAL:	name = "Num Del";		break;
	case VK_OEM_PLUS:	name = "=";				break;
	case VK_OEM_MINUS:	name = "-";				break;
	case VK_OEM_1:		name = ";";				break;
	case VK_OEM_COMMA:	name = ",";				break;
	case VK_OEM_PERIOD:	name = ".";				break;
	case VK_OEM_2:		name = "/";				break;
	case VK_OEM_3:		name = "`";				break;
	case VK_OEM_4:		name = "[";				break;
	case VK_OEM_5:		name = "\\";			break;
	case VK_OEM_6:		name = "]";				break;
	case VK_OEM_7:		name = "'";				break;
	case VK_SPACE:		name = "Space";			break;
	case VK_RETURN:		name = "Enter";			break;
	case VK_ESCAPE:		name = "Esc";			break;
	case VK_TAB:		name = "Tab";			break;
	case VK_BACK:		name = "Backspace";		break;
	case VK_DELETE:		name = "Delete";		break;
	case VK_INSERT:		name = "Insert";		break;
	case VK_HOME:		name = "Home";			break;
	case VK_END:		name = "End";			break;
	case VK_PRIOR:		name = "Page Up";		break;
	case VK_NEXT:		name = "Page Down";		break;
	case VK_LEFT:		name = "Left";			break;
	case VK_RIGHT:		name = "Right";			break;
	case VK_UP:			name = "Up";			break;
	case VK_DOWN:		name = "Down";			break;
	case VK_CANCEL:		name = "Break";			break;
	case VK_PAUSE:		name = "Pause";			break;
	case VK_SCROLL:		name = "Scroll Lock";	break;
	case VK_CAPITAL:	name = "Caps Lock";		break;
	case VK_SNAPSHOT:	name = "Print Screen";	break;
	case VK_APPS:		name = "Menu";			break;
	default:			break;
	}
	if (name)
		return text + name;
	sprintf_s(buf, "Key %u", key);
	return text + buf;
}
