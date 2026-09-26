#pragma once
// The chooser (###dlg.choose, an L dialog): a searchable tree of the items on the left - a search opens the folders
// that hold a match - and the picked item's picture, name and details on the right.
class XREPROPS_API UIChooseForm :  public XrUI
{
    friend class UIChooseFormItem;
    void UpdateSelected(UIChooseFormItem*NewSelected);
    UIChooseFormItem m_RootItem;
    ImTextureID m_Texture;
    char m_Search[256];
    xr_string m_AppliedSearch;
    bool m_FocusSearch;
    UIPropertiesForm* m_Props;
    Flags32  m_Flags;
    xr_string m_Title;
    xr_vector<SChooseItem*>m_SelectedItems;
    s32 m_SelectedList;
    bool m_ScrollToSelected;
    SChooseItem* GetSelectedItem();
    UIChooseFormItem* m_SelectedItem;
    SChooseItem m_ItemNone;
    ChooseItemVec	m_Items;
    u32 m_ChooseID;
    // drawn once a frame whoever asks, and whether the dialog was ever up (it has to come down then)
    int m_DrawnFrame;
    bool m_Shown;
    void  FillItems(u32 choose_id);
    void CheckFavorite();
    void DrawInfo();
    void DrawSelectedList();
    // OK: the picked item (or the list of them) is the result
    void Confirm();
protected:

    DEFINE_MAP(u32, SChooseEvents, EventsMap, EventsMapIt);
    static EventsMap			m_Events;
private:
    SChooseEvents 	E;
    static UIChooseForm* Form;
    static ImTextureID NullTexture;
    // the dialog of a form that is gone still has to be taken down
    static bool s_Closing;

public:
    enum Result
    {
        R_Ok,
        R_Cancel
    };
private:
        Result m_Result;
public:
    UIChooseForm();
    virtual ~UIChooseForm();
    virtual void Draw();
    static void SetNullTexture(ImTextureID Texture);
    static void Update();
    static bool IsActive();
    // Selects one item by its tree path and confirms, exactly like picking it and
    // pressing OK - the programmatic half of the modal.
    static bool PickAndConfirm(LPCSTR name);
    // same selection, no OK - the pane (preview, info) reacts as to a click
    static bool Pick(LPCSTR name);
    // name of the item highlighted in the open form, "" when none/no form
    static LPCSTR SelectedName();
    // closes the open form without applying, like the Cancel button
    static bool CancelForm();
    static bool GetResult(bool&change,shared_str&result);
    static bool GetResult(bool& change, xr_string& result);
    static bool GetResult(bool& change, xr_vector<xr_string>& result);
    static void SelectItem(u32 choose_ID,  int sel_cnt = 1, LPCSTR init_name = 0, TOnChooseFillItems item_fill = 0, void* fill_param = 0, TOnChooseSelectItem item_select = 0, ChooseItemVec* items = 0, u32 flags = cfAllowNone);
    static void AppendEvents(u32 choose_ID, LPCSTR caption, TOnChooseFillItems on_fill, TOnChooseSelectItem on_sel, TGetTexture on_thm, TOnChooseClose on_close, u32 flags);
    static void ClearEvents();
    static SChooseEvents* GetEvents(u32 choose_ID);
};

