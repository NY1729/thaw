"""Pure self-test for tools/aotdiff/run.py (no thaw, no node needed).

Run: python3 -m unittest discover -s tools/aotdiff -p 'test_*.py'
"""
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
        self.assertEqual(run.verdict("a => 1", "a => 1", 0), (run.MATCH, ""))
        self.assertEqual(run.verdict("a => 1", "a => 2", 0)[0], run.MISMATCH)
        self.assertEqual(run.verdict("a => 1", None, 0)[1], "no output line")
        self.assertEqual(run.verdict("a => 1", "a => 1", 139)[1], "exit code 139 (node 0)")


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
