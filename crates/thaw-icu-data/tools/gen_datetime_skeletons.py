#!/usr/bin/env python3
"""Generate `src/datetime_skeletons_data.rs` from CLDR JSON.

Vendors the CLDR `dateTimeFormats.availableFormats` skeleton table and the
`dateTimeFormats-atTime` date/time glue patterns for the curated locale list,
so `thaw-quickjs`'s `Intl.DateTimeFormat` can run UTS-35 skeleton matching at
runtime (matching Node/ICU4C) instead of icu4x's length-pattern formatting.

Non-Gregorian calendars inherit their date patterns from the Gregorian
calendar of the same locale (per CLDR calendar inheritance), so each
non-Gregorian table is the Gregorian table overlaid with the calendar's own
`availableFormats`/glue.

Usage:
    gen_datetime_skeletons.py <cldr-json-full.zip | extracted-root> <out.rs>

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


def candidate_formats(cal):
    """The full match table for a calendar: its standard date/time patterns
    (by derived skeleton) overridden by its `availableFormats`. Mirrors
    ICU's `addICUPatterns` (standard patterns) + `addCLDRData`
    (`availableFormats`, which override duplicates). Deliberately does *not*
    inherit Gregorian `availableFormats`: ICU's non-Gregorian calendars only
    carry era-prefixed skeletons, and their own standard date patterns are
    what produce e.g. Thai Buddhist's era-less `d MMM y`."""
    out = {}
    for section in ("dateFormats", "timeFormats"):
        for key in ("full", "long", "medium", "short"):
            pattern = val(cal[section][key])
            skeleton = pattern_to_skeleton(pattern)
            if skeleton:
                out.setdefault(skeleton, pattern)
    out.update(available_formats(cal))
    return out


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
            entries.append((calendar, tag, sorted(candidate_formats(cal).items()), glue(cal)))

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
