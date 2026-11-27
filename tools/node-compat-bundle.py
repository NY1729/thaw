#!/usr/bin/env python3
"""tests/node-compat/ <-> single-manifest helper.

  node-compat-bundle.py [DIR]            print {"cases":[...]} for DIR (default tests/node-compat), name order
  node-compat-bundle.py --split FILE DIR write each case of a one-file manifest to DIR/<slug>.json
  node-compat-bundle.py --check DIR      verify file names match slug(name) and names are unique

One case per file; file name = slug(name) + "-" + sha1(name)[:6], so two branches adding the
same case collide on the file name while different cases never do.
"""
import hashlib, json, pathlib, re, sys


def slug(name):
    base = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")[:80].strip("-") or "case"
    return f"{base}-{hashlib.sha1(name.encode('utf-8')).hexdigest()[:6]}"


def dump(case):
    return json.dumps(case, indent=2, ensure_ascii=False) + "\n"


def load(directory):
    cases, seen = [], {}
    for path in sorted(pathlib.Path(directory).glob("*.json")):
        case = json.loads(path.read_text(encoding="utf-8"))
        if case["name"] in seen:
            sys.exit(f"duplicate case name {case['name']!r}: {seen[case['name']]} and {path}")
        seen[case["name"]] = path
        cases.append((path, case))
    return cases


def main(argv):
    if argv[:1] == ["--split"]:
        out = pathlib.Path(argv[2])
        out.mkdir(parents=True, exist_ok=True)
        for case in json.load(open(argv[1], encoding="utf-8"))["cases"]:
            path = out / f"{slug(case['name'])}.json"
            if path.exists():
                sys.exit(f"duplicate case name {case['name']!r}")
            path.write_text(dump(case), encoding="utf-8")
    elif argv[:1] == ["--check"]:
        bad = [p for p, c in load(argv[1]) if p.stem != slug(c["name"])]
        if bad:
            sys.exit("file name does not match slug(name): " + ", ".join(map(str, bad)))
    else:
        directory = argv[0] if argv else "tests/node-compat"
        json.dump({"cases": [c for _, c in load(directory)]}, sys.stdout, ensure_ascii=False, indent=2)
        print()


if __name__ == "__main__":
    main(sys.argv[1:])
