#pragma once
// A few properties asked for in a dialog of their own (###dlg.properties): the random append settings of the Object
// tool, the key scaling of a motion. OK keeps what was set.
class UIPropertiesModal :
	public XrUI
{
public:
	UIPropertiesModal();
	virtual ~UIPropertiesModal();
	virtual void Draw();
	static void Update();
	static bool GetResult(bool& ok);
	// `title` names what is asked; the rows are the caller's
	static void Show(PropItemVec&items, LPCSTR title = nullptr);
	static IC UIPropertiesForm* GetProperties() { return Form->m_Props; }
private:
	static UIPropertiesModal* Form;
	// the dialog of a form that is gone still has to be taken down
	static bool s_Closing;
private:
	UIPropertiesForm* m_Props;
	xr_string m_Title;
	bool m_Shown;

private:
	enum Result
	{
		R_Ok,
		R_Cancel
	};
	Result m_Result;
};
