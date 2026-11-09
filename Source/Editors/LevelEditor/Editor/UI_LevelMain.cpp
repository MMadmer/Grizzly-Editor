#include "stdafx.h"
#include "../../XrECore/Editor/Nq/NqPickers.h"
#include "../../XrECore/Editor/Nq/NqUtil.h"
#include "Recovery/EditorRecovery.h"
#include "..\..\XrAPI\xrGameManager.h"
#include "..\..\XrECore\Editor\EditorModManifest.h"
#include "..\..\XrECore\Editor\EditorModPublish.h"
#include "..\..\XrECore\Editor\EditorCatalogSubmit.h"
#include "..\..\XrECore\Editor\EditorCharacters.h"
#include "..\..\XrECore\Editor\EditorWorld.h"
#include "..\..\XrECore\Editor\EditorScreens.h"
#include "..\..\XrECore\Editor\EditorPda.h"
#include "..\..\..\xrServerEntities\xrServer_Objects_ALife_All.h"
#include "..\..\XrECore\Editor\EditorGameContent.h"
#include "..\..\XrECore\Editor\UI_ViewportNavigation.h"
#include "..\UI\QuestGraph\UIQuestGraph.h"
#include "EditorModScene.h"
#include "Utils\Cursor3D.h"
#include "Mcp\LevelMcp.h"
#include "..\xrengine\GameFont.h"

#ifdef _LEVEL_EDITOR
//.    if (m_Cursor->GetVisible()) RedrawScene();
#endif

CLevelMain*	LUI=(CLevelMain*)UI;

// The restrictors of the scene that is open, for the quest pickers. They live one
// layer down and cannot reach the scene, so it is handed to them.
static void CollectSceneRestrictors(xr_vector<NqPickers::SEntry>& out)
{
    out.clear();
    if (!Scene) return;
    ObjectList& lst = Scene->ListObj(OBJCLASS_SPAWNPOINT);
    for (ObjectIt it = lst.begin(); it != lst.end(); ++it)
    {
        CSpawnPoint* sp = dynamic_cast<CSpawnPoint*>(*it);
        if (!sp) continue;
        LPCSTR section = sp->RefName();
        if (!section || 0 != xr_strcmp(section, "space_restrictor")) continue;
        LPCSTR name = sp->GetName();
        if (!name || !name[0]) continue;
        NqPickers::SEntry e;
        e.id = name;
        e.extra = "scene";
        out.push_back(e);
    }
}

// The way paths of the open scene - what a behaviour step walks (npc.goto,
// npc.hold). Live, for the same reason the restrictors are.
// The Characters window's Preview: a character's model is below meshes\\, the
// module's own first and the linked game's otherwise - both through the one viewer
static void PreviewCharacterModel(LPCSTR visual)
{
    if (!visual || !visual[0]) return;
    xr_string rel = xr_string("meshes\\") + visual;
    for (size_t i = 0; i < rel.size(); ++i) if (rel[i] == '/') rel[i] = '\\';
    if (rel.size() < 4 || 0 != _stricmp(rel.c_str() + rel.size() - 4, ".ogf")) rel += ".ogf";

    if (EditorProject::Active())
    {
        string_path abs;
        sprintf_s(abs, "%s\\gamedata\\%s", EditorProject::Root(), rel.c_str());
        HANDLE h = ::CreateFileA(abs, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
        if (INVALID_HANDLE_VALUE != h)
        {
            const DWORD size = ::GetFileSize(h, NULL);
            xr_vector<u8> bytes(size ? size : 1);
            DWORD got = 0;
            const bool ok = size && size != INVALID_FILE_SIZE && ::ReadFile(h, bytes.data(), size, &got, NULL) && got == size;
            ::CloseHandle(h);
            if (ok)
            {
                UIVisualPreview::ShowFromMemory(visual, bytes.data(), size);
                return;
            }
        }
    }
    xr_string err;
    if (!EditorGameContent::EnsureMounted(err)) return;
    xr_string key = rel;
    xr_strlwr(&key[0]);
    const int idx = EditorGameContent::Find(key.c_str());
    u32 size = 0;
    u8* bytes = (idx >= 0) ? EditorGameContent::ReadBytes(idx, size) : 0;
    if (bytes && size) UIVisualPreview::ShowFromMemory(visual, bytes, size);
    EditorGameContent::FreeBytes(bytes);
}

static void CollectSceneWayPaths(xr_vector<NqPickers::SEntry>& out)
{
    out.clear();
    if (!Scene) return;
    ObjectList& lst = Scene->ListObj(OBJCLASS_WAY);
    for (ObjectIt it = lst.begin(); it != lst.end(); ++it)
    {
        LPCSTR name = (*it) ? (*it)->GetName() : 0;
        if (!name || !name[0]) continue;
        NqPickers::SEntry e;
        e.id = name;
        e.extra = "scene";
        out.push_back(e);
    }
}

// The typed "Story ID (script)" of every spawn object in the open scene: the block sits
// in the object's custom data, and picking it up live means the name is usable in the
// quest pickers the moment the author writes it - no save, no export.
static void CollectSceneStories(xr_vector<NqPickers::SEntry>& out)
{
    out.clear();
    if (!Scene) return;
    ObjectList& lst = Scene->ListObj(OBJCLASS_SPAWNPOINT);
    for (ObjectIt it = lst.begin(); it != lst.end(); ++it)
    {
        CSpawnPoint* sp = dynamic_cast<CSpawnPoint*>(*it);
        if (!sp) continue;
        ISE_Abstract* se = sp->m_SpawnData.m_Data;
        if (!se) continue;
        LPCSTR ini = se->m_ini_string.c_str();
        if (!ini || !strstr(ini, "[story_object]")) continue;
        // minimal ini scan: story_id = X, or the bare-id line right under the header
        xr_string text = ini;
        size_t pos = text.find("[story_object]") + xr_strlen("[story_object]");
        xr_string sid;
        while (pos < text.size())
        {
            size_t eol = text.find('\n', pos);
            if (eol == xr_string::npos) eol = text.size();
            xr_string line = NqUtil::Trim(text.substr(pos, eol - pos).c_str());
            pos = eol + 1;
            if (line.empty()) continue;
            if (line[0] == '[') break;
            const size_t eq = line.find('=');
            if (eq != xr_string::npos)
            {
                if (NqUtil::Trim(line.substr(0, eq).c_str()) != "story_id") break;
                sid = NqUtil::Trim(line.substr(eq + 1).c_str());
            }
            else sid = line;
            break;
        }
        if (sid.empty()) continue;
        NqPickers::SEntry e;
        e.id = sid;
        LPCSTR name = sp->GetName();
        e.extra = xr_string("scene: ") + (name ? name : "");
        out.push_back(e);
    }
}

// ---- the mode's world (EditorWorld) ----------------------------------------------
// The Player Starts of the open scene, under the level the bake writes them for: the
// World window and the build's check see a start the moment it is placed.
static void CollectSceneStarts(xr_string& level, xr_vector<EditorWorld::SStart>& starts)
{
    level.clear();
    starts.clear();
    if (!Scene || !EditorModScene::SceneLevelName(level)) return;
    ObjectList& lst = Scene->ListObj(OBJCLASS_SPAWNPOINT);
    for (ObjectIt it = lst.begin(); it != lst.end(); ++it)
    {
        CSpawnPoint* sp = dynamic_cast<CSpawnPoint*>(*it);
        if (!sp || sp->m_Type != ptPlayerStart) continue;
        EditorWorld::SStart s;
        s.level     = level;
        s.name      = sp->GetName();
        s.source    = "scene";
        s.position  = sp->GetPosition();
        sp->_Transform().getXYZ(s.direction);
        starts.push_back(s);
    }
}

u32 DropObjectsToGround(ObjectList& objects, bool align_normal, bool line_trace);

// A Player Start in front of the camera, facing the way it looks, dropped onto the
// ground. Called from the World window's Draw: the frame is asked for, never forced.
static bool PlaceSceneStart(xr_string& err)
{
    if (!Scene || Scene->locked())  { err = "no scene is open, or it is locked"; return false; }
    ESceneCustomOTool* tool = Scene->GetOTool(OBJCLASS_SPAWNPOINT);
    if (!tool)                      { err = "no spawn tool in this build"; return false; }
    string256 name;
    Scene->GenObjectName(OBJCLASS_SPAWNPOINT, name, "player_start");
    CCustomObject* obj = tool->CreateObject((LPVOID)PLAYERSTART_CHOOSE_NAME, name);
    if (!obj)                       { err = "the spawn tool refused it"; return false; }
    const Fvector& dir = EDevice->m_Camera.GetDirection();
    Fvector pos;
    pos.mad(EDevice->m_Camera.GetPosition(), dir, 5.f);
    obj->SetPosition(pos);
    obj->SetRotation(Fvector().set(0.f, atan2f(dir.x, dir.z), 0.f));
    if (LTools->CurrentClassID() != OBJCLASS_SPAWNPOINT)
        LTools->SetTarget(OBJCLASS_SPAWNPOINT, 0);
    Scene->SelectObjects(false, OBJCLASS_DUMMY);
    Scene->AppendObject(obj);
    obj->Select(TRUE);
    ObjectList batch;
    batch.push_back(obj);
    DropObjectsToGround(batch, false, false);
    UI->RedrawScene();
    return true;
}

// selects an object and frames it, like the F key
static bool FocusSceneObject(LPCSTR name)
{
    if (!Scene || !name) return false;
    CCustomObject* obj = Scene->FindObjectByName(name, (CCustomObject*)0);
    if (!obj) return false;
    Scene->SelectObjects(false, OBJCLASS_DUMMY);
    obj->Select(TRUE);
    if (LTools->CurrentClassID() != obj->FClassID)
        LTools->SetTarget(obj->FClassID, 0);
    Fbox box;
    if (!obj->GetBox(box))
    {
        box.set(obj->GetPosition(), obj->GetPosition());
        box.grow(2.f);
    }
    EDevice->m_Camera.ZoomExtents(box);
    UI->RedrawScene();
    return true;
}

CLevelMain::CLevelMain()
{
    m_Cursor        = xr_new<C3DCursor>();
    EPrefs			= xr_new<CLevelPreferences>();
    NqPickers::SetSceneRestrictors(CollectSceneRestrictors);
    NqPickers::SetSceneStories(CollectSceneStories);
    NqPickers::SetSceneWayPaths(CollectSceneWayPaths);
    EditorCharacters::SetModelPreview(PreviewCharacterModel);
    EditorWorld::SetSceneStarts(CollectSceneStarts);
    EditorWorld::SetPlaceStart(PlaceSceneStart);
    EditorWorld::SetFocusObject(FocusSceneObject);
}

CLevelMain::~CLevelMain()
{
    xr_delete		(EPrefs);
    xr_delete		(m_Cursor);
   // TClipMaker::DestroyForm(g_clip_maker);
}



// Tools commands
CCommandVar CLevelTool::CommandChangeTarget(CCommandVar p1, CCommandVar p2)
{
	if (Scene->GetTool(p1)->IsEnabled())
    {
	    SetTarget	(p1,p2);
	    ExecCommand	(COMMAND_UPDATE_PROPERTIES);
        return 		TRUE;
    }else{
    	return 		FALSE;
    }
}
CCommandVar CLevelTool::CommandShowObjectList(CCommandVar p1, CCommandVar p2)
{
    if (LUI->GetEState()==esEditScene) ShowObjectList();
    return TRUE;
}

// Main commands
CCommandVar CommandLibraryEditor(CCommandVar p1, CCommandVar p2)
{
   /* if (Scene->ObjCount()||(LUI->GetEState()!=esEditScene)){
        if (LUI->GetEState()==esEditLibrary)	TfrmEditLibrary::ShowEditor();
        else									ELog.DlgMsg(mtError, "Scene must be empty before editing library!");
    }else{
        TfrmEditLibrary::ShowEditor();
    }*/
    return TRUE;
}
CCommandVar CommandLAnimEditor(CCommandVar p1, CCommandVar p2)
{
    //TfrmEditLightAnim::ShowEditor();
    return TRUE;
}
CCommandVar CommandFileMenu(CCommandVar p1, CCommandVar p2)
{
    //FHelper.ShowPPMenu(fraLeftBar->pmSceneFile,0);
    return TRUE;
}
CCommandVar CLevelTool::CommandEnableTarget(CCommandVar p1, CCommandVar p2)
{
	ESceneToolBase* M 	= Scene->GetTool(p1);
    VERIFY					(M);
    BOOL res				= FALSE;
	if (p2)
    {
    	res 				= ExecCommand(COMMAND_LOAD_LEVEL_PART,M->FClassID,TRUE);
        if(res)
		    M->m_EditFlags.set(ESceneToolBase::flEnable,TRUE);
    }else
    {
        if (!Scene->IfModified())
        {
		    M->m_EditFlags.set(ESceneToolBase::flEnable,TRUE);
            res				= FALSE;
        }else
        {
	    	res				= ExecCommand(COMMAND_UNLOAD_LEVEL_PART,M->FClassID,TRUE);
            if(res)
		    	M->m_EditFlags.set(ESceneToolBase::flEnable,FALSE);
        }
		if (res)        	
        	ExecCommand(COMMAND_CHANGE_TARGET,OBJCLASS_SCENEOBJECT);
    }
    ExecCommand				(COMMAND_REFRESH_UI_BAR);
    return res;
}

CCommandVar CLevelTool::CommandShowTarget(CCommandVar p1, CCommandVar p2)
{
	ESceneToolBase* M 	= Scene->GetTool(p1);
    if(p2)
    	M->m_EditFlags.set(ESceneToolBase::flVisible,TRUE);
    else
    	M->m_EditFlags.set(ESceneToolBase::flVisible,FALSE);
        
    return TRUE;
}

CCommandVar CLevelTool::CommandReadonlyTarget(CCommandVar p1, CCommandVar p2)
{
	ESceneToolBase* M 		= Scene->GetTool(p1); VERIFY(M);
    BOOL res				= TRUE;
	if (p2){
        if (!Scene->IfModified())
        {
		    M->m_EditFlags.set(ESceneToolBase::flForceReadonly,FALSE);
            res				= FALSE;
        }else
        {
//.            xr_string pn	= Scene->LevelPartName(LTools->m_LastFileName.c_str(),M->ClassID);
        }
    }else
    {
//.        xr_string pn		= Scene->LevelPartName(LTools->m_LastFileName.c_str(), M->ClassID);
    }
    if (res)
    {
    	Reset				();
	    ExecCommand			(COMMAND_REFRESH_UI_BAR);
    }
    return res;
}

CCommandVar CLevelTool::CommandMultiRenameObjects(CCommandVar p1, CCommandVar p2)
{
    if (Scene->locked())
    {
        ELog.DlgMsg			( mtError, "Scene sharing violation" );
        return 				FALSE;
    }
    // the objects the rename takes: the selected ones of the target class, or of every class
    XUI::ConfirmSpec spec;
    spec.id = "rename_objects";
    spec.title = "Rename Objects";
    spec.action = "Rename";
    if (!CommandConfirmed(p2))
    {
        ObjectList sel;
        Scene->GetQueryObjects(sel, LTools->GetTarget(), 1, -1, -1);
        for (ObjectIt it = sel.begin(); it != sel.end(); ++it)
            if (spec.items.size() < 10)
                spec.items.push_back((*it)->GetName());
        spec.total = u32(sel.size());
        if (!spec.total)
        {
            ELog.Msg(mtInformation, "Nothing is selected: no object is renamed.");
            return FALSE;
        }
    }
    xr_string headline;
    headline.sprintf("Rename the %u selected object(s) after their references?", spec.total);
    spec.headline = headline.c_str();
    spec.consequence = "Each one takes the next free name made from its reference. Undo puts the names back.";
    if (!ConfirmCommand(COMMAND_MULTI_RENAME_OBJECTS, p1, p2, spec))
        return FALSE;
    const int cnt = Scene->MultiRenameObjects();
    if (cnt)
    {
        ExecCommand	(COMMAND_UPDATE_PROPERTIES);
        Scene->UndoSave();
    }
    ELog.Msg(mtInformation, "Objects renamed: %d.", cnt);
    if (!XUI::FromMcp() && !XUI::Unattended())
    {
        char done[64];
        sprintf_s(done, cnt == 1 ? "%d object renamed" : "%d objects renamed", cnt);
        XUI::Toast(XUI::Tone::Success, done);
    }
    return 					cnt > 0;
}
CCommandVar CommandLoadLevelPart(CCommandVar p1, CCommandVar p2)
{
    xr_string temp_fn	= LTools->m_LastFileName.c_str();
    if (!temp_fn.empty())
        return			Scene->LoadLevelPart(temp_fn.c_str(),p1);
    return				TRUE;
}
CCommandVar CommandUnloadLevelPart(CCommandVar p1, CCommandVar p2)
{
    xr_string temp_fn	= LTools->m_LastFileName.c_str();
    if (!temp_fn.empty())
        return			Scene->UnloadLevelPart(temp_fn.c_str(),p1);
    return				TRUE;
}
static bool s_SuppressRecoveryCleanup = false;
static bool s_SuppressSceneSaveMetadata = false;
static bool s_SuppressSceneSaveDialog = false;
static xr_string s_LastSceneSaveError;
static bool s_LastSceneLoadAttempted = false;
static bool s_LastSceneLoadSucceeded = false;
static bool s_LastSceneLoadRollbackOk = true;
static xr_string s_LastSceneLoadPath;
static xr_string s_LastSceneLoadError;

CCommandVar CommandLoad(CCommandVar p1, CCommandVar p2)
{
	const u32 load_flags = p2.IsInteger() ? u32(p2) : 0;
	const bool authorized_discard = !!(load_flags & flSceneLoadDiscard);
	const bool suppress_dialog = !!(load_flags & flSceneLoadSuppressDialog);
	if (p1.IsString())
	{
		s_LastSceneLoadAttempted = true;
		s_LastSceneLoadSucceeded = false;
		s_LastSceneLoadRollbackOk = true;
		s_LastSceneLoadPath = xr_string(p1);
		xr_strlwr(s_LastSceneLoadPath);
		s_LastSceneLoadError.clear();
	}
    xr_string recovery_error;
    if (!EditorRecovery::CanReplaceScene(recovery_error))
    {
		s_LastSceneLoadAttempted = true;
		s_LastSceneLoadSucceeded = false;
		s_LastSceneLoadRollbackOk = true;
		if (!p1.IsString())
			s_LastSceneLoadPath.clear();
		s_LastSceneLoadError = recovery_error;
		if (suppress_dialog)
			Msg("! %s", recovery_error.c_str());
		else
			ELog.DlgMsg(mtError, "%s", recovery_error.c_str());
        return FALSE;
    }
    if( !Scene->locked() )
    {
    	if (!p1.IsString())
        {
        	xr_string temp_fn	= LTools->m_LastFileName.c_str();
        	if (EFS.GetOpenName	(EDevice->m_hWnd, _maps_, temp_fn ))
            	return 			ExecCommand(COMMAND_LOAD,temp_fn);
        }else
        {
	        xr_string temp_fn		= p1;
            xr_strlwr				(temp_fn);
			s_LastSceneLoadAttempted = true;
			s_LastSceneLoadSucceeded = false;
			s_LastSceneLoadRollbackOk = true;
			s_LastSceneLoadPath = temp_fn;
			s_LastSceneLoadError.clear();

            xr_string why;
            if (!EScene::IsSceneFile(temp_fn.c_str(), why))
            {
				s_LastSceneLoadError = why;
				if (suppress_dialog)
					Msg("! Can't load map '%s': %s", temp_fn.c_str(), why.c_str());
				else
					ELog.DlgMsg(mtError, "Can't load map '%s': %s", temp_fn.c_str(), why.c_str());
                return FALSE;
            }

			bool discarded_changes = false;
			if (!authorized_discard && !Scene->IfModified(&discarded_changes))
			{
				s_LastSceneLoadError = "Scene replacement was cancelled";
            	return FALSE;
			}

            SProgressOperation operation(*UI, false);
            UI->SetStatus			("Level loading...");
			xr_string load_error;
			const bool res = Scene->LoadTransactional(temp_fn.c_str(), load_error);
			UI->ResetStatus();

            if (res)
            {
				LTools->m_LastFileName = temp_fn.c_str();
				LTools->m_LastSelectionName.clear();
                Scene->UndoClear	();
                
                BOOL bk1 			= Scene->m_RTFlags.test(EScene::flRT_Unsaved);
                BOOL bk2 			= Scene->m_RTFlags.test(EScene::flRT_Modified);

                Scene->UndoSave		();

                 Scene->m_RTFlags.set(EScene::flRT_Unsaved,bk1);
                 Scene->m_RTFlags.set(EScene::flRT_Modified,bk2);

				ExecCommand			(COMMAND_CLEAN_LIBRARY);
                ExecCommand			(COMMAND_UPDATE_CAPTION);
				ExecCommand(COMMAND_CHANGE_TARGET, OBJCLASS_SCENEOBJECT);
				ExecCommand(COMMAND_CHANGE_ACTION, etaSelect, estDefault);
                EPrefs->AppendRecentFile(temp_fn.c_str());
                EditorRecovery::OnSceneReplaced();
				EditorProject::NoteSceneLoaded();
				s_LastSceneLoadSucceeded = true;
				s_LastSceneLoadError.clear();
            }else
            {
				s_LastSceneLoadRollbackOk = Scene->LastLoadRollbackSucceeded();
				s_LastSceneLoadError = load_error.empty() ? "Scene load failed" : load_error;
				if (s_LastSceneLoadRollbackOk && discarded_changes)
				{
					Scene->m_RTFlags.set(EScene::flRT_Unsaved | EScene::flRT_Modified, TRUE);
					ExecCommand(COMMAND_UPDATE_CAPTION);
					EditorRecovery::RequestSnapshot();
				}
				if (!s_LastSceneLoadRollbackOk)
					EditorRecovery::RequestSnapshot();
				if (suppress_dialog)
					Msg("! Can't load map '%s': %s", temp_fn.c_str(), s_LastSceneLoadError.c_str());
				else
					ELog.DlgMsg(mtError, "Can't load map '%s': %s", temp_fn.c_str(),
						s_LastSceneLoadError.c_str());
            }
            // update props
            ExecCommand			(COMMAND_UPDATE_PROPERTIES);
            UI->RedrawScene		();             
            return res ? TRUE : FALSE;
        }
    } else {
		s_LastSceneLoadAttempted = true;
		s_LastSceneLoadSucceeded = false;
		s_LastSceneLoadRollbackOk = true;
		s_LastSceneLoadError = "Scene sharing violation";
		if (suppress_dialog)
			Msg("! Scene sharing violation");
		else
			ELog.DlgMsg(mtError, "Scene sharing violation");
        return FALSE;
    }
    return TRUE;
}

CCommandVar CommandSaveBackup(CCommandVar p1, CCommandVar p2)
{
    string_path 	fn;
    strconcat(sizeof(fn),fn,Core.CompName,"_",Core.UserName,"_backup.level");
    FS.update_path	(fn,_maps_,fn);
    const bool previous = s_SuppressRecoveryCleanup;
    const bool previous_metadata = s_SuppressSceneSaveMetadata;
    const BOOL was_unsaved = Scene->m_RTFlags.test(EScene::flRT_Unsaved);
    s_SuppressRecoveryCleanup = true;
    s_SuppressSceneSaveMetadata = true;
    CCommandVar result = ExecCommand(COMMAND_SAVE,xr_string(fn));
    s_SuppressSceneSaveMetadata = previous_metadata;
    s_SuppressRecoveryCleanup = previous;
    Scene->m_RTFlags.set(EScene::flRT_Unsaved, was_unsaved);
    if ((BOOL)result)
        EditorRecovery::OnBackupSaved();
    return result;
}
CCommandVar CommandSave(CCommandVar p1, CCommandVar p2)
{
	s_LastSceneSaveError.clear();
    if( !Scene->locked() )
    {
        if (p2==1)
        {
            xr_string temp_fn	= LTools->m_LastFileName.c_str();
            if (EFS.GetSaveName	( _maps_, temp_fn ))
                return 			ExecCommand(COMMAND_SAVE,temp_fn, 66);
            else
                return          FALSE;
        }else{
            if (p1.IsInteger())
            	return 			ExecCommand(COMMAND_SAVE,xr_string(LTools->m_LastFileName.c_str()),0);
                
            xr_string temp_fn	= xr_string(p1);
            if (temp_fn.empty())
            {
                return 			ExecCommand(COMMAND_SAVE,temp_fn,1);
            }
            else
            {
                xr_strlwr(temp_fn);

                UI->SetStatus("Level saving...");
				xr_string save_error;
				bool saved = false;
                if (xrGameManager::GetGame() == EGame::SHOC)
                {
					saved = Scene->Save(temp_fn.c_str(), true, (p2 == 66), &save_error);
                }
                else
                {
					saved = Scene->SaveLTX(temp_fn.c_str(), false, (p2 == 66), &save_error);
                }
                UI->ResetStatus	();
				if (!saved)
				{
					s_LastSceneSaveError = save_error.empty() ? "Scene save failed" : save_error;
					if (s_SuppressSceneSaveDialog)
						Msg("! %s", s_LastSceneSaveError.c_str());
					else
						ELog.DlgMsg(mtError, "%s", s_LastSceneSaveError.c_str());
					return FALSE;
				}
				s_LastSceneSaveError.clear();
                if (!s_SuppressSceneSaveMetadata && 0!=xr_strcmp(Tools->m_LastFileName.c_str(),temp_fn.c_str()))
                {
    	            Tools->m_LastFileName 	= temp_fn.c_str();
                }
                if (!s_SuppressSceneSaveMetadata)
                {
                    ExecCommand		(COMMAND_UPDATE_CAPTION);
                    EPrefs->AppendRecentFile(temp_fn.c_str());
                }
                if (!s_SuppressRecoveryCleanup)
                {
                    Scene->m_RTFlags.set(EScene::flRT_Unsaved, FALSE);
                    EditorRecovery::OnSceneSaved();
                }
                return 			TRUE;
            }
        }
    } else {
        s_LastSceneSaveError = "Scene sharing violation";
        ELog.DlgMsg			( mtError, "Scene sharing violation" );
        return				FALSE;
    }
}

CCommandVar CommandClear(CCommandVar p1, CCommandVar p2)
{
    xr_string recovery_error;
    if (!EditorRecovery::CanReplaceScene(recovery_error))
    {
        ELog.DlgMsg(mtError, "%s", recovery_error.c_str());
        return FALSE;
    }
    if( !Scene->locked() ){
        if (!Scene->IfModified()) return TRUE;
        EDevice->m_Camera.Reset	();
		ViewportNavigation::ResetState();
        Scene->Reset			();
        Scene->m_LevelOp.Reset	();
        Tools->m_LastFileName 		= "";
        LTools->m_LastSelectionName = "";
        Scene->UndoClear		();
        ExecCommand				(COMMAND_CHANGE_TARGET,OBJCLASS_SCENEOBJECT);
        ExecCommand				(COMMAND_CHANGE_ACTION,etaSelect,estDefault);
        ExecCommand				(COMMAND_UPDATE_PROPERTIES,1);
        Scene->UndoSave			();
		// The empty undo baseline is initialization state, not a user edit.
		Scene->m_RTFlags.set(EScene::flRT_Unsaved | EScene::flRT_Modified, FALSE);
		ExecCommand				(COMMAND_UPDATE_CAPTION);
		EditorRecovery::OnSceneReplaced();
        return 					TRUE;
    } else {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
        return					FALSE;
    }
}
CCommandVar CommandLoadFirstRecent(CCommandVar p1, CCommandVar p2)
{
    if (EPrefs->FirstRecentFile())
        return 					ExecCommand(COMMAND_LOAD,xr_string(EPrefs->FirstRecentFile()));
    return 						FALSE;
}

CCommandVar CommandClearDebugDraw(CCommandVar p1, CCommandVar p2)
{
    Tools->ClearDebugDraw		();
    UI->RedrawScene				();
    return 						TRUE;
}
CCommandVar CommandShowClipEditor(CCommandVar p1, CCommandVar p2)
{
/*	if(g_clip_maker==NULL)
       g_clip_maker = TClipMaker::CreateForm();

    if(!g_clip_maker->Visible)	
    {
		ESceneCustomOTool* st = Scene->GetOTool(OBJCLASS_SPAWNPOINT);

    	ObjectList&	ol = st->GetObjects();
        ObjectList::iterator it = ol.begin();    
        ObjectList::iterator it_e = ol.end();    

		CCustomObject* CO = NULL;
        for(;it!=it_e;++it)
        {
        	if((*it)->Selected()==true)
            {
            	CO=*it;
                break;
            }
        }
        if(!CO)
        	return TRUE;
            
    	CSpawnPoint* sp = dynamic_cast<CSpawnPoint*>(CO);

        
    	CKinematicsAnimated* KA 	= PKinematicsAnimated(sp->m_SpawnData.m_Visual->visual);
        R_ASSERT					(KA);
   		g_clip_maker->ShowEditor	(KA);
    }*/
    return 							TRUE;
}

CCommandVar CommandImportCompilerError(CCommandVar p1, CCommandVar p2)
{
    xr_string fn;
    if(EFS.GetOpenName(EDevice->m_hWnd,"$logs$", fn, false, NULL, 0)){
        Scene->LoadCompilerError(fn.c_str());
    }
    UI->RedrawScene		();
    return TRUE;
}
CCommandVar CommandExportCompilerError(CCommandVar p1, CCommandVar p2)
{
    xr_string fn;
    if(EFS.GetSaveName("$logs$", fn, NULL, 0)){
        Scene->SaveCompilerError(fn.c_str());
    }
    return TRUE;
}
CCommandVar CommandValidateScene(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->Validate	(true,true,true,true,true,true);
	    return 			TRUE;
    } else {
        ELog.DlgMsg		( mtError, "Scene sharing violation" );
	    return 			FALSE;
    }
}
CCommandVar CommandCleanLibrary(CCommandVar p1, CCommandVar p2)
{
    if ( !Scene->locked() ){
        Lib.CleanLibrary();
        return 			TRUE;
    }else{
        ELog.DlgMsg		(mtError, "Scene must be empty before refreshing library!");
        return 			FALSE;
    }
}

CCommandVar CommandReloadObjects(CCommandVar p1, CCommandVar p2)
{
    Lib.ReloadObjects	();
    return 				TRUE;
}

CCommandVar CommandCut(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->CutSelection(LTools->CurrentClassID());
       /* fraLeftBar->miPaste->Enabled = true;
        fraLeftBar->miPaste2->Enabled = true;*/
        Scene->UndoSave	();
        return 			TRUE;
    } else {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
        return 			FALSE;
    }
    return FALSE;
}
CCommandVar CommandCopy(CCommandVar p1, CCommandVar p2)
{
      if( !Scene->locked() ){
        Scene->CopySelection(LTools->CurrentClassID());
        return 			TRUE;
    } else {
        ELog.DlgMsg		( mtError, "Scene sharing violation" );
        return 			FALSE;
    }
    return FALSE;
}

CCommandVar CommandPaste(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->PasteSelection();
        Scene->UndoSave	();
        return 			TRUE;
    } else {
        ELog.DlgMsg		( mtError, "Scene sharing violation" );
        return  		FALSE;
    }
    return FALSE;
}

CCommandVar CommandLoadSelection(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() )
    {
        xr_string fn			= LTools->m_LastSelectionName;
        if( EFS.GetOpenName(EDevice->m_hWnd, _maps_, fn ) )
        {
        	LPCSTR maps_path	= FS.get_path(_maps_)->m_Path;
        	if (fn.c_str()==strstr(fn.c_str(),maps_path))
		        LTools->m_LastSelectionName = fn.c_str()+xr_strlen(maps_path);
            UI->SetStatus		("Fragment loading...");

            Scene->LoadSelection(fn.c_str());

            UI->ResetStatus		();
            Scene->UndoSave		();
            ExecCommand			(COMMAND_CHANGE_ACTION,etaSelect);
            ExecCommand			(COMMAND_UPDATE_PROPERTIES);
            UI->RedrawScene		();
	        return 				TRUE;
        }               	
    } else {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
    }
    return FALSE;
}        
CCommandVar CommandSaveSelection(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        xr_string fn			= LTools->m_LastSelectionName;
        if( EFS.GetSaveName		( _maps_, fn ) ){
        	LPCSTR maps_path	= FS.get_path(_maps_)->m_Path;
        	if (fn.c_str()==strstr(fn.c_str(),maps_path))
		        LTools->m_LastSelectionName = fn.c_str()+xr_strlen(maps_path);
            UI->SetStatus		("Fragment saving...");
            Scene->SaveSelection(LTools->CurrentClassID(),fn.c_str());
            UI->ResetStatus		();
	        return 				TRUE;
        }
    } else {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
    }
    return 						FALSE;
}

CCommandVar CommandUndo(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
		if (LTools->GetGimzo()->IsFixed())
			LTools->GetGimzo()->Clear();
		if (!Scene->Undo())
			ELog.DlgMsg(Scene->UndoRollbackFailed() ? mtError : mtInformation,
				Scene->UndoRollbackFailed() ? "Undo failed to restore every object; the scene was marked modified" : "Undo buffer empty");
        else{
            LTools->Reset		();
            ExecCommand			(COMMAND_CHANGE_ACTION, etaSelect);
	        return 				TRUE;
        }
    } else {
        ELog.DlgMsg				( mtError, "Scene sharing violation" );
    }
    return 						FALSE;
}

CCommandVar CommandRedo(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
		if (LTools->GetGimzo()->IsFixed())
			LTools->GetGimzo()->Clear();
		if (!Scene->Redo())
			ELog.DlgMsg(Scene->UndoRollbackFailed() ? mtError : mtInformation,
				Scene->UndoRollbackFailed() ? "Redo failed to restore every object; the scene was marked modified" : "Redo buffer empty");
        else{
            LTools->Reset		();
            ExecCommand			(COMMAND_CHANGE_ACTION, etaSelect);
		    return 				TRUE;
        }
    } else {
        ELog.DlgMsg				( mtError, "Scene sharing violation" );
    }
    return 						FALSE;
}

CCommandVar CommandClearSceneSummary(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->ClearSummaryInfo	();
	    return 					TRUE;
    } else {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
	    return 					FALSE;
    }
}
CCommandVar CommandCollectSceneSummary(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->CollectSummaryInfo();
	    return 					TRUE;
    } else {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
	    return 					FALSE;
    }
}
CCommandVar CommandShowSceneSummary(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->ShowSummaryInfo();
	    return 					TRUE;
    } else {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
	    return 					FALSE;
    }
}
CCommandVar CommandExportSceneSummary(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->ExportSummaryInfo(xr_string(p1).c_str());
	    return 					TRUE;
    } else {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
	    return 					FALSE;
    }
}

CCommandVar CommandSceneHighlightTexture(CCommandVar p1, CCommandVar p2)
{
    /*if( !Scene->locked() ){
        LPCSTR new_val 		 	= 0;
		if (TfrmChoseItem::SelectItem(smTexture,new_val,1)){
	       	Scene->HighlightTexture(new_val,false,0,0,false);
		    return 				TRUE;
        }
    } else {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
    }
    return 						FALSE;*/
    return FALSE;
}

CCommandVar CommandOptions(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        ExecCommand				(COMMAND_SHOW_PROPERTIES, p1, p2);
	    return 					TRUE;
    } else {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
	    return 					FALSE;
    }
}

// The level builds write into the scene's level folder (SceneBuilder::PreparePath), and Make All empties it first:
// the question names the folder. A scene that names no folder is not asked about - the build stops with its own error.
static bool ConfirmLevelBuild(u32 command, CCommandVar p1, CCommandVar p2, LPCSTR title, LPCSTR id, LPCSTR headline,
    LPCSTR consequence, bool destructive)
{
    if (CommandConfirmed(p2) || !Scene->m_LevelOp.m_FNLevelPath.size())
        return true;
    string_path folder;
    FS.update_path(folder, _game_levels_, Scene->m_LevelOp.m_FNLevelPath.c_str());
    XUI::ConfirmSpec spec;
    spec.id = id;
    spec.title = title;
    spec.headline = headline;
    spec.items.push_back(folder);
    spec.consequence = consequence;
    spec.action = title;
    spec.destructive = destructive;
    return ConfirmCommand(command, p1, p2, spec);
}

CCommandVar CommandBuild(CCommandVar p1, CCommandVar p2)
{
    if (Scene->locked())
    {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
        return FALSE;
    }
    if (!ConfirmLevelBuild(COMMAND_BUILD, p1, p2, "Make All", "make_all", "Empty the level folder and build every level file of this scene into it?", "Everything in the folder is deleted for good first, then built anew.", true))
        return FALSE;
    return Builder.Compile(false);
}
CCommandVar CommandMakeAIMap(CCommandVar p1, CCommandVar p2)
{
    if (Scene->locked())
    {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
        return FALSE;
    }
    if (!ConfirmLevelBuild(COMMAND_MAKE_AIMAP, p1, p2, "Make AI Map", "make_aimap", "Export the AI map of this scene into its level folder?", "The AI map there is replaced.", false))
        return FALSE;
    return Builder.MakeAIMap( );
}
CCommandVar CommandMakeGame(CCommandVar p1, CCommandVar p2)
{
    if (Scene->locked())
    {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
        return FALSE;
    }
    if (!ConfirmLevelBuild(COMMAND_MAKE_GAME, p1, p2, "Make Game", "make_game", "Write the game files of this scene into its level folder?", "Files of the same name there are replaced.", false))
        return FALSE;
    return Builder.MakeGame( );
}
CCommandVar CommandMakeDetails(CCommandVar p1, CCommandVar p2)
{
    if (Scene->locked())
    {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
        return FALSE;
    }
    if (!ConfirmLevelBuild(COMMAND_MAKE_DETAILS, p1, p2, "Make Details", "make_details", "Export the detail objects of this scene into its level folder?", "The detail objects there are replaced.", false))
        return FALSE;
    return Builder.MakeDetails();
}
CCommandVar CommandMakeHOM(CCommandVar p1, CCommandVar p2)
{
    if (Scene->locked())
    {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
        return FALSE;
    }
    if (!ConfirmLevelBuild(COMMAND_MAKE_HOM, p1, p2, "Make HOM", "make_hom", "Build the occlusion model (HOM) of this scene into its level folder?", "The occlusion model there is replaced.", false))
        return FALSE;
    return Builder.MakeHOM();
}
CCommandVar CommandMakeSOM(CCommandVar p1, CCommandVar p2)
{
    if (Scene->locked())
    {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
        return FALSE;
    }
    if (!ConfirmLevelBuild(COMMAND_MAKE_SOM, p1, p2, "Make SOM", "make_som", "Build the sound occlusion model (SOM) of this scene into its level folder?", "The sound occlusion model there is replaced.", false))
        return FALSE;
    return Builder.MakeSOM();
}
CCommandVar CommandInvertSelectionAll(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->InvertSelection	(LTools->CurrentClassID());
	    return 					TRUE;
    } else {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
    }
    return 						FALSE;
}

CCommandVar CommandSelectAll(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->SelectObjects	(true,LTools->CurrentClassID());
	    return 					TRUE;
    } else {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
    }
    return 						FALSE;
}

CCommandVar CommandDeselectAll(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->SelectObjects	(false,LTools->CurrentClassID());
	    return 					TRUE;
    } else {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
	    return 					FALSE;
    }
}

CCommandVar CommandDeleteSelection(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->RemoveSelection	( LTools->CurrentClassID() );
        Scene->UndoSave			();
        return					TRUE;
    } else {
        ELog.DlgMsg( mtError, "Scene sharing violation" );
	    return 					FALSE;
    }
}

CCommandVar CommandHideUnsel(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->ShowObjects		( false, LTools->CurrentClassID(), true, false );
        Scene->UndoSave			();
        ExecCommand				(COMMAND_UPDATE_PROPERTIES);
	    return 					TRUE;
    } else {
        ELog.DlgMsg				( mtError, "Scene sharing violation" );
	    return 					FALSE;
    }
}
CCommandVar CommandHideSel(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->ShowObjects		( bool(p1), LTools->CurrentClassID(), true, true );
        Scene->UndoSave			();
        ExecCommand				(COMMAND_UPDATE_PROPERTIES);
	    return 					TRUE;
    } else {
        ELog.DlgMsg				( mtError, "Scene sharing violation" );
	    return 					FALSE;
    }
}
CCommandVar CommandHideAll(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->ShowObjects		( bool(p1), LTools->CurrentClassID(), false );
        Scene->UndoSave			();
        ExecCommand				(COMMAND_UPDATE_PROPERTIES);
	    return 					TRUE;
    }else{
        ELog.DlgMsg				( mtError, "Scene sharing violation" );
	    return 					FALSE;
    }
}
CCommandVar CommandSetSnapObjects(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->SetSnapList		();
	    return 					TRUE;
    }else{
        ELog.DlgMsg				( mtError, "Scene sharing violation" );
	    return 					FALSE;
    }
}
CCommandVar CommandAddSelSnapObjects(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->AddSelToSnapList	();
	    return 					TRUE;
    }else{
        ELog.DlgMsg				( mtError, "Scene sharing violation" );
	    return 					FALSE;
    }
}
CCommandVar CommandDelSelSnapObjects(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->DelSelFromSnapList();
	    return 					TRUE;
    }else{
        ELog.DlgMsg				( mtError, "Scene sharing violation" );
	    return 					FALSE;
    }
}
CCommandVar CommandClearSnapObjects(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->ClearSnapList	(true);
	    return 					TRUE;
    }else{
        ELog.DlgMsg				( mtError, "Scene sharing violation" );
	    return 					FALSE;
    }
}
CCommandVar CommandSelectSnapObjects(CCommandVar p1, CCommandVar p2)
{
    if( !Scene->locked() ){
        Scene->SelectSnapList	();
	    return 					TRUE;
    }else{
        ELog.DlgMsg				( mtError, "Scene sharing violation" );
	    return 					FALSE;
    }
}
CCommandVar CommandRefreshSnapObjects(CCommandVar p1, CCommandVar p2)
{
 //   fraLeftBar->UpdateSnapList();
    return 						TRUE;
}
/*
CCommandVar CommandRefreshSoundEnvs(CCommandVar p1, CCommandVar p2)
{
    ::Sound->refresh_env_library();
    return 						TRUE;
//		::Sound->_restart();
}
*/

CCommandVar CommandRefreshSoundEnvGeometry(CCommandVar p1, CCommandVar p2)
{
    LSndLib->RefreshEnvGeometry();
    return 						TRUE;
}
CCommandVar CommandShowContextMenu(CCommandVar p1, CCommandVar p2)
{
    LUI->ShowContextMenu		(p1);
    return 						TRUE;
}
//------        
CCommandVar CommandRefreshUIBar(CCommandVar p1, CCommandVar p2)
{
    if(MainForm)
        if(MainForm->GetTopBarForm())
            MainForm->GetTopBarForm()->RefreshBar();
    /*fraTopBar->RefreshBar		();
    fraLeftBar->RefreshBar		();
    fraBottomBar->RefreshBar	();*/
    return 						TRUE;
}
CCommandVar CommandRestoreUIBar(CCommandVar p1, CCommandVar p2)
{
    /*fraTopBar->fsStorage->RestoreFormPlacement();
    fraLeftBar->fsStorage->RestoreFormPlacement();
    fraBottomBar->fsStorage->RestoreFormPlacement();*/
    return 						TRUE;
}
CCommandVar CommandSaveUIBar(CCommandVar p1, CCommandVar p2)
{
   /* fraTopBar->fsStorage->SaveFormPlacement();
    fraLeftBar->fsStorage->SaveFormPlacement();
    fraBottomBar->fsStorage->SaveFormPlacement();*/
    return 						TRUE;
}
CCommandVar CommandUpdateToolBar(CCommandVar p1, CCommandVar p2)
{
 /*   fraLeftBar->UpdateBar		();*/
    return 						TRUE;
}
CCommandVar CommandUpdateCaption(CCommandVar p1, CCommandVar p2)
{
  /*  frmMain->UpdateCaption		();*/
    return 						TRUE;
}
//------
CCommandVar CommandCreateSoundLib(CCommandVar p1, CCommandVar p2)
{
    SndLib						= xr_new<CLevelSoundManager>();
    LSndLib = (CLevelSoundManager*)SndLib;
    return 						TRUE;
}

extern BOOL ai_map_shown;
CCommandVar CommandToggleAiMapVisibility(CCommandVar p1, CCommandVar p2)
{
	ai_map_shown 				= !ai_map_shown;
    return 						TRUE;
}

// UE-style gizmo hotkeys: Q/W/E/R pick the mode, Space (EType::Count sentinel)
// cycles Move -> Rotate -> Scale. Unsupported modes are auto-reset by the toolbar.
CCommandVar CommandGizmoMode(CCommandVar p1, CCommandVar p2)
{
    LTools->SetAction			(etaSelect);
    Gizmo* G					= LTools->GetGimzo();
    const Gizmo::EType mode		= (Gizmo::EType)u32(p1);
    if (mode == Gizmo::EType::Count){
        switch (G->GetType()){
        case Gizmo::EType::Move:	G->SetType(Gizmo::EType::Rotate);	break;
        case Gizmo::EType::Rotate:	G->SetType(Gizmo::EType::Scale);	break;
        default:					G->SetType(Gizmo::EType::Move);		break;
        }
    }
    else							G->SetType(mode);
    UI->RedrawScene				();
    return 						TRUE;
}

// Places a library object into the scene — mirrors TUI_ControlObjectAdd::Start,
// but takes the asset name from the content browser instead of the tool panel.
// p2==0 -> raycast down the view centre, p2==1 -> raycast under the mouse.
CCommandVar CommandCBPlaceAsset(CCommandVar p1, CCommandVar p2)
{
    // hold the string: xr_string(p1) is a temporary, its buffer dies at the
    // end of this statement and the name would dangle for the rest of the call
    const xr_string asset_name = p1;
    LPCSTR name = asset_name.c_str();
    if (!name[0]) return FALSE;

    // p2 bit0 = under the cursor, bit1 = raw engine visual (.ogf) instead of
    // a library reference
    const bool as_visual = 0 != (u32(p2) & 2);
    Fvector start, dir;
    if (u32(p2) & 1)
    {
        start.set(UI->m_CurrentRStart);
        dir.set  (UI->m_CurrentRDir);
    }
    else
    {
        // centre of the viewport
        Ivector2 c; c.set(UI->GetRenderWidth()/2, UI->GetRenderHeight()/2);
        EDevice->m_Camera.MouseRayFromPoint(start, dir, c);
    }

    Fvector p, n;
    if (!LUI->PickGround(p, start, dir, 1, &n))
    {
        // nothing under the ray — drop it a few metres ahead of the camera
        p.mad(EDevice->m_Camera.GetPosition(), dir, 10.f);
        n.set(0.f, 1.f, 0.f);
    }

    // A visual-mode object is named by its LEAF: "meshes\dynamics\barrel.ogf"
    // places as "barrel_0000", not as a path with an extension in it. A
    // library reference keeps the SDK convention of path-shaped names.
    string_path base_name;
    xr_strcpy(base_name, sizeof(base_name), name);
    if (as_visual)
    {
        LPCSTR leaf = strrchr(name, '\\');
        LPCSTR alt  = strrchr(name, '/');
        if (alt > leaf) leaf = alt;
        xr_strcpy(base_name, sizeof(base_name), leaf ? leaf + 1 : name);
        if (char* dot = strrchr(base_name, '.')) *dot = 0;
    }

    string256 namebuffer;
    Scene->GenObjectName(OBJCLASS_SCENEOBJECT, namebuffer, base_name);
    CSceneObject* obj = xr_new<CSceneObject>((LPVOID)0, namebuffer);
    const bool attached = as_visual ? obj->SetVisual(name) : (0 != obj->SetReference(name));
    if (!attached)
    {
        ELog.DlgMsg(mtError, "Content Browser: can't load %s '%s'.",
                    as_visual ? "model" : "reference object", name);
        xr_delete(obj);
        return FALSE;
    }
    obj->MoveTo(p, n);

    if (LTools->CurrentClassID() != OBJCLASS_SCENEOBJECT)
        LTools->SetTarget(OBJCLASS_SCENEOBJECT, 0);

    Scene->SelectObjects(false, OBJCLASS_SCENEOBJECT);
    Scene->AppendObject (obj);
    UI->RedrawScene     ();
    return TRUE;
}

CCommandVar CommandShowContentBrowser(CCommandVar p1, CCommandVar p2)
{
    if (UIContentBrowser::IsOpen())	UIContentBrowser::Close();
    else
    {
        // opened where it can be seen: the panel on Level, the drawer on a document
        UIContentBrowser::Show();
        UIContentBrowser::MakeVisible();
    }
    return TRUE;
}

CCommandVar CommandShowWorldOutliner(CCommandVar p1, CCommandVar p2)
{
    if (UIWorldOutliner::IsOpen())	UIWorldOutliner::Close();
    else							UIWorldOutliner::Focus(false);
    return TRUE;
}

//------------------------------------------------------------------------------
// drop to ground (the End key, like Unreal)
//------------------------------------------------------------------------------
// kDropReach and RetrieveSceneObjPointAndNormal: Mcp\LevelMcp.h (place_way drops its points the same way)

// Clips a triangle to an XZ rectangle (Sutherland-Hodgman) keeping the height
// of every produced point. Y is linear across a triangle, so the tallest point
// of the clipped polygon is the exact highest support inside the footprint -
// this is what makes a wide crate rest on the rim of a narrow hole instead of
// slipping through it.
static u32 ClipTriangleToRect(const Fvector* tri, float x0, float x1, float z0, float z1, Fvector* out)
{
    Fvector buf_a[16], buf_b[16];
    Fvector* src = buf_a;
    Fvector* dst = buf_b;
    u32 count = 3;
    src[0] = tri[0]; src[1] = tri[1]; src[2] = tri[2];

    // plane order: x>=x0, x<=x1, z>=z0, z<=z1
    for (int plane = 0; plane < 4 && count; ++plane)
    {
        u32 produced = 0;
        for (u32 i = 0; i < count; ++i)
        {
            const Fvector& cur = src[i];
            const Fvector& nxt = src[(i + 1) % count];
            float cur_d, nxt_d;
            switch (plane)
            {
            case 0: cur_d = cur.x - x0; nxt_d = nxt.x - x0; break;
            case 1: cur_d = x1 - cur.x; nxt_d = x1 - nxt.x; break;
            case 2: cur_d = cur.z - z0; nxt_d = nxt.z - z0; break;
            default: cur_d = z1 - cur.z; nxt_d = z1 - nxt.z; break;
            }
            const bool cur_in = cur_d >= 0.f;
            const bool nxt_in = nxt_d >= 0.f;
            if (cur_in && produced < 15)
                dst[produced++] = cur;
            if (cur_in != nxt_in && produced < 15)
            {
                const float t = cur_d / (cur_d - nxt_d);
                Fvector cross;
                cross.lerp(cur, nxt, t);
                dst[produced++] = cross;
            }
        }
        count = produced;
        std::swap(src, dst);
    }

    for (u32 i = 0; i < count; ++i)
        out[i] = src[i];
    return count;
}

// Settles objects onto the surface below them exactly the way Unreal's End key
// does: UE calls FindSuitableTransformAlongPath with FCollisionShape::MakeBox
// of the element bounds, i.e. it sweeps the whole bounding box straight down
// (EditorServer.cpp, UEditorEngine::SnapElementTo, bUseLineTrace=false).
// Here the same sweep is solved analytically - every triangle in the swept
// column is clipped to the footprint and the tallest clipped point wins - so
// the result is exact rather than sampled.
// line_trace mirrors UE's Alt+End variant: a single ray from the pivot.
u32 DropObjectsToGround(ObjectList& objects, bool align_normal, bool line_trace)
{
    if (objects.empty() || !Scene->GetOTool(OBJCLASS_SCENEOBJECT))
        return 0;

    ObjectList candidates;
    ObjectList& all = Scene->ListObj(OBJCLASS_SCENEOBJECT);
    for (ObjectIt it = all.begin(); it != all.end(); ++it)
    {
        if (!(*it)->Visible()) continue;
        if (std::find(objects.begin(), objects.end(), *it) != objects.end()) continue;
        candidates.push_back(*it);
    }
    if (candidates.empty())
        return 0;

    u32 moved = 0;
    for (ObjectIt it = objects.begin(); it != objects.end(); ++it)
    {
        CCustomObject* obj = *it;
        Fbox box;
        if (!obj->GetBox(box)) continue;

        // a vertical sweep can only ever touch objects overlapping in XZ
        ObjectList floor;
        for (ObjectIt f = candidates.begin(); f != candidates.end(); ++f)
        {
            Fbox fb;
            if (!(*f)->GetBox(fb)) continue;
            if (fb.max.x < box.min.x - EPS_L || fb.min.x > box.max.x + EPS_L) continue;
            if (fb.max.z < box.min.z - EPS_L || fb.min.z > box.max.z + EPS_L) continue;
            if (fb.min.y > box.max.y) continue;	// entirely above: cannot support
            floor.push_back(*f);
        }
        if (floor.empty()) continue;

        float drop = flt_max;
        Fvector contact_normal;
        contact_normal.set(0.f, 1.f, 0.f);

        if (line_trace)
        {
            Fvector start = obj->GetPosition();
            start.y += EPS_L;
            Fvector down; down.set(0.f, -1.f, 0.f);
            SRayPickInfo pinf;
            pinf.inf.range = kDropReach;
            if (Scene->RayPickObject(pinf.inf.range, start, down, OBJCLASS_SCENEOBJECT, &pinf, &floor))
            {
                Fvector hit, normal;
                normal.set(0.f, 1.f, 0.f);
                RetrieveSceneObjPointAndNormal(hit, &normal, pinf, 0);
                if (_valid(hit))
                {
                    drop = obj->GetPosition().y - hit.y;
                    if (_valid(normal)) contact_normal = normal;
                }
            }
        }
        else
        {
            // the volume the box bottom passes through on its way down
            Fbox sweep;
            sweep.set(box.min.x, box.min.y - kDropReach, box.min.z,
                      box.max.x, box.min.y + EPS_L,     box.max.z);

            SPickQuery PQ;
            if (Scene->BoxQuery(PQ, sweep, CDB::OPT_FULL_TEST, &floor))
            {
                float best_y = -flt_max;
                SPickQuery::SResult* results = PQ.r_begin();
                for (int r = 0; r < PQ.r_count(); ++r)
                {
                    Fvector clipped[16];
                    const u32 n = ClipTriangleToRect(results[r].verts, box.min.x, box.max.x,
                        box.min.z, box.max.z, clipped);
                    for (u32 c = 0; c < n; ++c)
                    {
                        // only surfaces at or below the object can support it
                        if (clipped[c].y > box.min.y + EPS_L) continue;
                        if (clipped[c].y > best_y)
                        {
                            best_y = clipped[c].y;
                            Fvector e1, e2, n_tri;
                            e1.sub(results[r].verts[1], results[r].verts[0]);
                            e2.sub(results[r].verts[2], results[r].verts[0]);
                            n_tri.crossproduct(e1, e2);
                            if (n_tri.magnitude() > EPS_S)
                            {
                                n_tri.normalize();
                                if (n_tri.y < 0.f) n_tri.invert();
                                contact_normal = n_tri;
                            }
                        }
                    }
                }
                if (best_y > -flt_max)
                    drop = box.min.y - best_y;
            }
        }

        if (drop == flt_max || drop <= EPS_L)
            continue;	// nothing underneath, or already resting on it

        Fvector pos = obj->GetPosition();
        pos.y -= drop;
        if (align_normal)
            obj->MoveTo(pos, contact_normal);
        else
            obj->SetPosition(pos);
        ++moved;
    }
    return moved;
}

// collects the current selection across every object tool
void CollectSelectedObjects(ObjectList& out)
{
    for (SceneToolsMapPairIt it = Scene->FirstTool(); it != Scene->LastTool(); ++it)
    {
        if (it->first == OBJCLASS_DUMMY || !Scene->GetOTool(it->first)) continue;
        ObjectList& lst = Scene->ListObj(it->first);
        for (ObjectIt o = lst.begin(); o != lst.end(); ++o)
            if ((*o)->Selected())
                out.push_back(*o);
    }
}

CCommandVar CommandDropToGround(CCommandVar p1, CCommandVar p2)
{
    if (Scene->locked()) return FALSE;

    ObjectList selection;
    CollectSelectedObjects(selection);
    if (selection.empty())
    {
        ELog.Msg(mtInformation, "Drop to ground: nothing selected.");
        return FALSE;
    }

    // p2 != 0 selects the pivot line trace, mirroring UE's Alt+End
	Scene->UndoTransformBegin(selection);
    const u32 moved = DropObjectsToGround(selection, false, 0 != u32(p2));
    if (moved)
    {
		Scene->UndoTransformEnd();
        ExecCommand(COMMAND_UPDATE_PROPERTIES);
        UI->RedrawScene(true);
    }
	else
		Scene->UndoTransformCancel();
    ELog.Msg(mtInformation, "Drop to ground: %d of %d object(s) landed.", moved, selection.size());
    return moved ? TRUE : FALSE;
}

// A command rather than a plain menu handler so the shortcut editor, macros
// and MCP all reach it. The build itself runs beside the editor (EditorMod::StartJob).
CCommandVar CommandModBuild(CCommandVar p1, CCommandVar p2)
{
    EditorMod::RequestBuildIntoGame();
    return TRUE;
}

// the two Mod dialogs, reachable from the palette, a shortcut and exec_command
CCommandVar CommandModEditManifest(CCommandVar p1, CCommandVar p2)
{
    EditorMod::RequestEditManifest();
    return TRUE;
}

CCommandVar CommandModPackage(CCommandVar p1, CCommandVar p2)
{
    EditorMod::RequestPackageRelease();
    return TRUE;
}

CCommandVar CommandModPublish(CCommandVar p1, CCommandVar p2)
{
    EditorPublish::RequestPublish();
    return TRUE;
}

CCommandVar CommandModCharacters(CCommandVar p1, CCommandVar p2)
{
    EditorCharacters::Show();
    return TRUE;
}

CCommandVar CommandModWorld(CCommandVar p1, CCommandVar p2)
{
    EditorWorld::Show();
    return TRUE;
}

CCommandVar CommandModPda(CCommandVar p1, CCommandVar p2)
{
    EditorPda::ShowModuleLayout();
    return TRUE;
}

CCommandVar CommandModReleases(CCommandVar p1, CCommandVar p2)
{
    EditorPublish::RequestReleases();
    return TRUE;
}

CCommandVar CommandModCatalog(CCommandVar p1, CCommandVar p2)
{
    EditorCatalogSubmit::Request();
    return TRUE;
}

//------------------------------------------------------------------------------
// MCP: the commands live in Mcp\ (LevelMcp.cpp routes them). What stays here is
// the scene IO state the scene commands answer from.
//------------------------------------------------------------------------------
void LevelMcpSceneLoadJson(xr_string& out)
{
	out = "{\"ok\":";
	out += s_LastSceneLoadAttempted && !s_LastSceneLoadSucceeded ? "false" : "true";
	out += ",\"attempted\":";
	out += s_LastSceneLoadAttempted ? "true" : "false";
	out += ",\"loaded\":";
	out += s_LastSceneLoadSucceeded ? "true" : "false";
	out += ",\"rollback_ok\":";
	out += s_LastSceneLoadRollbackOk ? "true" : "false";
	out += ",\"path\":\"";
	out += LevelMcp::JsonStr(s_LastSceneLoadPath.c_str());
	out += "\",\"error\":\"";
	out += LevelMcp::JsonStr(s_LastSceneLoadError.c_str());
	out += "\"}";
}

bool LevelMcpSaveScene(const xr_string& file, xr_string& err)
{
	const bool previous_dialog_suppression = s_SuppressSceneSaveDialog;
	s_SuppressSceneSaveDialog = true;
	CCommandVar save_result = ExecCommand(COMMAND_SAVE, file);
	s_SuppressSceneSaveDialog = previous_dialog_suppression;
	if ((BOOL)save_result)
		return true;
	err = s_LastSceneSaveError.empty() ? "scene save failed" : s_LastSceneSaveError.c_str();
	return false;
}

void CLevelMain::RegisterCommands()
{
	inherited::RegisterCommands	();
    // tools
	REGISTER_SUB_CMD_CE	(COMMAND_CHANGE_TARGET,             "Change Target", 		LTools,CLevelTool::CommandChangeTarget, true);
		APPEND_SUB_CMD	("Object", 							OBJCLASS_SCENEOBJECT,	0);
		APPEND_SUB_CMD	("Light", 							OBJCLASS_LIGHT, 		0);
		APPEND_SUB_CMD	("Sound Source",					OBJCLASS_SOUND_SRC, 	0);
		APPEND_SUB_CMD	("Sound Env", 		                OBJCLASS_SOUND_ENV, 	0);
		APPEND_SUB_CMD	("Glow", 			                OBJCLASS_GLOW, 			0);
		APPEND_SUB_CMD	("Shape", 			                OBJCLASS_SHAPE, 		0);
		APPEND_SUB_CMD	("Spawn Point", 	                OBJCLASS_SPAWNPOINT, 	0);
		APPEND_SUB_CMD	("Way", 			                OBJCLASS_WAY, 			0);
		APPEND_SUB_CMD	("Way Point", 		                OBJCLASS_WAY, 			1);
		APPEND_SUB_CMD	("Toggle Way Mode",	                OBJCLASS_WAY, 			2);
		APPEND_SUB_CMD	("Sector", 			                OBJCLASS_SECTOR, 		0);
		APPEND_SUB_CMD	("Portal", 			                OBJCLASS_PORTAL, 		0);
		APPEND_SUB_CMD	("Group", 			                OBJCLASS_GROUP, 		0);
		APPEND_SUB_CMD	("Particle System",                 OBJCLASS_PS, 			0);
		APPEND_SUB_CMD	("Detail Objects", 	                OBJCLASS_DO, 			0);
		APPEND_SUB_CMD	("AI Map", 			                OBJCLASS_AIMAP, 		0);
		APPEND_SUB_CMD	("Static Wallmark",                 OBJCLASS_WM, 			0);
		APPEND_SUB_CMD	("Fog Volume",		                OBJCLASS_FOG_VOL, 		0);
    REGISTER_SUB_CMD_END;    
	REGISTER_CMD_C	    (COMMAND_ENABLE_TARGET,           	LTools,CLevelTool::CommandEnableTarget);
	REGISTER_CMD_C	    (COMMAND_SHOW_TARGET,           	LTools,CLevelTool::CommandShowTarget);
	REGISTER_CMD_C	    (COMMAND_READONLY_TARGET,          	LTools,CLevelTool::CommandReadonlyTarget);
	REGISTER_CMD_C	    (COMMAND_MULTI_RENAME_OBJECTS,     	LTools,CLevelTool::CommandMultiRenameObjects);

	REGISTER_CMD_CE	    (COMMAND_SHOW_OBJECTLIST,           "Scene\\Show Object List",		LTools,CLevelTool::CommandShowObjectList, false);
	// common
	REGISTER_CMD_S	    (COMMAND_LIBRARY_EDITOR,           	CommandLibraryEditor);
	REGISTER_CMD_S	    (COMMAND_LANIM_EDITOR,            	CommandLAnimEditor);
	REGISTER_CMD_SE	    (COMMAND_FILE_MENU,              	"File\\Menu",					CommandFileMenu, 		true);
    REGISTER_CMD_S		(COMMAND_LOAD_LEVEL_PART,			CommandLoadLevelPart);
    REGISTER_CMD_S		(COMMAND_UNLOAD_LEVEL_PART,			CommandUnloadLevelPart);
	REGISTER_CMD_SE	    (COMMAND_LOAD,              		"File\\Load Level", 			CommandLoad, 			true);
    REGISTER_SUB_CMD_SE (COMMAND_SAVE, 						"File",							CommandSave,			true);
    	APPEND_SUB_CMD	("Save",							0,								0);
    	APPEND_SUB_CMD	("Save As",							0,								1);
    REGISTER_SUB_CMD_END;
	REGISTER_CMD_S	    (COMMAND_SAVE_BACKUP,              	CommandSaveBackup);
	REGISTER_CMD_SE	    (COMMAND_CLEAR,              		"File\\Clear Scene", 			CommandClear,			true);
	REGISTER_CMD_SE	    (COMMAND_LOAD_FIRSTRECENT,          "File\\Load First Recent",		CommandLoadFirstRecent, true);
	REGISTER_CMD_S	    (COMMAND_CLEAR_DEBUG_DRAW, 		    CommandClearDebugDraw);
	REGISTER_CMD_S	    (COMMAND_IMPORT_COMPILER_ERROR,     CommandImportCompilerError);
	REGISTER_CMD_S	    (COMMAND_EXPORT_COMPILER_ERROR,     CommandExportCompilerError);
	REGISTER_CMD_S	    (COMMAND_VALIDATE_SCENE,            CommandValidateScene);
	REGISTER_CMD_S	    (COMMAND_CLEAN_LIBRARY,           	CommandCleanLibrary);
	REGISTER_CMD_S	    (COMMAND_RELOAD_OBJECTS,            CommandReloadObjects);
	REGISTER_CMD_SE	    (COMMAND_CUT,              			"Edit\\Cut",					CommandCut,false);
	REGISTER_CMD_SE	    (COMMAND_COPY,              		"Edit\\Copy",					CommandCopy,false);
	REGISTER_CMD_SE	    (COMMAND_PASTE,              		"Edit\\Paste",					CommandPaste,false);
	REGISTER_CMD_S	    (COMMAND_LOAD_SELECTION,            CommandLoadSelection);
	REGISTER_CMD_S	    (COMMAND_SAVE_SELECTION,            CommandSaveSelection);
	REGISTER_CMD_SE	    (COMMAND_UNDO,              		"Edit\\Undo",					CommandUndo,false);
	REGISTER_CMD_SE	    (COMMAND_REDO,              		"Edit\\Redo",					CommandRedo,false);
	REGISTER_CMD_S	    (COMMAND_CLEAR_SCENE_SUMMARY,	    CommandClearSceneSummary);
	REGISTER_CMD_S	    (COMMAND_COLLECT_SCENE_SUMMARY,     CommandCollectSceneSummary);
	REGISTER_CMD_S	    (COMMAND_SHOW_SCENE_SUMMARY,        CommandShowSceneSummary);
	REGISTER_CMD_S	    (COMMAND_EXPORT_SCENE_SUMMARY,      CommandExportSceneSummary);
	REGISTER_CMD_S	    (COMMAND_SCENE_HIGHLIGHT_TEXTURE,	CommandSceneHighlightTexture);
	REGISTER_CMD_SE	    (COMMAND_OPTIONS,              		"Scene\\Options",		        CommandOptions,false);
	REGISTER_CMD_SE	    (COMMAND_BUILD,              		"Compile\\Build",		        CommandBuild,false);
	REGISTER_CMD_SE	    (COMMAND_MAKE_GAME,              	"Compile\\Make Game",	        CommandMakeGame,false);
	REGISTER_CMD_SE	    (COMMAND_MAKE_AIMAP,              	"Compile\\Make AI Map",	        CommandMakeAIMap,false);
	REGISTER_CMD_SE	    (COMMAND_MAKE_DETAILS,              "Compile\\Make Details",        CommandMakeDetails,false);
	REGISTER_CMD_SE	    (COMMAND_MAKE_HOM,              	"Compile\\Make HOM",	        CommandMakeHOM,false);
	REGISTER_CMD_SE	    (COMMAND_MAKE_SOM,              	"Compile\\Make SOM",	        CommandMakeSOM,false);
	REGISTER_CMD_SE	    (COMMAND_INVERT_SELECTION_ALL,      "Selection\\Invert", 			CommandInvertSelectionAll,false);
	REGISTER_CMD_SE	    (COMMAND_SELECT_ALL,              	"Selection\\Select All", 		CommandSelectAll,false);
	REGISTER_CMD_SE	    (COMMAND_DESELECT_ALL,              "Selection\\Unselect All", 		CommandDeselectAll,false);
	REGISTER_CMD_SE	    (COMMAND_DELETE_SELECTION,          "Edit\\Delete", 				CommandDeleteSelection,false);
	REGISTER_CMD_SE	    (COMMAND_HIDE_UNSEL,              	"Visibility\\Hide Unselected",	CommandHideUnsel,false);
	REGISTER_CMD_SE	    (COMMAND_HIDE_SEL,              	"Visibility\\Hide Selected", 	CommandHideSel,false);
	REGISTER_CMD_SE	    (COMMAND_HIDE_ALL,              	"Visibility\\Hide All", 		CommandHideAll,false);
	REGISTER_CMD_S		(COMMAND_SET_SNAP_OBJECTS,          CommandSetSnapObjects);
	REGISTER_CMD_S	    (COMMAND_ADD_SEL_SNAP_OBJECTS,      CommandAddSelSnapObjects);
	REGISTER_CMD_S	    (COMMAND_DEL_SEL_SNAP_OBJECTS,      CommandDelSelSnapObjects);
	REGISTER_CMD_S	    (COMMAND_CLEAR_SNAP_OBJECTS,        CommandClearSnapObjects);
	REGISTER_CMD_S	    (COMMAND_SELECT_SNAP_OBJECTS,       CommandSelectSnapObjects);
	REGISTER_CMD_S	    (COMMAND_REFRESH_SNAP_OBJECTS,      CommandRefreshSnapObjects);
//	REGISTER_CMD_S	    (COMMAND_REFRESH_SOUND_ENVS,        CommandRefreshSoundEnvs);
	REGISTER_CMD_S	    (COMMAND_REFRESH_SOUND_ENV_GEOMETRY,CommandRefreshSoundEnvGeometry);
	REGISTER_SUB_CMD_SE (COMMAND_GIZMO_MODE,				"Gizmo",						CommandGizmoMode,false);
		APPEND_SUB_CMD	("Select",	u32(Gizmo::EType::None),	u32(0));
		APPEND_SUB_CMD	("Move",	u32(Gizmo::EType::Move),	u32(0));
		APPEND_SUB_CMD	("Rotate",	u32(Gizmo::EType::Rotate),	u32(0));
		APPEND_SUB_CMD	("Scale",	u32(Gizmo::EType::Scale),	u32(0));
		APPEND_SUB_CMD	("Cycle",	u32(Gizmo::EType::Count),	u32(0));
	REGISTER_SUB_CMD_END;
	REGISTER_CMD_S	    (COMMAND_CB_PLACE_ASSET,			CommandCBPlaceAsset);
	REGISTER_CMD_SE	    (COMMAND_DROP_TO_GROUND,			"Object\\Drop To Ground",		CommandDropToGround,false);
	REGISTER_CMD_SE	    (COMMAND_SHOW_CONTENT_BROWSER,		"Windows\\Content Browser",		CommandShowContentBrowser,false);
	REGISTER_CMD_SE	    (COMMAND_SHOW_WORLD_OUTLINER,		"Windows\\World Outliner",		CommandShowWorldOutliner,false);
	REGISTER_CMD_S	    (COMMAND_SHOWCONTEXTMENU,           CommandShowContextMenu);
	REGISTER_CMD_S	    (COMMAND_REFRESH_UI_BAR,            CommandRefreshUIBar);
	REGISTER_CMD_S	    (COMMAND_RESTORE_UI_BAR,            CommandRestoreUIBar);
	REGISTER_CMD_S	    (COMMAND_SAVE_UI_BAR,              	CommandSaveUIBar);
	REGISTER_CMD_S	    (COMMAND_UPDATE_TOOLBAR,            CommandUpdateToolBar);
	REGISTER_CMD_S	    (COMMAND_UPDATE_CAPTION,            CommandUpdateCaption);
	REGISTER_CMD_S	    (COMMAND_CREATE_SOUND_LIB,          CommandCreateSoundLib);
	REGISTER_CMD_SE	    (COMMAND_TOGGLE_AIMAP_VISIBILITY,   "Visibility\\Toggle AIMap",			CommandToggleAiMapVisibility,true);
	REGISTER_CMD_S	    (COMMAND_SHOW_CLIP_EDITOR,			CommandShowClipEditor);
	REGISTER_CMD_SE	    (COMMAND_MOD_BUILD,					"Mod\\Build Into Game",			CommandModBuild,true);
	REGISTER_CMD_SE	    (COMMAND_MOD_EDIT_MANIFEST,			"Mod\\Edit Manifest",			CommandModEditManifest,true);
	REGISTER_CMD_SE	    (COMMAND_MOD_PACKAGE,				"Mod\\Package Release",			CommandModPackage,true);
	REGISTER_CMD_SE	    (COMMAND_MOD_PUBLISH,				"Mod\\Publish Release",			CommandModPublish,true);
	REGISTER_CMD_SE	    (COMMAND_MOD_RELEASES,				"Mod\\Manage Releases",			CommandModReleases,true);
	REGISTER_CMD_SE	    (COMMAND_MOD_CHARACTERS,			"Mod\\Characters",				CommandModCharacters,true);
	REGISTER_CMD_SE	    (COMMAND_MOD_WORLD,					"Mod\\World",					CommandModWorld,true);
	REGISTER_CMD_SE	    (COMMAND_MOD_PDA,					"Mod\\PDA Interface",			CommandModPda,true);
	REGISTER_CMD_SE	    (COMMAND_MOD_CATALOG,				"Mod\\Submit to Mod Browser",	CommandModCatalog,true);

	// Ctrl+B out of the box. LoadShortcuts runs later and only touches keys
	// level.ini actually lists, so a rebind still wins.
	{
		ECommandVec& cmds = GetEditorCommands();
		if (COMMAND_MOD_BUILD < cmds.size() && cmds[COMMAND_MOD_BUILD] &&
			!cmds[COMMAND_MOD_BUILD]->sub_commands.empty())
			cmds[COMMAND_MOD_BUILD]->sub_commands[0]->shortcut = xr_shortcut('B', FALSE, TRUE, FALSE);
	}
}

char* CLevelMain::GetCaption()
{
	return Tools->m_LastFileName.empty()?"noname":Tools->m_LastFileName.c_str();
}

bool CLevelMain::SceneVisible() const
{
    return XUI::Shell::IsActive("level") && EditorProject::Active() && EditorProject::GameLinked();
}

bool CLevelMain::ShouldDeferCommand(u32 command) const
{
	switch (command)
	{
	case COMMAND_LOAD_SELECTION:
	case COMMAND_LOAD_LEVEL_PART:
	case COMMAND_VALIDATE_SCENE:
	case COMMAND_RELOAD_OBJECTS:
	case COMMAND_CLEAN_LIBRARY:
	case COMMAND_COLLECT_SCENE_SUMMARY:
	case COMMAND_REFRESH_SOUND_ENV_GEOMETRY:
	case COMMAND_BUILD:
	case COMMAND_MAKE_GAME:
	case COMMAND_MAKE_DETAILS:
	case COMMAND_MAKE_HOM:
	case COMMAND_MAKE_SOM:
	case COMMAND_MAKE_AIMAP:
		return true;
	default:
		return inherited::ShouldDeferCommand(command);
	}
}

bool  CLevelMain::ApplyShortCut(DWORD Key, TShiftState Shift)
{
    if (Scene->IsPlayInEditor())return true;
    if (!EditorProject::Active())return true;	// project browser owns the screen
    return inherited::ApplyShortCut(Key,Shift);
}


bool  CLevelMain::ApplyGlobalShortCut(DWORD Key, TShiftState Shift)
{
    return inherited::ApplyGlobalShortCut(Key,Shift);
}

void RetrieveSceneObjPointAndNormal( Fvector& hitpoint, Fvector* hitnormal, const SRayPickInfo &pinf, int bSnap )
{
    if(pinf.e_mesh == 0)
    {
      hitpoint = pinf.pt;
       if (hitnormal && pinf.visual_inf.K )
        	*hitnormal = pinf.visual_inf.normal;
       return;
    }
    if (Tools->GetSettings(etfVSnap) && bSnap)
    {
        Fvector pn;
        float u = pinf.inf.u;
        float v = pinf.inf.v;
        float w = 1 - (u + v);
        Fvector verts[3];
        pinf.e_obj->GetFaceWorld(pinf.s_obj->_Transform(), pinf.e_mesh, pinf.inf.id, verts);

        if ((w > u) && (w > v))
            pn.set(verts[0]);
        else if ((u > w) && (u > v))
            pn.set(verts[1]);
        else
            pn.set(verts[2]);

        hitpoint.set(pinf.pt);

    }
    else
    {
    	hitpoint.set(pinf.pt);
    }

    if (hitnormal)
    {
    	Fvector verts[3];
        pinf.e_obj->GetFaceWorld(pinf.s_obj->_Transform(),pinf.e_mesh,pinf.inf.id,verts);
        hitnormal->mknormal(verts[0],verts[1],verts[2]);
    }
}


bool EditLibPickObjectGeometry(  Fvector& hitpoint,  const Fvector& start, const Fvector& direction, int bSnap, Fvector* hitnormal )
{
	SRayPickInfo pinf;
	/*if( TfrmEditLibrary::RayPick( start, direction, &pinf ) )
    {
    	RetrieveSceneObjPointAndNormal( hitpoint,  hitnormal, pinf, bSnap );
        return true;
    }*/
    return false;
}

bool ScenePickObjectGeometry( Fvector& hitpoint,  const Fvector& start, const Fvector& direction, int bSnap, Fvector* hitnormal )
{

	SRayPickInfo pinf;

   
    SRayPickInfo l_pinf;
    bool bResult = false;

    {
      SRayPickInfo l_pinf;
      bool l_bres = Scene->RayPickObject( l_pinf.inf.range, start,direction, OBJCLASS_SPAWNPOINT , &l_pinf, Scene->GetSnapList(false) );
      
      if( l_bres )
      {
          pinf = l_pinf;
          bResult = true;
      }
      
    }
    {
    
     SRayPickInfo l_pinf;
     bool l_bres = Scene->RayPickObject( l_pinf.inf.range, start, direction, OBJCLASS_SCENEOBJECT , &l_pinf, Scene->GetSnapList(false) );

     if( !bResult||(l_bres && l_pinf.inf.range < pinf.inf.range) )
          pinf = l_pinf;
     if( l_bres )
           bResult = true;
           
    }


     if( bResult )
     	    RetrieveSceneObjPointAndNormal( hitpoint,  hitnormal, pinf, bSnap );
            
     return  bResult;

}


bool PickObjectGeometry( EEditorState est, Fvector& hitpoint,  const Fvector& start, const Fvector& direction, int bSnap, Fvector* hitnormal )
{

	switch(est)
    {
       case esEditLibrary:
         	return EditLibPickObjectGeometry( hitpoint, start, direction, bSnap, hitnormal );
       case esEditScene:
         	return ScenePickObjectGeometry( hitpoint, start, direction, bSnap, hitnormal );
        default:
         	NODEFAULT;
    }
	return false;
}

bool PickGrid(  Fvector& hitpoint,  const Fvector& start, const Fvector& direction, int bSnap, Fvector* hitnormal )
{
     
    // pick grid
	Fvector normal;
	normal.set( 0, 1, 0 );
	float clcheck = direction.dotproduct( normal );

	if( fis_zero( clcheck ) )
    	 return false;

	float alpha = - start.dotproduct(normal) / clcheck;
    
	if( alpha <= 0 )
    	return false;

	hitpoint.x = start.x + direction.x * alpha;
	hitpoint.y = start.y + direction.y * alpha;
	hitpoint.z = start.z + direction.z * alpha;

    if (LTools->GetGimzo()->IsStepEnable(Gizmo::EType::Move) && bSnap)
    {
        hitpoint.x = snapto( hitpoint.x, LTools->GetGimzo()->GetStep(Gizmo::EType::Move));
        hitpoint.z = snapto( hitpoint.z, LTools->GetGimzo()->GetStep(Gizmo::EType::Move));
        hitpoint.y = 0.f;
    }
    
	if (hitnormal)
    	hitnormal->set(0,1,0);
	return true;
}

bool CLevelMain::PickGround(Fvector& hitpoint, const Fvector& start, const Fvector& direction, int bSnap, Fvector* hitnormal){
	VERIFY(m_bReady);
    
    EEditorState est = GetEState();
    if( est!= esEditLibrary && est != esEditScene )
    	return false;
   
    // pick object geometry
    if( (bSnap==-1) || ( Tools->GetSettings(etfOSnap) && (bSnap==1) ) )
    {
      bool b =  PickObjectGeometry( est, hitpoint, start, direction, bSnap,  hitnormal );
      if(b)
      return true;
    }

    return   PickGrid( hitpoint, start, direction, bSnap,  hitnormal );

}


bool CLevelMain::SelectionFrustum(CFrustum& frustum)
{
	VERIFY(m_bReady);
    Fvector st,d,p[4];
    Ivector2 pt[4];

    float depth = 0;

    float x1=m_StartCp.x, x2=m_CurrentCp.x;
    float y1=m_StartCp.y, y2=m_CurrentCp.y;

	if(!(x1!=x2&&y1!=y2)) return false;

	pt[0].set(_min(x1,x2),_min(y1,y2));
	pt[1].set(_max(x1,x2),_min(y1,y2));
	pt[2].set(_max(x1,x2),_max(y1,y2));
	pt[3].set(_min(x1,x2),_max(y1,y2));

    SRayPickInfo pinf;
    for (int i=0; i<4; i++){
	    EDevice->m_Camera.MouseRayFromPoint(st, d, pt[i]);
        if (EPrefs->bp_lim_depth){
			pinf.inf.range = EDevice->m_Camera._Zfar(); // max pick range
            if (Scene->RayPickObject(pinf.inf.range, st, d, OBJCLASS_SCENEOBJECT, &pinf, 0))
	            if (pinf.inf.range > depth) depth = pinf.inf.range;
        }
    }
    if (depth<EDevice->m_Camera._Znear()) depth = EDevice->m_Camera._Zfar();
    else depth += EPrefs->bp_depth_tolerance;

    for (int i=0; i<4; i++){
	    EDevice->m_Camera.MouseRayFromPoint(st, d, pt[i]);
        p[i].mad(st,d,depth);
    }

    Fvector pos = EDevice->m_Camera.GetPosition();
    frustum.CreateFromPoints(p,4,pos);

    Fplane P; P.build(p[0],p[1],p[2]);
    if (P.classify(st)>0) P.build(p[2],p[1],p[0]);
	frustum._add(P);

	return true;
}

void CLevelMain::RealUpdateScene()
{
	inherited::RealUpdateScene	();
	if (GetEState()==esEditScene)
    {
	    Scene->OnObjectsUpdate	();
    	LTools->OnObjectsUpdate	(); // �������� ��� ��� ���-�� ������� � ���������
	    RedrawScene				();
    }
}



void CLevelMain::ShowContextMenu(int cls)
{
	VERIFY(m_bReady);
   /* POINT pt;
    GetCursorPos(&pt);
    fraLeftBar->miProperties->Enabled = false;
    if (Scene->SelectionCount( true, cls )) fraLeftBar->miProperties->Enabled = true;
    RedrawScene(true);
    fraLeftBar->pmObjectContext->TrackButton = tbRightButton;
    fraLeftBar->pmObjectContext->Popup(pt.x,pt.y);*/
}






// Common

void CLevelMain::ResetStatus()
{
	VERIFY(m_bReady);
  /*  if (fraBottomBar->paStatus->Caption!=""){
	    fraBottomBar->paStatus->Caption=""; fraBottomBar->paStatus->Repaint();
    }*/
}
void CLevelMain::SetStatus(LPCSTR s, bool bOutLog)
{
	VERIFY(m_bReady);
  /*  if (fraBottomBar->paStatus->Caption!=s){
	    fraBottomBar->paStatus->Caption=s; fraBottomBar->paStatus->Repaint();
    	if (bOutLog&&s&&s[0]) ELog.Msg(mtInformation,s);
    }*/
}
void CLevelMain::ProgressDraw()
{
    inherited::ProgressDraw();
	//fraBottomBar->RedrawBar();
}

void CLevelMain::OutCameraPos()
{
	if (m_bReady){
        xr_string s;
        const Fvector& c 	= EDevice->m_Camera.GetPosition();
        s.sprintf("C: %3.1f, %3.1f, %3.1f",c.x,c.y,c.z);
    //	const Fvector& hpb 	= EDevice->m_Camera.GetHPB();
    //	s.sprintf(" Cam: %3.1f�, %3.1f�, %3.1f�",rad2deg(hpb.y),rad2deg(hpb.x),rad2deg(hpb.z));
       // fraBottomBar->paCamera->Caption=s; fraBottomBar->paCamera->Repaint();

    }
}

void CLevelMain::OutUICursorPos()
{
	/*VERIFY(fraBottomBar);
    xr_string s; POINT pt;
    GetCursorPos(&pt);
    s.sprintf("Cur: %d, %d",pt.x,pt.y);
    fraBottomBar->paUICursor->Caption=s; fraBottomBar->paUICursor->Repaint();*/
}

void CLevelMain::OutGridSize()
{
	/*VERIFY(fraBottomBar);
    xr_string s;
    s.sprintf("Grid: %1.1f",EPrefs->grid_cell_size);
    fraBottomBar->paGridSquareSize->Caption=s; fraBottomBar->paGridSquareSize->Repaint();*/
}

void CLevelMain::OutInfo()
{
	//fraBottomBar->paSel->Caption = Tools->GetInfo();
}

void CLevelMain::RealQuit()
{
	//frmMain->Close();
}


void CLevelMain::SaveSettings(CInifile* I)
{
	inherited::SaveSettings(I);
    SSceneSummary::Save(I);
}
void CLevelMain::LoadSettings(CInifile* I)
{
	inherited::LoadSettings(I);
    SSceneSummary::Load(I);
}

Ivector2 CLevelMain::GetRenderMousePosition() const
{
    return MainForm->GetRenderForm()->GetMousePos();
}

void CLevelMain::OnDrawUI()
{
    inherited::OnDrawUI();
    // the panels and the documents are drawn by the frame (UIMainForm, XUI::Shell); what is left here are the
    // modals of the mod windows, and the ticks that run whichever tab is up
    // quest tabs need a project, not a linked game: their documents can be opened
    // over MCP before the link exists
    if (EditorProject::Active())
        UIQuestGraph::Update();
    UIImagePreview::Update();
    if (EditorProject::Active() && EditorProject::GameLinked())
    {
        UIContentBrowser::Tick();
        UIVisualPreview::Update();
        EditorProject::DrawUI();
        EditorMod::DrawUI();
        EditorPublish::DrawUI();
        EditorCatalogSubmit::DrawUI();
        EditorCharacters::DrawUI();
        EditorWorld::DrawUI();
        EditorScreens::DrawUI();
        EditorPda::DrawUI();
    }
    // a tool's own windows belong to the Level tab
    if (LTools->GetToolForm() && XUI::Shell::IsActive("level"))
    {
        LTools->GetToolForm()->OnDrawUI();
    }
}

bool CLevelMain::KeyDown(WORD Key, TShiftState Shift)
{
    return TUI::KeyDown(Key, Shift);
}

// What the scene still needs built is the status bar's Rebuild chip (section 5, #30), not red lines in the render target.
void CLevelMain::OnStats(CGameFont* font)
{
}




bool CLevelMain::IsPlayInEditor()
{
	return Scene->IsPlayInEditor();
}
