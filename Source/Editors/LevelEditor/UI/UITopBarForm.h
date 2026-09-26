#pragma once
// The Level tab's toolbar and the project group every tab's toolbar ends with (docs/UI_DESIGN.md, section 5).
// Drawn into the frame's toolbar row (XUI::Shell::DrawToolbar) with vector icons.
class UITopBarForm
{
public:
	UITopBarForm();
	~UITopBarForm();
	// Save | Undo, Redo; the level builds; Play and the game; configs and the game folder
	void DrawLevel(XUI::Toolbar& tb);
	// [Build Mod (split)] [Publish Release] [Releases]
	void DrawProject(XUI::Toolbar& tb);
	void RefreshBar();
	bool VerifySpaceRestrictors()const {return m_VerifySpaceRestrictors;}
private:
	void ClickUndo();
	void ClickRedo();
	void ClickSave();
	void ClickReloadConfigs();
	void ClickOpenGameData();
	void ClickCForm();
	void ClickAIMap();
	void ClickGGraph();
	void ClickPlayInEditor();
	void ClickBuildAndMake();
	void ClickTerminated();
	void ClickPlayPC();
	void ClickPlayCleanGame();
	bool m_VerifySpaceRestrictors;
};
