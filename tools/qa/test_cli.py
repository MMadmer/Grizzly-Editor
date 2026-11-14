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


def run(ctx: qa.Run) -> None:
    if not os.path.isfile(CLI):
        raise RuntimeError("build the editor's twin first: " + CLI)
    work = tempfile.mkdtemp(prefix="gzcli_", dir=ctx.work)
    steps = {"A": lambda: phase_a(work), "B": lambda: phase_b(work), "C": lambda: phase_c(work),
             "D": lambda: phase_d(work, ctx.project), "E": lambda: phase_e(work, ctx.project),
             "F": lambda: phase_f(work), "G": lambda: phase_g(work), "H": lambda: phase_h(work, ctx.project),
             "I": lambda: phase_i(work)}
    qa.run_phases(steps, ctx.only)


def main() -> int:
    if len(sys.argv) == 6 and sys.argv[1] == "--console-inner":
        return console_inner(sys.argv[2], sys.argv[3], int(sys.argv[4]), sys.argv[5])
    return qa.suite("COMMAND LINE", run, doc=__doc__, phases="ABCDEFGHI")


if __name__ == "__main__":
    sys.exit(main())
