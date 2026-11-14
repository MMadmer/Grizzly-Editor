#!/usr/bin/env python3
"""The command line and the editor's process lifecycle: Grizzly-Cmd.exe (the editor for a shell) and the programs
behind it, each in a home of its own, on a COPY of a project where one is needed.

What it proves:
  A  `run`: one command, its answer on stdout (pretty, or one line with -json), nothing on stderr, exit 0;
     name=value parameters are read as text; `help` prints the areas, an area, a command's card
  B  exit codes: a command that fails is 1 with the sentence on stderr and nothing on stdout (a wrong parameter
     with the command's usage, an unknown command with its neighbours, an unknown help topic); a run that could not
     start is 2 (no command, no script, a script that cannot be read)
  C  `script`: a command a line, comments and blank lines skipped, stops at the first failure (exit 1) and runs
     past it with -keep_going; `-` reads the standard input
  D  -project: the commandlet opens the project, answers from it and ends; -quiet drops the job progress
  E  a commandlet listens on no port and writes a log of its own (grizzly_cmd_*), not the editor's
  F  a reader that leaves is no error: `help` into a closed pipe ends with 0 and no crash report; a headless
     editor whose log echo lost its reader answers on and ends with 0
  G  the twin: a headless editor started through Grizzly-Cmd.exe prints through it and ends it with the editor's
     exit code when closed over MCP; ended by force, the twin takes a headless editor with it; an editor with a
     window outlives its twin and starts minimised because the twin was
  H  a console's end: in a console of its own Ctrl+Break closes a headless editor started through the twin the way
     a quit does (exit 0, "Grizzly Editor closed" in its log, also while it opens its project); an editor with a
     window started there with -log_echo joins no console and lives on after it
  I  library.sync and library.rewrite_descriptions in a commandlet: a throw-away library (-root) is synchronised and
     its descriptions rewritten by the commands themselves - they answer what they did, not "started" - with no
     window, exit 0; a library that is no kind of theirs is refused with the real ones, exit 1
  J  a module from nothing, as an agent meets it: project.create in a temp folder, `check` says what to write in
     mod.ltx (the key, its values, the command) at the line of the section, `help mod.set` names the target's values,
     mod.set sets one, `check` is clean (exit 0), `reference manifest` prints the format, mod.build target=folder
     builds it - no game is needed; then a mod.ltx written by hand with one wrong value per rule: `check` is exit 1
     and names each with its line, mod.build refuses and says them, the warnings alone (a key or a value no reader
     takes) are exit 0 and listed, a manifest put right is clean; a quest a crashed packaging left under
     temp\release_stage_N is no quest of the project

    python tools/qa/test_cli.py --project <project>
"""
from __future__ import annotations

import ctypes
import glob
import json
import os
import signal
import subprocess
import sys
import tempfile
import time

import qa
from qa import check, until
from test_assets import QUEST
from test_previews import write_tga

CLI = os.path.join(os.path.dirname(qa.exe()), "Grizzly-Cmd.exe")


def home_of(work: str, name: str) -> str:
    return os.path.join(work, "home_" + name)


def cli(*args: str, home: str, stdin: str | None = None, seconds: int = 90) -> tuple[int, str, str]:
    """One run of Grizzly-Cmd.exe under a time limit: exit code, stdout, stderr. The twin runs the editor beside it
    and passes its streams on."""
    done = subprocess.run([sys.executable, qa.LIMITED, str(seconds), CLI, *args, "-home", home], input=stdin,
                          capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=seconds + 30)
    return done.returncode, done.stdout, done.stderr


def jsons(text: str) -> list[dict]:
    """The JSON objects in a text, pretty or one to a line."""
    out, depth, start = [], 0, None
    for i, ch in enumerate(text):
        if ch == "{":
            if depth == 0:
                start = i
            depth += 1
        elif ch == "}" and depth:
            depth -= 1
            if depth == 0 and start is not None:
                try:
                    out.append(json.loads(text[start:i + 1]))
                except ValueError:
                    pass
    return out


def phase_a(work: str) -> None:
    home = home_of(work, "a")
    code, out, err = cli("run", "ping", home=home)
    answer = (jsons(out) or [{}])[0]
    check(code == 0 and answer.get("name") == "Grizzly Editor" and str(answer.get("version", "")).count(".") == 2
          and not err.strip(), "A run ping: exit 0, the answer on stdout, nothing on stderr", (code, out[:200], err[:200]))
    check("\n  " in out, "A the answer is printed readable", out[:120])
    # no command line of this suite says -qa_github: the environment the QA client sets makes every program a QA run
    check(answer.get("qa") is True, "A a commandlet a suite starts is a QA run: it keeps to this computer", out[:200])
    code, out, err = cli("run", "ping", "-json", home=home)
    lines = [l for l in out.splitlines() if l.strip()]
    check(code == 0 and len(lines) == 1 and json.loads(lines[0]).get("ok") is True,
          "A -json prints the raw answer on one line", (code, out[:200]))
    code, out, _ = cli("run", "help", "about=scene.place", home=home)
    check(code == 0 and "scene.place" in out and "kind" in out, "A name=value reaches the command as text", out[:200])
    code, out, _ = cli("help", home=home)
    check(code == 0 and "Areas" in out and "scene" in out and "Topics" in out, "A help lists the areas and the topics",
          out[:200])
    code, out, _ = cli("help", "scene", home=home)
    check(code == 0 and "scene.place" in out, "A help <area> lists its commands", out[:200])
    code, out, _ = cli("help", "scene.place", home=home)
    check(code == 0 and "(required)" in out and "Example:" in out, "A help <command> prints its card", out[:200])


def phase_b(work: str) -> None:
    home = home_of(work, "b")
    code, out, err = cli("run", "no.such", home=home)
    check(code == 1 and not out.strip() and "unknown command 'no.such'" in err,
          "B an unknown command is 1, the sentence on stderr, nothing on stdout", (code, out[:100], err[:200]))
    code, out, err = cli("run", "scene.plase", home=home)
    check(code == 1 and "did you mean" in err and "scene.place" in err, "B and names its neighbours", err[:300])
    code, out, err = cli("run", "ping", "bogus=1", home=home)
    check(code == 1 and not out.strip() and "no parameter 'bogus'" in err, "B a wrong parameter is 1, named", err[:300])
    code, out, err = cli("run", "scene.place", home=home)
    check(code == 1 and "missing field" in err and "(required)" in err,
          "B a missing parameter is 1 with the command's usage", err[:300])
    code, out, err = cli("help", "nosuch", home=home)
    check(code == 1 and "knows no area, command or topic" in err, "B an unknown help topic is 1", err[:200])
    code, out, err = cli("run", home=home)
    check(code == 2 and "usage:" in err and not out.strip(), "B no command is 2 with the usage on stderr", (code, err[:200]))
    code, out, err = cli("script", home=home)
    check(code == 2 and "usage:" in err, "B no script is 2 with the usage", (code, err[:200]))
    code, out, err = cli("script", os.path.join(work, "no_such_script.txt"), home=home)
    check(code == 2 and "cannot be read" in err, "B a script that cannot be read is 2, in words", (code, err[:200]))


def phase_c(work: str) -> None:
    home = home_of(work, "c")
    script = os.path.join(work, "three.txt")
    qa.write(script, "ping\n# a comment\n\nhelp about=scene\nno.such\nping\n")
    count = lambda text: text.count('"name": "Grizzly Editor"')  # noqa: E731
    code, out, err = cli("script", script, home=home)
    check(code == 1 and count(out) == 1 and "scene.open" in out and "unknown command 'no.such'" in err,
          "C a script stops at the first failure: what ran before it printed, what follows did not", (code, count(out), err[:200]))
    code, out, err = cli("script", script, "-keep_going", home=home)
    check(code == 1 and count(out) == 2 and "unknown command 'no.such'" in err,
          "C -keep_going runs past it and still ends 1", (code, count(out), err[:200]))
    code, out, err = cli("script", "-", home=home, stdin="ping\nping\n")
    check(code == 0 and count(out) == 2, "C `-` reads the script from the standard input", (code, out[:200], err[:200]))


def phase_d(work: str, project: str) -> None:
    home = home_of(work, "d")
    code, out, err = cli("run", "mod.info", "-project", project, "-json", home=home)
    answer = (jsons(out) or [{}])[0]
    check(code == 0 and answer.get("ok") and (answer.get("manifest") or {}).get("id"),
          "D -project opens the project and the command answers from it", (code, out[:200], err[:200]))
    target = os.path.join(work, "built")
    code, out, err = cli("run", "mod.build", "target=folder", "path=" + target, "-project", project, "-json", home=home)
    answer = (jsons(out) or [{}])[0]
    check(code == 0 and answer.get("ok") and os.path.isdir(os.path.join(target, "modules")),
          "D a command with a job answers when the job has ended, and the work is done", (code, out[:200], err[:200]))
    code, out, quiet = cli("run", "mod.build", "target=folder", "path=" + target, "-project", project, "-quiet",
                           home=home)
    check(code == 0 and not quiet.strip(), "D -quiet drops the job progress from stderr", quiet[:200])
    # an unopenable project is a run that could not start
    code, out, err = cli("run", "ping", "-project", os.path.join(work, "no_such_project"), home=home)
    check(code == 2 and err.strip(), "D a project that cannot be opened is 2, in words", (code, err[:200]))


def phase_e(work: str, project: str) -> None:
    home = home_of(work, "e")
    port = qa.free_port()
    target = os.path.join(work, "built_e")
    proc = subprocess.Popen([sys.executable, qa.LIMITED, "120", CLI, "run", "mod.build", "target=folder",
                             "path=" + target, "-project", project, "-mcp_port", str(port), "-home", home, "-json"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    listened = False
    while proc.poll() is None:
        listened = listened or qa.port_open(port)
        time.sleep(0.03)
    check(proc.returncode == 0 and not listened, "E a commandlet listens on no port, though -mcp_port names one",
          (proc.returncode, listened))
    logs = sorted(os.path.basename(p) for p in glob.glob(os.path.join(home, "logs", "*.log")))
    check(any(n.startswith("grizzly_cmd") for n in logs), "E it writes a log of its own, grizzly_cmd_*", logs)
    mine = glob.glob(os.path.join(home, "logs", "grizzly_cmd*.log"))
    text = qa.read(mine[0]) if mine else ""
    check(len(text) > 0, "E and it says what it did", text[-200:])


def crash_reports(home: str) -> list[str]:
    return glob.glob(os.path.join(home, "logs", "*crash*"))


def phase_f(work: str) -> None:
    home = home_of(work, "f")
    proc = subprocess.Popen([sys.executable, qa.LIMITED, "60", CLI, "help", "-home", home], stdout=subprocess.PIPE)
    first = proc.stdout.readline()
    proc.stdout.close()
    try:
        code = proc.wait(timeout=60)
    except subprocess.TimeoutExpired:
        proc.kill()
        code = None
    check(bool(first.strip()) and code == 0 and not crash_reports(home),
          "F `help` whose reader leaves after one line ends with 0 and no crash report",
          (first[:60], code, crash_reports(home)))
    port = qa.free_port()
    home2 = home_of(work, "f2")
    echo = subprocess.Popen([qa.exe(), "-mcp_port", str(port), "-home", home2, "-headless", "-nodlg", "-log_echo"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        up = until(lambda: qa.port_open(port), 60)
        echoed = echo.stderr.readline()
        echo.stderr.close()
        # the editor goes on logging into a stream nobody reads, and closes as it would have
        answers = [qa.call(port, "project.list").get("ok") for _ in range(3)]
        qa.call(port, "app.quit", discard=True)
        try:
            code = echo.wait(timeout=30)
        except subprocess.TimeoutExpired:
            code = None
        check(up and bool(echoed.strip()) and all(answers) and code == 0 and not crash_reports(home2),
              "F a headless editor whose log echo lost its reader answers on and ends with 0, no crash report",
              (up, echoed[:60], answers, code, crash_reports(home2)))
    finally:
        if echo.poll() is None:
            echo.kill()
            echo.wait(timeout=30)


def hidden() -> subprocess.STARTUPINFO:
    info = subprocess.STARTUPINFO()
    info.dwFlags = subprocess.STARTF_USESHOWWINDOW
    info.wShowWindow = qa.SW_HIDDEN
    return info


def phase_g(work: str) -> None:
    home = home_of(work, "g")
    port = qa.free_port()
    args = [CLI, "-mcp_port", str(port), "-home", home, "-headless", "-nodlg", "-log_echo"]
    out = os.path.join(work, "twin_out.txt")
    with open(out, "wb") as f:
        proc = subprocess.Popen(args, stdout=f, stderr=subprocess.STDOUT)
    try:
        up = until(lambda: qa.port_open(port), 60)
        qa.call(port, "app.quit", discard=True)
        try:
            code = proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            code = None
        text = qa.read(out)
        check(up and code == 0 and "control endpoint" in text and "Grizzly Editor closed" in text,
              "G a headless editor prints through the twin and ends it with exit code 0 when closed over MCP",
              (up, code, text[-300:]))
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=30)

    # the twin ended the way a closed console ends it: the editor does not outlive it
    proc = subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        up = until(lambda: qa.port_open(port), 60)
        proc.kill()
        proc.wait(timeout=30)
        gone = until(lambda: not qa.port_open(port), 15)
        check(up and gone, "G a headless editor whose twin is ended by force is gone with it", (up, gone))
    finally:
        if qa.port_open(port):
            qa.call(port, "app.quit", discard=True)
            until(lambda: not qa.port_open(port), 15)

    # an editor with a window is the editor as Explorer starts it: the twin's end is not its end. The twin is started
    # hidden and passes that on, so the window stays off the author's screen
    proc = subprocess.Popen([CLI, "-mcp_port", str(port), "-home", home, "-nodlg"], stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL, startupinfo=hidden())
    try:
        up = until(lambda: qa.port_open(port), 60)
        proc.kill()
        proc.wait(timeout=30)
        time.sleep(2.0)
        alive = qa.port_open(port)
        check(up and alive, "G an editor with a window outlives its twin", (up, alive))
    finally:
        if proc.poll() is None:
            proc.kill()
        if qa.port_open(port):
            qa.call(port, "app.quit", discard=True)
            check(until(lambda: not qa.port_open(port), 30), "G that editor closes over MCP")
    # the start-up profile of that run, whole in the log once the editor has closed, says how the window came up
    time.sleep(1.0)
    marks = ""
    for path in glob.glob(os.path.join(home, "logs", "grizzly_*.log")):
        marks += " | ".join(line.strip() for line in qa.read(path).splitlines()
                            if "window minimised" in line or "window shown" in line)
    check("window minimised" in marks and "window shown" not in marks,
          "G the twin passes a minimised start on to the editor", marks[:200] or "no mark in the log")


def console_inner(out: str, project: str, port: int, home: str) -> int:
    """In a console of its own: Ctrl+Break on a headless editor started through the twin, and an editor with a
    window started here with -log_echo. Writes what it saw to `out`; the windowed editor is left running."""
    seen: dict = {}
    # with a project to open: the endpoint answers before the project is open, and the break right then finds
    # the editor opening it
    proc = subprocess.Popen([CLI, "-mcp_port", str(port), "-home", home, "-headless", "-nodlg", "-project", project],
                            creationflags=subprocess.CREATE_NEW_PROCESS_GROUP)
    seen["up"] = until(lambda: qa.port_open(port), 60)
    proc.send_signal(signal.CTRL_BREAK_EVENT)
    try:
        seen["exit"] = proc.wait(timeout=30)
    except subprocess.TimeoutExpired:
        seen["exit"] = None
        proc.kill()
    seen["gone"] = until(lambda: not qa.port_open(port), 15)
    tail = ""
    for path in glob.glob(os.path.join(home, "logs", "grizzly_*.log")):
        tail += "".join(qa.read(path).splitlines(True)[-3:])
    seen["closed"] = "Grizzly Editor closed" in tail

    # an editor with a window, typed into this console with -log_echo: it joins no console
    window = subprocess.Popen([qa.exe(), "-mcp_port", str(port), "-home", home, "-nodlg", "-log_echo"],
                              startupinfo=hidden())
    seen["window_up"] = until(lambda: qa.port_open(port), 60)
    pids = (ctypes.c_uint32 * 64)()
    count = ctypes.windll.kernel32.GetConsoleProcessList(pids, 64)
    seen["console"] = [int(x) for x in pids[:count]]
    seen["window_pid"] = window.pid
    with open(out, "w", encoding="utf-8") as f:
        json.dump(seen, f)
    return 0


def phase_h(work: str, project: str) -> None:
    port = qa.free_port()
    home = home_of(work, "h")
    out = os.path.join(work, "console.json")
    subprocess.run([sys.executable, os.path.abspath(__file__), "--console-inner", out, project, str(port), home],
                   creationflags=subprocess.CREATE_NEW_CONSOLE, startupinfo=hidden(), timeout=240)
    seen = {}
    try:
        if os.path.isfile(out):
            with open(out, encoding="utf-8") as f:
                seen = json.load(f)
        check(seen.get("up") and seen.get("exit") == 0 and seen.get("gone") and seen.get("closed"),
              "H Ctrl+Break in its console closes a headless editor started through the twin the way a quit does",
              json.dumps(seen)[:300])
        # that console is gone now (its only program, the inner run, ended)
        time.sleep(2.0)
        alive = qa.port_open(port)
        check(bool(seen.get("window_up")) and seen.get("window_pid") not in (seen.get("console") or [None]) and alive,
              "H an editor with a window started in a console with -log_echo is not in it and lives on after it",
              (alive, json.dumps(seen)[:300]))
    finally:
        if qa.port_open(port):
            qa.call(port, "app.quit", discard=True)
            check(until(lambda: not qa.port_open(port), 30), "H that editor closes over MCP")


def phase_i(work: str) -> None:
    home = home_of(work, "i")
    lib = os.path.join(work, "library_i")
    write_tga(os.path.join(lib, "rawdata", "textures", "qa", "qa_cli.tga"))
    code, out, err = cli("run", "library.sync", "kind=texture_library", "-root", lib, "-json", home=home)
    answer = (jsons(out) or [{}])[0]
    check(code == 0 and answer.get("ok") and answer.get("updated") == 1 and "started" not in answer
          and answer.get("names") == [r"qa\qa_cli"], "I a commandlet synchronises a library: the answer says what it made",
          (code, out[:300], err[:200]))
    check(os.path.isfile(os.path.join(lib, "gamedata", "textures", "qa", "qa_cli.dds"))
          and os.path.isfile(os.path.join(lib, "rawdata", "textures", "qa", "qa_cli.thm")),
          "I the game texture and the description are there")
    code, out, err = cli("run", "library.sync", "kind=texture_library", "-root", lib, "-json", home=home)
    answer = (jsons(out) or [{}])[0]
    check(code == 0 and answer.get("updated") == 0 and answer.get("up_to_date") == 1,
          "I a second run finds it current and writes nothing", (code, out[:300], err[:200]))
    code, out, err = cli("run", "library.rewrite_descriptions", "-root", lib, "-json", home=home)
    answer = (jsons(out) or [{}])[0]
    check(code == 0 and answer.get("ok") and answer.get("unchanged") == 1 and "started" not in answer,
          "I a commandlet rewrites the descriptions and says how many it left as they were", (code, out[:300], err[:200]))
    code, out, err = cli("run", "library.sync", "kind=textures", "-root", lib, home=home)
    check(code == 1 and "texture_library" in err and "sound_library" in err,
          "I a kind that is none of the two is refused with the real ones", (code, err[:300]))


def phase_u(work: str) -> None:
    """U: nothing saves by itself. A run that ends with unsaved work loses it, so it ends with 1 and says which on
    stderr; the same run with its save ends with 0 and the work is in the file."""
    home = home_of(work, "u")
    parent = os.path.join(work, "projects_u")
    code, out, err = cli("run", "project.create", "name=QaUnsaved", "parent=" + parent, "-json", home=home)
    project = qa.guard_project(os.path.join(parent, "QaUnsaved"))
    check(code == 0, "U a project made from nothing", (code, out[:200], err[:200]))
    steps = ["scene.new", "scene.save path=maps/a.scene", "scene.place kind=light position=[0,1,0]"]
    lost = os.path.join(work, "u_lost.txt")
    qa.write(lost, "\n".join(steps) + "\n")
    code, out, err = cli("script", lost, "-project", project, home=home)
    check(code == 1 and "unsaved work" in err and "scene.save" in err,
          "U a run that ends over an unsaved scene ends with 1 and says so on stderr", (code, err[-300:]))
    count = os.path.join(work, "u_count.txt")
    qa.write(count, "scene.open path=maps/a.scene\nscene.objects classes=light\n")
    code, out, err = cli("script", count, "-project", project, "-json", home=home)
    totals = [j.get("total") for j in jsons(out) if "total" in j]
    check(code == 0 and totals == [0], "U and the scene's file holds nothing of it", (code, totals, err[-200:]))
    kept = os.path.join(work, "u_kept.txt")
    qa.write(kept, "\n".join(["scene.open path=maps/a.scene", steps[2], "scene.save path=maps/a.scene"]) + "\n")
    code, out, err = cli("script", kept, "-project", project, home=home)
    check(code == 0 and "unsaved work" not in err, "U the same run with its save ends with 0", (code, err[-300:]))
    code, out, err = cli("script", count, "-project", project, "-json", home=home)
    totals = [j.get("total") for j in jsons(out) if "total" in j]
    check(code == 0 and totals == [1], "U and the light is in the file", (code, totals, err[-200:]))


def phase_j(work: str) -> None:
    home = home_of(work, "j")
    parent = os.path.join(work, "projects_j")
    code, out, err = cli("run", "project.create", "name=QaManifest", "parent=" + parent, "-json", home=home)
    answer = (jsons(out) or [{}])[0]
    check(code == 0 and answer.get("ok"), "J project.create makes a project from nothing", (code, out[:200], err[:200]))
    project = qa.guard_project(os.path.join(parent, "QaManifest"))
    manifest = qa.read(os.path.join(project, "mod.ltx"))
    check("[module]" in manifest and "target" not in manifest, "J the new mod.ltx states no target yet", manifest[:200])

    # the first check: one error, at the [module] line, naming the key, its values and the command
    code, out, err = cli("check", "-project", project, "-json", home=home)
    problems = (jsons(out) or [{}])[0].get("problems", [])
    first = problems[0] if problems else {}
    text = first.get("text", "")
    check(code == 1 and len(problems) == 1 and first.get("code") == "mod.target"
          and first.get("file") == "mod.ltx" and first.get("line") == 1,
          "J the first check is one error, mod.target, at the [module] line of mod.ltx", (code, out[:300], err[:200]))
    check(all(w in text for w in ("target = default", "mode = <id>", "target = existing", "[provides_mode]",
                                  "target = new", "mod.set set=", "reference manifest"))
          and first.get("fix") == "target = default",
          "J its sentence names the key, the values, the command and the fix", text)
    check(text.find("mod.set") < text.find("Mod Settings") and text.find("target = default") < text.find("Mod Settings"),
          "J and the window comes last", text)

    # mod.set's card says the values: a target is set from what it says, with no refusal first
    code, out, err = cli("help", "mod.set", home=home)
    check(code == 0 and all(w in out for w in ("`default`", "`existing`", "`new`", "`any`", "provides_mode")),
          "J help mod.set names the target's values", out[:500])
    code, out, err = cli("run", "mod.set", 'set={"module":{"target":"default"}}', "-project", project, "-json",
                         home=home)
    answer = (jsons(out) or [{}])[0]
    check(code == 0 and answer.get("ok") and answer.get("manifest", {}).get("target") == "default"
          and "target      = default" in qa.read(os.path.join(project, "mod.ltx")),
          "J mod.set writes target = default into mod.ltx", (code, out[:300], err[:200]))
    code, out, err = cli("run", "mod.set", 'set={"module":{"target":"sometimes"}}', "-project", project, home=home)
    check(code == 1 and all(w in err for w in ("default", "existing", "new", "any")),
          "J a wrong target is refused with every value", (code, err[:300]))

    code, out, err = cli("check", "-project", project, home=home)
    check(code == 0, "J the second check is clean, exit 0", (code, out[:300], err[:200]))

    # the format is printed, for the manifest as for every other kind
    code, out, err = cli("reference", "manifest", "-project", project, home=home)
    check(code == 0 and all(w in out for w in ("[module]", "[provides_mode]", "[world]", "target = default",
                                               "Example", "mode = <id>")),
          "J reference manifest prints the format with its example", (code, out[:300], err[:200]))
    for word in ("mod.ltx", "module manifest"):
        code, again, err = cli("reference", word, "-project", project, home=home)
        check(code == 0 and again == out, "J reference %s is the same page" % word, (code, err[:200]))
    code, out, err = cli("reference", "nothing", home=home)
    check(code == 1 and "module manifest" in err, "J an unknown kind names the manifest among the kinds", err[:300])

    target = os.path.join(work, "built_j")
    code, out, err = cli("run", "mod.build", "target=folder", "path=" + target, "-project", project, "-json", home=home)
    answer = (jsons(out) or [{}])[0]
    check(code == 0 and answer.get("ok") and os.path.isfile(os.path.join(target, "modules", "qamanifest", "mod.ltx")),
          "J mod.build target=folder builds the module", (code, out[:300], err[:200]))
    agents = qa.read(os.path.join(project, "AGENTS.md"))
    check("reference manifest" in agents and "target = default" in agents,
          "J the project's AGENTS.md names the manifest's reference and the target", agents[:400])
    manifest_values(work, home, project)
    manifest_rest(work, home, project)
    editors_scratch(work, home, project)


# one wrong value per rule, written the way an agent writes a file: (the line, the code, what the sentence names)
WRONG = [
    ("version     = v1.2-beta", "mod.version", "version takes 1 to 4 numbers"),
    ("website     = http://evil.example/x", "mod.website", "website takes an https link on ap-pro.ru or moddb.com"),
    ("tags        = banana, Story", "mod.tags", "'banana' is no tag"),
    ('description = "%s"' % ("x" * 4100), "mod.description", "description takes text, 4095 bytes at most"),
    ("target      = sideways", "mod.target_value", "default | existing | new | any"),
    ("mode        = Not A Mode!", "mod.mode", "mode takes a game mode id"),
    ("github      = not a repo", "mod.github", "github takes owner/repo"),
    ("bad-name    = 1", "mod.var_name", "letters, digits and _"),
    ("kills       = 1", "mod.var_twice", "declared twice"),
]
# what the editor does not refuse but says: a key no reader takes, a value none of them reads as written
WARNED = [
    ("bogus_rule  = true", "mod.key", "no reader takes this key"),
    ("mutants     = maybe", "mod.value", "mutants takes true | false"),
    ("loading_screens = sometimes", "mod.value", "loading_screens takes mix | only"),
]


def hand_written(wrong: bool, warned: bool) -> str:
    """A mod.ltx of the project, the lines in the order a person writes them."""
    lines = ["[module]", "id          = qamanifest", "name        = QaManifest"]
    if wrong:
        lines += [WRONG[0][0], WRONG[1][0], WRONG[2][0], WRONG[3][0], "api         = 1", WRONG[4][0], WRONG[5][0],
                  "[update]", WRONG[6][0], "[vars]", WRONG[7][0], "kills       = 0", WRONG[8][0]]
    else:
        lines += ["version     = 1.0.0", "api         = 1", "target      = default"]
    if warned:
        lines += ["[world]", WARNED[0][0], WARNED[1][0], "[screens]", WARNED[2][0]]
    return "\r\n".join(lines) + "\r\n"


def manifest_values(work: str, home: str, project: str) -> None:
    path = os.path.join(project, "mod.ltx")
    good = qa.read(path)

    def found() -> tuple[int, list[dict], str]:
        code, out, err = cli("check", "-project", project, "-json", home=home)
        return code, (jsons(out) or [{}])[0].get("problems", []), err

    # the wrong values, each named with its line
    text = hand_written(True, True)
    qa.write(path, text)
    code, problems, err = found()
    check(code == 1, "J a mod.ltx with a wrong value per rule is exit 1", (code, err[:200]))
    lines = text.splitlines()
    for row, name, says in WRONG + WARNED:
        want = lines.index(row) + 1
        hit = [p for p in problems if p.get("code") == name and p.get("line") == want]
        check(len(hit) >= 1 and hit[0].get("file") == "mod.ltx" and says in hit[0].get("text", ""),
              "J check names %s at line %d of mod.ltx" % (name, want), json.dumps(problems)[:600])
    wrong_codes = {name for _, name, _ in WRONG}
    check(all(p.get("severity") == "error" for p in problems if p.get("code") in wrong_codes)
          and all(p.get("severity") == "warning" for p in problems if p.get("code") in ("mod.key", "mod.value")),
          "J the wrong values are errors, the key and the values nobody reads are warnings", json.dumps(problems)[:300])
    case = [p for p in problems if p.get("code") == "mod.tags_case"]
    check(len(case) == 1 and case[0].get("severity") == "warning" and "'Story'" in case[0].get("text", ""),
          "J a tag in the wrong case is a warning of its own", json.dumps(case))

    # the build follows: it refuses the errors with the sentences, and builds nothing
    out_dir = os.path.join(work, "built_wrong")
    code, out, err = cli("run", "mod.build", "target=folder", "path=" + out_dir, "-project", project, home=home)
    check(code == 1 and "version takes 1 to 4 numbers" in err and "mode takes a game mode id" in err
          and not os.path.isdir(os.path.join(out_dir, "modules", "qamanifest")),
          "J mod.build refuses the wrong values and says them", (code, err[:400]))
    code, out, err = cli("run", "mod.info", "-project", project, "-json", home=home)
    info = (jsons(out) or [{}])[0]
    listed = {p.get("code") for p in info.get("problems", [])}
    check(code == 0 and wrong_codes <= listed and {"mod.key", "mod.value"} <= listed,
          "J mod.info lists the same problems", (code, sorted(listed)))

    # the warnings alone: exit 0, listed, and the build goes through and reports them
    qa.write(path, hand_written(False, True))
    code, problems, err = found()
    kinds = sorted(p.get("code") for p in problems)
    check(code == 0 and kinds == ["mod.key", "mod.value", "mod.value"]
          and all(p.get("severity") == "warning" for p in problems),
          "J a key or a value no reader takes is a warning: exit 0 and listed", (code, kinds, err[:200]))
    code, out, err = cli("run", "mod.build", "target=folder", "path=" + out_dir, "-project", project, "-json",
                         home=home)
    answer = (jsons(out) or [{}])[0]
    said = json.dumps(answer)
    check(code == 0 and answer.get("ok") and "bogus_rule" in said and "loading_screens" in said,
          "J mod.build builds it and reports the warnings", (code, said[:500], err[:200]))

    # put right: clean
    qa.write(path, good)
    code, problems, err = found()
    check(code == 0 and not problems, "J the manifest put right is clean", (code, json.dumps(problems)[:300]))


# what the page lists and the check let pass before, written by hand, one per line: (the line, the code, what it names)
LEFT = [
    ("version     =", "mod.version", "[module] version is empty"),
    ("versoin     = 2.0", "mod.key", "'version'"),
    ("Tags        = story", "mod.key", "'tags'"),
    ("api         = banana", "mod.value", "api takes 1"),
    ("target      = default", "mod.target_default", "[module] target = default with mode = night"),
    ("[screen.Bad Id]", "mod.section_id", "the id of a screen takes a-z, 0-9 and _"),
    ("layer       = art/gone.dds", "E401", "screen bad id: layer 1: the file does not exist"),
    ("opacity     = 7", "mod.value", "opacity takes 0 to 1"),
    ("file        = music/t.wav", "E409", "track t1: the game plays .ogg files only"),
    ("[track.t1]", "W410", "track t1: it joins no shuffle"),
    ("cheat       = maybe", "mod.value", "cheat takes true | false"),
    ("wpn_knife   = 0", "E207", "start kit item 'wpn_knife' has a count of 0"),
]


def manifest_rest(work: str, home: str, project: str) -> None:
    """The values the page lists and the pairs of target and mode, each named at its line; the areas' problems at
    their key; mod.set refusing the pairs with the check's sentence."""
    path = os.path.join(project, "mod.ltx")
    good = qa.read(path)

    def found() -> tuple[int, list[dict], str]:
        code, out, err = cli("check", "-project", project, "-json", home=home)
        return code, (jsons(out) or [{}])[0].get("problems", []), err

    lines = ["[module]", "id          = qamanifest", "name        = QaManifest", "version     =", "versoin     = 2.0",
             "Tags        = story", "api         = banana", "target      = default", "mode        = night",
             "[screen.Bad Id]", "layer       = art/gone.dds", "opacity     = 7", "[track.t1]",
             "file        = music/t.wav", "[commands]", "od_a        = od_cmd.status", "[command.od_a]",
             "cheat       = maybe", "[start_kit]", "money       = 10", "wpn_knife   = 0"]
    qa.write(path, "\r\n".join(lines) + "\r\n")
    code, problems, err = found()
    check(code == 1, "J a mod.ltx with the values the page lists and the check let pass is exit 1", (code, err[:200]))
    for row, name, says in LEFT:
        want = lines.index(row) + 1
        hit = [p for p in problems if p.get("code") == name and p.get("line") == want]
        check(len(hit) >= 1 and hit[0].get("file") == "mod.ltx" and says in hit[0].get("text", ""),
              "J check names %s at line %d of mod.ltx" % (name, want), json.dumps(problems)[:800])
    severity = {p.get("code"): p.get("severity") for p in problems}
    check(severity.get("mod.version") == "error" and severity.get("mod.target_default") == "error"
          and severity.get("mod.section_id") == "warning" and severity.get("mod.key") == "warning",
          "J the pair and the empty version are errors, the id and the key nobody reads are warnings", severity)

    # the pairs of target and mode, both ways: (the lines after [module] id, the code or None for clean, what it names)
    base = "[module]\r\nid          = qamanifest\r\n"
    pairs = [
        ("target      = default\r\n", None, ""),
        ("target      = existing\r\nmode        = night\r\n", None, ""),
        ("target      = new\r\n[provides_mode]\r\nid          = night\r\n", None, ""),
        ("target      = any\r\nmode        = *\r\n", None, ""),
        ("target      = any\r\nmode        = night\r\n", "mod.target_any", "target = any with mode = night"),
        ("target      = any\r\n", "mod.target_any", "target = any with no mode"),
        ("target      = existing\r\n", "mod.target_existing", "target = existing with no mode"),
        ("target      = new\r\n", "mod.target_new", "target = new with no mode"),
    ]
    for text, name, says in pairs:
        qa.write(path, base + text)
        code, problems, err = found()
        if name is None:
            check(code == 0 and not problems, "J the pair %r is clean" % text.split("\r\n")[0:2], (code, json.dumps(problems)[:300]))
            continue
        hit = [p for p in problems if p.get("code") == name and p.get("line") == 3]
        check(code == 1 and len(problems) == 1 and len(hit) == 1 and says in hit[0].get("text", "")
              and "no target game mode" not in hit[0].get("text", "") and hit[0].get("fix"),
              "J %s names the pair at its target line with its fix" % name, (code, json.dumps(problems)[:400]))

    # mod.set refuses the same pairs with the same sentence and writes nothing
    qa.write(path, base + "target      = default\r\n")
    before = qa.read(path)
    code, out, err = cli("run", "mod.set", 'set={"module":{"mode":"*"}}', "-project", project, home=home)
    check(code == 1 and "target = default with mode = *" in err and "goes with no mode" in err and qa.read(path) == before,
          "J mod.set refuses a mode that does not go with the target and writes nothing", (code, err[:400]))
    code, out, err = cli("run", "mod.set", 'set={"module":{"target":"any"}}', "-project", project, home=home)
    check(code == 1 and "target = any with no mode" in err and qa.read(path) == before,
          "J mod.set refuses target = any alone with the check's sentence", (code, err[:400]))
    code, out, err = cli("run", "mod.set", 'set={"module":{"mode":"night"}}', "-project", project, "-json", home=home)
    answer = (jsons(out) or [{}])[0]
    check(code == 0 and answer.get("ok") and answer.get("manifest", {}).get("target") == "existing",
          "J mod.set of a mode alone still sets the target from it", (code, out[:300], err[:200]))
    code, problems, err = found()
    check(code == 0 and not problems, "J and that manifest is clean", (code, json.dumps(problems)[:300]))
    code, out, err = cli("run", "mod.set", 'set={"module":{"target":"default"}}', "-project", project, home=home)
    check(code == 1 and "target = default with mode = night" in err,
          "J mod.set refuses target = default while a mode is set", (code, err[:300]))
    code, out, err = cli("run", "mod.set", 'set={"module":{"target":"default","mode":""}}', "-project", project,
                         "-json", home=home)
    answer = (jsons(out) or [{}])[0]
    check(code == 0 and answer.get("ok") and answer.get("manifest", {}).get("target") == "default",
          "J target = default with the mode taken away goes through", (code, out[:300], err[:200]))

    qa.write(path, good)
    code, problems, err = found()
    check(code == 0 and not problems, "J the manifest put right is clean again", (code, json.dumps(problems)[:300]))


def editors_scratch(work: str, home: str, project: str) -> None:
    """What a crashed packaging leaves under temp\\ is no quest of the module: a check of the project does not read
    it, a check asked for that folder does."""
    qa.write(os.path.join(project, "quests", "qa_assets.nqasset"), QUEST)
    stage = "temp/release_stage_1/modules/qamanifest/quests"
    qa.write(os.path.join(project, stage, "qa_assets.nqasset"), QUEST)
    qa.write(os.path.join(project, "releases", "1.0.0", "qa_assets.nqasset"), QUEST)
    code, out, err = cli("check", "-project", project, "-json", home=home)
    problems = (jsons(out) or [{}])[0].get("problems", [])
    check(code == 0 and not problems, "J a quest in temp\\release_stage_1 and one in releases\\ fail no check of the project",
          (code, json.dumps(problems)[:400]))
    code, out, err = cli("check", stage, "-project", project, "-json", home=home)
    check((jsons(out) or [{}])[0].get("files") == 1, "J a check asked for that folder reads it", (code, out[:300], err[:200]))
    code, out, err = cli("check", "quests", "-project", project, home=home)
    check(code == 0, "J the project's own quest passes", (code, out[:300], err[:200]))


def run(ctx: qa.Run) -> None:
    if not os.path.isfile(CLI):
        raise RuntimeError("build the editor's twin first: " + CLI)
    work = tempfile.mkdtemp(prefix="gzcli_", dir=ctx.work)
    steps = {"A": lambda: phase_a(work), "B": lambda: phase_b(work), "C": lambda: phase_c(work),
             "D": lambda: phase_d(work, ctx.project), "E": lambda: phase_e(work, ctx.project),
             "F": lambda: phase_f(work), "G": lambda: phase_g(work), "H": lambda: phase_h(work, ctx.project),
             "I": lambda: phase_i(work), "J": lambda: phase_j(work), "U": lambda: phase_u(work)}
    qa.run_phases(steps, ctx.only)


def main() -> int:
    if len(sys.argv) == 6 and sys.argv[1] == "--console-inner":
        return console_inner(sys.argv[2], sys.argv[3], int(sys.argv[4]), sys.argv[5])
    return qa.suite("COMMAND LINE", run, doc=__doc__, phases="ABCDEFGHIJ")


if __name__ == "__main__":
    sys.exit(main())
