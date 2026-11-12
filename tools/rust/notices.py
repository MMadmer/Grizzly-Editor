"""The third-party notices of the two programs, made from what they really link.

    python tools/rust/notices.py            writes licenses/THIRD_PARTY_NOTICES.txt
    python tools/rust/notices.py --check    exits 1 when the file on disk is not what this would write

The file is `licenses/NOTICES_HEAD.txt` (written by hand: the C code the crates vendor and the origin of the LZHUF
coder, which no manifest describes) followed by every third-party crate in the dependency closure of `grizzly` and `gz-mcp-bridge`
for Windows - normal dependencies only: a build script's and a proc macro's own dependencies run at build time and
are not in the programs. Each crate is listed with its version, the license its manifest states and its
repository; the license files the crates ship follow, each text once with the crates that carry it.

It reads `cargo metadata` (the lock file and the registry's sources on this machine); nothing is downloaded.
"""
import hashlib
import json
import os
import re
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(REPO, "licenses", "THIRD_PARTY_NOTICES.txt")
HEAD = os.path.join(REPO, "licenses", "NOTICES_HEAD.txt")
ROOTS = ("grizzly", "gz-mcp-bridge")
TARGET = "x86_64-pc-windows-msvc"
LICENSE_FILE = re.compile(r"^(licen[sc]e|copying|notice|unlicense|copyright)", re.IGNORECASE)
WIDTH = 100


def metadata() -> dict:
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--offline", "--filter-platform", TARGET],
        cwd=REPO, capture_output=True, text=True, encoding="utf-8")
    if out.returncode != 0:
        sys.exit("cargo metadata failed:\n" + out.stderr)
    return json.loads(out.stdout)


def linked(meta: dict) -> list[dict]:
    """The third-party packages reached from the two programs through normal dependencies, never through a proc
    macro (its code and its dependencies run in the compiler)."""
    packages = {p["id"]: p for p in meta["packages"]}
    members = set(meta["workspace_members"])
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    seen: set[str] = set()
    stack = [i for i in members if packages[i]["name"] in ROOTS]
    if len(stack) != len(ROOTS):
        sys.exit(f"the workspace does not hold {ROOTS}")
    while stack:
        i = stack.pop()
        if i in seen:
            continue
        if any("proc-macro" in t["kind"] for t in packages[i]["targets"]):
            continue
        seen.add(i)
        for dep in nodes[i]["deps"]:
            if any(k["kind"] is None for k in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    return sorted((packages[i] for i in seen if i not in members), key=lambda p: (p["name"].lower(), p["version"]))


def license_texts(package: dict) -> list[tuple[str, str]]:
    """The license files at the top of a package: (file name, text)."""
    folder = os.path.dirname(package["manifest_path"])
    found = []
    try:
        names = sorted(os.listdir(folder), key=str.lower)
    except OSError:
        return found
    paths = [(name, os.path.join(folder, name)) for name in names if LICENSE_FILE.match(name)]
    # a folder of fonts the crate embeds carries their licenses beside them, as text files
    for name in names:
        sub = os.path.join(folder, name)
        if not os.path.isdir(sub):
            continue
        inside = sorted(os.listdir(sub), key=str.lower)
        fonts = any(n.lower().endswith((".ttf", ".otf")) for n in inside)
        paths += [(f"{name}/{n}", os.path.join(sub, n)) for n in inside
                  if LICENSE_FILE.match(n) or (fonts and n.lower().endswith(".txt"))]
    for name, path in paths:
        if not os.path.isfile(path):
            continue
        with open(path, "rb") as f:
            text = f.read().decode("utf-8", "replace")
        text = "\n".join(line.rstrip() for line in text.replace("\r\n", "\n").replace("\r", "\n").split("\n")).strip()
        if text:
            found.append((name, text))
    return found


def wrap_names(names: list[str]) -> str:
    lines, line = [], "  "
    for n in names:
        piece = n + ", "
        if len(line) + len(piece) > WIDTH and line.strip():
            lines.append(line.rstrip())
            line = "  "
        line += piece
    lines.append(line.rstrip().rstrip(","))
    return "\n".join(lines)


def build() -> str:
    with open(HEAD, "rb") as f:
        head = f.read().decode("utf-8").replace("\r\n", "\n").rstrip() + "\n"
    crates = linked(metadata())
    out = [head, "", "Rust crates", "===========", "",
           f"The two programs link {len(crates)} third-party crates (their normal dependencies for Windows). Each is",
           "listed with the license its manifest states; the license files the crates ship follow, each text",
           "once with the crates that carry it.", ""]
    texts: dict[str, dict] = {}
    bare = []
    for p in crates:
        tag = f"{p['name']} {p['version']}"
        out.append(f"{tag} - {p.get('license') or 'see its license file'}" + (f" - {p['repository']}" if p.get("repository") else ""))
        files = license_texts(p)
        if not files:
            bare.append(tag)
        for name, text in files:
            key = hashlib.sha256(" ".join(text.split()).encode("utf-8")).hexdigest()
            entry = texts.setdefault(key, {"text": text, "users": []})
            entry["users"].append(tag)
    if bare:
        out += ["", "Crates whose package carries no license file (the license is the one their manifest states, above):",
                wrap_names(bare)]
    out += ["", "", "License texts", "=============", ""]
    for entry in sorted(texts.values(), key=lambda e: (e["users"][0].lower(), e["text"])):
        users = sorted(set(entry["users"]), key=str.lower)
        out += ["-" * WIDTH, "Carried by:", wrap_names(users), "-" * WIDTH, "", entry["text"], "", ""]
    return "\n".join(out).rstrip() + "\n"


def main() -> int:
    text = build()
    if "--check" in sys.argv[1:]:
        try:
            with open(OUT, "rb") as f:
                on_disk = f.read().decode("utf-8").replace("\r\n", "\n")
        except OSError:
            on_disk = ""
        if on_disk != text:
            print("licenses/THIRD_PARTY_NOTICES.txt is not what the lock file gives: python tools/rust/notices.py")
            return 1
        print("notices: current")
        return 0
    with open(OUT, "wb") as f:
        f.write(text.encode("utf-8"))
    print(f"{os.path.relpath(OUT, REPO)}: {len(text.encode('utf-8')) // 1024} KB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
