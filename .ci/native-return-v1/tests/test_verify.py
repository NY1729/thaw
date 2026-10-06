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
import re
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
    """Tiny eight-stage baseline whose patch bytes and pins are self-consistent."""

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
            "crates/thaw-runtime/src/runtime/native_values/strings.rs": "runtime strings base\n",
            "crates/thaw-runtime/src/runtime/promises.rs": "runtime promises base\n",
            "crates/thaw-runtime/src/tests.rs": "runtime tests base\n",
            "crates/thaw-quickjs/src/quickjs/api.rs": "api baseline\n",
            "crates/thaw-quickjs/src/quickjs/platform_globals/runtime.js": "runtime baseline\n",
            "outside.txt": "unmodified product file\n",
        }
        for rel, text in initial.items():
            write(self.baseline / rel, text)
        repair_extra = "crates/thaw-std/src/json.rs"
        api_owner = "crates/thaw-quickjs/src/quickjs/api.rs"
        runtime_owner = "crates/thaw-quickjs/src/quickjs/platform_globals/runtime.js"
        strings_owner = "crates/thaw-runtime/src/runtime/native_values/strings.rs"
        promises_owner = "crates/thaw-runtime/src/runtime/promises.rs"
        runtime_tests_owner = "crates/thaw-runtime/src/tests.rs"
        http_owner = "crates/thaw-std/src/http.rs"
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

        stage1 = {key: value for key, value in initial.items() if key not in ("outside.txt", api_owner, runtime_owner)}
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
        stage7 = dict(stage6)
        stage7[lib] = "pub fn fixture() -> &'static str { \"next-hir-v1\" }\n"
        write(self.payload / "next-hir-v1.patch", patch_for(lib, stage6[lib], stage7[lib]))
        stage8 = dict(stage7)
        stage8[api_owner] = "api forced root final\n"
        stage8[runtime_owner] = "runtime forced root final\n"
        forced_root_patch = patch_for(api_owner, initial[api_owner], stage8[api_owner])
        forced_root_patch += patch_for(runtime_owner, initial[runtime_owner], stage8[runtime_owner])
        write(self.payload / "forced-root-wire-v1.patch", forced_root_patch)
        paired_base = dict(stage4)
        paired_base[repair_extra] = repair_extra_bytes.decode()
        repair_final = dict(stage5)
        repair_final[repair_extra] = repair_json_bytes.decode()
        cumulative_base = dict(repair_final)
        final_stage = dict(stage6)
        final_stage[repair_extra] = repair_json_bytes.decode()
        next_hir_base = dict(final_stage)
        next_hir_final = dict(stage7)
        next_hir_final[repair_extra] = repair_json_bytes.decode()
        forced_root_base = dict(next_hir_final)
        forced_root_base[api_owner] = initial[api_owner]
        forced_root_base[runtime_owner] = initial[runtime_owner]
        forced_root_final = dict(stage8)
        forced_root_final[repair_extra] = repair_json_bytes.decode()
        dependency_final = dict(forced_root_final)
        dependency_final[strings_owner] = "runtime strings dependency final\n"
        dependency_final[promises_owner] = "runtime promises dependency final\n"
        dependency_final[runtime_tests_owner] = "runtime tests dependency final\n"
        dependency_json_bytes = b"fixture json dependency final\n"
        dependency_final[repair_extra] = dependency_json_bytes.decode()
        dependency_patch = patch_for(strings_owner, initial[strings_owner], dependency_final[strings_owner])
        dependency_patch += patch_for(promises_owner, initial[promises_owner], dependency_final[promises_owner])
        dependency_patch += patch_for(runtime_tests_owner, initial[runtime_tests_owner], dependency_final[runtime_tests_owner])
        dependency_patch += patch_for(repair_extra, repair_json_bytes.decode(), dependency_json_bytes.decode())
        write(self.payload / "dependency-compile-v1.patch", dependency_patch)
        http_base_bytes = b"fixture std http base\n"
        http_final_bytes = b"fixture std http compiled\n"
        json_final_bytes = b"fixture json std compile final\n"
        std_base = dict(dependency_final)
        std_base[http_owner] = http_base_bytes.decode()
        std_final = dict(std_base)
        std_final[http_owner] = http_final_bytes.decode()
        std_final[repair_extra] = json_final_bytes.decode()
        std_patch = patch_for(http_owner, http_base_bytes.decode(), http_final_bytes.decode())
        std_patch += patch_for(repair_extra, dependency_json_bytes.decode(), json_final_bytes.decode())
        write(self.payload / "std-compile-v1.patch", std_patch)
        write(self.baseline / http_owner, http_base_bytes)
        for name, files in (
            ("compile-repairs-v1-base", paired_base),
            ("compile-repairs-v1", repair_final),
            ("cumulative-hir-v1-base", cumulative_base),
            ("cumulative-hir-v1", final_stage),
            ("next-hir-v1-base", next_hir_base),
            ("next-hir-v1", next_hir_final),
            ("forced-root-wire-v1-base", forced_root_base),
            ("forced-root-wire-v1", forced_root_final),
            ("dependency-compile-v1", dependency_final),
            ("std-compile-v1-base", std_base),
            ("std-compile-v1", std_final),
        ):
            stage_dir = root / ("manifest-tree-" + name)
            for rel, text in files.items():
                write(stage_dir / rel, text)
            path_order = "lexical" if name in ("compile-repairs-v1", "cumulative-hir-v1-base", "cumulative-hir-v1", "next-hir-v1-base", "next-hir-v1", "forced-root-wire-v1-base", "forced-root-wire-v1", "dependency-compile-v1", "std-compile-v1-base", "std-compile-v1") else "components"
            (self.payload / f"{name}.sha256").write_bytes(manifest_for(stage_dir, path_order))
        (self.payload / "dependency-compile-v1-base.sha256").write_bytes((self.payload / "forced-root-wire-v1.sha256").read_bytes())

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
        stages.append({
            "name": "next-hir-v1",
            "file": "next-hir-v1.sha256",
            "sha256": sha((self.payload / "next-hir-v1.sha256").read_bytes()),
            "count": len(next_hir_final),
            "extra_paths": [repair_extra],
            "path_order": "lexical",
        })
        stages.append({
            "name": "forced-root-wire-v1",
            "file": "forced-root-wire-v1.sha256",
            "sha256": sha((self.payload / "forced-root-wire-v1.sha256").read_bytes()),
            "count": len(forced_root_final),
            "extra_paths": [repair_extra, api_owner, runtime_owner],
            "path_order": "lexical",
        })
        stages.append({
            "name": "dependency-compile-v1",
            "file": "dependency-compile-v1.sha256",
            "sha256": sha((self.payload / "dependency-compile-v1.sha256").read_bytes()),
            "count": len(dependency_final),
            "extra_paths": [repair_extra, api_owner, runtime_owner],
            "path_order": "lexical",
        })
        stages.append({
            "name": "std-compile-v1",
            "file": "std-compile-v1.sha256",
            "sha256": sha((self.payload / "std-compile-v1.sha256").read_bytes()),
            "count": len(std_final),
            "extra_paths": [repair_extra, api_owner, runtime_owner, http_owner],
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
            f"modified  {api_owner}\n",
            f"modified  {runtime_owner}\n",
            f"modified  {strings_owner}\n",
            f"modified  {promises_owner}\n",
            f"modified  {runtime_tests_owner}\n",
            f"modified  {http_owner}\n",
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
            for name in ("discard-v2.patch", "return-v1.patch", "compile-repairs-v1.patch", "cumulative-hir-v1.patch", "next-hir-v1.patch", "forced-root-wire-v1.patch", "dependency-compile-v1.patch", "std-compile-v1.patch")
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
                "count": 11,
                "modified": 8,
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
            "next_hir_base": {
                "file": "next-hir-v1-base.sha256",
                "sha256": sha((self.payload / "next-hir-v1-base.sha256").read_bytes()),
                "count": len(next_hir_base),
                "path_order": "lexical",
                "paired_after": "cumulative-hir-v1",
            },
            "forced_root_wire_base": {
                "file": "forced-root-wire-v1-base.sha256",
                "sha256": sha((self.payload / "forced-root-wire-v1-base.sha256").read_bytes()),
                "count": len(forced_root_base),
                "path_order": "lexical",
                "paired_after": "next-hir-v1",
                "extra_owners": [
                    {
                        "path": api_owner,
                        "sha256": sha(initial[api_owner].encode()),
                        "git_blob_sha1": verify._git_blob_sha1(initial[api_owner].encode()) if verify is not None else "",
                        "final_sha256": sha(stage8[api_owner].encode()),
                    },
                    {
                        "path": runtime_owner,
                        "sha256": sha(initial[runtime_owner].encode()),
                        "git_blob_sha1": verify._git_blob_sha1(initial[runtime_owner].encode()) if verify is not None else "",
                        "final_sha256": sha(stage8[runtime_owner].encode()),
                    },
                ],
            },
            "dependency_compile_base": {
                "file": "dependency-compile-v1-base.sha256",
                "sha256": sha((self.payload / "dependency-compile-v1-base.sha256").read_bytes()),
                "count": len(forced_root_final),
                "path_order": "lexical",
                "paired_after": "forced-root-wire-v1",
            },
            "std_compile_base": {
                "file": "std-compile-v1-base.sha256",
                "sha256": sha((self.payload / "std-compile-v1-base.sha256").read_bytes()),
                "count": len(std_base),
                "path_order": "lexical",
                "paired_after": "dependency-compile-v1",
                "extra_owner": {
                    "path": http_owner,
                    "sha256": sha(http_base_bytes),
                    "git_blob_sha1": verify._git_blob_sha1(http_base_bytes) if verify is not None else "",
                    "final_sha256": sha(http_final_bytes),
                },
            },
            "patch_order": ["original-native", "scope-v5", "discard-v2", "return-v1", "compile-repairs-v1", "cumulative-hir-v1", "next-hir-v1", "forced-root-wire-v1", "dependency-compile-v1", "std-compile-v1"],
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
            "expected_full_test_names": copy.deepcopy(verify.EXPECTED.get("expected_full_test_names", {})) if verify is not None else {},
            "provenance_only_review_fingerprints": copy.deepcopy(verify.EXPECTED["provenance_only_review_fingerprints"]) if verify is not None else {},
        }
        self.expected["std_compile_base"]["extra_owner"]["final_sha256"] = sha(http_final_bytes)
        self.expected["artifacts"]["std-compile-v1-base.sha256"] = sha((self.payload / "std-compile-v1-base.sha256").read_bytes())
        self.expected["artifacts"]["std-compile-v1.sha256"] = sha((self.payload / "std-compile-v1.sha256").read_bytes())
        self.expected["net_owners"] = {
            "file": "net-owners.txt",
            "sha256": sha((self.payload / "net-owners.txt").read_bytes()),
            "count": 12,
            "modified": 9,
            "new": 3,
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
            "next_hir_base": "next-hir-v1-base.sha256",
            "next_hir_final": "next-hir-v1.sha256",
            "forced_root_wire_base": "forced-root-wire-v1-base.sha256",
            "forced_root_wire_final": "forced-root-wire-v1.sha256",
            "dependency_compile_base": "dependency-compile-v1-base.sha256",
            "dependency_compile_final": "dependency-compile-v1.sha256",
            "std_compile_base": "std-compile-v1-base.sha256",
            "std_compile_final": "std-compile-v1.sha256",
            "native_base": "native-base.sha256",
            "net_owners": "net-owners.txt",
            "scope_test_plan": "scope-v5-test-plan.md",
        }

    def activate(self):
        """Install fixture pins in the module while preserving production logic."""
        if verify is None:
            return
        self.expected_before = verify.EXPECTED
        self.std_http_constants_before = (
            verify.STD_HTTP_BASE_SHA256,
            verify.STD_HTTP_BASE_GIT_BLOB_SHA1,
            verify.STD_HTTP_FINAL_SHA256,
        )
        owner = self.expected["std_compile_base"]["extra_owner"]
        verify.STD_HTTP_BASE_SHA256 = owner["sha256"]
        verify.STD_HTTP_BASE_GIT_BLOB_SHA1 = owner["git_blob_sha1"]
        verify.STD_HTTP_FINAL_SHA256 = owner["final_sha256"]
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
            (
                verify.STD_HTTP_BASE_SHA256,
                verify.STD_HTTP_BASE_GIT_BLOB_SHA1,
                verify.STD_HTTP_FINAL_SHA256,
            ) = self.fixture.std_http_constants_before
        self.temp.cleanup()

    def assertVerifyError(self, pattern: str, call, *args, **kwargs):
        mod = self.require_implementation()
        with self.assertRaisesRegex(mod.VerificationError, pattern):
            call(*args, **kwargs)

    def test_positive_reconstruction_replays_ten_stages_and_preserves_unrelated_file(self):
        mod = self.require_implementation()
        result = mod.reconstruct(
            self.fixture.baseline,
            self.fixture.candidate,
            self.fixture.payload,
            self.fixture.evidence,
        )
        self.assertEqual([x["count"] for x in mod.EXPECTED["stage_manifests"]], result["stage_counts"])
        self.assertEqual(10, len(result["stages"]))
        self.assertEqual("std-compile-v1", result["stages"][-1]["stage"])
        self.assertEqual(mod.EXPECTED["stage_manifests"][-1]["count"], result["stage_counts"][-1])
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
        self.assertEqual(b"fixture json std compile final\n", (self.fixture.candidate / extra).read_bytes())

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

    def test_seventh_next_hir_patch_changes_a_native_owner_after_the_paired_stage(self):
        mod = self.require_implementation()
        lib = "crates/thaw-hir/src/lib.rs"
        result = mod.reconstruct(self.fixture.baseline, self.fixture.candidate, self.fixture.payload, self.fixture.evidence)
        repair_bytes = (self.fixture.root / "manifest-tree-compile-repairs-v1" / lib).read_bytes()
        expected_repair_hash = next(
            digest for digest, rel in mod._parse_sha_manifest(self.fixture.payload / "compile-repairs-v1.sha256", "repair final", path_order="lexical")
            if rel == lib
        )
        self.assertEqual("next-hir-v1", result["stages"][6]["stage"])
        self.assertEqual(sha(repair_bytes), expected_repair_hash)
        self.assertEqual(b'pub fn fixture() -> &\'static str { "next-hir-v1" }\n', (self.fixture.candidate / lib).read_bytes())

    def test_next_hir_paired_base_is_checked_immediately_before_the_seventh_patch(self):
        mod = self.require_implementation()
        original_verify_stage = mod.verify_stage
        applied = []
        original_apply = mod._apply_patch

        def apply_and_record(tree, patch):
            applied.append(patch.name)
            return original_apply(tree, patch)

        def verify_then_drift(tree, manifest, *args, **kwargs):
            result = original_verify_stage(tree, manifest, *args, **kwargs)
            if Path(manifest).name == "cumulative-hir-v1.sha256":
                write(tree / "crates/thaw-hir/src/lib.rs", "drift after sixth final\n")
            return result

        with mock.patch.object(mod, "verify_stage", side_effect=verify_then_drift), mock.patch.object(mod, "_apply_patch", side_effect=apply_and_record):
            self.assertVerifyError("paired.*base|manifest hash mismatch|stage hash mismatch", mod.reconstruct, self.fixture.baseline, self.fixture.candidate, self.fixture.payload, self.fixture.evidence)
        self.assertEqual("cumulative-hir-v1.patch", applied[-1])
        self.assertNotIn("next-hir-v1.patch", applied)

    def test_forced_root_pair_is_exactly_next_hir_final_plus_two_pinned_providers(self):
        mod = self.require_implementation()
        pair = mod.EXPECTED["forced_root_wire_base"]
        self.assertEqual("next-hir-v1", pair["paired_after"])
        self.assertEqual("lexical", pair["path_order"])
        result = mod.verify_inputs(self.fixture.baseline, self.fixture.payload)
        self.assertEqual(12, result["net_owner_count"])
        self.assertEqual(2, len(pair["extra_owners"]))
        before = {rel: digest for digest, rel in mod._parse_sha_manifest(self.fixture.payload / pair["file"], "wire base", path_order="lexical")}
        previous = {rel: digest for digest, rel in mod._parse_sha_manifest(self.fixture.payload / "next-hir-v1.sha256", "next final", path_order="lexical")}
        self.assertEqual(len(previous) + 2, pair["count"])
        for owner in pair["extra_owners"]:
            self.assertNotIn(owner["path"], previous)
            previous[owner["path"]] = owner["sha256"]
        self.assertEqual(previous, before)

    def test_forced_root_provider_sha256_and_git_blob_are_pinned(self):
        mod = self.require_implementation()
        owner = mod.EXPECTED["forced_root_wire_base"]["extra_owners"][0]
        path = self.fixture.baseline / owner["path"]
        original = path.read_bytes()
        original_commit = self.fixture.commit
        original_expected = copy.deepcopy(mod.EXPECTED)
        try:
            path.write_bytes(original + b"baseline drift\n")
            subprocess.run(["git", "-C", str(self.fixture.baseline), "add", owner["path"]], check=True)
            subprocess.run(["git", "-C", str(self.fixture.baseline), "commit", "-qm", "fixture provider drift"], check=True)
            altered = copy.deepcopy(mod.EXPECTED)
            altered["baseline"]["commit"] = subprocess.check_output(["git", "-C", str(self.fixture.baseline), "rev-parse", "HEAD"], text=True).strip()
            altered["baseline"]["tree"] = subprocess.check_output(["git", "-C", str(self.fixture.baseline), "rev-parse", "HEAD^{tree}"], text=True).strip()
            mod.EXPECTED = altered
            write(self.fixture.payload / "pins.json", json.dumps(altered, sort_keys=True, indent=2) + "\n")
            self.assertVerifyError("forced-root.*provider|SHA-256|Git blob", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)
        finally:
            subprocess.run(["git", "-C", str(self.fixture.baseline), "reset", "--hard", original_commit], check=True)
            mod.EXPECTED = original_expected
            write(self.fixture.payload / "pins.json", json.dumps(self.fixture.expected, sort_keys=True, indent=2) + "\n")

    def test_forced_root_extra_paths_are_limited_to_the_eighth_stage(self):
        mod = self.require_implementation()
        expected_paths = [
            "crates/thaw-std/src/json.rs",
            "crates/thaw-quickjs/src/quickjs/api.rs",
            "crates/thaw-quickjs/src/quickjs/platform_globals/runtime.js",
        ]
        self.assertEqual(["crates/thaw-std/src/json.rs"], mod.EXPECTED["stage_manifests"][6]["extra_paths"])
        self.assertEqual(expected_paths, mod.EXPECTED["stage_manifests"][7]["extra_paths"])
        altered = copy.deepcopy(mod.EXPECTED)
        altered["stage_manifests"][6]["extra_paths"] = expected_paths
        mod.EXPECTED = altered
        write(self.fixture.payload / "pins.json", json.dumps(altered, sort_keys=True, indent=2) + "\n")
        self.assertVerifyError("unexpected stage-scoped extra path", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_forced_root_pair_cannot_be_retargeted_to_an_older_stage(self):
        mod = self.require_implementation()
        altered = copy.deepcopy(mod.EXPECTED)
        altered["forced_root_wire_base"]["paired_after"] = "cumulative-hir-v1"
        mod.EXPECTED = altered
        write(self.fixture.payload / "pins.json", json.dumps(altered, sort_keys=True, indent=2) + "\n")
        self.assertVerifyError("forced-root wire paired base|immediately follow", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_unexpected_forced_root_stage_input_is_rejected(self):
        mod = self.require_implementation()
        write(self.fixture.payload / "forced-root-wire-v2.patch", "unexpected stage\n")
        self.assertVerifyError("unexpected forced-root wire input", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_forced_root_pair_is_checked_immediately_before_eighth_patch(self):
        mod = self.require_implementation()
        original_verify_stage = mod.verify_stage
        original_apply = mod._apply_patch
        applied = []

        def apply_and_record(tree, patch):
            applied.append(patch.name)
            return original_apply(tree, patch)

        def verify_then_drift(tree, manifest, *args, **kwargs):
            result = original_verify_stage(tree, manifest, *args, **kwargs)
            if Path(manifest).name == "next-hir-v1.sha256":
                write(tree / mod.EXPECTED["forced_root_wire_base"]["extra_owners"][0]["path"], "drift after seventh final\n")
            return result

        with mock.patch.object(mod, "verify_stage", side_effect=verify_then_drift), mock.patch.object(mod, "_apply_patch", side_effect=apply_and_record):
            self.assertVerifyError("paired.*base|manifest hash mismatch|stage hash mismatch", mod.reconstruct, self.fixture.baseline, self.fixture.candidate, self.fixture.payload, self.fixture.evidence)
        self.assertEqual("next-hir-v1.patch", applied[-1])
        self.assertNotIn("forced-root-wire-v1.patch", applied)

    def test_dependency_compile_pair_preserves_wire_roots_and_changes_only_four_sources(self):
        mod = self.require_implementation()
        base = mod.EXPECTED["dependency_compile_base"]
        stage = next(item for item in mod.EXPECTED["stage_manifests"] if item["name"] == "dependency-compile-v1")
        predecessor = next(item for item in mod.EXPECTED["stage_manifests"] if item["name"] == "forced-root-wire-v1")
        self.assertEqual("forced-root-wire-v1", base["paired_after"])
        self.assertEqual(10, len(mod.EXPECTED["stage_manifests"]))
        self.assertEqual(predecessor["count"], base["count"])
        self.assertEqual(base["count"], stage["count"])
        self.assertEqual(predecessor["extra_paths"], stage["extra_paths"])
        self.assertEqual(
            (self.fixture.payload / predecessor["file"]).read_bytes(),
            (self.fixture.payload / base["file"]).read_bytes(),
        )
        base_map = dict((rel, digest) for digest, rel in mod._parse_sha_manifest(self.fixture.payload / base["file"], "dependency base", path_order="lexical"))
        final_map = dict((rel, digest) for digest, rel in mod._parse_sha_manifest(self.fixture.payload / stage["file"], "dependency final", path_order="lexical"))
        changed = {rel for rel in base_map if base_map[rel] != final_map[rel]}
        self.assertEqual(set(mod.DEPENDENCY_SOURCE_PATHS), changed)
        self.assertEqual(set(base_map), set(final_map))
        self.assertEqual(len(base_map) - 4, len(base_map) - len(changed))
        result = mod.verify_inputs(self.fixture.baseline, self.fixture.payload)
        self.assertEqual(4, len(result["dependency_compile"]["changed_source_owners"]))
        self.assertEqual(base["count"] - 4, result["dependency_compile"]["unchanged_source_count"])

    def test_dependency_compile_pair_is_checked_immediately_before_ninth_patch(self):
        mod = self.require_implementation()
        original_verify_stage = mod.verify_stage
        original_apply = mod._apply_patch
        applied = []

        def apply_and_record(tree, patch):
            applied.append(patch.name)
            return original_apply(tree, patch)

        def verify_then_drift(tree, manifest, *args, **kwargs):
            result = original_verify_stage(tree, manifest, *args, **kwargs)
            if Path(manifest).name == "forced-root-wire-v1.sha256":
                write(tree / mod.DEPENDENCY_SOURCE_PATHS[1], "drift after wire final\n")
            return result

        with mock.patch.object(mod, "verify_stage", side_effect=verify_then_drift), mock.patch.object(mod, "_apply_patch", side_effect=apply_and_record):
            self.assertVerifyError("dependency compile paired base|stage hash mismatch", mod.reconstruct, self.fixture.baseline, self.fixture.candidate, self.fixture.payload, self.fixture.evidence)
        self.assertEqual("forced-root-wire-v1.patch", applied[-1])
        self.assertNotIn("dependency-compile-v1.patch", applied)

    def test_dependency_compile_final_json_drift_is_rejected(self):
        mod = self.require_implementation()
        mod.reconstruct(self.fixture.baseline, self.fixture.candidate, self.fixture.payload, self.fixture.evidence)
        json_owner = mod.FORCED_ROOT_STAGE_EXTRA_PATHS[0]
        path = self.fixture.candidate / json_owner
        path.write_bytes(path.read_bytes() + b"drift\n")
        self.assertVerifyError("stage hash mismatch", mod.verify_prepared, self.fixture.baseline, self.fixture.candidate, self.fixture.payload)

    def test_dependency_compile_inputs_are_individually_pinned(self):
        mod = self.require_implementation()
        for name in ("dependency-compile-v1.patch", "dependency-compile-v1-base.sha256", "dependency-compile-v1.sha256"):
            with self.subTest(name=name):
                path = self.fixture.payload / name
                original = path.read_bytes()
                try:
                    path.write_bytes(original + b"# drift\n")
                    label = "payload patch hash mismatch" if name.endswith(".patch") else "payload artifact hash mismatch"
                    self.assertVerifyError(label, mod.verify_inputs, self.fixture.baseline, self.fixture.payload)
                finally:
                    path.write_bytes(original)

    def test_unexpected_dependency_compile_stage_input_is_rejected(self):
        mod = self.require_implementation()
        write(self.fixture.payload / "dependency-compile-v2.patch", "unexpected stage\n")
        self.assertVerifyError("unexpected dependency compile input", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_forced_root_final_drift_is_rejected_after_reconstruction(self):
        mod = self.require_implementation()
        mod.reconstruct(self.fixture.baseline, self.fixture.candidate, self.fixture.payload, self.fixture.evidence)
        path = self.fixture.candidate / mod.EXPECTED["forced_root_wire_base"]["extra_owners"][0]["path"]
        path.write_bytes(path.read_bytes() + b"drift\n")
        self.assertVerifyError("stage hash mismatch", mod.verify_prepared, self.fixture.baseline, self.fixture.candidate, self.fixture.payload)

    def test_actual_stage_inputs_and_49_group_contract_match_the_frozen_sources(self):
        mod = self.require_implementation()
        expected = self.fixture.expected_before
        self.assertEqual(expected, json.loads((MODULE_DIR / "pins.json").read_text(encoding="utf-8")))
        self.assertEqual(50, len(expected["test_filters"]))
        self.assertEqual("0c2490af44241dfc80bef8ca25c321323722ab899842a0a84b6fd57906a48113", sha(json.dumps(expected["test_filters"][:49], sort_keys=True, separators=(",", ":")).encode()))
        self.assertEqual("36f8d90adffb57aac119cd538c9525e9dd13a9213cb8260832cdc2bee609e621", sha(json.dumps(expected["test_filters"][:39], sort_keys=True, separators=(",", ":")).encode()))
        self.assertEqual("d94499d8d78d64e6dbb72fbac6abaa548398718f89b5685d1d5a759f944954f9", sha(json.dumps(expected["test_filters"][:30], sort_keys=True, separators=(",", ":")).encode()))
        self.assertEqual("76007d26d76fa871bbc129d1ff94718df82a53a5bf363cf707e039a0561e669b", sha(json.dumps(expected["expected_test_names"], sort_keys=True, separators=(",", ":")).encode()))
        old_full_names = {key: value for key, value in expected["expected_full_test_names"].items() if key not in {item["filter"] for item in expected["test_filters"][39:]}}
        self.assertEqual("5efbf022609d87fda1346b2ed8a55b76113385735228ed176a0a2f2a8d996a7e", sha(json.dumps(old_full_names, sort_keys=True, separators=(",", ":")).encode()))
        self.assertEqual((20, 9, 8, 7, 6), tuple(sum(item["package"] == package for item in expected["test_filters"]) for package in ("thaw-llvm", "thaw-hir", "thaw-std", "thaw-quickjs", "thaw-runtime")))
        appended = expected["test_filters"][39:]
        expected_appended = [
            ("thaw-runtime", "tests::dependency_compile_uri_byte_iteration"),
            ("thaw-runtime", "tests::dependency_compile_root_locale_case"),
            ("thaw-runtime", "tests::dependency_compile_finally_adopt_deferred_frame"),
            ("thaw-std", "json::tests::dependency_compile_writable_expression"),
            ("thaw-std", "json::tests::dependency_compile_as_string_variants"),
            ("thaw-runtime", "tests::private_exception_provenance_has_stable_nine_word_layout"),
            ("thaw-runtime", "tests::finally_adopt_roots_original_result_object_and_aggregate_until_output_owns_them"),
            ("thaw-runtime", "tests::purged_aggregate_callbacks_ignore_late_child_results_and_release_roots"),
            ("thaw-std", "json::tests::jit_dictionary_callbacks_preserve_presence_and_typed_failures"),
            ("thaw-std", "json::tests::a_missing_key_or_index_is_distinguishable_from_an_explicit_null"),
            ("thaw-std", "http::tests::peer_send_eof_preserves_pending_streamed_response"),
        ]
        self.assertEqual(expected_appended, [(item["package"], item["filter"]) for item in appended])
        self.assertTrue(all(set(item) == {"package", "filter"} for item in appended))
        self.assertEqual({name: [name] for _, name in expected_appended}, {key: expected["expected_full_test_names"][key] for _, key in expected_appended})
        self.assertEqual(58, expected["net_owners"]["count"])
        self.assertEqual((53, 5), (expected["net_owners"]["modified"], expected["net_owners"]["new"]))
        net_path = MODULE_DIR / expected["net_owners"]["file"]
        self.assertEqual("e6e1b5beb336d919941e914ddc7569c3b5567ed30344f69adde12c4f1f891d0c", sha(net_path.read_bytes()))
        fixture_expected = mod.EXPECTED
        mod.EXPECTED = expected
        try:
            roster = mod._read_net_roster(MODULE_DIR)
        finally:
            mod.EXPECTED = fixture_expected
        self.assertEqual(58, len(roster))
        self.assertEqual((53, 5), (sum(value == "modified" for value in roster.values()), sum(value == "new" for value in roster.values())))
        self.assertEqual("modified", roster["crates/thaw-runtime/src/runtime/native_values/strings.rs"])

        self.assertEqual(["original-native", "scope-v5", "discard-v2", "return-v1", "compile-repairs-v1", "cumulative-hir-v1", "next-hir-v1", "forced-root-wire-v1", "dependency-compile-v1", "std-compile-v1"], expected["patch_order"])
        self.assertEqual([175, 176, 179, 180, 181, 181, 181, 183, 183, 184], [stage["count"] for stage in expected["stage_manifests"]])
        self.assertEqual("83a99e4934c4108992f874ca75ff2ea366283fbc3c543afaf9fee6489cb95941", sha((MODULE_DIR / "forced-root-wire-v1.sha256").read_bytes()))
        self.assertEqual("d3b7c0e06ca053f4b342dcbc8ac1247a3460c946a35a2b22d2645c76251a2aa9", expected["artifacts"]["compile-repairs-v1.sha256"])
        self.assertEqual("0be3783d6241706ba2dd492704162aa9e0076dcc4a1de96e309eca9b512ba81c", expected["artifacts"]["cumulative-hir-v1.sha256"])
        self.assertEqual("f8915460efcaafc82a4392d9ca6b656dc150045d5cad4458b66755ef5705635b", expected["artifacts"]["next-hir-v1.sha256"])
        self.assertEqual("a7f04af1afa5b9d41d7a8dca984ccefba020cdd36f19175ef14a8f809a6e8683", expected["payload_patches"]["forced-root-wire-v1.patch"])
        self.assertEqual("23da8386704a188e890bd7e591bb395ce611d45e8fbf0ad264716b5d61e3c9bf", expected["forced_root_wire_base"]["sha256"])

        wire_stage = next(stage for stage in expected["stage_manifests"] if stage["name"] == "forced-root-wire-v1")
        dependency_stage = next(stage for stage in expected["stage_manifests"] if stage["name"] == "dependency-compile-v1")
        dependency_base = expected["dependency_compile_base"]
        self.assertEqual("dependency-compile-v1", dependency_stage["name"])
        self.assertEqual("forced-root-wire-v1", dependency_base["paired_after"])
        self.assertEqual(183, dependency_stage["count"])
        self.assertEqual(["crates/thaw-std/src/json.rs", "crates/thaw-quickjs/src/quickjs/api.rs", "crates/thaw-quickjs/src/quickjs/platform_globals/runtime.js"], dependency_stage["extra_paths"])
        base_path = MODULE_DIR / dependency_base["file"]
        wire_path = MODULE_DIR / wire_stage["file"]
        final_path = MODULE_DIR / dependency_stage["file"]
        patch_path = MODULE_DIR / "dependency-compile-v1.patch"
        self.assertEqual("68a5c8e397df3462aeb83879d854837a423ef297d523f455ecd62d05b7281aea", sha(patch_path.read_bytes()))
        self.assertEqual("83a99e4934c4108992f874ca75ff2ea366283fbc3c543afaf9fee6489cb95941", sha(base_path.read_bytes()))
        self.assertEqual("605121c9db1c995897770fc223d6207d18e9a6075e11725c2c9c0e25f8e2288d", sha(final_path.read_bytes()))
        self.assertEqual(wire_path.read_bytes(), base_path.read_bytes())
        wire_map = dict((rel, digest) for digest, rel in mod._parse_sha_manifest(wire_path, "wire final", path_order="lexical"))
        base_map = dict((rel, digest) for digest, rel in mod._parse_sha_manifest(base_path, "dependency base", path_order="lexical"))
        final_map = dict((rel, digest) for digest, rel in mod._parse_sha_manifest(final_path, "dependency final", path_order="lexical"))
        self.assertEqual(wire_map, base_map)
        self.assertEqual(183, len(base_map))
        self.assertEqual(set(base_map), set(final_map))
        changed = {rel for rel in base_map if base_map[rel] != final_map[rel]}
        self.assertEqual(set(mod.DEPENDENCY_SOURCE_PATHS), changed)
        self.assertEqual(179, len(base_map) - len(changed))
        self.assertEqual(dependency_stage["sha256"], sha(final_path.read_bytes()))
        self.assertEqual("std-compile-v1", expected["stage_manifests"][-1]["name"])
        self.assertEqual(184, expected["stage_manifests"][-1]["count"])
        for rel, expected_hash in {**expected["payload_patches"], **expected["artifacts"]}.items():
            with self.subTest(path=rel):
                self.assertEqual(expected_hash, sha((MODULE_DIR / rel).read_bytes()), rel)

    def test_tenth_std_compile_stage_preserves_the_exact_first_49_groups(self):
        self.require_implementation()
        expected = json.loads((MODULE_DIR / "pins.json").read_text(encoding="utf-8"))
        self.assertEqual(
            "0c2490af44241dfc80bef8ca25c321323722ab899842a0a84b6fd57906a48113",
            sha(json.dumps(expected["test_filters"][:49], sort_keys=True, separators=(",", ":")).encode()),
        )
        self.assertEqual(50, len(expected["test_filters"]))
        self.assertEqual(
            {"package": "thaw-std", "filter": "http::tests::peer_send_eof_preserves_pending_streamed_response"},
            expected["test_filters"][49],
        )
        self.assertEqual(
            ["original-native", "scope-v5", "discard-v2", "return-v1", "compile-repairs-v1", "cumulative-hir-v1", "next-hir-v1", "forced-root-wire-v1", "dependency-compile-v1", "std-compile-v1"],
            expected["patch_order"],
        )
        stage = expected["stage_manifests"][-1]
        self.assertEqual(("std-compile-v1", 184, "lexical"), (stage["name"], stage["count"], stage["path_order"]))
        self.assertEqual(
            ["crates/thaw-std/src/json.rs", "crates/thaw-quickjs/src/quickjs/api.rs", "crates/thaw-quickjs/src/quickjs/platform_globals/runtime.js", "crates/thaw-std/src/http.rs"],
            stage["extra_paths"],
        )
        self.assertEqual("4ad52d51d87666b1a910477daa3c8b7a2c3b4b9825198a1b1495242aba1cbddf", self.fixture.std_http_constants_before[0])
        self.assertEqual("ff3ff9850cef56b5dd7be727c6bdbf74adb143d8", self.fixture.std_http_constants_before[1])
        self.assertEqual(
            ["http::tests::peer_send_eof_preserves_pending_streamed_response"],
            expected["expected_full_test_names"][expected["test_filters"][49]["filter"]],
        )

    def test_std_compile_pair_rejects_wrong_baseline_http_identity(self):
        mod = self.require_implementation()
        helper = getattr(mod, "_verify_std_compile_inputs", None)
        self.assertTrue(callable(helper), "std compile source identity verifier is missing")
        stage_by_name = {stage["name"]: stage for stage in mod.EXPECTED["stage_manifests"]}
        net = mod._read_net_roster(self.fixture.payload)
        result = helper(self.fixture.payload, self.fixture.baseline, stage_by_name, net)
        self.assertEqual(stage_by_name["std-compile-v1"]["count"], result["base_count"])
        http = self.fixture.baseline / mod.STD_HTTP_PATH
        original = http.read_bytes()
        try:
            http.write_bytes(original + b"changed identity\n")
            self.assertVerifyError("baseline HTTP owner SHA-256 or Git blob identity mismatch", helper, self.fixture.payload, self.fixture.baseline, stage_by_name, net)
        finally:
            http.write_bytes(original)

    def test_shipped_roster_artifact_descriptor_and_bytes_share_the_expected_pin(self):
        production_expected = self.fixture.expected_before
        roster_path = MODULE_DIR / "net-owners.txt"
        roster_digest = sha(roster_path.read_bytes())
        self.assertEqual("e6e1b5beb336d919941e914ddc7569c3b5567ed30344f69adde12c4f1f891d0c", roster_digest)
        self.assertEqual(roster_digest, production_expected["net_owners"]["sha256"])
        self.assertEqual(roster_digest, production_expected["artifacts"]["net-owners.txt"])
        shipped_pins = json.loads((MODULE_DIR / "pins.json").read_text(encoding="utf-8"))
        self.assertEqual(production_expected, shipped_pins)
        for rel, expected_hash in {**production_expected["payload_patches"], **production_expected["artifacts"]}.items():
            with self.subTest(path=rel):
                self.assertEqual(expected_hash, sha((MODULE_DIR / rel).read_bytes()), rel)

    def test_next_hir_pair_descriptor_cannot_be_retargeted_to_an_older_stage(self):
        mod = self.require_implementation()
        altered = copy.deepcopy(mod.EXPECTED)
        altered["next_hir_base"]["paired_after"] = "compile-repairs-v1"
        mod.EXPECTED = altered
        write(self.fixture.payload / "pins.json", json.dumps(altered, sort_keys=True, indent=2) + "\n")
        self.assertVerifyError("next HIR paired base|immediately follow", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_next_hir_stage_order_cannot_move_ahead_of_its_paired_stage(self):
        mod = self.require_implementation()
        altered = copy.deepcopy(mod.EXPECTED)
        altered["stage_manifests"][6:8] = reversed(altered["stage_manifests"][6:8])
        altered["patch_order"][6:8] = reversed(altered["patch_order"][6:8])
        mod.EXPECTED = altered
        write(self.fixture.payload / "pins.json", json.dumps(altered, sort_keys=True, indent=2) + "\n")
        self.assertVerifyError("next HIR paired base|forced-root wire paired base|immediately follow|patch order mismatch", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_next_hir_patch_source_drift_is_rejected(self):
        mod = self.require_implementation()
        with (self.fixture.payload / "next-hir-v1.patch").open("ab") as stream:
            stream.write(b"# source drift\n")
        self.assertVerifyError("next-hir-v1.patch|hash|pin", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_next_hir_missing_paired_input_is_rejected(self):
        mod = self.require_implementation()
        for name in ("next-hir-v1.patch", "next-hir-v1-base.sha256", "next-hir-v1.sha256"):
            with self.subTest(name=name):
                path = self.fixture.payload / name
                original = path.read_bytes()
                expected = "payload patch missing" if name.endswith(".patch") else "payload artifact missing"
                try:
                    path.unlink()
                    self.assertVerifyError(f"{expected}: {re.escape(name)}", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)
                finally:
                    write(path, original)

    def test_next_hir_paired_and_final_manifest_drift_is_rejected(self):
        mod = self.require_implementation()
        for name in ("next-hir-v1-base.sha256", "next-hir-v1.sha256"):
            with self.subTest(name=name):
                path = self.fixture.payload / name
                original = path.read_bytes()
                try:
                    path.write_bytes(original + b"# drift\n")
                    self.assertVerifyError(f"payload artifact hash mismatch for {re.escape(name)}", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)
                finally:
                    path.write_bytes(original)

    def test_unexpected_next_hir_stage_input_is_rejected(self):
        mod = self.require_implementation()
        write(self.fixture.payload / "next-hir-v2.patch", "unexpected stage\n")
        self.assertVerifyError("unexpected next HIR input", mod.verify_inputs, self.fixture.baseline, self.fixture.payload)

    def test_roster_added_spelling_normalizes_to_new_and_keeps_counts(self):
        mod = self.require_implementation()
        roster_path = self.fixture.payload / "net-owners.txt"
        rows = roster_path.read_text(encoding="utf-8").splitlines()
        index = next(i for i, row in enumerate(rows) if row.startswith("new  "))
        rows[index] = "added  " + rows[index].removeprefix("new  ")
        write(roster_path, "\n".join(rows) + "\n")
        roster = mod._read_net_roster(self.fixture.payload)
        self.assertEqual(3, sum(status == "new" for status in roster.values()))
        self.assertEqual(0, sum(status == "added" for status in roster.values()))

    def test_roster_parser_still_rejects_unknown_status_duplicate_and_unsafe_paths(self):
        mod = self.require_implementation()
        roster_path = self.fixture.payload / "net-owners.txt"
        original = roster_path.read_text(encoding="utf-8")
        rows = original.splitlines()
        rows[0] = "created  " + rows[0].split("  ", 1)[1]
        write(roster_path, "\n".join(rows) + "\n")
        self.assertVerifyError("malformed", mod._read_net_roster, self.fixture.payload)

        rows = original.splitlines()
        rows.append(rows[0])
        write(roster_path, "\n".join(rows) + "\n")
        self.assertVerifyError("duplicate path", mod._read_net_roster, self.fixture.payload)

        rows = original.splitlines()
        rows[0] = rows[0].split("  ", 1)[0] + "  ../unsafe"
        write(roster_path, "\n".join(rows) + "\n")
        self.assertVerifyError("unsafe path|traversal", mod._read_net_roster, self.fixture.payload)

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
            "crates/thaw-runtime/src/runtime/native_values/strings.rs",
            "crates/thaw-runtime/src/runtime/promises.rs",
            "crates/thaw-runtime/src/tests.rs",
        ]


if __name__ == "__main__":
    unittest.main()
