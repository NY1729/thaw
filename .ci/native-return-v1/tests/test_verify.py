"""Mutation-oriented tests for the deterministic overlay verifier.

These tests use a tiny synthetic Git checkout and Python's standard library;
they never invoke Rust, Cargo, a compiler, or a network client.
"""

from __future__ import annotations

import copy
import difflib
import hashlib
import importlib
import json
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

MODULE_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(MODULE_DIR))
try:
    verify = importlib.import_module("verify")
except ModuleNotFoundError:
    verify = None


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def write(path: Path, data: bytes | str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data.encode() if isinstance(data, str) else data)


def patch_for(path: str, old: str | None, new: str | None) -> bytes:
    """Create one plain unified diff that git apply accepts."""
    if old is None:
        before: list[str] = []
        old_name = "/dev/null"
        header = f"diff --git a/{path} b/{path}\nnew file mode 100644\n"
    else:
        before = old.splitlines(keepends=True)
        old_name = f"a/{path}"
        header = f"diff --git a/{path} b/{path}\n"
    if new is None:
        after: list[str] = []
        new_name = "/dev/null"
        header += "deleted file mode 100644\n"
    else:
        after = new.splitlines(keepends=True)
        new_name = f"b/{path}"
    chunks = list(
        difflib.unified_diff(
            before,
            after,
            fromfile=old_name,
            tofile=new_name,
            lineterm="\n",
            n=3,
        )
    )
    if not chunks:
        return b""
    return (header + "".join(chunks)).encode()


def manifest_for(tree: Path, path_order: str = "components") -> bytes:
    rows = []
    key = (lambda rel: tuple(PurePosixPath(rel).parts)) if path_order == "components" else (lambda rel: rel)
    for path in sorted(
        (p for p in tree.rglob("*") if p.is_file() and ".git" not in p.parts),
        key=lambda p: key(p.relative_to(tree).as_posix()),
    ):
        rel = path.relative_to(tree).as_posix()
        rows.append(f"{sha(path.read_bytes())}  {rel}\n")
    return "".join(rows).encode()


def create_git_repo(root: Path) -> tuple[str, str]:
    root.mkdir(parents=True, exist_ok=True)
    subprocess.run(["git", "init", "-q", str(root)], check=True)
    subprocess.run(["git", "-C", str(root), "config", "user.email", "ci-test@example.invalid"], check=True)
    subprocess.run(["git", "-C", str(root), "config", "user.name", "CI test"], check=True)
    subprocess.run(["git", "-C", str(root), "add", "-A"], check=True)
    subprocess.run(["git", "-C", str(root), "commit", "-qm", "fixture baseline"], check=True)
    commit = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
    tree = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD^{tree}"], text=True).strip()
    return commit, tree


class Fixture:
    """Tiny six-stage baseline whose patch bytes and pins are self-consistent."""

    def __init__(self, root: Path):
        self.root = root
        self.baseline = root / "baseline"
        self.payload = root / "payload"
        self.candidate = root / "candidate"
        self.evidence = root / "evidence"
        self.baseline.mkdir(parents=True)
        self.payload.mkdir(parents=True)

        initial = {
            ".gitignore": "target/\n",
            "Cargo.lock": "fixture-lock\n",
            "Cargo.toml": "[workspace]\nmembers = []\n",
            "crates/thaw-arena/Cargo.toml": "[package]\nname='thaw-arena'\n",
            "crates/thaw-hir/Cargo.toml": "[package]\nname='thaw-hir'\n",
            "crates/thaw-hir/src/lib.rs": "pub fn fixture() -> &'static str { \"base\" }\n",
            "crates/thaw-llvm/Cargo.toml": "[package]\nname='thaw-llvm'\n",
            "crates/thaw-llvm/src/hir_codegen/tests/async.rs": "// baseline async test owner\n",
            "crates/thaw-runtime/Cargo.toml": "[package]\nname='thaw-runtime'\n",
            "outside.txt": "unmodified product file\n",
        }
        for rel, text in initial.items():
            write(self.baseline / rel, text)
        repair_extra = "crates/thaw-std/src/json.rs"
        repair_extra_bytes = b"fixture json base\n"
        write(self.baseline / repair_extra, repair_extra_bytes)

        lib = "crates/thaw-hir/src/lib.rs"
        async_owner = "crates/thaw-llvm/src/hir_codegen/tests/async.rs"
        v5_control = "crates/thaw-llvm/src/hir_codegen/tests/async/v5_control.rs"
        discard_control = "crates/thaw-llvm/src/hir_codegen/tests/async/union_discard.rs"
        return_control = "crates/thaw-llvm/src/hir_codegen/tests/async/eval_then_returns.rs"
        stages: list[dict[str, bytes]] = []

        # Patch 1: the inherited native draft remains visible in this lineage.
        p1 = patch_for(lib, initial[lib], "pub fn fixture() -> &'static str { \"native\" }\n")
        write(self.baseline / "docs/codebase-refactor-handoff/drafts/native/candidate.patch", p1)
        base_owner = f"{sha(initial[lib].encode())}  {lib}\n".encode()
        handoff_manifest = {
            "drafts": {
                "native": {
                    "owners": [
                        {
                            "path": lib,
                            "base_sha256": sha(initial[lib].encode()),
                            "candidate_sha256": sha(b"pub fn fixture() -> &'static str { \"native\" }\n"),
                            "base_matches_checkpoint": True,
                        }
                    ]
                }
            }
        }
        write(
            self.baseline / "docs/codebase-refactor-handoff/manifest.json",
            json.dumps(handoff_manifest, sort_keys=True) + "\n",
        )

        stage1 = {key: value for key, value in initial.items() if key != "outside.txt"}
        stage1[lib] = "pub fn fixture() -> &'static str { \"native\" }\n"

        # Patch 2: add one v5 control and wire it immediately.
        stage2 = dict(stage1)
        stage2[lib] = "pub fn fixture() -> &'static str { \"v5\" }\n"
        stage2[async_owner] = 'include!("async/v5_control.rs");\n'
        stage2[v5_control] = "#[test]\nfn scope_control() {}\n"
        p2 = patch_for(lib, stage1[lib], stage2[lib])
        p2 += patch_for(async_owner, stage1[async_owner], stage2[async_owner])
        p2 += patch_for(v5_control, None, stage2[v5_control])
        write(
            self.baseline / "docs/codebase-refactor-handoff/drafts/native/scope-boundary-continuation-20261005/delta.patch",
            p2,
        )

        # Patch 3: add the discard control and update the include owner.
        stage3 = dict(stage2)
        stage3[lib] = "pub fn fixture() -> &'static str { \"discard\" }\n"
        stage3[async_owner] += 'include!("async/union_discard.rs");\n'
        stage3[discard_control] = "#[test]\nfn discard_control() {}\n"
        p3 = patch_for(lib, stage2[lib], stage3[lib])
        p3 += patch_for(async_owner, stage2[async_owner], stage3[async_owner])
        p3 += patch_for(discard_control, None, stage3[discard_control])
        write(self.payload / "discard-v2.patch", p3)

        # Patch 4: add the Return control and update the include owner.
        stage4 = dict(stage3)
        stage4[lib] = "pub fn fixture() -> &'static str { \"return-v1\" }\n"
        stage4[async_owner] += 'include!("async/eval_then_returns.rs");\n'
        stage4[return_control] = "#[test]\nfn return_control() {}\n"
        p4 = patch_for(lib, stage3[lib], stage4[lib])
        p4 += patch_for(async_owner, stage3[async_owner], stage4[async_owner])
        p4 += patch_for(return_control, None, stage4[return_control])
        write(self.payload / "return-v1.patch", p4)

        # Patch 5 is a single isolated repair. The thaw-std JSON owner is
        # allowed only in the paired base/final manifests, never the first
        # four stage roots.
        stage5 = dict(stage4)
        stage5[lib] = "pub fn fixture() -> &'static str { \"compile-repairs-v1\" }\n"
        repair_json_bytes = b"fixture json repaired\n"
        repair_patch = patch_for(lib, stage4[lib], stage5[lib])
        repair_patch += patch_for(repair_extra, repair_extra_bytes.decode(), repair_json_bytes.decode())
        write(self.payload / "compile-repairs-v1.patch", repair_patch)
        stage6 = dict(stage5)
        stage6[lib] = "pub fn fixture() -> &'static str { \"cumulative-hir-v1\" }\n"
        write(self.payload / "cumulative-hir-v1.patch", patch_for(lib, stage5[lib], stage6[lib]))
        paired_base = dict(stage4)
        paired_base[repair_extra] = repair_extra_bytes.decode()
        repair_final = dict(stage5)
        repair_final[repair_extra] = repair_json_bytes.decode()
        cumulative_base = dict(repair_final)
        final_stage = dict(stage6)
        final_stage[repair_extra] = repair_json_bytes.decode()
        for name, files in (
            ("compile-repairs-v1-base", paired_base),
            ("compile-repairs-v1", repair_final),
            ("cumulative-hir-v1-base", cumulative_base),
            ("cumulative-hir-v1", final_stage),
        ):
            stage_dir = root / ("manifest-tree-" + name)
            for rel, text in files.items():
                write(stage_dir / rel, text)
            (self.payload / f"{name}.sha256").write_bytes(
                manifest_for(stage_dir, "lexical" if name in ("compile-repairs-v1", "cumulative-hir-v1-base", "cumulative-hir-v1") else "components")
            )

        for name, files in zip(("original-native", "scope-v5", "discard-v2", "return-v1"), (stage1, stage2, stage3, stage4)):
            stage_dir = root / ("manifest-tree-" + name)
            for rel, text in files.items():
                write(stage_dir / rel, text)
            data = manifest_for(stage_dir)
            (self.payload / f"{name}.sha256").write_bytes(data)
            stages.append({"name": name, "file": f"{name}.sha256", "sha256": sha(data), "count": len(files)})
        final_manifest_bytes = (self.payload / "compile-repairs-v1.sha256").read_bytes()
        stages.append({
            "name": "compile-repairs-v1",
            "file": "compile-repairs-v1.sha256",
            "sha256": sha(final_manifest_bytes),
            "count": len(repair_final),
            "extra_paths": [repair_extra],
            "path_order": "lexical",
        })
        stages.append({
            "name": "cumulative-hir-v1",
            "file": "cumulative-hir-v1.sha256",
            "sha256": sha((self.payload / "cumulative-hir-v1.sha256").read_bytes()),
            "count": len(final_stage),
            "extra_paths": [repair_extra],
            "path_order": "lexical",
        })

        # Stage fixture trees are written above from text maps; preserve the
        # expected subset inventory separately for fast mutation tests.
        self.native_base = base_owner
        write(self.payload / "native-base.sha256", base_owner)
        net_rows = [
            f"modified  {lib}\n",
            f"modified  {async_owner}\n",
            f"modified  {repair_extra}\n",
            f"new  {v5_control}\n",
            f"new  {discard_control}\n",
            f"new  {return_control}\n",
        ]
        net_rows.sort(key=lambda row: row.split("  ", 1)[1])
        write(self.payload / "net-owners.txt", "".join(net_rows))
        write(self.payload / "scope-v5-test-plan.md", "fixture test plan\n")

        # Git is initialized after patches and descriptors are present so the
        # baseline is clean and contains the exact bytes the runner will read.
        self.commit, self.tree = create_git_repo(self.baseline)
        source_files = {
            "handoff_manifest": {
                "path": "docs/codebase-refactor-handoff/manifest.json",
                "sha256": sha((self.baseline / "docs/codebase-refactor-handoff/manifest.json").read_bytes()),
            },
            "original_patch": {
                "path": "docs/codebase-refactor-handoff/drafts/native/candidate.patch",
                "sha256": sha(p1),
            },
            "scope_patch": {
                "path": "docs/codebase-refactor-handoff/drafts/native/scope-boundary-continuation-20261005/delta.patch",
                "sha256": sha(p2),
            },
            "cargo_lock": {"path": "Cargo.lock", "sha256": sha((self.baseline / "Cargo.lock").read_bytes())},
        }
        payload_files = {
            name: sha((self.payload / name).read_bytes())
            for name in ("discard-v2.patch", "return-v1.patch", "compile-repairs-v1.patch", "cumulative-hir-v1.patch")
        }
        artifact_hashes = {}
        for name, desc in self._artifact_paths().items():
            artifact_hashes[desc] = sha((self.payload / desc).read_bytes())
        self.expected = {
            "schema_version": 1,
            "validation_branch": verify.EXPECTED["validation_branch"] if verify is not None else "validation/native-return",
            "baseline": {"commit": self.commit, "tree": self.tree},
            "source_files": source_files,
            "payload_patches": payload_files,
            "artifacts": artifact_hashes,
            "stage_manifests": stages,
            "native_base": {"file": "native-base.sha256", "sha256": sha(base_owner), "count": 1},
            "net_owners": {
                "file": "net-owners.txt",
                "sha256": sha((self.payload / "net-owners.txt").read_bytes()),
                "count": 6,
                "modified": 3,
                "new": 3,
            },
            "repair_base": {
                "file": "compile-repairs-v1-base.sha256",
                "sha256": sha((self.payload / "compile-repairs-v1-base.sha256").read_bytes()),
                "count": len(paired_base),
                "path_order": "components",
                "extra_owner": {
                    "path": repair_extra,
                    "sha256": sha(repair_extra_bytes),
                    "git_blob_sha1": verify._git_blob_sha1(repair_extra_bytes) if verify is not None else "",
                    "final_sha256": sha(repair_json_bytes),
                },
            },
            "cumulative_base": {
                "file": "cumulative-hir-v1-base.sha256",
                "sha256": sha((self.payload / "cumulative-hir-v1-base.sha256").read_bytes()),
                "count": len(cumulative_base),
                "path_order": "lexical",
                "paired_after": "compile-repairs-v1",
            },
            "patch_order": ["original-native", "scope-v5", "discard-v2", "return-v1", "compile-repairs-v1", "cumulative-hir-v1"],
            "subset_roots": [
                ".gitignore",
                "Cargo.lock",
                "Cargo.toml",
                "crates/thaw-arena",
                "crates/thaw-hir",
                "crates/thaw-llvm",
                "crates/thaw-runtime",
            ],
            "required_includes": {
                async_owner: [
                    "async/v5_control.rs",
                    "async/union_discard.rs",
                    "async/eval_then_returns.rs",
                ]
            },
            "control_owners": [v5_control, discard_control, return_control],
            "runner_dependencies": copy.deepcopy(verify.EXPECTED["runner_dependencies"]) if verify is not None else {},
            "test_filters": copy.deepcopy(verify.EXPECTED["test_filters"]) if verify is not None else [],
            "expected_test_names": copy.deepcopy(verify.EXPECTED.get("expected_test_names", {})) if verify is not None else {},
            "provenance_only_review_fingerprints": copy.deepcopy(verify.EXPECTED["provenance_only_review_fingerprints"]) if verify is not None else {},
        }
        write(self.payload / "pins.json", json.dumps(self.expected, sort_keys=True, indent=2) + "\n")
        self._commit = self.commit
        self._tree = self.tree

    @staticmethod
    def _artifact_paths() -> dict[str, str]:
        return {
            "original_manifest": "original-native.sha256",
            "scope_manifest": "scope-v5.sha256",
            "discard_manifest": "discard-v2.sha256",
            "return_manifest": "return-v1.sha256",
            "compile_repair_patch": "compile-repairs-v1.patch",
            "compile_repair_base": "compile-repairs-v1-base.sha256",
            "compile_repair_final": "compile-repairs-v1.sha256",
            "cumulative_hir_base": "cumulative-hir-v1-base.sha256",
            "cumulative_hir_final": "cumulative-hir-v1.sha256",
            "native_base": "native-base.sha256",
            "net_owners": "net-owners.txt",
            "scope_test_plan": "scope-v5-test-plan.md",
        }

    def activate(self):
        """Install fixture pins in the module while preserving production logic."""
        if verify is None:
            return
        self.expected_before = verify.EXPECTED
        verify.EXPECTED = copy.deepcopy(self.expected)


class VerificationRedGreenTests(unittest.TestCase):
    def require_implementation(self):
        if verify is None:
            self.fail("verify.py must implement the pinned reconstruction interfaces")
        return verify

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="thaw-verify-test-")
        self.fixture = Fixture(Path(self.temp.name))
        self.fixture.activate()

    def tearDown(self):
        if verify is not None and hasattr(self, "fixture") and hasattr(self.fixture, "expected_before"):
            verify.EXPECTED = self.fixture.expected_before
        self.temp.cleanup()

    def assertVerifyError(self, pattern: str, call, *args, **kwargs):
        mod = self.require_implementation()
        with self.assertRaisesRegex(mod.VerificationError, pattern):
            call(*args, **kwargs)

    def test_positive_reconstruction_replays_six_stages_and_preserves_unrelated_file(self):
        mod = self.require_implementation()
        result = mod.reconstruct(
            self.fixture.baseline,
            self.fixture.candidate,
            self.fixture.payload,
            self.fixture.evidence,
        )
        self.assertEqual([x["count"] for x in mod.EXPECTED["stage_manifests"]], result["stage_counts"])
        self.assertEqual(6, len(result["stages"]))
        self.assertEqual("unmodified product file\n", (self.fixture.candidate / "outside.txt").read_text())
        self.assertTrue((self.fixture.evidence / "reconstruction.json").is_file())

    def test_repair_json_owner_is_stage_scoped_and_first_four_inventories_stay_fixed(self):
        mod = self.require_implementation()
        extra = mod.EXPECTED["repair_base"]["extra_owner"]["path"]
        self.assertEqual(4, len([stage for stage in mod.EXPECTED["stage_manifests"][:4]]))
        for stage in mod.EXPECTED["stage_manifests"][:4]:
            self.assertNotIn(extra, {rel for _, rel in mod._parse_sha_manifest(self.fixture.payload / stage["file"], stage["name"])})
        self.assertEqual([extra], mod.EXPECTED["stage_manifests"][4]["extra_paths"])
        mod.reconstruct(self.fixture.baseline, self.fixture.candidate, self.fixture.payload, self.fixture.evidence)
        self.assertIn(extra, mod.verify_prepared(self.fixture.baseline, self.fixture.candidate, self.fixture.payload)["final_stage"]["files"])
        self.assertEqual(b"fixture json repaired\n", (self.fixture.candidate / extra).read_bytes())

    def test_actual_frozen_final_repair_manifest_uses_its_pinned_lexical_order(self):
        mod = self.require_implementation()
        fixture_expected = mod.EXPECTED
        production_expected = self.fixture.expected_before
        mod.EXPECTED = production_expected
        try:
            stage = next(stage for stage in production_expected["stage_manifests"] if stage["name"] == "compile-repairs-v1")
            manifest = MODULE_DIR / stage["file"]
            self.assertEqual(stage["sha256"], sha(manifest.read_bytes()))
            with self.assertRaisesRegex(mod.VerificationError, "not strictly sorted"):
                mod._parse_sha_manifest(manifest, "actual final repair manifest")
            rows = mod._parse_sha_manifest(manifest, "actual final repair manifest", path_order=stage["path_order"])
            self.assertEqual(181, len(rows))
            final_hashes = {rel: digest for digest, rel in rows}
            json_owner = production_expected["repair_base"]["extra_owner"]
            self.assertEqual(json_owner["final_sha256"], final_hashes[json_owner["path"]])
        finally:
            mod.EXPECTED = fixture_expected

    def test_sixth_cumulative_patch_changes_a_native_owner_after_the_repair_stage(self):
        mod = self.require_implementation()
        lib = "crates/thaw-hir/src/lib.rs"
        result = mod.reconstruct(self.fixture.baseline, self.fixture.candidate, self.fixture.payload, self.fixture.evidence)
        repair_bytes = (self.fixture.root / "manifest-tree-compile-repairs-v1" / lib).read_bytes()
        expected_repair_hash = next(
            digest for digest, rel in mod._parse_sha_manifest(self.fixture.payload / "compile-repairs-v1.sha256", "repair final", path_order="lexical")
            if rel == lib
        )
        self.assertEqual("cumulative-hir-v1", result["stages"][-1]["stage"])
        self.assertEqual(sha(repair_bytes), expected_repair_hash)
        self.assertEqual(b'pub fn fixture() -> &\'static str { "cumulative-hir-v1" }\n', (self.fixture.candidate / lib).read_bytes())

    def test_actual_cumulative_inputs_and_name_pins_match_the_immutable_source(self):
        mod = self.require_implementation()
        production_expected = self.fixture.expected_before
        pins_path = MODULE_DIR / "pins.json"
        pins = json.loads(pins_path.read_text(encoding="utf-8"))
        self.assertEqual(production_expected, pins)

        self.assertIn("cumulative-hir-v1", [stage["name"] for stage in production_expected["stage_manifests"]])
        cumulative_stage = next(stage for stage in production_expected["stage_manifests"] if stage["name"] == "cumulative-hir-v1")
        patch_name = "cumulative-hir-v1.patch"
        self.assertEqual("63744b75a96ce30ff503d90379b08ec20ffbebb5f5f68716972d6c7d6f17f368", production_expected["payload_patches"][patch_name])
        self.assertEqual(production_expected["payload_patches"][patch_name], sha((MODULE_DIR / patch_name).read_bytes()))
        self.assertEqual(181, cumulative_stage["count"])
        self.assertEqual(["crates/thaw-std/src/json.rs"], cumulative_stage["extra_paths"])
        self.assertEqual("lexical", cumulative_stage["path_order"])

        cumulative_base = production_expected["cumulative_base"]
        base_path = MODULE_DIR / cumulative_base["file"]
        final_path = MODULE_DIR / cumulative_stage["file"]
        repair_final = MODULE_DIR / next(stage["file"] for stage in production_expected["stage_manifests"] if stage["name"] == "compile-repairs-v1")
        self.assertEqual("d3b7c0e06ca053f4b342dcbc8ac1247a3460c946a35a2b22d2645c76251a2aa9", cumulative_base["sha256"])
        self.assertEqual(cumulative_base["sha256"], sha(base_path.read_bytes()))
        self.assertEqual(repair_final.read_bytes(), base_path.read_bytes())
        base_rows = mod._parse_sha_manifest(base_path, "actual cumulative base", path_order="lexical")
        final_rows = mod._parse_sha_manifest(final_path, "actual cumulative final", path_order="lexical")
        self.assertEqual(181, len(base_rows))
        self.assertEqual(181, len(final_rows))
        self.assertEqual(production_expected["cumulative_base"]["sha256"], sha(base_path.read_bytes()))
        self.assertEqual(cumulative_stage["sha256"], sha(final_path.read_bytes()))
        final_hashes = {rel: digest for digest, rel in final_rows}
        repair_json = production_expected["repair_base"]["extra_owner"]
        self.assertEqual(repair_json["final_sha256"], final_hashes[repair_json["path"]])

        net = production_expected["net_owners"]
        roster_path = MODULE_DIR / net["file"]
        roster_lines = roster_path.read_text(encoding="utf-8").splitlines()
        self.assertEqual(net["sha256"], sha(roster_path.read_bytes()))
        self.assertEqual("2b44f2be32a7ba49a50436af2bb3890f853a0ea5e9cca38d7abaf43c68f92c76", sha(roster_path.read_bytes()))
        self.assertEqual((53, 48, 5), (len(roster_lines), sum(line.startswith("modified  ") for line in roster_lines), sum(line.startswith("new  ") for line in roster_lines)))
        self.assertEqual((53, 48, 5), (net["count"], net["modified"], net["new"]))
        fixture_expected = mod.EXPECTED
        mod.EXPECTED = production_expected
        try:
            parsed_roster = mod._read_net_roster(MODULE_DIR)
        finally:
            mod.EXPECTED = fixture_expected
        self.assertEqual(53, len(parsed_roster))
        self.assertEqual(48, sum(status == "modified" for status in parsed_roster.values()))
        self.assertEqual(5, sum(status == "new" for status in parsed_roster.values()))

        added_filters = production_expected["test_filters"][-2:]
        self.assertEqual(27, len(production_expected["test_filters"]))
        self.assertEqual((20, 6, 1), tuple(sum(item["package"] == package for item in production_expected["test_filters"]) for package in ("thaw-llvm", "thaw-hir", "thaw-std")))
        self.assertEqual(
            [
                {"package": "thaw-hir", "filter": "thaw_binding_helper_"},
                {"package": "thaw-hir", "filter": "receiver_pattern_inference_"},
            ],
            added_filters,
        )
        exact_names = {
            "thaw_binding_helper_": [
                "thaw_binding_helper_declaration_flags_follow_resolved_symbols",
                "thaw_binding_helper_iteration_cell_inherits_only_current_outer_immutability",
            ],
            "receiver_pattern_inference_": [
                "receiver_pattern_inference_parsed_annotations_keep_pattern_and_legacy_states",
                "receiver_pattern_inference_declarations_remain_accepted_without_calls",
                "receiver_pattern_inference_source_negative_and_shape_controls",
                "receiver_pattern_inference_physical_and_split_modes_match_receiver_first",
                "receiver_pattern_inference_synthetic_and_contextual_modes_stay_separate",
                "receiver_pattern_inference_distinguishes_absent_undefined_and_legacy_receivers",
                "receiver_pattern_inference_optional_rest_indices_and_key_literals_stay_visible_only",
                "receiver_pattern_inference_implicit_fallbacks_follow_actual_match_order",
                "receiver_pattern_inference_explicit_constraints_precede_actual_mismatch",
                "receiver_pattern_inference_type_only_and_receiver_free_promise_controls",
            ],
        }
        for filter_name, expected_names in exact_names.items():
            self.assertEqual(expected_names, production_expected["expected_test_names"][filter_name])

    def test_wrong_repair_patch_hash_and_paired_base_hash_are_rejected(self):
        mod = self.require_implementation()
        repair_patch = self.fixture.payload / "compile-repairs-v1.patch"
        repair_patch.write_bytes(repair_patch.read_bytes() + b"# mutation\n")
        self.assertVerifyError("compile-repairs-v1.patch|hash|pin", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

        # Restore the patch and corrupt the independently pinned paired base.
        repair_patch.write_bytes((self.fixture.payload / "compile-repairs-v1.patch").read_bytes().removesuffix(b"# mutation\n"))
        base = self.fixture.payload / "compile-repairs-v1-base.sha256"
        base.write_bytes(base.read_bytes() + b"0" * 64 + b"  crates/thaw-hir/src/extra.rs\n")
        self.assertVerifyError("compile-repairs-v1-base.sha256|hash|pin", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_missing_paired_json_owner_is_rejected_after_reconstruction(self):
        mod = self.require_implementation()
        mod.reconstruct(self.fixture.baseline, self.fixture.candidate, self.fixture.payload, self.fixture.evidence)
        extra = mod.EXPECTED["repair_base"]["extra_owner"]["path"]
        (self.fixture.candidate / extra).unlink()
        self.assertVerifyError("missing|stage file", mod.verify_prepared, self.fixture.baseline, self.fixture.candidate, self.fixture.payload)

    def test_wrong_repair_extra_owner_or_stage_pin_is_rejected(self):
        mod = self.require_implementation()
        data = json.loads((self.fixture.payload / "pins.json").read_text())
        data["repair_base"]["extra_owner"]["path"] = "crates/thaw-hir/src/lib.rs"
        write(self.fixture.payload / "pins.json", json.dumps(data, sort_keys=True, indent=2) + "\n")
        self.assertVerifyError("descriptor|pins.json|immutable|expected", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)
        write(self.fixture.payload / "pins.json", json.dumps(self.fixture.expected, sort_keys=True, indent=2) + "\n")
        data = json.loads((self.fixture.payload / "pins.json").read_text())
        data["stage_manifests"][4]["extra_paths"] = []
        write(self.fixture.payload / "pins.json", json.dumps(data, sort_keys=True, indent=2) + "\n")
        self.assertVerifyError("descriptor|pins.json|immutable|expected", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_repair_json_base_blob_sha1_is_independently_pinned(self):
        mod = self.require_implementation()
        original = copy.deepcopy(mod.EXPECTED)
        mod.EXPECTED["repair_base"]["extra_owner"]["git_blob_sha1"] = "f" * 40
        write(self.fixture.payload / "pins.json", json.dumps(mod.EXPECTED, sort_keys=True, indent=2) + "\n")
        try:
            self.assertVerifyError("Git blob SHA-1", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)
        finally:
            mod.EXPECTED = original
            write(self.fixture.payload / "pins.json", json.dumps(self.fixture.expected, sort_keys=True, indent=2) + "\n")

    def test_repair_json_final_sha256_is_separate_from_base_pin(self):
        mod = self.require_implementation()
        original = copy.deepcopy(mod.EXPECTED)
        mod.EXPECTED["repair_base"]["extra_owner"]["final_sha256"] = "0" * 64
        write(self.fixture.payload / "pins.json", json.dumps(mod.EXPECTED, sort_keys=True, indent=2) + "\n")
        try:
            self.assertVerifyError("compile repair final manifest", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)
        finally:
            mod.EXPECTED = original
            write(self.fixture.payload / "pins.json", json.dumps(self.fixture.expected, sort_keys=True, indent=2) + "\n")

    def test_wrong_baseline_commit_is_rejected_and_recorded_before_return(self):
        mod = self.require_implementation()
        expected = copy.deepcopy(mod.EXPECTED)
        mod.EXPECTED["baseline"]["commit"] = "0" * 40
        write(self.fixture.payload / "pins.json", json.dumps(mod.EXPECTED, sort_keys=True, indent=2) + "\n")
        try:
            self.assertVerifyError("baseline.*commit|commit.*baseline", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)
        finally:
            mod.EXPECTED = expected
            write(self.fixture.payload / "pins.json", json.dumps(self.fixture.expected, sort_keys=True, indent=2) + "\n")

    def test_wrong_baseline_tree_is_rejected(self):
        mod = self.require_implementation()
        expected = copy.deepcopy(mod.EXPECTED)
        mod.EXPECTED["baseline"]["tree"] = "f" * 40
        write(self.fixture.payload / "pins.json", json.dumps(mod.EXPECTED, sort_keys=True, indent=2) + "\n")
        try:
            self.assertVerifyError("baseline.*tree|tree.*baseline", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)
        finally:
            mod.EXPECTED = expected
            write(self.fixture.payload / "pins.json", json.dumps(self.fixture.expected, sort_keys=True, indent=2) + "\n")

    def test_dirty_baseline_is_rejected(self):
        mod = self.require_implementation()
        write(self.fixture.baseline / "untracked.txt", "dirty\n")
        self.assertVerifyError("dirty|untracked|clean", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_workflow_ref_must_match_the_pinned_validation_branch(self):
        mod = self.require_implementation()
        with mock.patch.dict("os.environ", {"GITHUB_REF": "refs/heads/validation/wrong"}):
            self.assertVerifyError("workflow ref mismatch|validation branch", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_pinned_workflow_ref_is_accepted(self):
        mod = self.require_implementation()
        expected = f"refs/heads/{mod.EXPECTED['validation_branch']}"
        with mock.patch.dict("os.environ", {"GITHUB_REF": expected}):
            result = mod.verify_inputs(self.fixture.baseline, self.fixture.payload)
        self.assertEqual(self.fixture.commit, result["baseline"]["commit"])

    def test_skip_worktree_corruption_is_detected_against_git_tree_blob(self):
        mod = self.require_implementation()
        write(self.fixture.baseline / "outside.txt", "skip-worktree corruption\n")
        subprocess.run(["git", "-C", str(self.fixture.baseline), "update-index", "--skip-worktree", "outside.txt"], check=True)
        self.assertVerifyError("tracked.*mismatch|tree.*inventory|blob|outside.txt", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_ignored_extra_file_is_detected_before_copy(self):
        mod = self.require_implementation()
        write(self.fixture.baseline / "target/untracked-ignored.txt", "ignored extra\n")
        self.assertVerifyError("tracked.*mismatch|inventory.*extra|untracked-ignored", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_changed_patch_bytes_are_rejected(self):
        mod = self.require_implementation()
        with (self.fixture.payload / "discard-v2.patch").open("ab") as stream:
            stream.write(b"# mutation\n")
        self.assertVerifyError("discard-v2.patch|hash|pin", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_changed_manifest_bytes_are_rejected(self):
        mod = self.require_implementation()
        with (self.fixture.payload / "return-v1.sha256").open("ab") as stream:
            stream.write(b"0" * 64 + "  crates/thaw-hir/src/extra.rs\n".encode())
        self.assertVerifyError("return-v1.sha256|manifest|hash|pin", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_descriptor_cannot_self_authorize_a_changed_payload_pin(self):
        mod = self.require_implementation()
        data = json.loads((self.fixture.payload / "pins.json").read_text())
        data["payload_patches"]["discard-v2.patch"] = sha(b"changed")
        write(self.fixture.payload / "pins.json", json.dumps(data, sort_keys=True, indent=2) + "\n")
        self.assertVerifyError("descriptor|pins.json|immutable|expected", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_duplicate_manifest_path_is_rejected(self):
        mod = self.require_implementation()
        tree = self.fixture.root / "simple-stage"
        write(tree / "crates/thaw-hir/src/lib.rs", "one\n")
        one_digest = sha(b"one\n")
        row = f"{one_digest}  crates/thaw-hir/src/lib.rs\n"
        manifest = self.fixture.root / "duplicate.sha256"
        write(manifest, row + row)
        self.assertVerifyError("duplicate|path", mod.verify_stage, tree, manifest, 2)

    def test_absolute_manifest_path_is_rejected(self):
        mod = self.require_implementation()
        manifest = self.fixture.root / "absolute.sha256"
        write(manifest, f"{'0' * 64}  /tmp/escape\n")
        self.assertVerifyError("absolute|unsafe|path", mod.verify_stage, self.fixture.root, manifest, 1)

    def test_parent_traversal_manifest_path_is_rejected(self):
        mod = self.require_implementation()
        manifest = self.fixture.root / "traversal.sha256"
        write(manifest, f"{'0' * 64}  ../escape\n")
        self.assertVerifyError("traversal|unsafe|path", mod.verify_stage, self.fixture.root, manifest, 1)

    def test_symlink_escape_is_rejected(self):
        mod = self.require_implementation()
        tree = self.fixture.root / "symlink-stage"
        outside = self.fixture.root / "outside-secret"
        write(outside, "outside\n")
        (tree / "crates/thaw-hir/src").mkdir(parents=True)
        (tree / "crates/thaw-hir/src/lib.rs").symlink_to(outside)
        manifest = self.fixture.root / "symlink.sha256"
        write(manifest, f"{sha(outside.read_bytes())}  crates/thaw-hir/src/lib.rs\n")
        self.assertVerifyError("symlink|escape|regular", mod.verify_stage, tree, manifest, 1)

    def test_wrong_stage_count_is_rejected(self):
        mod = self.require_implementation()
        data = self.fixture.payload / "original-native.sha256"
        self.assertVerifyError("count|expected", mod.verify_stage, self.fixture.baseline, data, 999)

    def test_native_base_owner_manifest_uses_its_pinned_lexical_order(self):
        mod = self.require_implementation()
        fixture_expected = mod.EXPECTED
        mod.EXPECTED = self.fixture.expected_before
        try:
            owners = mod._read_owner_map(MODULE_DIR)
            self.assertEqual(24, len(owners))
        finally:
            mod.EXPECTED = fixture_expected

    def test_review_fingerprints_are_not_runtime_payload_files(self):
        mod = self.require_implementation()
        self.assertFalse(any("review" in entry.name.lower() for entry in MODULE_DIR.iterdir()))
        self.assertTrue(mod.EXPECTED["provenance_only_review_fingerprints"])
        self.assertFalse(any("review" in name.lower() for name in mod.EXPECTED["artifacts"]))
        result = mod.verify_inputs(self.fixture.baseline, self.fixture.payload)
        self.assertEqual(1, result["native_owner_count"])

    def test_cli_help_is_safe_and_constructs_the_arguments(self):
        mod = self.require_implementation()
        result = subprocess.run([sys.executable, str(Path(mod.__file__)), "--help"], text=True, capture_output=True, check=False)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("--baseline", result.stdout)
        self.assertIn("--payload", result.stdout)

    def test_wrong_patch_order_in_descriptor_is_rejected(self):
        mod = self.require_implementation()
        data = json.loads((self.fixture.payload / "pins.json").read_text())
        data["patch_order"][2:] = reversed(data["patch_order"][2:])
        write(self.fixture.payload / "pins.json", json.dumps(data, sort_keys=True, indent=2) + "\n")
        self.assertVerifyError("descriptor|patch.order|pins.json", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_preexisting_successor_file_is_rejected_before_any_patch(self):
        mod = self.require_implementation()
        new_owner = "crates/thaw-llvm/src/hir_codegen/tests/async/union_discard.rs"
        write(self.fixture.baseline / new_owner, "preexisting\n")
        subprocess.run(["git", "-C", str(self.fixture.baseline), "add", new_owner], check=True)
        subprocess.run(["git", "-C", str(self.fixture.baseline), "commit", "-qm", "fixture bad baseline"], check=True)
        self.fixture.expected["baseline"]["commit"] = subprocess.check_output(["git", "-C", str(self.fixture.baseline), "rev-parse", "HEAD"], text=True).strip()
        self.fixture.expected["baseline"]["tree"] = subprocess.check_output(["git", "-C", str(self.fixture.baseline), "rev-parse", "HEAD^{tree}"], text=True).strip()
        mod.EXPECTED = copy.deepcopy(self.fixture.expected)
        write(self.fixture.payload / "pins.json", json.dumps(self.fixture.expected, sort_keys=True, indent=2) + "\n")
        self.assertVerifyError("pre-existing|new owner|absent", mod.reconstruct, self.fixture.baseline, self.fixture.candidate, self.fixture.payload, self.fixture.evidence)

    def test_evidence_directory_must_not_overlap_baseline_payload_or_candidate(self):
        mod = self.require_implementation()
        baseline_nested = self.fixture.baseline / "evidence"
        self.assertVerifyError("evidence.*overlap|independent|disjoint", mod.reconstruct, self.fixture.baseline, self.fixture.candidate, self.fixture.payload, baseline_nested)
        payload_nested = self.fixture.payload / "evidence"
        self.assertVerifyError("evidence.*overlap|independent|disjoint", mod.reconstruct, self.fixture.baseline, self.fixture.candidate, self.fixture.payload, payload_nested)
        candidate_nested = self.fixture.candidate / "evidence"
        self.assertVerifyError("evidence.*overlap|independent|disjoint", mod.reconstruct, self.fixture.baseline, self.fixture.candidate, self.fixture.payload, candidate_nested)

    def test_evidence_write_path_that_is_a_file_fails_visibly(self):
        mod = self.require_implementation()
        invalid_evidence = self.fixture.root / "evidence-is-file"
        write(invalid_evidence, "not a directory\n")
        self.assertVerifyError("evidence.*directory|evidence.*file|evidence.*write", mod.reconstruct, self.fixture.baseline, self.fixture.candidate, self.fixture.payload, invalid_evidence)
        self.assertFalse(self.fixture.candidate.exists())

    def test_missing_control_include_owner_is_rejected(self):
        mod = self.require_implementation()
        tree = self.fixture.root / "manifest-tree-return-v1"
        include_target = tree / "crates/thaw-llvm/src/hir_codegen/tests/async/v5_control.rs"
        include_target.unlink()
        final_manifest = self.fixture.payload / "return-v1.sha256"
        manifest_paths = {rel for _, rel in mod._parse_sha_manifest(final_manifest, "test final manifest")}
        self.assertVerifyError("missing.*include|owner", mod.check_required_controls, tree, manifest_paths)

    def test_extra_file_inside_native_subset_is_rejected(self):
        mod = self.require_implementation()
        tree = self.fixture.root / "extra-stage"
        # Use the expected final stage from the immutable synthetic manifest.
        for rel in self._final_paths():
            source = self.fixture.root / "manifest-tree-return-v1" / rel
            write(tree / rel, source.read_bytes())
        write(tree / "crates/thaw-llvm/src/unexpected.rs", "extra\n")
        self.assertVerifyError("extra|inventory|subset", mod.verify_stage, tree, self.fixture.payload / "return-v1.sha256", len(self._final_paths()))

    def test_outside_subset_mutation_is_rejected_but_unchanged_outside_file_is_allowed(self):
        mod = self.require_implementation()
        original_apply = mod._apply_patch
        counter = {"n": 0}

        def corrupt_outside(tree, patch):
            result = original_apply(tree, patch)
            counter["n"] += 1
            if counter["n"] == 4:
                write(tree / "outside.txt", "unexpected mutation\n")
            return result

        with mock.patch.object(mod, "_apply_patch", side_effect=corrupt_outside):
            self.assertVerifyError("net|roster|outside|unexpected", mod.reconstruct, self.fixture.baseline, self.fixture.candidate, self.fixture.payload, self.fixture.evidence)
        self.assertTrue((self.fixture.evidence / "reconstruction.json").is_file())
        self.assertEqual("failed", json.loads((self.fixture.evidence / "reconstruction.json").read_text())["status"])

    def test_incorrect_modified_new_roster_is_rejected(self):
        mod = self.require_implementation()
        write(self.fixture.payload / "net-owners.txt", "modified  crates/thaw-hir/src/lib.rs\n")
        self.assertVerifyError("net-owners|roster|hash|pin", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_reconstruction_failure_never_reaches_a_cargo_executor(self):
        mod = self.require_implementation()
        (self.fixture.payload / "return-v1.patch").write_bytes(b"tampered\n")
        calls = []
        with mock.patch.object(mod, "_apply_patch", side_effect=lambda *args: calls.append(args)):
            self.assertVerifyError("return-v1.patch|hash|pin", mod.reconstruct, self.fixture.baseline, self.fixture.candidate, self.fixture.payload, self.fixture.evidence)
        self.assertEqual([], calls)

    def test_added_control_mode_drift_is_rejected_by_prepared_verification(self):
        mod = self.require_implementation()
        mod.reconstruct(self.fixture.baseline, self.fixture.candidate, self.fixture.payload, self.fixture.evidence)
        control = self.fixture.candidate / self.fixture.expected["control_owners"][0]
        control.chmod(0o755)
        self.assertVerifyError("mode|control owner|new owner", mod.verify_prepared, self.fixture.baseline, self.fixture.candidate, self.fixture.payload)

    @staticmethod
    def _final_paths() -> list[str]:
        return [
            ".gitignore",
            "Cargo.lock",
            "Cargo.toml",
            "crates/thaw-arena/Cargo.toml",
            "crates/thaw-hir/Cargo.toml",
            "crates/thaw-hir/src/lib.rs",
            "crates/thaw-llvm/Cargo.toml",
            "crates/thaw-llvm/src/hir_codegen/tests/async.rs",
            "crates/thaw-llvm/src/hir_codegen/tests/async/v5_control.rs",
            "crates/thaw-llvm/src/hir_codegen/tests/async/union_discard.rs",
            "crates/thaw-llvm/src/hir_codegen/tests/async/eval_then_returns.rs",
            "crates/thaw-runtime/Cargo.toml",
        ]


if __name__ == "__main__":
    unittest.main()
