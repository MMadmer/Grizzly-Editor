#!/usr/bin/env python3
"""The release archives of Grizzly Editor (docs/UPDATES.md, sections 1 and 2).

    python tools/rust/package.py [--bin Bin\\x64\\Release] [--out Bin\\x64\\Package]

Takes the release build (the three programs, LICENSE.txt, THIRD_PARTY_NOTICES.txt) and the tracked
runtime data the editor reads, and writes into the output folder:

    update-manifest.txt                       the managed set of this version
    Grizzly-Editor-<version>.zip              the complete installation, the manifest included
    Grizzly-Editor-<version>-symbols.zip      the PDBs of the three programs

The archive is deterministic: entries sorted by name, deflated, UTF-8 names with forward slashes,
one fixed timestamp. It is read back and checked against the manifest before it is kept. The version
is the workspace's (Cargo.toml). Nothing is published: a release goes out by the author's hand.

`Build.ps1 -Package` builds the release and runs this. The functions are the suite's too
(tools/qa/test_update.py packs its fake releases with them).
"""
from __future__ import annotations

import argparse
import hashlib
import os
import re
import subprocess
import sys
import zipfile

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SCHEMA = "grizzly-editor.update/1"
MANIFEST = "update-manifest.txt"
# the oldest date a ZIP entry can carry: every entry has it, so the archive depends on content alone
STAMP = (1980, 1, 1, 0, 0, 0)

# what the release build puts beside the programs, all of it managed
PROGRAMS = ["Grizzly.exe", "Grizzly-Cmd.exe", "grizzly-mcp.exe", "LICENSE.txt", "THIRD_PARTY_NOTICES.txt"]
SYMBOLS = ["Grizzly.pdb", "Grizzly_Cmd.pdb", "grizzly_mcp.pdb"]
# the tracked data the editor reads at run time, under its data root
RUNTIME = ["fs.ltx", "fsgame.ltx", "gamedata", "rawdata"]
# tracked data no code of the editor opens, with the reason it stays out of the installation
LEFT_OUT = {
    "gamedata/shaders/": "shader sources of an earlier renderer: the editor draws with WGSL shaders built into it",
    "gamedata/DOT_NOT_DELETE.txt": "a placeholder that keeps the folder in git",
}


def workspace_version(repo: str = REPO) -> str:
    """[workspace.package] version of Cargo.toml."""
    with open(os.path.join(repo, "Cargo.toml"), encoding="utf-8") as f:
        text = f.read()
    section = text.split("[workspace.package]", 1)[1].split("\n[", 1)[0]
    found = re.search(r'^version\s*=\s*"(\d+\.\d+\.\d+)"', section, re.M)
    if not found:
        raise SystemExit("Cargo.toml names no workspace version")
    return found.group(1)


def runtime_files(repo: str = REPO) -> list[str]:
    """The tracked runtime data, forward slashes, minus what LEFT_OUT names."""
    out = subprocess.run(["git", "ls-files", "-z", "--", *RUNTIME], cwd=repo, capture_output=True, check=True).stdout
    files = [p for p in out.decode("utf-8").split("\0") if p]
    return sorted(p for p in files if not any(p == x or (x.endswith("/") and p.startswith(x)) for x in LEFT_OUT))


def sha256_size(data_or_path: bytes | str) -> tuple[str, int]:
    h = hashlib.sha256()
    if isinstance(data_or_path, bytes):
        h.update(data_or_path)
        return h.hexdigest(), len(data_or_path)
    size = 0
    with open(data_or_path, "rb") as f:
        while chunk := f.read(1 << 20):
            h.update(chunk)
            size += len(chunk)
    return h.hexdigest(), size


def manifest_text(version: str, rows: list[tuple[str, int, str]]) -> bytes:
    """The manifest of `rows` (sha256, size, path): one row per file, sorted by path."""
    lines = [f"schema={SCHEMA}", f"version={version}"]
    lines += [f"{sha}\t{size}\t{path}" for sha, size, path in sorted(rows, key=lambda r: r[2])]
    return ("\n".join(lines) + "\n").encode("utf-8")


def write_zip(path: str, entries: dict[str, bytes | str]) -> None:
    """A deterministic archive of `entries` (name -> bytes, or the path of a file to read)."""
    tmp = path + ".tmp"
    with zipfile.ZipFile(tmp, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9, allowZip64=True) as z:
        for name in sorted(entries):
            src = entries[name]
            info = zipfile.ZipInfo(name, date_time=STAMP)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.create_system = 0
            info.external_attr = 0
            # the level travels on the entry (its attribute's name depends on the Python version)
            for attr in ("compress_level", "_compresslevel"):
                if attr in zipfile.ZipInfo.__slots__:
                    setattr(info, attr, 9)
            if isinstance(src, bytes):
                z.writestr(info, src)
                continue
            size = os.path.getsize(src)
            with open(src, "rb") as f, z.open(info, "w", force_zip64=size >= (1 << 31)) as dst:
                while chunk := f.read(1 << 20):
                    dst.write(chunk)
    os.replace(tmp, path)


def pack(version: str, files: dict[str, bytes | str], out_dir: str) -> tuple[str, bytes]:
    """The release archive of `files` (path in the installation -> bytes or a file) with its manifest,
    written as Grizzly-Editor-<version>.zip into `out_dir` and checked. Answers its path and the
    manifest."""
    rows = []
    for name, src in files.items():
        sha, size = sha256_size(src)
        rows.append((sha, size, name))
    manifest = manifest_text(version, rows)
    archive = os.path.join(out_dir, f"Grizzly-Editor-{version}.zip")
    write_zip(archive, {**files, MANIFEST: manifest})
    verify(archive, manifest)
    return archive, manifest


def verify(archive: str, manifest: bytes) -> None:
    """Reads the archive back: the manifest and every file it names, exactly once, at its size and
    SHA-256, stored or deflated, nothing encrypted. Raises on the first difference."""
    lines = manifest.decode("utf-8").splitlines()
    if lines[:1] != [f"schema={SCHEMA}"]:
        raise ValueError("the manifest's schema is not " + SCHEMA)
    rows = {}
    for line in lines[2:]:
        sha, size, path = line.split("\t", 2)
        rows[path] = (sha, int(size))
    with zipfile.ZipFile(archive) as z:
        infos = z.infolist()
        names = [i.filename for i in infos]
        if len(set(n.lower() for n in names)) != len(names):
            raise ValueError("the archive holds a name twice")
        if sorted(names) != sorted([*rows, MANIFEST]):
            extra = sorted(set(names) - set(rows) - {MANIFEST})
            missing = sorted(set(rows) - set(names))
            raise ValueError(f"the archive and its manifest differ: extra {extra}, missing {missing}")
        for info in infos:
            if info.flag_bits & 1 or info.compress_type not in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED):
                raise ValueError(f"{info.filename} is encrypted or packed another way")
            if "\\" in info.filename:
                raise ValueError(f"{info.filename} is not named with forward slashes")
            data = z.read(info)
            if info.filename == MANIFEST:
                if data != manifest:
                    raise ValueError("the archive's manifest is not the one written")
                continue
            sha, size = rows[info.filename]
            if len(data) != size or hashlib.sha256(data).hexdigest() != sha:
                raise ValueError(f"{info.filename} in the archive is not what the manifest says")


def mb(n: int) -> str:
    return f"{n / (1 << 20):.1f} MB"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--bin", default=os.path.join(REPO, "Bin", "x64", "Release"), help="the release build's folder")
    ap.add_argument("--out", default=os.path.join(REPO, "Bin", "x64", "Package"), help="where the archives go")
    args = ap.parse_args()
    version = workspace_version()
    files: dict[str, bytes | str] = {}
    for name in PROGRAMS:
        path = os.path.join(args.bin, name)
        if not os.path.isfile(path):
            print(f"PACKAGE ERROR: {path} is missing: build the release first (pwsh -NoProfile -File .\\Build.ps1)")
            return 1
        files[name] = path
    for rel in runtime_files():
        files[rel] = os.path.join(REPO, rel.replace("/", os.sep))
    os.makedirs(args.out, exist_ok=True)
    archive, manifest = pack(version, files, args.out)
    with open(os.path.join(args.out, MANIFEST), "wb") as f:
        f.write(manifest)
    symbols = {n: os.path.join(args.bin, n) for n in SYMBOLS if os.path.isfile(os.path.join(args.bin, n))}
    symbols_zip = os.path.join(args.out, f"Grizzly-Editor-{version}-symbols.zip")
    write_zip(symbols_zip, symbols)
    print(f"Packaged Grizzly Editor {version} into {args.out}:")
    print(f"  {os.path.basename(archive)}  {mb(os.path.getsize(archive))}, {len(files)} files, checked against its manifest")
    print(f"  {os.path.basename(symbols_zip)}  {mb(os.path.getsize(symbols_zip))}, {', '.join(sorted(symbols)) or 'no PDBs'}")
    print(f"  {MANIFEST}")
    for prefix, why in LEFT_OUT.items():
        print(f"  left out {prefix}: {why}")
    print("Nothing was published.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
