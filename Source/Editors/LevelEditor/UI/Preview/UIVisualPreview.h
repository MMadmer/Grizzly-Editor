#pragma once

// Unreal-style asset preview: the editor's "model" tab. It renders one visual
// into its own render target - an orbit viewport with a headlight, the model
// standing on a floor grid - and shows beside it what the file is made of,
// textures included. The tab owns the model: closing it (or
// switching to another asset) releases it, so nothing stays loaded behind the
// user's back.
//
// The model is drawn straight into a render-target texture from inside the
// editor's already open scene: no BeginScene/EndScene, no readback.

#if defined(USE_DX11)
#include "../../../XrECore/Editor/EDX11Utils.h"
#endif

class IRenderVisual;
class CEditableObject;

class UIVisualPreview : public XrUI
{
public:
					UIVisualPreview		();
	virtual			~UIVisualPreview	();

	// opens/focuses the window and shows the model.
	//   visual_name - name for ::Render->model_Create (relative to
	//                 $game_meshes$, extension optional); null just opens the
	//                 window empty
	static void		Show				(LPCSTR visual_name = nullptr);
	// same, but the block comes from memory - assets inside the game archives
	// never hit the disk
	static void		ShowFromMemory		(LPCSTR title, const void* data, u32 size);
	// library object (.object under $objects$). That is the editor's own asset
	// format, not an engine visual, so the model pool cannot touch it - it gets
	// its own private CEditableObject and draws through RenderSingle.
	static void		ShowObject			(LPCSTR object_name);
	// same from memory: a project's own .object is outside $objects$ and outside
	// the editor FS entirely, so the caller reads it off the disk itself
	static void		ShowObjectFromMemory(LPCSTR title, const void* data, u32 size);
	static void		Close				();
	static IC bool	IsOpen				() { return !!Form; }
	static void		Update				();
	// MCP preview_model: what the tab shows, as JSON fields (no braces)
	static void		McpState			(xr_string& out);
	// The view asked for over MCP; below 0 keeps: lighting 0 studio, 1 unlit; wireframe, bones and details
	// 0 or 1; frame / reset put the camera back on the model.
	static bool		McpView				(int lighting, int wireframe, int bones, int details, bool frame, bool reset, xr_string& err);
	// the tab was drawn in the last frame: what McpState says it laid out is what shows
	static bool		DrawnLately			();
	virtual void	Draw				();


	// D3D9 refuses to Reset() while a D3DPOOL_DEFAULT surface is alive; these
	// are wired into CEditorRenderDevice::Reset through g_pOnEditorDeviceReset*
	static void		OnDeviceResetBegin	();
	static void		OnDeviceResetEnd	();

private:
	static UIVisualPreview*	Form;
	// the model preview is the editor's "model" tab (XUI::Shell): one of them, showing the last model given
	static void		ShowTab				();

	// model: exactly one of these is live at a time
	IRenderVisual*	m_Visual;
	CEditableObject* m_Editable;
	xr_string		m_Name;
	u32				m_Type;			// MT_*
	u32				m_Verts;
	u32				m_Tris;
	u16				m_Bones;
	Fbox			m_Box;
	Fvector			m_Center;
	float			m_Radius;
	xr_string		m_Error;

	// the textures an engine visual draws with and where the texture loader finds each - read once per model
	enum ETextureFrom { tfMissing, tfProject, tfEditor, tfGame };
	struct STexture
	{
		xr_string		name;		// engine name: below textures\, no extension
		ETextureFrom	from;
	};
	xr_vector<STexture>	m_Textures;

	// orbit camera, all relative to the model's AABB
	float			m_Yaw;
	float			m_Pitch;
	float			m_Distance;
	Fvector			m_Pan;

	bool			m_Wireframe;
	bool			m_ShowBones;
	int				m_Lighting;		// 0 studio (a headlight and ambient), 1 unlit (the textures as they are)
	bool			m_ShowDetails;
	float			m_DetailsW;		// pt, as the last frame laid it out
	int				m_DrawnFrame;	// the ImGui frame that drew the tab last

	// Render target, recreated only when the viewport outgrows it: it comes in 64 px steps and the view is
	// drawn into its top-left corner. Raw device objects: the resource manager's CRT lives inside XrECore and
	// is not exported.
#if defined(USE_DX11)
	// texture + RTV + depth + SRV in one; the SRV is what ImGui::Image gets
	SDX11Target		m_Target;
#else
	IDirect3DTexture9*	m_RTTex;		// what ImGui::Image gets
	IDirect3DSurface9*	m_RTSurf;		// mip 0 of the above, bound as RT
	IDirect3DSurface9*	m_ZBSurf;
#endif
	u32				m_RTW;
	u32				m_RTH;
	bool			m_RTFailed;		// creation refused - stop retrying every frame

	void			SetVisual			(IRenderVisual* v, LPCSTR name);
	void			ReadTextures		();
	void			SetEditable			(CEditableObject* o, LPCSTR name);
	void			ReleaseVisual		();
	void			ReleaseTarget		();
	bool			EnsureTarget		(u32 w, u32 h);
	void			ResetCamera			();
	void			FrameModel			();
	void			RenderToTarget		(u32 w, u32 h);
	void			DrawToolBar			(XUI::Toolbar& tb);
	void			DrawViewport		(const ImVec2& p0, const ImVec2& avail);
	void			DrawDetails			();
	void			DrawStatus			();
};
