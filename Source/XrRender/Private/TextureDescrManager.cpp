#include "stdafx.h"
#pragma hdrstop
#include "TextureDescrManager.h"
#include "ETextureParams.h"
#include <thread>	// the descriptions are read on a few threads, see LoadTHM

// eye-params
float					r__dtex_range	= 50;
class cl_dt_scaler		: public R_constant_setup {
public:
	float				scale;

	cl_dt_scaler		(float s) : scale(s)	{};
	virtual void setup	(R_constant* C)
	{
		RCache.set_c	(C,scale,scale,scale,1/r__dtex_range);
	}
};

void fix_texture_thm_name(LPSTR fn)
{
	LPSTR _ext = strext(fn);
	if(  _ext					&&
	  (0==stricmp(_ext,".tga")	||
		0==stricmp(_ext,".thm")	||
		0==stricmp(_ext,".dds")	||
		0==stricmp(_ext,".bmp")	||
		0==stricmp(_ext,".ogm")	) )
		*_ext = 0;
}

//------------------------------------------------------------------------------
// Reading the descriptions is one open, one read and one close per file, of a
// file a few hundred bytes long - and a stock install has eight and a half
// thousand of them. It is the largest burst of file operations anything does
// before the editor can draw, and what it costs is the per-file latency, not
// the bytes: cold, every one of those is a disk round trip that an on-access
// scanner also gets a say in.
//
// So the READING is spread over a few threads, which is all it takes to
// overlap that latency. The parsing stays on the calling thread: it fills
// state shared with the renderer, and next to the I/O it is free.
//------------------------------------------------------------------------------
namespace
{
struct SThmFile
{
	string_path	path;
	u8*			data;	// plain CRT allocation: filled off-thread, freed here
	u32			size;
};

const u32	kThmMaxThreads	= 8;
const u32	kThmPerThread	= 256;		// below this a single pass is cheaper
const u32	kThmSizeLimit	= 1 << 20;	// a description this big is not one

void ThmReadRange(SThmFile* files, u32 from, u32 to)
{
	for (u32 i = from; i < to; ++i)
	{
		SThmFile& f = files[i];
		HANDLE handle = ::CreateFileA(f.path, GENERIC_READ, FILE_SHARE_READ, NULL,
			OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL | FILE_FLAG_SEQUENTIAL_SCAN, NULL);
		if (INVALID_HANDLE_VALUE == handle)	continue;

		LARGE_INTEGER size = {};
		if (::GetFileSizeEx(handle, &size) && size.QuadPart > 0 && size.QuadPart < kThmSizeLimit)
		{
			const u32	bytes	= u32(size.QuadPart);
			u8*			buffer	= (u8*)::malloc(bytes);
			DWORD		got		= 0;
			if (buffer && ::ReadFile(handle, buffer, bytes, &got, NULL) && got == bytes)
			{
				f.data	= buffer;
				f.size	= got;
			}
			else
				::free(buffer);
		}
		::CloseHandle(handle);
	}
}

// Anything this misses simply arrives with data == 0 and is read the ordinary
// way below - a file that moved, a path the engine resolves through an archive.
void ThmPrefetch(xr_vector<SThmFile>& files)
{
	const u32 count = files.size();
	if (count < kThmPerThread)
	{
		ThmReadRange(&files.front(), 0, count);
		return;
	}

	u32 threads = count / kThmPerThread;
	if (threads > kThmMaxThreads)	threads = kThmMaxThreads;
	if (threads > std::thread::hardware_concurrency())
		threads = std::thread::hardware_concurrency();
	if (threads < 2)
	{
		ThmReadRange(&files.front(), 0, count);
		return;
	}

	std::thread	workers[kThmMaxThreads];
	u32			started	= 0;
	const u32	span	= (count + threads - 1) / threads;
	for (u32 t = 1; t < threads; ++t)
	{
		const u32 from	= t * span;
		const u32 to	= (from + span < count) ? from + span : count;
		if (from >= to)	break;
		workers[started++] = std::thread(&ThmReadRange, &files.front(), from, to);
	}
	// the caller takes the first slice itself rather than waiting on it
	ThmReadRange(&files.front(), 0, span < count ? span : count);
	for (u32 t = 0; t < started; ++t)
		workers[t].join();
}
} // namespace

void CTextureDescrMngr::LoadTHM(LPCSTR initial)
{
	FS_FileSet				flist;
	FS.file_list			(flist, initial, FS_ListFiles, "*.thm");
#ifdef DEBUG
	Msg						("count of .thm files=%d", flist.size());
#endif // #ifdef DEBUG
	if (flist.empty())		return;

	xr_vector<SThmFile>		prefetched;
	prefetched.resize		(flist.size());
	u32						slot = 0;
	for (FS_FileSetIt I = flist.begin(); I != flist.end(); ++I, ++slot)
	{
		FS.update_path		(prefetched[slot].path, initial, (*I).name.c_str());
		prefetched[slot].data = 0;
		prefetched[slot].size = 0;
	}
	ThmPrefetch				(prefetched);

	FS_FileSetIt It			= flist.begin();
	FS_FileSetIt It_e		= flist.end();
	STextureParams			tp;
	string_path				fn;
	slot					= 0;
	for(;It!=It_e;++It,++slot)
	{
		SThmFile&	held	= prefetched[slot];
		IReader		memory	(held.data, int(held.size));
		IReader*	F		= held.data ? &memory : FS.r_open(held.path);
		xr_strcpy			(fn,(*It).name.c_str());
		fix_texture_thm_name(fn);

		R_ASSERT			(F->find_chunk(THM_CHUNK_TYPE));
		F->r_u32			();
		tp.Clear			();
		tp.Load				(*F);
		if (F == &memory)	{ ::free(held.data); held.data = 0; }
		else				FS.r_close(F);
		if (STextureParams::ttImage		== tp.type ||
			STextureParams::ttTerrain	== tp.type ||
			STextureParams::ttNormalMap	== tp.type	)
		{
			texture_desc&	desc	= m_texture_details[fn];
			cl_dt_scaler*&	dts		= m_detail_scalers[fn];

			if( tp.detail_name.size() &&
				tp.flags.is_any(STextureParams::flDiffuseDetail|STextureParams::flBumpDetail) )
			{
				if(desc.m_assoc)
					xr_delete				(desc.m_assoc);

				desc.m_assoc				= xr_new<texture_assoc>();
				desc.m_assoc->detail_name	= tp.detail_name;
				if (dts)
					dts->scale = tp.detail_scale;
				else
					/*desc.m_assoc->cs*/dts	= xr_new<cl_dt_scaler>(tp.detail_scale);

				desc.m_assoc->usage			= 0;
				
				if( tp.flags.is(STextureParams::flDiffuseDetail) )
					desc.m_assoc->usage		|= (1<<0);
				
				if( tp.flags.is(STextureParams::flBumpDetail) )
					desc.m_assoc->usage		|= (1<<1);

			}
			if(desc.m_spec)
				xr_delete				(desc.m_spec);

			desc.m_spec					= xr_new<texture_spec>();
			desc.m_spec->m_material		= tp.material+tp.material_weight;
			desc.m_spec->m_use_steep_parallax = false;
			
			if(tp.bump_mode==STextureParams::tbmUse)
			{
				desc.m_spec->m_bump_name	= tp.bump_name;
			}
			else if (tp.bump_mode==STextureParams::tbmUseParallax)
			{
				desc.m_spec->m_bump_name	= tp.bump_name;
				desc.m_spec->m_use_steep_parallax = true;
			}

		}
	}
}

void CTextureDescrMngr::Load()
{
#ifdef DEBUG
	CTimer					TT;
	TT.Start				();
#endif // #ifdef DEBUG

	LoadTHM					("$game_textures$");
	LoadTHM					("$level$");

#ifdef DEBUG
	Msg("load time=%d ms",TT.GetElapsed_ms());
#endif // #ifdef DEBUG
}

void CTextureDescrMngr::UnLoad()
{
	map_TD::iterator I = m_texture_details.begin();
	map_TD::iterator E = m_texture_details.end();
	for(;I!=E;++I)
	{
		xr_delete(I->second.m_assoc);
		xr_delete(I->second.m_spec);
	}
	m_texture_details.clear	();
}

CTextureDescrMngr::~CTextureDescrMngr()
{
	map_CS::iterator I = m_detail_scalers.begin();
	map_CS::iterator E = m_detail_scalers.end();

	for(;I!=E;++I)
		xr_delete(I->second);

	m_detail_scalers.clear	();
}

shared_str CTextureDescrMngr::GetBumpName(const shared_str& tex_name) const
{
	map_TD::const_iterator I = m_texture_details.find	(tex_name);
	if (I!=m_texture_details.end())
	{
		if(I->second.m_spec)
		{
			return I->second.m_spec->m_bump_name;
		}	
	}
	return "";
}

BOOL CTextureDescrMngr::UseSteepParallax(const shared_str& tex_name) const
{
	map_TD::const_iterator I = m_texture_details.find	(tex_name);
	if (I!=m_texture_details.end())
	{
		if(I->second.m_spec)
		{
			return I->second.m_spec->m_use_steep_parallax;
		}	
	}
	return FALSE;
}

float CTextureDescrMngr::GetMaterial(const shared_str& tex_name) const
{
	map_TD::const_iterator I = m_texture_details.find	(tex_name);
	if (I!=m_texture_details.end())
	{
		if(I->second.m_spec)
		{
			return I->second.m_spec->m_material;
		}
	}
	return 1.0f;
}

void CTextureDescrMngr::GetTextureUsage	(const shared_str& tex_name, BOOL& bDiffuse, BOOL& bBump) const
{
	map_TD::const_iterator I = m_texture_details.find	(tex_name);
	if (I!=m_texture_details.end())
	{
		if(I->second.m_assoc)
		{
			u8 usage	= I->second.m_assoc->usage;
			bDiffuse	= !!(usage & (1<<0));
			bBump		= !!(usage & (1<<1));
		}	
	}
}

BOOL CTextureDescrMngr::GetDetailTexture(const shared_str& tex_name, LPCSTR& res, R_constant_setup* &CS) const
{
	map_TD::const_iterator I = m_texture_details.find	(tex_name);
	if (I!=m_texture_details.end())
	{
		if(I->second.m_assoc)
		{
            texture_assoc* TA = I->second.m_assoc;
			res	= TA->detail_name.c_str();
			map_CS::const_iterator It2 = m_detail_scalers.find(tex_name);
			CS	= It2==m_detail_scalers.end()?0:It2->second;//TA->cs;
			return TRUE;
		}
	}
	return FALSE;
}

