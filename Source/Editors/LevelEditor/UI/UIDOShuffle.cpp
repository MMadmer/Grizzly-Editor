#include "stdafx.h"

UIDOShuffle* UIDOShuffle::Form = nullptr;
UIDOShuffle::UIDOShuffle()
{
	bModif = false;
	m_ChooseObject = false;
	m_Texture = nullptr;
	m_RealTexture = nullptr;
	m_list_selected = -1;
}

UIDOShuffle::~UIDOShuffle()
{
	ApplyChanges();
	if (m_Texture) m_Texture->Release();
	ClearIndexForms();
	if (m_RealTexture)m_RealTexture->Release();
}

void UIDOShuffle::DrawObjects()
{
	// the two halves share button labels (Clear All...): each has an id scope of its own
	ImGui::PushID("objects");
	DrawObjectList();
	ImGui::PopID();
}

void UIDOShuffle::DrawObjectList()
{
	{
		XUI::FontScope strong(XUI::Font::Strong);
		XUI::TextEllipsis("Objects");
	}
	const bool picked = m_list_selected >= 0 && m_list_selected < int(m_list.size());
	const XUI_LocalButton buttons[] =
	{
		{ "Add...",			"Pick library objects to grow as details", m_list.size() < dm_max_objects, "A level holds 64 detail objects at most" },
		{ "Remove",			"Take the object picked in the list out", picked, "Pick an object in the list first" },
		{ "Clear All...",	"Take every object and every colour index out", !m_list.empty(), "The list is empty" },
		{ "Import...",		"Read the objects and colour indices from a file" },
		{ "Export...",		"Write the objects and colour indices to a file" },
	};
	switch (XUI_LocalButtonFlow(buttons, _countof(buttons)))
	{
	case 0:
		UIChooseForm::SelectItem(smObject, 8);
		m_ChooseObject = true;
		break;
	case 1:
		DM->RemoveDO(m_list[m_list_selected].c_str());
		for (UIDOOneColor* one_color : m_color_indices)
			one_color->RemoveObject(m_list[m_list_selected]);
		m_list.erase(m_list.begin() + m_list_selected);
		OnItemFocused(nullptr);
		bModif = true;
		break;
	case 2:
	{
		XUI::ConfirmSpec spec;
		spec.id = "clear_detail_objects";
		spec.title = "Clear Detail Objects";
		spec.headline = "Take every detail object and every colour index out?";
		for (const xr_string& name : m_list)
			spec.items.push_back(name);
		spec.consequence = "The slots are cleared too. This cannot be undone.";
		spec.action = "Clear All";
		spec.destructive = true;
		XUI::Confirm(spec, [](XUI::Answer answer)
		{
			// the dialog may have closed while the question was up
			if (answer != XUI::Answer::Yes || !Form)
				return;
			Form->m_list.clear();
			// the indices point at the objects: they go first
			Form->DM->ClearColorIndices();
			Form->DM->RemoveDOs();
			Form->DM->InvalidateSlots();
			Form->ClearIndexForms();
			Form->OnItemFocused(nullptr);
			Form->bModif = true;
			UI->RedrawScene();
		});
		break;
	}
	case 3:
	{
		xr_string fname;
		if (EFS.GetOpenName(EDevice->m_hWnd, _detail_objects_, fname) && DM->ImportColorIndices(fname.c_str()))
		{
			DM->InvalidateSlots();
			FillData();
			bModif = true;
		}
		break;
	}
	case 4:
	{
		xr_string fname;
		if (EFS.GetSaveName(_detail_objects_, fname))
		{
			ApplyChanges(false);
			DM->ExportColorIndices(fname.c_str());
		}
		break;
	}
	default: break;
	}

	if (m_list.empty())
	{
		XUI::StatusText(XUI::Tone::Muted, "No detail objects yet. Add library objects to grow them over the level.");
		return;
	}
	const int rows = _max(1, _min(int(m_list.size()), 10));
	if (XUI::BeginList("##objects", ImVec2(0.f, rows * 22.f + 8.f)))
	{
		for (int i = 0; i < int(m_list.size()); ++i)
		{
			ImGui::PushID(i);
			if (XUI::ListRow("##row", i == m_list_selected) && i != m_list_selected)
			{
				m_list_selected = i;
				OnItemFocused(m_list[i].c_str());
			}
			XUI::TextEllipsis(m_list[i].c_str(), -1.f, XUI::Clip::Start);
			ImGui::PopID();
		}
	}
	XUI::EndList();
	DrawObjectSettings();
}

void UIDOShuffle::DrawObjectSettings()
{
	EDetail* dd = m_list_selected >= 0 && m_list_selected < int(m_list.size()) ? DM->FindDOByName(m_list[m_list_selected].c_str()) : nullptr;
	if (!dd)
		return;
	ImGui::Spacing();
	XUI_LocalThumbnail(m_RealTexture, _min(ImGui::GetContentRegionAvail().x, XUI::Pt(96.f)), XUI::Icon::DetailObjects,
		"This object has no thumbnail");
	if (!XUI::BeginPropertyGrid("detail_object"))
		return;
	if (XUI::PropertyRow("Density", "How densely the object grows: 0.1 to 1"))
		bModif |= ImGui::DragFloat("##density", &dd->m_fDensityFactor, .01f, .1f, 1.f, "%.2f", ImGuiSliderFlags_AlwaysClamp);
	if (XUI::PropertyRow("Min Scale", "The smallest size a detail of this object gets"))
		bModif |= ImGui::DragFloat("##min_scale", &dd->m_fMinScale, .01f, .1f, 100.f, "%g", ImGuiSliderFlags_AlwaysClamp);
	if (XUI::PropertyRow("Max Scale", "The largest size a detail of this object gets"))
		bModif |= ImGui::DragFloat("##max_scale", &dd->m_fMaxScale, .01f, .1f, 100.f, "%g", ImGuiSliderFlags_AlwaysClamp);
	if (XUI::PropertyRow("No Waving", "The object does not sway in the wind"))
	{
		bool still = !!dd->m_Flags.is(DO_NO_WAVING);
		if (ImGui::Checkbox("##no_waving", &still))
		{
			dd->m_Flags.set(DO_NO_WAVING, still);
			bModif = true;
		}
	}
	XUI::EndPropertyGrid();
}

void UIDOShuffle::DrawColorIndices()
{
	ImGui::PushID("colours");
	DrawColorIndexList();
	ImGui::PopID();
}

void UIDOShuffle::DrawColorIndexList()
{
	{
		XUI::FontScope strong(XUI::Font::Strong);
		XUI::TextEllipsis("Colour Indices");
	}
	const XUI_LocalButton buttons[] =
	{
		{ "Add Index",		"A new colour of the detail mask, with the objects that grow on it" },
		{ "Clear All...",	"Remove every colour index", !m_color_indices.empty(), "There are no colour indices" },
	};
	switch (XUI_LocalButtonFlow(buttons, _countof(buttons)))
	{
	case 0:
		m_color_indices.push_back(xr_new<UIDOOneColor>());
		m_color_indices.back()->DOShuffle = this;
		bModif = true;
		break;
	case 1:
	{
		char item[48];
		sprintf_s(item, "%u colour ind%s", u32(m_color_indices.size()), m_color_indices.size() == 1 ? "ex" : "ices");
		XUI::ConfirmSpec spec;
		spec.id = "clear_colour_indices";
		spec.title = "Clear Colour Indices";
		spec.headline = "Remove every colour index?";
		spec.items.push_back(item);
		spec.consequence = "The objects stay in the list.";
		spec.action = "Clear All";
		spec.destructive = true;
		XUI::Confirm(spec, [](XUI::Answer answer)
		{
			if (answer != XUI::Answer::Yes || !Form)
				return;
			Form->ClearIndexForms();
			Form->bModif = true;
		});
		break;
	}
	default: break;
	}

	if (m_color_indices.empty())
		XUI::StatusText(XUI::Tone::Muted, "No colour indices. Add one, then assign the objects that grow on its colour.");
	int index = 0;
	for (UIDOOneColor* one_color : m_color_indices)
	{
		ImGui::PushID(index++);
		one_color->Draw();
		ImGui::PopID();
	}
	for (auto b = m_color_indices.begin(); b != m_color_indices.end();)
	{
		if ((*b)->IsClosed())
		{
			xr_delete(*b);
			b = m_color_indices.erase(b);
		}
		else
			++b;
	}
}

void UIDOShuffle::Draw()
{
	if (m_RealTexture != m_Texture)
	{
		if (m_RealTexture)m_RealTexture->Release();
		m_RealTexture = m_Texture;
		if (m_RealTexture) m_RealTexture->AddRef();
	}

	if (ImGui::BeginTable("##detail_objects", 2, ImGuiTableFlags_SizingStretchProp | ImGuiTableFlags_BordersInnerV))
	{
		ImGui::TableSetupColumn("objects", ImGuiTableColumnFlags_WidthStretch, .55f);
		ImGui::TableSetupColumn("colours", ImGuiTableColumnFlags_WidthStretch, .45f);
		ImGui::TableNextRow();
		ImGui::TableNextColumn();
		DrawObjects();
		ImGui::TableNextColumn();
		DrawColorIndices();
		ImGui::EndTable();
	}

	if (m_ChooseObject)
	{
		bool ok = false;
		xr_vector<xr_string> list;
		if (UIChooseForm::GetResult(ok,list))
		{
			if (ok && !list.empty())
			{
				for (xr_string& l : list)
				{
					if (FindItem(l.c_str()))
						continue;
					if (m_list.size() >= dm_max_objects)
					{
						XUI::Toast(XUI::Tone::Warning, "A level holds 64 detail objects at most; the rest were not added");
						break;
					}
					DM->AppendDO(l.c_str());
					m_list_selected = m_list.size();
					m_list.push_back(l);
				}
				if (m_list_selected >= 0 && m_list_selected < int(m_list.size()))
					OnItemFocused(m_list[m_list_selected].c_str());
				bModif = true;
			}
			m_ChooseObject = false;
		}
		UIChooseForm::Update();
	}
}

void UIDOShuffle::Update()
{
	if (!Form || Form->IsClosed())
		return;
	if (XUI::BeginDialog("Detail Objects###dlg.detail_objects", &Form->bOpen, XUI::DialogSize::L))
	{
		XUI::BeginDialogBody();
		Form->Draw();
		char status[96];
		sprintf_s(status, "%u object%s \xC2\xB7 %u colour ind%s", u32(Form->m_list.size()), Form->m_list.size() == 1 ? "" : "s",
			u32(Form->m_color_indices.size()), Form->m_color_indices.size() == 1 ? "ex" : "ices");
		if (XUI::BeginDialogFooter(Form->bModif ? "Changed: reinitialize the details after closing" : status,
			Form->bModif ? XUI::Tone::Warning : XUI::Tone::Muted))
		{
			if (XUI::DialogButton("Close", XUI::Kind::Primary, true))
				Form->bOpen = false;
		}
		XUI::EndDialog();
	}
}

bool UIDOShuffle::GetResult()
{
	if (!Form->bOpen)
	{
		xr_delete(Form);
		return true;
	}
	return false;
}

void UIDOShuffle::Show(EDetailManager* DM)
{
	VERIFY(!Form);
	Form = xr_new<UIDOShuffle>();
	Form->DM = DM;
	Form->FillData();
}

void UIDOShuffle::FillData()
{
	m_list.clear();
	OnItemFocused(nullptr);
	for (CDetailManager::DetailIt d_it = DM->objects.begin(); d_it != DM->objects.end(); d_it++)
		m_list.push_back(((EDetail*)(*d_it))->GetName());
	ClearIndexForms();
	ColorIndexPairIt S = DM->m_ColorIndices.begin();
	ColorIndexPairIt E = DM->m_ColorIndices.end();
	ColorIndexPairIt it = S;
	for (; it != E; it++) {
		UIDOOneColor* OneColor = xr_new<UIDOOneColor>();
		OneColor->DOShuffle = this;
		m_color_indices.push_back(OneColor);
		OneColor->Color[0] = color_get_R(it->first)/255.f;
		OneColor->Color[1] = color_get_G(it->first) / 255.f;
		OneColor->Color[2] = color_get_B(it->first) / 255.f;
		for (DOIt do_it = it->second.begin(); do_it != it->second.end(); do_it++)
		{
			EDetail* dd = 0;
			for (CDetailManager::DetailIt d_it = DM->objects.begin(); d_it != DM->objects.end(); d_it++)
				if (0 == strcmp(((EDetail*)(*d_it))->GetName(), (*do_it)->GetName())) { dd = (EDetail*)*d_it; break; }
			VERIFY(dd);
			OneColor->list.push_back(dd->GetName());
		}
	}
}

void UIDOShuffle::OnItemFocused(const char* name)
{
	if (m_Texture) m_Texture->Release();
	m_Texture = nullptr;
	if (!name)
	{
		m_list_selected = -1;
		return;
	}
	if (EImageThumbnail* thm = ImageLib.CreateThumbnail(name, EImageThumbnail::ETObject))
	{
		thm->Update(m_Texture);
		xr_delete(thm);
	}
}

bool UIDOShuffle::FindItem(const char* name)
{
	for (xr_string& nm : m_list)
	{
		if (nm == name)return true;
	}
	return false;
}

void UIDOShuffle::ClearIndexForms()
{
	for (int i = 0; i < m_color_indices.size(); i++)xr_delete(m_color_indices[i]);
	m_color_indices.clear();
}

bool UIDOShuffle::ApplyChanges(bool msg )
{
	DM->RemoveColorIndices();
	for (u32 k = 0; k < m_color_indices.size(); k++) {
		UIDOOneColor* OneColor = m_color_indices[k];
		if (OneColor->list.size())
		{
			u32 clr = color_rgba_f(OneColor->Color[0], OneColor->Color[1], OneColor->Color[2], 1.f);
			for (xr_string&i:OneColor->list)
				DM->AppendIndexObject(clr, i.c_str(), false);
		}
	}
	if (bModif && msg)
	{
		ELog.DlgMsg(mtInformation, "The detail objects changed. Reinitialize the details to see it in the level.");
		DM->InvalidateSlots();
		return true;
	}
	return false;
}
