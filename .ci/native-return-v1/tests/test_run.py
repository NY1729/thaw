"""Stdlib-only tests for the Cargo diagnostic gates and evidence state machine."""

from __future__ import annotations

import importlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest import mock

MODULE_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(MODULE_DIR))
try:
    run = importlib.import_module("run")
    verify = importlib.import_module("verify")
except ModuleNotFoundError:
    run = None
    verify = None


class FullNameFilterTests(unittest.TestCase):
    def setUp(self):
        self.original_expected = verify.EXPECTED
        self.temp = tempfile.TemporaryDirectory(prefix="thaw-full-name-filter-")
        self.root = Path(self.temp.name)
        self.evidence = self.root / "evidence"
        self.evidence.mkdir()
        self.candidate = self.root / "candidate"
        self.candidate.mkdir()
        self.target = self.root / "target"
        self.filter_name = "forced_root_"
        self.expected_names = [
            "forced_root_graph_wire_controls::forced_root_transfer_failure_rolls_back_once",
            "forced_root_graph_wire_controls::forced_root_decoded_lease_outlives_consumed_input_and_root",
        ]

    def tearDown(self):
        verify.EXPECTED = self.original_expected
        self.temp.cleanup()

    def execute(self, listed_names):
        def run_command(argv, cwd, env, log, timeout):
            if "--list" in argv:
                stdout = "".join(f"{name}: test\n" for name in listed_names) + f"{len(listed_names)} tests, 0 benchmarks\n"
            else:
                stdout = (
                    f"running {len(self.expected_names)} test(s)\n"
                    + "".join(f"test {name} ... ok\n" for name in self.expected_names)
                    + f"test result: ok. {len(self.expected_names)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n"
                )
            return {"exit_code": 0, "stdout": stdout, "timed_out": False}

        return run_command

    def run_filter(self, listed_names):
        verify.EXPECTED = dict(self.original_expected)
        verify.EXPECTED["expected_full_test_names"] = {self.filter_name: list(self.expected_names)}
        item = {
            "package": "thaw-quickjs",
            "filter": self.filter_name,
            "status": "not-run",
            "reason": "not started",
            "list": run._new_stage("filter_31_list"),
            "run": run._new_stage("filter_31_run"),
        }
        state = {"filters": [item]}
        passed = run._run_filter(
            state,
            item,
            31,
            self.execute(listed_names),
            self.evidence,
            self.target,
            self.candidate,
            time.monotonic(),
        )
        return passed, item

    def test_new_full_names_require_exact_unique_fully_qualified_list(self):
        passed, item = self.run_filter(list(self.expected_names))
        self.assertTrue(passed)
        self.assertEqual(self.expected_names, item["selected_names"])

        substituted = ["wrong_namespace::" + self.expected_names[0].rsplit("::", 1)[-1], self.expected_names[1]]
        passed, item = self.run_filter(substituted)
        self.assertFalse(passed)
        self.assertEqual("failed", item["status"])
        self.assertEqual("blocked", item["run"]["status"])

    def test_full_name_contract_accepts_bare_crate_root_test_names(self):
        self.filter_name = "graph_codec_roundtrip_preserves_negative_zero"
        self.expected_names = ["graph_codec_roundtrip_preserves_negative_zero"]
        passed, item = self.run_filter(list(self.expected_names))
        self.assertTrue(passed)
        self.assertEqual(self.expected_names, item["selected_names"])


class DiagnosticRunnerTests(unittest.TestCase):
    OLD_FILTER_COUNT = 30
    PRE_EXTENSION_FILTER_COUNT = 39
    FILTER_COUNT = 50
    PRESERVED_FILTERS = (
        ("thaw-llvm", "native_eval_then_return_"),
        ("thaw-llvm", "discarded_native_promise_union_"),
        ("thaw-llvm", "discarded_promise_results_release_only_the_selected_owned_value"),
        ("thaw-llvm", "named_and_closure_returns_retain_borrowed_promises"),
        ("thaw-llvm", "native_promise_scope_boundaries_codegen_regression_control"),
        ("thaw-llvm", "native_promise_scope_tables_restore_after_codegen_errors"),
        ("thaw-llvm", "reactive_preheader_promotions_preserve_exact_scope_and_successor_context"),
        ("thaw-llvm", "reactive_preheader_codegen_error_restores_nonempty_scope_exactly"),
        ("thaw-llvm", "stack_owner_live_merge_preserves_exact_branch_bindings"),
        ("thaw-llvm", "hir_if_live_arm_keeps_sibling_stack_binding_and_runtime_flag"),
        ("thaw-llvm", "native_promise_exception_descriptor_survives_all_cleanup_handoffs"),
        ("thaw-llvm", "nested_codegen_scope_contexts_restore_exact_nonempty_state"),
        ("thaw-llvm", "compile_lambda_restores_scope_after_real_inner_body_error"),
        ("thaw-llvm", "compile_async_lambda_restores_scope_after_real_inner_body_error"),
        ("thaw-llvm", "published_throw_keeps_fresh_native_exception_descriptor"),
        ("thaw-llvm", "text_only_throw_clears_stale_native_exception_descriptor"),
        ("thaw-llvm", "blocking_and_async_exception_handoffs_copy_native_descriptor_before_release"),
        ("thaw-llvm", "plain_string_resolver_clears_native_descriptor_from_real_typed_publisher"),
        ("thaw-llvm", "typed_native_reason_resolver_preserves_its_published_descriptor"),
        ("thaw-llvm", "pending_rethrow_keeps_text_and_descriptor_on_their_own_channels"),
        ("thaw-hir", "lowers_try_catch"),
        ("thaw-hir", "finally_separates_text_only_throws_from_fresh_published_tuples"),
        ("thaw-hir", "finally_snapshots_return_and_throw_values_before_mutation"),
        ("thaw-hir", "thaw_remaining_"),
        ("thaw-std", "typed_decode_scope_tracks_dynamic_retains_merges_and_excludes_reentry"),
        ("thaw-hir", "thaw_binding_helper_"),
        ("thaw-hir", "receiver_pattern_inference_"),
        ("thaw-hir", "thaw_rethrow_cleanup_"),
        ("thaw-hir", "error_argument_staging_"),
        ("thaw-hir", "existing_native_spread_staging_mode_false_is_unchanged"),
        ("thaw-quickjs", "forced_root_"),
        ("thaw-quickjs", "graph_codec_roundtrip_preserves_negative_zero"),
        ("thaw-quickjs", "graph_codec_roundtrip_retains_identity_and_releases_live_lease"),
        ("thaw-quickjs", "graph_codec_uses_bootstrap_intrinsics_after_global_replacement"),
        ("thaw-quickjs", "handle_registry_identity_ignores_later_object_is_override"),
        ("thaw-quickjs", "failed_exception_graph_grant_retires_producer_handle_lease"),
        ("thaw-quickjs", "exact_mixed_pre_dispatch_consumes_registered_graph_grant_once"),
        ("thaw-std", "unregistered_graph_wire_cannot_transfer_napi_lease_tokens"),
        ("thaw-std", "mutated_graph_wire_retires_only_registered_snapshot_leases"),
    )

    def require_implementation(self):
        if run is None or verify is None:
            self.fail("run.py must expose the diagnostic gate runner and verification APIs")
        return run

    def setUp(self):
        self.require_implementation()
        self.temp = tempfile.TemporaryDirectory(prefix="thaw-run-test-")
        root = Path(self.temp.name)
        self.fixture = SimpleNamespace(
            baseline=root / "baseline",
            candidate=root / "candidate",
            payload=root / "payload",
        )
        for path in (self.fixture.baseline, self.fixture.candidate, self.fixture.payload):
            path.mkdir(parents=True)
        self.original_expected = verify.EXPECTED
        self.identity_validation_patch = mock.patch.object(verify, "_validate_independent_paths", return_value=None)
        self.prepared_identity_patch = mock.patch.object(verify, "verify_prepared", return_value={"synthetic": "identity verified"})
        self.identity_validation_patch.start()
        self.prepared_identity = self.prepared_identity_patch.start()
        self.old_payload_dir = run.PAYLOAD_DIR
        run.PAYLOAD_DIR = self.fixture.payload
        self.evidence = root / "run-evidence"

    def tearDown(self):
        if verify is not None and hasattr(self, "original_expected"):
            verify.EXPECTED = self.original_expected
        if hasattr(self, "identity_validation_patch"):
            self.identity_validation_patch.stop()
        if hasattr(self, "prepared_identity_patch"):
            self.prepared_identity_patch.stop()
        if run is not None and hasattr(self, "old_payload_dir"):
            run.PAYLOAD_DIR = self.old_payload_dir
        self.temp.cleanup()

    def successful_executor(
        self,
        *,
        fail=None,
        timeout_no_run=None,
        list_zero=None,
        ignored=None,
        raise_on=None,
        raise_filter=None,
        raise_filter_when="list",
        timeout_filter=None,
        timeout_filter_when="list",
        list_variant=None,
        list_variant_filter=None,
        run_variant=None,
        run_variant_filter=None,
        failed_list_filter=None,
        failed_run_filter=None,
    ):
        calls = []

        def filter_name_for(argv):
            return next((name for name in run.TEST_FILTERS if name in argv), "control")

        def test_names_for(filter_name):
            full_names = verify.EXPECTED.get("expected_full_test_names", {}).get(filter_name)
            if full_names is not None:
                return list(full_names)
            leaves = verify.EXPECTED.get("expected_test_names", {}).get(filter_name)
            if leaves:
                package = verify.EXPECTED["test_filters"][run.TEST_FILTERS.index(filter_name)]["package"].replace("-", "_")
                return [f"{package}::tests::{leaf}" for leaf in leaves]
            return [filter_name + "::selected"]

        def execute(argv, cwd, env, log, timeout_seconds):
            argv = list(argv)
            call = {"argv": argv, "cwd": Path(cwd), "env": dict(env), "log": Path(log), "timeout": timeout_seconds}
            calls.append(call)
            lane = "baseline" if Path(cwd) == self.fixture.baseline else "candidate"
            if raise_on == f"{lane}-check" and argv[1:2] == ["check"]:
                raise RuntimeError("injected executor failure")
            no_run_package = {
                "quickjs-no-run": "thaw-quickjs",
                "runtime-no-run": "thaw-runtime",
            }.get(raise_on)
            if no_run_package is not None and "--no-run" in argv and no_run_package in argv:
                raise RuntimeError(f"injected {raise_on.removesuffix('-no-run')} no-run exception")
            if fail == f"{lane}-check" and argv[1:2] == ["check"]:
                return {"exit_code": 101, "stdout": "error: injected check failure\n", "timed_out": False}
            if fail == "llvm-no-run" and "--no-run" in argv and "thaw-llvm" in argv:
                return {"exit_code": 101, "stdout": "error: injected LLVM no-run failure\n", "timed_out": False}
            if fail == "hir-no-run" and "--no-run" in argv and "thaw-hir" in argv:
                return {"exit_code": 101, "stdout": "error: injected HIR no-run failure\n", "timed_out": False}
            if fail == "quickjs-no-run" and "--no-run" in argv and "thaw-quickjs" in argv:
                return {"exit_code": 101, "stdout": "error: injected QuickJS no-run failure\n", "timed_out": False}
            if fail == "runtime-no-run" and "--no-run" in argv and "thaw-runtime" in argv:
                return {"exit_code": 101, "stdout": "error: injected runtime no-run failure\n", "timed_out": False}
            if timeout_no_run is not None and "--no-run" in argv and timeout_no_run in argv:
                return {"exit_code": -signal.SIGTERM, "stdout": "no-run timed out\n", "timed_out": True}
            if fail == "std-no-run" and "--no-run" in argv and "thaw-std" in argv:
                return {"exit_code": 101, "stdout": "error: injected std no-run failure\n", "timed_out": False}
            filter_name = filter_name_for(argv)
            is_list = "--list" in argv
            is_filter_run = argv[1:2] == ["test"] and "--no-run" not in argv and not is_list
            if raise_filter == filter_name and ((raise_filter_when == "list" and is_list) or (raise_filter_when == "run" and is_filter_run)):
                raise RuntimeError("injected focused-control executor failure")
            if timeout_filter == filter_name and ((timeout_filter_when == "list" and is_list) or (timeout_filter_when == "run" and is_filter_run)):
                return {"exit_code": -signal.SIGTERM, "stdout": "timed out\n", "timed_out": True}
            if fail == "failed-list" and "--list" in argv and (failed_list_filter is None or failed_list_filter == filter_name):
                return {"exit_code": 1, "stdout": "list failed\n", "timed_out": False}
            if fail == "failed-run" and "--list" not in argv and "--no-run" not in argv and argv[1:2] == ["test"]:
                if failed_run_filter is None or failed_run_filter == filter_name:
                    names = test_names_for(filter_name)
                    return {
                        "exit_code": 101,
                        "stdout": "".join(f"test {name} ... FAILED\n" for name in names) + f"test result: FAILED. 0 passed; {len(names)} failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n",
                        "timed_out": False,
                    }
            if argv[1:2] == ["check"]:
                return {"exit_code": 0, "stdout": "Finished dev profile\n", "timed_out": False}
            if "--no-run" in argv:
                return {"exit_code": 0, "stdout": "Finished test profile\n", "timed_out": False}
            if "--list" in argv:
                if list_zero == filter_name:
                    return {"exit_code": 0, "stdout": "0 tests, 0 benchmarks\n", "timed_out": False}
                expected = test_names_for(filter_name)
                if verify.EXPECTED.get("expected_full_test_names", {}).get(filter_name) is not None:
                    names = list(expected)
                    variant = list_variant if list_variant_filter is None or list_variant_filter == filter_name else None
                    if variant == "substitute":
                        names[0] = names[0] + "_substituted_name"
                    if variant == "wrong_namespace":
                        names[0] = "wrong_namespace::" + names[0].rsplit("::", 1)[-1]
                    if variant == "wrong_leaf":
                        names[0] = names[0].rsplit("::", 1)[0] + "::wrong_leaf"
                    if variant == "short":
                        names = names[:-1]
                    if variant == "duplicate":
                        names.append(names[0])
                    return {"exit_code": 0, "stdout": "".join(f"{name}: test\n" for name in names) + f"{len(names)} tests, 0 benchmarks\n", "timed_out": False}
                legacy = verify.EXPECTED.get("expected_test_names", {}).get(filter_name)
                if legacy is not None:
                    package = verify.EXPECTED["test_filters"][run.TEST_FILTERS.index(filter_name)]["package"].replace("-", "_")
                    leaves = list(legacy)
                    variant = list_variant if list_variant_filter is None or list_variant_filter == filter_name else None
                    if variant in ("substitute", "wrong_leaf"):
                        leaves[0] = filter_name + "substituted_name"
                    if variant == "short":
                        leaves = leaves[:-1]
                    names = [f"{package}::tests::{leaf}" for leaf in leaves]
                    if variant == "wrong_namespace":
                        names[0] = "wrong_namespace::tests::" + names[0].rsplit("::", 1)[-1]
                    if variant == "duplicate":
                        names.append(names[0])
                    return {"exit_code": 0, "stdout": "".join(f"{name}: test\n" for name in names) + f"{len(names)} tests, 0 benchmarks\n", "timed_out": False}
                names = test_names_for(filter_name)
                return {"exit_code": 0, "stdout": "".join(f"{name}: test\n" for name in names) + f"{len(names)} tests, 0 benchmarks\n", "timed_out": False}
            names = test_names_for(filter_name)
            if ignored == filter_name:
                return {
                    "exit_code": 0,
                    "stdout": "".join(f"test {name} ... ignored\n" for name in names) + f"test result: ok. 0 passed; 0 failed; {len(names)} ignored; 0 measured; 0 filtered out; finished in 0.00s\n",
                    "timed_out": False,
                }
            variant = run_variant if run_variant_filter is None or run_variant_filter == filter_name else None
            if verify.EXPECTED.get("expected_full_test_names", {}).get(filter_name) is not None and variant == "wrong_name":
                names[0] = "wrong_namespace::" + names[0].rsplit("::", 1)[-1]
            if verify.EXPECTED.get("expected_full_test_names", {}).get(filter_name) is not None and variant == "duplicate":
                names.append(names[0])
            full_expected = verify.EXPECTED.get("expected_full_test_names", {}).get(filter_name)
            legacy_expected = verify.EXPECTED.get("expected_test_names", {}).get(filter_name)
            has_expected = full_expected is not None or legacy_expected is not None
            if has_expected and variant == "malformed":
                return {"exit_code": 0, "stdout": "running tests\n" + "".join(f"test {name} ... ok\n" for name in names), "timed_out": False}
            if has_expected and variant == "partial":
                partial_names = names[:-1]
                return {
                    "exit_code": 0,
                    "stdout": f"running {len(partial_names)} test(s)\n" + "".join(f"test {name} ... ok\n" for name in partial_names) + f"test result: ok. {len(names)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n",
                    "timed_out": False,
                }
            return {
                "exit_code": 0,
                "stdout": f"running {len(names)} test(s)\n" + "".join(f"test {name} ... ok\n" for name in names) + f"test result: ok. {len(names)} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n",
                "timed_out": False,
            }

        execute.calls = calls
        return execute

    def run_case(self, executor, **kwargs):
        return run.run_validation(
            self.fixture.baseline,
            self.fixture.candidate,
            self.evidence,
            executor,
            **kwargs,
        )

    def test_baseline_failure_does_not_suppress_candidate_or_controls(self):
        executor = self.successful_executor(fail="baseline-check")
        result = self.run_case(executor)
        self.assertEqual("failed", result["stages"]["baseline_check"]["status"])
        self.assertEqual("passed", result["stages"]["candidate_check"]["status"])
        self.assertEqual(self.FILTER_COUNT, len(result["filters"]))
        self.assertTrue(all(item["status"] == "passed" for item in result["filters"]))
        self.assertEqual(1, result["exit_code"])

    def test_candidate_check_failure_blocks_predecessor_gates_and_controls(self):
        result = self.run_case(self.successful_executor(fail="candidate-check"))
        self.assertEqual("passed", result["stages"]["baseline_check"]["status"])
        self.assertEqual("failed", result["stages"]["candidate_check"]["status"])
        for name in ("llvm_no_run", "hir_no_run"):
            self.assertEqual("blocked", result["stages"][name]["status"])
        self.assertEqual("passed", result["stages"]["quickjs_no_run"]["status"])
        self.assertEqual("passed", result["stages"]["std_no_run"]["status"])
        self.assertTrue(all(item["status"] == "blocked" for item in result["filters"][: self.OLD_FILTER_COUNT]))
        self.assertTrue(all(item["status"] == "passed" for item in result["filters"][self.OLD_FILTER_COUNT :]))
        self.assertEqual("failed", result["overall"]["status"])

    def test_llvm_no_run_failure_blocks_hir_and_old_controls(self):
        result = self.run_case(self.successful_executor(fail="llvm-no-run"))
        self.assertEqual("failed", result["stages"]["llvm_no_run"]["status"])
        self.assertEqual("blocked", result["stages"]["hir_no_run"]["status"])
        self.assertEqual("passed", result["stages"]["quickjs_no_run"]["status"])
        self.assertEqual("passed", result["stages"]["std_no_run"]["status"])
        self.assertTrue(all(item["status"] == "blocked" for item in result["filters"][: self.OLD_FILTER_COUNT]))
        self.assertTrue(all(item["status"] == "passed" for item in result["filters"][self.OLD_FILTER_COUNT :]))
        self.assertEqual("failed", result["overall"]["status"])

    def test_hir_no_run_failure_blocks_old_controls_but_leaves_new_lane_successful(self):
        result = self.run_case(self.successful_executor(fail="hir-no-run"))
        self.assertEqual("failed", result["stages"]["hir_no_run"]["status"])
        self.assertEqual("passed", result["stages"]["std_no_run"]["status"])
        self.assertTrue(all(item["status"] == "blocked" for item in result["filters"][: self.OLD_FILTER_COUNT]))
        self.assertTrue(all(item["status"] == "passed" for item in result["filters"][self.OLD_FILTER_COUNT :]))
        self.assertEqual("failed", result["overall"]["status"])

    def test_std_no_run_failure_blocks_old_and_std_controls_only(self):
        result = self.run_case(self.successful_executor(fail="std-no-run"))
        self.assertEqual("passed", result["stages"]["hir_no_run"]["status"])
        self.assertEqual("failed", result["stages"]["std_no_run"]["status"])
        self.assertTrue(all(item["status"] == "passed" for item in result["filters"][self.OLD_FILTER_COUNT : self.OLD_FILTER_COUNT + 7]))
        self.assertEqual(
            ["passed", "passed", "passed", "blocked", "blocked", "passed", "passed", "passed", "blocked", "blocked"],
            [item["status"] for item in result["filters"][39:49]],
        )
        self.assertTrue(all(item["status"] == "blocked" for item in result["filters"][: self.OLD_FILTER_COUNT]))
        self.assertEqual("failed", result["overall"]["status"])

    def test_zero_old_list_selection_fails_and_blocks_remaining_old_filters(self):
        first = run.TEST_FILTERS[0]
        result = self.run_case(self.successful_executor(list_zero=first))
        self.assertEqual("failed", result["filters"][0]["status"])
        self.assertIn("selected zero", result["filters"][0]["reason"])
        self.assertTrue(all(item["status"] == "blocked" for item in result["filters"][1 : self.OLD_FILTER_COUNT]))
        self.assertTrue(all(item["status"] == "passed" for item in result["filters"][self.OLD_FILTER_COUNT :]))

    def test_ignored_only_actual_execution_is_not_a_pass(self):
        first = run.TEST_FILTERS[0]
        result = self.run_case(self.successful_executor(ignored=first))
        self.assertEqual("failed", result["filters"][0]["status"])
        self.assertIn("ignored", result["filters"][0]["reason"])
        self.assertTrue(all(item["status"] == "blocked" for item in result["filters"][1 : self.OLD_FILTER_COUNT]))
        self.assertTrue(all(item["status"] == "passed" for item in result["filters"][self.OLD_FILTER_COUNT :]))

    def test_failed_old_list_and_control_stop_later_old_filters(self):
        list_result = self.run_case(self.successful_executor(fail="failed-list", failed_list_filter=run.TEST_FILTERS[0]))
        self.assertEqual("failed", list_result["filters"][0]["status"])
        self.assertEqual("blocked", list_result["filters"][1]["status"])
        self.assertTrue(all(item["status"] == "passed" for item in list_result["filters"][self.OLD_FILTER_COUNT :]))
        run_result = self.run_case(self.successful_executor(fail="failed-run", failed_run_filter=run.TEST_FILTERS[0]))
        self.assertEqual("failed", run_result["filters"][0]["status"])
        self.assertIn("failed", run_result["filters"][0]["reason"])
        self.assertTrue(all(item["status"] == "blocked" for item in run_result["filters"][1 : self.OLD_FILTER_COUNT]))
        self.assertTrue(all(item["status"] == "passed" for item in run_result["filters"][self.OLD_FILTER_COUNT :]))

    def test_signal_and_timeout_are_failures_not_success(self):
        def signal_executor(argv, cwd, env, log, timeout):
            if argv[1:2] == ["check"] and Path(cwd) == self.fixture.baseline:
                return {"exit_code": -signal.SIGKILL, "stdout": "killed\n", "timed_out": False}
            return self.successful_executor()(argv, cwd, env, log, timeout)

        result = self.run_case(signal_executor)
        self.assertEqual("failed", result["stages"]["baseline_check"]["status"])
        self.assertEqual(signal.SIGKILL, result["stages"]["baseline_check"]["signal"])
        self.assertEqual("passed", result["stages"]["candidate_check"]["status"])

        def timeout_executor(argv, cwd, env, log, timeout):
            if argv[1:2] == ["check"] and Path(cwd) == self.fixture.baseline:
                return {"exit_code": -signal.SIGTERM, "stdout": "timeout\n", "timed_out": True}
            return self.successful_executor()(argv, cwd, env, log, timeout)

        timed = self.run_case(timeout_executor)
        self.assertEqual("timed-out", timed["stages"]["baseline_check"]["status"])
        self.assertEqual("passed", timed["stages"]["candidate_check"]["status"])

    def test_wall_budget_expiry_is_explicit_and_keeps_candidate_stages_unrun(self):
        with mock.patch.object(run, "WALL_BUDGET_SECONDS", 0):
            result = self.run_case(self.successful_executor())
        self.assertEqual("blocked", result["stages"]["baseline_check"]["status"])
        self.assertIn("wall budget", result["stages"]["baseline_check"]["reason"])
        self.assertEqual("blocked", result["stages"]["candidate_check"]["status"])
        self.assertIn("wall budget", result["stages"]["candidate_check"]["reason"])
        self.assertEqual("blocked", result["overall"]["status"])
        self.assertTrue(all(item["status"] == "blocked" for item in result["filters"]))
        self.assertTrue((self.evidence / "validation.json").is_file())

    def test_executor_exception_keeps_log_and_still_runs_independent_candidate_check(self):
        executor = self.successful_executor(raise_on="baseline-check")
        result = self.run_case(executor)
        self.assertEqual("failed", result["stages"]["baseline_check"]["status"])
        self.assertEqual("passed", result["stages"]["candidate_check"]["status"])
        self.assertTrue(Path(result["stages"]["baseline_check"]["log"]).is_file())
        self.assertTrue((self.evidence / "validation.json").is_file())

    def test_identity_failure_runs_zero_cargo_commands_and_marks_all_stages(self):
        self.prepared_identity.side_effect = verify.VerificationError("injected prepared identity mismatch")
        executor = self.successful_executor()
        result = self.run_case(executor)
        self.assertEqual([], executor.calls)
        self.assertEqual("failed", result["stages"]["identity"]["status"])
        self.assertEqual("blocked", result["stages"]["runtime_no_run"]["status"])
        self.assertTrue(all(stage["status"] == "blocked" for stage in result["stages"].values() if stage["name"] not in ("identity",)))

    def test_success_uses_50_pinned_filters_locked_and_separate_target_dirs(self):
        executor = self.successful_executor()
        result = self.run_case(executor)
        self.assertEqual(self.FILTER_COUNT, len(result["filters"]))
        self.assertEqual(self.PRESERVED_FILTERS, tuple((item["package"], item["filter"]) for item in result["filters"][:self.PRE_EXTENSION_FILTER_COUNT]))
        self.assertEqual(20, sum(f["package"] == "thaw-llvm" for f in result["filters"]))
        self.assertEqual(9, sum(f["package"] == "thaw-hir" for f in result["filters"]))
        self.assertEqual(7, sum(f["package"] == "thaw-quickjs" for f in result["filters"]))
        self.assertEqual(8, sum(f["package"] == "thaw-std" for f in result["filters"]))
        self.assertEqual(6, sum(f["package"] == "thaw-runtime" for f in result["filters"]))
        cargo_calls = executor.calls
        self.assertTrue(cargo_calls)
        self.assertTrue(all("--locked" in call["argv"] for call in cargo_calls))
        target_dirs = {call["env"].get("CARGO_TARGET_DIR") for call in cargo_calls}
        self.assertEqual(2, len(target_dirs))
        self.assertNotEqual(*sorted(target_dirs))
        run_calls = [call["argv"] for call in cargo_calls if call["argv"][1:2] == ["test"] and "--no-run" not in call["argv"] and "--list" not in call["argv"]]
        self.assertEqual(self.FILTER_COUNT, len(run_calls))
        self.assertTrue(all("--test-threads=1" in argv for argv in run_calls))
        no_runs = [call["argv"] for call in cargo_calls if "--no-run" in call["argv"]]
        self.assertEqual(["thaw-quickjs", "thaw-std", "thaw-runtime", "thaw-llvm", "thaw-hir"], [argv[argv.index("-p") + 1] for argv in no_runs])
        self.assertEqual(1, sum(argv[argv.index("-p") + 1] == "thaw-quickjs" for argv in no_runs))
        self.assertEqual(1, sum(argv[argv.index("-p") + 1] == "thaw-std" for argv in no_runs))
        self.assertEqual(1, sum(argv[argv.index("-p") + 1] == "thaw-runtime" for argv in no_runs))
        listed_filters = [
            argv[argv.index("--lib") + 1]
            for call in cargo_calls
            if (argv := call["argv"])[1:2] == ["test"] and "--list" in argv
        ]
        self.assertEqual(list(run.TEST_FILTERS[30:]) + list(run.TEST_FILTERS[:30]), listed_filters)
        first_llvm_check = next(
            index
            for index, call in enumerate(cargo_calls)
            if call["argv"][1:2] == ["check"] and "thaw-llvm" in call["argv"]
        )
        for package in ("thaw-quickjs", "thaw-std"):
            preflight = next(
                index
                for index, call in enumerate(cargo_calls)
                if "--no-run" in call["argv"] and call["argv"][call["argv"].index("-p") + 1] == package
            )
            self.assertLess(preflight, first_llvm_check, f"{package} preflight must precede LLVM checks")
        new_control_indices = [
            index
            for filter_name in run.TEST_FILTERS[30:]
            for index, call in enumerate(cargo_calls)
            if call["argv"][1:2] == ["test"] and "--no-run" not in call["argv"] and filter_name in call["argv"]
        ]
        self.assertEqual(40, len(new_control_indices))
        self.assertTrue(all(index < first_llvm_check for index in new_control_indices))
        runtime_preflight = next(
            index for index, call in enumerate(cargo_calls)
            if "--no-run" in call["argv"] and call["argv"][call["argv"].index("-p") + 1] == "thaw-runtime"
        )
        wire_control_indices = [
            index for index, call in enumerate(cargo_calls)
            if call["argv"][1:2] == ["test"] and "--no-run" not in call["argv"]
            and any(name in call["argv"] for name in run.TEST_FILTERS[30:39])
        ]
        dependency_control_indices = [
            index for index, call in enumerate(cargo_calls)
            if call["argv"][1:2] == ["test"] and "--no-run" not in call["argv"]
            and any(name in call["argv"] for name in run.TEST_FILTERS[39:50])
        ]
        self.assertTrue(wire_control_indices)
        self.assertTrue(dependency_control_indices)
        self.assertLess(max(wire_control_indices), runtime_preflight)
        self.assertLess(runtime_preflight, min(dependency_control_indices))
        self.assertLess(max(dependency_control_indices), first_llvm_check)

    def test_previous_hir_filters_preserve_leaf_name_contracts_and_execution_evidence(self):
        result = self.run_case(self.successful_executor())
        hir_filters = result["filters"][25:30]
        self.assertEqual(
            [
                "thaw_binding_helper_",
                "receiver_pattern_inference_",
                "thaw_rethrow_cleanup_",
                "error_argument_staging_",
                "existing_native_spread_staging_mode_false_is_unchanged",
            ],
            [item["filter"] for item in hir_filters],
        )
        self.assertEqual([2, 10, 7, 7, 1], [len(verify.EXPECTED["expected_test_names"][item["filter"]]) for item in hir_filters])
        for item in hir_filters:
            expected = verify.EXPECTED["expected_test_names"][item["filter"]]
            self.assertEqual(expected, [name.rsplit("::", 1)[-1] for name in item["selected_names"]])
            self.assertEqual(item["selected_names"], item["execution_names"])

    def test_appended_twenty_groups_match_exact_full_names_without_leaf_normalization(self):
        result = self.run_case(self.successful_executor())
        expected_full = verify.EXPECTED["expected_full_test_names"]
        new_groups = result["filters"][self.OLD_FILTER_COUNT :]
        self.assertEqual(20, len(new_groups))
        self.assertEqual(set(expected_full), {item["filter"] for item in new_groups})
        for item in new_groups:
            expected = expected_full[item["filter"]]
            self.assertEqual(len(expected), len(set(expected)))
            self.assertEqual(sorted(expected), sorted(item["selected_names"]))
            self.assertEqual(sorted(expected), sorted(item["execution_names"]))

    def test_new_quickjs_and_std_preflights_run_before_old_llvm_hir_gates(self):
        executor = self.successful_executor()
        result = self.run_case(executor)
        self.assertEqual("passed", result["stages"]["quickjs_no_run"]["status"])
        self.assertEqual("passed", result["stages"]["std_no_run"]["status"])
        self.assertEqual("passed", result["stages"]["llvm_no_run"]["status"])
        self.assertEqual("passed", result["stages"]["hir_no_run"]["status"])
        self.assertEqual("passed", result["overall"]["status"])
        no_run_calls = [call["argv"] for call in executor.calls if "--no-run" in call["argv"]]
        names = [argv[argv.index("-p") + 1] for argv in no_run_calls]
        self.assertLess(names.index("thaw-quickjs"), names.index("thaw-llvm"))
        self.assertLess(names.index("thaw-std"), names.index("thaw-llvm"))

    def test_quickjs_and_std_preflight_failures_are_scoped_to_their_crate(self):
        quickjs = self.run_case(self.successful_executor(fail="quickjs-no-run"))
        self.assertEqual("failed", quickjs["stages"]["quickjs_no_run"]["status"])
        self.assertEqual("passed", quickjs["stages"]["std_no_run"]["status"])
        self.assertTrue(all(item["status"] == "blocked" for item in quickjs["filters"][30:37]))
        self.assertTrue(all(item["status"] == "passed" for item in quickjs["filters"][37:39]))
        self.assertTrue(all(item["status"] == "passed" for item in quickjs["filters"][39:50]))
        self.assertTrue(all(item["status"] == "passed" for item in quickjs["filters"][:30]))
        self.assertEqual("failed", quickjs["overall"]["status"])

        self.evidence = Path(self.temp.name) / "run-evidence-std-no-run-failure"
        std = self.run_case(self.successful_executor(fail="std-no-run"))
        self.assertEqual("passed", std["stages"]["quickjs_no_run"]["status"])
        self.assertEqual("failed", std["stages"]["std_no_run"]["status"])
        self.assertTrue(all(item["status"] == "passed" for item in std["filters"][30:37]))
        self.assertTrue(all(item["status"] == "blocked" for item in std["filters"][37:39]))
        self.assertEqual(
            ["passed", "passed", "passed", "blocked", "blocked", "passed", "passed", "passed", "blocked", "blocked"],
            [item["status"] for item in std["filters"][39:49]],
        )
        self.assertEqual("blocked", std["filters"][49]["status"])
        self.assertEqual("failed", std["overall"]["status"])

    def test_runtime_no_run_failure_timeout_and_exception_leave_std_and_quickjs_eligible(self):
        cases = (
            ("runtime compile failure", self.successful_executor(fail="runtime-no-run"), "failed"),
            ("runtime compile timeout", self.successful_executor(timeout_no_run="thaw-runtime"), "timed-out"),
            ("runtime compile exception", self.successful_executor(raise_on="runtime-no-run"), "failed"),
        )
        for label, executor, expected_status in cases:
            with self.subTest(label=label):
                self.evidence = Path(self.temp.name) / f"run-evidence-{label.replace(' ', '-') }"
                result = self.run_case(executor)
                self.assertEqual(expected_status, result["stages"]["runtime_no_run"]["status"])
                self.assertTrue(all(item["status"] == "passed" for item in result["filters"][30:39]))
                self.assertTrue(all(item["status"] == "blocked" for item in result["filters"][39:42]))
                self.assertTrue(all(item["status"] == "blocked" for item in result["filters"][44:47]))
                self.assertTrue(all(item["status"] == "passed" for item in result["filters"][42:44]))
                self.assertTrue(all(item["status"] == "passed" for item in result["filters"][47:49]))
                self.assertEqual("passed", result["filters"][49]["status"])
                self.assertTrue(all(item["status"] == "passed" for item in result["filters"][:30]))
                self.assertEqual("failed", result["overall"]["status"])

    def test_new_dependency_group_failure_timeout_and_exception_do_not_suppress_siblings(self):
        first_dependency = run.TEST_FILTERS[self.PRE_EXTENSION_FILTER_COUNT]
        cases = (
            ("failed run", self.successful_executor(fail="failed-run", failed_run_filter=first_dependency), "failed"),
            ("timeout", self.successful_executor(timeout_filter=first_dependency), "timed-out"),
            ("executor exception", self.successful_executor(raise_filter=first_dependency), "failed"),
        )
        for label, executor, expected_status in cases:
            with self.subTest(label=label):
                self.evidence = Path(self.temp.name) / f"run-evidence-dependency-{label.replace(' ', '-') }"
                result = self.run_case(executor)
                self.assertEqual(expected_status, result["filters"][39]["status"])
                self.assertTrue(all(item["status"] == "passed" for item in result["filters"][40:50]))
                self.assertEqual("failed", result["overall"]["status"])

    def test_new_dependency_exact_controls_have_one_expected_name_and_one_nonignored_outcome(self):
        result = self.run_case(self.successful_executor())
        expected_full = verify.EXPECTED["expected_full_test_names"]
        dependency_groups = result["filters"][self.PRE_EXTENSION_FILTER_COUNT :]
        self.assertEqual(11, len(dependency_groups))
        for item in dependency_groups:
            with self.subTest(filter=item["filter"]):
                expected = expected_full[item["filter"]]
                self.assertEqual(1, len(expected))
                self.assertEqual(expected, item["selected_names"])
                self.assertEqual(1, item["selected_count"])
                self.assertEqual(expected, item["execution_names"])
                self.assertEqual({"passed": 1, "failed": 0, "ignored": 0}, item["execution"])

    def test_appended_http_control_is_exact_and_uses_the_existing_std_preflight(self):
        self.assertEqual(
            ("thaw-std", "http::tests::peer_send_eof_preserves_pending_streamed_response"),
            (verify.EXPECTED["test_filters"][49]["package"], verify.EXPECTED["test_filters"][49]["filter"]),
        )
        result = self.run_case(self.successful_executor())
        http_group = result["filters"][49]
        self.assertEqual("passed", http_group["status"])
        self.assertEqual(["http::tests::peer_send_eof_preserves_pending_streamed_response"], http_group["selected_names"])
        self.assertEqual(http_group["selected_names"], http_group["execution_names"])
        self.evidence = Path(self.temp.name) / "run-evidence-http-std-gate"
        blocked = self.run_case(self.successful_executor(fail="std-no-run"))
        self.assertEqual("blocked", blocked["filters"][49]["status"])
        self.assertIn("thaw-std test no-run failed", blocked["filters"][49]["reason"])

    def test_new_dependency_list_and_execution_reject_substitution_duplicates_partials_and_ignored(self):
        first_dependency = run.TEST_FILTERS[self.PRE_EXTENSION_FILTER_COUNT]
        for variant in ("wrong_namespace", "wrong_leaf", "duplicate", "short", "substitute"):
            with self.subTest(list_variant=variant):
                self.evidence = Path(self.temp.name) / f"run-evidence-dependency-list-{variant}"
                result = self.run_case(self.successful_executor(list_variant=variant, list_variant_filter=first_dependency))
                self.assertEqual("failed", result["filters"][39]["status"])
                self.assertTrue(all(item["status"] == "passed" for item in result["filters"][40:50]))
        for variant in ("wrong_name", "duplicate", "partial", "malformed"):
            with self.subTest(run_variant=variant):
                self.evidence = Path(self.temp.name) / f"run-evidence-dependency-run-{variant}"
                result = self.run_case(self.successful_executor(run_variant=variant, run_variant_filter=first_dependency))
                self.assertEqual("failed", result["filters"][39]["status"])
                self.assertTrue(all(item["status"] == "passed" for item in result["filters"][40:50]))
        self.evidence = Path(self.temp.name) / "run-evidence-dependency-ignored"
        ignored = self.run_case(self.successful_executor(ignored=first_dependency))
        self.assertEqual("failed", ignored["filters"][39]["status"])
        self.assertIn("ignored", ignored["filters"][39]["reason"])
        self.assertTrue(all(item["status"] == "passed" for item in ignored["filters"][40:50]))

    def test_quickjs_no_run_exception_still_allows_std_controls(self):
        result = self.run_case(self.successful_executor(raise_on="quickjs-no-run"))
        self.assertEqual("failed", result["stages"]["quickjs_no_run"]["status"])
        self.assertEqual("passed", result["stages"]["std_no_run"]["status"])
        self.assertTrue(all(item["status"] == "blocked" for item in result["filters"][30:37]))
        self.assertTrue(all(item["status"] == "passed" for item in result["filters"][37:39]))
        self.assertEqual("failed", result["overall"]["status"])

    def test_quickjs_no_run_timeout_still_allows_std_controls(self):
        result = self.run_case(self.successful_executor(timeout_no_run="thaw-quickjs"))
        self.assertEqual("timed-out", result["stages"]["quickjs_no_run"]["status"])
        self.assertEqual("passed", result["stages"]["std_no_run"]["status"])
        self.assertTrue(all(item["status"] == "blocked" for item in result["filters"][30:37]))
        self.assertTrue(all(item["status"] == "passed" for item in result["filters"][37:39]))
        self.assertEqual("failed", result["overall"]["status"])

    def test_first_new_group_run_failure_does_not_suppress_groups_32_through_39(self):
        first_new = run.TEST_FILTERS[self.OLD_FILTER_COUNT]
        result = self.run_case(self.successful_executor(fail="failed-run", failed_run_filter=first_new))
        self.assertEqual("failed", result["filters"][30]["status"])
        self.assertTrue(all(item["status"] == "passed" for item in result["filters"][31:39]))
        self.assertEqual("failed", result["overall"]["status"])

    def test_first_new_group_list_substitution_does_not_suppress_groups_32_through_39(self):
        first_new = run.TEST_FILTERS[self.OLD_FILTER_COUNT]
        result = self.run_case(
            self.successful_executor(list_variant="wrong_namespace", list_variant_filter=first_new)
        )
        self.assertEqual("failed", result["filters"][30]["status"])
        self.assertIn("expected", result["filters"][30]["reason"])
        self.assertTrue(all(item["status"] == "passed" for item in result["filters"][31:39]))
        self.assertEqual("failed", result["overall"]["status"])

    def test_first_new_group_timeout_does_not_suppress_groups_32_through_39(self):
        first_new = run.TEST_FILTERS[self.OLD_FILTER_COUNT]
        result = self.run_case(self.successful_executor(timeout_filter=first_new))
        self.assertEqual("timed-out", result["filters"][30]["status"])
        self.assertEqual("blocked", result["filters"][30]["run"]["status"])
        self.assertTrue(all(item["status"] == "passed" for item in result["filters"][31:39]))
        self.assertEqual("failed", result["overall"]["status"])

    def test_first_new_group_executor_exception_does_not_suppress_groups_32_through_39(self):
        first_new = run.TEST_FILTERS[self.OLD_FILTER_COUNT]
        result = self.run_case(self.successful_executor(raise_filter=first_new))
        self.assertEqual("failed", result["filters"][30]["status"])
        self.assertEqual("failed", result["filters"][30]["list"]["status"])
        self.assertEqual("blocked", result["filters"][30]["run"]["status"])
        self.assertTrue(all(item["status"] == "passed" for item in result["filters"][31:39]))
        self.assertEqual("failed", result["overall"]["status"])

    def test_timeout_between_list_and_run_blocks_substeps_and_later_groups_honestly(self):
        first_new = run.TEST_FILTERS[self.OLD_FILTER_COUNT]
        executor = self.successful_executor()

        def expire_after_first_new_list(argv, cwd, env, log, timeout):
            result = executor(argv, cwd, env, log, timeout)
            if argv[1:2] == ["test"] and "--list" in argv and first_new in argv:
                run.WALL_BUDGET_SECONDS = 0
            return result

        with mock.patch.object(run, "WALL_BUDGET_SECONDS", 70 * 60):
            result = self.run_case(expire_after_first_new_list)
        first = result["filters"][30]
        self.assertEqual("passed", first["list"]["status"])
        self.assertEqual("blocked", first["run"]["status"])
        self.assertIn("wall budget", first["run"]["reason"])
        self.assertTrue(all(item["status"] == "blocked" for item in result["filters"][31:39]))
        self.assertEqual("blocked", result["overall"]["status"])

    def test_new_full_name_list_rejects_namespace_leaf_substitution_duplicates_and_partials(self):
        first_new = run.TEST_FILTERS[self.OLD_FILTER_COUNT]
        for variant in ("wrong_namespace", "wrong_leaf", "duplicate", "short", "substitute"):
            self.evidence = Path(self.temp.name) / f"run-evidence-full-list-{variant}"
            result = self.run_case(
                self.successful_executor(list_variant=variant, list_variant_filter=first_new)
            )
            self.assertEqual("failed", result["filters"][30]["status"], variant)
            self.assertTrue(all(item["status"] == "passed" for item in result["filters"][31:39]))

    def test_new_full_name_execution_rejects_wrong_namespace_and_partial_or_ignored_results(self):
        first_new = run.TEST_FILTERS[self.OLD_FILTER_COUNT]
        for variant in ("wrong_name", "partial", "malformed"):
            self.evidence = Path(self.temp.name) / f"run-evidence-full-run-{variant}"
            result = self.run_case(
                self.successful_executor(run_variant=variant, run_variant_filter=first_new)
            )
            self.assertEqual("failed", result["filters"][30]["status"], variant)
            self.assertTrue(all(item["status"] == "passed" for item in result["filters"][31:39]))

        self.evidence = Path(self.temp.name) / "run-evidence-full-run-ignored"
        ignored = self.run_case(self.successful_executor(ignored=first_new))
        self.assertEqual("failed", ignored["filters"][30]["status"])
        self.assertIn("ignored", ignored["filters"][30]["reason"])
        self.assertTrue(all(item["status"] == "passed" for item in ignored["filters"][31:39]))

    def test_post_run_identity_failure_overrides_passing_controls(self):
        self.prepared_identity.side_effect = [
            {"synthetic": "identity verified"},
            verify.VerificationError("injected post-run identity mismatch"),
        ]
        result = self.run_case(self.successful_executor())
        self.assertEqual("passed", result["filters"][30]["status"])
        self.assertEqual("failed", result["post_run_identity"]["status"])
        self.assertEqual("failed", result["overall"]["status"])

    def test_previous_hir_filters_reject_substituted_or_duplicate_listed_names(self):
        for filter_name in run.TEST_FILTERS[25:30]:
            for variant in ("substitute", "duplicate", "short"):
                result = self.run_case(self.successful_executor(list_variant=variant, list_variant_filter=filter_name))
                index = run.TEST_FILTERS.index(filter_name)
                self.assertEqual("failed", result["filters"][index]["status"])
                self.assertIn("expected", result["filters"][index]["reason"])
                if index + 1 < self.OLD_FILTER_COUNT:
                    self.assertEqual("blocked", result["filters"][index + 1]["status"])
                self.evidence = Path(self.temp.name) / f"run-evidence-{filter_name}-{variant}"

    def test_previous_hir_filter_rejects_ignored_only_execution(self):
        for filter_name in run.TEST_FILTERS[25:30]:
            result = self.run_case(self.successful_executor(ignored=filter_name))
            index = run.TEST_FILTERS.index(filter_name)
            self.assertEqual("failed", result["filters"][index]["status"])
            self.assertIn("ignored", result["filters"][index]["reason"])
            self.evidence = Path(self.temp.name) / f"run-evidence-ignored-{filter_name}"

    def test_previous_hir_filters_reject_malformed_summary_and_partial_execution(self):
        for filter_name in run.TEST_FILTERS[25:30]:
            for variant in ("malformed", "partial"):
                result = self.run_case(self.successful_executor(run_variant=variant, run_variant_filter=filter_name))
                index = run.TEST_FILTERS.index(filter_name)
                self.assertEqual("failed", result["filters"][index]["status"])
                self.assertTrue(any(term in result["filters"][index]["reason"] for term in ("summary", "omitted", "execution count")))
                self.evidence = Path(self.temp.name) / f"run-evidence-{filter_name}-{variant}"

    def test_previous_hir_filters_reject_zero_selected_and_failed_execution(self):
        for filter_name in run.TEST_FILTERS[25:30]:
            index = run.TEST_FILTERS.index(filter_name)
            zero = self.run_case(self.successful_executor(list_zero=filter_name))
            self.assertEqual("failed", zero["filters"][index]["status"])
            self.assertIn("got 0", zero["filters"][index]["reason"])
            self.evidence = Path(self.temp.name) / f"run-evidence-zero-{filter_name}"

            failed = self.run_case(self.successful_executor(fail="failed-run", failed_run_filter=filter_name))
            self.assertEqual("failed", failed["filters"][index]["status"])
            self.assertIn("failed", failed["filters"][index]["reason"])
            self.evidence = Path(self.temp.name) / f"run-evidence-failed-{filter_name}"

    def test_timeout_command_kills_process_group_and_writes_log(self):
        with tempfile.TemporaryDirectory(prefix="thaw-timeout-test-") as temp:
            root = Path(temp)
            marker = root / "child-wrote-after-timeout"
            child_script = "import signal,time; signal.signal(signal.SIGTERM,signal.SIG_IGN); time.sleep(1.0); open(%r,'w').write('bad')" % str(marker)
            parent_script = "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',%r]); time.sleep(10)" % child_script
            log = root / "timeout.log"
            status = run.run_command([sys.executable, "-c", parent_script], root, {}, log, 0.2)
            self.assertTrue(status["timed_out"])
            self.assertEqual("timed-out", status["status"])
            self.assertTrue(log.is_file())
            time.sleep(1.2)
            self.assertFalse(marker.exists(), "child process survived timeout process-group termination")

    def test_real_executor_captures_stdout_for_test_selection_parsing(self):
        with tempfile.TemporaryDirectory(prefix="thaw-log-capture-") as temp:
            root = Path(temp)
            log = root / "command.log"
            raw = run.run_command([sys.executable, "-c", "print('picked::test_case: test')"], root, dict(os.environ), log, 5)
            normalized = run._normalize_result(raw, raw["argv"], root, log, time.monotonic())
            self.assertEqual("passed", normalized["status"])
            self.assertIn("picked::test_case: test", normalized["stdout"])

    def test_initialize_and_finalize_cli_modes_are_safe_before_any_product_command(self):
        with tempfile.TemporaryDirectory(prefix="thaw-cli-state-") as temp:
            evidence = Path(temp) / "evidence"
            helper = Path(run.__file__)
            initialized = subprocess.run(
                [sys.executable, str(helper), "--initialize", "--evidence", str(evidence)],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(0, initialized.returncode, initialized.stderr)
            initial_state = json.loads((evidence / "validation.json").read_text())
            self.assertEqual("running", initial_state["status"])
            self.assertEqual(self.FILTER_COUNT, len(initial_state["filters"]))
            self.assertIn("quickjs_no_run", initial_state["stages"])
            self.assertEqual("not-run", initial_state["stages"]["quickjs_no_run"]["status"])
            self.assertIn("runtime_no_run", initial_state["stages"])
            self.assertEqual("not-run", initial_state["stages"]["runtime_no_run"]["status"])
            finalized = subprocess.run(
                [sys.executable, str(helper), "--finalize-if-incomplete", "--evidence", str(evidence), "--reason", "setup failed"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(0, finalized.returncode, finalized.stderr)
            blocked = json.loads((evidence / "validation.json").read_text())
            self.assertEqual("blocked", blocked["overall"]["status"])
            self.assertEqual(1, blocked["exit_code"])
            self.assertTrue(all(stage["status"] == "blocked" for stage in blocked["stages"].values()))
            self.assertTrue(all(item["status"] == "blocked" for item in blocked["filters"]))
            summary = json.loads((evidence / "summary.json").read_text())
            self.assertEqual(blocked["stages"]["quickjs_no_run"], summary["quickjs_no_run"])
            self.assertEqual(blocked["stages"]["runtime_no_run"], summary["runtime_no_run"])

    def test_finalizer_preserves_terminal_pass_and_exit_code_zero(self):
        state = run.initialize_evidence(self.evidence)
        for stage in state["stages"].values():
            stage.update({"status": "passed", "reason": "completed"})
        for item in state["filters"]:
            item.update({"status": "passed", "reason": "completed"})
            item["list"].update({"status": "passed", "reason": "completed"})
            item["run"].update({"status": "passed", "reason": "completed"})
        state.update({"status": "passed", "overall": {"status": "passed", "reason": "all passed"}, "exit_code": 0, "finished_at": "finished"})
        run._persist(self.evidence, state)
        result = run.finalize_incomplete(self.evidence, "post-run always step")
        self.assertEqual("passed", result["status"])
        self.assertEqual("passed", result["overall"]["status"])
        self.assertEqual(0, result["exit_code"])

    def test_finalizer_preserves_terminal_failure_and_exit_code_one(self):
        state = run.initialize_evidence(self.evidence)
        state["stages"]["identity"].update({"status": "passed", "reason": "identity okay"})
        state["stages"]["baseline_check"].update({"status": "failed", "reason": "baseline failed"})
        for name in ("candidate_check", "llvm_no_run", "hir_no_run"):
            state["stages"][name].update({"status": "blocked", "reason": "blocked by baseline diagnostics"})
        for item in state["filters"]:
            item.update({"status": "blocked", "reason": "blocked by baseline diagnostics"})
            item["list"].update({"status": "blocked", "reason": "blocked by baseline diagnostics"})
            item["run"].update({"status": "blocked", "reason": "blocked by baseline diagnostics"})
        state.update({"status": "failed", "overall": {"status": "failed", "reason": "baseline failed"}, "exit_code": 1, "finished_at": "finished"})
        run._persist(self.evidence, state)
        result = run.finalize_incomplete(self.evidence, "post-run always step")
        self.assertEqual("failed", result["status"])
        self.assertEqual("failed", result["overall"]["status"])
        self.assertEqual(1, result["exit_code"])

    def test_selected_github_run_identity_is_preserved_in_evidence(self):
        metadata = {
            "GITHUB_SHA": "a" * 40,
            "GITHUB_REPOSITORY": "NY1729/thaw",
            "GITHUB_REF": "refs/heads/validation/native-return",
            "GITHUB_RUN_ID": "123456789",
        }
        with mock.patch.dict(os.environ, metadata):
            state = run.initialize_evidence(self.evidence)
            finalized = run.finalize_incomplete(self.evidence, "setup did not finish")
        for key, env_key in (("control_commit", "GITHUB_SHA"), ("repository", "GITHUB_REPOSITORY"), ("ref", "GITHUB_REF"), ("run_id", "GITHUB_RUN_ID")):
            self.assertEqual(metadata[env_key], state[key])
            self.assertEqual(metadata[env_key], finalized[key])
        summary = json.loads((self.evidence / "summary.json").read_text())
        self.assertEqual(metadata["GITHUB_SHA"], summary["control_commit"])
        self.assertEqual(metadata["GITHUB_REF"], summary["ref"])


if __name__ == "__main__":
    unittest.main()
