"""Pure self-test for tools/aotdiff/run.py (no thaw, no node needed).

Run: python3 -m unittest discover -s tools/aotdiff -p 'test_*.py'
"""
import contextlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run  # noqa: E402


def probe(name):
    return (name, f'  p("{name}", () => 1);')


class BisectTest(unittest.TestCase):
    def test_rejects_found_and_rest_compiled_in_order(self):
        probes = [probe(f"p{i}") for i in range(40)]
        bad = {"p3", "p17", "p39"}
        calls = []

        def build(ps):
            calls.append(len(ps))
            names = {n for n, _ in ps}
            if names & bad:
                return False, f"error: {sorted(names & bad)[0]} rejected", None
            return True, "", "bin-" + ps[0][0]

        groups, rejects = run.bisect_compile(probes, build)
        self.assertEqual({n for n, _ in rejects}, bad)
        self.assertEqual([r[1] for r in rejects], [f"error: {n} rejected" for n, _ in rejects])
        good = [n for g, _ in groups for n, _ in g]
        self.assertEqual(good, [n for n, _ in probes if n not in bad])  # order kept, no loss
        self.assertLess(len(calls), 40 * 6)  # bisect is ~log2 per bad probe, not linear

    def test_all_good_is_one_build(self):
        probes = [probe(f"q{i}") for i in range(40)]
        groups, rejects = run.bisect_compile(probes, lambda ps: (True, "", "b"))
        self.assertEqual((len(groups), rejects), (1, []))

    def test_error_line_extraction(self):
        self.assertEqual(run.first_error("noise\nerror: nope here\nlast"), "error: nope here")
        self.assertEqual(run.first_error("only last line\n"), "only last line")
        self.assertEqual(run.first_error(""), "build failed")


class OutputTest(unittest.TestCase):
    def test_split_groups_continuations_and_ignores_unknown(self):
        text = (
            "leading noise, before any probe\n"      # dropped: no owner yet
            "a => 1\n"
            "b => {\n  x: 2,\n  y: 3\n}\n"          # multi-line value belongs to b
            "stray line from thaw\n"                 # unknown line attaches to b: a diff, not dropped
            "zz THROW TypeError boom\n"              # unknown name: also attaches to the block above
            "c THROW RangeError bad   \n"            # trailing space normalised
        )
        out = run.split_outputs(text, {"a", "b", "c"})
        self.assertEqual(out["a"], "a => 1")
        self.assertEqual(out["b"], "b => {\n  x: 2,\n  y: 3\n}\nstray line from thaw\nzz THROW TypeError boom")
        self.assertEqual(out["c"], "c THROW RangeError bad")
        self.assertNotIn("leading", "".join(out.values()))

    def test_normalise_strips_trailing_blank_and_crlf(self):
        self.assertEqual(run.normalise("x  \r\ny\n\n\n"), "x\ny")


class VerdictTest(unittest.TestCase):
    def test_outcomes(self):
        self.assertEqual(run.verdict("a => 1", "a => 1", 0), (run.MATCH, "", ""))
        self.assertEqual(run.verdict("a => 1", "a => 2", 0)[0], run.MISMATCH)
        self.assertEqual(run.verdict("a => 1", None, 0)[1], "no output line")
        self.assertEqual(run.verdict("a => 1", "a => 1", 139)[1], "exit code 139 (node 0)")

    def test_kind_splits_type_from_message_wording(self):
        self.assertEqual(run.verdict("e THROW TypeError x", "e THROW RangeError x", 0)[2], run.TYPE)
        self.assertEqual(run.verdict("e THROW TypeError x", "e THROW TypeError y", 0)[2], run.MSG)
        self.assertEqual(run.verdict("a => 1", "a => 2", 0)[2], run.OUTPUT)
        self.assertEqual(run.verdict("e THROW TypeError x", "e => 1", 0)[2], run.TYPE)
        self.assertEqual(run.verdict("a => 1", "a => 1", 139)[2], run.EXIT)
        self.assertEqual(run.verdict("a => 1", None, 0)[2], run.NO_OUTPUT)


class NormaliseTest(unittest.TestCase):
    def test_absolute_paths_become_placeholder_but_regex_literals_survive(self):
        self.assertEqual(run.normalise("at /home/fedora/work/x.ts"), "at <path>")
        self.assertEqual(run.normalise("/ab/g"), "/ab/g")

    def test_null_prototype_prefix_dropped(self):
        self.assertEqual(run.normalise("[Object: null prototype] { a: 1 }"), "{ a: 1 }")
        self.assertEqual(run.normalise("{ a: 1 }"), "{ a: 1 }")

    def test_timing_and_address(self):
        self.assertEqual(run.normalise("t => 12.5ms"), "t => <ms>")
        self.assertEqual(run.normalise("o => [Object 0x7f3a2b1c4d]"), "o => [Object <addr>]")

    def test_handle_id_is_not_normalised(self):
        s = "__thaw_js_handle_id__: 42"
        self.assertEqual(run.normalise(s), s)

    def test_stack_frames_after_first_line_collapse(self):
        t = "e THROW Error boom\n    at f (/a/b/c.ts:1:2)\n    at g (/a/b/c.ts:3:4)\n    at main"
        self.assertEqual(run.normalise(t), "e THROW Error boom\n    at <frame>")

    def test_header_and_non_frame_lines_untouched(self):
        self.assertEqual(run.normalise("at => 1"), "at => 1")
        self.assertEqual(run.normalise("x => {\n  at: 3\n}"), "x => {\n  at: 3\n}")


FIXTURE_MIS = {"class": "MISMATCH", "kind": "OUTPUT", "reason": "output differs"}
FIXTURE_REJ = {"class": "REJECT", "reason": "error: unsupported"}
FIXTURE_WAIVER = {"id": "W1", "reason": "by design: no eval"}
FIXTURE = {"numeric/a": FIXTURE_MIS, "numeric/b": FIXTURE_REJ}


class BaselineTest(unittest.TestCase):
    def test_validate_rejects_bad_entries(self):
        bad = [
            {"numeric/x": {"class": "FAIL", "reason": "r"}},
            {"numeric/x": {"class": "MISMATCH", "kind": "BOGUS", "reason": "r"}},
            {"numeric/x": {"class": "REJECT", "reason": ""}},
            {"numeric/x": {"class": "REJECT", "reason": "r", "waiver": {"id": "X1", "reason": "r"}}},
            {"numeric/x": {"class": "REJECT", "reason": "r", "waiver": {"id": "W1", "reason": ""}}},
        ]
        for gaps in bad:
            with self.subTest(gaps=gaps), self.assertRaises(run.HarnessError):
                run.validate_gaps(gaps)
        run.validate_gaps(FIXTURE)

    def test_round_trip_and_schema(self):
        path = Path(tempfile.mkdtemp()) / "known-gaps.json"
        run.write_baseline(path, FIXTURE, "thaw-test", "node-test")
        self.assertEqual(run.load_baseline(path), FIXTURE)
        self.assertEqual(json.loads(path.read_text())["schema"], 1)

    def test_merge_keeps_other_families_and_waiver(self):
        old = {"numeric/a": dict(FIXTURE_MIS, waiver=FIXTURE_WAIVER), "json/z": FIXTURE_MIS}
        cur = {"numeric/a": dict(FIXTURE_MIS, kind="MSG"), "numeric/c": FIXTURE_REJ}
        m = run.merge_baseline(old, cur, {"numeric"})
        self.assertEqual(set(m), {"numeric/a", "numeric/c", "json/z"})
        self.assertEqual(m["numeric/a"]["waiver"], FIXTURE_WAIVER)
        self.assertEqual(m["numeric/a"]["kind"], "MSG")
        self.assertNotIn("waiver", m["numeric/c"])

    def test_waiver_dropped_when_class_changes(self):
        old = {"numeric/a": dict(FIXTURE_REJ, waiver=FIXTURE_WAIVER)}
        m = run.merge_baseline(old, {"numeric/a": FIXTURE_MIS}, {"numeric"})
        self.assertNotIn("waiver", m["numeric/a"])


def cur_of(*pairs):
    return dict(pairs)


class GateTest(unittest.TestCase):
    def test_new_mismatch_fails(self):
        g = run.gate({}, {"numeric/a": FIXTURE_MIS}, {"numeric"})
        self.assertFalse(g["pass"])
        self.assertEqual(g["new_mismatch"], ["numeric/a"])

    def test_new_reject_does_not_gate(self):
        g = run.gate({}, {"numeric/b": FIXTURE_REJ}, {"numeric"})
        self.assertTrue(g["pass"])
        self.assertEqual(g["new_reject"], ["numeric/b"])

    def test_fixed_gap_passes_and_is_listed(self):
        g = run.gate(FIXTURE, {}, {"numeric"})
        self.assertTrue(g["pass"])
        self.assertEqual([i for i, _, _ in g["fixed"]], ["numeric/a", "numeric/b"])

    def test_waived_baseline_mismatch_does_not_count_open(self):
        base = {"numeric/a": dict(FIXTURE_MIS, waiver=FIXTURE_WAIVER)}
        g = run.gate(base, {"numeric/a": FIXTURE_MIS}, {"numeric"})
        self.assertTrue(g["pass"])
        self.assertEqual((g["open_mismatch"], g["waived"]), (0, 1))

    def test_known_mismatch_still_passes_and_counts_open(self):
        g = run.gate({"numeric/a": FIXTURE_MIS}, {"numeric/a": FIXTURE_MIS}, {"numeric"})
        self.assertTrue(g["pass"])
        self.assertEqual(g["open_mismatch"], 1)

    def test_reject_turning_into_mismatch_is_new(self):
        g = run.gate({"numeric/a": FIXTURE_REJ}, {"numeric/a": FIXTURE_MIS}, {"numeric"})
        self.assertFalse(g["pass"])
        self.assertEqual(g["new_mismatch"], ["numeric/a"])

    def test_unrun_family_is_ignored(self):
        g = run.gate({"json/z": FIXTURE_MIS}, {}, {"numeric"})
        self.assertTrue(g["pass"])
        self.assertEqual(g["fixed"], [])


class CliTest(unittest.TestCase):
    def test_check_and_update_are_exclusive(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            run.main(["--check", "--update-baseline"])




class ParseTest(unittest.TestCase):
    def write(self, body):
        d = tempfile.mkdtemp()
        p = Path(d) / "fam.ts"
        p.write_text("// hdr\nfunction p(): void {}\nfunction main(): void {\n" + body + "}\n")
        return p

    def test_accepts_one_line_probes_and_renders_node_copy(self):
        p = self.write('  p("a", () => 1);\n  p("b", () => 2);\n')
        fam = run.parse_family(p)
        self.assertEqual(fam.names, ["a", "b"])
        self.assertNotIn("main();", fam.render(fam.probes).split("\n"))
        self.assertTrue(fam.render(fam.probes, node=True).rstrip("\n").endswith("main();"))

    def test_rejects_multiline_probe(self):
        p = self.write('  p("a", () => {\n    return 1;\n  });\n')
        with self.assertRaises(run.HarnessError):
            run.parse_family(p)

    def test_rejects_duplicate_name(self):
        p = self.write('  p("a", () => 1);\n  p("a", () => 2);\n')
        with self.assertRaises(run.HarnessError):
            run.parse_family(p)

    def test_seed_families_parse(self):
        fams = run.load_families(set())
        total = sum(len(f.probes) for f in fams)
        self.assertEqual(len(fams), 6)
        self.assertEqual(total, 191)


class ChunkAttributionTest(unittest.TestCase):
    """A crashing batch is rerun per probe, so only the crashing probe is MISMATCH."""

    def test_crash_reruns_per_probe(self):
        fam = run.Family("fake", ["// hdr"], [probe("ok1"), probe("boom"), probe("ok2")])

        class FakeRunner(run.Runner):
            def __init__(self):
                self.work = Path(tempfile.mkdtemp())
                self.systemd = False
                self.tmpdir = tempfile.gettempdir()

            def build_fn(self, fam):
                def build(ps):
                    return True, "", ",".join(n for n, _ in ps)
                return build

            def run_bin(self, binary):
                names = binary.split(",")
                if "boom" in names:
                    if len(names) > 1:
                        return 134, "ok1 => 1\n", ""
                    return 134, "", "abort"
                return 0, "".join(f"{n} => 1\n" for n in names), ""

        r = FakeRunner()
        node = {"ok1": "ok1 => 1", "boom": "boom => 1", "ok2": "ok2 => 1"}
        res = {e["name"]: e for e in r.chunk_results(fam, fam.probes, node)}
        self.assertEqual(res["ok1"]["outcome"], run.MATCH)
        self.assertEqual(res["ok2"]["outcome"], run.MATCH)
        self.assertEqual(res["boom"]["outcome"], run.MISMATCH)
        self.assertEqual(res["boom"]["reason"], "exit code 134 (node 0)")


if __name__ == "__main__":
    unittest.main()
