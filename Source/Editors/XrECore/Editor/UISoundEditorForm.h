//---------------------------------------------------------------------------
#pragma once
#include "UIEditLightAnim.h"

// The Sound Library tab: the sound descriptions (.thm) of the editor's library as list | preview | properties.
// The preview plays the game sound and draws its attenuation; Save writes the changed descriptions and
// rebuilds their game sounds.
class UISoundEditorForm :public XrUI
{
public:
	UISoundEditorForm();
	virtual ~UISoundEditorForm();
	virtual void Draw();
public:
	static void Update();
	static void Show();
	static bool IsOpen() { return !!Form; }
	// MCP library kind=sound_library: state, select item=, search=
	static void Mcp(const xr_string& action, LPCSTR raw, xr_string& out);
private:

	void HideLib();
	static UISoundEditorForm* Form;
	bool m_UpdateQueued;
	// the tab closed while a save was queued: the form goes once the save is through
	bool m_DeleteAfterUpdate;
	// the description on show was edited since it was picked
	bool m_ItemModified;
	static void UpdateLibDeferred();

	void InitItemList();
	void OnItemsFocused(ListItem* item);
	void Select(int index);
	UIPropertiesForm* m_ItemProps;
	ListItemsVec m_List;
	XUI_LocalLibraryList m_ListView;
	xr_string m_Selected;
private:
	enum {
		flUpdateProperties = (1 << 0),
		flReadOnly = (1 << 1),
	};
	Flags32		m_Flags;
	DEFINE_VECTOR(ESoundThumbnail*, THMVec, THMIt);
	THMVec				m_THM_Used;
	THMVec				m_THM_Current;
	ESoundThumbnail* FindUsedTHM(LPCSTR name);
	void				SaveUsedTHM();
	void				DestroyUsedTHM();

	void RegisterModifiedTHM();

	void OnModified();
	bool UpdateLib();
	bool Dirty();
	void Revert();
	bool QueueSave(xr_string& err);

	BOOL				bAutoPlay;
	// what the file on show is: its size and length, for the preview
	u32					m_FileSize;
	u32					m_FileTime;
	bool				m_HasGameSound;

	FS_FileSet	modif_map;
	ref_sound			m_Snd;

	void				AppendModif(LPCSTR nm);
	void				DrawToolbar(XUI::Toolbar& tb);
	void				DrawPreview();
	void				DrawAttenuation(ESoundThumbnail* thm, float height);
	void				DrawStatus();
	void				Play();
	void				Stop();
	void				RebuildGameSound();
private:
	void   	OnAttClick(ButtonValue* sender, bool& bModif, bool& bSafe);
	void    PlaySound(LPCSTR name, u32& size, u32& time);
};
