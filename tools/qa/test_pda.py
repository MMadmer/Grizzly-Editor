#!/usr/bin/env python3
"""End-to-end check of PDA interfaces (EditorPda: the .pdaasset, its preview tab, the
manifest pickup, the build gate) over MCP, on a THROW-AWAY COPY of a project.

What it proves:
  A  a new layout from the template becomes the module's ([pda] layout in mod.ltx), and
     mod_manifest reports it
  B  the game's reader judges a file: the courier fixture reads clean, broken files come
     back with their codes (E3xx), a file that does not parse is refused with its line
  C  pda_use picks, clears and refuses (a missing file, a file the build leaves out)
  D  the preview plays the layout like the game: tabs, a row that fills vars and opens an
     overlay, a button that sets a var and switches pages, the keys, a stock section, the
     16:9 device, a click by canvas point - and the tab follows a write to the file
  E  the build ships the layout with [pda] in the module's mod.ltx, and refuses one with
     an error
  F  deleting the used layout clears [pda]; Create > PDA Interface makes NewPda
  G  the editor's bundled reader and sheet are the game's, byte for byte

    python tools/qa/test_pda.py --project D:\\XFinedProjects\\Test [--shots <folder>]
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
GAME_REPO = r"D:\Games\Dead Air\DeadAir-x64"
MCP = ("127.0.0.1", 28016)
FIXTURE = os.path.join(HERE, "fixtures", "courier.pdaasset")

results: list[tuple[bool, str]] = []


def check(ok: bool, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + detail) if detail and not ok else ""), flush=True)
    return bool(ok)


def mcp(cmd: str, timeout: float = 300.0, **extra) -> dict:
    payload = {"cmd": cmd}
    payload.update(extra)
    with socket.create_connection(MCP, timeout=10) as s:
        s.settimeout(timeout)
        s.sendall((json.dumps(payload) + "\n").encode())
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = s.recv(1 << 20)
            if not chunk:
                break
            buf += chunk
    return json.loads(buf.decode("utf-8"))


class Editor:
    def __init__(self) -> None:
        self.proc: subprocess.Popen | None = None

    def start(self, project: str) -> None:
        exe = os.path.join(REPO, "Bin", "x64", "Release", "LevelEditor.exe")
        if not os.path.isfile(exe):
            raise RuntimeError("build the editor first: " + exe)
        startup = subprocess.STARTUPINFO()
        startup.dwFlags = subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 7  # SW_SHOWMINNOACTIVE
        self.proc = subprocess.Popen([exe, "-nodlg", "-project", project],
                                     cwd=os.path.dirname(exe), startupinfo=startup)
        deadline = time.monotonic() + 600
        while time.monotonic() < deadline:
            try:
                if mcp("mod_manifest", timeout=30).get("ok"):
                    return
            except OSError:
                pass
            if self.proc.poll() is not None:
                raise RuntimeError("the editor exited during startup")
            time.sleep(1)
        raise RuntimeError("the editor never opened the project")

    def close(self) -> None:
        try:
            mcp("window_control", action="close")
        except OSError:
            pass
        if self.proc:
            try:
                self.proc.wait(timeout=120)
            except subprocess.TimeoutExpired:
                print("! the editor did not close on its own", flush=True)


def read(path: str) -> str:
    with open(path, "rb") as f:
        return f.read().decode("utf-8", "replace")


def sha(path: str) -> str:
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def codes(answer: dict) -> list[str]:
    return [p.get("code", "") for p in answer.get("problems", [])]


def visible(state: dict, **match) -> list[dict]:
    out = []
    for it in state.get("visible", []):
        if all(it.get(k) == v for k, v in match.items()):
            out.append(it)
    return out


def shot(folder: str | None, name: str) -> None:
    if not folder:
        return
    pic = mcp("screenshot_editor")
    if pic.get("png_base64"):
        os.makedirs(folder, exist_ok=True)
        path = os.path.join(folder, name + ".png")
        with open(path, "wb") as f:
            f.write(base64.b64decode(pic["png_base64"]))
        print("screenshot: " + path, flush=True)


def run(project: str, shots: str | None) -> None:
    ed = Editor()
    out_dir = tempfile.mkdtemp(prefix="xfined_pda_")
    try:
        ed.start(project)
        manifest = mcp("mod_manifest")
        module = manifest.get("id", "")
        check(bool(module), "A the project has a module id", json.dumps(manifest)[:200])
        mod_ltx = os.path.join(project, "mod.ltx")
        fixture = read(FIXTURE)

        # ------------------------------------------------------------ A: a new layout
        l = mcp("pda_list")
        check(l.get("ok") and l.get("used") == "" and l.get("layouts") == [], "A a fresh project has no PDA layout",
              json.dumps(l)[:300])
        w = mcp("pda_write", id="qa_app", title="QA App")
        check(w.get("ok") and w.get("created") and w.get("used"), "A the template is written and used at once",
              json.dumps(w)[:400])
        check(w.get("errors") == 0, "A and it reads clean", json.dumps(w.get("problems"))[:300])
        check(os.path.isfile(os.path.join(project, "pda", "qa_app.pdaasset")), "A pda/qa_app.pdaasset exists")
        text = read(mod_ltx)
        check("[pda]" in text and "pda\\qa_app.pdaasset" in text, "A mod.ltx names it in [pda] layout", text[-300:])
        check(mcp("mod_manifest").get("pda_layout") == "pda\\qa_app.pdaasset", "A mod_manifest reports it")
        pages = [p["id"] for p in w.get("layout", {}).get("pages", [])]
        check(pages == ["home", "map", "journal"], "A the template has home, map and a stock journal", str(pages))

        # ------------------------------------------------------------ B: the reader
        w = mcp("pda_write", path="pda/courier.pdaasset", text=fixture, use=False)
        check(w.get("ok") and w.get("errors") == 0 and w.get("warnings") == 0 and not w.get("use_changed"),
              "B the courier fixture reads clean and is not used by itself", json.dumps(w.get("problems"))[:400])
        ids = sorted(i["id"] for i in w.get("layout", {}).get("ids", []))
        check(ids == sorted(["tab_orders", "tab_map", "tab_journal", "orders", "card", "done", "details", "take", "back"]),
              "B every addressable id is listed", str(ids))
        g = mcp("pda_get", path="pda/courier.pdaasset")
        check(g.get("ok") and g.get("text") == fixture and g.get("used") is False, "B pda_get gives the file back", json.dumps(g)[:200])
        check("xms_pda_layout.script" in g.get("reader", ""), "B and names the reader", g.get("reader", ""))
        broken = fixture.replace('kind = "rect", x = 0, y = 0, w = 960', 'kind = "circle", x = 0, y = 0, w = 960', 1)
        v = mcp("pda_validate", text=broken)
        check(v.get("ok") and v.get("valid") is False and "E305" in codes(v), "B an unknown widget kind is E305", json.dumps(v)[:300])
        v = mcp("pda_validate", text=fixture.replace('page = "map" }', 'page = "nowhere" }', 1))
        check("E308" in codes(v), "B a tab to a page that does not exist is E308", str(codes(v)))
        v = mcp("pda_validate", text=fixture.replace('"ui_xms_icon_plus"', '"ui_qa_no_such_texture"', 1))
        check("W320" in codes(v), "B a texture neither the game nor the project has is W320", str(codes(v)))
        bad = mcp("pda_write", path="pda/broken.pdaasset", text="return { pda = 1,\n  id = \n}")
        check(not bad.get("ok") and bad.get("line", 0) >= 2 and not os.path.exists(os.path.join(project, "pda", "broken.pdaasset")),
              "B a file that does not parse is refused with its line", json.dumps(bad)[:300])
        code = mcp("pda_write", path="pda/code.pdaasset", text="os.execute('calc') return { pda = 1 }")
        check(not code.get("ok"), "B code outside strings is refused", json.dumps(code)[:300])

        # ------------------------------------------------------------ C: the pickup
        u = mcp("pda_use", path="pda/courier.pdaasset")
        check(u.get("ok") and u.get("used") == "pda/courier.pdaasset", "C pda_use picks another layout", json.dumps(u))
        check("pda\\courier.pdaasset" in read(mod_ltx), "C mod.ltx follows")
        u = mcp("pda_use", path="pda/none.pdaasset")
        check(not u.get("ok"), "C a missing file is refused", json.dumps(u))
        os.makedirs(os.path.join(project, "drafts"), exist_ok=True)
        shutil.copyfile(FIXTURE, os.path.join(project, "drafts", "draft.pdaasset"))
        mcp("mod_exclude", path="drafts", excluded=True)
        u = mcp("pda_use", path="drafts/draft.pdaasset")
        check(not u.get("ok") and "left out" in u.get("error", ""), "C a file the build leaves out is refused", json.dumps(u))
        mcp("mod_exclude", path="drafts", excluded=False)
        listed = [x["path"] for x in mcp("pda_list").get("layouts", [])]
        check(any(p.endswith("courier.pdaasset") for p in listed) and any(p.endswith("qa_app.pdaasset") for p in listed),
              "C pda_list shows both layouts", str(listed))

        # ------------------------------------------------------------ D: the preview
        p = mcp("pda_preview", action="open", path="pda/courier.pdaasset")
        st = p.get("state", {})
        check(p.get("ok") and p.get("open") and st.get("page") == "orders", "D the preview opens on the start page",
              json.dumps(p)[:300])
        time.sleep(1.5)
        shot(shots, "pda_orders")
        tabs = visible(st, id="tab_orders")
        check(tabs and tabs[0].get("selected") is True, "D the tab of the page is selected", json.dumps(tabs))
        rows = visible(st, kind="row")
        check(len(rows) == 2, "D the list shows its two rows", json.dumps(rows)[:300])
        check(visible(st, id="details") == [], "D the overlay starts hidden")
        p = mcp("pda_preview", action="click", n=rows[0]["n"])
        st = p.get("state", {})
        check(st.get("vars", {}).get("picked") == "Тушёнка" and visible(st, id="details"),
              "D a row fills its var and opens the overlay", json.dumps(st.get("vars"), ensure_ascii=False))
        check(visible(st, kind="row", selected=True), "D and becomes the list's selection")
        time.sleep(1.0)
        shot(shots, "pda_details")
        p = mcp("pda_preview", action="click", id="orders", row=2)
        check(p.get("state", {}).get("vars", {}).get("picked") == "Batteries", "D a list row by its number",
              json.dumps(p.get("state", {}).get("vars"), ensure_ascii=False))
        bad = mcp("pda_preview", action="click", id="orders", row=9)
        check(not bad.get("ok"), "D a row the list does not have is refused", json.dumps(bad)[:200])
        mcp("pda_preview", action="click", n=rows[0]["n"])
        p = mcp("pda_preview", action="click", id="take")
        st = p.get("state", {})
        check(st.get("page") == "map" and st.get("vars", {}).get("current") == "Тушёнка" and not visible(st, id="details"),
              "D Take: var, overlay hidden, the map page", json.dumps(st, ensure_ascii=False)[:300])
        time.sleep(1.0)
        shot(shots, "pda_map")
        p = mcp("pda_preview", action="click", id="done")
        st = p.get("state", {})
        check(st.get("vars", {}).get("current") == "Нет заказа" and "signal:courier_done" in st.get("log", []),
              "D Done: the signal goes to the log, the var resets", json.dumps(st.get("log"), ensure_ascii=False))
        orders_tab = visible(st, id="tab_orders")[0]
        p = mcp("pda_preview", action="click", x=orders_tab["x"] + orders_tab["w"] / 2, y=orders_tab["y"] + orders_tab["h"] / 2)
        check(p.get("state", {}).get("page") == "orders", "D a click by canvas point hits the tab", json.dumps(p)[:200])
        p = mcp("pda_preview", action="key", key="map")
        check(p.get("state", {}).get("page") == "map", "D the map key opens the map page")
        p = mcp("pda_preview", action="key", key="contacts")
        st = p.get("state", {})
        check(st.get("stock") == "eptContacts", "D the contacts key shows the stock contacts", json.dumps(st)[:200])
        check(visible(st, id="tab_journal") == [] and visible(st, id="back"),
              "D a stock section hides the chrome and keeps the overlay", json.dumps(st.get("visible"))[:300])
        p = mcp("pda_preview", action="click", id="back")
        check(p.get("state", {}).get("stock") == "" and p.get("state", {}).get("page") == "map",
              "D the overlay's back leaves the stock section", json.dumps(p.get("state"))[:200])
        p = mcp("pda_preview", action="click", id="tab_journal")
        st = p.get("state", {})
        check(st.get("page") == "journal" and st.get("stock") == "eptTasks", "D a stock page shows the stock section")
        time.sleep(1.0)
        shot(shots, "pda_journal")
        p = mcp("pda_preview", action="aspect", aspect="16:9")
        check(p.get("state", {}).get("aspect") == "16:9", "D the 16:9 device")
        time.sleep(1.0)
        shot(shots, "pda_wide")
        mcp("pda_preview", action="aspect", aspect="4:3")
        p = mcp("pda_preview", action="reset")
        st = p.get("state", {})
        check(st.get("page") == "orders" and st.get("vars", {}).get("current") == "Нет заказа", "D reset: the start page and the declared vars")
        # the tab follows the file
        mcp("pda_write", path="pda/courier.pdaasset", text=fixture.replace('title = "Orders"', 'title = "Deliveries"', 1), use=False)
        time.sleep(1.5)
        p = mcp("pda_preview", action="state")
        check(p.get("state", {}).get("read") is True, "D the tab re-reads a file written behind it", json.dumps(p)[:200])
        g = mcp("pda_get", path="pda/courier.pdaasset")
        check(any(pg.get("title") == "Deliveries" for pg in g.get("layout", {}).get("pages", [])), "D and the change is there")
        # the map slot lists what it shows, in canvas units: the player and the markers inside it
        p = mcp("pda_preview", action="page", page="map")
        mp = p.get("state", {}).get("map", {})
        b = mp.get("bound") or [0, 0, 100, 100]
        cx, cz = (b[0] + b[2]) / 2, (b[1] + b[3]) / 2
        marked = fixture.replace('preview = { level = "l01_escape" },',
                                 'preview = { level = "l01_escape", actor = { x = %g, z = %g }, markers = {'
                                 ' { x = %g, z = %g, hint = "Parcel" }, { x = %g, z = %g, hint = "Far" } } },'
                                 % (cx, cz, cx, cz + 20, b[0] + 1, b[1] + 1), 1)
        check(marked != fixture, "D the fixture takes preview markers")
        mcp("pda_write", path="pda/courier.pdaasset", text=marked, use=False)
        time.sleep(1.5)
        mcp("pda_preview", action="reset")
        p = mcp("pda_preview", action="page", page="map")
        spots = p.get("state", {}).get("map", {}).get("spots", [])
        me = [s for s in spots if s.get("what") == "player"]
        parcel = [s for s in spots if s.get("hint") == "Parcel"]
        check(len(me) == 1 and len(parcel) == 1 and parcel[0]["y"] < me[0]["y"],
              "D the map slot lists the player and a marker north of them", json.dumps(spots))
        check(not [s for s in spots if s.get("hint") == "Far"], "D a marker outside the slot is not listed", json.dumps(spots))
        mcp("pda_write", path="pda/courier.pdaasset", text=fixture, use=False)
        time.sleep(1.5)

        # ------------------------------------------------------------ E: the build
        ex = mcp("mod_export", target=out_dir)
        check(ex.get("ok"), "E the module builds with its PDA layout", json.dumps(ex)[:300])
        root = os.path.join(out_dir, "modules", module)
        check(os.path.isfile(os.path.join(root, "pda", "courier.pdaasset")), "E the layout ships as it is")
        shipped = read(os.path.join(root, "mod.ltx")) if os.path.isfile(os.path.join(root, "mod.ltx")) else ""
        check("[pda]" in shipped and "courier.pdaasset" in shipped, "E the module's mod.ltx carries [pda]", shipped[-200:])
        mcp("pda_write", path="pda/courier.pdaasset", text=broken, use=False)
        ex = mcp("mod_export", target=out_dir)
        check(not ex.get("ok") and "E305" in ex.get("error", ""), "E a used layout with an error stops the build",
              json.dumps(ex)[:300])
        mcp("pda_write", path="pda/courier.pdaasset", text=fixture, use=False)

        # ------------------------------------------------------------ F: delete and create
        d = mcp("pda_delete", path="pda/courier.pdaasset")
        check(d.get("ok") and d.get("use_cleared") and not os.path.exists(os.path.join(project, "pda", "courier.pdaasset")),
              "F deleting the used layout deletes the file and clears [pda]", json.dumps(d))
        check("[pda]" not in read(mod_ltx), "F mod.ltx has no [pda] any more")
        c = mcp("content_create", kind="pda")
        made = c.get("made", "")
        check(c.get("ok") and made.lower().endswith("newpda.pdaasset"), "F Create > PDA Interface makes NewPda", json.dumps(c))

        # ------------------------------------------------------------ G: the bundled copies
        pairs = [
            ("gamedata/configs/pda/xms_pda_layout.script", "packaging/dead-air-x64/compatibility/gamedata/scripts/xms_pda_layout.script"),
            ("gamedata/configs/pda/ui_xms_pda.dds", "packaging/dead-air-x64/compatibility/gamedata/textures/ui/ui_xms_pda.dds"),
            ("gamedata/configs/pda/ui_xms_pda.xml", "packaging/dead-air-x64/compatibility/gamedata/configs/ui/textures_descr/ui_xms_pda.xml"),
        ]
        if os.path.isdir(GAME_REPO):
            for mine, theirs in pairs:
                a, b = os.path.join(REPO, mine), os.path.join(GAME_REPO, theirs)
                check(os.path.isfile(a) and os.path.isfile(b) and sha(a) == sha(b), "G the bundled " + os.path.basename(mine) + " is the game's")
            fx = os.path.join(GAME_REPO, "tools", "nq", "fixtures", "pda", "courier.pdaasset")
            check(os.path.isfile(fx) and sha(fx) == sha(FIXTURE), "G the fixture is the game repo's")
    finally:
        ed.close()
        shutil.rmtree(out_dir, ignore_errors=True)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", required=True, help="a project to COPY and run against")
    ap.add_argument("--shots", help="a folder for the preview's screenshots")
    ap.add_argument("--keep", action="store_true", help="keep the throw-away copy")
    args = ap.parse_args()

    src = os.path.abspath(args.project)
    if not os.path.isdir(src):
        print("no such project: " + src)
        return 2

    work = tempfile.mkdtemp(prefix="xfined_pdaproj_")
    copy = os.path.join(work, "QaPda")
    shutil.copytree(src, copy, ignore=shutil.ignore_patterns(".git"))
    print("running against a copy: " + copy, flush=True)
    # a project that already names a layout starts from none: the test makes its own
    for folder, _dirs, files in os.walk(copy):
        for name in files:
            if name.lower().endswith(".pdaasset"):
                os.remove(os.path.join(folder, name))
    ml = os.path.join(copy, "mod.ltx")
    if os.path.isfile(ml):
        lines, skip = [], False
        for line in read(ml).splitlines():
            if line.strip().startswith("["):
                skip = line.strip().lower() == "[pda]"
            if not skip:
                lines.append(line)
        with open(ml, "w", encoding="utf-8", newline="\r\n") as f:
            f.write("\n".join(lines) + "\n")
    pl = os.path.join(copy, "project.ltx")
    if os.path.isfile(pl):
        text = read(pl)
        low = text.lower()
        at = low.find(src.lower())
        while at >= 0:
            text = text[:at] + copy + text[at + len(src):]
            low = text.lower()
            at = low.find(src.lower(), at + len(copy))
        with open(pl, "w", encoding="utf-8", newline="") as f:
            f.write(text)

    ini = os.path.join(REPO, "_projects.ini")
    ini_backup = None
    if os.path.isfile(ini):
        with open(ini, "rb") as f:
            ini_backup = f.read()

    try:
        run(copy, args.shots)
    except Exception as exc:  # noqa: BLE001 - the report is the point
        check(False, "the run finished", repr(exc))
    finally:
        if ini_backup is not None:
            with open(ini, "wb") as f:
                f.write(ini_backup)
        if not args.keep:
            shutil.rmtree(work, ignore_errors=True)

    ok = sum(1 for good, _ in results if good)
    print(f"PDA: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
