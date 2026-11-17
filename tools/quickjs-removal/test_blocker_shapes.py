"""Selftest for blocker_shapes.py. Run: python3 -m unittest discover -s tools/quickjs-removal -p 'test_*.py'"""

import json
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import blocker_shapes as bs  # noqa: E402


def sym(key):
    """The `__thaw_typed_js_<hex>__arity_1` symbol the shim gives an operation key."""
    return "declare function __thaw_typed_js_" + key.encode().hex() + "__arity_1"


METHOD = "$method$x::Y$run$overload0$arity1"
NEW = "$new$node:stream::Writable$arity1"
WRITABLE = (
    "class `Writable`: " + sym(NEW)
    + "(options: { highWaterMark: number | undefined; write: ((arg0: JsValue, arg1: string, arg2: () => void) => void) | undefined }): JsValue;"
)


def classify(why, arities=None):
    return bs.classify("pkg", why, arities)


class ShapeTests(unittest.TestCase):
    def test_s2_callback_nested_in_options_class_op(self):
        row = classify(WRITABLE)
        self.assertEqual((row["shape"], row["reason"], row["cause"], row["type_word"]),
                         ("S2", "class op", "callback", "-"))
        self.assertEqual(row["name"], "Writable")

    def test_s1_top_level_callback_param(self):
        row = classify("`run`: " + sym(METHOD) + "(f: (x: number) => number): number;")
        self.assertEqual((row["shape"], row["reason"]), ("S1", "callback"))

    def test_s1_node_style_callback_param(self):
        row = classify("`read`: " + sym(METHOD) + "(path: string, cb: (err: Json, v: Json) => void): void;")
        self.assertEqual((row["shape"], row["reason"]), ("S1", "callback"))

    def test_s6_generic_with_callback(self):
        row = classify("`map`: " + sym(METHOD) + "<T>(f: (x: T) => T): T;")
        self.assertEqual((row["shape"], row["reason"]), ("S6", "callback"))

    def test_s4_promise_of_class(self):
        row = classify("`open`: " + sym(METHOD) + "(path: string): Promise<Handle>;")
        self.assertEqual((row["shape"], row["reason"], row["type_word"]), ("S4", "type word", "Handle"))

    def test_s3_builtin_class_param(self):
        row = classify("`write`: " + sym(METHOD) + "(b: Buffer): string;")
        self.assertEqual((row["shape"], row["reason"], row["type_word"]), ("S3", "type word", "Buffer"))

    def test_s3_class_op_with_class_word(self):
        row = classify("class `Y`: " + sym(METHOD) + "(other: Stream): void;")
        self.assertEqual((row["shape"], row["reason"], row["cause"], row["type_word"]),
                         ("S3", "class op", "type word", "Stream"))

    def test_s5_constructor_with_several_arities(self):
        arities = {("pkg", "Db"): {"$new$pkg::Db$arity1", "$new$pkg::Db$arity2"}}
        row = classify("class `Db`: " + sym("$new$pkg::Db$arity2") + "(a: Stream): JsValue;", arities)
        self.assertEqual(row["shape"], "S5")

    def test_plain_declaration_is_other(self):
        self.assertEqual(classify("`f`: " + sym(METHOD) + "(a: string): number;")["shape"], "other")


class ReasonTests(unittest.TestCase):
    def test_no_declaration(self):
        self.assertEqual(classify("`fetch` has no declaration")["reason"], "no declaration")

    def test_no_typed_target(self):
        self.assertEqual(classify("`fetch` has no typed target")["reason"], "no typed target")

    def test_bundle_scan(self):
        row = classify("bundle uses unsupported features: eval, with")
        self.assertEqual(row["reason"], "bundle scan")

    def test_unknown_format_goes_to_other(self):
        row = classify("native createServer together with global fetch is not supported on thaw-js yet")
        self.assertEqual((row["shape"], row["reason"]), ("other", "other"))


class ParseTests(unittest.TestCase):
    def test_inspect_line(self):
        pairs = bs.parse_records("thaw-js-blocker: package=node:http why=`createServer` has no typed target\n")
        self.assertEqual(pairs, [("node:http", "`createServer` has no typed target")])

    def test_manifest_line(self):
        text = 'THAW_ARTIFACT_V1:{"packages":[],"thaw_js_blockers":[{"package":"<build>","why":"w"}]}'
        self.assertEqual(bs.parse_records(text), [("<build>", "w")])

    def test_stderr_routing_line(self):
        text = "thaw-js routing: package `node:stream` stays on QuickJS: " + WRITABLE + "\n"
        self.assertEqual(bs.parse_records(text), [("node:stream", WRITABLE)])

    def test_json_file(self):
        text = json.dumps({"thaw_js_blockers": [{"package": "p", "why": "`f` has no declaration"}]})
        self.assertEqual(bs.parse_records(text), [("p", "`f` has no declaration")])

    def test_collect_marks_named_case_as_corpus(self):
        with tempfile.TemporaryDirectory() as root:
            case = os.path.join(root, "case-01")
            os.makedirs(case)
            with open(os.path.join(case, "name.txt"), "w") as handle:
                handle.write("writable case\n")
            with open(os.path.join(case, "inspect.txt"), "w") as handle:
                handle.write("thaw-js-blocker: package=node:stream why=" + WRITABLE + "\n")
            records, scanned = bs.collect([root])
            rows = bs.build_rows(records)
            self.assertEqual(scanned, 2)  # name.txt and inspect.txt
            self.assertEqual(rows[0]["first_case"], "writable case")
            self.assertTrue(rows[0]["corpus"])
            self.assertEqual(rows[0]["case_count"], 1)


if __name__ == "__main__":
    unittest.main()
