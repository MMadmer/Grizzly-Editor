#!/usr/bin/env python3
"""Feedback to the editor's author (docs/REPORTS.md), against the report hub of tools/qa/github_mock.

An editor with a window on a home of its own, on the Start page, under names that are this suite's own
(USERNAME, COMPUTERNAME, USERDOMAIN - the profile stays the machine's: the graphics driver keeps its cache
under it), finds a crash planted in its log folder - its report, a dump with the
person's name in its module paths, stack and handles, and the crashed run's log - and sends it with a bug report.
What the mock received is read entry by entry for every trace of the machine. Nothing reaches the real hub: a QA run
speaks to the mock alone, with the QA key.

    python tools/qa/test_feedback.py [--only A,B,...] [--shots <dir>]

  A  feedback.status: the planted crash is offered, its log bound; the refusals of feedback.send
  B  a dry run: report.json in the hub's keys, the package's entries, nothing sent
  C  a bug report with the crash: the hub's form, the package without a trace of the person or the computer, the dump
     made anonymous; the crash offered no more
  D  a suggestion without diagnostics is its words alone; a busy hub is tried again, a refusal is a sentence
  E  the window: the crash's toast, Help > Send Feedback..., the dialog with its card and buttons; dismiss
"""
from __future__ import annotations

import base64
import io
import json
import os
import struct
import sys
import time
import zipfile

import qa
from qa import check, cmd

PERSON = "QaPersonName"
MACHINE = "QA-MACHINE-7Z"
MAIL = "qa.person@example.org"
SECRET = "abcdef1234567890qa"
ADDRESS = "10.1.2.3"
PRIVATE_FOLDER = "QaPrivateFolder"
TRACES = [PERSON, MACHINE, MAIL, SECRET, ADDRESS, PRIVATE_FOLDER]


def utf16(s: str) -> bytes:
    return s.encode("utf-16-le")


def dump_bytes() -> bytes:
    """A minidump of one thread with a stack, one module whose name and PDB path carry the person, and a handle
    stream naming a file of theirs: what the editor's sanitizer must clean (minidump.rs)."""
    d = bytearray(32)
    d[0:4] = b"MDMP"
    stack = len(d)
    d += ("stack bytes of C:\\Users\\%s\\Documents" % PERSON).encode()
    stack_len = len(d) - stack
    name = len(d)
    path = utf16("C:\\Users\\%s\\AppData\\Local\\Programs\\Grizzly Editor\\Grizzly.exe" % PERSON)
    d += struct.pack("<I", len(path)) + path + b"\0\0"
    cv = len(d)
    d += b"RSDS" + bytes(20) + ("D:\\Work\\%s\\target\\grizzly.pdb\0" % PERSON).encode()
    cv_len = len(d) - cv
    threads = len(d)
    t = bytearray(48)
    t[24:32] = struct.pack("<Q", 0x1000)
    t[32:40] = struct.pack("<II", stack_len, stack)
    d += struct.pack("<I", 1) + t
    modules = len(d)
    m = bytearray(108)
    m[20:24] = struct.pack("<I", name)
    m[76:84] = struct.pack("<II", cv_len, cv)
    d += struct.pack("<I", 1) + m
    handles = len(d)
    handle_text = ("\\Device\\HarddiskVolume3\\Users\\%s\\secret.txt" % PERSON).encode()
    d += handle_text
    directory = len(d)
    for kind, rva, size in ((3, threads, 52), (4, modules, 112), (12, handles, len(handle_text))):
        d += struct.pack("<III", kind, size, rva)
    d[8:16] = struct.pack("<II", 3, directory)
    return bytes(d)


def plant_crashes(logs: str) -> tuple[str, str]:
    """A crash that ended the editor 30 seconds ago (with its dump and the run's log) and a newer one it caught."""
    os.makedirs(logs, exist_ok=True)
    now = int(time.time())
    fatal = "grizzly_crash_%d_4242" % (now - 30)
    caught = "grizzly_crash_%d_4343" % (now - 10)
    qa.write(os.path.join(logs, fatal + ".txt"), "\n".join([
        "Grizzly Editor stopped.",
        "version: 0.7.0",
        "kind: fatal",
        "thread: main",
        "exception: 0xC0000005 EXCEPTION_ACCESS_VIOLATION",
        "module: Grizzly.exe",
        "module_rva: 0x1234",
        "at: C:\\Users\\%s\\src\\scene.rs:12" % PERSON,
        "message: opened D:\\%s\\Mods\\a.ltx token=%s mail %s from %s on %s" % (PRIVATE_FOLDER, SECRET, MAIL, ADDRESS, MACHINE),
        "",
        "frames:",
        "  Grizzly.exe+0x1234",
        "  ntdll.dll+0x5678",
        "",
    ]))
    with open(os.path.join(logs, fatal + ".dmp"), "wb") as f:
        f.write(dump_bytes())
    qa.write(os.path.join(logs, caught + ".txt"), "kind: caught\nthread: io-3\nmessage: index out of bounds\n")
    # the crashed run's log, as the next start sets it aside: the editor names its log after USERNAME
    log = os.path.join(logs, "grizzly_%s.log" % PERSON.lower())
    # the run's first line names its process, which the crash's name names too: that is how the next start
    # finds this log; a log of another process beside it must not be taken
    qa.write(os.path.join(logs, "grizzly_cmd_%s.previous.log" % PERSON.lower()),
             "Grizzly Editor 0.7.0 starting, process 99\nanother program's log\n")
    qa.write(log, "Grizzly Editor 0.7.0 starting, process 4242\n" + "".join("line %d of the run that crashed: user %s on %s, %s, password: %s, D:\\%s\\x.ltx\n"
                          % (i, PERSON, MACHINE, MAIL, SECRET, PRIVATE_FOLDER) for i in range(50)))
    os.utime(log, (now - 25, now - 25))
    return fatal, caught


def traces_in(data: bytes) -> list[str]:
    """Every trace of the person or the machine in `data`, in UTF-8 / ANSI and in UTF-16, any case."""
    low = data.lower()
    found = []
    for t in TRACES + [os.environ.get("USERNAME", "")] * (len(os.environ.get("USERNAME", "")) >= 3):
        if t.lower().encode() in low or utf16(t.lower()) in low or utf16(t) in data:
            found.append(t)
    return found


def reports(mock: qa.Mock) -> list[dict]:
    return mock.get("/__reports")["reports"]


def run(ctx: qa.Run) -> None:
    only = set(ctx.only.split(",")) if ctx.only else None
    home = os.path.join(ctx.work, "home")
    fatal, caught = plant_crashes(os.path.join(home, "logs"))
    mock = qa.Mock(0)
    env = {
        "GRIZZLY_REPORT_URL": mock.base + "/api/v1/reports",
        "GRIZZLY_REPORT_TOKEN": "qa-report-token",
        "USERNAME": PERSON,
        "COMPUTERNAME": MACHINE,
        "USERDOMAIN": MACHINE,
    }
    ed = qa.Editor(home_folder=home, env=env)
    qa.mark("editor start")
    ed.start()
    try:
        cmd("ui.list")
        st = qa.until(lambda: (s := cmd("feedback.status")).get("offer") and s, 15) or cmd("feedback.status")
        if only is None or "A" in only:
            offer = st.get("offer") or {}
            check(st.get("available") is True, "A a QA run can send (to the mock)", st)
            check(offer.get("name") == fatal and offer.get("fatal") and offer.get("dump"),
                  "A the crash that ended the editor is offered, with its dump", offer)
            check(offer.get("log") is True, "A the crashed run's log the start set aside is bound to it", offer)
            names = [c.get("name") for c in st.get("crashes", [])]
            check(names[:2] == [caught, fatal], "A the folder's crashes are listed newest first", names)
            r = cmd("feedback.send", title="abc", text="x" * 30)
            check(not r.get("ok") and "title" in r.get("error", ""), "A a title under 5 characters is refused", r)
            r = cmd("feedback.send", title="A real title", text="short")
            check(not r.get("ok") and "20 to 10000" in r.get("error", ""), "A a text under 20 characters is refused", r)
            r = cmd("feedback.send", title="A real title", text="x" * 30, crash="grizzly_crash_1_1")
            check(r.get("code") == "not_found", "A a crash the folder does not hold is named as such", r)
            r = cmd("feedback.send", title="A real title", text="x" * 30, crash=fatal, diagnostics=False)
            check(not r.get("ok") and "diagnostics" in r.get("error", ""), "A a crash goes with its diagnostics", r)
            r = cmd("feedback.send", title="A real title", text="x" * 30, color="red")
            check(not r.get("ok"), "A an unknown parameter is refused", r)
            # the dialog while the crash is offered: its card, ticked
            ed.cmd("ui.dialog", action="open", id="feedback")
            qa.until(lambda: [d for d in ed.cmd("ui.list").get("dialogs", []) if d.get("id") == "feedback"], 10)
            labels = [c.get("label") for c in ed.cmd("ui.audit").get("controls", [])
                      if c.get("window") == "Send Feedback###dlg.feedback"]
            check("Send the Crash with This Report" in labels and "Don't Offer Again" in labels,
                  "A the dialog shows the offered crash on a card: Send the Crash with This Report, Don't Offer Again",
                  labels)
            qa.shot(ctx.shots, "feedback_dialog_crash")
            ed.cmd("ui.dialog", action="close")

        title, text = "The gizmo jumps", "Dragging the move gizmo of a light jumps it ten metres."
        if only is None or "B" in only:
            before = len(reports(mock))
            r = cmd("feedback.send", kind="bug", title=title, text=text, crash=fatal, dry_run=True)
            rep = r.get("report") or {}
            names = [f.get("name") for f in r.get("files", [])]
            check(r.get("ok") and r.get("sent") is False, "B a dry run makes the package and sends nothing", qa.short(r))
            check(rep.get("schema") == "grizzly-editor.report/1" and rep.get("type") == "crash" and rep.get("kind") == "bug",
                  "B report.json declares its schema, the type and the kind", rep)
            build = rep.get("build") or {}
            check(build.get("version") and len(build.get("build_id", "")) == 64 and build.get("edition") == "development",
                  "B the build: the version, the program's SHA-256, a development build", build)
            crash = rep.get("crash") or {}
            check(crash.get("exception_name") == "EXCEPTION_ACCESS_VIOLATION" and crash.get("module") == "Grizzly.exe"
                  and crash.get("module_rva") == "0x1234", "B the crash in the keys the hub reads", crash)
            check(all(n in names for n in ("report.json", "editor.json", "logs/editor.log", "crash/crash.txt",
                                           "crash/run.log", "crash/crash.dmp")),
                  "B the package holds the state, the log and the crash's three files", names)
            check(not traces_in(json.dumps(rep).encode()), "B report.json names nobody", traces_in(json.dumps(rep).encode()))
            r = cmd("feedback.send", kind="bug", title=title, text=text, crash="latest", dry_run=True)
            check(r.get("crash") == fatal, "B crash=latest is the crash that ended the editor, not the newer one it caught",
                  r.get("crash"))
            check(len(reports(mock)) == before, "B the hub received nothing")

        if only is None or "C" in only:
            r = cmd("feedback.send", kind="bug", title=title, text=text, crash=fatal)
            got = reports(mock)
            check(r.get("ok") and r.get("sent") and got, "C the bug report with the crash is sent", qa.short(r))
            if got:
                h = got[-1]
                f = h["fields"]
                check(f.get("product") == "grizzly-editor" and f.get("kind") == "bug" and f.get("title") == title
                      and f.get("description") == text and f.get("version") == build_version(),
                      "C the hub's form: product, kind, title, text and the editor's version", f)
                check(h.get("idempotency_key") == r.get("report_id") and h.get("file_name") == "grizzly-report-%s.zip" % r.get("report_id"),
                      "C the report's id is its Idempotency-Key and names the package", (h.get("idempotency_key"), h.get("file_name")))
                check(h.get("user_agent", "").startswith("Grizzly-Editor/"), "C the client says it is the editor", h.get("user_agent"))
                z = zipfile.ZipFile(io.BytesIO(base64.b64decode(h["zip_base64"])))
                leaks = {n: traces_in(z.read(n)) for n in z.namelist()}
                leaks = {n: t for n, t in leaks.items() if t}
                check(not leaks, "C no entry of the package names the person or the computer", leaks)
                dump = z.read("crash/crash.dmp")
                check(dump[:4] == b"MDMP" and utf16("Grizzly.exe") in dump and b"grizzly.pdb" in dump,
                      "C the dump keeps its modules by file name", dump[:4])
                head = json.loads(z.read("report.json"))
                check(head.get("report_id") == r.get("report_id") and head.get("type") == "crash",
                      "C report.json in the package is the report", head.get("report_id"))
                log = z.read("crash/run.log").decode("utf-8", "replace")
                check("line 49 of the run that crashed" in log and "<user>" in log and "<secret>" in log and "<email>" in log
                      and "another program's log" not in log,
                      "C the crashed run's log is there - its process's, its names and keys as markers", log[:300])
            st = cmd("feedback.status")
            handled = {c["name"]: c["handled"] for c in st.get("crashes", [])}
            check(st.get("offer") is None and handled.get(fatal) is True, "C the crash sent is offered no more", st.get("offer"))
            check((st.get("last") or {}).get("report_id") == r.get("report_id"), "C feedback.status says the last report sent")

        if only is None or "D" in only:
            r = cmd("feedback.send", kind="suggestion", title="Snap to the ground", text="A key that drops the selection to the ground under it.")
            got = reports(mock)
            check(r.get("ok") and got and got[-1]["fields"].get("kind") == "suggestion" and got[-1]["zip_base64"] == "",
                  "D a suggestion goes without diagnostics: its words alone", qa.short(r))
            tried = mock.get("/__reports")["tried"]
            mock.post("/__reports/fail?status=503&count=1")
            t0 = time.monotonic()
            r = cmd("feedback.send", kind="suggestion", title="Busy hub test", text="The hub is busy once and takes it the second time.")
            check(r.get("ok") and mock.get("/__reports")["tried"] == tried + 2 and time.monotonic() - t0 >= 2.5,
                  "D a busy hub is tried again after a pause", qa.short(r))
            mock.post("/__reports/fail?status=413&count=1")
            r = cmd("feedback.send", kind="bug", title="Too large test", text="The hub refuses this one as too large, once.")
            check(not r.get("ok") and r.get("code") == "failed" and "larger than the report hub takes" in r.get("error", ""),
                  "D a refusal of the hub is a sentence, not tried again", r)

        if only is None or "E" in only:
            notes = ed.cmd("ui.list").get("toast_notes") or []
            check(any(n.get("action") == "help.feedback" and "stopped unexpectedly" in n.get("text", "") for n in notes),
                  "E the start showed the crash's toast with Send Report...", notes)
            # the Start page has no menus: the action is the Help menu's row and the palette's
            keys = ed.cmd("app.keys")
            action = [a for a in keys.get("keys", []) if a.get("id") == "help.feedback"]
            check(action and action[0].get("title") == "Send Feedback..." and action[0].get("group") == "Help",
                  "E Send Feedback... is an action of Help", action or qa.short(keys))
            ran = ed.cmd("ui.palette", action="run", id="help.feedback")
            dialogs = qa.until(lambda: [d for d in ed.cmd("ui.list").get("dialogs", []) if d.get("id") == "feedback"], 10)
            check(ran.get("ok") and dialogs, "E Send Feedback... opens the dialog", qa.short(ran))
            labels = [c.get("label") for c in ed.cmd("ui.audit").get("controls", [])
                      if c.get("window") == "Send Feedback###dlg.feedback"]
            check(all(l in labels for l in ("Send", "Cancel", "Attach Diagnostics", "Bug Report", "Suggestion")),
                  "E the dialog: the kind, the title, the text, Attach Diagnostics, Send and Cancel", labels)
            pic = qa.shot(ctx.shots, "feedback_dialog")
            check((pic.get("modal") or {}).get("id") == "feedback", "E the dialog is the window's modal (picture)",
                  qa.short(pic))
            ed.cmd("ui.dialog", action="close")
            r = cmd("feedback.dismiss", crash=caught)
            handled = {c["name"]: c["handled"] for c in r.get("crashes", [])}
            check(r.get("ok") and handled.get(caught) is True, "E a crash dismissed is offered no more", r)
    finally:
        ed.close()
        mock.stop()


def build_version() -> str:
    with open(os.path.join(qa.REPO, "Cargo.toml"), encoding="utf-8") as f:
        for line in f:
            if line.startswith("version"):
                return line.split('"')[1]
    return ""


if __name__ == "__main__":
    sys.exit(qa.suite("test_feedback", run, doc=__doc__, project=False, phases="A-E"))
