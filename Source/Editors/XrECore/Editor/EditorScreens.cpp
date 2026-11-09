#include "stdafx.h"
#include "EditorScreens.h"
#include "EditorCatalog.h"
#include "EditorProject.h"
#include "EditorWorld.h"
#include "EDX11Utils.h"
#include "Nq/NqUtil.h"

using namespace EditorMod;
using Token = GrizzlyTheme::ColorToken;

namespace
{
	XUI::Strings OneFile(const xr_string& file)
	{
		XUI::Strings out;
		out.push_back(file);
		return out;
	}

	enum EFile { fileNone, filePicture, fileGif, fileVideo, fileShader, filePage, fileMusic, fileConvert };

	EFile FileKind(const xr_string& path)
	{
		const xr_string e = NqUtil::Extension(path.c_str());
		if (e == ".dds" || e == ".seq")									return filePicture;
		if (e == ".gif")												return fileGif;
		if (e == ".mp4" || e == ".webm" || e == ".wmv" || e == ".ogm")	return fileVideo;
		if (e == ".s" || e == ".ps")									return fileShader;
		if (e == ".html" || e == ".htm")								return filePage;
		if (e == ".ogg")												return fileMusic;
		if (e == ".png" || e == ".jpg" || e == ".jpeg" || e == ".tga" || e == ".bmp")	return fileConvert;
		return fileNone;
	}

	LPCSTR KindLabel(EFile k)
	{
		switch (k)
		{
		case filePicture:	return "Picture";
		case fileGif:		return "GIF";
		case fileVideo:		return "Video";
		case fileShader:	return "Shader";
		case filePage:		return "HTML Page";
		default:			return "Empty";
		}
	}

	XUI::Icon KindIcon(EFile k)
	{
		switch (k)
		{
		case filePicture:
		case fileGif:		return XUI::Icon::Picture;
		case fileVideo:		return XUI::Icon::Play;
		case fileShader:	return XUI::Icon::FileScript;
		case filePage:		return XUI::Icon::FileXml;
		default:			return XUI::Icon::File;
		}
	}

	bool				s_Open = false;
	bool				s_Have = false;
	SManifest			s_Manifest;
	SScreens			s_S;
	u32					s_Gen = u32(-1);
	xr_string			s_Root;
	int					s_Surface = 0;			// 0 the loading screen, 1 the menu
	xr_string			s_SelScreen, s_SelTrack;
	int					s_SelLayer = -1;
	xr_string			s_Status;
	xr_vector<xr_string> s_Problems;
	u32					s_CheckedAt = 0;

	struct SThumb { ID3D11ShaderResourceView* view = nullptr; u32 w = 0, h = 0; };
	xr_map<xr_string, SThumb> s_Thumbs;

	void DropThumbs()
	{
		for (auto& it : s_Thumbs)
			if (it.second.view) it.second.view->Release();
		s_Thumbs.clear();
	}

	const SThumb& Thumb(const xr_string& rel)
	{
		auto it = s_Thumbs.find(rel);
		if (it != s_Thumbs.end()) return it->second;
		SThumb t;
		const xr_string abs = xr_string(EditorProject::Root()) + "\\" + rel;
		if (NqUtil::FileExists(abs.c_str()))
			if ((t.view = DX11TextureFromFile(abs.c_str()))) DX11TextureSize(t.view, t.w, t.h);
		return s_Thumbs[rel] = t;
	}

	u32 SurfaceMask() { return s_Surface == 0 ? u32(scrLoading) : u32(scrMenu | scrPause); }

	SScreen* FindScreen(SScreens& s, const xr_string& id)
	{
		for (SScreen& d : s.screens) if (d.id == id) return &d;
		return nullptr;
	}

	STrack* FindTrack(SScreens& s, const xr_string& id)
	{
		for (STrack& d : s.tracks) if (d.id == id) return &d;
		return nullptr;
	}

	xr_string Sanitize(const xr_string& name)
	{
		xr_string out;
		for (char c : name)
		{
			if (c >= 'A' && c <= 'Z') c = char(c - 'A' + 'a');
			out += (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_' ? c : '_';
		}
		return out.empty() ? xr_string("item") : out;
	}

	template <typename F>
	xr_string UniqueId(const xr_string& base, F&& taken)
	{
		xr_string id = Sanitize(base);
		for (u32 n = 2; taken(id); ++n)
			id = Sanitize(base) + "_" + xr_string(std::to_string(n).c_str());
		return id;
	}

	xr_string NewScreenId(const SScreens& s, const xr_string& base)
	{
		return UniqueId(base, [&](const xr_string& id) { return FindScreen(const_cast<SScreens&>(s), id) != nullptr; });
	}

	xr_string NewTrackId(const SScreens& s, const xr_string& base)
	{
		return UniqueId(base, [&](const xr_string& id) { return FindTrack(const_cast<SScreens&>(s), id) != nullptr; });
	}

	// Every edit is saved at once: the manifest is read again first, so nothing another tab saved is written over.
	bool Save(const SScreens& s, xr_string& err)
	{
		SManifest m;
		if (!EditorProject::Active())						{ err = "no active project"; return false; }
		if (!EditorMod::Load(EditorProject::Root(), m))		{ err = "mod.ltx not found in the project"; return false; }
		m.screens = s;
		if (!EditorMod::Save(EditorProject::Root(), m))		{ err = "can't write mod.ltx"; return false; }
		return true;
	}

	void Commit()
	{
		xr_string err;
		s_Status = Save(s_S, err) ? xr_string() : "Not saved: " + err;
		s_CheckedAt = 0;
	}

	bool ReadWhole(const xr_string& path, xr_string& out)
	{
		FILE* f = nullptr;
		if (fopen_s(&f, path.c_str(), "rb") || !f) return false;
		fseek(f, 0, SEEK_END);
		const long size = ftell(f);
		fseek(f, 0, SEEK_SET);
		out.resize(size_t(size > 0 ? size : 0));
		const bool ok = !size || fread(&out[0], 1, out.size(), f) == out.size();
		fclose(f);
		return ok;
	}

	// A .ps alone runs too: the .s the game asks for is three lines, and they are written for the author.
	bool EnsureShaderScript(const xr_string& ps_abs, xr_string& err)
	{
		const xr_string script = ps_abs.substr(0, ps_abs.size() - 3) + ".s";
		if (NqUtil::FileExists(script.c_str())) return true;
		const xr_string name = NqUtil::BaseName(ps_abs.c_str());
		const xr_string text = "function normal(shader, t_base, t_second, t_detail)\r\n"
			"\tshader:begin(\"stub_notransform_t\", \"" + name + "\")\r\n"
			"\t\t: fog(false)\r\n\t\t: zb(false, false)\r\n\t\t: blend(true, blend.srcalpha, blend.invsrcalpha)\r\n"
			"\tshader:dx10texture(\"s_base\", t_base)\r\n\tshader:dx10sampler(\"smp_base\")\r\nend\r\n";
		FILE* f = nullptr;
		if (fopen_s(&f, script.c_str(), "wb") || !f) { err = "can't write " + script; return false; }
		fwrite(text.c_str(), 1, text.size(), f);
		fclose(f);
		return true;
	}

	bool Inside(const xr_string& abs, const xr_string& root)
	{
		return abs.size() > root.size() + 1 && 0 == _strnicmp(abs.c_str(), root.c_str(), root.size()) && abs[root.size()] == '\\';
	}

	// What the project holds of a file given from anywhere: one of the project stays where it is, one from outside is
	// copied into screens\, a picture the game does not read becomes a .dds.
	bool Import(const xr_string& given, xr_string& rel, xr_string& err)
	{
		const xr_string root = EditorProject::Root();
		xr_string in;
		NqUtil::NormalizePath(given.c_str(), in);
		const bool absolute = in.size() > 2 && in[1] == ':';
		const xr_string abs = absolute ? in : root + "\\" + in;
		if (!NqUtil::FileExists(abs.c_str())) { err = "The file does not exist: " + given; return false; }

		const EFile kind = FileKind(abs);
		if (kind == fileNone)
		{
			err = "The game shows pictures, GIFs, videos, shaders and HTML pages, and plays .ogg music - not " + abs.substr(abs.rfind('\\') + 1) + ".";
			return false;
		}

		xr_string target = abs;
		if (!Inside(abs, root))
		{
			const xr_string folder = root + "\\screens";
			::CreateDirectoryA(folder.c_str(), nullptr);
			const xr_string leaf = abs.substr(abs.rfind('\\') + 1);
			const size_t dot = leaf.rfind('.');
			const xr_string stem = dot == xr_string::npos ? leaf : leaf.substr(0, dot), ext = dot == xr_string::npos ? xr_string() : leaf.substr(dot);
			xr_string source;
			if (!ReadWhole(abs, source)) { err = "The file cannot be read: " + abs; return false; }
			target = folder + "\\" + leaf;
			for (u32 n = 2; NqUtil::FileExists(target.c_str()); ++n)
			{
				xr_string there;
				if (ReadWhole(target, there) && there == source) break;
				target = folder + "\\" + stem + "_" + xr_string(std::to_string(n).c_str()) + ext;
			}
			if (!NqUtil::FileExists(target.c_str()) && !::CopyFileA(abs.c_str(), target.c_str(), TRUE))
			{
				err = "The file cannot be copied into the project: " + abs;
				return false;
			}
			// a shader is two files, and a .ps may be picked for either
			if (kind == fileShader)
			{
				const xr_string base = abs.substr(0, abs.rfind('.'));
				const xr_string to = target.substr(0, target.rfind('.'));
				for (LPCSTR e : { ".s", ".ps" })
					if (NqUtil::FileExists((base + e).c_str()))
						::CopyFileA((base + e).c_str(), (to + e).c_str(), FALSE);
			}
		}

		if (kind == fileConvert)
		{
			const xr_string dds = target.substr(0, target.rfind('.')) + ".dds";
			if (!EditorCatalog::MakeScreenPicture(target.c_str(), dds.c_str(), err)) return false;
			if (!Inside(abs, root)) ::DeleteFileA(target.c_str());
			target = dds;
		}
		if (NqUtil::Extension(target.c_str()) == ".ps" && !EnsureShaderScript(target, err)) return false;

		rel = target.substr(root.size() + 1);
		return true;
	}

	xr_string Stem(const xr_string& file) { return NqUtil::BaseName(file.c_str()); }
	xr_string Leaf(const xr_string& file) { return file.substr(file.rfind('\\') + 1); }

	u32 DefaultShow() { return s_Surface == 0 ? u32(scrLoading) : u32(scrMenu | scrPause); }

	// the one path of a file brought from anywhere: dropped on the tab, on a list, on a screen or on a layer, or told over MCP
	void TakeFiles(SScreens& s, const XUI::Strings& files, const xr_string& screen, int layer, bool to_music, xr_string& err)
	{
		for (const xr_string& file : files)
		{
			xr_string rel;
			if (!Import(file, rel, err)) continue;
			if (FileKind(rel) == fileMusic)
			{
				STrack t;
				t.id = NewTrackId(s, Stem(rel));
				t.file = rel;
				t.play = DefaultShow();
				s.tracks.push_back(t);
				s_SelTrack = t.id;
				s_SelScreen.clear();
				continue;
			}
			if (to_music)
			{
				err = "Only .ogg files go into the music.";
				continue;
			}
			SScreen* d = screen.empty() ? nullptr : FindScreen(s, screen);
			if (!d)
			{
				SScreen fresh;
				fresh.id = NewScreenId(s, Stem(rel));
				fresh.show = DefaultShow();
				SScreenLayer l;
				l.file = rel;
				fresh.layers.push_back(l);
				s.screens.push_back(fresh);
				s_SelScreen = fresh.id;
				s_SelTrack.clear();
				s_SelLayer = 0;
				continue;
			}
			if (layer >= 0 && layer < int(d->layers.size()))
				d->layers[layer].file = rel;
			else
			{
				SScreenLayer l;
				l.file = rel;
				d->layers.push_back(l);
				s_SelLayer = int(d->layers.size()) - 1;
			}
			s_SelScreen = d->id;
			s_SelTrack.clear();
		}
	}

	void TakeDrop(const XUI::Strings& files, const xr_string& screen, int layer, bool to_music)
	{
		xr_string err;
		TakeFiles(s_S, files, screen, layer, to_music, err);
		Commit();
		if (!err.empty()) s_Status = err;
	}

	void Refresh()
	{
		const bool new_project = s_Root != EditorProject::Root();
		if (s_Gen != EditorMod::ManifestGeneration() || new_project)
		{
			if (new_project && !s_Root.empty())
			{
				DropThumbs();
				s_SelScreen.clear();
				s_SelTrack.clear();
				s_SelLayer = -1;
			}
			s_Have = EditorMod::Load(EditorProject::Root(), s_Manifest);
			s_S = s_Manifest.screens;
			s_Gen = EditorMod::ManifestGeneration();
			s_Root = EditorProject::Root();
			s_CheckedAt = 0;
		}
		const u32 now = ::GetTickCount();
		if (s_CheckedAt && now - s_CheckedAt <= 1000) return;
		s_CheckedAt = now ? now : 1;
		s_Problems.clear();
		if (s_Have)
		{
			SManifest m = s_Manifest;
			m.screens = s_S;
			EditorScreens::Validate(EditorProject::Root(), m, s_Problems);
		}
	}

	// what is wrong with one screen or track: "screen <id>:" or "track <id>:" after the code
	void ProblemsOf(LPCSTR what, const xr_string& id, xr_vector<xr_string>& out)
	{
		const xr_string tag = xr_string(what) + " " + id + ":";
		for (const xr_string& p : s_Problems)
			if (p.compare(5, tag.size(), tag) == 0) out.push_back(p);
	}

	void RowSpec(XUI::NavRowSpec& spec, LPCSTR what, const xr_string& id)
	{
		xr_vector<xr_string> list;
		ProblemsOf(what, id, list);
		if (list.empty()) return;
		bool error = false;
		for (const xr_string& p : list) error |= p[0] == 'E';
		spec.dot = true;
		spec.tone = error ? XUI::Tone::Error : XUI::Tone::Warning;
	}

	bool ListHeader(LPCSTR title, LPCSTR id, LPCSTR tip)
	{
		{
			XUI::FontScope strong(XUI::Font::Strong);
			ImGui::AlignTextToFramePadding();
			ImGui::TextUnformatted(title);
		}
		ImGui::SameLine(_max(0.f, ImGui::GetContentRegionMax().x - XUI::Pt(26.f)));
		return XUI::IconButton(id, XUI::Icon::Plus, tip);
	}

	void DrawScreensList()
	{
		const ImVec2 from = ImGui::GetCursorScreenPos();
		if (ListHeader("Screens", "##add_screen", "Add a screen"))
		{
			SScreen d;
			d.id = NewScreenId(s_S, "screen");
			d.show = DefaultShow();
			s_S.screens.push_back(d);
			s_SelScreen = d.id;
			s_SelTrack.clear();
			s_SelLayer = -1;
			Commit();
		}
		xr_string remove;
		bool any = false;
		for (const SScreen& d : s_S.screens)
		{
			if (!(d.show & SurfaceMask())) continue;
			any = true;
			XUI::NavRowSpec spec{};
			spec.icon = XUI::Icon::Picture;
			spec.action = XUI::Icon::Delete;
			spec.action_tip = "Remove this screen. Its files stay in the project.";
			const xr_string count = xr_string(std::to_string(d.layers.size()).c_str());
			spec.count = count.c_str();
			RowSpec(spec, "screen", d.id);
			bool action = false;
			if (XUI::NavRow(("screen." + d.id).c_str(), d.id.c_str(), s_SelScreen == d.id, spec, &action))
			{
				s_SelScreen = d.id;
				s_SelTrack.clear();
				s_SelLayer = -1;
			}
			if (action) remove = d.id;
		}
		if (!any)
			XUI::StatusText(XUI::Tone::Muted, "Drop pictures, videos or shaders here. Without a screen the game shows its own.");
		if (!remove.empty())
		{
			s_S.screens.erase(std::remove_if(s_S.screens.begin(), s_S.screens.end(), [&](const SScreen& d) { return d.id == remove; }), s_S.screens.end());
			if (s_SelScreen == remove) s_SelScreen.clear();
			Commit();
		}
		XUI::Strings files;
		const ImVec2 to(from.x + ImGui::GetContentRegionAvail().x, ImGui::GetCursorScreenPos().y + XUI::Pt(8.f));
		if (XUI::FileDropZone("##drop_screens", from, to, files))
			TakeDrop(files, xr_string(), -1, false);
	}

	void DrawMusicList()
	{
		const ImVec2 from = ImGui::GetCursorScreenPos();
		if (ListHeader("Music", "##add_track", "Add a track (.ogg)"))
		{
			char path[MAX_PATH * 2] = {};
			if (XUI::BrowsePath(path, sizeof(path), false))
				TakeDrop(OneFile(path), xr_string(), -1, true);
		}
		xr_string remove;
		for (const STrack& d : s_S.tracks)
		{
			XUI::NavRowSpec spec{};
			spec.icon = XUI::Icon::FileSound;
			spec.action = XUI::Icon::Delete;
			spec.action_tip = "Remove this track. Its file stays in the project.";
			spec.dim = !(d.play & SurfaceMask());
			RowSpec(spec, "track", d.id);
			bool action = false;
			if (XUI::NavRow(("track." + d.id).c_str(), d.id.c_str(), s_SelTrack == d.id, spec, &action))
			{
				s_SelTrack = d.id;
				s_SelScreen.clear();
				s_SelLayer = -1;
			}
			if (action) remove = d.id;
		}
		if (s_S.tracks.empty())
			XUI::StatusText(XUI::Tone::Muted, "Drop .ogg files here.");
		if (!remove.empty())
		{
			s_S.tracks.erase(std::remove_if(s_S.tracks.begin(), s_S.tracks.end(), [&](const STrack& d) { return d.id == remove; }), s_S.tracks.end());
			if (s_SelTrack == remove) s_SelTrack.clear();
			for (SScreen& d : s_S.screens)
			{
				d.tracks.erase(std::remove(d.tracks.begin(), d.tracks.end(), remove), d.tracks.end());
				if (d.music == "tracks" && d.tracks.empty()) d.music = "any";
			}
			Commit();
		}
		XUI::Strings files;
		const ImVec2 to(from.x + ImGui::GetContentRegionAvail().x, ImGui::GetCursorScreenPos().y + XUI::Pt(8.f));
		if (XUI::FileDropZone("##drop_music", from, to, files))
			TakeDrop(files, xr_string(), -1, true);

		if (!s_S.tracks.empty())
		{
			ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
			static const char* labels[] = { "Repeat", "Next Track" };
			int cur = s_S.next_track[s_Surface] ? 1 : 0;
			if (XUI::Segmented("##music_end", &cur, labels, 2))
			{
				s_S.next_track[s_Surface] = cur == 1;
				Commit();
			}
			XUI::Tip("What happens when a track ends.");
		}
	}

	void DrawPreview(const SScreen& d)
	{
		const float w = _min(ImGui::GetContentRegionAvail().x, XUI::Pt(520.f)), h = w * 9.f / 16.f;
		const ImVec2 a = ImGui::GetCursorScreenPos(), b(a.x + w, a.y + h);
		ImGui::Dummy(ImVec2(w, h));
		XUI::Tip("Where the layers go. Pictures are shown as they are; videos, GIFs, shaders and pages are marked, and move in the game.");
		ImDrawList* dl = ImGui::GetWindowDrawList();
		dl->AddRectFilled(a, b, GrizzlyTheme::U32(Token::Base));
		dl->PushClipRect(a, b, true);
		for (size_t i = 0; i < d.layers.size(); ++i)
		{
			const SScreenLayer& l = d.layers[i];
			const EFile kind = l.file.empty() ? fileNone : FileKind(l.file);
			ImVec2 p0(a.x + l.x * .01f * w, a.y + l.y * .01f * h), p1(p0.x + l.w * .01f * w, p0.y + l.h * .01f * h);
			const SThumb* t = kind == filePicture && NqUtil::Extension(l.file.c_str()) == ".dds" ? &Thumb(l.file) : nullptr;
			if (t && t->view && t->w && t->h)
			{
				ImVec2 uv0(0.f, 0.f), uv1(1.f, 1.f);
				const float bw = p1.x - p0.x, bh = p1.y - p0.y;
				if (l.fit == scrCover)
				{
					const float k = _max(bw / t->w, bh / t->h);
					const float u = bw / k / t->w, v = bh / k / t->h;
					uv0 = ImVec2((1.f - u) * .5f, (1.f - v) * .5f);
					uv1 = ImVec2((1.f + u) * .5f, (1.f + v) * .5f);
				}
				else if (l.fit == scrContain)
				{
					const float k = _min(bw / t->w, bh / t->h);
					const float ix = (bw - t->w * k) * .5f, iy = (bh - t->h * k) * .5f;
					p0 = ImVec2(p0.x + ix, p0.y + iy);
					p1 = ImVec2(p1.x - ix, p1.y - iy);
				}
				dl->AddImage((ImTextureID)t->view, p0, p1, uv0, uv1, IM_COL32(255, 255, 255, int(l.opacity * 255.f)));	// ui-units: ok a picture's own tint
			}
			else if (kind != fileNone)
			{
				dl->AddRect(p0, p1, GrizzlyTheme::U32(Token::Border));
				const ImVec2 c((p0.x + p1.x) * .5f, (p0.y + p1.y) * .5f - XUI::Pt(16.f) + XUI::Pt(float(i) * 18.f));
				XUI::DrawIcon(dl, KindIcon(kind), ImVec2(c.x - XUI::Pt(8.f), c.y - XUI::Pt(8.f)), 16.f, GrizzlyTheme::U32(Token::TextSecondary));
			}
		}
		dl->PopClipRect();
		dl->AddRect(a, b, GrizzlyTheme::U32(Token::Border));
	}

	void PickLayerFile(xr_string& value, LPCSTR id)
	{
		const int pick = XUI::PickField(id, value.c_str(), !value.empty());
		if (pick == XUI::PickClear)
		{
			value.clear();
			Commit();
		}
		else if (pick == XUI::PickBrowse)
		{
			char path[MAX_PATH * 2] = {};
			if (XUI::BrowsePath(path, sizeof(path), false))
			{
				xr_string rel, err;
				if (Import(path, rel, err)) value = rel;
				Commit();
				if (!err.empty()) s_Status = err;
			}
		}
	}

	void DrawLayerDetails(SScreen& d, SScreenLayer& l)
	{
		const EFile kind = l.file.empty() ? fileNone : FileKind(l.file);
		if (XUI::BeginPropertyGrid("screen_layer", .28f))
		{
			if (XUI::PropertyRow("File", "A picture, a GIF, a video, a shader or an HTML page. Drop one here from anywhere."))
				PickLayerFile(l.file, "##layer_file");
			if (kind == filePicture || kind == fileGif || kind == fileVideo)
				if (XUI::PropertyRow("Fit", "Cover fills the place and cuts the picture to it. Contain shows it whole. Stretch fills the place whatever the proportions."))
				{
					static const char* fits[] = { "Cover", "Contain", "Stretch" };
					if (XUI::Segmented("##fit", &l.fit, fits, 3)) Commit();
				}
			if (kind != fileNone && XUI::PropertyRow("Opacity"))
			{
				float percent = l.opacity * 100.f;
				ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
				if (ImGui::SliderFloat("##opacity", &percent, 0.f, 100.f, "%.0f %%")) l.opacity = percent * .01f;
				if (ImGui::IsItemDeactivatedAfterEdit()) Commit();
			}
			if (kind == fileShader && XUI::PropertyRow("Texture", "The picture the shader receives as s_base. Optional."))
				PickLayerFile(l.texture, "##layer_texture");
			if (kind == fileVideo && d.music == "video" && XUI::PropertyRow("Its Sound", "This video is heard while the screen shows.", 0))
				if (ImGui::Checkbox("##sound", &l.sound)) Commit();
			if (kind != fileNone && XUI::PropertySection("Position", false))
			{
				const auto field = [&](LPCSTR label, LPCSTR id, float& v)
				{
					if (!XUI::PropertyRow(label)) return;
					ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
					ImGui::DragFloat(id, &v, .1f, -100.f, 200.f, "%.1f %%");
					if (ImGui::IsItemDeactivatedAfterEdit()) Commit();
				};
				field("Left", "##x", l.x);
				field("Top", "##y", l.y);
				field("Width", "##w", l.w);
				field("Height", "##h", l.h);
				XUI::EndPropertySection();
			}
			XUI::EndPropertyGrid();
		}
	}

	void DrawLayers(SScreen& d)
	{
		const ImVec2 from = ImGui::GetCursorScreenPos();
		if (ListHeader("Layers", "##add_layer", "Add a layer"))
		{
			d.layers.push_back(SScreenLayer());
			s_SelLayer = int(d.layers.size()) - 1;
			Commit();
		}
		if (d.layers.empty())
			XUI::StatusText(XUI::Tone::Muted, "Add a layer, or drop a file here.");

		int remove = -1, move_from = -1, move_to = -1;
		const float row_h = XUI::Pt(28.f);
		// the top row is the top layer
		for (int i = int(d.layers.size()) - 1; i >= 0; --i)
		{
			SScreenLayer& l = d.layers[i];
			const EFile kind = l.file.empty() ? fileNone : FileKind(l.file);
			ImGui::PushID(i);
			const ImVec2 at = ImGui::GetCursorScreenPos();
			const float width = ImGui::GetContentRegionAvail().x;
			if (ImGui::Selectable("##row", s_SelLayer == i, ImGuiSelectableFlags_AllowOverlap, ImVec2(width, row_h)))
				s_SelLayer = s_SelLayer == i ? -1 : i;
			if (ImGui::BeginDragDropSource())
			{
				ImGui::SetDragDropPayload("XUI_SCREEN_LAYER", &i, sizeof(i));
				ImGui::TextUnformatted(l.file.empty() ? "Empty layer" : Leaf(l.file).c_str());
				ImGui::EndDragDropSource();
			}
			if (ImGui::BeginDragDropTarget())
			{
				if (const ImGuiPayload* p = ImGui::AcceptDragDropPayload("XUI_SCREEN_LAYER"))
				{
					move_from = *static_cast<const int*>(p->Data);
					move_to = i;
				}
				ImGui::EndDragDropTarget();
			}
			ImDrawList* dl = ImGui::GetWindowDrawList();
			const float cy = at.y + row_h * .5f;
			XUI::DrawIcon(dl, XUI::Icon::Grip, ImVec2(at.x + XUI::Pt(4.f), cy - XUI::Pt(6.f)), 12.f, GrizzlyTheme::U32(Token::TextDisabled));
			XUI::DrawIcon(dl, KindIcon(kind), ImVec2(at.x + XUI::Pt(20.f), cy - XUI::Pt(7.f)), 14.f, GrizzlyTheme::U32(Token::TextSecondary));
			const float right = at.x + width - XUI::Pt(26.f);
			XUI::TextAt(dl, XUI::Font::Body, ImVec2(at.x + XUI::Pt(40.f), cy - XUI::FontSize(XUI::Font::Body) * .5f),
				GrizzlyTheme::U32(kind == fileNone ? Token::TextSecondary : Token::Text), l.file.empty() ? "Empty: drop a file here" : Leaf(l.file).c_str(),
				_max(0.f, right - at.x - XUI::Pt(120.f)), XUI::Clip::Start);
			XUI::TextAt(dl, XUI::Font::Small, ImVec2(right - XUI::Pt(68.f), cy - XUI::FontSize(XUI::Font::Small) * .5f),
				GrizzlyTheme::U32(Token::TextSecondary), KindLabel(kind));
			ImGui::SameLine(width - XUI::Pt(22.f));
			ImGui::SetCursorScreenPos(ImVec2(right, at.y + XUI::Pt(3.f)));
			if (XUI::IconButton("##remove", XUI::Icon::Close, "Remove this layer. Its file stays in the project."))
				remove = i;
			ImGui::SetCursorScreenPos(ImVec2(at.x, at.y + row_h));
			XUI::Strings files;
			if (XUI::FileDropZone("##drop_layer", at, ImVec2(at.x + width, at.y + row_h), files))
				TakeDrop(OneFile(files[0]), d.id, i, false);
			if (s_SelLayer == i)
			{
				ImGui::Indent(XUI::Pt(20.f));
				DrawLayerDetails(d, l);
				ImGui::Unindent(XUI::Pt(20.f));
			}
			ImGui::PopID();
		}
		if (remove >= 0)
		{
			d.layers.erase(d.layers.begin() + remove);
			if (s_SelLayer == remove) s_SelLayer = -1;
			Commit();
		}
		if (move_from >= 0 && move_from != move_to)
		{
			const SScreenLayer moved = d.layers[move_from];
			d.layers.erase(d.layers.begin() + move_from);
			d.layers.insert(d.layers.begin() + move_to, moved);
			if (s_SelLayer == move_from) s_SelLayer = move_to;
			Commit();
		}
		XUI::Strings files;
		const ImVec2 to(from.x + ImGui::GetContentRegionAvail().x, ImGui::GetCursorScreenPos().y + XUI::Pt(8.f));
		if (XUI::FileDropZone("##drop_layers", from, to, files))
			TakeDrop(files, d.id, -1, false);
	}

	xr_string MusicLabel(const SScreen& d)
	{
		if (d.music == "none")	return "Silence";
		if (d.music == "video")	return "The Video's Own Sound";
		if (d.music == "tracks")
		{
			xr_string out;
			for (const xr_string& t : d.tracks) out += (out.empty() ? "" : ", ") + t;
			return out;
		}
		return "Any Track";
	}

	void DrawMusicCombo(SScreen& d)
	{
		ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
		if (!ImGui::BeginCombo("##music", MusicLabel(d).c_str())) return;
		const auto simple = [&](LPCSTR label, LPCSTR value)
		{
			if (ImGui::Selectable(label, d.music == value))
			{
				d.music = value;
				d.tracks.clear();
				Commit();
			}
		};
		simple("Any Track", "any");
		simple("Silence", "none");
		if (s_Surface == 0)
			simple("The Video's Own Sound", "video");
		if (!s_S.tracks.empty()) ImGui::Separator();
		for (const STrack& t : s_S.tracks)
		{
			const bool on = d.music == "tracks" && std::find(d.tracks.begin(), d.tracks.end(), t.id) != d.tracks.end();
			if (ImGui::Selectable(t.id.c_str(), on, ImGuiSelectableFlags_DontClosePopups))
			{
				if (d.music != "tracks") d.tracks.clear();
				if (on)	d.tracks.erase(std::remove(d.tracks.begin(), d.tracks.end(), t.id), d.tracks.end());
				else	d.tracks.push_back(t.id);
				d.music = d.tracks.empty() ? "any" : "tracks";
				Commit();
			}
		}
		ImGui::EndCombo();
	}

	bool MaskBox(LPCSTR label, u32& mask, u32 bit, bool invert = false)
	{
		bool on = invert ? !(mask & bit) : (mask & bit) != 0;
		if (!ImGui::Checkbox(label, &on)) return false;
		if (on != invert) mask |= bit; else mask &= ~bit;
		return true;
	}

	// the mode this module's games run in; empty when it targets the plain game or every mode
	xr_string ModuleMode()
	{
		if (!s_Manifest.provides_modes.empty()) return s_Manifest.provides_modes[0].id;
		return s_Manifest.mode == "*" ? xr_string() : s_Manifest.mode;
	}

	bool ScreenRename(SScreen& d)
	{
		char buf[64];
		strncpy_s(buf, d.id.c_str(), _TRUNCATE);
		ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
		ImGui::InputText("##name", buf, sizeof(buf));
		if (!ImGui::IsItemDeactivatedAfterEdit()) return false;
		const xr_string wanted = Sanitize(buf);
		if (wanted == d.id || FindScreen(s_S, wanted)) return false;
		d.id = wanted;
		s_SelScreen = wanted;
		return true;
	}

	void DrawScreenDetails(SScreen& d)
	{
		xr_vector<xr_string> problems;
		ProblemsOf("screen", d.id, problems);
		for (const xr_string& p : problems)
			XUI::StatusText(p[0] == 'E' ? XUI::Tone::Error : XUI::Tone::Warning, p.c_str() + 5 + 8 + d.id.size());

		DrawPreview(d);
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(6.f)));
		DrawLayers(d);
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(6.f)));

		if (XUI::BeginPropertyGrid("screen", .28f))
		{
			if (XUI::PropertyRow("Music", "What plays with this screen. Any Track is the shuffle of this surface; pick tracks to tie them to the screen."))
				DrawMusicCombo(d);
			if (XUI::PropertySection("More", false))
			{
				if (XUI::PropertyRow("Name", "How this screen is called in the list and in other mods' eyes. Letters, digits and _."))
					if (ScreenRename(d)) Commit();
				if (XUI::PropertyRow("Show On"))
				{
					bool changed = MaskBox("Loading", d.show, scrLoading);
					ImGui::SameLine();
					changed |= MaskBox("Main Menu", d.show, scrMenu);
					ImGui::SameLine();
					changed |= MaskBox("Pause Menu", d.show, scrPause);
					if (changed) Commit();
				}
				if (XUI::PropertyRow("Levels", "Only while these levels load or run. Empty: everywhere."))
				{
					if (EditorWorld::LevelChips("levels", d.levels)) Commit();
				}
				const xr_string mode = ModuleMode();
				if (!mode.empty() && (d.show & (scrLoading | scrPause)) &&
					XUI::PropertyRow("Only In Its Mode", ("Shown only in games of this module's mode, " + mode + ". The main menu belongs to no game.").c_str(), XUI::PropInline))
				{
					bool only = !d.modes.empty();
					if (ImGui::Checkbox("##only_mode", &only))
					{
						d.modes.clear();
						if (only) d.modes.push_back(mode);
						Commit();
					}
				}
				if ((d.show & scrLoading) && XUI::PropertyRow("Show Over It", "What of the loading screen's interface stays on top of this screen."))
				{
					bool changed = MaskBox("Bar", d.hide, scrHideBar, true);
					ImGui::SameLine();
					changed |= MaskBox("Per Cent", d.hide, scrHidePercent, true);
					ImGui::SameLine();
					changed |= MaskBox("Stage", d.hide, scrHideStage, true);
					ImGui::SameLine();
					changed |= MaskBox("Tip", d.hide, scrHideTip, true);
					if (changed) Commit();
				}
				if (XUI::PropertyRow("Weight", "How often this screen comes up against the others. The game's own screen weighs 1."))
				{
					ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
					ImGui::DragFloat("##weight", &d.weight, .05f, 0.f, 100.f, "%.2f");
					if (ImGui::IsItemDeactivatedAfterEdit()) Commit();
				}
				XUI::EndPropertySection();
			}
			XUI::EndPropertyGrid();
		}
	}

	void DrawTrackDetails(STrack& t)
	{
		xr_vector<xr_string> problems;
		ProblemsOf("track", t.id, problems);
		for (const xr_string& p : problems)
			XUI::StatusText(p[0] == 'E' ? XUI::Tone::Error : XUI::Tone::Warning, p.c_str() + 5 + 7 + t.id.size());
		if (XUI::BeginPropertyGrid("track", .28f))
		{
			if (XUI::PropertyRow("File", "An .ogg of the project."))
				PickLayerFile(t.file, "##track_file");
			if (XUI::PropertyRow("Shuffle On", "Where this track is played at random. Leave all off to play it only with the screens that name it."))
			{
				bool changed = MaskBox("Loading", t.play, scrLoading);
				ImGui::SameLine();
				changed |= MaskBox("Main Menu", t.play, scrMenu);
				ImGui::SameLine();
				changed |= MaskBox("Pause Menu", t.play, scrPause);
				if (changed) Commit();
			}
			if (XUI::PropertyRow("Volume"))
			{
				float percent = t.volume * 100.f;
				ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
				if (ImGui::SliderFloat("##volume", &percent, 0.f, 100.f, "%.0f %%")) t.volume = percent * .01f;
				if (ImGui::IsItemDeactivatedAfterEdit()) Commit();
			}
			if (XUI::PropertyRow("Weight", "How often this track comes up against the others."))
			{
				ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
				ImGui::DragFloat("##weight", &t.weight, .05f, 0.f, 100.f, "%.2f");
				if (ImGui::IsItemDeactivatedAfterEdit()) Commit();
			}
			XUI::EndPropertyGrid();
		}
	}

	void DrawToolbar(XUI::Toolbar& tb)
	{
		static const char* surfaces[] = { "Loading", "Menu" };
		static const char* policy[] = { "Mix In", "Only Mine" };
		tb.Group(2);
		if (tb.Segmented("surface", nullptr, &s_Surface, surfaces, 2, "Which screens to edit"))
			s_SelLayer = -1;
		tb.Separator();
		tb.Group(1);
		int screens = s_S.only_screens[s_Surface] ? 1 : 0, music = s_S.only_music[s_Surface] ? 1 : 0;
		tb.Status("Screens");
		if (tb.Segmented("policy_screens", "Screens", &screens, policy, 2, "Mix In: yours come up among the game's and other mods'. Only Mine: nobody else's are shown."))
		{
			s_S.only_screens[s_Surface] = screens == 1;
			Commit();
		}
		tb.Status("Music");
		if (tb.Segmented("policy_music", "Music", &music, policy, 2, "Mix In: yours plays among the game's and other mods'. Only Mine: nobody else's is played, and no tracks of yours means silence."))
		{
			s_S.only_music[s_Surface] = music == 1;
			Commit();
		}
		tb.Group(0);
		tb.Separator();
		tb.Status(s_Status.empty() ? "Saved to mod.ltx as you edit" : s_Status.c_str(), s_Status.empty() ? XUI::Tone::Muted : XUI::Tone::Error);
	}

	void DrawBody()
	{
		if (!EditorProject::Active())
		{
			XUI::EmptyState(XUI::Icon::Picture, "No project", "Screens and music live in the mod.ltx of the open project.");
			return;
		}
		Refresh();
		if (!s_Have)
		{
			if (XUI::EmptyState(XUI::Icon::Picture, "No mod.ltx yet", "Screens and music live in the module's mod.ltx. Mod Settings makes it.", "Open Mod Settings"))
				EditorMod::RequestEditManifest();
			return;
		}

		const ImVec2 tab_from = ImGui::GetCursorScreenPos(), tab_size = ImGui::GetContentRegionAvail();
		ImGui::BeginChild("##screens_left", ImVec2(XUI::Pt(260.f), 0.f), ImGuiChildFlags_AlwaysUseWindowPadding);
		DrawScreensList();
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(12.f)));
		DrawMusicList();
		ImGui::EndChild();
		ImGui::SameLine();
		ImGui::BeginChild("##screens_right", ImVec2(0.f, 0.f), ImGuiChildFlags_AlwaysUseWindowPadding);
		{
			const float avail = ImGui::GetContentRegionAvail().x;
			const float column = _min(avail, XUI::Pt(640.f));
			ImGui::SetCursorPosX(ImGui::GetCursorPosX() + _max(0.f, (avail - column) * .5f));
			ImGui::BeginChild("##screens_column", ImVec2(column, 0.f), ImGuiChildFlags_AutoResizeY);
			if (SScreen* d = s_SelScreen.empty() ? nullptr : FindScreen(s_S, s_SelScreen))
				DrawScreenDetails(*d);
			else if (STrack* t = s_SelTrack.empty() ? nullptr : FindTrack(s_S, s_SelTrack))
				DrawTrackDetails(*t);
			else
				XUI::EmptyState(XUI::Icon::Picture, "Pick a screen or a track", "Or drop files anywhere on this tab.");
			ImGui::EndChild();
		}
		ImGui::EndChild();

		// what no list or layer took falls on the tab: sorted by what it is
		XUI::Strings files;
		if (XUI::FileDropZone("##drop_tab", tab_from, ImVec2(tab_from.x + tab_size.x, tab_from.y + tab_size.y), files))
			TakeDrop(files, xr_string(), -1, false);
	}

	void ShowTab(bool activate)
	{
		if (XUI::Shell::IsOpen("screens"))
		{
			if (activate) XUI::Shell::Activate("screens");
			return;
		}
		XUI::Shell::DocDesc d;
		d.id = "screens";
		d.kind = "screens";
		d.icon = XUI::Icon::Picture;
		d.label = [] { return xr_string("Screens & Music"); };
		d.tooltip = [] { return xr_string("The module's loading and menu screens and their music - saved to mod.ltx as you edit"); };
		d.alive = [] { return s_Open; };
		d.toolbar = [](XUI::Toolbar& tb) { DrawToolbar(tb); };
		d.body = [] { DrawBody(); };
		d.on_close = [] { s_Open = false; };
		XUI::Shell::Open(d, activate);
	}

	void JsonLayer(xr_string& out, const SScreenLayer& l)
	{
		const EFile kind = l.file.empty() ? fileNone : FileKind(l.file);
		out += "{\"file\":";
		NqUtil::JsonString(out, l.file);
		out += ",\"kind\":";
		NqUtil::JsonString(out, KindLabel(kind));
		out += ",\"fit\":";
		NqUtil::JsonString(out, l.fit == scrContain ? "contain" : l.fit == scrStretch ? "stretch" : "cover");
		out += ",\"opacity\":";
		NqUtil::JsonNumber(out, l.opacity);
		out += ",\"rect\":[";
		NqUtil::JsonNumber(out, l.x); out += ",";
		NqUtil::JsonNumber(out, l.y); out += ",";
		NqUtil::JsonNumber(out, l.w); out += ",";
		NqUtil::JsonNumber(out, l.h);
		out += "],\"texture\":";
		NqUtil::JsonString(out, l.texture);
		out += ",\"sound\":";
		NqUtil::JsonBool(out, l.sound);
		out += "}";
	}

	xr_string MaskText(u32 mask, const std::initializer_list<std::pair<u32, LPCSTR>>& names)
	{
		xr_string out;
		for (const auto& it : names)
			if (mask & it.first) out += (out.empty() ? "" : ",") + xr_string(it.second);
		return out;
	}

	xr_string SurfacesText(u32 mask) { return MaskText(mask, { { u32(scrLoading), "loading" }, { u32(scrMenu), "menu" }, { u32(scrPause), "pause" } }); }

	void Answer(const SManifest& m, xr_string& out)
	{
		const SScreens& s = m.screens;
		out = "{\"ok\":true,\"policy\":{\"loading\":{\"screens\":";
		NqUtil::JsonString(out, s.only_screens[0] ? "only" : "mix");
		out += ",\"music\":";
		NqUtil::JsonString(out, s.only_music[0] ? "only" : "mix");
		out += ",\"end\":";
		NqUtil::JsonString(out, s.next_track[0] ? "next" : "loop");
		out += "},\"menu\":{\"screens\":";
		NqUtil::JsonString(out, s.only_screens[1] ? "only" : "mix");
		out += ",\"music\":";
		NqUtil::JsonString(out, s.only_music[1] ? "only" : "mix");
		out += ",\"end\":";
		NqUtil::JsonString(out, s.next_track[1] ? "next" : "loop");
		out += "}},\"screens\":[";
		for (size_t i = 0; i < s.screens.size(); ++i)
		{
			const SScreen& d = s.screens[i];
			out += i ? ",{\"id\":" : "{\"id\":";
			NqUtil::JsonString(out, d.id);
			out += ",\"show\":";
			NqUtil::JsonString(out, SurfacesText(d.show));
			out += ",\"levels\":";
			NqUtil::JsonStringArray(out, d.levels);
			out += ",\"modes\":";
			NqUtil::JsonStringArray(out, d.modes);
			out += ",\"weight\":";
			NqUtil::JsonNumber(out, d.weight);
			out += ",\"music\":";
			NqUtil::JsonString(out, d.music);
			out += ",\"tracks\":";
			NqUtil::JsonStringArray(out, d.tracks);
			out += ",\"hide\":";
			NqUtil::JsonString(out, MaskText(d.hide, { { u32(scrHideBar), "bar" }, { u32(scrHidePercent), "percent" }, { u32(scrHideStage), "stage" }, { u32(scrHideTip), "tip" } }));
			out += ",\"layers\":[";
			for (size_t k = 0; k < d.layers.size(); ++k)
			{
				if (k) out += ",";
				JsonLayer(out, d.layers[k]);
			}
			out += "]}";
		}
		out += "],\"tracks\":[";
		for (size_t i = 0; i < s.tracks.size(); ++i)
		{
			const STrack& t = s.tracks[i];
			out += i ? ",{\"id\":" : "{\"id\":";
			NqUtil::JsonString(out, t.id);
			out += ",\"file\":";
			NqUtil::JsonString(out, t.file);
			out += ",\"play\":";
			NqUtil::JsonString(out, SurfacesText(t.play));
			out += ",\"weight\":";
			NqUtil::JsonNumber(out, t.weight);
			out += ",\"volume\":";
			NqUtil::JsonNumber(out, t.volume);
			out += "}";
		}
		xr_vector<xr_string> problems;
		EditorScreens::Validate(EditorProject::Root(), m, problems);
		out += "],\"problems\":";
		NqUtil::JsonStringArray(out, problems);
		out += "}";
	}

	u32 ParseSurfaces(const xr_string& text)
	{
		u32 mask = 0;
		if (text.find("loading") != xr_string::npos)	mask |= scrLoading;
		if (text.find("menu") != xr_string::npos)		mask |= scrMenu;
		if (text.find("pause") != xr_string::npos)		mask |= scrPause;
		return mask;
	}

	void ArgListInto(LPCSTR raw, LPCSTR field, xr_vector<xr_string>& to)
	{
		xr_string text;
		if (!NqUtil::ArgString(raw, field, text)) return;
		to.clear();
		NqUtil::Split(text.c_str(), ',', to);
	}

	void ApplyLayerArgs(LPCSTR raw, SScreenLayer& l, xr_string& err)
	{
		xr_string v;
		if (NqUtil::ArgString(raw, "file", v))
		{
			if (v.empty()) l.file.clear();
			else
			{
				xr_string rel;
				if (Import(v, rel, err)) l.file = rel;
			}
		}
		if (NqUtil::ArgString(raw, "texture", v))
		{
			if (v.empty()) l.texture.clear();
			else
			{
				xr_string rel;
				if (Import(v, rel, err)) l.texture = rel;
			}
		}
		if (NqUtil::ArgString(raw, "fit", v))
			l.fit = v == "contain" ? scrContain : v == "stretch" ? scrStretch : scrCover;
		float f;
		if (NqUtil::ArgFloat(raw, "opacity", f))	l.opacity = _max(0.f, _min(f, 1.f));
		if (NqUtil::ArgString(raw, "rect", v))		sscanf_s(v.c_str(), "%f %f %f %f", &l.x, &l.y, &l.w, &l.h);
		if (NqUtil::ArgBool(raw, "sound", false) == NqUtil::ArgBool(raw, "sound", true))
			l.sound = NqUtil::ArgBool(raw, "sound", true);
	}

	void ApplyScreenArgs(LPCSTR raw, SScreen& d, SScreens& s, xr_string& err)
	{
		xr_string v;
		if (NqUtil::ArgString(raw, "show", v))		d.show = ParseSurfaces(v);
		ArgListInto(raw, "levels", d.levels);
		ArgListInto(raw, "modes", d.modes);
		float f;
		if (NqUtil::ArgFloat(raw, "weight", f))		d.weight = _max(0.f, f);
		if (NqUtil::ArgString(raw, "music", v))
		{
			d.tracks.clear();
			if (v.empty() || v == "any" || v == "none" || v == "video")	d.music = v.empty() ? "any" : v;
			else
			{
				d.music = "tracks";
				NqUtil::Split(v.c_str(), ',', d.tracks);
			}
		}
		if (NqUtil::ArgString(raw, "hide", v))
		{
			d.hide = 0;
			if (v.find("bar") != xr_string::npos)		d.hide |= scrHideBar;
			if (v.find("percent") != xr_string::npos)	d.hide |= scrHidePercent;
			if (v.find("stage") != xr_string::npos)		d.hide |= scrHideStage;
			if (v.find("tip") != xr_string::npos)		d.hide |= scrHideTip;
		}
		if (NqUtil::ArgString(raw, "new_id", v) && Sanitize(v) != d.id)
		{
			const xr_string wanted = Sanitize(v);
			if (FindScreen(s, wanted))	err = "a screen '" + wanted + "' already exists";
			else						d.id = wanted;
		}
	}

	void ApplyTrackArgs(LPCSTR raw, STrack& t, xr_string& err)
	{
		xr_string v;
		if (NqUtil::ArgString(raw, "file", v))
		{
			xr_string rel;
			if (Import(v, rel, err)) t.file = rel;
		}
		if (NqUtil::ArgString(raw, "play", v))		t.play = ParseSurfaces(v);
		float f;
		if (NqUtil::ArgFloat(raw, "weight", f))		t.weight = _max(0.f, f);
		if (NqUtil::ArgFloat(raw, "volume", f))		t.volume = _max(0.f, _min(f, 1.f));
	}
}

void EditorScreens::Validate(LPCSTR project_root, const SManifest& m, xr_vector<xr_string>& problems)
{
	const xr_string root = project_root;
	const auto exists = [&](const xr_string& rel) { return NqUtil::FileExists((root + "\\" + rel).c_str()); };
	const SScreens& s = m.screens;

	for (const SScreen& d : s.screens)
	{
		const xr_string tag = "screen " + d.id + ": ";
		if (d.layers.empty())
			problems.push_back("W406 " + tag + "it has no layers, so it is never shown");
		bool video = false;
		for (size_t i = 0; i < d.layers.size(); ++i)
		{
			const SScreenLayer& l = d.layers[i];
			const xr_string at = "layer " + xr_string(std::to_string(i + 1).c_str());
			if (l.file.empty())
			{
				problems.push_back("W405 " + tag + at + " is empty and is not drawn");
				continue;
			}
			const EFile kind = FileKind(l.file);
			video |= kind == fileVideo;
			if (kind == fileNone || kind == fileMusic || kind == fileConvert)
				problems.push_back("E402 " + tag + at + ": the game shows no " + NqUtil::Extension(l.file.c_str()) + " file");
			else if (!exists(l.file))
				problems.push_back("E401 " + tag + at + ": the file does not exist: " + l.file);
			else if (EditorMod::OutsideBuild(project_root, l.file.c_str()))
				problems.push_back("E408 " + tag + at + ": the build leaves this file out: " + l.file);
			if (kind == fileShader && exists(l.file) && !exists(l.file.substr(0, l.file.rfind('.')) + ".ps"))
				problems.push_back("E404 " + tag + at + ": there is no .ps beside this shader");
			if (kind == fileShader && NqUtil::Extension(l.file.c_str()) == ".ps" && !exists(l.file.substr(0, l.file.rfind('.')) + ".s"))
				problems.push_back("E404 " + tag + at + ": there is no .s beside this shader");
			if (!l.texture.empty() && !exists(l.texture))
				problems.push_back("E401 " + tag + at + ": the shader's texture does not exist: " + l.texture);
		}
		if (d.music == "tracks")
			for (const xr_string& t : d.tracks)
				if (!FindTrack(const_cast<SScreens&>(s), t))
					problems.push_back("E403 " + tag + "it names a track the module does not have: " + t);
		if (d.music == "video" && !video)
			problems.push_back("W411 " + tag + "it asks for a video's sound and has no video");
		if (EditorWorld::Available())
			for (const xr_string& level : d.levels)
				if (!EditorWorld::LevelExists(level.c_str()))
					problems.push_back("W407 " + tag + "the linked game has no level " + level);
	}

	for (const STrack& t : s.tracks)
	{
		const xr_string tag = "track " + t.id + ": ";
		if (NqUtil::Extension(t.file.c_str()) != ".ogg")
			problems.push_back("E409 " + tag + "the game plays .ogg files only: " + t.file);
		else if (!exists(t.file))
			problems.push_back("E401 " + tag + "the file does not exist: " + t.file);
		else if (EditorMod::OutsideBuild(project_root, t.file.c_str()))
			problems.push_back("E408 " + tag + "the build leaves this file out: " + t.file);
		bool named = false;
		for (const SScreen& d : s.screens)
			named |= d.music == "tracks" && std::find(d.tracks.begin(), d.tracks.end(), t.id) != d.tracks.end();
		if (!t.play && !named)
			problems.push_back("W410 " + tag + "it joins no shuffle and no screen names it, so it never plays");
	}
}

void EditorScreens::Show()		{ s_Open = true; ShowTab(true); }
bool EditorScreens::IsOpen()	{ return s_Open; }

void EditorScreens::DrawUI()
{
	if (s_Open) ShowTab(false);
}

void EditorScreens::McpGet(LPCSTR, xr_string& out)
{
	SManifest m;
	if (!EditorProject::Active())						{ NqUtil::JsonError(out, "no active project"); return; }
	if (!EditorMod::Load(EditorProject::Root(), m))		{ NqUtil::JsonError(out, "mod.ltx not found in the project"); return; }
	Answer(m, out);
}

void EditorScreens::McpSet(LPCSTR raw, xr_string& out)
{
	SManifest m;
	if (!EditorProject::Active())						{ NqUtil::JsonError(out, "no active project"); return; }
	if (!EditorMod::Load(EditorProject::Root(), m))		{ NqUtil::JsonError(out, "mod.ltx not found in the project"); return; }
	SScreens& s = m.screens;

	xr_string action, id, err;
	NqUtil::ArgString(raw, "action", action);
	NqUtil::ArgString(raw, "id", id);

	const auto need_screen = [&]() -> SScreen*
	{
		SScreen* d = FindScreen(s, id);
		if (!d) err = "no screen '" + id + "'";
		return d;
	};

	if (action == "policy")
	{
		xr_string surface, v;
		NqUtil::ArgString(raw, "surface", surface);
		if (surface != "loading" && surface != "menu") { NqUtil::JsonError(out, "surface must be loading or menu"); return; }
		const int side = surface == "loading" ? 0 : 1;
		if (NqUtil::ArgString(raw, "screens", v))	s.only_screens[side] = v == "only";
		if (NqUtil::ArgString(raw, "music", v))		s.only_music[side] = v == "only";
		if (NqUtil::ArgString(raw, "end", v))		s.next_track[side] = v == "next";
	}
	else if (action == "screen_add")
	{
		SScreen d;
		d.id = NewScreenId(s, id.empty() ? xr_string("screen") : id);
		xr_string v;
		ApplyScreenArgs(raw, d, s, err);
		if (NqUtil::ArgString(raw, "file", v) && !v.empty())
		{
			SScreenLayer l;
			ApplyLayerArgs(raw, l, err);
			d.layers.push_back(l);
		}
		s.screens.push_back(d);
	}
	else if (action == "screen_set")
	{
		if (SScreen* d = need_screen()) ApplyScreenArgs(raw, *d, s, err);
	}
	else if (action == "screen_remove")
	{
		if (need_screen())
			s.screens.erase(std::remove_if(s.screens.begin(), s.screens.end(), [&](const SScreen& d) { return d.id == id; }), s.screens.end());
	}
	else if (action == "layer_add")
	{
		if (SScreen* d = need_screen())
		{
			SScreenLayer l;
			ApplyLayerArgs(raw, l, err);
			const int index = NqUtil::ArgInt(raw, "index", int(d->layers.size()));
			d->layers.insert(d->layers.begin() + _max(0, _min(index, int(d->layers.size()))), l);
		}
	}
	else if (action == "layer_set" || action == "layer_remove" || action == "layer_move")
	{
		if (SScreen* d = need_screen())
		{
			const int index = NqUtil::ArgInt(raw, "index", -1);
			if (index < 0 || index >= int(d->layers.size()))	err = "no layer at that index (0 is the bottom layer)";
			else if (action == "layer_set")						ApplyLayerArgs(raw, d->layers[index], err);
			else if (action == "layer_remove")					d->layers.erase(d->layers.begin() + index);
			else
			{
				const int to = _max(0, _min(NqUtil::ArgInt(raw, "to", index), int(d->layers.size()) - 1));
				const SScreenLayer moved = d->layers[index];
				d->layers.erase(d->layers.begin() + index);
				d->layers.insert(d->layers.begin() + to, moved);
			}
		}
	}
	else if (action == "track_add")
	{
		STrack t;
		t.id = NewTrackId(s, id.empty() ? xr_string("track") : id);
		ApplyTrackArgs(raw, t, err);
		if (t.file.empty() && err.empty()) err = "a track needs a file";
		if (err.empty()) s.tracks.push_back(t);
	}
	else if (action == "track_set")
	{
		if (STrack* t = FindTrack(s, id))	ApplyTrackArgs(raw, *t, err);
		else								err = "no track '" + id + "'";
	}
	else if (action == "track_remove")
	{
		s.tracks.erase(std::remove_if(s.tracks.begin(), s.tracks.end(), [&](const STrack& t) { return t.id == id; }), s.tracks.end());
		for (SScreen& d : s.screens)
		{
			d.tracks.erase(std::remove(d.tracks.begin(), d.tracks.end(), id), d.tracks.end());
			if (d.music == "tracks" && d.tracks.empty()) d.music = "any";
		}
	}
	else if (action == "drop")
	{
		// the path of a drop in the tab: files separated by |; on a screen, a layer of it (index), the music, or the tab
		xr_string list;
		NqUtil::ArgString(raw, "files", list);
		XUI::Strings files;
		NqUtil::Split(list.c_str(), '|', files);
		TakeFiles(s, files, id, NqUtil::ArgInt(raw, "index", -1), NqUtil::ArgBool(raw, "music", false), err);
	}
	else
	{
		NqUtil::JsonError(out, "action must be policy, screen_add, screen_set, screen_remove, layer_add, layer_set, layer_remove, layer_move, track_add, track_set, track_remove or drop");
		return;
	}

	if (!err.empty()) { NqUtil::JsonError(out, err); return; }
	if (!EditorMod::Save(EditorProject::Root(), m)) { NqUtil::JsonError(out, "can't write mod.ltx"); return; }
	Answer(m, out);
}

void EditorScreens::McpView(LPCSTR raw, xr_string& out)
{
	if (NqUtil::ArgBool(raw, "open", false)) Show();
	xr_string v;
	if (NqUtil::ArgString(raw, "surface", v)) s_Surface = v == "menu" ? 1 : 0;
	if (NqUtil::ArgString(raw, "screen", v)) { s_SelScreen = v; s_SelTrack.clear(); s_SelLayer = NqUtil::ArgInt(raw, "layer", -1); }
	if (NqUtil::ArgString(raw, "track", v)) { s_SelTrack = v; s_SelScreen.clear(); s_SelLayer = -1; }
	// the same way a drop from Explorer lands: at a point of the editor's window
	if (NqUtil::ArgString(raw, "simulate_drop", v))
	{
		XUI::Strings files;
		NqUtil::Split(v.c_str(), '|', files);
		float x = 0.f, y = 0.f;
		NqUtil::ArgFloat(raw, "x", x);
		NqUtil::ArgFloat(raw, "y", y);
		XUI::FileDropSimulate(files, x, y);
	}
	out = "{\"ok\":true,\"tab\":\"screens\",\"open\":";
	NqUtil::JsonBool(out, s_Open);
	out += ",\"active\":";
	NqUtil::JsonBool(out, XUI::Shell::IsActive("screens"));
	out += ",\"surface\":";
	NqUtil::JsonString(out, s_Surface ? "menu" : "loading");
	out += ",\"screen\":";
	NqUtil::JsonString(out, s_SelScreen);
	out += ",\"track\":";
	NqUtil::JsonString(out, s_SelTrack);
	out += ",\"layer\":";
	NqUtil::JsonInt(out, s_SelLayer);
	out += ",\"status\":";
	NqUtil::JsonString(out, s_Status);
	out += "}";
}
