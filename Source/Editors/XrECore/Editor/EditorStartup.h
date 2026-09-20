#pragma once

//------------------------------------------------------------------------------
// The startup: everything between the double-click and the first frame.
//
// The editor reads a great deal before it can draw anything. Measured on a warm
// cache it is ~19k file reads and ~88k directory/metadata operations for 26 MB
// of data - the cost is the NUMBER of touches, not their size. Warm that is a
// few seconds; cold (a file cache that has aged out, an on-access virus scanner
// meeting those files for the first time) the same walk takes a minute, and for
// that whole minute nothing appears on the desktop at all.
//
// Two answers live here:
//
//   * a splash window, up before the first file is touched. It runs on a thread
//     of its own, so it keeps painting and answering Windows while the main
//     thread sits in a scan that takes as long as it takes.
//   * a stage-by-stage timing of the start, written into the log at the first
//     frame. A slow start is then read off the log instead of guessed at - and
//     the stage named "loader" is the part that belongs to Windows and the
//     scanner, before a line of editor code runs.
//
// Nothing here allocates through the engine or logs before Core is up: the
// first call happens in wWinMain, ahead of Core._initialize.
//------------------------------------------------------------------------------

namespace EditorStartup
{
	// Per-Monitor V2 awareness for the process, once, plus the scale windows
	// are sized with. Awareness cannot be changed after the first window
	// exists, so the splash and the main window share this one call.
	ECORE_API float	DpiScale	();

	// Puts the splash on screen and starts the profile.
	ECORE_API void	SplashShow	();

	// Closes the stage that is running and opens `name` - which the splash
	// shows and the log names. Short and user-facing ("Scanning files").
	ECORE_API void	Stage		(LPCSTR name);

	// The first frame is the first thing worth looking at: closes the splash
	// and writes the profile line. Every later call does nothing.
	ECORE_API void	Finish		();
}
