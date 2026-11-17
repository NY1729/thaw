#!/usr/bin/env python3
"""Bucket thaw-js declaration blockers by declaration shape and gate reason.

Input: any mix of files or directories. Recognised records:
  - `THAW_ARTIFACT_V1:{...}` manifest lines (manifest.txt) and JSON files with `thaw_js_blockers`
  - `thaw-js-blocker: package=P why=W` lines (`thaw inspect` output, inspect.txt)
  - `thaw-js routing: package `P` stays on QuickJS: W` lines (THAW_JS_RUNTIME_DEBUG stderr)
A directory holding a `name.txt` is a corpus case (its name is the file content, marked `*`);
otherwise the case is the parent directory name.

Usage:
  python3 blocker_shapes.py [--json] PATH...
"""

import argparse
import json
import os
import re
import sys
from collections import defaultdict

SCANNED_SUFFIXES = (".txt", ".log", ".json", ".stderr", ".stdout")
ALLOWED_WORDS = {"number", "string", "boolean", "Json", "void", "null", "undefined", "JsValue"}
SHAPE_TEXT = {
    "S1": "callback param",
    "S2": "callback nested in options",
    "S3": "class-typed param/result",
    "S4": "Promise<Class> / async callback",
    "S5": "constructor arity",
    "S6": "generic with callbacks",
    "other": "unclassified",
    "-": "no declaration shape",
}
IDENT_RE = re.compile(r"[0-9][A-Za-z0-9_]*|[A-Za-z_$][A-Za-z0-9_$]*(?:\.[A-Za-z_$][A-Za-z0-9_$]*)*")
SYMBOL_RE = re.compile(r"declare function __thaw_typed_(?:js|tjs)_([0-9a-f]+)")
ROUTING_RE = re.compile(r"thaw-js routing: package `(.+?)` stays on QuickJS: (.*)$")
INSPECT_RE = re.compile(r"^thaw-js-blocker: package=(\S+) why=(.*)$")
MANIFEST_MARK = "THAW_ARTIFACT_V1:"


def decode_key(hex_text):
    """The `$new$pkg::Class$arity1`-style key a declaration symbol encodes, or ''."""
    try:
        return bytes.fromhex(hex_text).decode("utf-8")
    except ValueError:
        return ""


def split_signature(line):
    """(generics, params, result) of a `declare function` line, or None when it does not parse."""
    match = SYMBOL_RE.search(line)
    if not match:
        return None
    after = line[match.end():]
    start = next((i for i, ch in enumerate(after) if ch in "(<"), None)
    if start is None:
        return None
    generics = ""
    if after[start] == "<":
        depth, close = 0, None
        for index in range(start, len(after)):
            if after[index] == "<":
                depth += 1
            elif after[index] == ">":
                depth -= 1
                if depth == 0:
                    close = index
                    break
        if close is None or "(" not in after[close:]:
            return None
        generics = after[start + 1:close]
        start = after.index("(", close)
    end = after.rfind("): ")
    if end < start:
        return None
    return generics, after[start + 1:end], after[end + 3:].strip().rstrip(";").strip()


def generic_names(generics):
    """Type parameter names: the leading identifier of each top-level entry of `<...>`."""
    entries, depth, current = [], 0, ""
    for ch in generics:
        if ch in "<([{":
            depth += 1
        elif ch in ">)]}":
            depth -= 1
        if ch == "," and depth == 0:
            entries.append(current)
            current = ""
        else:
            current += ch
    entries.append(current)
    names = []
    for entry in entries:
        match = re.match(r"\s*([A-Za-z_$][A-Za-z0-9_$]*)", entry)
        if match:
            names.append(match.group(1))
    return names


def type_words(text, generics):
    """Identifiers used as types (not parameter/property names, not allowed or generic)."""
    words, index = [], 0
    while index < len(text):
        ch = text[index]
        if ch in "\"'":
            close = text.find(ch, index + 1)
            index = len(text) if close == -1 else close + 1
            continue
        match = IDENT_RE.match(text, index)
        if not match:
            index += 1
            continue
        word, after = match.group(0), match.end()
        rest = text[after:].lstrip(" ")
        if rest.startswith("?"):
            rest = rest[1:].lstrip(" ")
        is_name = rest.startswith(":")
        if not is_name and word not in ALLOWED_WORDS and word not in generics and word != "Promise":
            if word not in words:
                words.append(word)
        index = after
    return words


def top_level_arrow(params):
    """True when a `=>` sits outside every `{...}` of the parameter list."""
    depth = 0
    for index, ch in enumerate(params):
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
        elif params.startswith("=>", index) and depth == 0:
            return True
    return False


def classify(package, why, class_arities=None):
    """Bucket one blocker. Returns a dict; `class_arities` maps (package, class) to the set of
    `$new$` keys seen in the input (more than one makes the constructor shape S5)."""
    class_arities = class_arities or {}
    row = {
        "package": package, "why": why, "name": "-", "kind": "other", "shape": "other",
        "reason": "other", "cause": "-", "type_word": "-",
    }
    class_match = re.match(r"class `([^`]+)`: (.*)$", why)
    fn_match = re.match(r"`([^`]+)`: (.*)$", why)
    if class_match or fn_match:
        match = class_match or fn_match
        row["kind"] = "class" if class_match else "fn"
        row["name"] = match.group(1)
        line = match.group(2)
        parsed = split_signature(line) if line.startswith("declare function") else None
        if parsed is None:
            return row
        generics, params, result = parsed
        symbol = SYMBOL_RE.search(line).group(1)
        key = decode_key(symbol)
        names = generic_names(generics)
        has_arrow = "=>" in params or "=>" in result
        words = type_words(params + " " + result, names)
        causes = []
        if has_arrow:
            causes.append("callback")
        if words:
            causes.append("type word")
        row["cause"] = ",".join(causes) or "-"
        row["type_word"] = ",".join(words) or "-"
        if row["kind"] == "class":
            row["reason"] = "class op"
        elif has_arrow:
            row["reason"] = "callback"
        elif words:
            row["reason"] = "type word"
        else:
            return row
        if "Promise<" in params or "Promise<" in result:
            row["shape"] = "S4"
        elif has_arrow and names:
            row["shape"] = "S6"
        elif key.startswith("$new$") and len(class_arities.get((package, row["name"]), ())) > 1:
            row["shape"] = "S5"
        elif has_arrow:
            row["shape"] = "S1" if top_level_arrow(params) else "S2"
        elif words:
            row["shape"] = "S3"
        return row
    if "has no declaration" in why:
        row.update(reason="no declaration", name=(re.search(r"`([^`]+)`", why) or [None, "-"])[1])
    elif "has no typed target" in why:
        row.update(reason="no typed target", name=(re.search(r"`([^`]+)`", why) or [None, "-"])[1])
    elif why.startswith(("bundle uses unsupported features", "bundle scan failed")):
        row["reason"] = "bundle scan"
    elif why.startswith("routed build fell back to QuickJS"):
        row["cause"] = "build fallback"
    return row


def parse_records(text):
    """(package, why) pairs from one file's text, in order."""
    pairs = []
    stripped = text.lstrip()
    if stripped.startswith("{"):
        try:
            data = json.loads(text)
        except ValueError:
            data = None
        if isinstance(data, dict) and isinstance(data.get("thaw_js_blockers"), list):
            return [(e.get("package", "?"), e.get("why", "?")) for e in data["thaw_js_blockers"]]
    for line in text.splitlines():
        if MANIFEST_MARK in line:
            blob = line[line.index(MANIFEST_MARK) + len(MANIFEST_MARK):]
            try:
                manifest, _ = json.JSONDecoder().raw_decode(blob)
            except ValueError:
                continue
            pairs += [(e.get("package", "?"), e.get("why", "?"))
                      for e in manifest.get("thaw_js_blockers", []) if isinstance(e, dict)]
            continue
        match = INSPECT_RE.match(line) or ROUTING_RE.match(line.strip())
        if match:
            pairs.append((match.group(1), match.group(2)))
    return pairs


def case_of(path, root):
    """(case name, is_corpus_case) for a recorded file."""
    folder = os.path.dirname(path)
    name_file = os.path.join(folder, "name.txt")
    if os.path.isfile(name_file):
        with open(name_file, encoding="utf-8", errors="replace") as handle:
            return handle.read().strip() or os.path.basename(folder), True
    rel = os.path.relpath(path, root) if os.path.isdir(root) else os.path.basename(path)
    return rel, False


def collect(paths):
    """All (package, why, case, corpus) records under the given paths, plus the files scanned."""
    records, scanned = [], 0
    for top in paths:
        files = []
        if os.path.isdir(top):
            for folder, _, names in os.walk(top):
                files += [os.path.join(folder, n) for n in names if n.endswith(SCANNED_SUFFIXES)]
        elif os.path.isfile(top):
            files = [top]
        for path in sorted(files):
            scanned += 1
            with open(path, encoding="utf-8", errors="replace") as handle:
                text = handle.read()
            pairs = parse_records(text)
            if not pairs:
                continue
            case, corpus = case_of(path, top)
            records += [(package, why, case, corpus) for package, why in pairs]
    return records, scanned


def build_rows(records):
    class_arities = defaultdict(set)
    for package, why, _, _ in records:
        match = re.match(r"class `([^`]+)`: declare function __thaw_typed_(?:js|tjs)_([0-9a-f]+)", why)
        if match:
            key = decode_key(match.group(2))
            if key.startswith("$new$"):
                class_arities[(package, match.group(1))].add(key)
    grouped = {}
    for package, why, case, corpus in records:
        row = classify(package, why, class_arities)
        ident = (row["shape"], row["reason"], row["cause"], row["type_word"], package, row["name"], why)
        entry = grouped.setdefault(ident, {**row, "cases": set(), "corpus": False})
        entry["cases"].add(case)
        entry["corpus"] = entry["corpus"] or corpus
    rows = []
    for entry in grouped.values():
        cases = entry.pop("cases")
        entry["case_count"] = len(cases)
        entry["first_case"] = min(cases)
        rows.append(entry)
    rows.sort(key=lambda r: (r["shape"], r["reason"], r["package"], r["name"]))
    return rows


def print_table(rows, scanned, records):
    print(f"files scanned: {scanned}; blocker records: {len(records)}; unique blocker lines: {len(rows)}")
    print()
    print("shape | reason | cause | type-word | package | fn/class | #records | first case | corpus")
    for row in rows:
        marker = "*" if row["corpus"] else ""
        print(" | ".join([row["shape"], row["reason"], row["cause"], row["type_word"], row["package"],
                          row["name"], str(row["case_count"]), row["first_case"], marker]))
    print()
    print("summary (unique blocker lines per shape/reason):")
    summary = defaultdict(int)
    for row in rows:
        summary[(row["shape"], row["reason"])] += 1
    for (shape, reason), count in sorted(summary.items()):
        print(f"  {shape:6} {reason:16} {count:4}  {SHAPE_TEXT.get(shape, '')}")
    others = [row for row in rows if row["shape"] == "other" or row["reason"] == "other"]
    if others:
        print()
        print("unclassified (listed for review):")
        for row in others:
            print(f"  {row['package']}: {row['why'][:200]}")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("paths", nargs="+", help="artifact files or directories")
    parser.add_argument("--json", action="store_true", help="print JSON instead of the table")
    args = parser.parse_args(argv)
    records, scanned = collect(args.paths)
    rows = build_rows(records)
    if args.json:
        out = {"files_scanned": scanned, "records": len(records), "rows": rows}
        json.dump(out, sys.stdout, indent=2)
        print()
    else:
        print_table(rows, scanned, records)


if __name__ == "__main__":
    main()
