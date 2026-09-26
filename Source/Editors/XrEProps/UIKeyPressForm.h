#pragma once
// Assign Shortcut (###dlg.assign_shortcut): the next key pressed, with its modifiers, becomes the candidate; Assign
// takes it. Every key can be taken - Esc and Enter too, which is why only Cancel leaves the dialog.
class XREPROPS_API UIKeyPressForm :public XrUI
{
public:
	UIKeyPressForm();
	virtual ~UIKeyPressForm();
	virtual void Draw();
public:
	// the command a shortcut answers to now, or "" when none does (it is shown as a warning, never refused)
	using TConflict = std::function<xr_string(const xr_shortcut&)>;
	static void Update(float timeGlobal);
	// `what`: the command the shortcut is for
	static void Show(LPCSTR what = nullptr, const TConflict& conflict = {});
	static bool IsActive() { return !!Form; }
	// a key the editor's own key handler saw while the dialog is up: the candidate
	static bool SetResult(const xr_shortcut& result);
	static bool GetResult(bool &Ok, xr_shortcut& result);
	// "Ctrl+Shift+S", "Num Del", "F5": the names the menus print (Public/ShortcutText.h); false for anything else
	static bool Parse(LPCSTR text, xr_shortcut& out);
private:
	void Capture(const xr_shortcut& shortcut);
	xr_shortcut m_Resutl;
	bool m_Captured;
	bool m_Shown;
	xr_string m_What;
	xr_string m_ConflictText;
	TConflict m_Conflict;
	float m_TimeGlobal;
	static UIKeyPressForm* Form;
	// the dialog of a form that is gone still has to be taken down
	static bool s_Closing;
	bool m_Ok;
};
