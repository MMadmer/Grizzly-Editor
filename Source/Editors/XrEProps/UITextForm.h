#pragma once
// Edit Text (###dlg.text_editor): a text in a dialog of its own, OK takes it
class XREPROPS_API UITextForm :public XrUI
{
public:
	UITextForm(const char*str);
	virtual ~UITextForm();
	virtual void Draw();
	static void RunEditor(const char* str);
	static void Update();
	static bool GetResult(bool& change, xr_string& result);
private:
	void CLBOk();
	void CLBCancel();
private:
	bool m_Ok;
	bool m_Shown;
	xr_string m_Text;
	xr_vector<char> m_EditText;

	static UITextForm* Form;
	// the dialog of a form that is gone still has to be taken down
	static bool s_Closing;
};

