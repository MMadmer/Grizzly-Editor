//---------------------------------------------------------------------------
#include "stdafx.h"
#include "UISoundEditorForm.h"
#include "SoundManager.h"
#include "ui_main.h"
#include "Nq/NqUtil.h"
#include "../../../xrSound/stdafx.h"
#include "../../../xrSound/soundrender_source.h"
UISoundEditorForm *UISoundEditorForm::Form = nullptr;

namespace
{
	using GrizzlyTheme::ColorToken;

	ImU32 Tok(ColorToken token, float alpha = 1.f) { return GrizzlyTheme::U32(token, alpha); }

	// pt: list | preview | properties (docs/UI_DESIGN.md, section 6, Libraries)
	constexpr float kListW		= 260.f;
	constexpr float kGridW		= 320.f;
	constexpr float kSideMin	= 220.f;
	constexpr float kCanvasMin	= 400.f;
	constexpr int	kCurvePoints	= 96;
}

UISoundEditorForm::UISoundEditorForm()
{
	m_ItemProps = xr_new<UIPropertiesForm>();
	m_ItemProps->SetModifiedEvent(TOnModifiedEvent(this, &UISoundEditorForm::OnModified));
	modif_map.clear();
	m_Flags.zero();
	m_UpdateQueued = false;
	m_DeleteAfterUpdate = false;
	m_ItemModified = false;
	m_FileSize = 0;
	m_FileTime = 0;
	m_HasGameSound = false;
	// a sound plays on a click only; picking one in the list stays silent unless Auto Play is on
	bAutoPlay = FALSE;
	InitItemList();
	if (!FS.can_write_to_alias(_sounds_))
	{
		Log("#!The sound folder cannot be written: sound descriptions are read-only.");
		m_ItemProps->SetReadOnly(TRUE);
		m_Flags.set(flReadOnly, TRUE);
		bAutoPlay = TRUE;
	}
}

UISoundEditorForm::~UISoundEditorForm()
{
	m_Snd.destroy();
	for (ListItem* item : m_List)
		xr_delete(item);
	DestroyUsedTHM();
	xr_delete(m_ItemProps);
}

void UISoundEditorForm::Draw()
{
	// list | preview | the description's properties; the list and the properties keep their pt size
	const ImGuiChildFlags pane = ImGuiChildFlags_AlwaysUseWindowPadding;
	float list_w = kListW;
	XUI::Splitter("sound_library.list", true, &list_w, kSideMin, kCanvasMin + kSideMin + 8.f, kListW);
	ImGui::BeginDisabled(m_UpdateQueued);
	if (ImGui::BeginChild("##list", ImVec2(XUI::Pt(list_w), 0.f), pane))
	{
		// only an empty list says where the library is read from
		const xr_string empty = m_List.empty() ? NqUtil::Format("The library lists the .wav files in %s. A sound copied there shows up "
			"the next time the library opens.", LibraryFolderText(_sounds_).c_str()) : xr_string();
		const int picked = m_ListView.Draw("sound_library", m_List, m_Selected.c_str(), "No sounds", empty.c_str());
		if (picked >= 0)
			Select(picked);
	}
	ImGui::EndChild();
	ImGui::SameLine();
	if (ImGui::BeginChild("##rest", ImVec2(0.f, 0.f)))
	{
		float grid_w = kGridW;
		XUI::SplitterTail("sound_library.details", true, &grid_w, kCanvasMin, kSideMin, kGridW);
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
				XUI::EmptyState(XUI::Icon::Settings, "No properties", "Pick a sound to edit its description.");
		}
		ImGui::EndChild();
	}
	ImGui::EndChild();
	ImGui::EndDisabled();
}

void UISoundEditorForm::DrawToolbar(XUI::Toolbar& tb)
{
	const bool read_only = m_Flags.is(flReadOnly);
	const bool on = !read_only && !m_UpdateQueued && Dirty();
	const bool one = m_THM_Current.size() == 1;
	tb.Group(2);
	if (tb.Action("save", XUI::Icon::Save, "Save", on ? "Save the changed descriptions and rebuild their game sounds"
		: read_only ? "The sound folder cannot be written" : m_UpdateQueued ? "The library is saving" : "No unsaved changes", on))
	{
		xr_string err;
		if (!QueueSave(err))
			XUI::Notice(XUI::Tone::Error, "Save failed", err.c_str());
	}
	tb.Group(3);
	const bool playable = one && m_HasGameSound;
	const char* no_sound = one ? "This description has no game sound (.ogg) yet" : "Pick a sound first";
	if (tb.Action("play", XUI::Icon::Play, "Play", playable ? "Play the game sound" : no_sound, playable))
		Play();
	// Stop is there to stop something: off while nothing plays, and named like Play so it reads as off too
	const bool playing = !!m_Snd._feedback();
	if (tb.Action("stop", XUI::Icon::Stop, "Stop", playing ? "Stop the sound" : "Nothing is playing", playing))
		Stop();
	if (tb.Action("auto_play", XUI::Icon::SoundSource, "Auto Play", "Play every sound as it is picked in the list", true, !!bAutoPlay))
		bAutoPlay = !bAutoPlay;
	tb.Group(1);
	const bool rebuild = !read_only && !m_UpdateQueued && one;
	if (tb.Action("rebuild", XUI::Icon::Refresh, "Rebuild", rebuild ? "Build the game sound (.ogg) of this description again"
		: read_only ? "The sound folder cannot be written" : "Pick a sound first", rebuild))
		RebuildGameSound();
	tb.Group(0);
	if (read_only)
		tb.Status("Read-only", XUI::Tone::Warning);
	else if (m_UpdateQueued)
		tb.Status("Saving...", XUI::Tone::Info);
}

void UISoundEditorForm::DrawStatus()
{
	u32 changed = u32(modif_map.size());
	if (m_ItemModified)
		++changed;
	xr_string text = NqUtil::Format("%u sounds", u32(m_List.size()));
	if (m_ListView.shown.size() != m_List.size())
		text += NqUtil::Format(" \xC2\xB7 %u shown", u32(m_ListView.shown.size()));
	if (changed)
		text += NqUtil::Format(" \xC2\xB7 %u changed", changed);
	ImGui::TextUnformatted(text.c_str());
}

void UISoundEditorForm::DrawPreview()
{
	ESoundThumbnail* thm = m_THM_Current.size() == 1 ? m_THM_Current.back() : nullptr;
	if (!thm)
	{
		XUI::EmptyState(XUI::Icon::FileSound, "No sound selected", "Pick a sound in the list to hear it and see how far it carries.");
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
		ImGui::PopStyleColor();
	}
	if (m_HasGameSound)
	{
		const xr_string facts = NqUtil::Format("%.1f KB \xC2\xB7 %.2f s", float(m_FileSize) / 1024.f, float(m_FileTime) / 1000.f);
		XUI::FontScope small_font(XUI::Font::Small);
		ImGui::PushStyleColor(ImGuiCol_Text, Tok(ColorToken::TextMuted));
		XUI::TextEllipsis(facts.c_str());
		ImGui::PopStyleColor();
	}
	else
		XUI::StatusText(XUI::Tone::Warning, "No game sound (.ogg) was built from this description yet.");
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(12.f)));
	if (XUI::Section("Attenuation", true))
	{
		DrawAttenuation(thm, clampr(ImGui::GetContentRegionAvail().y - XUI::Pt(28.f), XUI::Pt(96.f), XUI::Pt(240.f)));
		XUI::EndSection();
	}
}

// How loud the sound is with distance: full up to the minimum distance, then falling off with the rolloff the
// game uses, silent past the maximum - the same curve the old property canvas plotted.
void UISoundEditorForm::DrawAttenuation(ESoundThumbnail* thm, float height)
{
	ImDrawList* dl = ImGui::GetWindowDrawList();
	const float w = _max(ImGui::GetContentRegionAvail().x, XUI::Pt(64.f));
	const ImVec2 a = ImGui::GetCursorScreenPos();
	const ImVec2 b(a.x + w, a.y + height);
	dl->AddRectFilled(a, b, Tok(ColorToken::Inset), XUI::Pt(4.f));
	const float max_d = _max(thm->MaxDist(), EPS_L);
	const float rolloff = _max(psSoundRolloff, EPS_L);
	ImVec2 points[kCurvePoints + 1];
	for (int i = 0; i <= kCurvePoints; ++i)
	{
		const float d = _max(max_d * i / kCurvePoints, EPS_L);
		const float volume = clampr(thm->MinDist() / (rolloff * d), 0.f, 1.f);
		points[i] = ImVec2(a.x + w * i / kCurvePoints, b.y - volume * height);
	}
	dl->AddPolyline(points, kCurvePoints + 1, Tok(ColorToken::AccentBright), _max(1.f, XUI::Pt(2.f)));
	const float min_x = a.x + w * clampr(thm->MinDist() / max_d, 0.f, 1.f);
	dl->AddLine(ImVec2(min_x, a.y), ImVec2(min_x, b.y), Tok(ColorToken::Border), _max(1.f, XUI::Pt(1.f)));
	dl->AddRect(a, b, Tok(ColorToken::BorderSubtle), XUI::Pt(4.f));
	ImGui::Dummy(ImVec2(w, height));
	XUI::Tip("Volume over distance: full up to the minimum distance, silent past the maximum.");

	XUI::FontScope small_font(XUI::Font::Small);
	ImGui::PushStyleColor(ImGuiCol_Text, Tok(ColorToken::TextMuted));
	const xr_string left = NqUtil::Format("Min %.1f m", thm->MinDist());
	const xr_string right = NqUtil::Format("Max %.1f m", thm->MaxDist());
	const float right_x = ImGui::GetCursorPosX() + ImGui::GetContentRegionAvail().x - ImGui::CalcTextSize(right.c_str()).x;
	ImGui::TextUnformatted(left.c_str());
	ImGui::SameLine(_max(right_x, ImGui::GetItemRectMax().x - ImGui::GetWindowPos().x + XUI::Pt(8.f)));
	ImGui::TextUnformatted(right.c_str());
	ImGui::PopStyleColor();
}

void UISoundEditorForm::Update()
{
}

void UISoundEditorForm::Show()
{
	if (Form && Form->m_DeleteAfterUpdate)
	{
		XUI::Notice(XUI::Tone::Info, "Sound Library", "The library is still saving its changes. Open it again when that is done.");
		return;
	}
	if (Form)
	{
		XUI::Shell::Activate("sound_library");
		return;
	}
	Form = xr_new<UISoundEditorForm>();
	XUI::Shell::DocDesc d;
	d.id = "sound_library";
	d.kind = "sound_library";
	d.icon = XUI::Icon::FileSound;
	d.padded = false;
	d.label = [] { return xr_string("Sound Library"); };
	d.tooltip = [] { return xr_string("The sound descriptions of the editor's library"); };
	d.alive = [] { return Form && !Form->IsClosed() && !Form->m_DeleteAfterUpdate; };
	d.dirty = [] { return Form && Form->Dirty(); };
	d.toolbar = [](XUI::Toolbar& tb) { if (Form) Form->DrawToolbar(tb); };
	d.body = [] { if (Form) Form->Draw(); };
	d.status = [] { if (Form) Form->DrawStatus(); };
	// the save runs deferred, outside the frame, with its progress; the form outlives its tab until it is through
	d.save = [](xr_string& err) { return Form && Form->QueueSave(err); };
	d.discard = [] { if (Form) Form->Revert(); };
	d.on_close = [] { if (Form && Form->m_UpdateQueued) Form->m_DeleteAfterUpdate = true; else xr_delete(Form); };
	XUI::Shell::Open(d, true);
}

void UISoundEditorForm::HideLib()
{
	m_Snd.destroy();
	bOpen = false;
}

void UISoundEditorForm::RegisterModifiedTHM()
{
	if (m_ItemModified)
		for (THMIt t_it = m_THM_Current.begin(); t_it != m_THM_Current.end(); t_it++)
			AppendModif((*t_it)->SrcName());
	m_ItemModified = false;
}

void UISoundEditorForm::OnModified()
{
	m_ItemModified = true;
}

bool UISoundEditorForm::Dirty()
{
	return !modif_map.empty() || m_ItemModified;
}

void UISoundEditorForm::Revert()
{
	// every description is read again from its file
	const xr_string keep = m_Selected;
	m_Snd.destroy();
	m_THM_Current.clear();
	m_ItemProps->ClearProperties();
	DestroyUsedTHM();
	modif_map.clear();
	m_ItemModified = false;
	m_Selected.clear();
	const int again = m_ListView.Find(m_List, keep.c_str());
	if (again >= 0 && 0 == _stricmp(m_List[again]->Key(), keep.c_str()))
		Select(again);
}

bool UISoundEditorForm::QueueSave(xr_string& err)
{
	if (m_Flags.is(flReadOnly))
	{
		err = "The sound folder cannot be written.";
		return false;
	}
	if (m_UpdateQueued)
		return true;
	RegisterModifiedTHM();
	m_UpdateQueued = UI->DeferUIWork(&UISoundEditorForm::UpdateLibDeferred);
	if (!m_UpdateQueued)
		err = "The save could not be started.";
	return m_UpdateQueued;
}

void UISoundEditorForm::UpdateLibDeferred()
{
	if (!Form)
		return;

	Form->m_Snd.destroy();
	SProgressOperation progress(*UI, true);
	Form->UpdateLib();
	Form->m_UpdateQueued = false;
	// its tab closed while the save was waiting: nothing shows the form any more
	if (Form->m_DeleteAfterUpdate)
		xr_delete(Form);
}

bool UISoundEditorForm::UpdateLib()
{
	RegisterModifiedTHM();
	SaveUsedTHM();
	// save game sounds
	if (modif_map.size())
	{
		SndLib->SynchronizeSounds(true, true, true, &modif_map, 0);
		if (UI->NeedAbort())
			return false;
		SndLib->RefreshSounds(false);
		modif_map.clear();
	}
	return true;
}

void UISoundEditorForm::AppendModif(LPCSTR nm)
{
	FS_File 		dest;
	string_path		fname;
	FS.update_path(fname, _sounds_, EFS.ChangeFileExt(nm, ".wav").c_str());
	BOOL bFind = FS.file_find(fname, dest); R_ASSERT(bFind);
	modif_map.insert(dest);
}

void UISoundEditorForm::Play()
{
	if (m_Snd._handle())
		m_Snd.play(0, sm_2D);
}

void UISoundEditorForm::Stop()
{
	m_Snd.stop();
}

void UISoundEditorForm::RebuildGameSound()
{
	for (THMIt it = m_THM_Current.begin(); it != m_THM_Current.end(); ++it)
	{
		ESoundThumbnail* pTHM = *it;

		string_path             src_name, game_name;
		FS.update_path(src_name, _sounds_, pTHM->SrcName());
		strconcat(sizeof(src_name), src_name, src_name, ".wav");

		FS.update_path(game_name, _game_sounds_, pTHM->SrcName());
		strconcat(sizeof(game_name), game_name, game_name, ".ogg");

		Msg("Rebuilding the game sound %s", game_name);
		SndLib->MakeGameSound(pTHM, src_name, game_name);
	}
	// the preview follows the new file
	Select(m_ListView.Find(m_List, m_Selected.c_str()));
}

void UISoundEditorForm::OnAttClick(ButtonValue* V, bool& bModif, bool& bSafe)
{
	bModif = true;
	ESoundThumbnail* thm = m_THM_Current.back();
	switch (V->btn_num) {
	case 0: {
		float dist = thm->MinDist() / (0.01f * psSoundRolloff);
		thm->SetMaxDist(dist + 0.1f * dist);
	}break;
	case 1: {
		float dist = psSoundRolloff * (thm->MaxDist() - (0.1f / 1.1f) * thm->MaxDist()) * 0.01f;
		thm->SetMinDist(dist);
	}break;
	}
	OnModified();
}

void UISoundEditorForm::InitItemList()
{
	FS_FileSet		sound_map;
	SndLib->GetSounds(sound_map, TRUE);

	for (ListItem* item : m_List)
		xr_delete(item);
	m_List.clear();
	for (FS_FileSetIt it = sound_map.begin(); it != sound_map.end(); it++)
		LHelper().CreateItem(m_List, it->name.c_str(), 0);
	m_ListView.Invalidate();
}

void UISoundEditorForm::Select(int index)
{
	OnItemsFocused(index >= 0 && index < int(m_List.size()) ? m_List[index] : nullptr);
}

void UISoundEditorForm::OnItemsFocused(ListItem* item)
{
	PropItemVec props;

	RegisterModifiedTHM();
	m_Snd.destroy();
	m_THM_Current.clear();
	m_FileSize = m_FileTime = 0;
	m_HasGameSound = false;
	m_Selected = item ? item->Key() : "";

	if (item)
	{
		ESoundThumbnail* thm = FindUsedTHM(item->Key());
		if (!thm) m_THM_Used.push_back(thm = xr_new<ESoundThumbnail>(item->Key()));
		m_THM_Current.push_back(thm);
		thm->FillProp(props);
	}

	if (m_THM_Current.size() == 1)
	{
		ESoundThumbnail* thm = m_THM_Current.back();
		PlaySound(thm->SrcName(), m_FileSize, m_FileTime);
		ButtonValue* B = PHelper().CreateButton(props, "Attenuation\\Auto", "Max from Min,Min from Max", ButtonValue::flFirstOnly);
		B->OnBtnClickEvent.bind(this, &UISoundEditorForm::OnAttClick);
		if (m_HasGameSound)
		{
			PHelper().CreateCaption(props, "Game Sound\\File Size", shared_str().printf("%.2f KB", float(m_FileSize) / 1024.f));
			PHelper().CreateCaption(props, "Game Sound\\Length", shared_str().printf("%.2f s", float(m_FileTime) / 1000.f));
		}
	}
	m_ItemProps->AssignItems(props);
	m_ItemModified = false;
}

void UISoundEditorForm::PlaySound(LPCSTR name, u32& size, u32& time)
{
	string_path fname;
	FS.update_path(fname, _game_sounds_, EFS.ChangeFileExt(name, ".ogg").c_str());
	FS_File F;
	if (FS.file_find(fname, F))
	{
		m_Snd.create(name, st_Effect, sg_Undefined);
		m_Snd.play(0, sm_2D);
		CSoundRender_Source* src = (CSoundRender_Source*)m_Snd._handle(); VERIFY(src);
		size = F.size;
		time = iFloor(src->fTimeTotal / 1000.0f);
		m_HasGameSound = true;
		if (!bAutoPlay)		m_Snd.stop();
	}
}

ESoundThumbnail* UISoundEditorForm::FindUsedTHM(LPCSTR name)
{
	for (THMIt it = m_THM_Used.begin(); it != m_THM_Used.end(); it++)
		if (0 == strcmp((*it)->SrcName(), name)) return *it;
	return 0;
}

void UISoundEditorForm::SaveUsedTHM()
{
	int m_age = time(NULL);
	for (THMIt t_it = m_THM_Used.begin(); t_it != m_THM_Used.end(); t_it++)
		(*t_it)->Save(m_age, 0);
}

void UISoundEditorForm::DestroyUsedTHM()
{
	for (THMIt it = m_THM_Used.begin(); it != m_THM_Used.end(); it++)
		xr_delete(*it);
	m_THM_Used.clear();
}

void UISoundEditorForm::Mcp(const xr_string& action, LPCSTR raw, xr_string& out)
{
	UISoundEditorForm* f = Form;
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
			NqUtil::JsonError(out, NqUtil::Format("no sound is named or matches '%s'", item.c_str()));
			return;
		}
		f->Select(at);
	}
	int first = -1;
	const u32 shown = f->m_ListView.Count(f->m_List, &first);
	out += NqUtil::Format(",\"shown\":%u,\"first\":", shown);
	NqUtil::JsonString(out, first >= 0 ? f->m_List[first]->Key() : "");
	out += NqUtil::Format(",\"items\":%u,\"read_only\":%s,\"saving\":%s,\"dirty\":%s,\"auto_play\":%s,\"selected\":", u32(f->m_List.size()),
		f->m_Flags.is(flReadOnly) ? "true" : "false", f->m_UpdateQueued ? "true" : "false", f->Dirty() ? "true" : "false",
		f->bAutoPlay ? "true" : "false");
	NqUtil::JsonString(out, f->m_Selected);
	// where the list is read from: the library's .wav files
	out += ",\"folder\":";
	NqUtil::JsonPath(out, FS.path_exist(_sounds_) ? FS.get_path(_sounds_)->m_Path : "");
	if (f->m_THM_Current.size() == 1)
	{
		ESoundThumbnail* thm = f->m_THM_Current.back();
		out += NqUtil::Format(",\"min_dist\":%g,\"max_dist\":%g,\"game_sound\":%s,\"bytes\":%u,\"ms\":%u", thm->MinDist(), thm->MaxDist(),
			f->m_HasGameSound ? "true" : "false", f->m_FileSize, f->m_FileTime);
	}
}
