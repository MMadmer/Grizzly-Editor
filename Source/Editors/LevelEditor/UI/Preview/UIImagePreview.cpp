#include "stdafx.h"

#include "UIImagePreview.h"
#include "../../../XrECore/Editor/EDX11Utils.h"
#include "../../../XrECore/Editor/EditorGameContent.h"
#include "../../../XrECore/Editor/EditorProject.h"
#include "../../../XrECore/Editor/Nq/NqUtil.h"

UIImagePreview* UIImagePreview::Form = nullptr;

namespace
{
	using GrizzlyTheme::ColorToken;

	ImU32 Tok(ColorToken token, float alpha = 1.f) { return GrizzlyTheme::U32(token, alpha); }

	// pt (docs/UI_DESIGN.md, sections 2 and 6): the Details pane and what the canvas keeps
	constexpr float kDetailsW	= 320.f;
	constexpr float kSideMin	= 220.f;
	constexpr float kCanvasMin	= 400.f;
	constexpr float kZoomMin	= 1.f / 64.f;
	constexpr float kZoomMax	= 32.f;

	const char* const kChannels[]		= { "RGBA", "R", "G", "B", "A" };
	const char* const kChannelIds[]		= { "rgba", "r", "g", "b", "a" };

	// A texture swapped or closed inside a frame may already be in that frame's draw list (the tab draws
	// before the Content Drawer that double-clicks the next one): it is released a frame later.
	xr_vector<IUnknown*>	s_Dead;
	int						s_DeadFrame = -1;

	void Bury(IUnknown* p)
	{
		if (!p) return;
		s_Dead.push_back(p);
		s_DeadFrame = ImGui::GetCurrentContext() ? ImGui::GetFrameCount() : -1;
	}

	void FreeDead()
	{
		if (s_Dead.empty() || (ImGui::GetCurrentContext() && ImGui::GetFrameCount() <= s_DeadFrame)) return;
		for (IUnknown* p : s_Dead) p->Release();
		s_Dead.clear();
	}

	// The A view: the image blended onto white with the destination scaled by the source's alpha, so what
	// lands is the alpha itself as grey. ImGui has no blend modes of its own; a draw callback sets this one
	// and the backend's reset puts its own back.
	ID3D11BlendState*	s_AlphaBlend = nullptr;

	void SetAlphaBlend(const ImDrawList*, const ImDrawCmd*)
	{
		if (!s_AlphaBlend && HW.pDevice)
		{
			D3D11_BLEND_DESC desc = {};
			D3D11_RENDER_TARGET_BLEND_DESC& rt = desc.RenderTarget[0];
			rt.BlendEnable				= TRUE;
			rt.SrcBlend					= D3D11_BLEND_ZERO;
			rt.DestBlend				= D3D11_BLEND_SRC_ALPHA;
			rt.BlendOp					= D3D11_BLEND_OP_ADD;
			rt.SrcBlendAlpha			= D3D11_BLEND_ZERO;
			rt.DestBlendAlpha			= D3D11_BLEND_ONE;
			rt.BlendOpAlpha				= D3D11_BLEND_OP_ADD;
			rt.RenderTargetWriteMask	= D3D11_COLOR_WRITE_ENABLE_ALL;
			HW.pDevice->CreateBlendState(&desc, &s_AlphaBlend);
		}
		const float factor[4] = { 0.f, 0.f, 0.f, 0.f };
		if (s_AlphaBlend && HW.pContext)
			HW.pContext->OMSetBlendState(s_AlphaBlend, factor, 0xffffffff);
	}

	// Where a texture of each browser source physically is. The library names a
	// texture without an extension, and the compiled .dds is what the engine
	// actually ships - the uncompressed source is the fallback.
	struct SProbe { LPCSTR alias; LPCSTR ext; };
	static const SProbe s_LibProbes[] =
	{
		{ "$game_textures$",	".dds"	},
		{ "$textures$",			".tga"	},
		{ "$textures$",			".dds"	},
	};

	struct SFormatInfo
	{
		DXGI_FORMAT	fmt;
		LPCSTR		name;
		u32			block;		// bytes per 4x4 block (compressed) or per pixel
		bool		compressed;
		bool		alpha;
		bool		srgb;
	};

	// Only what this editor's decoders can actually produce. Anything else is
	// reported by its numeric id rather than guessed at.
	static const SFormatInfo s_Formats[] =
	{
		{ DXGI_FORMAT_BC1_UNORM,			"BC1 (DXT1)",		8,	true,	false,	false	},
		{ DXGI_FORMAT_BC1_UNORM_SRGB,		"BC1 sRGB",			8,	true,	false,	true	},
		{ DXGI_FORMAT_BC2_UNORM,			"BC2 (DXT3)",		16,	true,	true,	false	},
		{ DXGI_FORMAT_BC2_UNORM_SRGB,		"BC2 sRGB",			16,	true,	true,	true	},
		{ DXGI_FORMAT_BC3_UNORM,			"BC3 (DXT5)",		16,	true,	true,	false	},
		{ DXGI_FORMAT_BC3_UNORM_SRGB,		"BC3 sRGB",			16,	true,	true,	true	},
		{ DXGI_FORMAT_BC4_UNORM,			"BC4 (ATI1)",		8,	true,	false,	false	},
		{ DXGI_FORMAT_BC5_UNORM,			"BC5 (ATI2)",		16,	true,	false,	false	},
		{ DXGI_FORMAT_BC6H_UF16,			"BC6H",				16,	true,	false,	false	},
		{ DXGI_FORMAT_BC7_UNORM,			"BC7",				16,	true,	true,	false	},
		{ DXGI_FORMAT_BC7_UNORM_SRGB,		"BC7 sRGB",			16,	true,	true,	true	},
		{ DXGI_FORMAT_B8G8R8A8_UNORM,		"B8G8R8A8",			4,	false,	true,	false	},
		{ DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,	"B8G8R8A8 sRGB",	4,	false,	true,	true	},
		{ DXGI_FORMAT_R8G8B8A8_UNORM,		"R8G8B8A8",			4,	false,	true,	false	},
		{ DXGI_FORMAT_R8G8B8A8_UNORM_SRGB,	"R8G8B8A8 sRGB",	4,	false,	true,	true	},
		{ DXGI_FORMAT_B8G8R8X8_UNORM,		"B8G8R8X8",			4,	false,	false,	false	},
		{ DXGI_FORMAT_R16G16B16A16_FLOAT,	"R16G16B16A16F",	8,	false,	true,	false	},
		{ DXGI_FORMAT_R8_UNORM,				"R8",				1,	false,	false,	false	},
	};

	const SFormatInfo* FindFormat(DXGI_FORMAT f)
	{
		for (u32 i = 0; i < sizeof(s_Formats)/sizeof(s_Formats[0]); ++i)
			if (s_Formats[i].fmt == f)	return &s_Formats[i];
		return 0;
	}

	// What kind of file the bytes are, by their magic; TGA has none and is named by the extension.
	LPCSTR ContainerOf(const u8* p, u32 size, LPCSTR name)
	{
		if (size >= 4 && !memcmp(p, "DDS ", 4))						return "DDS";
		if (size >= 8 && !memcmp(p, "\x89PNG", 4))					return "PNG";
		if (size >= 3 && p[0] == 0xFF && p[1] == 0xD8 && p[2] == 0xFF)	return "JPEG";
		if (size >= 2 && p[0] == 'B' && p[1] == 'M')					return "BMP";
		LPCSTR ext = name ? strrchr(name, '.') : nullptr;
		if (ext && !_stricmp(ext, ".tga"))								return "TGA";
		return "image";
	}

	// A DDS the GPU takes as it is - a block-compressed or 32-bit 2D texture - is shown in its own format with every
	// mip it holds, which is what the game keeps in memory. Anything else (a cube map, a volume, 24 or 16 bits) is
	// left to the decoder, which makes mip 0 of it B8G8R8A8.
	ID3D11Texture2D* NativeDds(const u8* p, u32 size)
	{
		if (size < 128 || memcmp(p, "DDS ", 4) || !HW.pDevice)
			return nullptr;
		const auto at = [p](u32 offset) { u32 v; memcpy(&v, p + offset, sizeof(v)); return v; };
		// DDS_HEADER after the magic, then DDS_PIXELFORMAT at 76
		const u32 flags = at(8), height = at(12), width = at(16), mip_count = at(28);
		const u32 pf_flags = at(80), four_cc = at(84), bits = at(88), rmask = at(92), gmask = at(96), bmask = at(100), amask = at(104);
		const u32 caps2 = at(112);
		constexpr u32 kCubeMap = 0x200, kVolume = 0x200000, kFourCC = 0x4, kRgb = 0x40, kAlphaPixels = 0x1, kMipCount = 0x20000;
		if (!width || !height || width > D3D11_REQ_TEXTURE2D_U_OR_V_DIMENSION || height > D3D11_REQ_TEXTURE2D_U_OR_V_DIMENSION ||
			(caps2 & (kCubeMap | kVolume)))
			return nullptr;

		u32 data = 128;
		DXGI_FORMAT fmt = DXGI_FORMAT_UNKNOWN;
		if (pf_flags & kFourCC)
		{
			switch (four_cc)
			{
			case MAKEFOURCC('D', 'X', 'T', '1'):	fmt = DXGI_FORMAT_BC1_UNORM; break;
			case MAKEFOURCC('D', 'X', 'T', '2'):
			case MAKEFOURCC('D', 'X', 'T', '3'):	fmt = DXGI_FORMAT_BC2_UNORM; break;
			case MAKEFOURCC('D', 'X', 'T', '4'):
			case MAKEFOURCC('D', 'X', 'T', '5'):	fmt = DXGI_FORMAT_BC3_UNORM; break;
			case MAKEFOURCC('A', 'T', 'I', '1'):
			case MAKEFOURCC('B', 'C', '4', 'U'):	fmt = DXGI_FORMAT_BC4_UNORM; break;
			case MAKEFOURCC('A', 'T', 'I', '2'):
			case MAKEFOURCC('B', 'C', '5', 'U'):	fmt = DXGI_FORMAT_BC5_UNORM; break;
			case MAKEFOURCC('D', 'X', '1', '0'):
				// DDS_HEADER_DXT10: format, dimension (3 = 2D), misc (4 = cube), array size
				if (size < 148 || at(132) != 3 || (at(136) & 0x4) || at(140) > 1)
					return nullptr;
				fmt = DXGI_FORMAT(at(128));
				data = 148;
				break;
			}
		}
		else if ((pf_flags & kRgb) && bits == 32)
		{
			const bool alpha = (pf_flags & kAlphaPixels) && amask == 0xFF000000u;
			if (rmask == 0x00FF0000u && gmask == 0x0000FF00u && bmask == 0x000000FFu)
				fmt = alpha ? DXGI_FORMAT_B8G8R8A8_UNORM : DXGI_FORMAT_B8G8R8X8_UNORM;
			else if (rmask == 0x000000FFu && gmask == 0x0000FF00u && bmask == 0x00FF0000u && alpha)
				fmt = DXGI_FORMAT_R8G8B8A8_UNORM;
		}
		const SFormatInfo* fi = FindFormat(fmt);
		// a block-compressed texture's top level is whole blocks
		if (!fi || (fi->compressed && ((width & 3) || (height & 3))))
			return nullptr;

		u32 full = 1;
		for (u32 s = _max(width, height); s > 1; s >>= 1)
			++full;
		const u32 mips = clampr((flags & kMipCount) && mip_count ? mip_count : 1u, 1u, full);
		xr_vector<D3D11_SUBRESOURCE_DATA> levels(mips);
		u64 offset = data;
		u32 kept = 0;
		// a file cut short keeps the levels it holds whole
		for (; kept < mips; ++kept)
		{
			const u32 w = _max(1u, width >> kept), h = _max(1u, height >> kept);
			const u64 pitch = fi->compressed ? u64(_max(1u, (w + 3) / 4)) * fi->block : u64(w) * fi->block;
			const u64 rows = fi->compressed ? _max(1u, (h + 3) / 4) : h;
			if (offset + pitch * rows > size)
				break;
			levels[kept].pSysMem			= p + offset;
			levels[kept].SysMemPitch		= u32(pitch);
			levels[kept].SysMemSlicePitch	= u32(pitch * rows);
			offset += pitch * rows;
		}
		if (!kept)
			return nullptr;

		D3D11_TEXTURE2D_DESC desc = {};
		desc.Width				= width;
		desc.Height				= height;
		desc.MipLevels			= kept;
		desc.ArraySize			= 1;
		desc.Format				= fmt;
		desc.SampleDesc.Count	= 1;
		desc.Usage				= D3D11_USAGE_IMMUTABLE;
		desc.BindFlags			= D3D11_BIND_SHADER_RESOURCE;
		ID3D11Texture2D* tex = nullptr;
		if (FAILED(HW.pDevice->CreateTexture2D(&desc, levels.data(), &tex)))
			return nullptr;
		return tex;
	}

	// the whole file, or false; the process speaks UTF-8, so the narrow API takes any name
	bool ReadFileBytes(LPCSTR path, xr_vector<u8>& out)
	{
		out.clear();
		HANDLE h = ::CreateFileA(path, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
		if (h == INVALID_HANDLE_VALUE)
			return false;
		LARGE_INTEGER size = {};
		bool ok = ::GetFileSizeEx(h, &size) && size.QuadPart > 0 && size.QuadPart < 0x7FFFFFFF;
		if (ok)
		{
			out.resize(size_t(size.QuadPart));
			DWORD got = 0;
			ok = ::ReadFile(h, out.data(), DWORD(out.size()), &got, nullptr) && got == out.size();
		}
		::CloseHandle(h);
		if (!ok)
			out.clear();
		return ok;
	}

	// Bytes the whole mip chain costs on the GPU. Block formats round every mip
	// up to whole 4x4 blocks, which is why the small mips are not free.
	u32 EstimateBytes(const D3D11_TEXTURE2D_DESC& d)
	{
		const SFormatInfo* fi = FindFormat(d.Format);
		if (!fi)	return 0;

		u32 total = 0;
		for (u32 m = 0; m < d.MipLevels; ++m)
		{
			u32 w = d.Width  >> m;	if (!w) w = 1;
			u32 h = d.Height >> m;	if (!h) h = 1;
			if (fi->compressed)
				total += ((w + 3) / 4) * ((h + 3) / 4) * fi->block;
			else
				total += w * h * fi->block;
		}
		return total * (d.ArraySize ? d.ArraySize : 1);
	}

	xr_string HumanBytes(u32 bytes)
	{
		if (bytes >= 1024*1024)	return NqUtil::Format("%.2f MB", double(bytes)/(1024.0*1024.0));
		if (bytes >= 1024)		return NqUtil::Format("%.1f KB", double(bytes)/1024.0);
		return NqUtil::Format("%u B", bytes);
	}

	// The checkerboard an alpha-blended image is judged against: only the cells of `clip` are drawn, lined up
	// with the image so they move with it - a zoomed-in 4096 texture is millions of cells, a canvas a few hundred.
	void DrawChecker(ImDrawList* dl, const ImVec2& image_min, const ImVec2& a, const ImVec2& b)
	{
		const float cell = XUI::Pt(8.f);
		dl->AddRectFilled(a, b, Tok(ColorToken::Input));
		const int col0 = int(floorf((a.x - image_min.x) / cell)), row0 = int(floorf((a.y - image_min.y) / cell));
		for (int row = row0; image_min.y + row * cell < b.y; ++row)
			for (int col = col0 + ((col0 + row) & 1); image_min.x + col * cell < b.x; col += 2)
			{
				const ImVec2 p(image_min.x + col * cell, image_min.y + row * cell);
				dl->AddRectFilled(ImVec2(_max(p.x, a.x), _max(p.y, a.y)), ImVec2(_min(p.x + cell, b.x), _min(p.y + cell, b.y)),
					Tok(ColorToken::Hover));
			}
	}
}

UIImagePreview::UIImagePreview()
{
	m_Tex			= 0;
	m_View			= 0;
	ZeroMemory		(&m_Desc, sizeof(m_Desc));
	m_Decoded		= false;
	m_Origin		= oNone;
	m_FileSize		= 0;
	m_Mip			= 0;
	m_Zoom			= 0.f;		// fit
	m_Pan			= ImVec2(0.f, 0.f);
	m_Channels		= 0;
	m_ShowDetails	= true;
	m_Scale			= 1.f;
	m_DetailsW		= 0.f;
	m_DrawnFrame	= -2;
}

UIImagePreview::~UIImagePreview()
{
	Release();
	// nothing draws through it any more: the tab is gone before its form is
	Bury(s_AlphaBlend);
	s_AlphaBlend = nullptr;
}

void UIImagePreview::Release()
{
	Bury		(m_View);
	Bury		(m_Tex);
	m_View		= 0;
	m_Tex		= 0;
	ZeroMemory	(&m_Desc, sizeof(m_Desc));
	m_Mip		= 0;
	m_FileSize	= 0;
}

// Takes the decoder's view, keeps the texture behind it, and re-views it per
// mip. See the header for why swapping here is safe against the draw list.
bool UIImagePreview::Adopt(ID3D11ShaderResourceView* srv)
{
	if (!srv)	return false;

	ID3D11Resource* res = 0;
	srv->GetResource(&res);
	srv->Release();
	if (!res)	return false;

	ID3D11Texture2D* t2d = 0;
	res->QueryInterface(__uuidof(ID3D11Texture2D), (void**)&t2d);
	res->Release();
	return AdoptTexture(t2d);
}

bool UIImagePreview::AdoptTexture(ID3D11Texture2D* tex)
{
	if (!tex)	return false;
	m_Tex = tex;
	m_Tex->GetDesc(&m_Desc);
	return ViewMip(0);
}

bool UIImagePreview::ViewMip(u32 mip)
{
	if (!m_Tex || !HW.pDevice)		return false;
	if (mip >= m_Desc.MipLevels)	mip = m_Desc.MipLevels ? m_Desc.MipLevels - 1 : 0;

	D3D11_SHADER_RESOURCE_VIEW_DESC vd = {};
	vd.Format						= m_Desc.Format;
	vd.ViewDimension				= D3D11_SRV_DIMENSION_TEXTURE2D;
	vd.Texture2D.MostDetailedMip	= mip;
	vd.Texture2D.MipLevels			= 1;

	ID3D11ShaderResourceView* v = 0;
	if (FAILED(HW.pDevice->CreateShaderResourceView(m_Tex, &vd, &v)) || !v)	return false;

	Bury(m_View);
	m_View	= v;
	m_Mip	= mip;
	return true;
}

bool UIImagePreview::Show(LPCSTR name, int source, xr_string* err)
{
	if (err) err->clear();
	if (!name || !name[0])	{ if (err) *err = "empty texture name"; return false; }

	if (!Form) Form = xr_new<UIImagePreview>();
	Form->bOpen		= true;

	// already showing this one - just bring the tab forward
	if (Form->m_View && (Form->m_Name == name))
	{
		ShowTab();
		return true;
	}

	Form->Release	();
	Form->m_Name	= name;
	Form->m_File	= "";
	Form->m_Archive	= "";
	Form->m_Origin	= oNone;
	Form->m_Container = "";
	Form->m_Decoded	= false;
	Form->m_Error	= "";
	Form->m_Zoom	= 0.f;
	Form->m_Pan		= ImVec2(0.f, 0.f);
	ShowTab();

	// the file's bytes, wherever it lives; a texture is judged by what the file holds, not by a decoded copy
	xr_vector<u8> bytes;
	if (2 == source)
	{
		// the game install: a loose file under its gamedata, or one inside an archive
		xr_string mount_err;
		const int idx = EditorGameContent::EnsureMounted(mount_err) ? EditorGameContent::Find(name) : -1;
		u32 sz = 0;
		u8* data = (idx >= 0) ? EditorGameContent::ReadBytes(idx, sz) : 0;
		if (data && sz)
		{
			bytes.assign(data, data + sz);
			const EditorGameContent::SItem* item = EditorGameContent::Get(idx);
			if (item && item->source == EditorGameContent::srcLoose)
			{
				Form->m_File	= NqUtil::Format("%s\\gamedata\\%s", EditorGameContent::MountedRoot(), item->path.c_str());
				Form->m_Origin	= oGameFolder;
			}
			else
			{
				Form->m_Archive	= item ? item->path : xr_string(name);
				Form->m_Origin	= oGameArchive;
			}
		}
		EditorGameContent::FreeBytes(data);
	}
	else if (0 == source)
	{
		string_path abs;
		xr_sprintf	(abs, sizeof(abs), "%s\\%s", EditorProject::Root(), name);
		if (ReadFileBytes(abs, bytes))
		{
			Form->m_File	= abs;
			Form->m_Origin	= oProject;
		}
	}
	else
	{
		// the SDK library: a name without an extension, resolved through fs.ltx
		string_path stem;
		xr_strcpy	(stem, sizeof(stem), name);
		if (char* dot = strrchr(stem, '.'))
			if (!strchr(dot, '\\')) *dot = 0;

		for (int i = 0; i < int(sizeof(s_LibProbes)/sizeof(s_LibProbes[0])) && bytes.empty(); ++i)
		{
			string_path rel, full;
			xr_sprintf	(rel, sizeof(rel), "%s%s", stem, s_LibProbes[i].ext);
			FS.update_path(full, s_LibProbes[i].alias, rel);
			if (INVALID_FILE_ATTRIBUTES != ::GetFileAttributesA(full) && ReadFileBytes(full, bytes))
			{
				Form->m_File	= full;
				Form->m_Origin	= oLibrary;
			}
		}
	}

	bool shown = false;
	if (!bytes.empty())
	{
		const u32 size		= u32(bytes.size());
		Form->m_FileSize	= size;
		Form->m_Container	= ContainerOf(bytes.data(), size, Form->m_File.empty() ? name : Form->m_File.c_str());
		shown = Form->AdoptTexture(NativeDds(bytes.data(), size));
		if (!shown)
		{
			Form->Release();
			Form->m_FileSize	= size;
			Form->m_Decoded		= true;
			shown = Form->Adopt(DX11TextureFromMemory(bytes.data(), size));
		}
	}
	if (!shown)
	{
		Form->m_Error = "The file cannot be decoded as an image.";
		if (err) *err = "cannot decode this file as an image";
		return false;
	}
	return true;
}

void UIImagePreview::Close()
{
	xr_delete(Form);
}

void UIImagePreview::Update()
{
	FreeDead();
}

void UIImagePreview::McpState(xr_string& out)
{
	out += "\"open\":";
	out += Form ? "true" : "false";
	if (!Form)
		return;
	out += ",\"name\":";
	NqUtil::JsonString(out, Form->m_Name);
	out += ",\"file\":";
	NqUtil::JsonPath(out, Form->m_File.c_str());
	out += ",\"archive\":";
	NqUtil::JsonPath(out, Form->m_Archive.c_str());
	// where the file lives: project | library (the editor's) | game_folder (loose under the game's gamedata) | game_archive
	static const char* const kOrigins[] = { "", "project", "library", "game_folder", "game_archive" };
	out += NqUtil::Format(",\"origin\":\"%s\"", kOrigins[Form->m_Origin]);
	out += ",\"error\":";
	NqUtil::JsonString(out, Form->m_Error);
	const SFormatInfo* fi = Form->m_Tex ? FindFormat(Form->m_Desc.Format) : nullptr;
	// the format the GPU holds; `decoded` = the file could not be taken as it is, so it is B8G8R8A8 of mip 0
	out += ",\"format\":";
	NqUtil::JsonString(out, fi ? fi->name : Form->m_Tex ? "unknown" : "");
	out += ",\"container\":";
	NqUtil::JsonString(out, Form->m_Container);
	out += ",\"decoded\":";
	out += Form->m_Decoded ? "true" : "false";
	// zoom is the factor in effect - with fit, the one the fit came to in the last frame
	const bool fit = Form->m_Zoom <= 0.f;
	out += NqUtil::Format(",\"width\":%u,\"height\":%u,\"mips\":%u,\"mip\":%u,\"zoom\":%.3f,\"fit\":%s,\"scale\":%.3f,\"channel\":\"%s\","
		"\"alpha\":%s,\"has_alpha\":%s,\"bytes\":%u,\"panes\":{\"details\":%s,\"details_width\":%.0f}",
		Form->m_Desc.Width, Form->m_Desc.Height, Form->m_Desc.MipLevels, Form->m_Mip, fit ? Form->m_Scale : Form->m_Zoom, fit ? "true" : "false",
		Form->m_Scale, kChannelIds[clampr(Form->m_Channels, 0, 4)], Form->m_Channels == 0 ? "true" : "false", fi && fi->alpha ? "true" : "false",
		Form->m_FileSize, Form->m_ShowDetails ? "true" : "false", Form->m_ShowDetails ? Form->m_DetailsW : 0.f);
}

bool UIImagePreview::DrawnLately()
{
	// the frame being built or the one before it
	return Form && ImGui::GetCurrentContext() && Form->m_DrawnFrame >= ImGui::GetFrameCount() - 1;
}

bool UIImagePreview::McpView(float zoom, int mip, int channel, int details, xr_string& err)
{
	if (!Form)
	{
		err = "no texture is open";
		return false;
	}
	if (zoom >= 0.f)
	{
		Form->m_Zoom = zoom > 0.f ? clampr(zoom, kZoomMin, kZoomMax) : 0.f;
		Form->m_Pan = ImVec2(0.f, 0.f);
	}
	if (channel >= 0)
		Form->m_Channels = clampr(channel, 0, 4);
	if (details >= 0)
		Form->m_ShowDetails = details > 0;
	if (mip >= 0 && Form->m_Tex && !Form->ViewMip(u32(mip)))
	{
		err = "that mip cannot be shown";
		return false;
	}
	return true;
}

void UIImagePreview::ShowTab()
{
	if (XUI::Shell::IsOpen("texture"))
	{
		XUI::Shell::Activate("texture");
		return;
	}
	XUI::Shell::DocDesc d;
	d.id = "texture";
	d.kind = "texture";
	d.icon = XUI::Icon::FileTexture;
	d.padded = false;
	d.label = []
	{
		if (!Form || Form->m_Name.empty()) return xr_string("Texture");
		xr_string name = Form->m_Name;
		const size_t slash = name.find_last_of("\\/");
		if (slash != xr_string::npos) name = name.substr(slash + 1);
		const size_t dot = name.rfind('.');
		if (dot != xr_string::npos && dot) name.resize(dot);
		return name;
	};
	d.tooltip = [] { return Form && !Form->m_Name.empty() ? Form->m_Name : xr_string("Texture"); };
	d.alive = [] { return Form && !Form->IsClosed(); };
	d.toolbar = [](XUI::Toolbar& tb) { if (Form) Form->DrawToolBar(tb); };
	d.body = [] { if (Form) Form->Draw(); };
	d.status = [] { if (Form) Form->DrawStatus(); };
	// F fits the image into the canvas
	d.shortcut = [](int key, int mods)
	{
		if (!Form || key != ImGuiKey_F || mods) return false;
		Form->m_Zoom = 0.f;
		Form->m_Pan = ImVec2(0.f, 0.f);
		return true;
	};
	d.on_close = [] { Close(); };
	XUI::Shell::Open(d, true);
}

void UIImagePreview::DrawToolBar(XUI::Toolbar& tb)
{
	const bool image = m_View != nullptr;
	const char* none = "No texture is open";
	tb.Group(3);
	if (tb.Action("fit", XUI::Icon::ZoomFit, "Fit", image ? "Fit the image into the canvas (F)" : none, image, m_Zoom <= 0.f))
	{
		m_Zoom = 0.f;
		m_Pan = ImVec2(0.f, 0.f);
	}
	// the icon already reads "1:1": the label says what it does
	if (tb.Action("one_to_one", XUI::Icon::Zoom100, "Actual Size", image ? "One screen pixel per texel (1:1)" : none, image, m_Zoom == 1.f))
	{
		m_Zoom = 1.f;
		m_Pan = ImVec2(0.f, 0.f);
	}
	if (tb.Menu("zoom", XUI::Icon::ZoomIn, "Zoom", "Pick a zoom; the wheel zooms at the mouse, any button drags"))
	{
		static const float kSteps[] = { .125f, .25f, .5f, 1.f, 2.f, 4.f, 8.f, 16.f };
		for (float step : kSteps)
		{
			const xr_string label = NqUtil::Format("%g%%", step * 100.f);
			if (ImGui::MenuItem(label.c_str(), nullptr, m_Zoom == step, image))
			{
				m_Zoom = step;
				m_Pan = ImVec2(0.f, 0.f);
			}
		}
		ImGui::EndPopup();
	}

	// in the overflow the channels become radio items
	tb.Group(5);
	tb.Segmented("channels", "Channels", &m_Channels, kChannels, 5,
		"RGBA shows the image over a checkerboard; R, G and B one colour channel; A the alpha as grey", image);

	tb.Group(2);
	if (tb.Menu("mip", XUI::Icon::Picture, "Mip", "Which mip level the canvas shows"))
	{
		const u32 mips = image ? _max(1u, m_Desc.MipLevels) : 0;
		for (u32 m = 0; m < mips; ++m)
		{
			const u32 w = _max(1u, m_Desc.Width >> m), h = _max(1u, m_Desc.Height >> m);
			const xr_string label = NqUtil::Format("Mip %u \xC2\xB7 %u x %u", m, w, h);
			if (ImGui::MenuItem(label.c_str(), nullptr, m == m_Mip))
				ViewMip(m);
		}
		if (!mips)
			ImGui::MenuItem(none, nullptr, false, false);
		ImGui::EndPopup();
	}

	tb.Group(4);
	if (tb.Action("details", XUI::Icon::SidebarRight, "Details", "Show the texture's facts beside the canvas", true, m_ShowDetails))
		m_ShowDetails = !m_ShowDetails;
}

void UIImagePreview::DrawStatus()
{
	if (!m_View)
		return;
	const xr_string text = NqUtil::Format("%u x %u \xC2\xB7 %s \xC2\xB7 mip %u of %u \xC2\xB7 %.0f%%", MipW(), MipH(),
		FormatText().c_str(), m_Mip, m_Desc.MipLevels, m_Scale * 100.f);
	ImGui::TextUnformatted(text.c_str());
}

// the file's pixel format as the GPU holds it; a file it cannot take as it is says what it was decoded from
xr_string UIImagePreview::FormatText() const
{
	const SFormatInfo* fi = FindFormat(m_Desc.Format);
	const xr_string gpu = fi ? xr_string(fi->name) : NqUtil::Format("DXGI format %u", u32(m_Desc.Format));
	return m_Decoded ? NqUtil::Format("%s, decoded to %s", m_Container.empty() ? "image" : m_Container.c_str(), gpu.c_str()) : gpu;
}

void UIImagePreview::DrawDetails()
{
	if (!m_Tex)
	{
		XUI::EmptyState(XUI::Icon::Info, "No facts", "Open a texture to see what it is made of.");
		return;
	}
	const SFormatInfo* fi = FindFormat(m_Desc.Format);
	if (!XUI::BeginPropertyGrid("texture"))
		return;
	if (XUI::PropertySection("Texture"))
	{
		xr_string leaf = m_Name;
		const size_t slash = leaf.find_last_of("\\/");
		if (slash != xr_string::npos) leaf = leaf.substr(slash + 1);
		if (XUI::PropertyRow("Name", "The texture's file name", XUI::PropReadOnly))
			XUI::PropertyText(leaf.c_str());
		if (XUI::PropertyRow("Size", "Its size as imported, in pixels", XUI::PropReadOnly))
			XUI::PropertyText(NqUtil::Format("%u x %u", m_Desc.Width, m_Desc.Height).c_str());
		if (XUI::PropertyRow("Format", m_Decoded ? "The GPU cannot take this file as it is: the preview shows its top level decoded"
				: "The file's pixel format, as the GPU holds it", XUI::PropReadOnly))
			XUI::PropertyText(FormatText().c_str());
		// what a decoded copy has is not what the file has: those rows would describe the preview
		if (!m_Decoded)
		{
			if (XUI::PropertyRow("Alpha", "Whether the format keeps an alpha channel", XUI::PropReadOnly))
				XUI::PropertyText(fi ? (fi->alpha ? "Yes" : "No") : "Unknown");
			if (XUI::PropertyRow("sRGB", "Whether the colours are read as sRGB", XUI::PropReadOnly))
				XUI::PropertyText(fi ? (fi->srgb ? "Yes" : "No") : "Unknown");
			if (XUI::PropertyRow("Mips", "The mip levels the file holds", XUI::PropReadOnly))
				XUI::PropertyText(NqUtil::Format("%u", m_Desc.MipLevels).c_str());
			if (m_Desc.ArraySize > 1 && XUI::PropertyRow("Slices", "Array slices", XUI::PropReadOnly))
				XUI::PropertyText(NqUtil::Format("%u", m_Desc.ArraySize).c_str());
			if (XUI::PropertyRow("GPU Memory", "What the whole mip chain takes on the GPU", XUI::PropReadOnly))
				XUI::PropertyText(HumanBytes(EstimateBytes(m_Desc)).c_str());
		}
		if (m_FileSize && XUI::PropertyRow("On Disk", "The size of the file", XUI::PropReadOnly))
			XUI::PropertyText(HumanBytes(m_FileSize).c_str());
		// where it lives and its path are two rows: an ellipsis at the start of one line ate the origin's label
		static const char* const kOrigin[] = { "", "Project", "Editor library", "Game folder", "Game archive" };
		static const char* const kOriginHint[] = { "",
			"A file of the project",
			"A file of the editor's own library",
			"A loose file under the linked game's gamedata folder",
			"The linked game keeps this texture packed in its database archives" };
		if (m_Origin != oNone && XUI::PropertyRow("Source", kOriginHint[m_Origin], XUI::PropReadOnly))
			XUI::PropertyText(kOrigin[m_Origin]);
		const xr_string& path = m_File.empty() ? m_Archive : m_File;
		if (!path.empty() && XUI::PropertyRow("Path", m_File.empty() ? "Its path below the game's gamedata, inside the archives"
				: "The file the texture was read from", XUI::PropReadOnly))
		{
			const float w = ImGui::CalcItemWidth();
			ImGui::AlignTextToFramePadding();
			ImGui::PushStyleColor(ImGuiCol_Text, Tok(ColorToken::TextSecondary));
			XUI::TextEllipsis(path.c_str(), w, XUI::Clip::Start);
			ImGui::PopStyleColor();
			if (!m_File.empty() && XUI::PropertyTrailing("##reveal", XUI::Icon::Reveal, "Show in Explorer"))
				::ShellExecuteA(0, "open", "explorer.exe", ("/select,\"" + m_File + "\"").c_str(), 0, SW_SHOWNORMAL);
		}
		XUI::EndPropertySection();
	}
	if (XUI::PropertySection("View"))
	{
		if (XUI::PropertyRow("Shown", "The mip level the canvas shows and its size", XUI::PropReadOnly))
			XUI::PropertyText(NqUtil::Format("Mip %u \xC2\xB7 %u x %u", m_Mip, MipW(), MipH()).c_str());
		if (XUI::PropertyRow("Zoom", "Screen pixels per texel", XUI::PropReadOnly))
			XUI::PropertyText(m_Zoom > 0.f ? NqUtil::Format("%.0f%%", m_Scale * 100.f).c_str() : NqUtil::Format("Fit (%.0f%%)", m_Scale * 100.f).c_str());
		if (XUI::PropertyRow("Channels", "What the canvas shows of the colour", XUI::PropReadOnly))
			XUI::PropertyText(kChannels[clampr(m_Channels, 0, 4)]);
		XUI::EndPropertySection();
	}
	XUI::EndPropertyGrid();
}

// The canvas: fit keeps the aspect and never blows a small texture up past 1:1; the wheel zooms about the
// point under the mouse, any button drags, a double-click fits again.
void UIImagePreview::DrawImage(const ImVec2& p0, const ImVec2& avail)
{
	ImGui::SetCursorScreenPos(p0);
	ImGui::InvisibleButton("##image_canvas", ImVec2(_max(avail.x, 1.f), _max(avail.y, 1.f)),
		ImGuiButtonFlags_MouseButtonLeft | ImGuiButtonFlags_MouseButtonRight | ImGuiButtonFlags_MouseButtonMiddle);
	const bool hovered = ImGui::IsItemHovered();
	const bool active = ImGui::IsItemActive();
	const ImGuiIO& io = ImGui::GetIO();

	const float w = float(MipW()), h = float(MipH());
	const float fit = _min(1.f, _min(avail.x / w, avail.y / h));
	float scale = m_Zoom > 0.f ? m_Zoom : fit;
	const auto centred = [&](float s) { return ImVec2(p0.x + (avail.x - w * s) * .5f, p0.y + (avail.y - h * s) * .5f); };
	ImVec2 origin(centred(scale).x + (m_Zoom > 0.f ? m_Pan.x : 0.f), centred(scale).y + (m_Zoom > 0.f ? m_Pan.y : 0.f));

	if (hovered && io.MouseWheel != 0.f)
	{
		const float next = clampr(scale * (io.MouseWheel > 0.f ? 1.25f : 1.f / 1.25f), kZoomMin, kZoomMax);
		const ImVec2 texel((io.MousePos.x - origin.x) / scale, (io.MousePos.y - origin.y) / scale);
		m_Zoom = next;
		m_Pan = ImVec2(io.MousePos.x - texel.x * next - centred(next).x, io.MousePos.y - texel.y * next - centred(next).y);
		scale = next;
	}
	if (active && (ImGui::IsMouseDragging(ImGuiMouseButton_Left, 0.f) || ImGui::IsMouseDragging(ImGuiMouseButton_Right, 0.f) ||
		ImGui::IsMouseDragging(ImGuiMouseButton_Middle, 0.f)))
	{
		if (m_Zoom <= 0.f)
		{
			m_Zoom = scale;
			m_Pan = ImVec2(0.f, 0.f);
		}
		m_Pan.x += io.MouseDelta.x;
		m_Pan.y += io.MouseDelta.y;
		ImGui::SetMouseCursor(ImGuiMouseCursor_ResizeAll);
	}
	if (hovered && ImGui::IsMouseDoubleClicked(ImGuiMouseButton_Left))
	{
		m_Zoom = 0.f;
		scale = fit;
	}
	if (m_Zoom > 0.f)
	{
		// some of the image always stays in view
		const float keep = XUI::Pt(64.f);
		const float mx = _max(0.f, (avail.x + w * scale) * .5f - keep), my = _max(0.f, (avail.y + h * scale) * .5f - keep);
		m_Pan.x = clampr(m_Pan.x, -mx, mx);
		m_Pan.y = clampr(m_Pan.y, -my, my);
	}
	else
		m_Pan = ImVec2(0.f, 0.f);
	m_Scale = scale;
	origin = ImVec2(floorf(centred(scale).x + m_Pan.x), floorf(centred(scale).y + m_Pan.y));

	const ImVec2 a = origin, b(origin.x + w * scale, origin.y + h * scale);
	const ImVec2 c0 = p0, c1(p0.x + avail.x, p0.y + avail.y);
	const ImVec2 va(_max(a.x, c0.x), _max(a.y, c0.y)), vb(_min(b.x, c1.x), _min(b.y, c1.y));
	if (va.x >= vb.x || va.y >= vb.y)
		return;

	ImDrawList* dl = ImGui::GetWindowDrawList();
	const ImGuiPlatformIO& pio = ImGui::GetPlatformIO();
	dl->PushClipRect(c0, c1, true);
	// the channel views: RGBA over the checkerboard, one colour over black, the alpha as grey on white
	// channel masks, not colours: the tint multiplies the texel, so each keeps one channel of it
	static const ImU32 kTint[4] = { IM_COL32_WHITE, IM_COL32_A_MASK | (0xFFu << IM_COL32_R_SHIFT), IM_COL32_A_MASK | (0xFFu << IM_COL32_G_SHIFT),	// ui-units: ok a texture channel view, not a UI colour
		IM_COL32_A_MASK | (0xFFu << IM_COL32_B_SHIFT) };
	const int channel = clampr(m_Channels, 0, 4);
	if (channel == 0)
		DrawChecker(dl, a, va, vb);
	else
		dl->AddRectFilled(va, vb, channel == 4 ? IM_COL32_WHITE : IM_COL32_BLACK);	// ui-units: ok a texture channel view, not a UI colour
	// past 2:1 every texel is a block of pixels: sampled nearest, so it reads as one
	const bool nearest = scale >= 2.f && pio.DrawCallback_SetSamplerNearest;
	if (nearest)
		dl->AddCallback(pio.DrawCallback_SetSamplerNearest, nullptr);
	if (channel == 4)
		dl->AddCallback(&SetAlphaBlend, nullptr);
	dl->AddImage(m_View, a, b, ImVec2(0, 0), ImVec2(1, 1), channel == 4 ? IM_COL32_WHITE : kTint[channel]);
	if ((nearest || channel == 4) && pio.DrawCallback_ResetRenderState)
		dl->AddCallback(pio.DrawCallback_ResetRenderState, nullptr);
	dl->AddRect(a, b, Tok(ColorToken::BorderSubtle));
	dl->PopClipRect();
}

// the texture tab's work area: canvas | Details; Details keeps its pt size, the canvas takes every resize
void UIImagePreview::Draw()
{
	m_DrawnFrame = ImGui::GetFrameCount();
	const ImGuiWindowFlags still = ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse;
	float details_w = kDetailsW;
	if (m_ShowDetails)
		XUI::SplitterTail("texture.details", true, &details_w, kCanvasMin, kSideMin, kDetailsW);
	m_DetailsW = m_ShowDetails ? details_w : 0.f;
	const float canvas_w = m_ShowDetails ? _max(1.f, ImGui::GetContentRegionAvail().x - XUI::Pt(details_w) - ImGui::GetStyle().ItemSpacing.x) : 0.f;
	ImGui::PushStyleColor(ImGuiCol_ChildBg, Tok(ColorToken::Canvas));
	const bool canvas = ImGui::BeginChild("##texture_canvas", ImVec2(canvas_w, 0.f), ImGuiChildFlags_None, still);
	ImGui::PopStyleColor();
	if (canvas)
	{
		if (!m_Error.empty())
			XUI::EmptyState(XUI::Icon::Warning, "This texture cannot be shown", (m_Name + ": " + m_Error).c_str());
		else if (!m_View)
			XUI::EmptyState(XUI::Icon::FileTexture, "No texture", "Double-click an image in the Content Browser to see it here.");
		else
			DrawImage(ImGui::GetCursorScreenPos(), ImGui::GetContentRegionAvail());
	}
	ImGui::EndChild();
	if (m_ShowDetails)
	{
		ImGui::SameLine();
		if (ImGui::BeginChild("##texture_details", ImVec2(0.f, 0.f), ImGuiChildFlags_AlwaysUseWindowPadding))
			DrawDetails();
		ImGui::EndChild();
	}
}
