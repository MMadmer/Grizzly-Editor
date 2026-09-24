#include "stdafx.h"
#include "EditorPda.h"
#include "EditorPdaInternal.h"
#include "EditorProject.h"
#include "EditorGameContent.h"
#include "EditorModManifest.h"
#include "EditorFileOps.h"
#include "EDX11Utils.h"
#include "XFinedMCP.h"
#include "Nq/NqUtil.h"
#include "Nq/NqLua.h"
#include "ui_main.h"

//------------------------------------------------------------------------------
// The preview tab: the 2D PDA with a module's layout in it, drawn from the flat
// item list the game's reader produced, with the game's own textures and bitmap
// fonts, and playing the actions the way CUIPdaLayout plays them in the game.
// A stock section or a script action cannot run outside the game: the first is
// shown as a labelled panel, the second goes into the action log.
//------------------------------------------------------------------------------
namespace
{
	// ============================================================== game files
	bool ReadGameFile(LPCSTR rel, xr_string& out)
	{
		xr_string err;
		if (!EditorProject::GameLinked() || !EditorGameContent::EnsureMounted(err)) return false;
		xr_string key = rel;
		xr_strlwr(&key[0]);
		const int idx = EditorGameContent::Find(key.c_str());
		u32 size = 0;
		u8* bytes = idx >= 0 ? EditorGameContent::ReadBytes(idx, size) : 0;
		if (!bytes) return false;
		out.assign((const char*)bytes, (const char*)bytes + size);
		EditorGameContent::FreeBytes(bytes);
		return true;
	}

	bool ReadProjectFile(LPCSTR rel, xr_string& out)
	{
		if (!EditorProject::Active()) return false;
		string_path abs;
		sprintf_s(abs, "%s\\gamedata\\%s", EditorProject::Root(), rel);
		xr_string err;
		return NqUtil::FileExists(abs) && NqLua::ReadFile(abs, out, err);
	}

	xr_string Lower(const xr_string& s)
	{
		xr_string r = s;
		for (size_t i = 0; i < r.size(); ++i)
			if (r[i] >= 'A' && r[i] <= 'Z') r[i] = char(r[i] - 'A' + 'a');
		return r;
	}

	xr_string TagAttr(const xr_string& tag, LPCSTR name)
	{
		const xr_string key = xr_string(name) + "=\"";
		size_t p = 0;
		while ((p = tag.find(key, p)) != xr_string::npos)
		{
			// a whole attribute name, not the tail of another one
			if (p == 0 || tag[p - 1] == ' ' || tag[p - 1] == '\t' || tag[p - 1] == '\n' || tag[p - 1] == '\r')
			{
				const size_t v = p + key.size();
				const size_t e = tag.find('"', v);
				return e == xr_string::npos ? xr_string() : tag.substr(v, e - v);
			}
			p += key.size();
		}
		return xr_string();
	}

	xr_string StripComments(const xr_string& raw)
	{
		xr_string text;
		for (size_t pos = 0; pos < raw.size();)
		{
			const size_t c = raw.find("<!--", pos);
			if (c == xr_string::npos) { text += raw.substr(pos); break; }
			text += raw.substr(pos, c - pos);
			const size_t ce = raw.find("-->", c + 4);
			pos = (ce == xr_string::npos) ? raw.size() : ce + 3;
		}
		return text;
	}

	// ============================================================== textures
	enum ESheetFrom { sfGame, sfProject, sfBundled };

	struct STexId
	{
		xr_string	sheet;		// textures-relative, no extension
		ESheetFrom	from;
		float		x, y, w, h;
	};

	struct SSheet
	{
		ID3D11ShaderResourceView*	view;
		u32							w, h;
		bool						tried;
		SSheet() : view(0), w(0), h(0), tried(false) {}
	};

	xr_map<xr_string, STexId>	s_TexIds;
	xr_map<xr_string, SSheet>	s_Sheets;
	xr_string					s_TexKey;

	xr_string LinkKey()
	{
		xr_string k = EditorProject::Active() ? EditorProject::Root() : "";
		k += "|";
		k += EditorProject::GameLinked() ? EditorProject::GameRoot() : "";
		return k;
	}

	void DropTextures()
	{
		for (xr_map<xr_string, SSheet>::iterator it = s_Sheets.begin(); it != s_Sheets.end(); ++it)
			if (it->second.view) it->second.view->Release();
		s_Sheets.clear();
		s_TexIds.clear();
		s_TexKey.clear();
	}

	void ParseDescr(const xr_string& raw, ESheetFrom from)
	{
		const xr_string text = StripComments(raw);
		xr_string sheet;
		for (size_t pos = 0;;)
		{
			const size_t f = text.find("<file", pos), t = text.find("<texture", pos);
			if (f == xr_string::npos && t == xr_string::npos) break;
			const size_t at = (f != xr_string::npos && (t == xr_string::npos || f < t)) ? f : t;
			const size_t gt = text.find('>', at);
			if (gt == xr_string::npos) break;
			const xr_string tag = text.substr(at, gt - at);
			pos = gt + 1;
			if (at == f) { sheet = TagAttr(tag, "name"); continue; }
			STexId e;
			e.sheet = sheet;
			e.from = from;
			e.x = (float)atof(TagAttr(tag, "x").c_str());
			e.y = (float)atof(TagAttr(tag, "y").c_str());
			e.w = (float)atof(TagAttr(tag, "width").c_str());
			e.h = (float)atof(TagAttr(tag, "height").c_str());
			const xr_string id = Lower(TagAttr(tag, "id"));
			// later descriptions win, the way UICore::ReadTextureInfo lets a mod redefine an id
			if (!id.empty() && !sheet.empty() && e.w > 0 && e.h > 0) s_TexIds[id] = e;
		}
	}

	void LoadTexIds()
	{
		const xr_string key = LinkKey();
		if (key == s_TexKey) return;
		DropTextures();
		s_TexKey = key;
		const LPCSTR prefix = "configs\\ui\\textures_descr\\";
		const size_t plen = xr_strlen(prefix);
		xr_string err;
		if (EditorProject::GameLinked() && EditorGameContent::EnsureMounted(err))
		{
			const int total = EditorGameContent::Count();
			for (int i = 0; i < total; ++i)
			{
				const EditorGameContent::SItem* it = EditorGameContent::Get(i);
				if (!it || 0 != _strnicmp(it->path.c_str(), prefix, plen)) continue;
				if (it->path.size() < 4 || 0 != _stricmp(it->path.c_str() + it->path.size() - 4, ".xml")) continue;
				u32 size = 0;
				u8* bytes = EditorGameContent::ReadBytes(i, size);
				if (!bytes) continue;
				xr_string raw;
				raw.assign((const char*)bytes, (const char*)bytes + size);
				EditorGameContent::FreeBytes(bytes);
				ParseDescr(raw, sfGame);
			}
		}
		// a game without Refined's PDA support still previews with the editor's copy of the sheet
		if (s_TexIds.find("ui_xms_pda_white") == s_TexIds.end())
		{
			string_path path;
			sprintf_s(path, "%s\\gamedata\\configs\\pda\\ui_xms_pda.xml", NqUtil::SdkRoot());
			xr_string raw;
			if (NqLua::ReadFile(path, raw, err)) ParseDescr(raw, sfBundled);
		}
		if (EditorProject::Active())
		{
			string_path dir;
			sprintf_s(dir, "%s\\gamedata\\configs\\ui\\textures_descr", EditorProject::Root());
			xr_vector<xr_string> files;
			if (NqUtil::DirExists(dir)) NqUtil::ListFiles(dir, ".xml", files);
			std::sort(files.begin(), files.end());
			for (u32 i = 0; i < files.size(); ++i)
			{
				xr_string raw;
				if (NqLua::ReadFile(files[i].c_str(), raw, err)) ParseDescr(raw, sfProject);
			}
		}
	}

	// textures\<rel>.dds from where the description came from (a project sheet may also
	// live in the game, and a game description may name a sheet the project overrides)
	SSheet* Sheet(const xr_string& rel_in, ESheetFrom from)
	{
		xr_string rel = Lower(rel_in);
		for (size_t i = 0; i < rel.size(); ++i) if (rel[i] == '/') rel[i] = '\\';
		if (rel.size() > 4 && rel.compare(rel.size() - 4, 4, ".dds") == 0) rel.erase(rel.size() - 4);
		const xr_string key = rel;
		SSheet& s = s_Sheets[key];
		if (s.tried) return s.view ? &s : 0;
		s.tried = true;
		string_path abs;
		if (EditorProject::Active())
		{
			sprintf_s(abs, "%s\\gamedata\\textures\\%s.dds", EditorProject::Root(), rel.c_str());
			if (NqUtil::FileExists(abs)) s.view = DX11TextureFromFile(abs);
		}
		if (!s.view)
		{
			xr_string bytes;
			if (ReadGameFile(("textures\\" + rel + ".dds").c_str(), bytes))
				s.view = DX11TextureFromMemory(bytes.data(), (u32)bytes.size());
		}
		if (!s.view && from == sfBundled)
		{
			sprintf_s(abs, "%s\\gamedata\\configs\\pda\\%s.dds", NqUtil::SdkRoot(), NqUtil::BaseName(rel.c_str()).c_str());
			if (NqUtil::FileExists(abs)) s.view = DX11TextureFromFile(abs);
		}
		if (s.view) DX11TextureSize(s.view, s.w, s.h);
		if (!s.w || !s.h) { if (s.view) s.view->Release(); s.view = 0; }
		return s.view ? &s : 0;
	}

	bool Tex(const xr_string& id, ImTextureID& tex, ImVec2& uv0, ImVec2& uv1)
	{
		if (id.empty()) return false;
		LoadTexIds();
		xr_map<xr_string, STexId>::const_iterator it = s_TexIds.find(Lower(id));
		if (it != s_TexIds.end())
		{
			SSheet* s = Sheet(it->second.sheet, it->second.from);
			if (!s) return false;
			tex = (ImTextureID)s->view;
			uv0 = ImVec2(it->second.x / s->w, it->second.y / s->h);
			uv1 = ImVec2((it->second.x + it->second.w) / s->w, (it->second.y + it->second.h) / s->h);
			return true;
		}
		// not an id: a texture file, the way CUIStatic::InitTexture falls back to one
		SSheet* s = Sheet(id, sfGame);
		if (!s) return false;
		tex = (ImTextureID)s->view;
		uv0 = ImVec2(0, 0);
		uv1 = ImVec2(1, 1);
		return true;
	}

	// ============================================================== fonts
	struct SFont
	{
		SSheet*	sheet;
		float	height;			// glyph height in texture pixels = virtual units at 768
		float	gx[256], gy[256], gw[256];
		bool	tried, ok;
		SFont() : sheet(0), height(16), tried(false), ok(false) {}
	};

	xr_map<xr_string, SFont>	s_Fonts;
	xr_string					s_FontKey;

	LPCSTR FontSection(const xr_string& name)
	{
		if (name == "letterica16") return "ui_font_letterica16_russian";
		if (name == "letterica25") return "ui_font_letter_25";
		if (name == "graffiti19") return "ui_font_graffiti19_russian";
		if (name == "graffiti22") return "ui_font_graffiti22_russian";
		if (name == "graffiti32") return "ui_font_graff_32";
		if (name == "graffiti50") return "ui_font_graff_50";
		if (name == "arial_14") return "ui_font_arial_14";
		if (name == "medium") return "hud_font_medium";
		if (name == "small") return "stat_font";
		if (name == "di") return "hud_font_di";
		return "ui_font_letterica18_russian";
	}

	// [section] key = value of a plain ltx text; `: parent` inheritance followed once
	bool LtxValue(const xr_string& text, LPCSTR section, LPCSTR key, xr_string& out, int depth = 0)
	{
		const xr_string head = xr_string("[") + section + "]";
		size_t p = 0;
		while ((p = text.find(head, p)) != xr_string::npos)
		{
			if (p > 0 && text[p - 1] != '\n' && text[p - 1] != '\r' && text[p - 1] != ' ' && text[p - 1] != '\t') { ++p; continue; }
			size_t line_end = text.find('\n', p);
			const xr_string header = text.substr(p, line_end == xr_string::npos ? xr_string::npos : line_end - p);
			size_t pos = line_end == xr_string::npos ? text.size() : line_end + 1;
			while (pos < text.size())
			{
				size_t e = text.find('\n', pos);
				if (e == xr_string::npos) e = text.size();
				xr_string line = text.substr(pos, e - pos);
				pos = e + 1;
				const size_t sc = line.find(';');
				if (sc != xr_string::npos) line.erase(sc);
				line = NqUtil::Trim(line);
				if (line.empty()) continue;
				if (line[0] == '[') break;
				const size_t eq = line.find('=');
				if (eq == xr_string::npos) continue;
				if (NqUtil::Trim(line.substr(0, eq)) == key) { out = NqUtil::Trim(line.substr(eq + 1)); return true; }
			}
			// [child]:parent
			const size_t colon = header.find("]:");
			if (colon != xr_string::npos && depth < 4)
			{
				const xr_string parent = NqUtil::Trim(header.substr(colon + 2));
				if (!parent.empty()) return LtxValue(text, parent.c_str(), key, out, depth + 1);
			}
			return false;
		}
		return false;
	}

	const SFont* FontOf(const xr_string& name)
	{
		const xr_string key = LinkKey();
		if (key != s_FontKey) { s_Fonts.clear(); s_FontKey = key; }
		SFont& f = s_Fonts[name.empty() ? "letterica18" : name];
		if (f.tried) return f.ok ? &f : 0;
		f.tried = true;
		xr_string fonts, tex;
		if (!ReadGameFile("configs\\fonts.ltx", fonts)) return 0;
		if (!LtxValue(fonts, FontSection(name), "texture", tex)) return 0;
		tex = Lower(NqUtil::Trim(tex));
		xr_string ini;
		if (!ReadGameFile(("textures\\" + tex + ".ini").c_str(), ini)) return 0;
		xr_string h;
		if (!LtxValue(ini, "symbol_coords", "height", h)) return 0;
		f.height = (float)atof(h.c_str());
		for (int i = 0; i < 256; ++i)
		{
			char k[8];
			sprintf_s(k, "%03d", i);
			xr_string v;
			f.gx[i] = f.gy[i] = f.gw[i] = 0;
			if (!LtxValue(ini, "symbol_coords", k, v)) continue;
			float x1 = 0, y1 = 0, x2 = 0;
			if (3 == sscanf_s(v.c_str(), "%f , %f , %f", &x1, &y1, &x2) ||
				3 == sscanf_s(v.c_str(), "%f,%f,%f", &x1, &y1, &x2))
			{
				f.gx[i] = x1;
				f.gy[i] = y1;
				f.gw[i] = x2 - x1;
			}
		}
		f.sheet = Sheet(tex, sfGame);
		f.ok = f.sheet != 0 && f.height > 0;
		return f.ok ? &f : 0;
	}

	// ============================================================== the device
	struct SDevice
	{
		bool		loaded;
		Frect		bg[2], noise[2], close[2];
		xr_string	bg_tex[2], noise_tex[2], close_tex[2];
		u32			noise_alpha[2];
		SDevice() : loaded(false) {}
	};
	SDevice		s_Device;
	xr_string	s_DeviceKey;

	bool XmlNode(const xr_string& text, LPCSTR name, Frect& r, xr_string& texture, xr_string* tex_name = 0, u32* alpha = 0)
	{
		const xr_string open = xr_string("<") + name;
		size_t p = text.find(open);
		while (p != xr_string::npos && !(text[p + open.size()] == ' ' || text[p + open.size()] == '>' ||
			text[p + open.size()] == '\t' || text[p + open.size()] == '\r' || text[p + open.size()] == '\n'))
			p = text.find(open, p + 1);
		if (p == xr_string::npos) return false;
		const size_t gt = text.find('>', p);
		if (gt == xr_string::npos) return false;
		const xr_string tag = text.substr(p, gt - p);
		const float x = (float)atof(TagAttr(tag, "x").c_str()), y = (float)atof(TagAttr(tag, "y").c_str());
		r.set(x, y, x + (float)atof(TagAttr(tag, "width").c_str()), y + (float)atof(TagAttr(tag, "height").c_str()));
		// the first <texture...> inside the node
		const xr_string tex_open = tex_name ? xr_string("<") + *tex_name : xr_string("<texture");
		const size_t close_tag = text.find(xr_string("</") + name, gt);
		const size_t t = text.find(tex_open, gt);
		if (t != xr_string::npos && (close_tag == xr_string::npos || t < close_tag))
		{
			const size_t tgt = text.find('>', t);
			const size_t tend = text.find('<', tgt + 1);
			if (tgt != xr_string::npos && tend != xr_string::npos)
			{
				texture = NqUtil::Trim(text.substr(tgt + 1, tend - tgt - 1));
				if (alpha)
				{
					const xr_string a = TagAttr(text.substr(t, tgt - t), "a");
					*alpha = a.empty() ? 255 : (u32)atoi(a.c_str());
				}
			}
		}
		return true;
	}

	const SDevice& Device()
	{
		const xr_string key = LinkKey();
		if (s_Device.loaded && key == s_DeviceKey) return s_Device;
		s_Device = SDevice();
		s_Device.loaded = true;
		s_DeviceKey = key;
		// Dead Air's own values: what a game without the files would show
		s_Device.bg[0].set(0, 0, 1024, 768);			s_Device.bg[1].set(102, 0, 921, 768);
		s_Device.noise[0].set(25, 33, 998, 716);		s_Device.noise[1].set(122, 33, 900, 716);
		s_Device.close[0].set(973, 26, 1006, 60);		s_Device.close[1].set(880, 26, 906, 60);
		for (int k = 0; k < 2; ++k)
		{
			s_Device.bg_tex[k] = "ui_inGame2_pda_texture";
			s_Device.noise_tex[k] = "ui_pda2_noice";
			s_Device.close_tex[k] = "ui_inGame2_pda_offline_e";
			s_Device.noise_alpha[k] = 70;
			xr_string xml;
			if (!ReadGameFile(k ? "configs\\ui\\pda_16.xml" : "configs\\ui\\pda.xml", xml)) continue;
			xml = StripComments(xml);
			Frect r;
			xr_string t;
			if (XmlNode(xml, "background_static", r, t)) { s_Device.bg[k] = r; if (!t.empty()) s_Device.bg_tex[k] = t; }
			t.clear();
			u32 a = 255;
			if (XmlNode(xml, "noice_static", r, t, 0, &a)) { s_Device.noise[k] = r; if (!t.empty()) s_Device.noise_tex[k] = t; s_Device.noise_alpha[k] = a; }
			t.clear();
			xr_string te = "texture_e";
			if (XmlNode(xml, "close_button", r, t, &te)) { s_Device.close[k] = r; if (!t.empty()) s_Device.close_tex[k] = t; }
		}
		return s_Device;
	}

	// ============================================================== level maps
	struct SLevelMap
	{
		xr_string	tex;
		Fvector4	bound;		// x1, z1 (bottom), x2, z2 (top) in world units
		bool		tried, ok;
		SLevelMap() : tried(false), ok(false) { bound.set(0, 0, 0, 0); }
	};
	xr_map<xr_string, SLevelMap>	s_Maps;
	xr_string						s_MapKey;

	const SLevelMap* LevelMap(const xr_string& level)
	{
		const xr_string key = LinkKey();
		if (key != s_MapKey) { s_Maps.clear(); s_MapKey = key; }
		SLevelMap& m = s_Maps[level];
		if (m.tried) return m.ok ? &m : 0;
		m.tried = true;
		xr_string ltx;
		if (level.empty() || !ReadGameFile(("levels\\" + level + "\\level.ltx").c_str(), ltx)) return 0;
		xr_string tex, rect;
		if (!LtxValue(ltx, "level_map", "texture", tex) || !LtxValue(ltx, "level_map", "bound_rect", rect)) return 0;
		float a = 0, b = 0, c = 0, d = 0;
		xr_string r = rect;
		for (size_t i = 0; i < r.size(); ++i) if (r[i] == 'f') r[i] = ' ';
		if (4 != sscanf_s(r.c_str(), "%f , %f , %f , %f", &a, &b, &c, &d) && 4 != sscanf_s(r.c_str(), "%f,%f,%f,%f", &a, &b, &c, &d))
			return 0;
		m.tex = Lower(NqUtil::Trim(tex));
		m.bound.set(a, b, c, d);
		m.ok = c > a && d > b;
		return m.ok ? &m : 0;
	}

	// ============================================================== colours and text
	ImU32 Col(u32 argb, float mul = 1.f)
	{
		const u32 a = (u32)(((argb >> 24) & 0xFF) * mul);
		return IM_COL32((argb >> 16) & 0xFF, (argb >> 8) & 0xFF, argb & 0xFF, a > 255 ? 255 : a);
	}

	struct SLine
	{
		xr_string	cp;		// cp1251 bytes
		float		w;		// in glyph pixels
	};

	float GlyphW(const SFont& f, u8 c) { return f.gw[c]; }

	float Width(const SFont& f, const xr_string& cp)
	{
		float w = 0;
		for (size_t i = 0; i < cp.size(); ++i) w += GlyphW(f, (u8)cp[i]);
		return w;
	}

	// the lines a label shows in `max_w` glyph pixels: words wrap, or one line cut with "..."
	void LayoutText(const SFont& f, const xr_string& cp, float max_w, bool wrap, xr_vector<SLine>& out)
	{
		out.clear();
		xr_vector<xr_string> paras;
		size_t s = 0;
		for (size_t i = 0; i <= cp.size(); ++i)
			if (i == cp.size() || cp[i] == '\n') { paras.push_back(cp.substr(s, i - s)); s = i + 1; }
		for (u32 p = 0; p < paras.size(); ++p)
		{
			const xr_string& para = paras[p];
			if (!wrap)
			{
				SLine l;
				l.cp = para;
				l.w = Width(f, para);
				if (l.w > max_w && max_w > 0)
				{
					const float dots = Width(f, "...");
					while (!l.cp.empty() && Width(f, l.cp) + dots > max_w) l.cp.erase(l.cp.size() - 1);
					l.cp += "...";
					l.w = Width(f, l.cp);
				}
				out.push_back(l);
				continue;
			}
			xr_string line;
			size_t i = 0;
			while (i <= para.size())
			{
				size_t e = para.find(' ', i);
				if (e == xr_string::npos) e = para.size();
				const xr_string word = para.substr(i, e - i);
				const xr_string cand = line.empty() ? word : line + " " + word;
				if (!line.empty() && Width(f, cand) > max_w)
				{
					SLine l; l.cp = line; l.w = Width(f, line); out.push_back(l);
					line = word;
				}
				else line = cand;
				// a word wider than the line breaks where it must
				while (Width(f, line) > max_w && line.size() > 1)
				{
					size_t cut = line.size();
					while (cut > 1 && Width(f, line.substr(0, cut)) > max_w) --cut;
					SLine l; l.cp = line.substr(0, cut); l.w = Width(f, l.cp); out.push_back(l);
					line.erase(0, cut);
				}
				i = e + 1;
			}
			SLine l; l.cp = line; l.w = Width(f, line); out.push_back(l);
		}
	}

	// ============================================================== the preview state
	struct SAbs
	{
		ImVec2	a, b;		// pixels
		bool	shown;
	};

	struct SPreview
	{
		bool		open;
		int			focus;
		xr_string	path;			// absolute
		xr_string	rel;
		xr_string	text;			// what the source pane edits
		bool		text_dirty;
		u64			file_time;
		u32			next_poll;

		EditorPda::SLayout	layout;
		bool		read_ok;
		xr_string	read_err;
		int			read_line;

		// the running interface
		xr_string	page, prev_page, stock;
		bool		closed;
		xr_map<int, bool>		vis;			// item index -> shown, once an action changed it
		xr_map<xr_string, xr_string>	vars;	// set by actions
		xr_map<int, int>		row_sel;		// list index -> selected row
		xr_map<int, float>		scroll;			// list index -> scroll in canvas units
		float		map_zoom;
		Fvector2	map_center;
		bool		map_center_set;
		xr_vector<xr_string>	log;
		xr_string	toast;
		u32			toast_until;
		int			hovered, pressed;

		bool		wide;
		xr_string	lang;
		bool		show_source, show_side;
		float		zoom;		// 0 = fit

		// geometry of the last frame, for hit tests and MCP
		ImVec2		origin;
		float		kx, ky;		// pixels per virtual unit
		Frect		screen;		// virtual
		xr_vector<SAbs>	abs;

		SPreview() : open(false), focus(0), text_dirty(false), file_time(0), next_poll(0), read_ok(false),
			read_line(0), closed(false), map_zoom(1.f), map_center_set(false), toast_until(0), hovered(-1),
			pressed(-1), wide(false), lang("rus"), show_source(false), show_side(true), zoom(0), kx(1), ky(1)
		{ map_center.set(0, 0); }
	};
	SPreview	s_P;

	void Log(const xr_string& s)
	{
		s_P.log.push_back(s);
		if (s_P.log.size() > 60) s_P.log.erase(s_P.log.begin());
	}

	void Toast(const xr_string& s)
	{
		s_P.toast = s;
		s_P.toast_until = GetTickCount() + 2500;
		Log(s);
	}

	void ResetState()
	{
		s_P.page = s_P.layout.start;
		s_P.prev_page.clear();
		s_P.stock.clear();
		s_P.closed = false;
		s_P.vis.clear();
		s_P.vars.clear();
		s_P.row_sel.clear();
		s_P.scroll.clear();
		// the game centres on the player close up (ViewActor); zoom = "global" shows it all
		s_P.map_zoom = 3.f;
		for (u32 i = 0; i < s_P.layout.items.size(); ++i)
			if (s_P.layout.items[i].kind == "map" && s_P.layout.items[i].zoom == "global") s_P.map_zoom = 1.f;
		s_P.map_center_set = false;
		s_P.hovered = s_P.pressed = -1;
		// the page stays when the layout still has it: an edit should not throw the author back
	}

	void Reload(bool keep_state)
	{
		const xr_string page = s_P.page;
		EditorPda::SLayout l;
		xr_string err;
		int line = 0;
		s_P.read_ok = EditorPda::Read(s_P.text.c_str(), (u32)s_P.text.size(), NqUtil::BaseName(s_P.path.c_str()).c_str(),
			s_P.lang.c_str(), l, err, line);
		s_P.read_err = err;
		s_P.read_line = line;
		if (s_P.read_ok) s_P.layout = l;
		if (!keep_state || !s_P.layout.FindPage(page.c_str())) ResetState();
		else
		{
			// the item list may have changed shape: index-keyed state goes
			s_P.vis.clear();
			s_P.row_sel.clear();
			s_P.scroll.clear();
		}
	}

	bool LoadFile(LPCSTR abs)
	{
		xr_string text, err;
		if (!NqLua::ReadFile(abs, text, err)) return false;
		const bool same = s_P.path == abs;
		s_P.path = abs;
		s_P.rel = NqUtil::ProjectRelative(abs);
		s_P.text = text;
		s_P.text_dirty = false;
		s_P.file_time = NqLua::FileTime(abs);
		Reload(same);
		return true;
	}

	// ============================================================== placeholders
	bool VarOf(const xr_string& name, xr_string& out)
	{
		xr_map<xr_string, xr_string>::const_iterator it = s_P.vars.find(name);
		if (it != s_P.vars.end()) { out = it->second; return true; }
		for (u32 i = 0; i < s_P.layout.preview_vars.size(); ++i)
			if (s_P.layout.preview_vars[i].first == name) { out = s_P.layout.preview_vars[i].second; return true; }
		for (u32 i = 0; i < s_P.layout.vars.size(); ++i)
			if (s_P.layout.vars[i].first == name) { out = s_P.layout.vars[i].second; return true; }
		return false;
	}

	xr_string Format(const xr_string& raw)
	{
		if (raw.find('{') == xr_string::npos) return raw;
		xr_string out;
		for (size_t i = 0; i < raw.size();)
		{
			const size_t close = raw[i] == '{' ? raw.find('}', i) : xr_string::npos;
			if (close == xr_string::npos) { out += raw[i++]; continue; }
			const xr_string name = raw.substr(i + 1, close - i - 1);
			xr_string v;
			bool known = true;
			// the game fills these from the world; the preview from `preview.vars` or a stand-in
			static const LPCSTR kBuiltins[][2] = { { "time", "" }, { "date", "" }, { "actor", "Stalker" },
				{ "money", "12500" }, { "rank", "Experienced" }, { "community", "Loners" }, { "level", "" },
				{ "task", "The current task" } };
			bool builtin = false;
			for (u32 b = 0; b < sizeof(kBuiltins) / sizeof(kBuiltins[0]); ++b)
				if (name == kBuiltins[b][0])
				{
					builtin = true;
					bool from_preview = false;
					for (u32 k = 0; k < s_P.layout.preview_vars.size(); ++k)
						if (s_P.layout.preview_vars[k].first == name) { v = s_P.layout.preview_vars[k].second; from_preview = true; }
					if (from_preview) break;
					if (name == "time")
					{
						SYSTEMTIME st;
						GetLocalTime(&st);
						v = NqUtil::Format("%02d:%02d", st.wHour, st.wMinute);
					}
					else if (name == "date")
					{
						SYSTEMTIME st;
						GetLocalTime(&st);
						v = NqUtil::Format("%02d.%02d.2012", st.wDay, st.wMonth);
					}
					else if (name == "level") v = s_P.layout.preview_level.empty() ? "Cordon" : s_P.layout.preview_level;
					else v = kBuiltins[b][1];
				}
			if (builtin) {}
			else if (name == "title") v = s_P.layout.title;
			else if (name == "page") { const EditorPda::SPage* p = s_P.layout.FindPage(s_P.page.c_str()); v = p ? p->title : ""; }
			else if (!VarOf(name, v)) known = false;
			if (known) out += v; else out += raw.substr(i, close - i + 1);
			i = close + 1;
		}
		return out;
	}

	// ============================================================== geometry
	const EditorPda::SItem& It(int i) { return s_P.layout.items[i]; }

	bool ItemVisible(int i)
	{
		xr_map<int, bool>::const_iterator it = s_P.vis.find(i);
		return it != s_P.vis.end() ? it->second : It(i).visible;
	}

	ImVec2 VPx(float vx, float vy) { return ImVec2(s_P.origin.x + vx * s_P.kx, s_P.origin.y + vy * s_P.ky); }

	// canvas units -> pixels, per axis (the game's m_scale, then the UI's own scale)
	float Cx() { return s_P.screen.width() / s_P.layout.canvas_w * s_P.kx; }
	float Cy() { return s_P.screen.height() / s_P.layout.canvas_h * s_P.ky; }

	void ComputeLayout()
	{
		const EditorPda::SLayout& l = s_P.layout;
		const u32 n = (u32)l.items.size();
		s_P.abs.assign(n, SAbs());
		const ImVec2 root = VPx(s_P.screen.x1, s_P.screen.y1);
		const float cx = Cx(), cy = Cy();
		const bool stock = !s_P.stock.empty();
		for (u32 i = 0; i < n; ++i)
		{
			const EditorPda::SItem& it = l.items[i];
			SAbs& a = s_P.abs[i];
			bool shown = ItemVisible(i) && !s_P.closed;
			ImVec2 base = root;
			if (it.parent > 0)
			{
				const SAbs& p = s_P.abs[it.parent - 1];
				base = p.a;
				shown = shown && p.shown;
			}
			else if (it.layer == "page") shown = shown && it.page == s_P.page && !stock;
			else if (it.layer == "chrome") shown = shown && !stock;
			if (it.kind == "row" && it.parent > 0)
			{
				// rows stack in their list, scrolled
				const EditorPda::SItem& list = l.items[it.parent - 1];
				xr_map<int, float>::const_iterator sc = s_P.scroll.find(it.parent - 1);
				const float off = sc != s_P.scroll.end() ? sc->second : 0.f;
				const float y = (it.row - 1) * (list.row_h + list.gap) - off;
				a.a = ImVec2(base.x, base.y + y * cy);
				a.b = ImVec2(base.x + list.w * cx, base.y + (y + it.h) * cy);
			}
			else
			{
				a.a = ImVec2(base.x + it.x * cx, base.y + it.y * cy);
				a.b = ImVec2(a.a.x + it.w * cx, a.a.y + it.h * cy);
			}
			a.shown = shown;
		}
	}

	// the list a row (or anything under one) sits in, -1 if none
	int ListOf(int i)
	{
		for (int p = It(i).parent; p > 0; p = It(p - 1).parent)
			if (It(p - 1).kind == "list") return p - 1;
		return -1;
	}

	bool Inside(const ImVec2& m, const SAbs& a) { return m.x >= a.a.x && m.x < a.b.x && m.y >= a.a.y && m.y < a.b.y; }

	bool Clickable(const EditorPda::SItem& it)
	{
		return it.kind == "row" || (it.kind == "button" && !it.action.empty());
	}

	// the topmost clickable item under the mouse; a coloured panel above stops the search
	int HitTest(const ImVec2& m)
	{
		for (int i = (int)s_P.abs.size() - 1; i >= 0; --i)
		{
			const SAbs& a = s_P.abs[i];
			if (!a.shown || !Inside(m, a)) continue;
			const int list = ListOf(i);
			if (list >= 0 && !Inside(m, s_P.abs[list])) continue;
			const EditorPda::SItem& it = It(i);
			if (it.kind == "row" && m.y > a.a.y + it.h * Cy()) continue;
			if (Clickable(it)) return i;
			if ((it.kind == "rect" || it.kind == "group") && (it.color >> 24) > 0) return -2;
			if (it.kind == "map") return -3;
		}
		return -1;
	}

	bool Selected(int i)
	{
		const EditorPda::SItem& it = It(i);
		if (it.kind == "row")
		{
			xr_map<int, int>::const_iterator s = s_P.row_sel.find(it.parent - 1);
			return s != s_P.row_sel.end() && s->second == it.row;
		}
		return std::find(it.selected_on.begin(), it.selected_on.end(), s_P.page) != it.selected_on.end();
	}

	// ============================================================== actions
	void SetPage(const xr_string& id)
	{
		if (!s_P.layout.FindPage(id.c_str())) return;
		if (s_P.page != id) s_P.prev_page = s_P.page;
		s_P.page = id;
		const EditorPda::SPage* p = s_P.layout.FindPage(id.c_str());
		s_P.stock = p ? p->stock : "";
	}

	int FindId(const xr_string& id)
	{
		for (u32 i = 0; i < s_P.layout.items.size(); ++i)
			if (It(i).id == id) return (int)i;
		return -1;
	}

	void StockAction(const xr_string& section)
	{
		for (u32 i = 0; i < s_P.layout.pages.size(); ++i)
			if (s_P.layout.pages[i].stock == section) { SetPage(s_P.layout.pages[i].id); return; }
		if (!s_P.page.empty()) s_P.prev_page = s_P.page;
		s_P.stock = section;
	}

	void RunAction(const xr_string& a, int item)
	{
		Log(a);
		const size_t colon = a.find(':');
		const xr_string verb = colon == xr_string::npos ? a : a.substr(0, colon);
		const xr_string arg = colon == xr_string::npos ? "" : a.substr(colon + 1);
		if (verb == "page") SetPage(arg);
		else if (verb == "back")
		{
			if (!s_P.stock.empty() && (s_P.prev_page.empty() || s_P.prev_page == s_P.page))
			{
				const EditorPda::SPage* p = s_P.layout.FindPage(s_P.page.c_str());
				if (!p || p->stock.empty()) s_P.stock.clear();
			}
			else if (!s_P.prev_page.empty()) SetPage(s_P.prev_page);
		}
		else if (verb == "close")
		{
			s_P.closed = true;
			Toast("The PDA closes");
		}
		else if (verb == "show" || verb == "hide" || verb == "toggle")
		{
			const int i = FindId(arg);
			if (i >= 0) s_P.vis[i] = verb == "show" ? true : verb == "hide" ? false : !ItemVisible(i);
		}
		else if (verb == "stock") StockAction(arg);
		else if (verb == "map")
		{
			for (u32 i = 0; i < s_P.layout.items.size(); ++i)
				if (It(i).kind == "map" && It(i).page != s_P.page && !It(i).page.empty()) { SetPage(It(i).page); break; }
			if (arg == "zoom_in") s_P.map_zoom = _min(s_P.map_zoom * 1.5f, 12.f);
			else if (arg == "zoom_out") s_P.map_zoom = _max(s_P.map_zoom / 1.5f, 1.f);
			else if (arg == "global") { s_P.map_zoom = 1.f; s_P.map_center_set = false; }
			else if (arg == "actor") { s_P.map_zoom = _max(s_P.map_zoom, 3.f); s_P.map_center_set = false; }
		}
		else if (verb == "var")
		{
			const size_t eq = arg.find('=');
			if (eq != xr_string::npos) s_P.vars[arg.substr(0, eq)] = Format(arg.substr(eq + 1));
		}
		else if (verb == "signal") Toast("Quests receive signal \"" + arg + "\"");
		else if (verb == "lua") Toast("The game calls " + arg + "()");
		(void)item;
	}

	void Click(int i)
	{
		if (i < 0) return;
		const EditorPda::SItem& it = It(i);
		if (it.kind == "row") s_P.row_sel[it.parent - 1] = it.row;
		const xr_vector<xr_string> actions = it.action;
		for (u32 k = 0; k < actions.size(); ++k) RunAction(actions[k], i);
	}

	void OpenKey(LPCSTR key)
	{
		const EditorPda::SLayout& l = s_P.layout;
		const xr_string& a = 0 == xr_strcmp(key, "map") ? l.key_map : 0 == xr_strcmp(key, "contacts") ? l.key_contacts : l.key_tasks;
		s_P.closed = false;
		if (!a.empty()) RunAction(a, -1);
	}

	// ============================================================== drawing
	ImDrawList*	s_DL = 0;

	void DrawTex(const xr_string& id, const ImVec2& a, const ImVec2& b, u32 tint, float rounding = 0.f)
	{
		ImTextureID tex;
		ImVec2 uv0, uv1;
		if (!Tex(id, tex, uv0, uv1))
		{
			// a texture the game would not find: the magenta X-Ray shows for one
			s_DL->AddRectFilled(a, b, IM_COL32(255, 0, 255, 90));
			return;
		}
		if (rounding > 0.f) s_DL->AddImageRounded(tex, a, b, uv0, uv1, Col(tint), rounding);
		else s_DL->AddImage(tex, a, b, uv0, uv1, Col(tint));
	}

	void DrawLabel(const xr_string& utf8, const xr_string& font, u32 color, char align, char valign, bool wrap,
				   const ImVec2& a, const ImVec2& b)
	{
		if (utf8.empty() || (color >> 24) == 0) return;
		const xr_string cp = NqUtil::Utf8ToCp1251(utf8.c_str());
		const SFont* f = FontOf(font);
		const float px = s_P.ky;					// glyph pixel -> screen pixel (768 lines = the 1024 font set)
		const float max_w = (b.x - a.x) / px;
		if (!f)
		{
			// no game fonts (no linked game): the editor's own font, sized alike
			const float size = 16.f * px;
			ImFont* im = ImGui::GetFont();
			const ImVec2 ts = im->CalcTextSizeA(size, FLT_MAX, 0.f, utf8.c_str());
			float x = align == 'c' ? (a.x + b.x - ts.x) * 0.5f : align == 'r' ? b.x - ts.x : a.x;
			float y = valign == 't' ? a.y : valign == 'b' ? b.y - ts.y : (a.y + b.y - ts.y) * 0.5f;
			s_DL->AddText(im, size, ImVec2(x, y), Col(color), utf8.c_str());
			return;
		}
		xr_vector<SLine> lines;
		LayoutText(*f, cp, max_w, wrap, lines);
		const float lh = f->height * px;
		const float total = lh * lines.size();
		float y = valign == 't' ? a.y : valign == 'b' ? b.y - total : (a.y + b.y - total) * 0.5f;
		const float tw = (float)f->sheet->w, th = (float)f->sheet->h;
		for (u32 li = 0; li < lines.size(); ++li, y += lh)
		{
			const SLine& ln = lines[li];
			float x = align == 'c' ? (a.x + b.x - ln.w * px) * 0.5f : align == 'r' ? b.x - ln.w * px : a.x;
			for (size_t k = 0; k < ln.cp.size(); ++k)
			{
				const u8 c = (u8)ln.cp[k];
				const float gw = f->gw[c];
				if (gw > 0 && c != ' ')
				{
					const ImVec2 p0(x, y), p1(x + gw * px, y + f->height * px);
					const ImVec2 uv0(f->gx[c] / tw, f->gy[c] / th), uv1((f->gx[c] + gw) / tw, (f->gy[c] + f->height) / th);
					s_DL->AddImage((ImTextureID)f->sheet->view, p0, p1, uv0, uv1, Col(color));
				}
				x += gw * px;
			}
		}
	}

	u32 BackColor(int i)
	{
		const EditorPda::SItem& it = It(i);
		const bool clickable = Clickable(it);
		if (clickable && s_P.pressed == i && s_P.hovered == i && it.has_press) return it.press;
		if (Selected(i) && it.has_selected) return it.selected;
		if (clickable && s_P.hovered == i && it.has_hover) return it.hover;
		return it.color;
	}

	// Where the preview map puts a world point inside a map slot: the level fits the slot at zoom
	// 1 like the game's OptimalFit, world x right, world z up.
	struct SMapView
	{
		ImVec2		mid;
		float		fit;
		ImVec2 W2P(float x, float z) const
		{
			return ImVec2(mid.x + (x - s_P.map_center.x) * fit, mid.y - (z - s_P.map_center.y) * fit);
		}
	};

	bool MapView(const SLevelMap* m, const SAbs& a, SMapView& v)
	{
		if (!m) return false;
		const EditorPda::SLayout& l = s_P.layout;
		const float bw = m->bound.z - m->bound.x, bh = m->bound.w - m->bound.y;
		if (bw <= 0 || bh <= 0) return false;
		v.fit = _min((a.b.x - a.a.x) / bw, (a.b.y - a.a.y) / bh) * s_P.map_zoom;
		if (!s_P.map_center_set)
		{
			// on the player; the whole level when zoomed all the way out
			s_P.map_center = l.has_actor ? Fvector2().set(l.actor_x, l.actor_z)
				: Fvector2().set((m->bound.x + m->bound.z) * 0.5f, (m->bound.y + m->bound.w) * 0.5f);
			if (s_P.map_zoom <= 1.f) s_P.map_center.set((m->bound.x + m->bound.z) * 0.5f, (m->bound.y + m->bound.w) * 0.5f);
			s_P.map_center_set = true;
		}
		v.mid = ImVec2((a.a.x + a.b.x) * 0.5f, (a.a.y + a.b.y) * 0.5f);
		return true;
	}

	void DrawMap(int i, const SAbs& a)
	{
		const EditorPda::SLayout& l = s_P.layout;
		s_DL->PushClipRect(a.a, a.b, true);
		const SLevelMap* m = LevelMap(l.preview_level);
		SSheet* sheet = m ? Sheet(m->tex, sfGame) : 0;
		SMapView v;
		if (sheet && MapView(m, a, v))
		{
			s_DL->AddImage((ImTextureID)sheet->view, v.W2P(m->bound.x, m->bound.w), v.W2P(m->bound.z, m->bound.y));
			for (u32 k = 0; k < l.markers.size(); ++k)
			{
				const ImVec2 p = v.W2P(l.markers[k].x, l.markers[k].z);
				const float s = 11.f * s_P.ky;
				DrawTex(l.markers[k].icon, ImVec2(p.x - s, p.y - 2 * s), ImVec2(p.x + s, p.y), l.markers[k].color);
				if (!l.markers[k].hint.empty() && ImGui::IsMouseHoveringRect(ImVec2(p.x - s, p.y - 2 * s), ImVec2(p.x + s, p.y)))
					ImGui::SetTooltip("%s", l.markers[k].hint.c_str());
			}
			const ImVec2 ap = l.has_actor ? v.W2P(l.actor_x, l.actor_z) : v.mid;
			s_DL->AddCircleFilled(ap, 6.f * s_P.ky, IM_COL32(255, 255, 255, 255));
			s_DL->AddCircleFilled(ap, 4.f * s_P.ky, IM_COL32(40, 160, 255, 255));
		}
		else
		{
			s_DL->AddRectFilled(a.a, a.b, IM_COL32(24, 28, 22, 255));
			for (float x = a.a.x; x < a.b.x; x += 40.f * s_P.ky) s_DL->AddLine(ImVec2(x, a.a.y), ImVec2(x, a.b.y), IM_COL32(60, 70, 55, 120));
			for (float y = a.a.y; y < a.b.y; y += 40.f * s_P.ky) s_DL->AddLine(ImVec2(a.a.x, y), ImVec2(a.b.x, y), IM_COL32(60, 70, 55, 120));
			const char* msg = l.preview_level.empty() ? "The live map: set preview.level to see a level here"
				: "The live map: the level map was not found in the linked game";
			const ImVec2 ts = ImGui::CalcTextSize(msg);
			s_DL->AddText(ImVec2((a.a.x + a.b.x - ts.x) * 0.5f, (a.a.y + a.b.y - ts.y) * 0.5f), IM_COL32(200, 210, 190, 220), msg);
		}
		s_DL->PopClipRect();
		(void)i;
	}

	void DrawItem(int i, const xr_vector<xr_vector<int> >& kids)
	{
		const SAbs& a = s_P.abs[i];
		if (!a.shown) return;
		const EditorPda::SItem& it = It(i);
		const float cx = Cx(), cy = Cy();
		const float rounding = it.radius * cy;
		const bool shaped = it.kind != "text" && it.kind != "image";
		ImVec2 b = a.b;
		if (it.kind == "row") b.y = a.a.y + it.h * cy;
		if (shaped)
		{
			const u32 c = BackColor(i);
			if ((c >> 24) > 0) s_DL->AddRectFilled(a.a, b, Col(c), rounding);
		}
		if (it.kind == "map") DrawMap(i, a);
		else if (it.kind == "image") DrawTex(it.texture, a.a, a.b, it.tint, rounding);
		else if (it.kind == "text") DrawLabel(Format(it.text), it.font, it.text_color, it.align, it.valign, it.wrap, a.a, a.b);
		else if (it.kind == "button")
		{
			// the same placement CUIPdaItem::InitParts makes, in canvas units
			const float W = it.w, H = it.h;
			const bool stacked = !it.icon.empty() && it.has_text && it.align == 'c' && it.valign != 'c';
			const bool sel = Selected(i);
			const bool hov = Clickable(it) && s_P.hovered == i;
			float text_left = 0.f;
			if (!it.icon.empty())
			{
				const float icon = _min(it.icon_size, _min(W, H));
				const bool alone = !it.has_text;
				const float x = alone || stacked ? (W - icon) * 0.5f : 12.f;
				const float y = stacked ? H * 0.5f - icon + 2.f : (H - icon) * 0.5f;
				const u32 ic = sel && it.has_icon_selected ? it.icon_selected : it.icon_color;
				DrawTex(it.icon, ImVec2(a.a.x + x * cx, a.a.y + y * cy), ImVec2(a.a.x + (x + icon) * cx, a.a.y + (y + icon) * cy), ic);
				if (!alone && !stacked) text_left = x + icon + 8.f;
			}
			if (it.has_text)
			{
				const float pad = it.align != 'c' ? 12.f : 0.f;
				const float l = _max(pad, text_left), t = stacked ? H * 0.5f + 2.f : 0.f;
				u32 tc = it.text_color;
				if (sel && it.has_text_selected) tc = it.text_selected;
				else if (hov && it.has_text_hover) tc = it.text_hover;
				DrawLabel(Format(it.text), it.font, tc, it.align, stacked ? 't' : it.valign, it.wrap,
					ImVec2(a.a.x + l * cx, a.a.y + t * cy), ImVec2(a.a.x + (W - pad) * cx, a.a.y + H * cy));
			}
		}
		// children: a list clips its rows, and shows its empty line while it has none
		if (it.kind == "list")
		{
			s_DL->PushClipRect(a.a, a.b, true);
			bool any = false;
			for (u32 k = 0; k < kids[i].size(); ++k) { any = true; DrawItem(kids[i][k], kids); }
			s_DL->PopClipRect();
			if (!any && !it.empty.empty())
				DrawLabel(it.empty, it.font, it.text_color, 'c', 'c', false, a.a, ImVec2(a.b.x, a.a.y + _min(it.h, 60.f) * cy));
		}
		else
			for (u32 k = 0; k < kids[i].size(); ++k) DrawItem(kids[i][k], kids);
		if (shaped && it.has_border && (it.border >> 24) > 0)
			s_DL->AddRect(a.a, b, Col(it.border), rounding, 0, _max(1.f, it.border_w * cy));
	}

	void DrawStockPanel(const ImVec2& a, const ImVec2& b)
	{
		s_DL->AddRectFilled(a, b, IM_COL32(22, 24, 20, 245));
		const char* names[][2] = { { "eptTasks", "Tasks and map" }, { "eptRanking", "Ranking" }, { "eptRelations", "Relations" },
			{ "eptContacts", "Contacts" }, { "eptLogs", "Logs" }, { "eptEncyclopedia", "Encyclopedia" }, { "eptMap", "Map" },
			{ "eptStatistics", "Statistics" }, { "eptFractionWar", "Faction war" } };
		const char* title = s_P.stock.c_str();
		for (u32 k = 0; k < sizeof(names) / sizeof(names[0]); ++k)
			if (s_P.stock == names[k][0]) title = names[k][1];
		const xr_string l1 = NqUtil::Format("Stock PDA section: %s (%s)", title, s_P.stock.c_str());
		const char* l2 = "The game shows its own screen here; the overlay stays on top.";
		const ImVec2 t1 = ImGui::CalcTextSize(l1.c_str()), t2 = ImGui::CalcTextSize(l2);
		const float mx = (a.x + b.x) * 0.5f, my = (a.y + b.y) * 0.5f;
		s_DL->AddText(ImVec2(mx - t1.x * 0.5f, my - t1.y - 2), IM_COL32(230, 230, 220, 255), l1.c_str());
		s_DL->AddText(ImVec2(mx - t2.x * 0.5f, my + 2), IM_COL32(170, 170, 160, 255), l2);
	}

	// Draws the device and the layout into [p0, p0 + size) and handles the mouse there.
	void DrawDevice(const ImVec2& p0, const ImVec2& avail)
	{
		const SDevice& d = Device();
		const int k = s_P.wide ? 1 : 0;
		// the virtual 1024x768 space shown at 4:3 or 16:9
		const float aspect = s_P.wide ? 16.f / 9.f : 4.f / 3.f;
		float w = avail.x, h = avail.x / aspect;
		if (h > avail.y) { h = avail.y; w = h * aspect; }
		if (s_P.zoom > 0) { h = 768.f * s_P.zoom; w = h * aspect; }
		s_P.origin = ImVec2(p0.x + _max(0.f, (avail.x - w) * 0.5f), p0.y + _max(0.f, (avail.y - h) * 0.5f));
		s_P.kx = w / 1024.f;
		s_P.ky = h / 768.f;
		s_P.screen = d.noise[k];

		s_DL = ImGui::GetWindowDrawList();
		s_DL->PushClipRect(p0, ImVec2(p0.x + avail.x, p0.y + avail.y), true);
		s_DL->AddRectFilled(s_P.origin, ImVec2(s_P.origin.x + w, s_P.origin.y + h), IM_COL32(12, 12, 12, 255));
		DrawTex(d.bg_tex[k], VPx(d.bg[k].x1, d.bg[k].y1), VPx(d.bg[k].x2, d.bg[k].y2), 0xFFFFFFFF);

		const EditorPda::SLayout& l = s_P.layout;
		const u32 n = (u32)l.items.size();
		ComputeLayout();
		xr_vector<xr_vector<int> > kids(n);
		xr_vector<int> roots;
		for (u32 i = 0; i < n; ++i)
		{
			if (l.items[i].parent > 0) kids[l.items[i].parent - 1].push_back((int)i);
			else roots.push_back((int)i);
		}
		const ImVec2 sa = VPx(s_P.screen.x1, s_P.screen.y1), sb = VPx(s_P.screen.x2, s_P.screen.y2);
		s_DL->PushClipRect(sa, sb, true);
		if (s_P.closed)
			s_DL->AddRectFilled(sa, sb, IM_COL32(8, 8, 8, 255));
		else
		{
			for (u32 r = 0; r < roots.size(); ++r)
				if (l.items[roots[r]].layer != "overlay") DrawItem(roots[r], kids);
			if (!s_P.stock.empty()) DrawStockPanel(sa, sb);
			for (u32 r = 0; r < roots.size(); ++r)
				if (l.items[roots[r]].layer == "overlay") DrawItem(roots[r], kids);
		}
		s_DL->PopClipRect();
		if (l.noise) DrawTex(d.noise_tex[k], sa, sb, (d.noise_alpha[k] << 24) | 0xFFFFFF);
		if (l.close) DrawTex(d.close_tex[k], VPx(d.close[k].x1, d.close[k].y1), VPx(d.close[k].x2, d.close[k].y2), 0xFFFFFFFF);
		if (s_P.closed)
		{
			const char* msg = "Closed - click to open the PDA again";
			const ImVec2 ts = ImGui::CalcTextSize(msg);
			s_DL->AddText(ImVec2((sa.x + sb.x - ts.x) * 0.5f, (sa.y + sb.y - ts.y) * 0.5f), IM_COL32(200, 200, 200, 255), msg);
		}

		// the mouse
		ImGui::SetCursorScreenPos(p0);
		ImGui::InvisibleButton("##pda_canvas", ImVec2(_max(avail.x, 1.f), _max(avail.y, 1.f)));
		const bool over = ImGui::IsItemHovered();
		const ImVec2 m = ImGui::GetIO().MousePos;
		s_P.hovered = -1;
		if (over && !s_P.closed)
		{
			const int hit = HitTest(m);
			s_P.hovered = hit >= 0 ? hit : -1;
			if (hit >= 0 && !It(hit).hint.empty()) ImGui::SetTooltip("%s", Format(It(hit).hint).c_str());
			if (ImGui::IsMouseClicked(ImGuiMouseButton_Left)) s_P.pressed = hit >= 0 ? hit : -1;
			if (ImGui::IsMouseReleased(ImGuiMouseButton_Left))
			{
				if (s_P.pressed >= 0 && s_P.pressed == hit) Click(hit);
				s_P.pressed = -1;
			}
			// the wheel scrolls the list under the mouse, or zooms the map
			const float wheel = ImGui::GetIO().MouseWheel;
			if (wheel != 0.f)
			{
				for (int i = (int)s_P.abs.size() - 1; i >= 0; --i)
				{
					if (!s_P.abs[i].shown || !Inside(m, s_P.abs[i])) continue;
					if (It(i).kind == "list")
					{
						int rows = 0;
						for (u32 c = 0; c < kids[i].size(); ++c) if (It(kids[i][c]).kind == "row") ++rows;
						const float content = rows * (It(i).row_h + It(i).gap);
						float& sc = s_P.scroll[i];
						sc = clampr(sc - wheel * 60.f, 0.f, _max(0.f, content - It(i).h));
						break;
					}
					if (It(i).kind == "map")
					{
						s_P.map_zoom = clampr(s_P.map_zoom * (wheel > 0 ? 1.25f : 0.8f), 1.f, 12.f);
						break;
					}
				}
			}
			// dragging the map pans it
			if (ImGui::IsMouseDragging(ImGuiMouseButton_Left) && s_P.pressed < 0)
				for (int i = (int)s_P.abs.size() - 1; i >= 0; --i)
				{
					if (!s_P.abs[i].shown || It(i).kind != "map" || !Inside(m, s_P.abs[i])) continue;
					const SLevelMap* lm = LevelMap(l.preview_level);
					if (lm && s_P.map_center_set)
					{
						const float bw = lm->bound.z - lm->bound.x, bh = lm->bound.w - lm->bound.y;
						const float fit = _min((s_P.abs[i].b.x - s_P.abs[i].a.x) / bw, (s_P.abs[i].b.y - s_P.abs[i].a.y) / bh) * s_P.map_zoom;
						const ImVec2 dd = ImGui::GetIO().MouseDelta;
						s_P.map_center.x -= dd.x / fit;
						s_P.map_center.y += dd.y / fit;
					}
					break;
				}
		}
		else if (over && s_P.closed && ImGui::IsMouseClicked(ImGuiMouseButton_Left))
			s_P.closed = false;
		if (!over && !ImGui::IsMouseDown(ImGuiMouseButton_Left)) s_P.pressed = -1;

		// a script action or a signal leaves a note on the screen for a moment
		if (!s_P.toast.empty() && GetTickCount() < s_P.toast_until)
		{
			const ImVec2 ts = ImGui::CalcTextSize(s_P.toast.c_str());
			const ImVec2 c((sa.x + sb.x) * 0.5f, sb.y - 40.f * s_P.ky);
			s_DL->AddRectFilled(ImVec2(c.x - ts.x * 0.5f - 12, c.y - ts.y * 0.5f - 7), ImVec2(c.x + ts.x * 0.5f + 12, c.y + ts.y * 0.5f + 7),
				IM_COL32(0, 0, 0, 200), 8.f);
			s_DL->AddText(ImVec2(c.x - ts.x * 0.5f, c.y - ts.y * 0.5f), IM_COL32(255, 255, 255, 255), s_P.toast.c_str());
		}
		s_DL->PopClipRect();
		s_DL = 0;
	}

	// ============================================================== the window
	int GrowText(ImGuiInputTextCallbackData* data)
	{
		if (ImGuiInputTextFlags_CallbackResize == data->EventFlag)
		{
			xr_string* text = (xr_string*)data->UserData;
			text->resize((size_t)data->BufTextLen);
			data->Buf = &(*text)[0];
		}
		return 0;
	}

	bool SaveText(xr_string& err)
	{
		if (!NqLua::WriteFile(s_P.path.c_str(), s_P.text, err)) return false;
		s_P.text_dirty = false;
		s_P.file_time = NqLua::FileTime(s_P.path.c_str());
		return true;
	}

	void DrawToolbar()
	{
		const float em = ImGui::GetFontSize();
		ImGui::TextUnformatted(s_P.rel.c_str());
		ImGui::SameLine();
		const xr_string used = EditorPda::UsedLayout();
		const bool is_used = EditorPda::SamePath(used.c_str(), s_P.rel.c_str());
		if (is_used) ImGui::TextColored(ImVec4(0.45f, 0.85f, 0.45f, 1.f), "used by the module");
		else
		{
			if (ImGui::SmallButton("Use in Module"))
			{
				xr_string err;
				if (!EditorPda::Use(s_P.rel.c_str(), err)) Toast(err);
				else Toast("mod.ltx: [pda] layout = " + s_P.rel);
			}
			if (ImGui::IsItemHovered())
				ImGui::SetTooltip("Writes [pda] layout = %s into mod.ltx: the game shows this interface in the PDA.", s_P.rel.c_str());
		}
		ImGui::SameLine(0, em);
		if (ImGui::SmallButton(s_P.wide ? "16:9" : "4:3")) s_P.wide = !s_P.wide;
		if (ImGui::IsItemHovered()) ImGui::SetTooltip("Screen shape: 4:3 draws pda.xml, 16:9 draws pda_16.xml");
		ImGui::SameLine();
		if (ImGui::SmallButton(s_P.lang == "eng" ? "eng" : "rus"))
		{
			s_P.lang = s_P.lang == "eng" ? "rus" : "eng";
			Reload(true);
		}
		if (ImGui::IsItemHovered()) ImGui::SetTooltip("The language of the texts: { rus = ..., eng = ... }");
		ImGui::SameLine();
		if (ImGui::SmallButton(s_P.zoom > 0 ? "100%" : "Fit")) s_P.zoom = s_P.zoom > 0 ? 0.f : 1.f;
		if (ImGui::IsItemHovered()) ImGui::SetTooltip("Fit the window, or show the 1024x768 PDA pixel for pixel");
		ImGui::SameLine();
		if (ImGui::SmallButton("Restart")) ResetState();
		if (ImGui::IsItemHovered()) ImGui::SetTooltip("Back to the start page, every var and panel as the file declares them");
		ImGui::SameLine(0, em);
		if (ImGui::SmallButton("Map Key")) OpenKey("map");
		if (ImGui::IsItemHovered()) ImGui::SetTooltip("What the map key opens: %s", s_P.layout.key_map.c_str());
		ImGui::SameLine();
		if (ImGui::SmallButton("Contacts Key")) OpenKey("contacts");
		if (ImGui::IsItemHovered()) ImGui::SetTooltip("What the contacts key opens: %s", s_P.layout.key_contacts.c_str());
		ImGui::SameLine(0, em);
		ImGui::Checkbox("Source", &s_P.show_source);
		ImGui::SameLine();
		ImGui::Checkbox("Details", &s_P.show_side);
	}

	void DrawSide(float height)
	{
		const EditorPda::SLayout& l = s_P.layout;
		ImGui::BeginChild("pda_side", ImVec2(0, height), true);
		ImGui::TextDisabled("Pages");
		for (u32 i = 0; i < l.pages.size(); ++i)
		{
			const EditorPda::SPage& p = l.pages[i];
			const xr_string label = p.stock.empty() ? NqUtil::Format("%s  (%s)", p.title.c_str(), p.id.c_str())
				: NqUtil::Format("%s  (%s, stock %s)", p.title.c_str(), p.id.c_str(), p.stock.c_str());
			if (ImGui::Selectable(label.c_str(), s_P.page == p.id)) SetPage(p.id);
		}
		ImGui::Separator();
		ImGui::TextDisabled("Vars");
		xr_vector<xr_string> names;
		for (u32 i = 0; i < l.vars.size(); ++i) names.push_back(l.vars[i].first);
		for (xr_map<xr_string, xr_string>::const_iterator it = s_P.vars.begin(); it != s_P.vars.end(); ++it)
			if (std::find(names.begin(), names.end(), it->first) == names.end()) names.push_back(it->first);
		for (u32 i = 0; i < names.size(); ++i)
		{
			xr_string v;
			VarOf(names[i], v);
			ImGui::Text("{%s} = %s", names[i].c_str(), v.c_str());
		}
		ImGui::Separator();
		ImGui::TextDisabled("Actions");
		for (int i = (int)s_P.log.size() - 1; i >= 0 && i >= (int)s_P.log.size() - 14; --i)
			ImGui::TextUnformatted(s_P.log[i].c_str());
		ImGui::Separator();
		const int missing_errors = l.errors;
		ImGui::TextDisabled("Problems (%d error(s), %d warning(s))", missing_errors, l.warnings);
		if (!s_P.read_ok)
			ImGui::TextColored(ImVec4(1.f, 0.4f, 0.4f, 1.f), "%s%s", s_P.read_line ? NqUtil::Format("line %d: ", s_P.read_line).c_str() : "",
				s_P.read_err.c_str());
		for (u32 i = 0; i < l.problems.size(); ++i)
		{
			const EditorPda::SProblem& p = l.problems[i];
			const ImVec4 c = p.code.size() && p.code[0] == 'E' ? ImVec4(1.f, 0.45f, 0.45f, 1.f) : ImVec4(1.f, 0.8f, 0.35f, 1.f);
			ImGui::TextColored(c, "%s", p.code.c_str());
			ImGui::SameLine();
			ImGui::TextWrapped("%s: %s", p.path.empty() ? "layout" : p.path.c_str(), p.message.c_str());
		}
		ImGui::Separator();
		ImGui::TextDisabled("Reader: %s", EditorPda::ReaderSource());
		ImGui::EndChild();
	}

	void DrawSource(float height)
	{
		const float em = ImGui::GetFontSize();
		ImGui::BeginChild("pda_source", ImVec2(0, height), true);
		if (ImGui::SmallButton("Apply"))
			Reload(true);
		if (ImGui::IsItemHovered()) ImGui::SetTooltip("Shows the text in the preview without saving it");
		ImGui::SameLine();
		if (ImGui::SmallButton("Save"))
		{
			xr_string err;
			if (SaveText(err)) { Reload(true); Toast("Saved " + s_P.rel); }
			else Toast(err);
		}
		ImGui::SameLine();
		if (ImGui::SmallButton("Revert")) LoadFile(s_P.path.c_str());
		if (ImGui::IsItemHovered()) ImGui::SetTooltip("Reads the file again; unsaved edits of the text are dropped");
		if (s_P.text_dirty) { ImGui::SameLine(); ImGui::TextColored(ImVec4(1.f, 0.8f, 0.3f, 1.f), "unsaved"); }
		if (s_P.text.capacity() < s_P.text.size() + 4096) s_P.text.reserve(s_P.text.size() + 16384);
		if (ImGui::InputTextMultiline("##pda_text", &s_P.text[0], s_P.text.capacity() + 1, ImVec2(-1.f, -1.f),
			ImGuiInputTextFlags_CallbackResize | ImGuiInputTextFlags_AllowTabInput, &GrowText, &s_P.text))
			s_P.text_dirty = true;
		(void)em;
		ImGui::EndChild();
	}

	void Poll()
	{
		// an agent or another editor wrote the file: the preview follows unless the text pane holds edits
		const u32 now = GetTickCount();
		if (now < s_P.next_poll || s_P.path.empty()) return;
		s_P.next_poll = now + 500;
		const u64 t = NqLua::FileTime(s_P.path.c_str());
		if (t && t != s_P.file_time && !s_P.text_dirty) LoadFile(s_P.path.c_str());
	}

	// ============================================================== MCP state
	// A preview no window has drawn yet (or one closed) still answers in canvas units: the
	// device is laid out at one pixel per virtual unit.
	void EnsureGeometry()
	{
		if (s_P.open && s_P.screen.width() > 0) return;
		s_P.origin = ImVec2(0, 0);
		s_P.kx = s_P.ky = 1.f;
		s_P.screen = Device().noise[s_P.wide ? 1 : 0];
	}

	void JsonState(xr_string& out)
	{
		EnsureGeometry();
		ComputeLayout();
		out += "{\"path\":";
		NqUtil::JsonPath(out, s_P.rel.c_str());
		out += ",\"page\":";
		NqUtil::JsonString(out, s_P.page);
		out += ",\"stock\":";
		NqUtil::JsonString(out, s_P.stock);
		out += NqUtil::Format(",\"closed\":%s,\"aspect\":\"%s\",\"lang\":\"%s\",\"read\":%s", s_P.closed ? "true" : "false",
			s_P.wide ? "16:9" : "4:3", s_P.lang.c_str(), s_P.read_ok ? "true" : "false");
		if (!s_P.read_ok)
		{
			out += ",\"read_error\":";
			NqUtil::JsonString(out, s_P.read_line ? NqUtil::Format("line %d: %s", s_P.read_line, s_P.read_err.c_str()) : s_P.read_err);
		}
		out += NqUtil::Format(",\"errors\":%d,\"warnings\":%d", s_P.layout.errors, s_P.layout.warnings);
		// the preview map's world: where preview.actor and preview.markers land (x right, z up)
		if (const SLevelMap* m = LevelMap(s_P.layout.preview_level))
		{
			out += ",\"map\":{\"level\":";
			NqUtil::JsonString(out, s_P.layout.preview_level);
			out += NqUtil::Format(",\"bound\":[%g,%g,%g,%g],\"zoom\":%g", m->bound.x, m->bound.y, m->bound.z, m->bound.w,
				s_P.map_zoom);
			// what the slot shows, in canvas units - the game's describe() lists its spots the same way
			for (u32 i = 0; i < s_P.abs.size(); ++i)
			{
				SMapView v;
				if (It(i).kind != "map" || !s_P.abs[i].shown || !MapView(m, s_P.abs[i], v)) continue;
				const ImVec2 root = VPx(s_P.screen.x1, s_P.screen.y1);
				const float cx = Cx(), cy = Cy();
				const SAbs& slot = s_P.abs[i];
				bool first_spot = true;
				const auto spot = [&](LPCSTR what, const xr_string& hint, const ImVec2& p) {
					if (!Inside(p, slot)) return;
					out += first_spot ? "" : ",";
					first_spot = false;
					out += NqUtil::Format("{\"what\":\"%s\",\"x\":%.0f,\"y\":%.0f", what, (p.x - root.x) / cx, (p.y - root.y) / cy);
					if (!hint.empty()) { out += ",\"hint\":"; NqUtil::JsonString(out, hint); }
					out += "}";
				};
				out += ",\"spots\":[";
				const EditorPda::SLayout& l = s_P.layout;
				spot("player", xr_string(), l.has_actor ? v.W2P(l.actor_x, l.actor_z) : v.mid);
				for (u32 k = 0; k < l.markers.size(); ++k)
					spot("marker", l.markers[k].hint, v.W2P(l.markers[k].x, l.markers[k].z));
				out += "]";
				break;
			}
			out += "}";
		}
		out += ",\"vars\":{";
		bool first = true;
		for (u32 i = 0; i < s_P.layout.vars.size(); ++i)
		{
			xr_string v;
			VarOf(s_P.layout.vars[i].first, v);
			if (!first) out += ",";
			first = false;
			NqUtil::JsonString(out, s_P.layout.vars[i].first);
			out += ":";
			NqUtil::JsonString(out, v);
		}
		for (xr_map<xr_string, xr_string>::const_iterator it = s_P.vars.begin(); it != s_P.vars.end(); ++it)
		{
			bool declared = false;
			for (u32 i = 0; i < s_P.layout.vars.size(); ++i) declared |= s_P.layout.vars[i].first == it->first;
			if (declared) continue;
			if (!first) out += ",";
			first = false;
			NqUtil::JsonString(out, it->first);
			out += ":";
			NqUtil::JsonString(out, it->second);
		}
		out += "},\"log\":";
		NqUtil::JsonStringArray(out, s_P.log);
		// what is on the screen, in CANVAS units: the coordinates an agent writes into the file
		out += ",\"visible\":[";
		first = true;
		const ImVec2 root = VPx(s_P.screen.x1, s_P.screen.y1);
		const float cx = Cx(), cy = Cy();
		for (u32 i = 0; i < s_P.abs.size(); ++i)
		{
			const SAbs& a = s_P.abs[i];
			const EditorPda::SItem& it = It(i);
			if (!a.shown) continue;
			const bool interesting = !it.id.empty() || Clickable(it) || it.has_text || it.kind == "map" || it.kind == "list";
			if (!interesting) continue;
			if (!first) out += ",";
			first = false;
			out += NqUtil::Format("{\"n\":%u,\"kind\":\"%s\"", i + 1, it.kind.c_str());
			if (!it.id.empty()) { out += ",\"id\":"; NqUtil::JsonString(out, it.id); }
			if (it.kind == "row") out += NqUtil::Format(",\"row\":%d,\"selected\":%s", it.row, Selected(i) ? "true" : "false");
			out += NqUtil::Format(",\"x\":%.0f,\"y\":%.0f,\"w\":%.0f,\"h\":%.0f", (a.a.x - root.x) / cx, (a.a.y - root.y) / cy,
				(a.b.x - a.a.x) / cx, (a.b.y - a.a.y) / cy);
			if (it.has_text) { out += ",\"text\":"; NqUtil::JsonString(out, Format(it.text)); }
			if (!it.action.empty()) { out += ",\"action\":"; NqUtil::JsonStringArray(out, it.action); }
			if (it.kind == "button" && !it.selected_on.empty()) out += NqUtil::Format(",\"selected\":%s", Selected(i) ? "true" : "false");
			out += "}";
		}
		out += "]}";
	}
}

//==============================================================================
// the shared half
//==============================================================================
bool EditorPdaInternal::TextureKnown(LPCSTR id)
{
	if (!id || !id[0]) return false;
	LoadTexIds();
	if (s_TexIds.find(Lower(id)) != s_TexIds.end()) return true;
	// a texture file of the project or the game
	xr_string rel = Lower(id);
	if (EditorProject::Active())
	{
		string_path abs;
		sprintf_s(abs, "%s\\gamedata\\textures\\%s.dds", EditorProject::Root(), rel.c_str());
		if (NqUtil::FileExists(abs)) return true;
	}
	return EditorGameContent::Find(("textures\\" + rel + ".dds").c_str()) >= 0;
}

void EditorPdaInternal::FileWritten(LPCSTR abs_path)
{
	if (s_P.path.empty() || 0 != _stricmp(s_P.path.c_str(), abs_path)) return;
	if (!NqUtil::FileExists(abs_path)) { s_P.open = false; s_P.path.clear(); return; }
	if (!s_P.text_dirty) LoadFile(abs_path);
}

void EditorPda::Invalidate()
{
	DropTextures();
	s_Fonts.clear();
	s_FontKey.clear();
	s_Device = SDevice();
	s_Maps.clear();
}

void EditorPda::Open(LPCSTR path)
{
	if (!path || !path[0]) return;
	if (LoadFile(path))
	{
		s_P.open = true;
		s_P.focus = 3;
	}
}

bool EditorPda::IsOpen() { return s_P.open; }

void EditorPda::DrawUI()
{
	if (!s_P.open) return;
	if (!EditorProject::Active()) { s_P.open = false; return; }
	Poll();
	if (s_P.focus > 0) { ImGui::SetNextWindowFocus(); --s_P.focus; }
	// a tab beside the viewport, like a quest graph
	UI->DockNextWindowWith("Render");
	ImGui::SetNextWindowSize(ImVec2(1100.f, 760.f), ImGuiCond_FirstUseEver);
	const xr_string title = NqUtil::Format("PDA: %s%s###pda_preview", NqUtil::BaseName(s_P.path.c_str()).c_str(),
		s_P.text_dirty ? " *" : "");
	bool open = true;
	if (!ImGui::Begin(title.c_str(), &open))
	{
		ImGui::End();
		if (!open) s_P.open = false;
		return;
	}
	if (!open) s_P.open = false;
	DrawToolbar();
	ImGui::Separator();
	const ImVec2 avail = ImGui::GetContentRegionAvail();
	const float em = ImGui::GetFontSize();
	const float side_w = s_P.show_side ? _min(em * 22.f, avail.x * 0.35f) : 0.f;
	const float source_h = s_P.show_source ? _max(avail.y * 0.38f, em * 8.f) : 0.f;
	const ImVec2 canvas(_max(avail.x - side_w - (side_w > 0 ? 8.f : 0.f), 64.f), _max(avail.y - source_h, 64.f));
	ImGui::BeginChild("pda_canvas", canvas, false, ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse);
	DrawDevice(ImGui::GetCursorScreenPos(), ImGui::GetContentRegionAvail());
	ImGui::EndChild();
	if (s_P.show_side)
	{
		ImGui::SameLine();
		DrawSide(canvas.y);
	}
	if (s_P.show_source) DrawSource(0.f);
	ImGui::End();
}

//==============================================================================
// MCP pda_preview
//==============================================================================
void EditorPda::McpPreview(LPCSTR raw, xr_string& out)
{
	if (!EditorProject::Active()) { NqUtil::JsonError(out, "no active project"); return; }
	xr_string action = "state", path;
	NqUtil::ArgString(raw, "action", action);
	NqUtil::ArgString(raw, "path", path);
	if (!path.empty() || s_P.path.empty() || action == "open")
	{
		if (path.empty()) path = s_P.path.empty() ? UsedLayout() : s_P.path;
		if (path.empty()) { NqUtil::JsonError(out, "missing 'path' (and the module uses no PDA layout yet)"); return; }
		string_path abs;
		xr_string err;
		if (!EditorFileOps::ResolveInProject(path.c_str(), abs, err)) { NqUtil::JsonError(out, err); return; }
		for (char* c = abs; *c; ++c)
			if (*c == '/') *c = '\\';
		if (0 != _stricmp(abs, s_P.path.c_str()) || action == "open")
		{
			if (!LoadFile(abs)) { NqUtil::JsonError(out, NqUtil::Format("%s cannot be read", path.c_str())); return; }
		}
	}
	if (action == "open") { s_P.open = true; s_P.focus = 3; }
	else if (action == "close") s_P.open = false;
	else if (action == "reset") ResetState();
	else if (action == "reload") LoadFile(s_P.path.c_str());
	else if (action == "page")
	{
		xr_string id;
		NqUtil::ArgString(raw, "page", id);
		if (!s_P.layout.FindPage(id.c_str())) { NqUtil::JsonError(out, NqUtil::Format("there is no page '%s'", id.c_str())); return; }
		s_P.closed = false;
		SetPage(id);
	}
	else if (action == "key")
	{
		xr_string key = "map";
		NqUtil::ArgString(raw, "key", key);
		OpenKey(key.c_str());
	}
	else if (action == "aspect")
	{
		xr_string a;
		NqUtil::ArgString(raw, "aspect", a);
		s_P.wide = a == "16:9" || a == "wide";
	}
	else if (action == "lang")
	{
		NqUtil::ArgString(raw, "lang", s_P.lang);
		Reload(true);
	}
	else if (action == "click")
	{
		// by id, by the n of the state's `visible` list, or by a canvas point
		xr_string id;
		int target = -1;
		EnsureGeometry();
		ComputeLayout();
		if (NqUtil::ArgString(raw, "id", id) && !id.empty())
		{
			target = FindId(id);
			if (target < 0) { NqUtil::JsonError(out, NqUtil::Format("no item has the id '%s'", id.c_str())); return; }
			if (!s_P.abs[target].shown) { NqUtil::JsonError(out, NqUtil::Format("'%s' is not on the screen", id.c_str())); return; }
			// a list's row by its number, like xms.pda.click(id, row) in the game
			const int row = NqUtil::ArgInt(raw, "row", 0);
			if (row > 0)
			{
				const int list = target;
				target = -1;
				for (u32 i = 0; i < s_P.layout.items.size(); ++i)
					if (It(i).kind == "row" && It(i).parent == list + 1 && It(i).row == row) target = (int)i;
				if (target < 0) { NqUtil::JsonError(out, NqUtil::Format("'%s' has no row %d", id.c_str(), row)); return; }
			}
		}
		else if (NqUtil::ArgInt(raw, "n", 0) > 0)
		{
			target = NqUtil::ArgInt(raw, "n", 0) - 1;
			if (target >= (int)s_P.abs.size() || !s_P.abs[target].shown) { NqUtil::JsonError(out, "that item is not on the screen"); return; }
		}
		else
		{
			float x = 0, y = 0;
			if (!NqUtil::ArgFloat(raw, "x", x) || !NqUtil::ArgFloat(raw, "y", y))
				{ NqUtil::JsonError(out, "click needs 'id', 'n' or a canvas point 'x','y'"); return; }
			const ImVec2 root = VPx(s_P.screen.x1, s_P.screen.y1);
			target = HitTest(ImVec2(root.x + x * Cx() + 0.01f, root.y + y * Cy() + 0.01f));
			if (target < 0) { NqUtil::JsonError(out, NqUtil::Format("nothing clickable at %.0f,%.0f", x, y)); return; }
		}
		if (s_P.closed) s_P.closed = false;
		Click(target);
	}
	else if (action != "state") { NqUtil::JsonError(out, NqUtil::Format("unknown action '%s'", action.c_str())); return; }
	out = "{\"ok\":true,\"open\":";
	out += s_P.open ? "true" : "false";
	out += ",\"state\":";
	JsonState(out);
	out += "}";
}
