//---------------------------------------------------------------------------
#include "stdafx.h"
#include "UIImageEditorForm.h"
#include "EThumbnail.h"
#include "EDX11Utils.h"
#include "ui_main.h"
#include "Nq/NqUtil.h"
UIImageEditorForm* UIImageEditorForm::Form = nullptr;

namespace
{
	using XFinedTheme::ColorToken;

	ImU32 Tok(ColorToken token, float alpha = 1.f) { return XFinedTheme::U32(token, alpha); }

	// pt: list | preview | properties (docs/UI_DESIGN.md, section 6, Libraries)
	constexpr float kListW		= 260.f;
	constexpr float kGridW		= 320.f;
	constexpr float kSideMin	= 220.f;
	constexpr float kCanvasMin	= 400.f;
	constexpr float kImageMax	= 512.f;
}

UIImageEditorForm::UIImageEditorForm()
{
	m_ItemProps = xr_new<UIPropertiesForm>();
	m_ItemProps->SetModifiedEvent(TOnModifiedEvent(this, &UIImageEditorForm::OnPropModified));
	m_Texture = nullptr;
	m_TextureDeadFrame = -1;
	m_bFilterImage = true;
	m_bFilterCube = true;
	m_bFilterBump = true;
	m_bFilterNormal = true;
	m_bFilterTerrain = true;
	bImportMode = false;
	bReadonlyMode = false;
	m_UpdateQueued = false;
	m_CloseAfterUpdate = false;
	m_DeleteAfterUpdate = false;
	m_Discarded = false;
	m_ItemModified = false;
	m_ReturnToLibrary = false;
}

UIImageEditorForm::~UIImageEditorForm()
{
	if (m_Texture)
		m_Texture->Release();
	for (ImTextureID dead : m_TextureDead)
		dead->Release();
	for (ListItem* item : m_List)
		xr_delete(item);
	for (THMMapIt it = m_THM_Used.begin(); it != m_THM_Used.end(); ++it)
		xr_delete(it->second);
	xr_delete(m_ItemProps);
}

void UIImageEditorForm::Draw()
{
	if (m_CloseAfterUpdate)
	{
		m_CloseAfterUpdate = false;
		HideLib();
		return;
	}
	if (!m_TextureDead.empty() && ImGui::GetFrameCount() > m_TextureDeadFrame)
	{
		for (ImTextureID dead : m_TextureDead)
			dead->Release();
		m_TextureDead.clear();
	}

	// list | preview | the description's properties; the list and the properties keep their pt size
	const ImGuiChildFlags pane = ImGuiChildFlags_AlwaysUseWindowPadding;
	float list_w = kListW;
	XUI::Splitter("texture_library.list", true, &list_w, kSideMin, kCanvasMin + kSideMin + 8.f, kListW);
	ImGui::BeginDisabled(m_UpdateQueued);
	if (ImGui::BeginChild("##list", ImVec2(XUI::Pt(list_w), 0.f), pane))
	{
		// an empty library says where its textures come from; a full one the Types filter emptied says that
		const bool none = m_List.empty();
		xr_string where;
		if (none)
			where = bImportMode ? NqUtil::Format("Nothing in %s is new to the library.", LibraryFolderText(_import_).c_str())
				: NqUtil::Format("The library lists the .tga images in %s. To add one, put the image into %s and check for new textures.",
					LibraryFolderText(_textures_).c_str(), LibraryFolderText(_import_).c_str());
		const int picked = m_ListView.Draw("texture_library", m_List, m_Selected.c_str(),
			bImportMode ? "No new textures" : none ? "No textures" : "No textures of these kinds",
			none ? where.c_str() : "The Types filter hides every texture in the library.",
			none && !bImportMode && !bReadonlyMode ? "Check New Textures" : nullptr);
		// Check New Textures runs outside the frame: it may turn this tab into New Textures
		if (m_ListView.acted)
			ExecCommand(COMMAND_CHECK_TEXTURES);
		if (picked >= 0)
			Select(picked);
	}
	ImGui::EndChild();
	ImGui::SameLine();
	if (ImGui::BeginChild("##rest", ImVec2(0.f, 0.f)))
	{
		float grid_w = kGridW;
		XUI::SplitterTail("texture_library.details", true, &grid_w, kCanvasMin, kSideMin, kGridW);
		const float head = _max(1.f, ImGui::GetContentRegionAvail().x - XUI::Pt(grid_w) - ImGui::GetStyle().ItemSpacing.x);
		ImGui::PushStyleColor(ImGuiCol_ChildBg, Tok(ColorToken::Canvas));
		const bool shown = ImGui::BeginChild("##preview", ImVec2(head, 0.f), pane);
		ImGui::PopStyleColor();
		if (shown)
			DrawPreview();
		ImGui::EndChild();
		ImGui::SameLine();
		if (ImGui::BeginChild("##details", ImVec2(0.f, 0.f), pane))
		{
			if (!m_THM_Current.empty())
				m_ItemProps->Draw();
			else
				XUI::EmptyState(XUI::Icon::Settings, "No properties", "Pick a texture to edit its description.");
		}
		ImGui::EndChild();
	}
	ImGui::EndChild();
	ImGui::EndDisabled();
}

void UIImageEditorForm::DrawToolbar(XUI::Toolbar& tb)
{
	const bool dirty = Dirty();
	const bool on = !bReadonlyMode && !m_UpdateQueued && dirty;
	const char* tip = bImportMode ? "Import the new textures into the library and build their game textures"
		: "Save the changed descriptions and rebuild their game textures";
	const char* why = bReadonlyMode ? "The texture folder cannot be written" : m_UpdateQueued ? "The library is saving"
		: bImportMode ? "There is nothing to import" : "No unsaved changes";
	tb.Group(1);
	if (tb.Action("save", bImportMode ? XUI::Icon::Plus : XUI::Icon::Save, bImportMode ? "Import" : "Save", on ? tip : why, on))
	{
		xr_string err;
		if (!QueueSave(err))
			XUI::Notice(XUI::Tone::Error, "Save failed", err.c_str());
	}
	if (!bImportMode)
	{
		tb.Group(0);
		if (tb.Menu("types", XUI::Icon::Filter, "Types", "The kinds of texture the list shows"))
		{
			bool changed = false;
			changed |= ImGui::MenuItem("Images", nullptr, &m_bFilterImage);
			changed |= ImGui::MenuItem("Cube Maps", nullptr, &m_bFilterCube);
			changed |= ImGui::MenuItem("Bump Maps", nullptr, &m_bFilterBump);
			changed |= ImGui::MenuItem("Normal Maps", nullptr, &m_bFilterNormal);
			changed |= ImGui::MenuItem("Terrain", nullptr, &m_bFilterTerrain);
			if (changed)
				FilterUpdate();
			ImGui::EndPopup();
		}
	}
	tb.Group(2);
	if (bReadonlyMode)
		tb.Status("Read-only", XUI::Tone::Warning);
	else if (m_UpdateQueued)
		tb.Status("Saving...", XUI::Tone::Info);
}

void UIImageEditorForm::DrawStatus()
{
	u32 changed = u32(modif_map.size());
	if (m_ItemModified)
		++changed;
	xr_string text = NqUtil::Format("%u textures", u32(m_List.size()));
	if (m_ListView.shown.size() != m_List.size())
		text += NqUtil::Format(" \xC2\xB7 %u shown", u32(m_ListView.shown.size()));
	if (changed && !bImportMode)
		text += NqUtil::Format(" \xC2\xB7 %u changed", changed);
	ImGui::TextUnformatted(text.c_str());
}

void UIImageEditorForm::DrawPreview()
{
	ETextureThumbnail* thm = m_THM_Current.empty() ? nullptr : m_THM_Current.back();
	if (!thm)
	{
		XUI::EmptyState(XUI::Icon::FileTexture, "No texture selected", "Pick a texture in the list to see its description.");
		return;
	}
	{
		XUI::FontScope title(XUI::Font::Title);
		XUI::TextEllipsis(NqUtil::BaseName(thm->SrcName()).c_str());
	}
	{
		XUI::FontScope small_font(XUI::Font::Small);
		ImGui::PushStyleColor(ImGuiCol_Text, Tok(ColorToken::TextMuted));
		XUI::TextEllipsis(thm->SrcName(), -1.f, XUI::Clip::Start);
		const xr_string facts = NqUtil::Format("%u x %u \xC2\xB7 %s%s", thm->_Width(), thm->_Height(), thm->FormatString(),
			thm->_Alpha() ? " \xC2\xB7 alpha" : "");
		XUI::TextEllipsis(facts.c_str());
		ImGui::PopStyleColor();
	}
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(8.f)));
	if (!m_Texture)
	{
		XUI::StatusText(XUI::Tone::Warning, "No preview: the texture file was not found.");
		return;
	}
	// the thumbnail at the texture's own aspect, as big as the pane allows up to 512 pt, on a checkerboard for alpha
	const ImVec2 avail = ImGui::GetContentRegionAvail();
	const float tw = float(_max(1u, thm->_Width())), th = float(_max(1u, thm->_Height()));
	const float scale = _min(_min(avail.x / tw, avail.y / th), XUI::Pt(kImageMax) / _max(tw, th));
	if (scale <= 0.f)
		return;
	const ImVec2 size(tw * scale, th * scale);
	const ImVec2 a(ImGui::GetCursorScreenPos().x + (avail.x - size.x) * .5f, ImGui::GetCursorScreenPos().y);
	const ImVec2 b(a.x + size.x, a.y + size.y);
	ImDrawList* dl = ImGui::GetWindowDrawList();
	if (thm->_Alpha())
	{
		const float cell = XUI::Pt(8.f);
		dl->AddRectFilled(a, b, Tok(ColorToken::Input));
		dl->PushClipRect(a, b, true);
		for (float y = a.y, row = 0.f; y < b.y; y += cell, row += 1.f)
			for (float x = a.x + (int(row) & 1 ? cell : 0.f); x < b.x; x += cell * 2.f)
				dl->AddRectFilled(ImVec2(x, y), ImVec2(x + cell, y + cell), Tok(ColorToken::Hover));
		dl->PopClipRect();
	}
	dl->AddImage(m_Texture, a, b);
	dl->AddRect(a, b, Tok(ColorToken::BorderSubtle));
	ImGui::Dummy(ImVec2(avail.x, size.y));
}

void UIImageEditorForm::Update()
{
}

void UIImageEditorForm::Show(bool bImport)
{
	if (Form && Form->m_DeleteAfterUpdate)
	{
		XUI::Notice(XUI::Tone::Info, "Texture Library", "The library is still saving its changes. Open it again when that is done.");
		return;
	}
	if (Form && XUI::Shell::IsOpen("texture_library"))
	{
		XUI::Shell::Activate("texture_library");
		return;
	}
	if (!Form)
		Form = xr_new<UIImageEditorForm>();
	Present(bImport);
}

void UIImageEditorForm::Present(bool bImport)
{
	Form->bImportMode = bImport;
	Form->bReadonlyMode = !FS.can_write_to_alias(_textures_);
	if (Form->bReadonlyMode)
	{
		Log("#!The texture folder cannot be written: texture descriptions are read-only.");
		Form->m_ItemProps->SetReadOnly(TRUE);
	}
	Form->modif_map.clear();
	Form->InitItemList();
	// the Types filter belongs to the library; New Textures shows everything it found
	if (!bImport)
		Form->FilterUpdate();

	XUI::Shell::DocDesc d;
	d.id = "texture_library";
	d.kind = "texture_library";
	d.icon = XUI::Icon::FileTexture;
	d.padded = false;
	d.label = [] { return xr_string(Form && Form->bImportMode ? "New Textures" : "Texture Library"); };
	d.tooltip = [] { return xr_string(Form && Form->bImportMode ? "Textures found in the import folder" : "The texture descriptions of the editor's library"); };
	d.alive = [] { return Form && !Form->IsClosed() && !Form->m_DeleteAfterUpdate; };
	d.dirty = [] { return Form && !Form->m_Discarded && Form->Dirty(); };
	d.toolbar = [](XUI::Toolbar& tb) { if (Form) Form->DrawToolbar(tb); };
	d.body = [] { if (Form) Form->Draw(); };
	d.status = [] { if (Form) Form->DrawStatus(); };
	// the save runs deferred, outside the frame, with its progress; the form outlives its tab until it is through
	d.save = [](xr_string& err) { return Form && Form->QueueSave(err); };
	d.discard = [] { if (Form) Form->Revert(); };
	d.on_close = [] { if (Form && Form->m_UpdateQueued) Form->m_DeleteAfterUpdate = true; else xr_delete(Form); };
	XUI::Shell::Open(d, true);
}

// Runs outside the frame (a deferred command): it may turn the library's tab into New Textures in place.
void UIImageEditorForm::ImportTextures()
{
	if (Form && (Form->m_UpdateQueued || Form->m_DeleteAfterUpdate))
	{
		XUI::Notice(XUI::Tone::Info, "Check New Textures", "The Texture Library is still saving its changes. Check again when that is done.");
		return;
	}
	// New Textures is up already
	if (Form && Form->bImportMode)
	{
		XUI::Shell::Activate("texture_library");
		return;
	}
	FS_FileSet TextureMap;
	const int new_cnt = ImageLib.GetLocalNewTextures(TextureMap);
	if (!new_cnt)
	{
		XUI::Notice(XUI::Tone::Info, "Check New Textures", NqUtil::Format("Nothing in %s is new to the library.",
			LibraryFolderText(_import_).c_str()).c_str());
		return;
	}
	// the library and New Textures share one tab: the library gives way unless it holds unsaved changes
	if (Form)
	{
		if (Form->Dirty())
		{
			XUI::Shell::Activate("texture_library");
			XUI::Notice(XUI::Tone::Warning, "Check New Textures", "Save the Texture Library's changes, or close it without saving, then check again.");
			return;
		}
		Form->ClearItems();
		// the library's search would hide what was found
		Form->m_ListView.search[0] = 0;
		Form->m_ReturnToLibrary = true;
	}
	else
		Form = xr_new<UIImageEditorForm>();
	Form->texture_map.swap(TextureMap);
	Present(true);
}

ETextureThumbnail* UIImageEditorForm::FindUsedTHM(const shared_str& name)
{
	THMMapIt it = m_THM_Used.find(name);
	if (it != m_THM_Used.end())
		return it->second;

	ETextureThumbnail* thm = xr_new<ETextureThumbnail>(name.c_str(), false);
	m_THM_Used[name] = thm;

	if (bImportMode)
	{
		xr_string fn = name.c_str();
		ImageLib.UpdateFileName(fn);

		if (!thm->Load(name.c_str(), _import_))
		{
			bool bLoad = thm->Load(fn.c_str(), _game_textures_);
			ImageLib.CreateTextureThumbnail(thm, name.c_str(), _import_, !bLoad);
		}
	}
	else
	{
		thm->Load();
	}
	return thm;
}

void UIImageEditorForm::RegisterModifiedTHM()
{
	if (m_ItemModified || bImportMode)
	{
		for (THMIt t_it = m_THM_Current.begin(); t_it != m_THM_Current.end(); ++t_it)
		{
			FS_FileSetIt it = texture_map.find(FS_File((*t_it)->SrcName()));
			R_ASSERT(it != texture_map.end());
			modif_map.insert(*it);
		}
	}
	m_ItemModified = false;
}

void UIImageEditorForm::OnPropModified()
{
	m_ItemModified = true;
	m_Discarded = false;
}

bool UIImageEditorForm::Dirty()
{
	return bImportMode ? !texture_map.empty() : (!modif_map.empty() || m_ItemModified);
}

void UIImageEditorForm::Revert()
{
	// New Textures: nothing is imported, the tab goes; the library: every description is read again
	if (bImportMode)
	{
		m_Discarded = true;
		return;
	}
	const xr_string keep = m_Selected;
	ClearItems();
	InitItemList();
	FilterUpdate();
	const int again = m_ListView.Find(m_List, keep.c_str());
	if (again >= 0 && 0 == _stricmp(m_List[again]->Key(), keep.c_str()))
		Select(again);
}

void UIImageEditorForm::ClearItems()
{
	// the edits of the description on show are dropped with the rest: nothing is registered as modified on the way
	m_ItemModified = false;
	m_THM_Current.clear();
	OnItemsFocused(nullptr);
	for (ListItem* item : m_List)
		xr_delete(item);
	m_List.clear();
	for (THMMapIt it = m_THM_Used.begin(); it != m_THM_Used.end(); ++it)
		xr_delete(it->second);
	m_THM_Used.clear();
	texture_map.clear();
	modif_map.clear();
	m_ListView.Invalidate();
}

bool UIImageEditorForm::QueueSave(xr_string& err)
{
	if (bReadonlyMode)
	{
		err = "The texture folder cannot be written.";
		return false;
	}
	if (m_UpdateQueued)
		return true;
	RegisterModifiedTHM();
	m_UpdateQueued = UI->DeferUIWork(&UIImageEditorForm::UpdateLibDeferred);
	if (!m_UpdateQueued)
		err = "The save could not be started.";
	return m_UpdateQueued;
}

void UIImageEditorForm::OnCubeMapBtnClick(ButtonValue* value, bool& bModif, bool& bSafe)
{
	ButtonValue* B = dynamic_cast<ButtonValue*>(value); R_ASSERT(B);
	bModif = false;
	if (B->btn_num == 0 && !m_Selected.empty())
	{
		xr_string new_name = m_Selected + "#small";
		ImageLib.CreateSmallerCubeMap(m_Selected.c_str(), new_name.c_str());
	}
}

void UIImageEditorForm::OnTypeChange(PropValue* prop)
{
}

void UIImageEditorForm::InitItemList()
{
	R_ASSERT(m_THM_Used.empty());
	if (!bImportMode)
		ImageLib.GetTexturesRaw(texture_map);

	for (ListItem* item : m_List)
		xr_delete(item);
	m_List.clear();
	for (FS_FileSetIt it = texture_map.begin(); it != texture_map.end(); ++it)
	{
		ListItem* I = LHelper().CreateItem(m_List, it->name.c_str(), 0);
		I->m_Object = (void*)(FindUsedTHM(it->name.c_str()));
		R_ASSERT2(I->m_Object, it->name.c_str());
	}
	m_ListView.Invalidate();
}

void UIImageEditorForm::HideLib()
{
	bOpen = false;
}

void UIImageEditorForm::UpdateLibDeferred()
{
	if (!Form)
		return;

	SProgressOperation progress(*UI, true);
	const bool completed = Form->UpdateLib();
	Form->m_UpdateQueued = false;
	// its tab closed while the save was waiting: nothing shows the form any more
	if (Form->m_DeleteAfterUpdate)
	{
		xr_delete(Form);
		return;
	}
	// the new textures are in the library now: New Textures has done its job, and the library it stood in for comes back
	if (completed && Form->bImportMode)
	{
		if (Form->m_ReturnToLibrary)
		{
			Form->m_ReturnToLibrary = false;
			Form->ClearItems();
			Form->m_ListView.search[0] = 0;
			Present(false);
		}
		else
			Form->m_CloseAfterUpdate = true;
	}
}

bool UIImageEditorForm::UpdateLib()
{
	VERIFY(!bReadonlyMode);
	RegisterModifiedTHM();
	SaveUsedTHM();
	if (bImportMode && !texture_map.empty())
	{
		AStringVec modif;
		ImageLib.SafeCopyLocalToServer(texture_map);
		// rename with folder
		FS_FileSet files = texture_map;
		texture_map.clear();
		xr_string fn;
		for (FS_FileSetIt it = files.begin(); it != files.end(); ++it)
		{
			fn = EFS.ChangeFileExt(it->name.c_str(), "");
			ImageLib.UpdateFileName(fn);
			FS_File F(*it);
			F.name = fn;
			texture_map.insert(F);
		}
		// sync
		ImageLib.SynchronizeTextures(true, true, true, &texture_map, &modif);
		if (UI->NeedAbort())
			return false;
		ImageLib.RefreshTextures(&modif);
		texture_map.clear();
	}
	else if (modif_map.size())
	{
		// save game textures
		AStringVec modif;
		ImageLib.SynchronizeTextures(true, true, true, &modif_map, &modif);
		if (UI->NeedAbort())
			return false;
		ImageLib.RefreshTextures(&modif);
	}
	modif_map.clear();
	return true;
}

void UIImageEditorForm::Select(int index)
{
	OnItemsFocused(index >= 0 && index < int(m_List.size()) ? m_List[index] : nullptr);
}

void UIImageEditorForm::OnItemsFocused(ListItem* item)
{
	PropItemVec props;

	RegisterModifiedTHM();
	m_THM_Current.clear();
	if (m_Texture)
	{
		m_TextureDead.push_back(m_Texture);
		m_TextureDeadFrame = ImGui::GetCurrentContext() ? ImGui::GetFrameCount() : -1;
	}
	m_Texture = nullptr;
	m_Selected = item ? item->Key() : "";
	if (item)
	{
		ETextureThumbnail* thm = FindUsedTHM(item->Key());
		m_THM_Current.push_back(thm);
		thm->FillProp(props, PropValue::TOnChange(this, &UIImageEditorForm::OnTypeChange));
		if (thm->_Format().type == STextureParams::ttCubeMap)
		{
			ButtonValue* B = PHelper().CreateButton(props, "Cube Map\\Smaller Copy", "Make", 0);
			B->OnBtnClickEvent.bind(this, &UIImageEditorForm::OnCubeMapBtnClick);
		}
		thm->Update(m_Texture);
	}
	m_ItemProps->AssignItems(props);
	m_ItemModified = false;
}

void UIImageEditorForm::SaveUsedTHM()
{
	for (THMMapIt t_it = m_THM_Used.begin(); t_it != m_THM_Used.end(); ++t_it)
	{
		if (modif_map.find(FS_File(t_it->second->SrcName())) != modif_map.end())
			t_it->second->Save();
	}
}

void UIImageEditorForm::FilterUpdate()
{
	for (ListItem* I : m_List)
	{
		ETextureThumbnail* thm = (ETextureThumbnail*)I->m_Object;
		const int type = thm->_Format().type;
		const bool shown = (STextureParams::ttImage == type && m_bFilterImage) ||
			(STextureParams::ttCubeMap == type && m_bFilterCube) ||
			(STextureParams::ttBumpMap == type && m_bFilterBump) ||
			(STextureParams::ttNormalMap == type && m_bFilterNormal) ||
			(STextureParams::ttTerrain == type && m_bFilterTerrain);
		I->Visible(shown);
		// the one on show leaves with its kind
		if (!shown && m_Selected == I->Key())
			OnItemsFocused(nullptr);
	}
	m_ListView.Invalidate();
}

void UIImageEditorForm::Mcp(const xr_string& action, LPCSTR raw, xr_string& out)
{
	UIImageEditorForm* f = Form;
	xr_string item, search;
	if (NqUtil::ArgString(raw, "search", search))
	{
		xr_strcpy(f->m_ListView.search, sizeof(f->m_ListView.search), search.c_str());
		f->m_ListView.Invalidate();
	}
	if (action == "select")
	{
		NqUtil::ArgString(raw, "item", item);
		const int at = f->m_ListView.Find(f->m_List, item.c_str());
		if (at < 0)
		{
			NqUtil::JsonError(out, NqUtil::Format("no texture is named or matches '%s'", item.c_str()));
			return;
		}
		f->Select(at);
	}
	int first = -1;
	const u32 shown = f->m_ListView.Count(f->m_List, &first);
	out += NqUtil::Format(",\"shown\":%u,\"first\":", shown);
	NqUtil::JsonString(out, first >= 0 ? f->m_List[first]->Key() : "");
	out += NqUtil::Format(",\"items\":%u,\"import\":%s,\"read_only\":%s,\"saving\":%s,\"dirty\":%s,\"selected\":", u32(f->m_List.size()),
		f->bImportMode ? "true" : "false", f->bReadonlyMode ? "true" : "false", f->m_UpdateQueued ? "true" : "false", f->Dirty() ? "true" : "false");
	NqUtil::JsonString(out, f->m_Selected);
	// where the list is read from: the library's source images, or the import folder New Textures looks in
	LPCSTR alias = f->bImportMode ? _import_ : _textures_;
	out += ",\"folder\":";
	NqUtil::JsonPath(out, FS.path_exist(alias) ? FS.get_path(alias)->m_Path : "");
	if (!f->m_THM_Current.empty())
	{
		ETextureThumbnail* thm = f->m_THM_Current.back();
		out += NqUtil::Format(",\"width\":%u,\"height\":%u,\"alpha\":%s,\"format\":", thm->_Width(), thm->_Height(), thm->_Alpha() ? "true" : "false");
		NqUtil::JsonString(out, thm->FormatString());
		out += NqUtil::Format(",\"preview\":%s", f->m_Texture ? "true" : "false");
	}
}
