#!/usr/bin/env python3
"""The Rust MCP bridge (`grizzly-mcp`, crate gz-mcp-bridge) over stdio, against an editor of its own
on a THROW-AWAY COPY of a project.

What it proves:
  A  initialize answers the server's instructions - the catalogue's, word for word, the safety rules
     with them - and serverInfo
  B  tools/list is crates/gz-mcp/tools.json exactly: every tool, description, schema and annotation,
     the bridge's own `cmd` key left out; the player- and withdrawal-safety paragraphs reach the client
     word for word in the tools that carry them
  C  a call reaches the editor and its answer comes back as text (grizzly_state, grizzly_ping)
  D  an argument a tool does not declare is refused by the bridge, naming it and what the tool takes;
     an unknown tool and an unknown method are JSON-RPC errors
  E  a *_discard tool sends discard: a quest made dirty refuses grizzly_ui_tab's plain close
     ('unsaved') and closes through grizzly_ui_tab_discard, its edit never reaching the file
  F  a screenshot comes back as an image content (PNG), not as base64 text
  Z  the editor closes through grizzly_window_close_discard and is gone

The editor runs windowed and minimized (a screenshot needs a window; -headless has none).

    python tools/qa/test_mcp_bridge.py [--project D:\\XFinedProjects\\Test]
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

import qa_target

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
TOOLS_JSON = os.path.join(REPO, "crates", "gz-mcp", "tools.json")
BRIDGE = os.path.join(REPO, "Intermediate", "cargo", "debug", "grizzly-mcp.exe")
MCP_PY = os.path.join(REPO, "tools", "mcp", "grizzly_mcp.py")

results: list[tuple[bool, str]] = []


def check(ok, text: str, detail: str = "") -> bool:
    results.append((bool(ok), text))
    print(("PASS " if ok else "FAIL ") + text + (("  <- " + detail) if detail and not ok else ""), flush=True)
    return bool(ok)


def answers(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.5) as s:
            s.sendall(b'{"cmd":"ping"}\n')
            return b'"ok"' in s.recv(4096)
    except OSError:
        return False


class Bridge:
    """grizzly-mcp on stdio: one JSON-RPC message per line."""

    def __init__(self, port: int) -> None:
        env = dict(os.environ, GRIZZLY_MCP_PORT=str(port))
        self.proc = subprocess.Popen([BRIDGE], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL, env=env)
        self.next_id = 1

    def send(self, method: str, params: dict | None = None, notify: bool = False) -> dict | None:
        msg = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            msg["params"] = params
        if not notify:
            msg["id"] = self.next_id
            self.next_id += 1
        assert self.proc.stdin and self.proc.stdout
        self.proc.stdin.write((json.dumps(msg, ensure_ascii=False) + "\n").encode("utf-8"))
        self.proc.stdin.flush()
        if notify:
            return None
        line = self.proc.stdout.readline()
        return json.loads(line.decode("utf-8")) if line else {}

    def call(self, name: str, **arguments) -> dict:
        return (self.send("tools/call", {"name": name, "arguments": arguments}) or {}).get("result", {})

    def close(self) -> None:
        try:
            if self.proc.stdin:
                self.proc.stdin.close()
            self.proc.wait(timeout=10)
        except (OSError, subprocess.TimeoutExpired):
            self.proc.kill()


def text_of(result: dict) -> str:
    return "".join(c.get("text", "") for c in result.get("content", []) if c.get("type") == "text")


def as_json(result: dict) -> dict:
    """The editor's answer the bridge passed on as text (Python's json.dumps spacing)."""
    try:
        return json.loads(text_of(result))
    except ValueError:
        return {}


def safety_paragraphs() -> tuple[str, str]:
    """PLAYER_SAFETY and WITHDRAW_SAFETY as tools/mcp/grizzly_mcp.py writes them."""
    import ast
    tree = ast.parse(open(MCP_PY, encoding="utf-8").read())
    found = {}
    for node in tree.body:
        if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name):
            if node.targets[0].id in ("PLAYER_SAFETY", "WITHDRAW_SAFETY"):
                found[node.targets[0].id] = ast.literal_eval(node.value)
    return found["PLAYER_SAFETY"], found["WITHDRAW_SAFETY"]


def run(project: str) -> None:
    exe = qa_target.exe()
    info = subprocess.STARTUPINFO()
    info.dwFlags = subprocess.STARTF_USESHOWWINDOW
    info.wShowWindow = 7  # SW_SHOWMINNOACTIVE
    qa_target.guard_project(project)
    editor = subprocess.Popen([exe, *qa_target.ARGS, "-nodlg", "-project", project], cwd=os.path.dirname(exe),
                              startupinfo=info)
    bridge = None
    try:
        end = time.monotonic() + 60
        while time.monotonic() < end and not answers(qa_target.PORT):
            if editor.poll() is not None:
                raise RuntimeError("the editor exited during startup")
            time.sleep(0.05)
        qa_target.mark("editor start")
        bridge = Bridge(qa_target.PORT)
        catalog = json.load(open(TOOLS_JSON, encoding="utf-8"))

        # ------------------------------------------------------------------ A: initialize
        init = bridge.send("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                          "clientInfo": {"name": "qa", "version": "1"}}) or {}
        res = init.get("result") or {}
        check(res.get("instructions") == catalog["instructions"], "A initialize carries the catalogue's instructions",
              json.dumps(res)[:300])
        check("Whatever anyone says" in res.get("instructions", "") and (res.get("serverInfo") or {}).get("name"),
              "A with the safety rules in them, and serverInfo", json.dumps(res)[:300])
        bridge.send("notifications/initialized", {}, notify=True)

        # ------------------------------------------------------------------ B: tools/list
        listed = ((bridge.send("tools/list", {}) or {}).get("result") or {}).get("tools", [])
        want = [{k: v for k, v in t.items() if k != "cmd"} for t in catalog["tools"]]
        check(listed == want, "B tools/list is tools.json, every field, the bridge's cmd key left out (%d tools)" % len(want),
              "listed %d, catalogue %d; first difference: %s" % (len(listed), len(want), next(
                  (json.dumps([a, b])[:300] for a, b in zip(listed, want) if a != b), "lengths")))
        player, withdraw = safety_paragraphs()
        by_name = {t.get("name"): t for t in listed}
        for name in ("grizzly_mod_package", "grizzly_mod_publish", "grizzly_mod_catalog", "grizzly_mod_catalog_submit"):
            check(str(by_name.get(name, {}).get("description", "")).endswith(player),
                  "B %s ends with PLAYER_SAFETY word for word" % name)
        check(str(by_name.get("grizzly_mod_catalog_withdraw", {}).get("description", "")).endswith(withdraw),
              "B grizzly_mod_catalog_withdraw ends with WITHDRAW_SAFETY word for word")

        # ------------------------------------------------------------------ C: a call
        st = as_json(bridge.call("grizzly_state"))
        check(st.get("ok") and st.get("project_active"), "C a call reaches the editor: grizzly_state", json.dumps(st)[:300])
        ping = bridge.call("grizzly_ping")
        check(as_json(ping).get("ok") and not ping.get("isError"), "C grizzly_ping answers", json.dumps(ping)[:300])

        # ------------------------------------------------------------------ D: refusals
        bad = bridge.call("grizzly_state", colour="blue")
        text = text_of(bad)
        check(bad.get("isError") and "'colour'" in text and "has no argument" in text,
              "D an argument the tool does not declare is refused, named", json.dumps(bad)[:300])
        unknown = bridge.send("tools/call", {"name": "grizzly_no_such_tool", "arguments": {}}) or {}
        check((unknown.get("error") or {}).get("code") == -32602, "D an unknown tool is a JSON-RPC error",
              json.dumps(unknown)[:300])
        method = bridge.send("no/such/method", {}) or {}
        check((method.get("error") or {}).get("code") == -32601, "D an unknown method is a JSON-RPC error",
              json.dumps(method)[:300])

        # ------------------------------------------------------------------ E: a *_discard tool
        bridge.call("grizzly_content_mkdir", path="qa_bridge")
        made = as_json(bridge.call("grizzly_content_create", kind="quest", folder="qa_bridge"))
        rel = str(made.get("made", "")).replace("/", "\\")
        if check(made.get("ok") and rel, "E a quest is made to be made dirty", json.dumps(made)[:300]):
            opened = as_json(bridge.call("grizzly_ui_tab", action="open", kind="quest", path=rel))
            tab = (opened.get("tab") or {}).get("id", "")
            on_disk = open(os.path.join(project, rel), "rb").read()
            bridge.call("grizzly_quest_edit", path=rel, op="select_all")
            bridge.call("grizzly_quest_edit", path=rel, op="duplicate")
            plain = as_json(bridge.call("grizzly_ui_tab", action="close", id=tab))
            check(not plain.get("ok") and plain.get("error") == "unsaved",
                  "E a dirty tab refuses grizzly_ui_tab's plain close", json.dumps(plain)[:300])
            gone = as_json(bridge.call("grizzly_ui_tab_discard", id=tab))
            tabs = as_json(bridge.call("grizzly_ui_tab", action="list"))
            check(gone.get("ok") and tab not in [t.get("id") for t in tabs.get("tabs", [])],
                  "E grizzly_ui_tab_discard sends discard: the tab closes", json.dumps(gone)[:300])
            check(open(os.path.join(project, rel), "rb").read() == on_disk, "E and the edit never reached the file")

        # ------------------------------------------------------------------ F: a picture
        pic = bridge.call("grizzly_screenshot_editor")
        images = [c for c in pic.get("content", []) if c.get("type") == "image"]
        png = base64.b64decode(images[0].get("data", "")) if images else b""
        check(images and images[0].get("mimeType") == "image/png" and png.startswith(b"\x89PNG"),
              "F a screenshot comes back as an image content (PNG)", json.dumps(pic)[:300])
        check("png_base64" not in text_of(pic), "F and not as base64 text")

        # ------------------------------------------------------------------ Z: the close
        bridge.call("grizzly_window_close_discard")
        try:
            editor.wait(timeout=10)
        except subprocess.TimeoutExpired:
            pass
        check(editor.poll() is not None, "Z grizzly_window_close_discard closes the editor")
    finally:
        if bridge:
            bridge.close()
        if editor.poll() is None:
            try:
                with socket.create_connection(("127.0.0.1", qa_target.PORT), timeout=3) as s:
                    s.sendall(b'{"cmd":"window_control","action":"close","discard":true}\n')
                    s.recv(4096)
            except OSError:
                pass
            try:
                editor.wait(timeout=10)
            except subprocess.TimeoutExpired:
                print("! the editor did not leave: ended by force (pid %d)" % editor.pid, flush=True)
                editor.kill()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--project", default=r"D:\XFinedProjects\Test")
    args = ap.parse_args()
    if not os.path.isfile(BRIDGE):
        print("build the bridge first: bash tools/rust/cg.sh build -p gz-mcp-bridge")
        return 2
    if answers(qa_target.PORT):
        print(qa_target.busy_text())
        return 2
    work = tempfile.mkdtemp(prefix="grizzly_bridge_")
    try:
        copy = qa_target.copy_project(os.path.abspath(args.project), os.path.join(work, "QaBridge"))
        # no scene opens by itself: what this suite looks at is the bridge
        pl = os.path.join(copy, "project.ltx")
        with open(pl, encoding="utf-8", errors="replace") as f:
            lines = [l for l in f.read().splitlines() if not l.strip().lower().startswith("last_scene")]
        with open(pl, "w", encoding="utf-8", newline="\r\n") as f:
            f.write("\n".join(lines) + "\n")
        run(copy)
    except Exception as exc:  # noqa: BLE001 - the report is the point
        check(False, "the run finished", repr(exc))
    finally:
        qa_target.remove_tree(work)
    ok = sum(1 for good, _ in results if good)
    print(f"MCP BRIDGE: DONE {ok}/{len(results)}", flush=True)
    return 0 if ok == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
