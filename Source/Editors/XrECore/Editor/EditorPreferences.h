//---------------------------------------------------------------------------

#ifndef EditorPreferencesH
#define EditorPreferencesH
//---------------------------------------------------------------------------
// refs
class UIPropertiesForm;
class PropValue;
struct SESubCommand;

inline constexpr u32 kEditorIdleFpsDefault = 120;
inline constexpr u32 kEditorIdleFpsMinimum = 30;
inline constexpr u32 kEditorIdleFpsMaximum = 240;
// level_imgui.ini of another version is skipped once: GrizzlyLevel_v2 and the renamed panels
inline constexpr u32 kUiLayoutVersion = 2;

//---------------------------------------------------------------------------
enum{
    epoDrawPivot		= (1<<0),
    epoDrawAnimPath		= (1<<1),
    epoDrawJoints		= (1<<2),
    epoDrawBoneAxis		= (1<<3),
    epoDrawBoneNames	= (1<<4),
    epoDrawBoneShapes	= (1<<5),
    epoShowHint			= (1<<6),
    epoDrawLOD			= (1<<7),
    epoDiscardInstance	= (1<<8),
    epoEagerLoadRenderBuffers = (1<<9),
    epoEagerLoadCollisionModels = (1<<10),
    epoSelectInGroup    = (1<<11),
};
class ECORE_API CCustomPreferences:protected XrUI
{
private:	// User declarations
    UIPropertiesForm*	m_ItemProps;
    // the Preferences tab: its search, the page it shows, a save waiting for the field being typed in to let go,
    // and the command the Assign Shortcut dialog is asking for
    char				m_Search[128];
    xr_string			m_Page;
    bool				m_SavePending;
    SESubCommand*		m_AssignFor;
    void				OnModified		();
    void				DrawNav			(float width);
    void				DrawPage		();
    void				DrawKeyboard	(bool searching);
    u32					KeyboardMatches	() const;
public:
    // the commands that take a shortcut: their id is the .ini name (COMMAND_X, or COMMAND_X."sub"); their official
    // name and the group it is listed under (the Keyboard page, the Command Palette), and the place the Keyboard page
    // lists it at (an unlisted command after the listed ones); the label is the group and the name joined by a middle
    // dot ("Edit", "Undo") - what MCP shortcuts calls a command besides its id
    static xr_string	ShortcutId		(SESubCommand* sub);
    static u32			CommandName		(SESubCommand* sub, xr_string& group, xr_string& label);
    static xr_string	ShortcutLabel	(SESubCommand* sub);
    // the command that has this shortcut now, other than `except`; null for none
    static SESubCommand* ShortcutOwner	(const xr_shortcut& shortcut, SESubCommand* except = nullptr);
    // gives `sub` the shortcut, taking it from the command that had it; saved at once
    void				AssignShortcut	(SESubCommand* sub, const xr_shortcut& shortcut);
    // the tab's page (a top-level folder of the rows, or Keyboard) and its search; the page it shows now
    void				ShowPage		(LPCSTR page, LPCSTR search = nullptr);
    LPCSTR				Page			() const { return m_Page.c_str(); }
protected:
    // what a row of a derived FillProp does: its tooltip and the help under the grid; and the unit its number is
    // shown in ("min", "s"), which its range in the help then says too
    void				AddHelp			(LPCSTR key, LPCSTR help);
    void				AddUnit			(LPCSTR key, LPCSTR unit);
    // the icon the nav list shows for a page a derived FillProp adds (its top-level folder); a page nobody names one
    // for gets the settings gear, never a bare row
    void				AddPageIcon		(LPCSTR page, XUI::Icon icon);
    XUI::Icon			PageIcon		(const xr_string& page) const;
    xr_vector<std::pair<xr_string, xr_string> > m_ExtraHelp;
    xr_vector<std::pair<xr_string, xr_string> > m_ExtraUnits;
    xr_vector<std::pair<xr_string, XUI::Icon> > m_PageIcons;
public:
    u32             start_w;
    u32             start_h;
    u32             start_maximized;
	u32				ui_theme_preset;
	u32				active_idle_fps;
	// [ui]: the layout's version (another one is not loaded), whether the author rearranged the Level
	// workspace, and panels in windows of their own (a restart takes effect)
	u32				ui_layout_version;
	bool			level_layout_custom;
	BOOL			ui_floating_panels;
	// view
    float 			view_np;
    float 			view_fp;
    float 			view_fov;
	// fog    
    u32 			fog_color;
    float			fog_fogness;
    // camera
    float			cam_fly_speed;
    float			cam_fly_alt;
    float			cam_sens_rot;
    float			cam_sens_move;
	// tools mouse
    float			tools_sens_rot;
    float			tools_sens_move;
    float			tools_sens_scale;
    BOOL			tools_show_move_axis;
    // box pick
    BOOL			bp_lim_depth;
    BOOL			bp_cull;
    float			bp_depth_tolerance;
    // snap: the steps of the viewport toolbar's Angle (radians), Grid and Scale snaps - the gizmo reads and sets these
    float			snap_angle;
    float			snap_move;
    float			snap_scale;
    float			snap_moveto;
    // grid
    float			grid_cell_size;
    u32 			grid_cell_count;
    // scene
    u32				scene_undo_level;
    u32				scene_recent_count;
    u32				scene_clear_color;
    AStringVec 		scene_recent_list;
    // objects
    Flags32			object_flags;
    shared_str      sWeather;
protected:
	void 			ImportShortcuts			();
	void 			ExportShortcuts			();
	void			OnThemeChanged			(PropValue* value);
	void 	        OnClose();
    void			ApplyValues();

    virtual void 	Load				(CInifile*);
    virtual void 	Save				(CInifile*);
    virtual void    Draw();
public:				// User declarations
    				CCustomPreferences	();
    virtual 		~CCustomPreferences	();

    void			OnCreate			();
    void			OnDestroy			();

    virtual void	FillProp          	(PropItemVec& items);

    void			Edit				();

    void 			Load				();
    void 			Save				();
	void			SetThemePreset			(u32 preset, bool persist = true);
	// what the Preferences tab does with a changed value: applied at once, and saved (MCP preferences set)
	void			ApplyAndSave			() { ApplyValues(); Save(); }
    
    void 			AppendRecentFile	(LPCSTR name);
    LPCSTR 			FirstRecentFile		(){return scene_recent_list.empty()?"":scene_recent_list.front().c_str();}
};
//---------------------------------------------------------------------------
#define R_FLOAT_SAFE(S,L,D)	I->line_exist(S,L)?I->r_float(S,L):D;
#define R_U32_SAFE(S,L,D) 	I->line_exist(S,L)?I->r_u32(S,L):D;
#define R_BOOL_SAFE(S,L,D) 	I->line_exist(S,L)?I->r_bool(S,L):D;
#define R_STRING_SAFE(S,L,D)I->line_exist(S,L)?I->r_string_wb(S,L):D;
//---------------------------------------------------------------------------
extern ECORE_API CCustomPreferences* 	EPrefs;
//---------------------------------------------------------------------------

#endif
