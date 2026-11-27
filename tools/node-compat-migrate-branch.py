#!/usr/bin/env python3
"""Move a branch's additions to the old single-file tests/node-compat.json into tests/node-compat/<slug>.json.

usage: node-compat-migrate-branch.py [--branch REF] [--base REF] [--dry-run]

Reads git objects only: the cases of REF:tests/node-compat.json (default HEAD) that are new or
changed relative to merge-base(REF, BASE):tests/node-compat.json (BASE default origin/main).
Run it BEFORE merging the main that deletes tests/node-compat.json; after the merge, resolve the
modify/delete conflict with `git rm tests/node-compat.json`. If main was already merged, pass
--base <last commit before the split> (the merge-base of the branch's pre-merge parent and main).
"""
import argparse, importlib.util, json, pathlib, subprocess, sys

spec = importlib.util.spec_from_file_location("bundle", pathlib.Path(__file__).with_name("node-compat-bundle.py"))
bundle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bundle)


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def cases(ref):
    return {c["name"]: c for c in json.loads(git("show", f"{ref}:tests/node-compat.json"))["cases"]}


ap = argparse.ArgumentParser()
ap.add_argument("--branch", default="HEAD")
ap.add_argument("--base", default="origin/main")
ap.add_argument("--dry-run", action="store_true")
a = ap.parse_args()
try:
    old = cases(git("merge-base", a.branch, a.base).strip())
    mine = cases(a.branch)
except subprocess.CalledProcessError as e:
    sys.exit(f"cannot read tests/node-compat.json (already split? pass --base): {e.stderr.strip()}")
added = [c for n, c in mine.items() if old.get(n) != c]
out = pathlib.Path(git("rev-parse", "--show-toplevel").strip()) / "tests" / "node-compat"
out.mkdir(parents=True, exist_ok=True)
for c in added:
    path = out / f"{bundle.slug(c['name'])}.json"
    print(("would write " if a.dry_run else "write ") + str(path.relative_to(out.parent.parent)))
    if not a.dry_run:
        path.write_text(bundle.dump(c), encoding="utf-8")
print(f"{len(added)} case(s)")
