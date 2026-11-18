"""Selftest for run.py. Run: python3 -m unittest discover -s tools/jit-coverage -p 'test_*.py'

The yup default profile is the real output of the counters PR (profile-bench run). The other fixtures
are hand-written in the same line formats; their numbers are illustrative, not measurements.
"""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run  # noqa: E402

FIXTURES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "testdata")


def read(name):
    with open(os.path.join(FIXTURES, name), encoding="utf-8") as handle:
        return handle.read()


class ParseTests(unittest.TestCase):
    def test_profile_rows_sorted_and_total(self):
        profile = run.parse_profile(read("yup.default.profile"))
        self.assertEqual(profile["entries"][0], ("entry:yup@host.rs:1627", 15072, 1161.148))
        self.assertEqual(len(profile["entries"]), 9)
        self.assertEqual(profile["total"], (1172.396, 1363.048))
        self.assertEqual(profile["jit"], [])

    def test_profile_ignores_other_stderr_lines(self):
        text = "panic noise\nthaw-js-profile\tonly-two-fields\t3\nthaw-js-profile-total\tx\ty\n" + read("ajv.default.profile")
        profile = run.parse_profile(text)
        self.assertEqual(profile["entries"], [])
        self.assertEqual(profile["total"], (0.0, 12.5))
        self.assertEqual(profile["jit"], [("expr:a0,ld", 4)])

    def test_inspect_counts_clones_and_fallbacks(self):
        counts = run.parse_inspect(read("yup.infer.inspect"))
        self.assertEqual(counts, {"clones": 1, "clones_direct": 1, "clones_guarded": 1, "clones_rejected": 1,
                                  "quickjs_reasons": 1, "fallback_reasons": 1})
        default = run.parse_inspect(read("yup.default.inspect"))
        self.assertEqual(default["clones"], 0)
        self.assertEqual(default["fallback_reasons"], 2)

    def test_shim_counts_distinct_jit_symbols(self):
        # two arities of one JIT export plus one typed-js declaration: 1 distinct JIT export
        self.assertEqual(run.parse_shim(read("yup.default.shim.ts")), 1)

    def test_meta_last_value_wins(self):
        self.assertEqual(run.parse_meta("build_ok=0\nbuild_ok=1\nx\n"), {"build_ok": "1"})


class RenderTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tsv, cls.md = run.render(FIXTURES)
        cls.rows = [line.split("\t") for line in cls.tsv.splitlines()]
        cls.header = cls.rows[0]
        cls.by = {(r[0], r[1]): dict(zip(cls.header, r)) for r in cls.rows[1:]}

    def test_one_row_per_program_and_variant(self):
        self.assertEqual(len(self.rows) - 1, 6)  # ajv, ms, yup x (default, infer)
        self.assertEqual(sorted(self.by), sorted([(p, v) for p in ("ajv", "ms", "yup") for v in ("default", "infer")]))
        for row in self.rows:
            self.assertEqual(len(row), len(run.TSV_COLUMNS))

    def test_yup_default_row(self):
        row = self.by[("yup", "default")]
        self.assertEqual(row["build_ok"], "1")
        self.assertEqual(row["stdout_ok"], "1")
        self.assertEqual(row["entry_rows"], "5")
        self.assertEqual(row["typed_rows"], "4")
        self.assertEqual(row["entry_calls"], "24110")
        self.assertEqual(row["typed_calls"], "5")
        self.assertEqual(row["top_key"], "entry:yup@host.rs:1627")
        self.assertEqual(row["top_calls"], "15072")
        self.assertEqual(row["top_ms"], "1161.148")
        self.assertEqual(row["interp_share"], "0.8601")
        self.assertEqual(row["top_share"], "0.8519")
        self.assertEqual(row["jit_exports"], "1")
        self.assertEqual(row["jit_calls"], "0")
        self.assertEqual(row["clones"], "0")

    def test_yup_infer_row_has_clones_and_jit_calls(self):
        row = self.by[("yup", "infer")]
        self.assertEqual(row["entry_rows"], "3")
        self.assertEqual(row["typed_rows"], "2")
        self.assertEqual(row["clones"], "1")
        self.assertEqual(row["clones_direct"], "1")
        self.assertEqual(row["jit_calls"], "3002")
        self.assertEqual(row["interp_share"], "0.6764")

    def test_failed_build_row_is_blank(self):
        row = self.by[("ms", "default")]
        self.assertEqual(row["build_ok"], "0")
        self.assertEqual(row["interp_share"], "")
        self.assertEqual(row["top_key"], "")

    def test_zero_profile_is_share_zero_and_no_entry_rows(self):
        row = self.by[("ajv", "default")]
        self.assertEqual(row["interp_share"], "0.0000")
        self.assertEqual(row["entry_rows"], "0")
        self.assertEqual(row["top_key"], "")


class SummaryTests(unittest.TestCase):
    def test_corpus_numbers(self):
        rows = [run.variant_row(FIXTURES, name, variant)
                for name in run.programs(FIXTURES) for variant in run.VARIANTS]
        summary = run.summarize(rows)
        self.assertEqual(run.programs(FIXTURES), ["ajv", "ms", "yup"])
        d = summary["default"]
        self.assertEqual((d["programs"], d["built"], d["stdout_ok"]), (3, 2, 2))
        self.assertEqual(d["under_share"], ["ajv"])  # yup is 86%, not under 40%
        self.assertEqual(d["zero_entry"], ["ajv"])
        self.assertEqual(d["failed"], ["ms"])
        i = summary["infer"]
        self.assertEqual((i["built"], i["under_share"], i["zero_entry"]), (1, [], []))
        self.assertEqual(i["clones_total"], 1)
        self.assertEqual(i["programs_with_clones"], ["yup"])
        self.assertEqual(i["failed"], ["ajv", "ms"])

    def test_markdown_summary(self):
        _, md = run.render(FIXTURES)
        self.assertIn("| yup | 0.860", md)
        self.assertIn("| ms | build failed |", md)
        self.assertIn("interpreter share under 40%: 1 (ajv)", md)
        self.assertIn("zero entry rows in the profiled run: 1 (ajv)", md)
        self.assertIn("AOT clones: 1 in 1 programs (yup)", md)
        self.assertTrue(md.isascii())  # no emoji or other non-ASCII in the summary


if __name__ == "__main__":
    unittest.main()
