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
import unittest
from unittest import mock

MODULE_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(MODULE_DIR))
try:
    run = importlib.import_module("run")
    verify = importlib.import_module("verify")
    from test_verify import Fixture, sha, write
except ModuleNotFoundError:
    run = None
    verify = None
    Fixture = None


class DiagnosticRunnerTests(unittest.TestCase):
    def require_implementation(self):
        if run is None or verify is None or Fixture is None:
            self.fail("run.py must expose the diagnostic gate runner and verification APIs")
        return run

    def setUp(self):
        self.require_implementation()
        self.temp = tempfile.TemporaryDirectory(prefix="thaw-run-test-")
        self.fixture = Fixture(Path(self.temp.name))
        self.fixture.activate()
        verify.reconstruct(self.fixture.baseline, self.fixture.candidate, self.fixture.payload, self.fixture.evidence)
        self.old_payload_dir = run.PAYLOAD_DIR
        run.PAYLOAD_DIR = self.fixture.payload
        self.evidence = Path(self.temp.name) / "run-evidence"

    def tearDown(self):
        if verify is not None and hasattr(self, "fixture") and hasattr(self.fixture, "expected_before"):
            verify.EXPECTED = self.fixture.expected_before
        if run is not None and hasattr(self, "old_payload_dir"):
            run.PAYLOAD_DIR = self.old_payload_dir
        self.temp.cleanup()

    def successful_executor(self, *, fail=None, list_zero=None, ignored=None, raise_on=None, list_variant=None, list_variant_filter=None, run_variant=None, run_variant_filter=None, failed_run_filter=None):
        calls = []

        def execute(argv, cwd, env, log, timeout_seconds):
            argv = list(argv)
            call = {"argv": argv, "cwd": Path(cwd), "env": dict(env), "log": Path(log), "timeout": timeout_seconds}
            calls.append(call)
            lane = "baseline" if Path(cwd) == self.fixture.baseline else "candidate"
            if raise_on == f"{lane}-check" and argv[1:2] == ["check"]:
                raise RuntimeError("injected executor failure")
            if fail == f"{lane}-check" and argv[1:2] == ["check"]:
                return {"exit_code": 101, "stdout": "error: injected check failure\n", "timed_out": False}
            if fail == "llvm-no-run" and "--no-run" in argv and "thaw-llvm" in argv:
                return {"exit_code": 101, "stdout": "error: injected LLVM no-run failure\n", "timed_out": False}
            if fail == "hir-no-run" and "--no-run" in argv and "thaw-hir" in argv:
                return {"exit_code": 101, "stdout": "error: injected HIR no-run failure\n", "timed_out": False}
            if fail == "std-no-run" and "--no-run" in argv and "thaw-std" in argv:
                return {"exit_code": 101, "stdout": "error: injected std no-run failure\n", "timed_out": False}
            if fail == "failed-list" and "--list" in argv:
                return {"exit_code": 1, "stdout": "list failed\n", "timed_out": False}
            if fail == "failed-run" and "--list" not in argv and "--no-run" not in argv and argv[1:2] == ["test"]:
                filter_name = next((x for x in argv if x in run.TEST_FILTERS), "control")
                if failed_run_filter is None or failed_run_filter == filter_name:
                    expected = verify.EXPECTED.get("expected_test_names", {}).get(filter_name)
                    if expected:
                        package = verify.EXPECTED["test_filters"][run.TEST_FILTERS.index(filter_name)]["package"].replace("-", "_")
                        names = [f"{package}::tests::{leaf}" for leaf in expected]
                    else:
                        names = [filter_name + "::selected"]
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
                filter_name = next((x for x in argv if x in run.TEST_FILTERS), "control")
                if list_zero == filter_name:
                    return {"exit_code": 0, "stdout": "0 tests, 0 benchmarks\n", "timed_out": False}
                expected = verify.EXPECTED.get("expected_test_names", {}).get(filter_name)
                if expected:
                    package = verify.EXPECTED["test_filters"][run.TEST_FILTERS.index(filter_name)]["package"].replace("-", "_")
                    leaves = list(expected)
                    variant = list_variant if list_variant_filter is None or list_variant_filter == filter_name else None
                    if variant == "substitute":
                        leaves[0] = "thaw_remaining_substituted_name"
                    if variant == "short":
                        leaves = leaves[:-1]
                    names = [f"{package}::tests::{leaf}" for leaf in leaves]
                    if variant == "duplicate":
                        names.append(names[0])
                    return {"exit_code": 0, "stdout": "".join(f"{name}: test\n" for name in names) + f"{len(names)} tests, 0 benchmarks\n", "timed_out": False}
                name = filter_name + "::selected"
                return {"exit_code": 0, "stdout": f"{name}: test\n1 test, 0 benchmarks\n", "timed_out": False}
            filter_name = next((x for x in argv if x in run.TEST_FILTERS), "control")
            expected = verify.EXPECTED.get("expected_test_names", {}).get(filter_name)
            if expected:
                package = verify.EXPECTED["test_filters"][run.TEST_FILTERS.index(filter_name)]["package"].replace("-", "_")
                names = [f"{package}::tests::{leaf}" for leaf in expected]
            else:
                names = [filter_name + "::selected"]
            if ignored == filter_name:
                return {
                    "exit_code": 0,
                    "stdout": "".join(f"test {name} ... ignored\n" for name in names) + f"test result: ok. 0 passed; 0 failed; {len(names)} ignored; 0 measured; 0 filtered out; finished in 0.00s\n",
                    "timed_out": False,
                }
            variant = run_variant if run_variant_filter is None or run_variant_filter == filter_name else None
            if expected and variant == "malformed":
                return {"exit_code": 0, "stdout": "running tests\n" + "".join(f"test {name} ... ok\n" for name in names), "timed_out": False}
            if expected and variant == "partial":
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
        self.assertEqual(27, len(result["filters"]))
        self.assertTrue(all(item["status"] == "passed" for item in result["filters"]))
        self.assertEqual(1, result["exit_code"])

    def test_candidate_check_failure_blocks_both_no_runs_and_controls(self):
        result = self.run_case(self.successful_executor(fail="candidate-check"))
        self.assertEqual("passed", result["stages"]["baseline_check"]["status"])
        self.assertEqual("failed", result["stages"]["candidate_check"]["status"])
        for name in ("llvm_no_run", "hir_no_run", "std_no_run"):
            self.assertEqual("blocked", result["stages"][name]["status"])
        self.assertTrue(all(item["status"] == "blocked" for item in result["filters"]))

    def test_llvm_no_run_failure_blocks_hir_and_all_controls(self):
        result = self.run_case(self.successful_executor(fail="llvm-no-run"))
        self.assertEqual("failed", result["stages"]["llvm_no_run"]["status"])
        self.assertEqual("blocked", result["stages"]["hir_no_run"]["status"])
        self.assertTrue(all(item["status"] == "blocked" for item in result["filters"]))

    def test_hir_no_run_failure_blocks_all_controls(self):
        result = self.run_case(self.successful_executor(fail="hir-no-run"))
        self.assertEqual("failed", result["stages"]["hir_no_run"]["status"])
        self.assertEqual("blocked", result["stages"]["std_no_run"]["status"])
        self.assertTrue(all(item["status"] == "blocked" for item in result["filters"]))

    def test_std_no_run_failure_blocks_all_controls(self):
        result = self.run_case(self.successful_executor(fail="std-no-run"))
        self.assertEqual("passed", result["stages"]["hir_no_run"]["status"])
        self.assertEqual("failed", result["stages"]["std_no_run"]["status"])
        self.assertTrue(all(item["status"] == "blocked" for item in result["filters"]))

    def test_zero_list_selection_fails_and_blocks_remaining_filters(self):
        first = run.TEST_FILTERS[0]
        result = self.run_case(self.successful_executor(list_zero=first))
        self.assertEqual("failed", result["filters"][0]["status"])
        self.assertIn("selected zero", result["filters"][0]["reason"])
        self.assertTrue(all(item["status"] == "blocked" for item in result["filters"][1:]))

    def test_ignored_only_actual_execution_is_not_a_pass(self):
        first = run.TEST_FILTERS[0]
        result = self.run_case(self.successful_executor(ignored=first))
        self.assertEqual("failed", result["filters"][0]["status"])
        self.assertIn("ignored", result["filters"][0]["reason"])
        self.assertTrue(all(item["status"] == "blocked" for item in result["filters"][1:]))

    def test_failed_list_and_failed_control_stop_later_filters(self):
        list_result = self.run_case(self.successful_executor(fail="failed-list"))
        self.assertEqual("failed", list_result["filters"][0]["status"])
        self.assertEqual("blocked", list_result["filters"][1]["status"])
        run_result = self.run_case(self.successful_executor(fail="failed-run"))
        self.assertEqual("failed", run_result["filters"][0]["status"])
        self.assertIn("failed", run_result["filters"][0]["reason"])
        self.assertTrue(all(item["status"] == "blocked" for item in run_result["filters"][1:]))

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
        self.assertTrue((self.evidence / "validation.json").is_file())

    def test_executor_exception_keeps_log_and_still_runs_independent_candidate_check(self):
        executor = self.successful_executor(raise_on="baseline-check")
        result = self.run_case(executor)
        self.assertEqual("failed", result["stages"]["baseline_check"]["status"])
        self.assertEqual("passed", result["stages"]["candidate_check"]["status"])
        self.assertTrue(Path(result["stages"]["baseline_check"]["log"]).is_file())
        self.assertTrue((self.evidence / "validation.json").is_file())

    def test_identity_failure_runs_zero_cargo_commands_and_marks_all_stages(self):
        self.fixture.candidate.joinpath("unexpected.txt").write_text("tamper\n")
        executor = self.successful_executor()
        result = self.run_case(executor)
        self.assertEqual([], executor.calls)
        self.assertEqual("failed", result["stages"]["identity"]["status"])
        self.assertTrue(all(stage["status"] == "blocked" for stage in result["stages"].values() if stage["name"] not in ("identity",)))

    def test_success_uses_27_pinned_filters_locked_and_separate_target_dirs(self):
        executor = self.successful_executor()
        result = self.run_case(executor)
        self.assertEqual(27, len(result["filters"]))
        self.assertEqual(20, sum(f["package"] == "thaw-llvm" for f in result["filters"]))
        self.assertEqual(6, sum(f["package"] == "thaw-hir" for f in result["filters"]))
        self.assertEqual(1, sum(f["package"] == "thaw-std" for f in result["filters"]))
        cargo_calls = executor.calls
        self.assertTrue(cargo_calls)
        self.assertTrue(all("--locked" in call["argv"] for call in cargo_calls))
        target_dirs = {call["env"].get("CARGO_TARGET_DIR") for call in cargo_calls}
        self.assertEqual(2, len(target_dirs))
        self.assertNotEqual(*sorted(target_dirs))
        run_calls = [call["argv"] for call in cargo_calls if call["argv"][1:2] == ["test"] and "--no-run" not in call["argv"] and "--list" not in call["argv"]]
        self.assertEqual(27, len(run_calls))
        self.assertTrue(all("--test-threads=1" in argv for argv in run_calls))
        no_runs = [call["argv"] for call in cargo_calls if "--no-run" in call["argv"]]
        self.assertEqual(["thaw-llvm", "thaw-hir", "thaw-std"], [argv[argv.index("-p") + 1] for argv in no_runs])

    def test_new_filters_preserve_exact_full_names_counts_and_execution_evidence(self):
        result = self.run_case(self.successful_executor())
        hir_filters = result["filters"][-2:]
        self.assertEqual(["thaw_binding_helper_", "receiver_pattern_inference_"], [item["filter"] for item in hir_filters])
        self.assertEqual([2, 10], [len(verify.EXPECTED["expected_test_names"][item["filter"]]) for item in hir_filters])
        for item in hir_filters:
            expected = verify.EXPECTED["expected_test_names"][item["filter"]]
            self.assertEqual(expected, [name.rsplit("::", 1)[-1] for name in item["selected_names"]])
            self.assertEqual(item["selected_names"], item["execution_names"])

    def test_new_filters_reject_substituted_or_duplicate_listed_names(self):
        for filter_name in run.TEST_FILTERS[-2:]:
            for variant in ("substitute", "duplicate", "short"):
                result = self.run_case(self.successful_executor(list_variant=variant, list_variant_filter=filter_name))
                index = run.TEST_FILTERS.index(filter_name)
                self.assertEqual("failed", result["filters"][index]["status"])
                self.assertIn("expected", result["filters"][index]["reason"])
                if index + 1 < len(result["filters"]):
                    self.assertEqual("blocked", result["filters"][index + 1]["status"])
                self.evidence = Path(self.temp.name) / f"run-evidence-{filter_name}-{variant}"

    def test_new_filter_rejects_ignored_only_execution(self):
        for filter_name in run.TEST_FILTERS[-2:]:
            result = self.run_case(self.successful_executor(ignored=filter_name))
            index = run.TEST_FILTERS.index(filter_name)
            self.assertEqual("failed", result["filters"][index]["status"])
            self.assertIn("ignored", result["filters"][index]["reason"])
            self.evidence = Path(self.temp.name) / f"run-evidence-ignored-{filter_name}"

    def test_new_filter_rejects_malformed_summary_and_partial_execution(self):
        for filter_name in run.TEST_FILTERS[-2:]:
            for variant in ("malformed", "partial"):
                result = self.run_case(self.successful_executor(run_variant=variant, run_variant_filter=filter_name))
                index = run.TEST_FILTERS.index(filter_name)
                self.assertEqual("failed", result["filters"][index]["status"])
                self.assertTrue(any(term in result["filters"][index]["reason"] for term in ("summary", "omitted", "execution count")))
                self.evidence = Path(self.temp.name) / f"run-evidence-{filter_name}-{variant}"

    def test_each_new_filter_rejects_zero_selected_and_failed_execution(self):
        for filter_name in run.TEST_FILTERS[-2:]:
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
            self.assertEqual(27, len(initial_state["filters"]))
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
