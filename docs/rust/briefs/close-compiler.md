# Brief: the compiler's proof without a gap

The level compiler (`crates/gz-lc`, `crates/gz-lc-light`) is proven against the C++ compilers on
five scenes (`tools/qa/diff_compile.py`, the rules in `tools/qa/compare_compile.py`). The plan
(`docs/RUST_MIGRATION.md`, section 9, "Not proven against the C++") and the tool's header list
what no recorded scene exercises. The author's word now: nothing stays open. You close that
list: every line is recorded and passes, or is proven another way that is written down - none
is left as "not proven".

Read first: `docs/rust/STATE.md`, "Phase 8b", "Phase 8c" (section 1), "The final review, the
compiler", "The reviewer's verification of the fixes, closed"; the header of
`tools/qa/compare_compile.py`; `docs/RUST_MIGRATION.md` section 9 and phase 8.

**Two other engineers work in the editor's tree at the same time**: one on Play
(`crates/gz-levelbuild`, `crates/gz-game`, `tools/qa/test_play_game.py`,
`tools/qa/test_scene_tools.py`, `crates/gz-mcp/tools.json`), one on the rest
(`crates/gz-fmt-object`, `crates/gz-ltx`, `crates/gz-mod`, `crates/gz-ui-app`, `tools/rust/*`).
Your files: `crates/gz-lc`, `crates/gz-lc-light`, `crates/gz-fmt-level` (only where the compile
needs it), `tools/qa/diff_compile.py`, `tools/qa/compare_compile.py`, the C++ working copy named
below, and a section of your own appended to `docs/rust/STATE.md` ("Closing: the compiler's
proof"). Keep your crates compiling between edits (`bash tools/rust/cg.sh check -p <crate>`); if
a build fails in a file that is not yours, wait a minute and build again; never edit it.

**The C++ compilers' source** is checked out for you at `D:\Games\XFined-Editor-cpp` (the tag
`cpp-compilers-last` on a branch of its own). Its `Build.ps1 -Only compilers -Out
D:\Games\XFined-Editor\Bin\x64\CppCompilers` builds the oracle (about three minutes the first
time; a toolset with `<hash_map>`, as that script finds it). You may edit C++ there ONLY to make
the oracle run where it crashes (below) - a change that alters no result of a scene that already
passes, proven by recording `mp_test` again and comparing with its record. You commit nothing
there or anywhere.

## The list

1. **Draft quality** - recorded on the cheapest scene that shows what Draft skips (the implicit
   light, the progressive meshes, the MU light's one sample): both compilers, the rules as they
   are.
2. **The flags** `-nosun`, `-nosmg`, `-no_optimize`, `-att`, `-no_invalidefaces` - each recorded
   on the cheapest scene it changes something on (say for each what it changes there: a flag
   that changes nothing on a scene proves nothing).
3. **The abort on invalid faces** - an input with an invalid face: both compilers refuse the
   same way (the exit, `build.err` byte for byte or under a stated rule).
4. **More than two lightmap pages** - a recorded scene that needs three or more.
5. **The implicit light at the scene's own jitter** (nine samples), and the nine-sample jitter
   on a third scene.
6. **MU models and the implicit light in one level.** XrLC crashes on `mp_close_combat`'s MU
   models (an access violation in "Converting MU-models to OGFs", `calc_ogf`, twice at the same
   log line). Find the cause in the C++ (the source is there; a debug build or added log lines
   tell where). If it is a fault of the C++ on a valid input, fix it in the C++ working copy
   under the rule above, rebuild the oracle and record the scene whole, with its MU models. If
   it is the INPUT - something in the `build.prj` the Rust editor wrote that the C++ editor
   would not have written - that is a fault of the editor's level build: say exactly what, fix
   it where it is yours, and hand the rest to the lead in the report. Either way the scene ends
   recorded and passing with its MU models, or with a reason nobody can argue with.
7. **MU models at a scene's own settings** - `l03u_agr_underground` without the lowered density
   and without `-noise` (its one recording may take forty minutes: give it that limit, run it
   once), or another MU scene at its own settings if that one cannot finish - say which and why.
8. **The paged rule** (deflectors on another page): which sector a static triangle belongs to
   is compared with the triangle (the key of the multiset), and the sliding windows are held by
   a check that can fail - each progressive visual's windows against its own index buffer and
   vertex count, on both sides - since they cannot be compared across another split.
9. **No program is copied to be run**: the oracle compilers start from `Bin\x64\CppCompilers`
   itself on the stage's fs config (`-fsltx`, if their xrCore takes it - read it at the tag; if
   it does not, say what it would take), and nothing with a program's name is written under the
   temp folder by a recording.
10. **The header and the plan say what is true afterwards**: `compare_compile.py`'s list of what
    is recorded, phase by phase of `CBuild::Run`, with nothing under "never recorded" - or, for
    a line that cannot be recorded, why not and what proves it instead; the same for the plan
    (in your STATE.md section, "For the plan": the lead edits the plan).

What a new record finds wrong in the port is fixed in the port (the last pass found a real
fault in how lightmap pages are filled that way). The bounds stay as they are (lightmaps
1.5 / 16, the second picture's red and blue 3 / 16, vertex and MU light 0.2 / 2, the detail
light byte for byte) unless a new scene shows they must move - then say which scene and
channel, and never by more than twice what is seen.

## How to work

- Fully autonomous; never ask; never start agents of your own.
- Never run a git command that changes a working tree or an index (the main repository or the
  C++ working copy). The lead commits.
- Every cargo command through `bash tools/rust/cg.sh <cargo args>`. Scripts longer than a line
  go into a file under `Intermediate/` (the shell eats backslashes and quotes in heredocs).
- **Time.** A C++ recording is long by its nature and is made ONCE per scene and option set;
  the machine is shared with two engineers who build, and XrLC puts itself into the idle
  priority class, so give every recording a limit that holds (`LIMITS`, forty minutes at most)
  and report CPU seconds. A wait of yours is 300 s at most, with the run's log read between
  waits. No series of runs to chase a number. The author watches the list of your background
  commands: every long one must be a recording or the final proof.
- Cheap first: the flags and the Draft quality on the small scene take minutes; start the long
  recordings early and work on the rules while they run - never two C++ compilers at once.
- The C++ compilers create no window (`-silent`); the tool checks it. Nothing under `Bin\` is
  removed; `Bin\x64\CppCompilers` is replaced only by a build of the C++ working copy.
- The author's projects (`D:\XFinedProjects\*`) are only a source the tools copy from; the game
  is only read; `find` over `D:\XFinedProjects` and `D:\Games\Dead Air\modules` before your first
  run and after your last shows nothing changed.
- Proof at the end, once: clippy and fmt on your crates, their tests, `python
  tools/qa/diff_compile.py --all` with every new scene.

## Report

Under 450 words: each of the ten lines - what was recorded (the scene, the options, the C++ CPU
seconds) and the result, or how else it is proven; the crash of point 6 (its cause, the fix,
what it changed); what the new records found in the port and how it was fixed; the bounds; the
proofs with their times; any C++ file you changed in `D:\Games\XFined-Editor-cpp`; anything
still open and exactly why.
