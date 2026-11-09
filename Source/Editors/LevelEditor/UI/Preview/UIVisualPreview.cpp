#include "stdafx.h"

#include "UIVisualPreview.h"
#include "../../../../XrRender/Private/FVisual.h"
#include "../../../../XrRender/Private/FHierrarhyVisual.h"
#include "../../../../XrRender/Public/KinematicsAnimated.h"
// library objects (.object) draw through CEditableObject::RenderSingle
#include "../../../XrECore/Editor/EditObject.h"
#include "../../../XrECore/Editor/EditorGameContent.h"
#include "../../../XrECore/Editor/Nq/NqUtil.h"

extern BOOL ENGINE_API g_bRendering;

UIVisualPreview* UIVisualPreview::Form = nullptr;

namespace
{
	// Three-quarter shot from above, same framing the asset thumbnails use:
	// a flat front view flattens most props into unreadable silhouettes.
	static const Fvector	s_StartOffset		= {  1.0f,  0.8f, -1.0f };
	static const float		s_FOVDegrees		= 45.f;
	static const float		s_OrbitSpeed		= 0.008f;	// rad per pixel
	static const float		s_PitchLimit		= 1.5533f;	// ~89 deg
	static const float		s_JointSize			= 0.02f;
	static const u32		s_JointColor		= 0xffffff00;
	static const u32		s_LinkColor			= 0xffa0a000;

	// pt (docs/UI_DESIGN.md, sections 2 and 6): the Details pane and what the viewport keeps
	constexpr float			kDetailsW			= 320.f;
	constexpr float			kSideMin			= 220.f;
	constexpr float			kCanvasMin			= 400.f;
	// the render target grows in steps: a splitter dragged across the tab does not rebuild it every pixel
	constexpr u32			kTargetStep			= 64;

	// a theme colour as the device wants it (0xAARRGGBB): the clear, the floor's lines
	u32 DeviceColor(GrizzlyTheme::ColorToken token)
	{
		return 0xFF000000 | GrizzlyTheme::Rgb(token);
	}

	u32 BackgroundColor()
	{
		return DeviceColor(GrizzlyTheme::ColorToken::Canvas);
	}

	// the floor: at most this many lines each way, and the margin round the model's box in radii
	constexpr int			kFloorLinesMax		= 48;
	constexpr float			kFloorMargin		= .5f;

#if defined(USE_DX11)
	// The target's alpha is whatever the model's shader wrote - a character texture keeps its gloss mask there - and
	// ImGui blends every image by its alpha, which would let the canvas show through the model. The view is a picture
	// of the target, so it is drawn opaque; the backend's reset puts its own blending back after it.
	ID3D11BlendState*	s_Opaque = nullptr;

	void SetOpaqueBlend(const ImDrawList*, const ImDrawCmd*)
	{
		if (!s_Opaque && HW.pDevice)
		{
			D3D11_BLEND_DESC desc = {};
			desc.RenderTarget[0].BlendEnable			= FALSE;
			desc.RenderTarget[0].RenderTargetWriteMask	= D3D11_COLOR_WRITE_ENABLE_ALL;
			HW.pDevice->CreateBlendState(&desc, &s_Opaque);
		}
		if (s_Opaque && HW.pContext)
			HW.pContext->OMSetBlendState(s_Opaque, nullptr, 0xffffffff);
	}
#endif

	//--------------------------------------------------------------------------
	// We are a guest inside the editor's frame - the imgui pass runs between
	// EDevice->Begin() and End() - so everything we poke gets captured on entry
	// and pushed back on exit, exception or not.
	//--------------------------------------------------------------------------
	struct SPreviewStateGuard
	{
#if defined(USE_DX11)
		// D3D11 has no state blocks: SDX11TargetGuard captures the render target,
		// depth view and viewport, and the shader state is re-applied per draw
		// anyway once RCache is invalidated below.
		SDX11TargetGuard		m_Views;
		// lights/material live in the recorded FF block now - restore it so the
		// preview's light rig does not leak into the scene pass
		SEditorFixedFunc		m_FF;
#else
		IDirect3DStateBlock9*	m_Block;
		IDirect3DSurface9*		m_OldRT;
		IDirect3DSurface9*		m_OldZB;
		D3DVIEWPORT9			m_OldViewport;
#endif
		Fmatrix					m_OldWorld;
		Fmatrix					m_OldView;
		Fmatrix					m_OldProject;
		Fmatrix					m_OldDeviceView;
		Fmatrix					m_OldDeviceProject;
		Fmatrix					m_OldDeviceFull;
		Fvector					m_OldCameraPos;
		Fvector					m_OldCameraDir;
		CFrustum				m_OldFrustum;

		SPreviewStateGuard()
		{
#if defined(USE_DX11)
			m_FF				= EDevice->ff;
#else
			m_Block				= 0;
			m_OldRT				= 0;
			m_OldZB				= 0;
			// D3DSBT_ALL covers render states, samplers, lights and material -
			// the shader passes we are about to run overwrite all of those
			HW.pDevice->CreateStateBlock	(D3DSBT_ALL,&m_Block);
			HW.pDevice->GetRenderTarget		(0,&m_OldRT);
			HW.pDevice->GetDepthStencilSurface(&m_OldZB);
			HW.pDevice->GetViewport			(&m_OldViewport);
#endif
			// transforms travel through RCache, which mirrors them into the
			// device, so save/restore has to go through RCache as well
			m_OldWorld			= RCache.get_xform_world	();
			m_OldView			= RCache.get_xform_view		();
			m_OldProject		= RCache.get_xform_project	();
			m_OldDeviceView		= EDevice->mView;
			m_OldDeviceProject	= EDevice->mProject;
			m_OldDeviceFull		= EDevice->mFullTransform;
			m_OldCameraPos		= EDevice->vCameraPosition;
			m_OldCameraDir		= EDevice->vCameraDirection;
			// CRender shadows IRender_interface::ViewBase, and the culling in
			// CModelPool goes through CRender::occ_visible - so the frustum
			// that matters is the one on RImplementation, not on ::Render
			m_OldFrustum		= RImplementation.ViewBase;
		}

		~SPreviewStateGuard()
		{
#if defined(USE_DX11)
			EDevice->ff			= m_FF;
#else
			if (m_Block)		m_Block->Apply();
			// render targets are not part of a D3D9 state block - restore by
			// hand, and only then the viewport, since SetRenderTarget resets it
			// (slot 0 refuses a NULL surface, so only push a real one back)
			if (m_OldRT)	HW.pDevice->SetRenderTarget(0,m_OldRT);
			HW.pDevice->SetDepthStencilSurface	(m_OldZB);
			HW.pDevice->SetViewport				(&m_OldViewport);
#endif
			RCache.set_xform_world				(m_OldWorld);
			RCache.set_xform_view				(m_OldView);
			RCache.set_xform_project			(m_OldProject);
			EDevice->mView						= m_OldDeviceView;
			EDevice->mProject					= m_OldDeviceProject;
			EDevice->mFullTransform				= m_OldDeviceFull;
			EDevice->vCameraPosition			= m_OldCameraPos;
			EDevice->vCameraDirection			= m_OldCameraDir;
			RImplementation.ViewBase			= m_OldFrustum;
			// the state block put the device somewhere RCache does not know
			// about - OnFrameEnd is the public way to drop the cache, so the
			// next draw re-applies everything
			RCache.OnFrameEnd					();
#if !defined(USE_DX11)
			_RELEASE							(m_Block);
			_RELEASE							(m_OldRT);
			_RELEASE							(m_OldZB);
#endif
			// on D3D11 m_Views puts the render target, depth view and viewport
			// back when it goes out of scope, right after this body
		}
	};

	//--------------------------------------------------------------------------
	// The editor draws visuals through the fixed-function lighting it emulates
	// (EDevice_PushFFConstants): Studio is a headlight - a light from the camera,
	// so whatever faces the viewer is lit whichever way the model is turned - a
	// soft key from above so the shape reads, over an ambient. Unlit enables no
	// light at all, which the editor shaders take as "the texture as it is".
	//--------------------------------------------------------------------------
	void SetupPreviewLights(bool studio, const Fvector& view_dir)
	{
		for (u32 i=0; i<8; i++)
			EDevice->LightEnable(i,FALSE);
		if (!studio)
			return;

		Flight L;
		ZeroMemory		(&L,sizeof(L));
		L.type			= D3DLIGHT_DIRECTIONAL;

		L.diffuse.set	(.85f,.85f,.85f,1.f);
		L.direction		= view_dir;
		EDevice->SetLight(0,L);			EDevice->LightEnable(0,TRUE);

		L.diffuse.set	(.35f,.35f,.38f,1.f);
		L.direction.set	(.3f,-1.f,.2f);	L.direction.normalize();
		EDevice->SetLight(1,L);			EDevice->LightEnable(1,TRUE);
	}

	//--------------------------------------------------------------------------
	void SetupPreviewStates(bool wireframe, bool studio)
	{
		// states the shader passes do not own themselves: the editor may sit in
		// wireframe/flat/fog mode and that must not leak into the preview
		// through the device wrapper: on D3D9 it writes the state right away, on
		// D3D11 it records it for the fixed-function emulation
		EDevice->SetRS(D3DRS_COLORWRITEENABLE,	D3DCOLORWRITEENABLE_ALPHA|D3DCOLORWRITEENABLE_BLUE|D3DCOLORWRITEENABLE_GREEN|D3DCOLORWRITEENABLE_RED);
		EDevice->SetRS(D3DRS_FILLMODE,			wireframe?D3DFILL_WIREFRAME:D3DFILL_SOLID);
		EDevice->SetRS(D3DRS_SHADEMODE,			D3DSHADE_GOURAUD);
		EDevice->SetRS(D3DRS_FOGENABLE,			FALSE);
		EDevice->SetRS(D3DRS_AMBIENT,			studio?0x40404040:0x00000000);
		EDevice->SetRS(D3DRS_TEXTUREFACTOR,		0xffffffff);
		for (u32 k=0; k<HW.Caps.raster.dwStages; k++)
		{
			EDevice->SetSS(k,D3DSAMP_MAGFILTER,D3DTEXF_LINEAR);
			EDevice->SetSS(k,D3DSAMP_MINFILTER,D3DTEXF_LINEAR);
			EDevice->SetSS(k,D3DSAMP_MIPFILTER,D3DTEXF_LINEAR);
		}
		EDevice->ResetMaterial();
	}

	//--------------------------------------------------------------------------
	// CModelPool::Instance_Create FATALs on visual types the editor does not
	// build (MT_LOD, MT_TREE_*), so anything unknown has to be rejected before
	// the pool ever sees it.
	//--------------------------------------------------------------------------
	bool IsSupportedVisualType(u8 type)
	{
		switch (type)
		{
		case MT_NORMAL:
		case MT_HIERRARHY:
		case MT_PROGRESSIVE:
		case MT_SKELETON_ANIM:
		case MT_SKELETON_RIGID:
		case MT_SKELETON_GEOMDEF_PM:
		case MT_SKELETON_GEOMDEF_ST:
			return true;
		default:
			return false;
		}
	}

	LPCSTR VisualTypeName(u32 type)
	{
		switch (type)
		{
		case MT_NORMAL:					return "static mesh";
		case MT_HIERRARHY:				return "hierarchy";
		case MT_PROGRESSIVE:			return "progressive mesh";
		case MT_SKELETON_ANIM:			return "animated skeleton";
		case MT_SKELETON_RIGID:			return "rigid skeleton";
		case MT_SKELETON_GEOMDEF_PM:	return "skinned mesh (PM)";
		case MT_SKELETON_GEOMDEF_ST:	return "skinned mesh (ST)";
		case MT_LOD:					return "LOD";
		case MT_TREE_ST:				return "tree (ST)";
		case MT_TREE_PM:				return "tree (PM)";
		case MT_PARTICLE_EFFECT:		return "particle effect";
		case MT_PARTICLE_GROUP:			return "particle group";
		default:						return "unknown";
		}
	}

	// CModelPool::Instance_Load feeds the block straight into r_chunk_safe,
	// which R_ASSERTs on a missing or mis-sized header. Peek it ourselves first.
	bool PeekVisualType(IReader* data, u8& type)
	{
		if (!data)								return false;
		ogf_header H;
		if (sizeof(H)!=data->find_chunk(OGF_HEADER))	{ data->rewind(); return false; }
		data->r		(&H,sizeof(H));
		data->rewind();
		type = H.type;
		return true;
	}

	// Same lookup CModelPool::Instance_Load does, so we can peek the header
	// without letting the pool load a file it will choke on.
	bool ResolveVisualFile(LPCSTR N, string_path& fn)
	{
		string_path name;
		if (0==strext(N))	strconcat	(sizeof(name),name,N,".ogf");
		else				xr_strcpy	(name,sizeof(name),N);

		if (FS.exist(N))						{ xr_strcpy(fn,sizeof(fn),N); return true; }
		if (FS.exist(fn,"$level$",name))		return true;
		if (FS.exist(fn,"$game_meshes$",name))	return true;
		return false;
	}

	bool IsDeviceUsable()
	{
		return (0!=EDevice)&&(FALSE!=EDevice->b_is_Ready)&&(0!=HW.pDevice);
	}

	//--------------------------------------------------------------------------
	// Geometry stats. dxRender_Visual is the only exported node type, so the
	// walk switches on getType() and static_casts - no RTTI across the DLL
	// boundary and no guessing about the concrete class.
	//--------------------------------------------------------------------------
	void CountGeometry(dxRender_Visual* v, u32& verts, u32& tris)
	{
		if (!v)	return;
		switch (v->getType())
		{
		case MT_HIERRARHY:
		case MT_SKELETON_ANIM:
		case MT_SKELETON_RIGID:
			{
				FHierrarhyVisual* h = (FHierrarhyVisual*)v;
				for (u32 i=0; i<h->children.size(); i++)
					CountGeometry(h->children[i],verts,tris);
			}
			break;
		case MT_NORMAL:
		case MT_PROGRESSIVE:
		case MT_SKELETON_GEOMDEF_ST:
		case MT_SKELETON_GEOMDEF_PM:
			{
				Fvisual* f = (Fvisual*)v;
				verts	+= f->vCount;
				tris	+= f->dwPrimitives;
			}
			break;
		default:
			break;
		}
	}

	// Animated skeletons with no active blend collapse into a heap: the bone
	// transforms never leave identity. Any cycle at all fixes the pose, so take
	// the first motion the model ships (idle when it has one).
	void PoseSkeleton(IRenderVisual* visual)
	{
		if (IKinematicsAnimated* KA = visual->dcast_PKinematicsAnimated())
		{
			MotionID motion = KA->LL_MotionID("idle");
			for (u16 slot = 0; !motion.valid() && slot < KA->LL_MotionsSlotCount(); ++slot)
			{
				// ref-counted handle: the accessors are non-const, and the copy
				// keeps the motions alive for the lookup
				shared_motions ms = KA->LL_MotionsSlot(slot);
				if (accel_map* cycles = ms.cycle())
					if (!cycles->empty())
						motion = KA->LL_MotionID(*cycles->begin()->first);
			}
			if (motion.valid())
				KA->LL_PlayCycle(0, motion, TRUE, 0, 0);
		}

		// A skeleton's vis.box straight out of the pool is the union over every
		// animation, so framing by it leaves the model tiny and off-centre.
		// CalculateBones fits the box to the pose we are about to draw.
		if (IKinematics* K = visual->dcast_PKinematics())
			K->CalculateBones(TRUE);
	}

	// the texture names a visual's shaders bind, each once; a hierarchy's own shader is empty, its children draw
	void CollectTextures(dxRender_Visual* v, xr_vector<xr_string>& out)
	{
		if (!v)	return;
		switch (v->getType())
		{
		case MT_HIERRARHY:
		case MT_SKELETON_ANIM:
		case MT_SKELETON_RIGID:
			{
				FHierrarhyVisual* h = (FHierrarhyVisual*)v;
				for (u32 i=0; i<h->children.size(); i++)
					CollectTextures(h->children[i],out);
			}
			return;
		default:
			break;
		}
		if (!v->shader)	return;
		for (const ref_selement& E : v->shader->E)
		{
			if (!E)	continue;
			for (u32 p=0; p<E->passes.size(); ++p)
			{
				const ref_pass& P = E->passes[p];
				if (!P || !P->T)	continue;
				for (const auto& stage : *P->T)
				{
					LPCSTR name = stage.second ? stage.second->cName.c_str() : nullptr;
					// '$' names are the engine's own targets ($user$..., $null), no file behind them
					if (!name || !name[0] || '$'==name[0])	continue;
					if (std::find(out.begin(),out.end(),name)==out.end())
						out.push_back(name);
				}
			}
		}
	}

	// 1, 2 or 5 times a power of ten, near x
	float RoundStep(float x)
	{
		const float p = powf(10.f,floorf(log10f(_max(x,1e-4f))));
		const float m = x/p;
		return p*(m<1.5f ? 1.f : m<3.5f ? 2.f : m<7.5f ? 5.f : 10.f);
	}

	//--------------------------------------------------------------------------
	// The floor the model stands on, the way a mesh editor shows one: lines at a
	// round step in metres, in world units, under the lowest point of the box
	// and a margin wider than it. Every fifth line is stronger, the lines through
	// the origin the strongest. It goes down before the model, which then covers
	// what it stands on.
	//--------------------------------------------------------------------------
	void DrawFloor(const Fbox& box, float radius)
	{
		if (box.min.x>box.max.x||box.min.y>box.max.y||box.min.z>box.max.z)	return;
		const float margin = radius*kFloorMargin;
		float step = RoundStep(radius*.25f);
		int i0, i1, k0, k1;
		for (;;)
		{
			i0 = iFloor((box.min.x-margin)/step);	i1 = iCeil((box.max.x+margin)/step);
			k0 = iFloor((box.min.z-margin)/step);	k1 = iCeil((box.max.z+margin)/step);
			if (i1-i0<kFloorLinesMax && k1-k0<kFloorLinesMax)
				break;
			step = RoundStep(step*2.5f);
		}

		// minor, every fifth, the origin's: one draw per colour
		static xr_vector<Fvector> s_Lines[3];
		for (xr_vector<Fvector>& l : s_Lines)
			l.clear();
		const auto tier = [](int n) { return 0==n ? 2 : 0==n%5 ? 1 : 0; };
		const float y = box.min.y;
		for (int i=i0; i<=i1; ++i)
		{
			xr_vector<Fvector>& l = s_Lines[tier(i)];
			l.push_back(Fvector().set(i*step,y,k0*step));
			l.push_back(Fvector().set(i*step,y,k1*step));
		}
		for (int k=k0; k<=k1; ++k)
		{
			xr_vector<Fvector>& l = s_Lines[tier(k)];
			l.push_back(Fvector().set(i0*step,y,k*step));
			l.push_back(Fvector().set(i1*step,y,k*step));
		}

		static const GrizzlyTheme::ColorToken kTier[3] = { GrizzlyTheme::ColorToken::BorderSubtle, GrizzlyTheme::ColorToken::Border,
			GrizzlyTheme::ColorToken::BorderStrong };
		EDevice->SetShader		(EDevice->m_WireShader);
		RCache.set_xform_world	(Fidentity);
		for (int t=0; t<3; ++t)
			if (!s_Lines[t].empty())
				DU_impl.DrawPrimitiveL(D3DPT_LINELIST,u32(s_Lines[t].size()/2),s_Lines[t].data(),int(s_Lines[t].size()),DeviceColor(kTier[t]),TRUE,FALSE);
	}
}
//------------------------------------------------------------------------------

UIVisualPreview::UIVisualPreview()
{
	m_Visual		= nullptr;
	m_Editable		= nullptr;
	m_Type			= u32(-1);
	m_Verts			= 0;
	m_Tris			= 0;
	m_Bones			= 0;
	m_Box.invalidate();
	m_Center.set	(0.f,0.f,0.f);
	m_Radius		= 1.f;
	m_Yaw			= 0.f;
	m_Pitch			= 0.f;
	m_Distance		= 1.f;
	m_Pan.set		(0.f,0.f,0.f);
	m_Wireframe		= false;
	m_ShowBones		= false;
	m_Lighting		= 0;
	m_ShowDetails	= true;
	m_DetailsW		= 0.f;
	m_DrawnFrame	= -2;
#if !defined(USE_DX11)
	m_RTTex			= nullptr;
	m_RTSurf		= nullptr;
	m_ZBSurf		= nullptr;
#endif
	m_RTW			= 0;
	m_RTH			= 0;
	m_RTFailed		= false;
	ResetCamera		();
}

UIVisualPreview::~UIVisualPreview()
{
	ReleaseVisual	();
	ReleaseTarget	();
#if defined(USE_DX11)
	// a draw list still holding the callback makes a new one: never a dangling state
	_RELEASE		(s_Opaque);
#endif
}

//------------------------------------------------------------------------------
// lifetime
//------------------------------------------------------------------------------
void UIVisualPreview::ShowTab()
{
	if (XUI::Shell::IsOpen("model"))
	{
		XUI::Shell::Activate("model");
		return;
	}
	XUI::Shell::DocDesc d;
	d.id = "model";
	d.kind = "model";
	d.icon = XUI::Icon::FileModel;
	d.padded = false;
	d.label = []
	{
		if (!Form || Form->m_Name.empty()) return xr_string("Model Preview");
		// the model's own name, without its folders and extension
		xr_string name = Form->m_Name;
		const size_t slash = name.find_last_of("\\/");
		if (slash != xr_string::npos) name = name.substr(slash + 1);
		const size_t dot = name.rfind('.');
		if (dot != xr_string::npos && dot) name.resize(dot);
		return name;
	};
	d.tooltip = [] { return Form && !Form->m_Name.empty() ? Form->m_Name : xr_string("Model Preview"); };
	d.alive = [] { return Form && !Form->IsClosed(); };
	d.toolbar = [](XUI::Toolbar& tb) { if (Form) Form->DrawToolBar(tb); };
	d.body = [] { if (Form) Form->Draw(); };
	d.status = [] { if (Form) Form->DrawStatus(); };
	// F frames the model
	d.shortcut = [](int key, int mods)
	{
		if (!Form || key != ImGuiKey_F || mods) return false;
		Form->FrameModel();
		return true;
	};
	d.on_close = [] { Close(); };
	XUI::Shell::Open(d, true);
}

void UIVisualPreview::Show(LPCSTR visual_name)
{
	if (!Form) Form = xr_new<UIVisualPreview>();
	Form->bOpen		= true;
	ShowTab();

	if (!visual_name||!visual_name[0])	return;
	// already showing this one - just bring the window forward
	if ((Form->m_Visual||Form->m_Editable)&&(0==xr_strcmp(Form->m_Name.c_str(),visual_name)))	return;

	Form->ReleaseVisual	();
	Form->m_Name		= visual_name;

	if (!IsDeviceUsable())	{ Form->m_Error = "render device is not ready"; return; }

	string_path fn;
	if (!ResolveVisualFile(visual_name,fn))
	{
		// no engine visual under that name - it may still be a library object,
		// which is what the Objects category of the content browser hands us
		string_path obj_fn;
		FS.update_path	(obj_fn,_objects_,EFS.ChangeFileExt(visual_name,".object").c_str());
		if (FS.exist(obj_fn))	{ ShowObject(visual_name); return; }

		Form->m_Error = "file not found";
		return;
	}

	IReader* test = FS.r_open(fn);
	if (!test)								{ Form->m_Error = "cannot open file"; return; }
	u8	 type		= u8(-1);
	bool type_ok	= PeekVisualType(test,type);
	FS.r_close		(test);
	if (!type_ok)							{ Form->m_Error = "not an OGF model"; return; }
	if (!IsSupportedVisualType(type))
	{
		string64 msg; xr_sprintf(msg,sizeof(msg),"unsupported visual type (%s)",VisualTypeName(type));
		Form->m_Error = msg;
		return;
	}

	// CModelPool::Delete refuses to discard while the engine believes a frame is
	// being rendered; this load is entirely our own, so clear the flag for it
	const BOOL saved_rendering = g_bRendering;
	g_bRendering	= FALSE;
	IRenderVisual* v = ::Render->model_Create(visual_name);
	g_bRendering	= saved_rendering;

	if (!v)	{ Form->m_Error = "model_Create failed"; return; }
	Form->SetVisual	(v,visual_name);
}

void UIVisualPreview::ShowFromMemory(LPCSTR title, const void* data, u32 size)
{
	if (!Form) Form = xr_new<UIVisualPreview>();
	Form->bOpen		= true;
	ShowTab();

	Form->ReleaseVisual	();
	Form->m_Name		= (title&&title[0])?title:"<memory>";

	if (!data||(0==size))	{ Form->m_Error = "empty block"; return; }
	if (!IsDeviceUsable())	{ Form->m_Error = "render device is not ready"; return; }

	IReader reader	((void*)data,int(size));
	u8	 type		= u8(-1);
	if (!PeekVisualType(&reader,type))	{ Form->m_Error = "not an OGF model"; return; }
	if (!IsSupportedVisualType(type))
	{
		string64 msg; xr_sprintf(msg,sizeof(msg),"unsupported visual type (%s)",VisualTypeName(type));
		Form->m_Error = msg;
		return;
	}

	// The pool keys models by name and returns the cached copy on a hit, which
	// would silently ignore our block. Generate a private key instead - and keep
	// it dot-free, CModelPool::Create truncates the key at the last '.'.
	static u32	s_uid = 0;
	string_path	pool_key;
	xr_sprintf	(pool_key,sizeof(pool_key),"$preview$%u",++s_uid);

	const BOOL saved_rendering = g_bRendering;
	g_bRendering	= FALSE;
	IRenderVisual* v = ::Render->model_Create(pool_key,&reader);
	g_bRendering	= saved_rendering;

	if (!v)	{ Form->m_Error = "model_Create failed"; return; }
	Form->SetVisual	(v,Form->m_Name.c_str());
}

void UIVisualPreview::ShowObjectFromMemory(LPCSTR title, const void* data, u32 size)
{
	if (!Form) Form = xr_new<UIVisualPreview>();
	Form->bOpen		= true;
	ShowTab();

	Form->ReleaseVisual	();
	Form->m_Name		= (title&&title[0])?title:"<memory>";

	if (!data||(0==size))	{ Form->m_Error = "empty block"; return; }
	if (!IsDeviceUsable())	{ Form->m_Error = "render device is not ready"; return; }

	// A project's .object is not under $objects$ - and, the project living
	// outside the SDK root, not in the editor FS at all - so ShowObject's
	// by-name resolution can never find it. Load the bytes we were handed.
	//
	// Load(IReader&) takes the object BODY, not the file: the file-based
	// overload opens EOBJ_CHUNK_OBJECT_BODY first, and handing it the whole
	// file asserts on the missing version chunk.
	IReader reader	((void*)data,int(size));
	IReader* body	= reader.open_chunk(EOBJ_CHUNK_OBJECT_BODY);
	if (!body)	{ Form->m_Error = "not an .object file"; return; }

	CEditableObject* obj = xr_new<CEditableObject>(Form->m_Name.c_str());
	const bool loaded = obj->Load(*body);
	body->close();
	if (!loaded)	{ xr_delete(obj); Form->m_Error = "cannot load object"; return; }

	Form->SetEditable	(obj,Form->m_Name.c_str());
}

void UIVisualPreview::ShowObject(LPCSTR object_name)
{
	if (!Form) Form = xr_new<UIVisualPreview>();
	Form->bOpen		= true;
	ShowTab();

	if (!object_name||!object_name[0])	return;
	if ((Form->m_Visual||Form->m_Editable)&&(0==xr_strcmp(Form->m_Name.c_str(),object_name)))	return;

	Form->ReleaseVisual	();
	Form->m_Name		= object_name;

	if (!IsDeviceUsable())	{ Form->m_Error = "render device is not ready"; return; }

	string_path fn;
	FS.update_path	(fn,_objects_,EFS.ChangeFileExt(object_name,".object").c_str());
	if (!FS.exist(fn))	{ Form->m_Error = "file not found"; return; }

	// a private copy, exactly like the thumbnail renderer: the shared library
	// instance is not ours to hold on to across frames
	CEditableObject* obj = xr_new<CEditableObject>(object_name);
	if (!obj->Load(fn))	{ xr_delete(obj); Form->m_Error = "cannot load object"; return; }

	Form->SetEditable	(obj,object_name);
}

void UIVisualPreview::Close()
{
	xr_delete(Form);
}

// the tab draws it; closing the tab closes the preview
void UIVisualPreview::Update()
{
}

//------------------------------------------------------------------------------
// model
//------------------------------------------------------------------------------
void UIVisualPreview::SetVisual(IRenderVisual* v, LPCSTR name)
{
	m_Visual	= v;
	m_Name		= name?name:"";
	m_Error		= "";
	m_Type		= v->getType();
	m_Verts		= 0;
	m_Tris		= 0;
	m_Bones		= 0;

	PoseSkeleton	(v);
	CountGeometry	(static_cast<dxRender_Visual*>(v),m_Verts,m_Tris);
	if (IKinematics* K = v->dcast_PKinematics())	m_Bones = K->LL_BoneCount();

	m_Box		= v->getVisData().box;
	ReadTextures();
	ResetCamera	();
}

// Where the texture loader finds each texture, asked in its order (CRender::texture_load): the editor's folders - the
// project's when the file lies in it - then the linked game; a texture neither has draws as the loader's placeholder.
void UIVisualPreview::ReadTextures()
{
	m_Textures.clear();
	xr_vector<xr_string> names;
	CollectTextures(static_cast<dxRender_Visual*>(m_Visual),names);
	LPCSTR root = EditorProject::Active() ? EditorProject::Root() : "";
	const size_t root_len = xr_strlen(root);
	for (const xr_string& name : names)
	{
		STexture t;
		t.name = name;
		string_path fn;
		if (FS.exist(fn,"$level$",name.c_str(),".dds")||FS.exist(fn,"$game_saves$",name.c_str(),".dds")||
			FS.exist(fn,"$game_textures$",name.c_str(),".dds"))
			t.from = root_len&&0==_strnicmp(fn,root,root_len) ? tfProject : tfEditor;
		else
			t.from = EditorGameContent::Find(NqUtil::Format("textures\\%s.dds",name.c_str()).c_str())>=0 ? tfGame : tfMissing;
		m_Textures.push_back(t);
	}
}

void UIVisualPreview::SetEditable(CEditableObject* o, LPCSTR name)
{
	m_Editable	= o;
	m_Name		= name?name:"";
	m_Error		= "";
	// a library object is not an engine visual, so there is no MT_* for it;
	// DrawDetails reports the kind from m_Editable instead
	m_Type		= u32(-1);
	m_Verts		= u32(o->GetVertexCount());
	m_Tris		= u32(o->GetFaceCount());
	m_Bones		= u16(o->BoneCount());

	m_Box		= o->GetBox();
	ResetCamera	();
}

void UIVisualPreview::ReleaseVisual()
{
	if (m_Visual)
	{
		// the pool refuses to discard mid-frame, and this can run from anywhere
		const BOOL saved_rendering = g_bRendering;
		g_bRendering	= FALSE;
		::Render->model_Delete(m_Visual,TRUE);
		g_bRendering	= saved_rendering;
	}
	xr_delete	(m_Editable);
	m_Visual	= nullptr;
	m_Textures.clear();
	m_Type		= u32(-1);
	m_Verts		= 0;
	m_Tris		= 0;
	m_Bones		= 0;
	m_Box.invalidate();
	m_Error		= "";
}

//------------------------------------------------------------------------------
// render target
//------------------------------------------------------------------------------
void UIVisualPreview::ReleaseTarget()
{
#if defined(USE_DX11)
	m_Target.release();
#else
	_RELEASE	(m_ZBSurf);
	_RELEASE	(m_RTSurf);
	_RELEASE	(m_RTTex);
#endif
	m_RTW		= 0;
	m_RTH		= 0;
	m_RTFailed	= false;
}

bool UIVisualPreview::EnsureTarget(u32 view_w, u32 view_h)
{
	// the view fits into the target it has: only a bigger view, or a much smaller one, makes a new one
	const u32 w = (view_w + kTargetStep - 1) / kTargetStep * kTargetStep;
	const u32 h = (view_h + kTargetStep - 1) / kTargetStep * kTargetStep;
	const bool fits = view_w <= m_RTW && view_h <= m_RTH && m_RTW <= w + kTargetStep * 4 && m_RTH <= h + kTargetStep * 4;
#if defined(USE_DX11)
	if (m_Target.valid()&&fits)								return true;
#else
	if (m_RTTex&&m_RTSurf&&m_ZBSurf&&fits)					return true;
#endif
	// a refused size must not be retried every frame
	if (m_RTFailed&&(m_RTW==w)&&(m_RTH==h))					return false;

	ReleaseTarget	();
	m_RTW	= w;
	m_RTH	= h;

#if defined(USE_DX11)
	// with_srv: ImGui samples the result straight from this texture
	bool ok = m_Target.create(w,h,true);
	if (!ok)	m_Target.release();
#else
	// D3DPOOL_DEFAULT is the only pool a render target may live in - hence the
	// device-reset hook, see OnDeviceResetBegin()
	bool ok = SUCCEEDED(HW.pDevice->CreateTexture(w,h,1,D3DUSAGE_RENDERTARGET,D3DFMT_A8R8G8B8,D3DPOOL_DEFAULT,&m_RTTex,0))&&(0!=m_RTTex);
	if (ok)	ok = SUCCEEDED(m_RTTex->GetSurfaceLevel(0,&m_RTSurf))&&(0!=m_RTSurf);
	if (ok)	ok = SUCCEEDED(HW.pDevice->CreateDepthStencilSurface(w,h,HW.Caps.bStencil?D3DFMT_D24S8:D3DFMT_D24X8,D3DMULTISAMPLE_NONE,0,FALSE,&m_ZBSurf,0))&&(0!=m_ZBSurf);
	if (!ok)
	{
		_RELEASE	(m_ZBSurf);
		_RELEASE	(m_RTSurf);
		_RELEASE	(m_RTTex);
	}
#endif

	if (!ok)
	{
		// keep the size cached so the latch above can compare against it
		m_RTW		= w;
		m_RTH		= h;
		m_RTFailed	= true;
	}
	return ok;
}

void UIVisualPreview::OnDeviceResetBegin()
{
	// the surfaces have to be gone before HW.Reset, the next Draw() rebuilds
	// them at whatever size the window ends up with
	if (Form) Form->ReleaseTarget();
}

void UIVisualPreview::OnDeviceResetEnd()
{
}

//------------------------------------------------------------------------------
// camera
//------------------------------------------------------------------------------
void UIVisualPreview::ResetCamera()
{
	Fvector dir	= s_StartOffset;	dir.normalize();
	m_Pitch		= asinf(dir.y);
	m_Yaw		= atan2f(dir.x,dir.z);
	FrameModel	();
}

// the bounding sphere framed with a small margin, from wherever the camera looks now
void UIVisualPreview::FrameModel()
{
	m_Pan.set	(0.f,0.f,0.f);

	if (m_Box.min.x>m_Box.max.x||m_Box.min.y>m_Box.max.y||m_Box.min.z>m_Box.max.z)
	{
		m_Center.set(0.f,0.f,0.f);
		m_Radius	= 1.f;
	}
	else
	{
		m_Box.getcenter	(m_Center);
		Fvector size;	m_Box.getsize(size);
		m_Radius		= 0.5f*size.magnitude();
		if (m_Radius<EPS_L)	m_Radius = EPS_L;
	}
	m_Distance	= m_Radius/_sin(deg2rad(s_FOVDegrees)*0.5f)*1.12f;
}

//------------------------------------------------------------------------------
// drawing
//------------------------------------------------------------------------------
void UIVisualPreview::RenderToTarget(u32 w, u32 h)
{
	if ((!m_Visual&&!m_Editable)||!IsDeviceUsable())	return;
	Fvector center	= m_Center;	center.add(m_Pan);
	Fvector dir;
	dir.x	= _cos(m_Pitch)*_sin(m_Yaw);
	dir.y	= _sin(m_Pitch);
	dir.z	= _cos(m_Pitch)*_cos(m_Yaw);
	Fvector eye;	eye.mad(center,dir,m_Distance);
	Fvector view;	view.sub(center,eye).normalize_safe();
#if defined(USE_DX11)
	if (!m_Target.valid())				return;

	SPreviewStateGuard	guard;

	// Invalidate, NOT OnFrameEnd: under D3D11 OnFrameEnd calls ClearState(),
	// which also drops the viewport, and nothing would set one again before the
	// draw - everything gets clipped and the preview stays empty. Invalidate
	// only forgets the cache, so the bind below is what the device ends up with.
	RCache.Invalidate();

	// bind() sets a viewport over the whole target, which comes in 64 px steps: the view is its top-left corner
	m_Target.bind	();
	m_Target.clear	(BackgroundColor());
	{
		D3D11_VIEWPORT vp = {};
		vp.Width	= float(w);
		vp.Height	= float(h);
		vp.MaxDepth	= 1.f;
		HW.pContext->RSSetViewports(1, &vp);
	}
	// and now tell the cache what the device is actually bound to
	RCache.set_RT	(m_Target.rtv);
	RCache.set_ZB	(m_Target.dsv);
#else
	if (!m_RTSurf||!m_ZBSurf)			return;

	SPreviewStateGuard	guard;

	if (FAILED(HW.pDevice->SetRenderTarget(0,m_RTSurf)))		return;
	if (FAILED(HW.pDevice->SetDepthStencilSurface(m_ZBSurf)))	return;

	D3DVIEWPORT9 vp;
	vp.X = 0; vp.Y = 0; vp.Width = w; vp.Height = h; vp.MinZ = 0.f; vp.MaxZ = 1.f;
	HW.pDevice->SetViewport	(&vp);
	HW.pDevice->Clear		(0,0,D3DCLEAR_ZBUFFER|D3DCLEAR_TARGET|(HW.Caps.bStencil?D3DCLEAR_STENCIL:0),BackgroundColor(),1.f,0);
#endif

	// imgui (or the scene pass) may have written device state behind RCache's
	// back - clear the cache so our draw sets everything up itself
	// (the D3D11 path already did this above, before binding)
#if !defined(USE_DX11)
	RCache.OnFrameEnd		();
#endif
	const bool studio = 0 == m_Lighting;
	SetupPreviewStates		(m_Wireframe, studio);
	SetupPreviewLights		(studio, view);

	const float fov		= deg2rad(s_FOVDegrees);
	const float znear	= _max(0.01f,m_Distance-m_Radius*2.f);
	const float zfar	= m_Distance+m_Radius*4.f+1.f;

	Fmatrix mV, mP;
	mV.build_camera		(eye,center,Fvector().set(0.f,1.f,0.f));
	// X-Ray takes the aspect as height/width
	mP.build_projection	(fov,float(h)/float(w),znear,zfar);

	RCache.set_xform_world	(Fidentity);
	RCache.set_xform_view	(mV);
	RCache.set_xform_project(mP);
	EDevice->mView			= mV;
	EDevice->mProject		= mP;
	EDevice->mFullTransform.mul(mP,mV);
	EDevice->vCameraPosition= eye;
	EDevice->vCameraDirection= view;
	// CModelPool::Render culls through CRender::occ_visible - without this the
	// model is thrown away before it is ever drawn
	RImplementation.ViewBase.CreateFromMatrix(EDevice->mFullTransform,FRUSTUM_P_ALL);

	DrawFloor			(m_Box,m_Radius);
	if (m_Visual)	RImplementation.model_RenderSingle(m_Visual,Fidentity,1.f);
	else			m_Editable->RenderSingle(Fidentity);

	if (m_ShowBones&&m_Visual)
	{
		if (IKinematics* K = m_Visual->dcast_PKinematics())
		{
			EDevice->SetShader		(EDevice->m_WireShader);
			RCache.set_xform_world	(Fidentity);
			// scale the markers to the model, otherwise they either vanish or
			// swallow the mesh depending on how big the asset is
			const float js = m_Radius*s_JointSize*2.f;
			const u16 cnt = K->LL_BoneCount();
			for (u16 b=0; b<cnt; ++b)
			{
				const Fmatrix& M = K->LL_GetTransform(b);
				DU_impl.DrawJoint(M.c,js,s_JointColor);
				const u16 parent = K->LL_GetData(b).GetParentID();
				if (u16(-1)!=parent)
					DU_impl.DrawLine(K->LL_GetTransform(parent).c,M.c,s_LinkColor);
			}
		}
	}
}

void UIVisualPreview::DrawToolBar(XUI::Toolbar& tb)
{
	const bool model = m_Visual || m_Editable;
	const char* none = "No model is open";
	if (!model)
		return;
	tb.Group(3);
	if (tb.Action("frame", XUI::Icon::FrameAll, "Frame", model ? "Frame the model, keeping the angle (F)" : none, model))
		FrameModel();
	if (tb.Action("reset_view", XUI::Icon::Refresh, "Reset View", model ? "Back to the three-quarter view, framed" : none, model))
		ResetCamera();

	tb.Group(2);
	if (tb.Action("wireframe", XUI::Icon::Wireframe, "Wireframe", "Draw the triangles' edges only", model, m_Wireframe))
		m_Wireframe = !m_Wireframe;
	const bool bones = m_Visual && m_Bones;
	if (tb.Action("bones", XUI::Icon::Bones, "Bones", bones ? "Draw the skeleton over the model" : "The model has no bones", bones, m_ShowBones && bones))
		m_ShowBones = !m_ShowBones;

	tb.Group(5);
	static const char* const kLighting[] = { "Studio", "Unlit" };
	tb.Segmented("lighting", "Lighting", &m_Lighting, kLighting, 2,
		"Studio lights the model from the camera, over a soft ambient; Unlit shows its textures as they are");

	tb.Group(4);
	if (tb.Action("details", XUI::Icon::SidebarRight, "Details", "Show what the model is made of beside the viewport", true, m_ShowDetails))
		m_ShowDetails = !m_ShowDetails;
}

void UIVisualPreview::DrawStatus()
{
	if (!m_Visual && !m_Editable)
		return;
	xr_string text = NqUtil::Format("%u verts \xC2\xB7 %u tris", m_Verts, m_Tris);
	if (m_Bones)
		text += NqUtil::Format(" \xC2\xB7 %u bones", u32(m_Bones));
	text += m_Lighting ? " \xC2\xB7 Unlit" : " \xC2\xB7 Studio";
	ImGui::TextUnformatted(text.c_str());
}

// the orbit viewport: the left button turns the model, the right or middle one pans, the wheel dollies, a
// double-click frames it
void UIVisualPreview::DrawViewport(const ImVec2& canvas_pos, const ImVec2& avail)
{
	const ImVec2 canvas_size(_max(avail.x, 64.f), _max(avail.y, 64.f));

	// the hit area goes down first so navigation is applied before the frame is
	// rendered - otherwise every drag lags one frame behind the cursor
	ImGui::SetCursorScreenPos(canvas_pos);
	ImGui::InvisibleButton("##preview_canvas",canvas_size,
		ImGuiButtonFlags_MouseButtonLeft|ImGuiButtonFlags_MouseButtonRight|ImGuiButtonFlags_MouseButtonMiddle);

	const bool hovered	= ImGui::IsItemHovered();
	const bool active	= ImGui::IsItemActive();

	if (active)
	{
		const ImVec2 d = ImGui::GetIO().MouseDelta;
		if (ImGui::IsMouseDown(ImGuiMouseButton_Middle)||ImGui::IsMouseDown(ImGuiMouseButton_Right))
		{
			// 1:1 screen panning: how much world one pixel covers at the pivot
			const float per_px = 2.f*m_Distance*tanf(deg2rad(s_FOVDegrees)*0.5f)/canvas_size.y;
			Fvector fwd;	fwd.set(-_cos(m_Pitch)*_sin(m_Yaw),-_sin(m_Pitch),-_cos(m_Pitch)*_cos(m_Yaw));
			Fvector right;	right.crossproduct(Fvector().set(0.f,1.f,0.f),fwd);	right.normalize_safe();
			Fvector up;		up.crossproduct(fwd,right);							up.normalize_safe();
			m_Pan.mad	(right,-d.x*per_px);
			m_Pan.mad	(up,    d.y*per_px);
		}
		else if (ImGui::IsMouseDown(ImGuiMouseButton_Left))
		{
			// drag right turns the model to the right: the camera has to orbit
			// the other way, which is +d.x for this yaw convention
			m_Yaw	+= d.x*s_OrbitSpeed;
			m_Pitch	+= d.y*s_OrbitSpeed;
			if (m_Pitch> s_PitchLimit)	m_Pitch =  s_PitchLimit;
			if (m_Pitch<-s_PitchLimit)	m_Pitch = -s_PitchLimit;
		}
	}
	if (hovered)
	{
		const float wheel = ImGui::GetIO().MouseWheel;
		if (wheel!=0.f)
		{
			m_Distance *= expf(-wheel*0.15f);
			const float lo = m_Radius*0.05f, hi = m_Radius*100.f;
			if (m_Distance<lo)	m_Distance = lo;
			if (m_Distance>hi)	m_Distance = hi;
		}
		if (ImGui::IsMouseDoubleClicked(ImGuiMouseButton_Left))
			FrameModel();
	}

	const u32 w = u32(canvas_size.x), h = u32(canvas_size.y);
	if (EnsureTarget(w,h))
	{
		RenderToTarget(w,h);
		// the view is the target's top-left corner
		const ImVec2 uv1(float(w)/float(m_RTW), float(h)/float(m_RTH));
#if defined(USE_DX11)
		ImDrawList* dl = ImGui::GetWindowDrawList();
		const ImDrawCallback reset = ImGui::GetPlatformIO().DrawCallback_ResetRenderState;
		// without a reset to follow it the opaque state would leak into the rest of the UI: blended as before then
		if (reset)
			dl->AddCallback(&SetOpaqueBlend, nullptr);
		dl->AddImage(m_Target.srv,canvas_pos,ImVec2(canvas_pos.x+canvas_size.x,canvas_pos.y+canvas_size.y),ImVec2(0,0),uv1);
		if (reset)
			dl->AddCallback(reset, nullptr);
#else
		ImGui::GetWindowDrawList()->AddImage(m_RTTex,canvas_pos,ImVec2(canvas_pos.x+canvas_size.x,canvas_pos.y+canvas_size.y),ImVec2(0,0),uv1);
#endif
	}
	else
	{
		ImGui::SetCursorScreenPos(canvas_pos);
		XUI::EmptyState(XUI::Icon::Warning, "The viewport cannot be drawn", NqUtil::Format("The device refused a %u x %u render target.", w, h).c_str());
	}
}

void UIVisualPreview::DrawDetails()
{
	if (!m_Visual && !m_Editable)
	{
		XUI::EmptyState(XUI::Icon::Info, "No facts", "Open a model to see what it is made of.");
		return;
	}
	// library objects carry no MT_* type - name the format instead
	LPCSTR	kind	= m_Editable ? "library object" : VisualTypeName(m_Type);
	Fvector	size;	m_Box.getsize(size);
	Fvector	centre;	m_Box.getcenter(centre);
	if (!XUI::BeginPropertyGrid("model"))
		return;
	if (XUI::PropertySection("Asset"))
	{
		xr_string leaf = m_Name;
		const size_t slash = leaf.find_last_of("\\/");
		if (slash != xr_string::npos) leaf = leaf.substr(slash + 1);
		if (XUI::PropertyRow("Name", "The model's file name", XUI::PropReadOnly))
			XUI::PropertyText(leaf.c_str());
		if (XUI::PropertyRow("Path", "Where it lives, below the game's meshes or the project", XUI::PropReadOnly))
		{
			const float w = ImGui::CalcItemWidth();
			ImGui::AlignTextToFramePadding();
			ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(GrizzlyTheme::ColorToken::TextSecondary));
			XUI::TextEllipsis(m_Name.c_str(), w, XUI::Clip::Start);
			ImGui::PopStyleColor();
		}
		if (XUI::PropertyRow("Kind", "What kind of visual the engine sees", XUI::PropReadOnly))
			XUI::PropertyText(kind);
		XUI::EndPropertySection();
	}
	if (XUI::PropertySection("Geometry"))
	{
		if (XUI::PropertyRow("Vertices", nullptr, XUI::PropReadOnly))
			XUI::PropertyText(NqUtil::Format("%u", m_Verts).c_str());
		if (XUI::PropertyRow("Triangles", nullptr, XUI::PropReadOnly))
			XUI::PropertyText(NqUtil::Format("%u", m_Tris).c_str());
		if (m_Bones && XUI::PropertyRow("Bones", nullptr, XUI::PropReadOnly))
			XUI::PropertyText(NqUtil::Format("%u", u32(m_Bones)).c_str());
		if (m_Editable)
		{
			if (XUI::PropertyRow("Meshes", nullptr, XUI::PropReadOnly))
				XUI::PropertyText(NqUtil::Format("%d", m_Editable->MeshCount()).c_str());
			// a shared_str that was never set is null, not empty
			LPCSTR lods = m_Editable->GetLODs();
			if (lods && lods[0] && XUI::PropertyRow("LOD", "The level-of-detail model it switches to", XUI::PropReadOnly))
				XUI::PropertyText(lods);
			if (m_Editable->IsSkeleton() && XUI::PropertyRow("Motions", nullptr, XUI::PropReadOnly))
				XUI::PropertyText(NqUtil::Format("%d", m_Editable->SMotionCount()).c_str());
		}
		XUI::EndPropertySection();
	}
	if (XUI::PropertySection("Bounds"))
	{
		if (XUI::PropertyRow("Size", "Its box, in metres", XUI::PropReadOnly))
			XUI::PropertyText(NqUtil::Format("%.2f x %.2f x %.2f", size.x, size.y, size.z).c_str());
		if (XUI::PropertyRow("Centre", "The box's centre", XUI::PropReadOnly))
			XUI::PropertyText(NqUtil::Format("%.2f, %.2f, %.2f", centre.x, centre.y, centre.z).c_str());
		if (XUI::PropertyRow("Radius", "The radius of the sphere around it", XUI::PropReadOnly))
			XUI::PropertyText(NqUtil::Format("%.2f", m_Radius).c_str());
		XUI::EndPropertySection();
	}
	// what the engine visual draws with, and where each texture comes from: one the loader cannot find draws as its
	// placeholder, a dark blue that reads as an unlit model
	if (!m_Textures.empty())
	{
		const xr_string badge = NqUtil::Format("%u", u32(m_Textures.size()));
		if (XUI::PropertySection("Textures", true, badge.c_str()))
		{
			static const char* const kFrom[] = { "Not found", "Project", "Editor", "Game" };
			static const char* const kWhy[] = {
				"Neither the editor nor the linked game has it: the preview draws the placeholder",
				"The project's own file",
				"The editor's library",
				"The linked game's archives or its gamedata folder" };
			for (u32 i = 0; i < m_Textures.size(); ++i)
			{
				const STexture& t = m_Textures[i];
				xr_string leaf = t.name;
				const size_t slash = leaf.find_last_of("\\/");
				if (slash != xr_string::npos) leaf = leaf.substr(slash + 1);
				const xr_string hint = NqUtil::Format("%s\n%s", t.name.c_str(), kWhy[t.from]);
				ImGui::PushID(int(i));
				if (XUI::PropertyRow(leaf.c_str(), hint.c_str(), XUI::PropReadOnly))
				{
					if (tfMissing == t.from)
					{
						ImGui::AlignTextToFramePadding();
						ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(GrizzlyTheme::ColorToken::Warning));
						XUI::TextEllipsis(kFrom[t.from], ImGui::CalcItemWidth());
						ImGui::PopStyleColor();
					}
					else
						XUI::PropertyText(kFrom[t.from]);
				}
				ImGui::PopID();
			}
			XUI::EndPropertySection();
		}
	}
	// Surfaces are what a static mesh IS to whoever has to ship it: which texture, which shader, which game
	// material. Unreal puts the same list in its material slots panel.
	if (m_Editable && m_Editable->SurfaceCount())
	{
		const xr_string badge = NqUtil::Format("%d", m_Editable->SurfaceCount());
		if (XUI::PropertySection("Materials", true, badge.c_str()))
		{
			SurfaceVec& sv = m_Editable->Surfaces();
			for (u32 i = 0; i < sv.size(); ++i)
			{
				CSurface* s = sv[i];
				if (!s) continue;
				ImGui::PushID(int(i));
				// a field that was never set is an EMPTY shared_str, which dereferences to null
				LPCSTR name = s->_Name(), tex = s->_Texture(), shader = s->_ShaderName(), mtl = s->_GameMtlName();
				if (XUI::PropertyRow(name && name[0] ? name : "(unnamed)", "The texture of this slot", XUI::PropReadOnly))
				{
					const float w = ImGui::CalcItemWidth();
					ImGui::AlignTextToFramePadding();
					ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(GrizzlyTheme::ColorToken::TextSecondary));
					XUI::TextEllipsis(tex && tex[0] ? tex : "-", w, XUI::Clip::Start);
					ImGui::PopStyleColor();
				}
				if (XUI::PropertyRow("Shader", "The engine shader it draws with", XUI::PropReadOnly, 1))
					XUI::PropertyText(shader && shader[0] ? shader : "-");
				if (XUI::PropertyRow("Game Material", "What the game makes of hits and steps on it", XUI::PropReadOnly, 1))
					XUI::PropertyText(mtl && mtl[0] ? mtl : "-");
				ImGui::PopID();
			}
			XUI::EndPropertySection();
		}
	}
	XUI::EndPropertyGrid();
}

// the model tab's work area: viewport | Details; Details keeps its pt size, the viewport takes every resize
void UIVisualPreview::Draw()
{
	m_DrawnFrame = ImGui::GetFrameCount();
	const ImGuiWindowFlags still = ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse;
	float details_w = kDetailsW;
	if (m_ShowDetails)
		XUI::SplitterTail("model.details", true, &details_w, kCanvasMin, kSideMin, kDetailsW);
	m_DetailsW = (m_ShowDetails && (m_Visual || m_Editable)) ? details_w : 0.f;
	const float canvas_w = (m_ShowDetails && (m_Visual || m_Editable)) ? _max(1.f, ImGui::GetContentRegionAvail().x - XUI::Pt(details_w) - ImGui::GetStyle().ItemSpacing.x) : 0.f;
	ImGui::PushStyleColor(ImGuiCol_ChildBg, GrizzlyTheme::U32(GrizzlyTheme::ColorToken::Canvas));
	const bool canvas = ImGui::BeginChild("##model_viewport", ImVec2(canvas_w, 0.f), ImGuiChildFlags_None, still);
	ImGui::PopStyleColor();
	if (canvas)
	{
		if (!m_Error.empty())
		{
			// nothing loaded - do not sit on a render target either
			ReleaseTarget();
			XUI::EmptyState(XUI::Icon::Warning, "This model cannot be shown", (m_Name + ": " + m_Error).c_str());
		}
		else if (!m_Visual && !m_Editable)
		{
			ReleaseTarget();
		{
			const ImVec2 from = ImGui::GetCursorScreenPos(), room = ImGui::GetContentRegionAvail();
			XUI::EmptyState(XUI::Icon::FileModel, "No model", "Drop a model here, or double-click one in the Content Browser.");
			XUI::Strings dropped;
			if (XUI::FileDropZone("##drop_model", from, ImVec2(from.x + room.x, from.y + room.y), dropped))
				Show(dropped[0].c_str());
		}
		}
		else
			DrawViewport(ImGui::GetCursorScreenPos(), ImGui::GetContentRegionAvail());
	}
	ImGui::EndChild();
	if (m_ShowDetails && (m_Visual || m_Editable))
	{
		ImGui::SameLine();
		if (ImGui::BeginChild("##model_details", ImVec2(0.f, 0.f), ImGuiChildFlags_AlwaysUseWindowPadding))
			DrawDetails();
		ImGui::EndChild();
	}
}

void UIVisualPreview::McpState(xr_string& out)
{
	out += "\"open\":";
	out += Form ? "true" : "false";
	if (!Form)
		return;
	UIVisualPreview* f = Form;
	out += ",\"name\":";
	NqUtil::JsonString(out, f->m_Name);
	out += ",\"error\":";
	NqUtil::JsonString(out, f->m_Error);
	out += ",\"kind\":";
	NqUtil::JsonString(out, f->m_Editable ? "library object" : (f->m_Visual ? VisualTypeName(f->m_Type) : ""));
	out += NqUtil::Format(",\"verts\":%u,\"tris\":%u,\"bones\":%u,\"lighting\":\"%s\",\"wireframe\":%s,\"bones_shown\":%s,"
		"\"panes\":{\"details\":%s,\"details_width\":%.0f}", f->m_Verts, f->m_Tris, u32(f->m_Bones), f->m_Lighting ? "unlit" : "studio",
		f->m_Wireframe ? "true" : "false", f->m_ShowBones && f->m_Bones && f->m_Visual ? "true" : "false", f->m_ShowDetails && (f->m_Visual || f->m_Editable) ? "true" : "false",
		f->m_ShowDetails ? f->m_DetailsW : 0.f);
	// where the texture loader finds each texture: project | editor | game | missing (drawn as its placeholder)
	static const char* const kFrom[] = { "missing", "project", "editor", "game" };
	out += ",\"textures\":[";
	for (u32 i = 0; i < f->m_Textures.size(); ++i)
	{
		out += i ? ",{\"name\":" : "{\"name\":";
		NqUtil::JsonPath(out, f->m_Textures[i].name.c_str());
		out += NqUtil::Format(",\"from\":\"%s\"}", kFrom[f->m_Textures[i].from]);
	}
	out += "]";
}

bool UIVisualPreview::DrawnLately()
{
	// the frame being built or the one before it
	return Form && ImGui::GetCurrentContext() && Form->m_DrawnFrame >= ImGui::GetFrameCount() - 1;
}

bool UIVisualPreview::McpView(int lighting, int wireframe, int bones, int details, bool frame, bool reset, xr_string& err)
{
	if (!Form)
	{
		err = "no model is open";
		return false;
	}
	if (lighting >= 0)	Form->m_Lighting = lighting ? 1 : 0;
	if (wireframe >= 0)	Form->m_Wireframe = wireframe > 0;
	if (bones >= 0)		Form->m_ShowBones = bones > 0;
	if (details >= 0)	Form->m_ShowDetails = details > 0;
	if (reset)			Form->ResetCamera();
	else if (frame)		Form->FrameModel();
	return true;
}

//------------------------------------------------------------------------------
// Wiring. Both targets are plain pointers with no dynamic initialiser of their
// own, so claiming them from a static object here cannot be undone later.
//------------------------------------------------------------------------------
namespace
{
	struct SVisualPreviewHooks
	{
		SVisualPreviewHooks()
		{
			// content browser: double click routes here instead of spawning
			UIContentBrowser::SetVisualPreview(&UIVisualPreview::Show,&UIVisualPreview::ShowFromMemory,
											   &UIVisualPreview::ShowObjectFromMemory);
			// device reset: our render target must die before HW.Reset
			g_pOnEditorDeviceResetBegin	= &UIVisualPreview::OnDeviceResetBegin;
			g_pOnEditorDeviceResetEnd	= &UIVisualPreview::OnDeviceResetEnd;
		}
	} s_VisualPreviewHooks;
}
