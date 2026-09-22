#!/usr/bin/env python3
"""Write src/dev/fixture.json from the demo folders and the real engine.

The browser harness (mock.html) answers every command from this file, so
screens can be photographed without driving the real window. Numbers, paths
and moves come from `tungstate plan --json` and `folder compare` against
scripts/demo-folder.sh's roots; nothing is typed to fit a layout.
Run after scripts/demo-folder.sh and a debug build of the CLI.
"""
import json, os, subprocess, re, time
from pathlib import Path

DEMO = Path.home() / ".tungstate-demo"
CLI = Path(__file__).resolve().parents[4] / "target/debug/tungstate"
ENV = dict(os.environ, TUNGSTATE_JOURNAL=str(DEMO / "journal.db"),
           TUNGSTATE_SECRETS="memory", HOME=str(DEMO / "home"))
OUT = Path(__file__).resolve().parents[1] / "src/dev/fixture.json"

def cli(*a):
    r = subprocess.run([str(CLI), *a], env=ENV, capture_output=True, text=True)
    if not r.stdout:
        raise SystemExit(f"{' '.join(a)}: {r.stderr.strip()}")
    return r.stdout

def walk(root):
    out = []
    for p in sorted(root.rglob("*")):
        rel = p.relative_to(root).as_posix()
        if rel.startswith(".tungstate") or p.name == ".DS_Store":
            continue
        is_dir = p.is_dir() and not p.is_symlink()
        out.append((rel, is_dir, 0 if is_dir else p.lstat().st_size))
    return out

def preview(name):
    root = DEMO / name
    plan = json.loads(cli("plan", str(root), "--json"))
    moves = [o for o in plan["ops"] if o["op"] == "move"]
    frm = {m["from"]: m["to"] for m in moves}
    files = walk(root)
    before = [dict(path=p, is_dir=d, size=s, moves=p in frm) for p, d, s in files]
    after_files = {}
    for p, d, s in files:
        if not d:
            after_files[frm.get(p, p)] = (s, p in frm)
    dirs = set()
    for p in after_files:
        parts = p.split("/")[:-1]
        for i in range(1, len(parts) + 1):
            dirs.add("/".join(parts[:i]))
    after = sorted([dict(path=d, is_dir=True, size=0, moves=False) for d in dirs] +
                   [dict(path=p, is_dir=False, size=s, moves=m) for p, (s, m) in after_files.items()],
                   key=lambda e: e["path"])
    def why(r):
        k = r["kind"]
        return {"already_there": "already where its rule puts it",
                "ignored": f"ignored: {r.get('because', '')}",
                "unmatched": "no rule claims it",
                "cooling": "changed too recently; waiting for its cooldown"}.get(k, k.replace("_", " "))
    b = plan["blast"]
    return dict(folder=plan["folder"], before=before, after=after,
                moves=[dict(frm=m["from"], to=m["to"], why=f"rule {m['because'].get('name', '')}") for m in moves],
                left_alone=[dict(path=u["file"], why=why(u["reason"])) for u in plan["untouched"]],
                files=b["files"], of=b["of"], bytes=b["bytes"], large=b["over_limit"],
                settles=plan["settles"], waiting=0, longest_wait=0, tidy=not moves, undoable=None)

def compare(name):
    text = cli("folder", "compare", str(DEMO / name))
    rows, cur = [], None
    for line in text.splitlines():
        m = re.match(r"\s{2}(\(the rules you have\)|[\w-]+)\s+(\d+) of (\d+) would move, (\d+) dir\(s\) made, (\d+) removed", line)
        if m:
            cur = dict(name=m[1], summary="", loads=True, settles=True, files=int(m[2]), of=int(m[3]),
                       bytes=0, created=int(m[4]), removed=int(m[5]), example=None)
            rows.append(cur)
            continue
        m = re.match(r"\s+e\.g\. (.+) → (.+)$", line)
        if m and cur:
            cur["example"] = dict(**{"from": m[1]}, to=m[2])
    return rows

def listing(path):
    p = Path(path)
    ents = []
    for c in sorted(p.iterdir()):
        if c.name.startswith("."):
            continue
        st = c.lstat()
        ents.append(dict(name=c.name, path=str(c), is_dir=c.is_dir(), size=0 if c.is_dir() else st.st_size,
                         modified=int(st.st_mtime)))
    return dict(path=str(p), parent=str(p.parent), entries=ents)

# real-shape has no rules of its own, so a copy governed by `documents` stands
# in for "a folder whose preview would dismantle a shape". Cloned, not copied,
# so the sparse files cost nothing.
shaped = DEMO / "real-shape-media"
if not shaped.exists():
    subprocess.run(["cp", "-cR", str(DEMO / "real-shape"), str(shaped)], check=True)
    subprocess.run([str(CLI), "init", str(shaped), "--template", "documents"], env=ENV, check=True,
                   capture_output=True)

fx = dict(
    preview={n: preview(n) for n in ["messy-downloads", "real-shape-media"]},
    compare={n: compare(n) for n in ["messy-downloads", "real-shape"]},
    listings={str(DEMO / d): listing(DEMO / d) for d in ["to-drain", "nas/incoming", "messy-downloads", "photos-by-nothing"]},
    now=int(time.time()),
)
for p in fx["preview"].values():
    for m in p["moves"]:
        m["from"] = m.pop("frm")
# The file is committed and the repository is public, so the capturing
# machine's home directory is swapped for a neutral one.
OUT.write_text(json.dumps(fx, indent=1).replace(str(Path.home()), "/Users/you"))
print("wrote", OUT, OUT.stat().st_size, "bytes")
