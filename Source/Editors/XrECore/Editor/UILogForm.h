//---------------------------------------------------------------------------
#ifndef LogFormH
#define LogFormH
//---------------------------------------------------------------------------

// The Output Log (docs/UI_DESIGN.md, section 6): a search, All / Warnings / Errors with their counts, Auto-scroll,
// Copy and Clear over Mono lines that are drawn only where they are on screen, and the console under them. The same
// body is the panel on the Level tab and the Log drawer over any tab.
class ECORE_API UILogForm
{
public:
	enum class Level { ByPrefix, Info, Warning, Error };
	// any thread. A line of the engine's log takes its level from its "! " / "~ " / "* " prefix; the editor's own
	// report (ELog) passes its level and is always worth the status bar's last line
	static void AddMessage( const xr_string& msg, Level level = Level::ByPrefix, bool own = false);
	static void Show();
	static void Hide();
	static void Destroy();
	static void DrawBody();
	static void DrawPanel();
	static bool PanelOpen();
	static LPCSTR PanelName() { return "Output Log###Log"; }
	// What the status bar shows: the last line worth an author's attention (the editor's own reports, the errors and
	// warnings it counts; never the render and loader chatter), and the errors and warnings since CountFromHere or the
	// last Clear. A Lua traceback is one error, not one per frame of its stack.
	static void State(xr_string& last, int& errors, int& warnings);
	// The counts start again here - the status bar's and the log's Warnings / Errors alike, and the last line with them:
	// what the start and the opening of a project wrote (the engine's registry and key binding notes, the editor's own
	// scripts as a scene loads) stays under All. `mark` is the row the log shows where the counting starts.
	static void CountFromHere(LPCSTR mark = nullptr);
	// the lines the log lists: Warning or Error - the counted ones -, anything else every line (the status bar's chips)
	static void ShowLevel(Level level);
	static Level ShownLevel();
};
//---------------------------------------------------------------------------
#endif
