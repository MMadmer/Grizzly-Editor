#include "stdafx.h"

void CLevelPreferences::Load(CInifile* I)
{
	inherited::Load		(I);        
    {
        OpenObjectList = R_BOOL_SAFE("windows", "object_list", false);
     
    }
    {
        OpenProperties = R_BOOL_SAFE("windows", "properties", true);
      
    }
	{
		OpenWorldProperties = R_BOOL_SAFE("windows", "world_properties", true);

	}
    {
        OpenContentBrowser = R_BOOL_SAFE("windows", "content_browser", true);
    }
    {
        OpenWorldOutliner = R_BOOL_SAFE("windows", "world_outliner", true);
    }
    {
    }
    {
        RecoveryEnabled = R_BOOL_SAFE("recovery", "enabled", true);
        RecoveryIntervalMinutes = R_U32_SAFE("recovery", "interval_minutes", 10);
        RecoveryInteractionDelaySeconds = R_U32_SAFE("recovery", "interaction_delay_seconds", 15);
        RecoveryRetention = R_U32_SAFE("recovery", "retention", 10);
        RecoveryIntervalMinutes = clampr(RecoveryIntervalMinutes, 1u, 120u);
        RecoveryInteractionDelaySeconds = clampr(RecoveryInteractionDelaySeconds, 0u, 120u);
        RecoveryRetention = clampr(RecoveryRetention, 2u, 64u);
    }
    {
        // an empty value reads back as a null pointer, not as "" - assigning that
        // to a string is what made the editor die on startup.
        // r_string_wb, not r_string: adapter names contain spaces, and the ini
        // parser eats spaces in unquoted values - the value is stored quoted,
        // same as recent_files.
        shared_str gpu = I->line_exist("render", "gpu_adapter") ? I->r_string_wb("render", "gpu_adapter") : shared_str("");
        GpuAdapter = gpu.size() ? gpu.c_str() : "";
        // hand it to the render layer before the device is created
        SetPreferredGpu(GpuAdapter.c_str());
    }
    SceneToolsMapPairIt _I 	= Scene->FirstTool();
    SceneToolsMapPairIt _E 	= Scene->LastTool();
    for (; _I!=_E; _I++)
        if (_I->second&&(_I->first!=OBJCLASS_DUMMY))
        	_I->second->m_EditFlags.flags = R_U32_SAFE("targets",_I->second->ClassName(),_I->second->m_EditFlags.flags);
}

void CLevelPreferences::Save(CInifile* I)
{
	inherited::Save		(I);
    I->w_bool("windows", "object_list", OpenObjectList);
	I->w_bool("windows", "properties", OpenProperties);
	I->w_bool("windows", "world_properties", OpenWorldProperties);
    I->w_bool("windows", "content_browser", OpenContentBrowser);
    I->w_bool("windows", "world_outliner", OpenWorldOutliner);
    I->w_bool("recovery", "enabled", RecoveryEnabled);
    I->w_u32("recovery", "interval_minutes", RecoveryIntervalMinutes);
    I->w_u32("recovery", "interaction_delay_seconds", RecoveryInteractionDelaySeconds);
    I->w_u32("recovery", "retention", RecoveryRetention);
    // only write a real choice: a key with an empty value is what the reader
    // above chokes on, and "no key" already means "system default".
    // quoted on disk - the ini parser eats spaces in unquoted values, and
    // adapter names are full of them
    if (!GpuAdapter.empty())
    {
        xr_string quoted; quoted.sprintf("\"%s\"", GpuAdapter.c_str());
        I->w_string("render", "gpu_adapter", quoted.c_str());
    }
    SceneToolsMapPairIt _I 	= Scene->FirstTool();
    SceneToolsMapPairIt _E 	= Scene->LastTool();
    for (; _I!=_E; _I++)
        if (_I->second&&(_I->first!=OBJCLASS_DUMMY))	I->w_u32	("targets",_I->second->ClassName(),_I->second->m_EditFlags.get());
}

void CLevelPreferences::OnEnabledChange(PropValue* prop)
{
	ESceneToolBase* M		= Scene->GetTool(prop->tag); VERIFY(M);
	ExecCommand				(COMMAND_ENABLE_TARGET,prop->tag,M->IsEnabled());
}

void CLevelPreferences::OnReadonlyChange(PropValue* prop)
{
	ESceneToolBase* M		= Scene->GetTool(prop->tag); VERIFY(M);
	ExecCommand				(COMMAND_READONLY_TARGET,prop->tag,M->IsForceReadonly());
}

void CLevelPreferences::FillProp(PropItemVec& items)
{
	inherited::FillProp	(items);
    AddPageIcon("Recovery", XUI::Icon::History);
    AddPageIcon("Scene", XUI::Icon::Level);
    PHelper().CreateBOOL(items, "Recovery\\Enabled", &RecoveryEnabled);
    PHelper().CreateU32(items, "Recovery\\Interval", &RecoveryIntervalMinutes, 1, 120);
    PHelper().CreateU32(items, "Recovery\\Delay After Input", &RecoveryInteractionDelaySeconds, 0, 120);
    PHelper().CreateU32(items, "Recovery\\Snapshots Kept", &RecoveryRetention, 2, 64);
    AddHelp("Recovery\\Enabled", "Keeps snapshots of the open scene while it has unsaved changes, and offers the last one after "
        "the editor did not close cleanly.");
    AddUnit("Recovery\\Interval", "min");
    AddUnit("Recovery\\Delay After Input", "s");
    AddHelp("Recovery\\Interval", "Time between two snapshots of the open scene.");
    AddHelp("Recovery\\Delay After Input", "How long a snapshot waits after the last click or key, so it never interrupts work.");
    AddHelp("Recovery\\Snapshots Kept", "How many snapshots of a scene are kept; the oldest go first.");
    SceneToolsMapPairIt _I 	= Scene->FirstTool();
    SceneToolsMapPairIt _E 	= Scene->LastTool();
    for (; _I!=_E; _I++)
        if (_I->second&&(_I->first!=OBJCLASS_DUMMY)){
        	if (_I->second->AllowEnabling()){
                const shared_str key = PrepareKey("Scene\\Targets\\Enable",_I->second->ClassDesc());
                PropValue* V 	= PHelper().CreateFlag32(items,key,	&_I->second->m_EditFlags, ESceneToolBase::flEnable);
                AddHelp(key.c_str(), "Loads this part of the scene; off unloads it.");
                V->tag			= _I->second->FClassID;
                V->OnChangeEvent.bind(this,&CLevelPreferences::OnEnabledChange);
            }
		    const shared_str key = PrepareKey("Scene\\Targets\\Read Only",_I->second->ClassDesc());
		    PropValue* V		= PHelper().CreateFlag32(items,key,	&_I->second->m_EditFlags, ESceneToolBase::flForceReadonly);
		    AddHelp(key.c_str(), "This part of the scene can be looked at, but not changed or saved.");
            V->tag				= _I->second->FClassID;
            V->OnChangeEvent.bind(this,&CLevelPreferences::OnReadonlyChange);
        }
}

