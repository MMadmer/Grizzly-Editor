#include "stdafx.h"
#pragma hdrstop

#include "EStats.h"
#include "hw.h"
#include "gamefont.h"

//////////////////////////////////////////////////////////////////////
// Construction/Destruction
//////////////////////////////////////////////////////////////////////

CEStats::CEStats()
{
	fFPS		= 30.f;
	fRFPS		= 30.f;
	fTPS		= 0;
	dwLevelSelFaceCount	= 0;
	dwLevelSelVertexCount=0;
}

CEStats::~CEStats()
{

}

#include "igame_persistent.h"
void CEStats::Show(CGameFont* font)
{
	// Stop timers
	{
		RenderTOTAL.FrameEnd		();
		RenderCALC.FrameEnd			();

        RenderDUMP_SKIN.FrameEnd	();

		Animation.FrameEnd			();
		Input.FrameEnd				();
		clRAY.FrameEnd				();
		clBOX.FrameEnd				();
        clFRUSTUM.FrameEnd			();

        RenderDUMP_RT.FrameEnd		();

		RenderDUMP_DT_VIS.FrameEnd	();
		RenderDUMP_DT_Render.FrameEnd();
		RenderDUMP_DT_Cache.FrameEnd();
        TEST0.FrameEnd				();
        TEST1.FrameEnd				();
        TEST2.FrameEnd				();
        TEST3.FrameEnd				();
	}

	// calc FPS & TPS
    CBackend::_stats& DPS = RCache.stat;
	if (EDevice->fTimeDelta>EPS_S) {
		float fps  = 1.f/EDevice->fTimeDelta;
		float fOne = 0.3f;
		float fInv = 1.f-fOne;
		fFPS = fInv*fFPS + fOne*fps;

		if (RenderTOTAL.result>EPS_S) {
			fTPS = fInv*fTPS + fOne*float(DPS.polys)/(RenderTOTAL.result*1000.f);
			fRFPS= fInv*fRFPS+ fOne*1000.f/RenderTOTAL.result;
		}
	}

	// what the pass drew, before its counters start over
	Shown.fps			= fFPS;
	Shown.render_fps	= fRFPS;
	Shown.tps			= fTPS;
	Shown.verts			= DPS.verts;
	Shown.polys			= DPS.polys;
	Shown.calls			= DPS.calls;
	Shown.lights_scene	= dwLightInScene;
	Shown.lights_total	= dwTotalLight;
	Shown.skeletons_ms	= Animation.result;
	Shown.skeletons		= Animation.count;
	Shown.skinning_ms	= RenderDUMP_SKIN.result;

	{
		Animation.FrameStart		();
		RenderTOTAL.FrameStart		();

		Input.FrameStart			();
		clRAY.FrameStart			();
		clBOX.FrameStart			();
		clFRUSTUM.FrameStart		();

		RenderDUMP_SKIN.FrameStart	();
		RenderDUMP_RT.FrameStart	();

		RenderDUMP_DT_VIS.FrameStart();
		RenderDUMP_DT_Render.FrameStart();
		RenderDUMP_DT_Cache.FrameStart();

		TEST0.FrameStart			();
		TEST1.FrameStart			();
		TEST2.FrameStart			();
		TEST3.FrameStart			();
	}
	dwShader_Codes = dwShader_Textures = dwShader_Matrices = dwShader_Constants = 0;
	dwSND_Played = dwSND_Allocated = 0;
    dwTotalLight = dwLightInScene = 0;

	DPS.polys	= 0;
	DPS.verts	= 0;
	DPS.calls	= 0;
	DPS.vs		= 0;
//	DPS.vb		= 0;
//	DPS.ib		= 0;

	dwLevelSelFaceCount		= 0;
	dwLevelSelVertexCount	= 0;
}
