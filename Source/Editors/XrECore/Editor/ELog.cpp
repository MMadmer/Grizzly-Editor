//----------------------------------------------------
// file: NetDeviceELog.cpp
//----------------------------------------------------

#include "stdafx.h"
#pragma hdrstop

#include "ELog.h"
#include "UILogForm.h"
#include "ui_main.h"
void  ELogCallback(LPCSTR txt)
{
	if (0 == txt[0]) return;
	TMsgDlgType mt = TMsgDlgType::mtCustom;
	if (strncmp(txt, "! ", 2) == 0)
	{
		mt = mtError;
	}
	if (strncmp(txt, "~ ", 2) == 0)
	{
		mt = mtConfirmation;
	}
	if (strncmp(txt, "* ", 2) == 0)
	{
		mt = mtInformation;
	}

	UILogForm::AddMessage(txt);
	if(UI)
		UI->WriteConsole(mt, txt);

}

//----------------------------------------------------
CLog ELog;
//----------------------------------------------------
// -nodlg: an unattended editor (MCP-driven, smoke scripts) must never sit on a
// system-modal box - the main loop stops pumping and every remote command
// times out. The question still goes to the log, together with the answer
// given for the caller: No for confirmations (never save, never overwrite -
// the conservative choice; automation that wants the action asks for it
// explicitly), OK for plain error/info boxes.
static bool no_dialogs()
{
	return !!strstr(GetCommandLine(), "-nodlg");
}

// An error or a note is one of the UI's own notices once there is a UI to draw it - queued, it never stalls the
// frame. Before the first frame, and while a blocking job holds the frame, it stays the native box. Over MCP and
// under -nodlg nobody reads a box: the log has it.
static bool AsNotice(TMsgDlgType mt, const char* text)
{
	if (mtConfirmation == mt || no_dialogs() || XUI::FromMcp())
		return false;
	if (!UI || !UI->m_bReady || !ImGui::GetCurrentContext() || ImGui::GetFrameCount() < 2 || UI->ProgressOperationActive())
		return false;
	XUI::Notice(mtError == mt ? XUI::Tone::Error : XUI::Tone::Info, mtError == mt ? "Error" : "Information", text);
	return true;
}

inline TMsgDlgButtons MessageDlg(const char*text, TMsgDlgType mt, int btn)
{
	// the caller over MCP has decided: never a box that stalls the request, today's -nodlg answers instead
	if (no_dialogs() || XUI::FromMcp())
	{
		// the box's own level: an error counts as one, a question answered for the caller is a warning, a note is a note
		Msg("%s [dlg suppressed] %s%s", mtError == mt ? "!" : mtConfirmation == mt ? "~" : "*", text,
			(mtConfirmation == mt) ? "  -> auto-answer: No" : "");
		return (mtConfirmation == mt) ? mrNo : mrOK;
	}
	UINT Flags = 0;
	const char* Title = "";
	switch (mt)
	{
	case mtCustom:
		break;
	case mtError:
		Title = "Error";
		Flags = MB_ICONERROR;
		break;
	case mtInformation:
		Title = "Info";
		Flags = MB_ICONINFORMATION;
		break;
	case mtConfirmation:
		Title = "Warning";
		Flags = MB_ICONWARNING;
		break;
	default:
		R_ASSERT(0);
		break;
	}
	if (btn == mbYes)
	{
		Flags |=  MB_OK;
	}
	if (btn == mbOK)
	{
		Flags |= MB_OK;
	}
	else if(btn==(mbYes|mbNo))
	{
		Flags |= MB_YESNO;
	}
	else if (btn == (mbYes | mbNo|mbCancel))
	{
		Flags |= MB_YESNOCANCEL;
	}
	else
	{
		R_ASSERT(0);
	}
	int msgboxID = MessageBox(
		NULL,
		text,
		Title,
		Flags
	);
	switch (msgboxID)
	{
	case IDCANCEL:
		return mrCancel;	// TODO: add code
		break;
	case IDYES:
		return mrYes;
		break;
	case IDNO:
		return mrNo;
		break;
	case IDOK:
		return mrOK;
		break;
	}
	if (btn | mbCancel)return mrCancel;
	if (btn | mbNo)return mrNo;
	if (btn == mbOK)return mrOK;
	return mrYes;
}
int CLog::DlgMsg (TMsgDlgType mt, int btn, LPCSTR _Format, ...)
{
    in_use = true;
	char buf[4096];
	va_list l;
	va_start( l, _Format );
	vsprintf( buf, _Format, l );

	int res=0;
	if (AsNotice(mt, buf))
	{
		in_use = false;
		return mrOK;
	}
#if 1
    ExecCommand(COMMAND_RENDER_FOCUS);

    res=MessageDlg(buf, mt, btn);
    if (mtConfirmation==mt){
        switch (res){
        case mrYes: 	strcat(buf," - Yes."); 	break;
        case mrNo: 		strcat(buf," - No."); 	break;
        case mrCancel:  strcat(buf," - Cancel.");	break;
        default: strcat(buf," - Something.");
        }
    }
#endif
#ifdef _LW_EXPORT
	switch(mt){
	case mtError:		g_msg->error(buf,0);	break;
	case mtInformation: g_msg->info(buf,0);		break;
	default:			g_msg->info(buf,0);		break;
	}
#endif
#ifdef _MAX_PLUGIN
	switch(mt){
	case mtError:		MessageBox(0,buf,"Error",		MB_OK|MB_ICONERROR);		break;
	case mtInformation: MessageBox(0,buf,"Information",	MB_OK|MB_ICONINFORMATION);	break;
	default:			MessageBox(0,buf,"Information",	MB_OK|MB_ICONINFORMATION);	break;
	}
#endif

    Msg(mt, "%s", buf);

    in_use = false;

    return res;
}

void CLog::Close()
{
	SetLogCB(0);
	UILogForm::Destroy();
}


int CLog::DlgMsg (TMsgDlgType mt, LPCSTR _Format, ...)
{
    in_use = true;
	char buf[4096];
	va_list l;
	va_start( l, _Format );
	vsprintf( buf, _Format, l );

    int res=0;
	if (AsNotice(mt, buf))
	{
		in_use = false;
		return mrOK;
	}
#if 1
    ExecCommand(COMMAND_RENDER_FOCUS);

    if (mtConfirmation==mt)	res=MessageDlg(buf, mt,  mbYes | mbNo | mbCancel);
    else                   	res=MessageDlg(buf, mt,  mbOK);

    if (mtConfirmation==mt){
        switch (res){
        case mrYes: 	strcat(buf," - Yes."); 	break;
        case mrNo: 		strcat(buf," - No."); 	break;
        case mrCancel:  strcat(buf," - Cancel.");	break;
        default: strcat(buf," - Something.");
        }
    }
#endif
#ifdef _LW_EXPORT
	switch(mt){
	case mtError:		g_msg->error(buf,0);	break;
	case mtInformation: g_msg->info(buf,0);		break;
	default:			g_msg->info(buf,0);		break;
	}
#endif
#ifdef _MAX_PLUGIN
	switch(mt){
	case mtError:		MessageBox(0,buf,"Error",		MB_OK|MB_ICONERROR);		break;
	case mtInformation: MessageBox(0,buf,"Information",	MB_OK|MB_ICONINFORMATION);	break;
	default:			MessageBox(0,buf,"Information",	MB_OK|MB_ICONINFORMATION);	break;
	}
#endif

    Msg(mt, "%s", buf);

    in_use = false;
    
    return res;
}

void CLog::Msg(TMsgDlgType mt, LPCSTR _Format, ...)
{
	char buf[4096];
	va_list l;
	va_start( l, _Format );
	vsprintf( buf, _Format, l );

#if 1
	// the editor's own report: its type is its level (the text carries no "! " of its own)
	const UILogForm::Level level = mtError == mt ? UILogForm::Level::Error
		: mtInformation == mt || mtConfirmation == mt ? UILogForm::Level::Info : UILogForm::Level::ByPrefix;
	UILogForm::AddMessage(xr_string(buf), level, true);
#endif
#ifdef _MAX_EXPORT
	EConsole.print(mt,buf);
#endif
#ifdef _LW_EXPORT
	switch (mt){
	case mtError: g_msg->error(buf,0); break;
	}
#endif

	::LogExecCB = FALSE;
    ::Msg		("%s", buf);
	::LogExecCB	= TRUE;
}

void CLog::Progress(LPCSTR text)
{
	if (!text || !*text)
		return;
	UILogForm::AddMessage(xr_string(text), UILogForm::Level::Info, false);
	::LogExecCB = FALSE;
	::Msg("%s", text);
	::LogExecCB = TRUE;
}
//----------------------------------------------------
