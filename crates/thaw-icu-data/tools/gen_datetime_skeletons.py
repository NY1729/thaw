#!/usr/bin/env python3
"""Generate `src/datetime_skeletons_data.rs` from CLDR JSON.

Vendors, for each curated locale and calendar, a candidate table mirroring
ICU's `DateTimePatternGenerator` `PatternMap` -- its canonical single-field
items, its standard `dateFormats`/`timeFormats` (added with `override=false`,
so only the first pattern per *base skeleton* survives), and its
`availableFormats` -- plus the `dateTimeFormats-atTime` glue patterns and the
locale's default hour cycle.

`thaw-quickjs`'s runtime matcher (`intl_datetime_skeleton.rs`) then runs
ICU's `DateTimeMatcher::getDistance`/`adjustFieldTypes` over that set,
reproducing Node/ICU4C's pattern choice instead of icu4x's length-pattern
formatting.

Non-Gregorian calendars use their own `ca-*.json` (era-prefixed skeletons),
not an overlay of Gregorian.

Usage:
    gen_datetime_skeletons.py <cldr-json-full.zip | extracted-root> <out.rs.data>

The curated locale list is read from `intl_locale.rs`, so this stays in sync.
"""

import json
import os
import re
import sys
import zipfile

REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))

# icu4x calendar marker name -> CLDR `ca-*.json` stem. Mirrors
# `icu_provider_source`'s `DatagenCalendar::cldr_name` (islamic variants are
# merged into one `islamic`, ethiopic-amete-alem into `ethiopic`).
CALENDARS = {
    "buddhist": "buddhist",
    "chinese": "chinese",
    "coptic": "coptic",
    "dangi": "dangi",
    "ethiopian": "ethiopic",
    "gregorian": "gregorian",
    "hebrew": "hebrew",
    "indian": "indian",
    "hijri": "islamic",
    "japanese": "japanese",
    "persian": "persian",
    "roc": "roc",
}

# Curated tags with no exact CLDR locale directory fall back to the CLDR
# root locale for their language (same data icu4x's own locale fallback
# resolved when the baked data was generated).
LOCALE_DIR_FALLBACK = {
    "en-US": "en",
    "pt-BR": "pt",
}


def curated_locales():
    src = open(os.path.join(REPO, "crates/thaw-quickjs/src/quickjs/intl_locale.rs")).read()
    m = re.search(r"const CURATED_LOCALES: &\[&str\] = &\[(.*?)\];", src, re.S)
    if not m:
        raise SystemExit("could not find CURATED_LOCALES in intl_locale.rs")
    return re.findall(r'"([^"]+)"', m.group(1))


def rust_str(s):
    out = []
    for ch in s:
        if ch == "\\":
            out.append("\\\\")
        elif ch == '"':
            out.append('\\"')
        else:
            out.append(ch)
    return '"' + "".join(out) + '"'


class Cldr:
    def __init__(self, path):
        self.zip = zipfile.ZipFile(path) if zipfile.is_zipfile(path) else None
        self.root = None if self.zip else path

    def find(self, stem, locale_dir):
        suffix = f"main/{locale_dir}/ca-{stem}.json"
        if self.zip is not None:
            for name in self.zip.namelist():
                if name.endswith(suffix):
                    return name
            return None
        for dirpath, _, files in os.walk(self.root):
            if f"ca-{stem}.json" in files and dirpath.replace("\\", "/").endswith(f"main/{locale_dir}"):
                return os.path.join(dirpath, f"ca-{stem}.json")
        return None

    def calendar(self, stem, locale_dir):
        name = self.find(stem, locale_dir)
        if name is None:
            return None
        data = json.loads(self.zip.read(name)) if self.zip is not None else json.load(open(name))
        return data["main"][locale_dir]["dates"]["calendars"][stem]


def val(x):
    return x["_value"] if isinstance(x, dict) else x


def available_formats(cal):
    af = cal["dateTimeFormats"]["availableFormats"]
    out = {}
    for key, pattern in af.items():
        # Drop `-alt-ascii` (ICU4X's default preference is the non-alt form)
        # and collapse `-count-<plural>` variants to their base skeleton.
        if key.endswith("-alt-ascii"):
            continue
        key = key.split("-count-", 1)[0]
        out.setdefault(key, val(pattern))
    return out


def pattern_to_skeleton(pattern):
    """Extract a UTS-35 skeleton (field symbols + counts, literals dropped)
    from a pattern, the way ICU's `staticGetSkeleton` does. Used to register
    a calendar's standard date/time patterns in the match table -- this is
    where non-Gregorian era-inclusive patterns come from (e.g. ja `japanese`
    `dateFormats.medium` = `Gy年M月d日` -> skeleton `GyMMMd`)."""
    out = []
    i, n, in_quote = 0, len(pattern), False
    while i < n:
        ch = pattern[i]
        if ch == "'":
            if i + 1 < n and pattern[i + 1] == "'":
                i += 2
                continue
            in_quote = not in_quote
            i += 1
            continue
        if not in_quote and ch.isascii() and ch.isalpha():
            j = i
            while j < n and pattern[j] == ch:
                j += 1
            out.append(ch * (j - i))
            i = j
        else:
            i += 1
    return "".join(out)


# UTS-35 field categories in ICU's `Canonical_Items` order
# ("GyQMwWEDFdaHmsSv"), used for canonical/base skeletons and the runtime
# type-array distance.
_UDATPG_CATEGORY = {
    "G": 0,
    "y": 1, "Y": 1, "u": 1, "r": 1,
    "Q": 2,
    "M": 3, "L": 3, "l": 3,
    "w": 4,
    "W": 5,
    "E": 6, "c": 6, "e": 6,
    "D": 7,
    "F": 8,
    "d": 9, "g": 9,
    "a": 10, "b": 10, "B": 10,
    "H": 11, "h": 11, "K": 11, "k": 11, "J": 11, "C": 11,
    "m": 12,
    "s": 13, "A": 13,
    "S": 14,
    "z": 15, "Z": 15, "O": 15, "v": 15, "V": 15, "x": 15, "X": 15,
}


def _dt_min_len(ch, length):
    """ICU `dtTypes` `minLen` for the row matching `ch` at `length`: text
    fields keep their width bracket's minimum, numeric fields collapse to 1.
    This is what `PtnSkeleton::baseOriginal` (and thus `getBasePattern`)
    stores, so `MMMM` -> 4 but `MM` -> 1."""
    if ch in "ML":
        return length if length >= 3 else 1
    if ch in "GzZEce":
        return length if length >= 4 else 1
    if ch in "abB":
        return length if length >= 4 else 1
    if ch in "vVO":
        return length if length >= 4 else 1
    if ch in "xX":
        return length if length >= 4 else 1
    return 1


def _parse_runs(skeleton):
    """Split a skeleton into `[(category, char, count)]`, or `None` for a
    field this crate can't represent (`Q`/`w`/`W`/...)."""
    runs = []
    i, n = 0, len(skeleton)
    while i < n:
        ch = skeleton[i]
        if ch not in _UDATPG_CATEGORY:
            return None
        j = i
        while j < n and skeleton[j] == ch:
            j += 1
        runs.append((_UDATPG_CATEGORY[ch], ch, j - i))
        i = j
    return runs


def canonical_skeleton(skeleton):
    """Sorted (category-order) skeleton with actual lengths, or `None`."""
    runs = _parse_runs(skeleton)
    if runs is None:
        return None
    runs.sort()
    return "".join(ch * count for _, ch, count in runs)


def base_skeleton(skeleton):
    """ICU `getBasePattern`: category-order, text widths kept at their
    bracket minimum, numeric widths collapsed to 1."""
    runs = _parse_runs(skeleton)
    if runs is None:
        return None
    runs.sort()
    return "".join(ch * _dt_min_len(ch, count) for _, ch, count in runs)


def candidate_formats(cal):
    """The full match table for a calendar, mirroring ICU's `PatternMap`
    construction (`addCanonicalItems` + `addICUPatterns` + `addCLDRData`):

    - canonical single-field items first (pattern = the char);
    - standard `dateFormats`/`timeFormats` (full/long/medium/short) added
      with `override=false`, so only the *first* pattern for a given
      `base_skeleton` survives. The base keeps text widths (`MMMM` -> 4)
      but collapses numeric widths (`MM` -> 1), which is why fr's short
      `dd/MM/y` (base `yMd`, distinct from long `d MMMM y`'s `yMMMMd`)
      survives while cs's short `dd.MM.yy` (base `yMd`, same as medium
      `d. M. y`) does not;
    - `availableFormats` last, overwriting the value of an existing
      `(base, skeleton)` entry in place.

    Deliberately does *not* inherit Gregorian `availableFormats`: ICU's
    non-Gregorian calendars only carry era-prefixed skeletons, and their own
    standard date patterns are what produce e.g. Thai Buddhist's era-less
    `d MMM y`."""
    out = {}
    bases = set()

    def add_standard(pattern):
        skeleton = canonical_skeleton(pattern_to_skeleton(pattern))
        if skeleton is None:
            return
        base = base_skeleton(skeleton)
        if base in bases:
            return  # override=false: first pattern per base wins
        bases.add(base)
        out.setdefault(skeleton, pattern)

    def add_available(skeleton_key, pattern):
        canonical = canonical_skeleton(skeleton_key)
        if canonical is None:
            return
        out[canonical] = pattern  # overwrite value in place if duplicate
        bases.add(base_skeleton(canonical))

    for ch in "GyQMwWEDFdaHmsSv":
        out.setdefault(ch, ch)
        bases.add(ch)
    for section in ("dateFormats", "timeFormats"):
        for key in ("full", "long", "medium", "short"):
            add_standard(val(cal[section][key]))
    for skeleton, pattern in available_formats(cal).items():
        add_available(skeleton, pattern)
    return out


def _bucket(skeleton):
    """ICU `PatternMap` bucket index (A-Z then a-z) of a skeleton's base."""
    base = base_skeleton(skeleton) or skeleton
    c = base[0]
    if "A" <= c <= "Z":
        return ord(c) - ord("A")
    return 26 + ord(c) - ord("a")


def glue(cal):
    at = cal["dateTimeFormats-atTime"]["standard"]
    return {k: val(at[k]) for k in ("full", "long", "medium", "short")}


def preferred_hour_cycle(cal):
    """Locale's default hour cycle, from its time skeletons (same source
    `icu_provider_source`'s `preferred_hour_cycle` reads)."""
    for key in ("full", "long", "medium", "short"):
        pattern = val(cal["timeSkeletons"][key])
        for ch in pattern:
            if ch == "K":
                return "h11"
            if ch == "h":
                return "h12"
            if ch == "H":
                return "h23"
            if ch == "k":
                return "h24"
    return "h23"


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    src_path, out_path = sys.argv[1], sys.argv[2]
    cldr = Cldr(src_path)
    locales = curated_locales()

    entries = []
    hour_cycles = {}
    for calendar, stem in CALENDARS.items():
        for tag in locales:
            locale_dir = LOCALE_DIR_FALLBACK.get(tag, tag)
            cal = cldr.calendar(stem, locale_dir)
            if cal is None:
                print(f"  skip {calendar}/{tag}: no ca-{stem}.json", file=sys.stderr)
                continue
            if tag not in hour_cycles:
                gregorian = cldr.calendar("gregorian", locale_dir)
                hour_cycles[tag] = preferred_hour_cycle(gregorian or cal)
            formats = sorted(candidate_formats(cal).items(), key=lambda kv: _bucket(kv[0]))
            entries.append((calendar, tag, formats, glue(cal)))

    with open(out_path, "w") as f:
        f.write("// @generated by tools/gen_datetime_skeletons.py -- do not edit.\n")
        f.write("//\n")
        f.write("// CLDR `availableFormats` (skeleton -> pattern) and `dateTimeFormats-atTime`\n")
        f.write("// glue patterns for the curated locales, used by `thaw-quickjs`'s runtime\n")
        f.write("// UTS-35 skeleton matching for `Intl.DateTimeFormat`.\n\n")
        f.write("#[allow(clippy::type_complexity)]\n")
        f.write("pub fn datetime_skeletons(\n")
        f.write("    calendar: &str,\n")
        f.write("    locale: &str,\n")
        f.write(") -> Option<(\n")
        f.write("    &'static [(&'static str, &'static str)],\n")
        f.write("    &'static [&'static str],\n")
        f.write(")> {\n")
        f.write("    match (calendar, locale) {\n")
        for calendar, tag, formats, gl in entries:
            fmt = ", ".join(f"({rust_str(k)}, {rust_str(v)})" for k, v in formats)
            g = ", ".join(rust_str(gl[k]) for k in ("full", "long", "medium", "short"))
            f.write(f'        ("{calendar}", {rust_str(tag)}) => Some((&[{fmt}], &[{g}])),\n')
        f.write("        _ => None,\n")
        f.write("    }\n")
        f.write("}\n")
        f.write("\n")
        f.write("/// The locale's default hour cycle, as an ECMA-402 `hourCycle` value.\n")
        f.write("pub fn preferred_hour_cycle(locale: &str) -> Option<&'static str> {\n")
        f.write("    match locale {\n")
        for tag in locales:
            if tag in hour_cycles:
                f.write(f'        {rust_str(tag)} => Some("{hour_cycles[tag]}"),\n')
        f.write("        _ => None,\n")
        f.write("    }\n")
        f.write("}\n")

    print(f"wrote {out_path}: {len(entries)} (calendar, locale) tables", file=sys.stderr)


if __name__ == "__main__":
    main()
