#include "stdafx.h"
#pragma hdrstop

#include "EThumbnail.h"
#include "EDX11Utils.h"
#include "EThumbnailVisual.h"
#include "RedImageTool/RedImage.hpp"
#ifndef XR_EPROPS_EXPORTS
	#include "ImageManager.h"
#endif
#pragma package(smart_init)

//------------------------------------------------------------------------------
#define THM_TEXTURE_VERSION				0x0012
//------------------------------------------------------------------------------

//------------------------------------------------------------------------------
ETextureThumbnail::ETextureThumbnail(LPCSTR src_name, bool bLoad):EImageThumbnail(src_name,ETTexture)
{
    if(!strchr(src_name,'\\'))
    {
      xr_string _name                     	= src_name;
      ImageLib.UpdateFileName             	(_name);
      m_Name                              	= _name.c_str();
      m_Name 	                			= ChangeFileExt(m_Name,".thm");
     }
    m_bValid                            	= false;
    if (bLoad)
#ifdef XR_EPROPS_EXPORTS
    	Load();
#else
		if (!Load())
                {
                 ImageLib.CreateTextureThumbnail(this,src_name);
                 }
#endif
}
//------------------------------------------------------------------------------

ETextureThumbnail::~ETextureThumbnail()
{
	m_Pixels.clear();
}
//------------------------------------------------------------------------------

int ETextureThumbnail::MemoryUsage()
{
	int mem_usage = _Width()*_Height()*4;
    switch (m_TexParams.fmt){
    case STextureParams::tfDXT1:
    case STextureParams::tfADXT1: 	mem_usage/=6; break;
    case STextureParams::tfDXT3:
    case STextureParams::tfDXT5: 	mem_usage/=4; break;
    case STextureParams::tf4444:
    case STextureParams::tf1555:
    case STextureParams::tf565: 	mem_usage/=2; break;
    case STextureParams::tfRGBA:	break;
    }
    string_path fn;
    FS.update_path	(fn,_game_textures_,EFS.ChangeFileExt(m_Name.c_str(),".seq").c_str());
    if (FS.exist(fn))
    {
        string128		buffer;
        IReader* F		= FS.r_open(0,fn);
        F->r_string		(buffer,sizeof(buffer));
        int cnt = 0;
        while (!F->eof()){
            F->r_string(buffer,sizeof(buffer));
            cnt++;
        }
        FS.r_close		(F);
        mem_usage *= cnt?cnt:1;
    }
    return mem_usage;
}
//------------------------------------------------------------------------------

void ETextureThumbnail::CreateFromData(u32* p, u32 w, u32 h)
{
	EImageThumbnail::CreatePixels(p, w, h);
    m_TexParams.width = w;
    m_TexParams.height= h;
    m_TexParams.flags.set(STextureParams::flHasAlpha,FALSE);
}
//------------------------------------------------------------------------------

bool Stbi_Load(LPCSTR full_name, U32Vec& data, u32& w, u32& h, u32& a);

bool ETextureThumbnail::Load(LPCSTR src_name, LPCSTR path)
{
    string_path fn;
    strcpy(fn,EFS.ChangeFileExt(src_name?src_name:m_Name.c_str(),".thm").c_str() );
    if (path)
        FS.update_path(fn,path,fn);
    else
    {
        FS.update_path                  (fn,_game_textures_,fn);
    }

    if (!FS.exist(fn)) return false;

    IReader* F 		= FS.r_open(fn);
    u16 version 	= 0;

    R_ASSERT(F->r_chunk(THM_CHUNK_VERSION,&version));
    if( version!=THM_TEXTURE_VERSION ){
		Msg			("!Thumbnail: Unsupported version.");
        return 		false;
    }

/*
    IReader* D 		= F->open_chunk(THM_CHUNK_DATA); R_ASSERT(D);
    m_Pixels.resize	(THUMB_SIZE);
    D->r		 (m_Pixels.begin(),THUMB_SIZE*sizeof(u32));
    D->close		();
*/

    R_ASSERT		(F->find_chunk(THM_CHUNK_TYPE));
    m_Type		= THMType(F->r_u32());
    R_ASSERT		(m_Type==ETTexture);

    m_TexParams.Load(*F);
    m_Age 			= FS.get_file_age(fn);

    FS.r_close		(F);
    SetValid            ();
    return true;
}
//------------------------------------------------------------------------------

void ETextureThumbnail::Save(int age, LPCSTR path)
{
	if (!Valid()) 	return;

    CMemoryWriter F;
	F.open_chunk	(THM_CHUNK_VERSION);
	F.w_u16			(THM_TEXTURE_VERSION);
	F.close_chunk	();

/*
	F.w_chunk		(THM_CHUNK_DATA | CFS_CompressMark,m_Pixels.begin(),m_Pixels.size()*sizeof(u32));
*/        

    F.open_chunk	(THM_CHUNK_TYPE);
    F.w_u32			(m_Type);
	F.close_chunk	();

	m_TexParams.Save(F);

    string_path		fn;
    if (path)
        FS.update_path(fn, path, m_Name.c_str());
    else
    	FS.update_path(fn, _game_textures_, m_Name.c_str());

    if (F.save_to(fn))
    {
	    FS.set_file_age	(fn,age?age:m_Age);
    }else{
        Log			("!Can't save thumbnail:",fn);
    }
}
//------------------------------------------------------------------------------

void ETextureThumbnail::FillProp(PropItemVec& items, PropValue::TOnChange on_type_change)
{
	m_TexParams.FillProp		(m_SrcName.c_str(),items,on_type_change);
}
//------------------------------------------------------------------------------

void ETextureThumbnail::FillInfo(PropItemVec& items)
{                                                                         
	STextureParams& F			= m_TexParams;
    PHelper().CreateCaption		(items, "Format",					get_token_name(tfmt_token,F.fmt));
    PHelper().CreateCaption		(items, "Type",						get_token_name(ttype_token,F.type));
    PHelper().CreateCaption		(items, "Width",					shared_str().printf("%d",_Width()));
    PHelper().CreateCaption		(items, "Height",					shared_str().printf("%d",_Height()));
    PHelper().CreateCaption		(items, "Alpha",					_Alpha()?"on":"off");
}

void ETextureThumbnail::Update(ImTextureID& Texture)
{
    VERIFY(!Texture);
    if (0 == m_Pixels.size())
    {
        u32                 image_w, image_h, image_a;
        xr_string fn_img = EFS.ChangeFileExt(m_Name.c_str(), ".tga");
        string_path fn;
        FS.update_path(fn, _textures_, fn_img.c_str());

        if (!FS.exist(fn))
        {
            fn_img = EFS.ChangeFileExt(m_Name.c_str(), ".dds");
            FS.update_path(fn, _game_textures_, fn_img.c_str());

            if (!FS.exist(fn))
            {
                ELog.Msg(mtError, "Can't make preview for texture '%s'.", m_Name.c_str());
                return;
            }
        }

        U32Vec data;
        u32 w, h, a;
        if (!Stbi_Load(fn, data, image_w, image_h, image_a))
        {

            fn_img = EFS.ChangeFileExt(m_Name.c_str(), ".dds");
            u32	mem = 0;
#if defined(USE_DX11)
            // RedImage first: it decodes BC1-BC7 on the CPU and yields an
            // UNCOMPRESSED B8G8R8A8 texture. texture_load would hand back the
            // compressed resource as-is, which draws fine but cannot be read
            // back - that is what breaks the asset_preview MCP command.
            Texture = DX11TextureFromFile(fn);
            // still keep the engine loader for whatever RedImage cannot parse
            if (!Texture)
            {
                if (ID3DBaseTexture* res = ::RImplementation.texture_load(fn_img.c_str(), mem))
                {
                    // ImGui binds a view, so make one and drop the resource -
                    // the view keeps its own reference
                    ID3D11ShaderResourceView* view = nullptr;
                    if (SUCCEEDED(HW.pDevice->CreateShaderResourceView(res, nullptr, &view)))
                        Texture = view;
                    _RELEASE(res);
                }
            }
#else
            Texture = ::RImplementation.texture_load(fn_img.c_str(), mem);
#endif
            if (!Texture)
                ELog.Msg(mtError, "Can't make preview for texture '%s'.", m_Name.c_str());

            return;
        }
        ImageLib.MakeThumbnailImage(this, data.data(), image_w, image_h, image_a);
    }

    if (Valid())
    {
       
        inherited::Update(Texture);
    }
}

BOOL ETextureThumbnail::similar(ETextureThumbnail* thm1, xr_vector<xr_string>& sel_params)
{
	BOOL res = m_TexParams.similar(thm1->m_TexParams, sel_params);
  /*
    if(res)
    {
		xr_vector<xr_string>::iterator it = sel_params.begin();
		xr_vector<xr_string>::iterator it_e = sel_params.end();

        for(;it!=it_e;++it)
        {
           const xr_string& par_name = *it;
			if(par_name=="Format")
            {
            	res = (m_TexParams.fmt == thm1->m_TexParams.fmt);
            }else
			if(par_name=="Type")
            {
            	res = (m_TexParams.type == thm1->m_TexParams.type);
            }else
			if(par_name=="Width")
            {
            	res = (_Width()==thm1->_Width());
            }else
			if(par_name=="Height")
            {
            	res = (_Height()==thm1->_Height());
            }else
			if(par_name=="Alpha")
            {
            	res = (_Alpha()==thm1->_Alpha());
            }
           if(!res)
           	break;
        }
    }
*/
    return res;
}

LPCSTR ETextureThumbnail::FormatString()
{
    return m_TexParams.FormatString();
}
//------------------------------------------------------------------------------

//------------------------------------------------------------------------------
// Content browser tiles (EThumbnailVisual.h)
//------------------------------------------------------------------------------
static bool TextureTileFromImage(RedImageTool::RedImage& img, u32 side, STextureTile& out)
{
	using RedImageTool::RedTextureUtils;
	using RedImageTool::RedTexturePixelFormat;
	const size_t w0 = img.GetWidth(), h0 = img.GetHeight();
	if (img.Empty() || !w0 || !h0 || !side)
		return false;
	out.src_w = u32(w0);
	out.src_h = u32(h0);

	// the smallest level that still covers `side`; one under a block (4 px) is left alone, the decoders only ever
	// see whole blocks here
	const size_t mips = img.GetMips();
	size_t level = 0;
	while (level + 1 < mips)
	{
		const size_t w = RedTextureUtils::GetMip(w0, level + 1), h = RedTextureUtils::GetMip(h0, level + 1);
		if (_max(w, h) < size_t(side) || _min(w, h) < 4)
			break;
		++level;
	}
	RedImageTool::RedImage picked;
	if (level)
	{
		const RedTexturePixelFormat format = img.GetFormat();
		const size_t w = RedTextureUtils::GetMip(w0, level), h = RedTextureUtils::GetMip(h0, level);
		picked.Create(w, h, 1, 1, format);
		CopyMemory(*picked, RedTextureUtils::GetImage(static_cast<unsigned char*>(*img), w0, h0, mips, 0, level, format),
			RedTextureUtils::GetSizeInMemory(w, h, 1, format));
	}
	else
	{
		img.ClearMipLevels();
		picked.Swap(img);
	}
	picked.Convert(RedTexturePixelFormat::R8G8B8A8);

	const u32 sw = u32(picked.GetWidth()), sh = u32(picked.GetHeight());
	if (picked.Empty() || !sw || !sh)
		return false;
	const u32 longest = _max(sw, sh);
	u32 dw = sw, dh = sh;
	if (longest > side)
	{
		dw = _max(1u, u32((u64(sw) * side + longest / 2) / longest));
		dh = _max(1u, u32((u64(sh) * side + longest / 2) / longest));
	}

	// R8G8B8A8 in, B8G8R8A8 out; every output pixel is the mean of the texels it covers, their colour weighted by
	// their alpha
	const unsigned char* src = static_cast<const unsigned char*>(*picked);
	out.w = dw;
	out.h = dh;
	out.pixels.resize(size_t(dw) * dh);
	u32 alpha_min = 255, alpha_max = 0;
	for (u32 y = 0; y < dh; ++y)
	{
		const u32 y0 = u32(u64(y) * sh / dh), y1 = _max(y0 + 1, u32(u64(y + 1) * sh / dh));
		for (u32 x = 0; x < dw; ++x)
		{
			const u32 x0 = u32(u64(x) * sw / dw), x1 = _max(x0 + 1, u32(u64(x + 1) * sw / dw));
			u64 r = 0, g = 0, b = 0, a = 0;
			for (u32 sy = y0; sy < y1; ++sy)
			{
				const unsigned char* p = src + (size_t(sy) * sw + x0) * 4;
				for (u32 sx = x0; sx < x1; ++sx, p += 4)
				{
					const u32 pa = p[3];
					r += p[0] * pa;
					g += p[1] * pa;
					b += p[2] * pa;
					a += pa;
					alpha_min = _min(alpha_min, pa);
					alpha_max = _max(alpha_max, pa);
				}
			}
			const u64 n = u64(y1 - y0) * u64(x1 - x0);
			const u32 ca = u32((a + n / 2) / n);
			const u32 cr = a ? u32((r + a / 2) / a) : 0;
			const u32 cg = a ? u32((g + a / 2) / a) : 0;
			const u32 cb = a ? u32((b + a / 2) / a) : 0;
			out.pixels[size_t(y) * dw + x] = (ca << 24) | (cr << 16) | (cg << 8) | cb;
		}
	}
	out.alpha = alpha_min == 255 ? 0 : alpha_max == 0 ? 2 : 1;
	return true;
}

bool TextureTileFromFile(LPCSTR path, u32 side, STextureTile& out)
{
	out = STextureTile();
	if (!path || !path[0])
		return false;
	RedImageTool::RedImage img;
	return img.LoadFromFile(path) && TextureTileFromImage(img, side, out);
}

bool TextureTileFromMemory(const void* data, u32 size, u32 side, STextureTile& out)
{
	out = STextureTile();
	if (!data || !size)
		return false;
	RedImageTool::RedImage img;
	// the block is only read; the missing const on the loader is its own
	return img.LoadFromMemory(const_cast<void*>(data), size) && TextureTileFromImage(img, side, out);
}
