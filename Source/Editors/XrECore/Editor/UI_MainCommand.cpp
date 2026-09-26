//---------------------------------------------------------------------------

#include "stdafx.h"
#pragma hdrstop

#include "ui_main.h"
#include "UI_ToolsCustom.h"

#include "UIEditLightAnim.h"
#include "UIImageEditorForm.h"
#include "UISoundEditorForm.h"
#include "d3dutils.h"

#include "PSLibrary.h"
#include "Library.h"
#include "LightAnimLibrary.h"

#include "ImageManager.h"
#include "SoundManager.h"
#include "ResourceManager.h"
#include "engine\XrGamePersistentEditors.h"
#include "Nq\NqDoc.h"
#include "UI_CommandPalette.h"
#include "EditorStartup.h"


ECommandVec 		ECommands;
BOOL 				bAllowReceiveCommand	= FALSE;
bool 				bAllowLogCommands		= false;
static int			s_CommandExecutionDepth = 0;

struct SCommandExecutionScope
{
	int& depth;

	explicit SCommandExecutionScope(int& value) : depth(value) { ++depth; }
	~SCommandExecutionScope()
	{
		VERIFY(depth > 0);
		--depth;
	}
};
//TfrmText*			frmEditCommandList		= 0;
xr_string			sCommandListText;

BOOL AllowLogCommands()
{
	return bAllowLogCommands;
}

ECommandVec&  GetEditorCommands()
{
	return 	ECommands;
}

bool CommandConfirmed(CCommandVar p2)
{
	return p2.IsInteger() && u32(p2) == kCommandConfirmed;
}

bool ConfirmCommand(u32 cmd, CCommandVar p1, CCommandVar p2, const XUI::ConfirmSpec& spec)
{
	if (CommandConfirmed(p2))
		return true;
	// Confirm answers at once over MCP and under -nodlg, on a later frame when it asks: the first is this call's answer,
	// the second runs the command again - after the frame, like any command a click defers
	struct SAsk { bool returned = false; bool yes = false; };
	auto ask = std::make_shared<SAsk>();
	XUI::Confirm(spec, [cmd, p1, ask](XUI::Answer answer)
	{
		if (!ask->returned)
			ask->yes = answer == XUI::Answer::Yes;
		else if (answer == XUI::Answer::Yes && UI)
			UI->DeferCommand(cmd, p1, CCommandVar(kCommandConfirmed));
	});
	ask->returned = true;
	return ask->yes;
}
void 	EnableReceiveCommands()
{
	bAllowReceiveCommand = TRUE;
}
SESubCommand* FindCommandByShortcut(const xr_shortcut& val)
{
    ECommandVec& cmds		= GetEditorCommands();
    for (u32 cmd_idx=0; cmd_idx<cmds.size(); cmd_idx++){
    	SECommand*& CMD		= cmds[cmd_idx];
        if (CMD&&CMD->editable){
        	VERIFY(!CMD->sub_commands.empty());
		    for (u32 sub_cmd_idx=0; sub_cmd_idx<CMD->sub_commands.size(); sub_cmd_idx++){
            	SESubCommand*& SUB_CMD = CMD->sub_commands[sub_cmd_idx];
                if (SUB_CMD->shortcut.similar(val)) return SUB_CMD;
            }
        }
    }
    return 0;
}
SECommand* FindCommandByName(LPCSTR nm)
{
    ECommandVec& cmds		= GetEditorCommands();
    for (u32 cmd_idx=0; cmd_idx<cmds.size(); cmd_idx++){
    	SECommand*& CMD		= cmds[cmd_idx];
        if (CMD&&(0==stricmp(CMD->name,nm))) return CMD;
    }
    return 0;
}
SESubCommand* FindSubCommandByName(SECommand* CMD, LPCSTR nm)
{
    VERIFY(CMD && !CMD->sub_commands.empty());
    for (u32 sub_cmd_idx=0; sub_cmd_idx<CMD->sub_commands.size(); sub_cmd_idx++){
        SESubCommand* SUB_CMD = CMD->sub_commands[sub_cmd_idx];
        if (0==stricmp(SUB_CMD->desc.c_str(),nm)) return SUB_CMD;
    }
    return 0;
}
void ParseParam(xr_string sp, CCommandVar& res)
{
    if (!sp.empty()){
    	u32 rs=0,ip=0;
        if (0==strstr(sp.c_str(),"\""))
        	rs			= sscanf(sp.c_str(),"%d",&ip); 
        if (1!=rs){
            _GetItem(sp.c_str(),1,sp,'\"');
            if (!sp.empty()) res = sp;
        }else		res = ip;
    }
}
CCommandVar		ExecCommand	(const xr_shortcut& val)
{
	SESubCommand* SUB 	= FindCommandByShortcut(val);
    CCommandVar res		= CCommandVar(u32(0));
    if (SUB)
    	res				= ExecCommand(SUB->parent->idx,SUB->p0,SUB->p1);
    return res;
}
CCommandVar 	ExecCommand	(u32 cmd, CCommandVar p1, CCommandVar p2)
{
	if (!bAllowReceiveCommand)	return 0;

	VERIFY				(cmd<ECommands.size());
	if (UI && UI->InUIPass() && !s_CommandExecutionDepth && UI->ShouldDeferCommand(cmd))
		return UI->DeferCommand(cmd, p1, p2) ? u32(1) : u32(0);
    CCommandVar	res;
	SECommand*	CMD 	= ECommands[cmd];
    VERIFY				(CMD&&!CMD->command.empty());
    if (bAllowLogCommands){
		string128 level;strcpy(level,s_CommandExecutionDepth==0?"":";");
        for(int k=0; k<s_CommandExecutionDepth; ++k) strcat(level,".");
        xr_string sp1	= p1.IsString()?xr_string(p1):xr_string("");
        xr_string sp2	= p2.IsString()?xr_string(p2):xr_string("");
        if (p1.IsString()) sp1 = ((sp1.find("\n")==sp1.npos)&&(sp1.find("\r")==sp1.npos))?sp1:xr_string("..."); 
        if (p2.IsString()) sp2 = ((sp2.find("\n")==sp2.npos)&&(sp2.find("\r")==sp2.npos))?sp2:xr_string("...");
    /*    if (p1.IsString() && p2.IsString()) 		Msg("%s%s (\"%s\",\"%s\")", level, CMD->Name(), sp1.c_str(), sp2.c_str());
        else if (p1.IsInteger()&&p2.IsInteger())Msg("%s%s (%d,%d)",			level,CMD->Name(),u32(p1),u32(p2));
        else if (p1.IsInteger()&&p2.IsString()) Msg("%s%s (%d,\"%s\")",		level,CMD->Name(),u32(p1),sp2.c_str());
        else if (p1.IsString()&&p2.IsInteger()) Msg("%s%s (\"%s\",%d)",		level,CMD->Name(),sp1.c_str(),u32(p2));*/
    }
	SCommandExecutionScope execution_scope(s_CommandExecutionDepth);
    res 	 			= CMD->command(p1,p2);
    return res;
}
void	RegisterCommand (u32 cmd, SECommand* cmd_impl)
{
	if (cmd>=ECommands.size()) 
    	ECommands.resize(cmd+1,0);
	SECommand*&	CMD = ECommands[cmd];
    if (CMD){
    	Msg			("RegisterCommand: command '%s' overridden by command '%s'.",*CMD->desc,*cmd_impl->desc);
    	xr_delete	(CMD);
    }
    CMD	   			= cmd_impl;
}
void	RegisterSubCommand(SECommand* cmd_impl, LPCSTR desc, CCommandVar p0, CCommandVar p1)
{
    VERIFY		(cmd_impl);
    cmd_impl->AppendSubCommand(desc,p0,p1);
}
BOOL	LoadShortcuts(CInifile* ini)
{
    for (u32 cmd_idx=0; cmd_idx<ECommands.size(); cmd_idx++){
    	SECommand*& CMD		= ECommands[cmd_idx];
        if (CMD&&CMD->editable){
		    for (u32 sub_cmd_idx=0; sub_cmd_idx<CMD->sub_commands.size(); sub_cmd_idx++){
            	SESubCommand*& SUB	 	= CMD->sub_commands[sub_cmd_idx];
                string256 nm,tmp; 	
                if (SUB->desc.size())	sprintf(nm,"%s.\"%s\"",CMD->Name(),SUB->desc.c_str());
                else   					sprintf(nm,"%s",CMD->Name());
                if (ini->line_exist("shortcuts",nm)){ 
                	LPCSTR val			= ini->r_string("shortcuts",nm);
                    int res 			= sscanf(val,"%d,%s",&SUB->shortcut.hotkey,tmp);
                    if (2==res){
                    	xr_string 		sp;
                    	_GetItem		(tmp,0,sp);
                        ParseParam		(sp,SUB->p0);
                    	_GetItem		(tmp,1,sp);
                        ParseParam		(sp,SUB->p1);
                    }
                }
            }
        }
    }
	return TRUE;
}
BOOL	SaveShortcuts(CInifile* ini)
{
    for (u32 cmd_idx=0; cmd_idx<ECommands.size(); cmd_idx++){
    	SECommand*& CMD		= ECommands[cmd_idx];
        if (CMD&&CMD->editable){
		    for (u32 sub_cmd_idx=0; sub_cmd_idx<CMD->sub_commands.size(); sub_cmd_idx++){
            	SESubCommand*& SUB = CMD->sub_commands[sub_cmd_idx];
                string256 nm,tmp; 	
                if (SUB->desc.size())	sprintf(nm,"%s.\"%s\"",CMD->Name(),SUB->desc.c_str());
                else   					sprintf(nm,"%s",CMD->Name());
                if (SUB->p0.IsString()&&SUB->p1.IsString()) 		sprintf(tmp,"%d, \"%s\",\"%s\"",SUB->shortcut.hotkey,xr_string(SUB->p0).c_str(),xr_string(SUB->p1).c_str());
                else if (SUB->p0.IsInteger()&&SUB->p1.IsInteger())	sprintf(tmp,"%d, %d,%d",		SUB->shortcut.hotkey,u32(SUB->p0),u32(SUB->p1));
                else if (SUB->p0.IsInteger()&&SUB->p1.IsString()) 	sprintf(tmp,"%d, %d,\"%s\"",	SUB->shortcut.hotkey,u32(SUB->p0),xr_string(SUB->p1).c_str());
                else if (SUB->p0.IsString()&&SUB->p1.IsInteger()) 	sprintf(tmp,"%d, \"%s\",%d",	SUB->shortcut.hotkey,xr_string(SUB->p0).c_str(),u32(SUB->p1));
                ini->w_string	("shortcuts",nm,tmp);
            }
        }
    }
	return TRUE;
}
void	ClearCommands()
{
	for (ECommandVecIt it=ECommands.begin(); it!=ECommands.end(); it++)
    	xr_delete	(*it);
	ECommands.clear	();
}

void	TUI::ClearCommands ()
{
	::ClearCommands	();
}

//------------------------------------------------------------------------------
// UI Commands
//------------------------------------------------------------------------------
CCommandVar	TUI::CommandRenderFocus(CCommandVar p1, CCommandVar p2)
{
    SetFocus(EDevice->m_hWnd);
    return 1;
}
CCommandVar	TUI::CommandBreakLastOperation(CCommandVar p1, CCommandVar p2)
{
	// No question: a blocking job holds the frame, which draws no dialog until the job ends - and a cancelled job
	// stops at its next checkpoint, the same as the Cancel of its task card, which does not ask either
	if (!ProgressOperationActive())
	{
		ELog.Msg(mtInformation, "No job is running: nothing to cancel.");
		return 0;
	}
	if (!ProgressCancelable())
	{
		ELog.Msg(mtInformation, "The running job cannot be cancelled safely.");
		return 0;
	}
	if (!NeedBreak())
	{
		ELog.Msg(mtInformation, "The job is already stopping.");
		return 0;
	}
	ELog.Msg(mtInformation, "Cancellation requested: the job stops at its next checkpoint.");
	return 1;
}
CCommandVar 	TUI::CommandRenderResize(CCommandVar p1, CCommandVar p2)
{
  /*  if (psDeviceFlags.is(rsDrawSafeRect)){
        int w=m_D3DPanel->Width,h=m_D3DPanel->Height,w_2=w/2,h_2=h/2;
        Irect rect;
        if ((0.75f*float(w))>float(h)) 	rect.set(w_2-1.33f*float(h_2),0,1.33f*h,h);
        else                   			rect.set(0,h_2-0.75f*float(w_2),w,0.75f*w);
        m_D3DWindow->Left  	= rect.x1;
        m_D3DWindow->Top  	= rect.y1;
        m_D3DWindow->Width 	= rect.x2;
        m_D3DWindow->Height	= rect.y2;
    }else{
        m_D3DWindow->Left  	= 0;
        m_D3DWindow->Top  	= 0;
        m_D3DWindow->Width 	= m_D3DPanel->Width;
        m_D3DWindow->Height	= m_D3DPanel->Height;
    }
    UI->RedrawScene		();*/
    return 1;
}

//------------------------------------------------------------------------------
// Common Commands
//------------------------------------------------------------------------------
CCommandVar CommandInitialize(CCommandVar p1, CCommandVar p2)
{

    EDevice = xr_new< CEditorRenderDevice>();
    Device = EDevice;
	CCommandVar res		= TRUE;
    EditorStartup::Stage("Reading configuration");
    {
        string_path              fn;
        strconcat(sizeof(fn), fn, UI->EditorName(), ".log");
        FS.update_path(fn, _local_root_, fn);
        string_path 			si_name;
        FS.update_path(si_name, "$game_config$", "system.ltx");
        pSettings = xr_new<CInifile>(si_name, TRUE);// FALSE,TRUE,TRUE);
        string_path					fname;
        FS.update_path(fname, "$game_config$", "game.ltx");
        pGameIni = xr_new<CInifile>(fname, TRUE);
        CHECK_OR_EXIT(0 != pGameIni->section_count(), make_string("Cannot find file %s.\nReinstalling application may fix this problem.", fname));
    }
    // make interface
    //----------------
    if(EPrefs)EPrefs->OnCreate		();
    EditorStartup::Stage("Starting the render device");
    if (UI->OnCreate(/*(TD3DWindow*)(u32)p1,(TPanel*)(u32)p2)*/))
    {
        // the three faces of the UI are read once, and timed as a stage of their own
        EditorStartup::Stage("Loading fonts");
        XUI::LoadFonts();
        EditorStartup::Stage("Loading the editor library");
        ExecCommand		(COMMAND_CREATE_SOUND_LIB);	R_ASSERT(SndLib);
        SndLib->OnCreate();
        LALib.OnCreate	();
        Lib.OnCreate	();
        BOOL bWeather = psDeviceFlags.is(rsEnvironment);
        psDeviceFlags.set(rsEnvironment, FALSE);
        g_pGamePersistent= xr_new<XrGamePersistentEditors>();
        if (Tools)
        {
            if (Tools->OnCreate())
            {
                if(EPrefs)
                    EPrefs->Load();
                EDevice->seqAppStart.Process(rp_AppStart);
                ExecCommand(COMMAND_RESTORE_UI_BAR);
                ExecCommand(COMMAND_REFRESH_UI_BAR);
                ExecCommand(COMMAND_CLEAR);
                ExecCommand(COMMAND_RENDER_FOCUS);
                ExecCommand(COMMAND_CHANGE_ACTION, etaSelect);
                ExecCommand(COMMAND_RENDER_RESIZE);
                /*
                            if(bWeather && EPrefs->sWeather.size() )
                            {
                                psDeviceFlags.set(rsEnvironment, TRUE);
                                g_pGamePersistent->Environment().SetWeather(EPrefs->sWeather, true);

                            }
                */
            }
            else {
                res = FALSE;
            }
        }
    }else{
        res 			= FALSE;
    }
    return res;
}             
CCommandVar 	CommandDestroy(CCommandVar p1, CCommandVar p2)
{
    ExecCommand			(COMMAND_SAVE_UI_BAR);
    EPrefs->OnDestroy	();
    ExecCommand			(COMMAND_CLEAR);
    EDevice->seqAppEnd.Process(rp_AppEnd);
    xr_delete			(g_pGamePersistent);
    LALib.OnDestroy		();
    Tools->OnDestroy	();
    SndLib->OnDestroy	();
    xr_delete			(SndLib);
    DU_impl.DestroyObjects();
    Lib.OnDestroy		();
    UI->OnDestroy		();
    {
        xr_delete(pSettings);
    }
    {
        xr_delete(pGameIni);
    }
    ELog.Close();
    for (auto& item : ECommands)
    {
        xr_delete(item);
    }
    ECommands.clear();
    xr_delete(EDevice);
    Device = nullptr;
    return				TRUE;
}             
CCommandVar 	CommandQuit(CCommandVar p1, CCommandVar p2)
{
    // The IsModified() gate here was inverted logic: File\Quit did nothing at all
    // on a saved scene. Quit unconditionally - the save prompt belongs to
    // COMMAND_EXIT, which callers run before this one when they want it.
    // Quest documents have no such prompt (a modal would stall MCP and -nodlg),
    // so their unsaved edits are written out instead of dying with the process.
    NqDocs::SaveDirty	();
    UI->Quit			();
    return				TRUE;
}
CCommandVar 	CommandEditorPrefs(CCommandVar p1, CCommandVar p2)
{
    EPrefs->Edit		();
    return				TRUE;
}             
CCommandVar 	CommandChangeAction(CCommandVar p1, CCommandVar p2)
{
     Tools->SetAction	(ETAction(u32(p1)));
    return				TRUE;
}       

CCommandVar 	CommandSimulate(CCommandVar p1, CCommandVar p2)
{

	Tools->Simulate();

    
    return				TRUE;
}

CCommandVar 	CommandUseSimulatePositions(CCommandVar p1, CCommandVar p2)
{

   Tools->UseSimulatePositions();

    return				TRUE;
}



CCommandVar 	CommandSetSettings(CCommandVar p1, CCommandVar p2)
{
	Tools->SetSettings(p1,p2);
    return				TRUE;
}             
CCommandVar 	CommandSoundEditor(CCommandVar p1, CCommandVar p2)
{
    UISoundEditorForm::Show();
  //  TfrmSoundLib::EditLib(xr_string("Sound Editor"));
    return				TRUE;
}
// What a sound sync takes away (CSoundManager::CleanupSounds): the game sounds and the sound descriptions whose .wav
// source is gone. They are not deleted but renamed to .~ogg / .~thm, which the question says.
void OrphanSounds(XUI::Strings& items, u32& total)
{
	FS_FileSet base, thumbs, game;
	FS.file_list(base, _sounds_, FS_ListFiles | FS_ClampExt, "*.wav");
	FS.file_list(thumbs, _sounds_, FS_ListFiles | FS_ClampExt, "*.thm");
	FS.file_list(game, _game_sounds_, FS_ListFiles | FS_ClampExt, "*.ogg");
	total = 0;
	for (FS_FileSet* set : { &game, &thumbs })
		for (const FS_File& file : *set)
		{
			xr_string name = EFS.ChangeFileExt(file.name, "");
			xr_strlwr(name);
			if (base.find(name) != base.end())
				continue;
			if (items.size() < 10)
				items.push_back(name + (set == &game ? ".ogg" : ".thm"));
			++total;
		}
}

CCommandVar 	CommandSyncSounds(CCommandVar p1, CCommandVar p2)
{
	XUI::ConfirmSpec spec;
	spec.id = "sync_sounds";
	spec.title = "Synchronize Sounds";
	spec.action = "Synchronize";
	if (!CommandConfirmed(p2))
		OrphanSounds(spec.items, spec.total);
	spec.headline = spec.total
		? "Synchronize the sound library? These sounds have no .wav source any more:"
		: "Synchronize the sound library with the .wav sources?";
	spec.consequence = spec.total
		? "They are renamed to .~ogg and .~thm; the sounds whose source changed are converted again."
		: "The sounds whose source changed are converted again and their game files replaced.";
	if (!ConfirmCommand(COMMAND_SYNC_SOUNDS, p1, p2, spec))
		return FALSE;
	SProgressOperation progress(*UI, true);
	SndLib->RefreshSounds(true);
	return !UI->NeedAbort();
}
CCommandVar 	CommandImageEditor(CCommandVar p1, CCommandVar p2)
{
    UIImageEditorForm::Show(false);
    return				TRUE;
}
CCommandVar 	CommandLightAnimEditor(CCommandVar p1, CCommandVar p2)
{
    UIEditLightAnim::Show();
    return				TRUE;
}

CCommandVar 	CommandCheckTextures(CCommandVar p1, CCommandVar p2)
{
    UIImageEditorForm::ImportTextures();
    return				TRUE;
}
CCommandVar 	CommandRefreshTextures(CCommandVar p1, CCommandVar p2)
{
	XUI::ConfirmSpec spec;
	spec.id = "sync_textures";
	spec.title = "Synchronize Textures";
	spec.headline = "Synchronize the texture library with the texture sources?";
	spec.consequence = "The textures whose source changed are converted again: their .dds and .thm files are replaced.";
	spec.action = "Synchronize";
	if (!ConfirmCommand(COMMAND_REFRESH_TEXTURES, p1, p2, spec))
		return FALSE;
	SProgressOperation progress(*UI, true);
	ImageLib.RefreshTextures(0);
	return !UI->NeedAbort();
}
CCommandVar 	CommandReloadTextures(CCommandVar p1, CCommandVar p2)
{
    EDevice->ReloadTextures();
    UI->RedrawScene		();
    return				TRUE;
}
CCommandVar 	CommandChangeSnap(CCommandVar p1, CCommandVar p2)
{
    not_implemented();
  //  ((TExtBtn*)(u32)p1)->Down = !((TExtBtn*)(u32)p1)->Down;
    return				TRUE;
}
CCommandVar 	CommandUnloadTextures(CCommandVar p1, CCommandVar p2)
{
    EDevice->UnloadTextures();
    return				TRUE;
}
CCommandVar 	CommandEvictObjects(CCommandVar p1, CCommandVar p2)
{
    Lib.EvictObjects	();
    return				TRUE;
}
CCommandVar 	CommandEvictTextures(CCommandVar p1, CCommandVar p2)
{
    EDevice->Resources->Evict();
    return				TRUE;
}
CCommandVar 	CommandCheckModified(CCommandVar p1, CCommandVar p2)
{
    return 		Tools->IsModified();
}
CCommandVar 	CommandExit(CCommandVar p1, CCommandVar p2)
{
    return 		Tools->IfModified();
}
CCommandVar 	CommandShowProperties(CCommandVar p1, CCommandVar p2)
{
	if(p1.IsString())
    {
    	xr_string SSS = p1;
    	Tools->ShowProperties(SSS.c_str());
	}else
    	Tools->ShowProperties(NULL);
    return				TRUE;
}
CCommandVar 	CommandUpdateProperties(CCommandVar p1, CCommandVar p2)
{
    Tools->UpdateProperties(p1);
    return				TRUE;
}
CCommandVar 	CommandRefreshProperties(CCommandVar p1, CCommandVar p2)
{
    Tools->RefreshProperties();
    return				TRUE;
}
CCommandVar 	CommandZoomExtents(CCommandVar p1, CCommandVar p2)
{
    Tools->ZoomObject	(p1);
    UI->RedrawScene		();
    return				TRUE;
}
CCommandVar 	CommandToggleRenderWire(CCommandVar p1, CCommandVar p2)
{
    if (EDevice->dwFillMode!=D3DFILL_WIREFRAME)	EDevice->dwFillMode 	= D3DFILL_WIREFRAME;
    else 										EDevice->dwFillMode 	= D3DFILL_SOLID;
    UI->RedrawScene		();
    return				TRUE;
}
CCommandVar 	CommandToggleSafeRect(CCommandVar p1, CCommandVar p2)
{
    psDeviceFlags.set	(rsDrawSafeRect,!psDeviceFlags.is(rsDrawSafeRect));
    ExecCommand			(COMMAND_RENDER_RESIZE);
    UI->RedrawScene		();
    return				TRUE;
}
CCommandVar 	CommandToggleGrid(CCommandVar p1, CCommandVar p2)
{
    psDeviceFlags.set(rsDrawGrid,!psDeviceFlags.is(rsDrawGrid));
    UI->RedrawScene		();
    return				TRUE;
}
CCommandVar 	CommandUpdateGrid(CCommandVar p1, CCommandVar p2)
{
    DU_impl.UpdateGrid		(EPrefs->grid_cell_count,EPrefs->grid_cell_size);
    UI->OutGridSize		();
    UI->RedrawScene		();
    return				TRUE;
}
CCommandVar 	CommandGridNumberOfSlots(CCommandVar p1, CCommandVar p2)
{
    if (p1)				EPrefs->grid_cell_count += 2;
    else				EPrefs->grid_cell_count -= 2;
    ExecCommand			(COMMAND_UPDATE_GRID);
    UI->RedrawScene		();
    return				TRUE;
}
CCommandVar 	CommandGridSlotSize(CCommandVar p1, CCommandVar p2)
{
    float step = 1.f;
    float val = EPrefs->grid_cell_size;
    if (p1){
        if (val<1) step/=10.f;
        EPrefs->grid_cell_size += step;
    }else{
        if (fsimilar(val,1.f)||(val<1)) step/=10.f;
        EPrefs->grid_cell_size -= step;
    }
    ExecCommand			(COMMAND_UPDATE_GRID);
    UI->RedrawScene		();
    return				TRUE;
}
CCommandVar 	CommandCreateSoundLib(CCommandVar p1, CCommandVar p2)
{
    SndLib		= xr_new<CSoundManager>();
    return				TRUE;
}
CCommandVar 	CommandMuteSound(CCommandVar p1, CCommandVar p2)
{
    SndLib->MuteSounds	(p1);
    return				TRUE;
}
CCommandVar CommandMoveCameraTo(CCommandVar p1, CCommandVar p2)
{
    not_implemented();
    Fvector pos					= EDevice->m_Camera.GetPosition();
    /*if (NumericVectorRun		("Move to",&pos,3))
        EDevice->m_Camera.Set		(EDevice->m_Camera.GetHPB(),pos);*/
    return 						TRUE;
}

CCommandVar 	ExecuteCommandList(LPCSTR text)
{
	CCommandVar	res		= TRUE;
    IReader F			((void*)text,xr_strlen(text));
    while (!F.eof()){
        xr_string 			line, cmd, params, sp1, sp2;
        F.r_string			(line);
        line				= _Trim(line);
        if (!line.empty()){
            if (line[0]==';' || (line[0]=='/'&&line[1]=='/')) continue;
            _GetItem			(line.c_str(),0,cmd,'(');
            _GetItem			(line.c_str(),1,params,'(');
            _GetItem			(params.c_str(),0,params,')');
            _GetItem			(params.c_str(),0,sp1,',');
            _GetItem			(params.c_str(),1,sp2,',');
            // parse cmd
            xr_string 			cmd_name, sub_cmd_name;
            _GetItem			(cmd.c_str(),0,cmd_name,'.');
            _GetItem			(cmd.c_str(),1,sub_cmd_name,'.');
            
            SECommand* CMD 		= FindCommandByName(cmd_name.c_str()); 
            if (CMD){
                SESubCommand* SUB= FindSubCommandByName(CMD,sub_cmd_name.c_str());
                if (!sub_cmd_name.empty()&&!SUB){
                    ELog.DlgMsg	(mtError,"Can't find sub-command: '%s'",sub_cmd_name.c_str());
	                res			= FALSE;
                    break;
                }
                // parse params
                CCommandVar p1,p2;
                if (SUB){
                	p1			= SUB->p0;
                    p2 			= SUB->p1;
                }
                ParseParam		(sp1,p1);
                ParseParam		(sp2,p2);
                // execute command
                if (FALSE==ExecCommand(CMD->idx,p1,p2)){	
                    ELog.DlgMsg	(mtError,"Can't execute command: '%s'",cmd.c_str());
	                res			= FALSE;
                    break;
                }
            }else{
                ELog.DlgMsg		(mtError,"Can't find command: '%s'",cmd.c_str());
                res				= FALSE;
                break;
            }
        }
    }
    return				res;
}

bool 	OnRunExecuteListClick(LPCSTR txt)
{
	ExecuteCommandList		(txt);
    return true;
}

CCommandVar 	CommandExecuteCommandList(CCommandVar _p1, CCommandVar _p2)
{
    xr_string 	cmds_text			= _p1;
    return  	ExecuteCommandList	(cmds_text.c_str());
}

bool  OnCloseCommandListEditor()
{
    not_implemented();
	//frmEditCommandList	= 0;
    return 		true;
}

CCommandVar 	CommandEditCommandList(CCommandVar _p1, CCommandVar _p2)
{
    not_implemented();
  /*  if (NULL==frmEditCommandList){
    //	frmEditCommandList	= TfrmText::CreateForm(sCommandListText,"Execute command list",0,0,"Run",OnRunExecuteListClick,OnCloseCommandListEditor);
        return TRUE;
    }*/
    return FALSE;
}

CCommandVar 	CommandLogCommands(CCommandVar _p1, CCommandVar _p2)
{
    
	bAllowLogCommands	= !_p1;
    return 				TRUE;
}
CCommandVar 	CommandRunMacro(CCommandVar p1, CCommandVar p2)
{
	xr_string fn;
	if (p1.IsString()){
		fn 				= xr_string(p1); 
	    IReader* F 		= FS.r_open(fn.c_str());
        if (NULL==F) F	= FS.r_open(_import_,fn.c_str());
        if (F){
            ExecCommand	(COMMAND_EXECUTE_COMMAND_LIST,xr_string((LPCSTR)F->pointer()));
            FS.r_close 	(F);
            return 	   	TRUE;
        }
    }else{
        SECommand* CMD 	= GetEditorCommands()[COMMAND_RUN_MACRO]; VERIFY(CMD);
        u32 num 		= p1; VERIFY(num<CMD->sub_commands.size());
        SESubCommand* SUB=CMD->sub_commands[num];
        fn 				= xr_string(SUB->p0);
        return ExecCommand(COMMAND_RUN_MACRO,fn,p2);
    }
    return 				FALSE;
}
CCommandVar 	CommandAssignMacro(CCommandVar p1, CCommandVar p2)
{
    xr_string fn 		= p2.IsString()?xr_string(p2):xr_string(""); 
    if (p2.IsString()){
        if (0==fn.find(FS.get_path(_import_)->m_Path))
            fn 			= xr_string(fn.c_str()+xr_strlen(FS.get_path(_import_)->m_Path));
	    ECommands[COMMAND_RUN_MACRO]->sub_commands[p1]->p0 = fn;
	    return 			TRUE;
    }else{
    	if (EFS.GetOpenName(EDevice->m_hWnd, _import_,fn,false,NULL,2))
        	return 		ExecCommand	(COMMAND_ASSIGN_MACRO,p1,fn);
    }
    return FALSE;
}
CCommandVar CommandOpenCommandPalette(CCommandVar, CCommandVar)
{
	CommandPalette::Open();
	return TRUE;
}
void TUI::RegisterCommands()
{
	REGISTER_CMD_S		(COMMAND_INITIALIZE,			CommandInitialize);
	REGISTER_CMD_S		(COMMAND_DESTROY,        		CommandDestroy);
	REGISTER_CMD_SE		(COMMAND_EXIT,               	"Exit",					CommandExit,		true);
	REGISTER_CMD_S		(COMMAND_QUIT,           		CommandQuit);
	REGISTER_CMD_SE		(COMMAND_EDITOR_PREF,    		"Editor Preference",	CommandEditorPrefs, false);


    REGISTER_CMD_SE	(COMMAND_SIMULATE,  			"Simulate",      		CommandSimulate, true);
    REGISTER_CMD_SE	(COMMAND_USE_SIMULATE_POSITIONS,"Use Simulate Positions",CommandUseSimulatePositions, true);

	REGISTER_SUB_CMD_SE	(COMMAND_CHANGE_ACTION,  		"Change Action",      	CommandChangeAction,false);
	   	APPEND_SUB_CMD	("Select",						etaSelect,	0);
    	APPEND_SUB_CMD	("Add",							etaAdd,		0);
    REGISTER_SUB_CMD_END;
	REGISTER_CMD_S	    (COMMAND_SET_SETTINGS,			CommandSetSettings);
	REGISTER_CMD_S	    (COMMAND_SOUND_EDITOR,   		CommandSoundEditor);
	REGISTER_CMD_S	    (COMMAND_SYNC_SOUNDS,    		CommandSyncSounds);
    REGISTER_CMD_S	    (COMMAND_IMAGE_EDITOR,   		CommandImageEditor); 
    REGISTER_CMD_S      (COMMAND_LIGHTANIM_EDITOR,      CommandLightAnimEditor);
	REGISTER_CMD_S	    (COMMAND_CHECK_TEXTURES,     	CommandCheckTextures);
	REGISTER_CMD_S	    (COMMAND_REFRESH_TEXTURES,   	CommandRefreshTextures);	
	REGISTER_CMD_S	    (COMMAND_RELOAD_TEXTURES,    	CommandReloadTextures);
	REGISTER_CMD_S	    (COMMAND_CHANGE_SNAP,        	CommandChangeSnap);
    REGISTER_CMD_S	    (COMMAND_UNLOAD_TEXTURES,    	CommandUnloadTextures);
    REGISTER_CMD_S	    (COMMAND_EVICT_OBJECTS,      	CommandEvictObjects);
    REGISTER_CMD_S	    (COMMAND_EVICT_TEXTURES,     	CommandEvictTextures);
    REGISTER_CMD_S	    (COMMAND_CHECK_MODIFIED,     	CommandCheckModified);
	REGISTER_CMD_SE	    (COMMAND_SHOW_PROPERTIES,    	"Show Properties",		CommandShowProperties, false);
	REGISTER_CMD_S	    (COMMAND_UPDATE_PROPERTIES,  	CommandUpdateProperties);
	REGISTER_CMD_S	    (COMMAND_REFRESH_PROPERTIES, 	CommandRefreshProperties);
    REGISTER_SUB_CMD_SE (COMMAND_ZOOM_EXTENTS,     		"Zoom",					CommandZoomExtents,false);
    	APPEND_SUB_CMD	("Extent",						0,0);
    	APPEND_SUB_CMD	("Selected",					1,0);
    REGISTER_SUB_CMD_END;
	REGISTER_CMD_SE	    (COMMAND_MOVE_CAMERA_TO,        "Scene\\Move Camera To",CommandMoveCameraTo,false);
    REGISTER_CMD_SE	    (COMMAND_TOGGLE_RENDER_WIRE,	"Toggle Wireframe",		CommandToggleRenderWire,			false);
    REGISTER_CMD_C	    (COMMAND_RENDER_FOCUS,       	this,TUI::CommandRenderFocus);
	REGISTER_CMD_CE	    (COMMAND_BREAK_LAST_OPERATION,	"Break Last Operation",	this,TUI::CommandBreakLastOperation,false);
    REGISTER_CMD_SE	    (COMMAND_TOGGLE_SAFE_RECT,   	"Toggle Safe Rect",		CommandToggleSafeRect,false);
	REGISTER_CMD_C	    (COMMAND_RENDER_RESIZE,      	this,TUI::CommandRenderResize);
    REGISTER_CMD_SE	    (COMMAND_TOGGLE_GRID,        	"Toggle Grid",			CommandToggleGrid,false);
	REGISTER_CMD_S	    (COMMAND_UPDATE_GRID,        	CommandUpdateGrid);
    REGISTER_CMD_S	    (COMMAND_GRID_NUMBER_OF_SLOTS,	CommandGridNumberOfSlots);
    REGISTER_SUB_CMD_SE (COMMAND_GRID_SLOT_SIZE,     	"Change Grid Size",		CommandGridSlotSize,false);
    	APPEND_SUB_CMD	("Decrease",					0,0);
    	APPEND_SUB_CMD	("Increase",					1,0);
    REGISTER_SUB_CMD_END;
    REGISTER_CMD_S	    (COMMAND_CREATE_SOUND_LIB,   	CommandCreateSoundLib);
    REGISTER_CMD_S	    (COMMAND_MUTE_SOUND,         	CommandMuteSound);
    REGISTER_CMD_S	    (COMMAND_EDIT_COMMAND_LIST, 	CommandEditCommandList);
    REGISTER_CMD_S	    (COMMAND_EXECUTE_COMMAND_LIST, 	CommandExecuteCommandList);
    REGISTER_CMD_S	    (COMMAND_LOG_COMMANDS, 			CommandLogCommands);
    REGISTER_SUB_CMD_SE (COMMAND_RUN_MACRO,     		"Run Macro",			CommandRunMacro,false);
    	APPEND_SUB_CMD	("Slot #1",						xr_string(""),0);
    	APPEND_SUB_CMD	("Slot #2",						xr_string(""),0);
    	APPEND_SUB_CMD	("Slot #3",						xr_string(""),0);
    	APPEND_SUB_CMD	("Slot #4",						xr_string(""),0);
    	APPEND_SUB_CMD	("Slot #5",						xr_string(""),0);
    	APPEND_SUB_CMD	("Slot #6",						xr_string(""),0);
    	APPEND_SUB_CMD	("Slot #7",						xr_string(""),0);
    	APPEND_SUB_CMD	("Slot #8",						xr_string(""),0);
    REGISTER_SUB_CMD_END;
    REGISTER_CMD_S	    (COMMAND_ASSIGN_MACRO, 			CommandAssignMacro);
	REGISTER_CMD_SE		(COMMAND_COMMAND_PALETTE,		"Windows\\Command Palette",	CommandOpenCommandPalette, true);

	SECommand* palette_command = GetEditorCommands()[COMMAND_COMMAND_PALETTE];
	if (palette_command && !palette_command->sub_commands.empty())
		palette_command->sub_commands[0]->shortcut = xr_shortcut('P', FALSE, TRUE, TRUE);
}                                                                        

//---------------------------------------------------------------------------
bool TUI::ApplyShortCut(DWORD Key, TShiftState Shift)
{
	VERIFY(m_bReady);

    if (ApplyGlobalShortCut(Key,Shift))	return true;

    // Esc resets the tool to Select, then falls through so an Esc shortcut
    // from the table (e.g. Deselect All) can fire as well — UE-style cancel
    if (Key==VK_ESCAPE)			ExecCommand	(COMMAND_CHANGE_ACTION, etaSelect);

    xr_shortcut SC; 
    SC.key						= Key;
    SC.ext.assign				(u8((Shift&ssShift?xr_shortcut::flShift:0)|
    							 (Shift&ssCtrl ?xr_shortcut::flCtrl:0)|
                                 (Shift&ssAlt  ?xr_shortcut::flAlt:0)));
	SESubCommand* SUB 			= FindCommandByShortcut(SC);

    if (!SUB||SUB->parent->global_shortcut) 			return false;

    return 						ExecCommand(SC);
}
//---------------------------------------------------------------------------

bool TUI::ApplyGlobalShortCut(DWORD Key, TShiftState Shift)
{
	VERIFY(m_bReady);

    xr_shortcut SC;
    SC.key = Key;
    SC.ext.assign(u8((Shift & ssShift ? xr_shortcut::flShift : 0) |
        (Shift & ssCtrl ? xr_shortcut::flCtrl : 0) |
        (Shift & ssAlt ? xr_shortcut::flAlt : 0)));

    if (UIKeyPressForm::SetResult(SC))return true;
    if (Key == VK_OEM_3) { ExecCommand(COMMAND_RENDER_FOCUS); return true; }

  
	SESubCommand* SUB 			= FindCommandByShortcut(SC);

    if (!SUB||!SUB->parent->global_shortcut) 			return false;

    return						ExecCommand(SUB->parent->idx,SUB->p0,SUB->p1);
}
//---------------------------------------------------------------------------


