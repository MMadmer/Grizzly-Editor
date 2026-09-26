//---------------------------------------------------------------------------

#ifndef ImageLibH
#define ImageLibH
#include "UIEditLightAnim.h"

class ETextureThumbnail;

// The Texture Library tab: the texture descriptions (.thm) of the editor's library - or, as New Textures, the
// ones Check New Textures found in the import folder - as list | preview | properties. Save writes the
// changed descriptions and rebuilds their game textures (Import brings the new ones in). The two share one
// tab: Check New Textures turns a library without unsaved changes into New Textures, and the import brings
// the library back.
class UIImageEditorForm :public XrUI
{
public:
	UIImageEditorForm();
	virtual ~UIImageEditorForm();
	virtual void Draw();
public:
	static void Update();
	static void Show(bool bImport);
	static void ImportTextures();
	static bool IsOpen() { return !!Form; }
	// MCP library kind=texture_library: state, select item=, search=
	static void Mcp(const xr_string& action, LPCSTR raw, xr_string& out);
private:
	DEFINE_VECTOR(ETextureThumbnail*, THMVec, THMIt);
	DEFINE_MAP(shared_str, ETextureThumbnail*, THMMap, THMMapIt);
	THMMap m_THM_Used;
	THMVec m_THM_Current;
	ListItemsVec m_List;
	XUI_LocalLibraryList m_ListView;
	xr_string m_Selected;
	UIPropertiesForm* m_ItemProps;
	FS_FileSet	texture_map;
	FS_FileSet	modif_map;
	bool bImportMode;
	bool bReadonlyMode;
	bool m_UpdateQueued;
	bool m_CloseAfterUpdate;
	// the tab closed while a save was queued: the form goes once the save is through
	bool m_DeleteAfterUpdate;
	// the unsaved prompt was answered Don't Save
	bool m_Discarded;
	// the description on show was edited since it was picked
	bool m_ItemModified;
	// New Textures took the library's tab: the library comes back once the import is through
	bool m_ReturnToLibrary;
	static UIImageEditorForm* Form;
	ImTextureID m_Texture;
	// previews replaced this frame: released once the frame that may have drawn them is through
	xr_vector<ImTextureID> m_TextureDead;
	int m_TextureDeadFrame;
private:
	ETextureThumbnail* FindUsedTHM(const shared_str& name);
	void RegisterModifiedTHM();
	void OnCubeMapBtnClick(ButtonValue* value, bool& bModif, bool& bSafe);
	void OnTypeChange(PropValue* prop);
	void OnPropModified();
	void InitItemList();
	// drops every description the tab holds, the preview and the unsaved marks with them
	void ClearItems();
	// the tab on what Form holds: the library read from disk, or New Textures on its texture_map
	static void Present(bool bImport);
	void HideLib();
	bool UpdateLib();
	static void UpdateLibDeferred();
	void OnItemsFocused(ListItem* item);
	void SaveUsedTHM();
	bool Dirty();
	void Revert();
	bool QueueSave(xr_string& err);
	void Select(int index);
	void DrawToolbar(XUI::Toolbar& tb);
	void DrawPreview();
	void DrawStatus();
private:
	bool m_bFilterImage;
	bool m_bFilterTerrain;
	bool m_bFilterBump;
	bool m_bFilterNormal;
	bool m_bFilterCube;
	void FilterUpdate();
};
//---------------------------------------------------------------------------
#endif
