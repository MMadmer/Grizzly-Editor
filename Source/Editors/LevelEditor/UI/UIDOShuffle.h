#pragma once
class EDetailManager;
class EImageThumbnail;
class UIDOOneColor;
// The Detail Objects dialog (###dlg.detail_objects, L): the objects the level's details are made of with their
// settings, and the colour indices that say which objects grow on which colour of the detail mask.
class UIDOShuffle :
	public XrUI
{
	friend UIDOOneColor;
public:
	UIDOShuffle();
	virtual ~UIDOShuffle();
	virtual void Draw();
	static void Update();
	static bool GetResult();
	static void Show(EDetailManager* DM);
private:
	static UIDOShuffle* Form;
private:
	EDetailManager* DM;
	void FillData();
	void DrawObjects();
	void DrawObjectList();
	void DrawObjectSettings();
	void DrawColorIndices();
	void DrawColorIndexList();
private:
	ImTextureID m_Texture;
	ImTextureID m_RealTexture;
private:
	void OnItemFocused(const char* name);
	xr_vector<xr_string> m_list;
	int m_list_selected;
	bool FindItem(const char* name);
private:
	bool bModif;
	bool m_ChooseObject;
private:
	void ClearIndexForms();
	xr_vector< UIDOOneColor*> m_color_indices;
	bool ApplyChanges(bool msg=true);
};
