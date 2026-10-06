#!/usr/bin/env python3
"""Run bounded, evidence-preserving baseline/candidate Cargo diagnostics."""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import time
from typing import Any, Callable

import verify


MODULE_DIR = Path(__file__).resolve().parent
PAYLOAD_DIR = MODULE_DIR
WALL_BUDGET_SECONDS = 70 * 60
TIMEOUTS = {
    "check": 15 * 60,
    "no_run": 10 * 60,
    "list": 60,
    "test": 2 * 60,
}
TEST_FILTERS = tuple(item["filter"] for item in verify.EXPECTED["test_filters"])
FILTER_PACKAGES = {item["filter"]: item["package"] for item in verify.EXPECTED["test_filters"]}
STAGE_NAMES = ("identity", "baseline_check", "candidate_check", "quickjs_no_run", "std_no_run", "llvm_no_run", "hir_no_run")
ALLOWED_STATES = {"not-run", "passed", "failed", "timed-out", "blocked"}


def _now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def _sha(path: Path) -> str | None:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError:
        return None


def _atomic_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    data = (json.dumps(value, sort_keys=True, indent=2, ensure_ascii=False) + "\n").encode("utf-8")
    fd, temp_name = tempfile.mkstemp(prefix=path.name + ".", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp_name, path)
    finally:
        try:
            os.unlink(temp_name)
        except FileNotFoundError:
            pass


def _signal_from_return_code(return_code: int | None) -> int | None:
    return -return_code if isinstance(return_code, int) and return_code < 0 else None


def run_command(argv: list[str], cwd: Path, env: dict[str, str], log: Path, timeout_seconds: float) -> dict[str, Any]:
    """Run one command with direct log capture and whole-process-group timeout."""
    log.parent.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    timed_out = False
    proc: subprocess.Popen | None = None
    try:
        with log.open("wb") as stream:
            proc = subprocess.Popen(
                argv,
                cwd=str(cwd),
                env=env,
                stdout=stream,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            try:
                proc.wait(timeout=max(0.01, float(timeout_seconds)))
            except subprocess.TimeoutExpired:
                timed_out = True
                try:
                    os.killpg(proc.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    proc.wait(timeout=1.0)
                except subprocess.TimeoutExpired:
                    pass
                # The leader may have exited while a child ignored SIGTERM. Kill
                # the original process group unconditionally before releasing it.
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                if proc.poll() is None:
                    proc.wait()
    except OSError as exc:
        log.parent.mkdir(parents=True, exist_ok=True)
        with log.open("ab") as stream:
            stream.write(f"command launch failed: {exc}\n".encode("utf-8", errors="replace"))
        return {
            "status": "failed",
            "argv": list(argv),
            "cwd": str(cwd),
            "exit_code": None,
            "signal": None,
            "timed_out": False,
            "duration_seconds": round(time.monotonic() - started, 3),
            "log": str(log),
            "log_sha256": _sha(log),
            "error": f"launch failed: {exc}",
        }
    return_code = proc.returncode if proc is not None else None
    status = "timed-out" if timed_out else "passed" if return_code == 0 else "failed"
    return {
        "status": status,
        "argv": list(argv),
        "cwd": str(cwd),
        "exit_code": return_code,
        "signal": _signal_from_return_code(return_code),
        "timed_out": timed_out,
        "duration_seconds": round(time.monotonic() - started, 3),
        "log": str(log),
        "log_sha256": _sha(log),
    }


def _new_stage(name: str) -> dict[str, Any]:
    return {"name": name, "status": "not-run", "reason": "not started"}


def _initial_state(evidence: Path) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "status": "running",
        "started_at": _now(),
        "finished_at": None,
        "baseline_commit": verify.EXPECTED["baseline"]["commit"],
        "control_commit": os.environ.get("GITHUB_SHA"),
        "repository": os.environ.get("GITHUB_REPOSITORY"),
        "ref": os.environ.get("GITHUB_REF"),
        "run_id": os.environ.get("GITHUB_RUN_ID"),
        "stages": {name: _new_stage(name) for name in STAGE_NAMES},
        "filters": [
            {
                "index": index,
                "package": item["package"],
                "filter": item["filter"],
                "status": "not-run",
                "reason": "not started",
                "list": _new_stage(f"filter_{index:02d}_list"),
                "run": _new_stage(f"filter_{index:02d}_run"),
            }
            for index, item in enumerate(verify.EXPECTED["test_filters"], 1)
        ],
        "evidence_dir": str(evidence),
        "overall": {"status": "not-run", "reason": "validation has not started"},
    }


def _persist(evidence: Path, state: dict[str, Any]) -> None:
    _atomic_json(evidence / "validation.json", state)
    summary = {
        "status": state.get("status"),
        "overall": state.get("overall"),
        "baseline_commit": state.get("baseline_commit"),
        "control_commit": state.get("control_commit"),
        "repository": state.get("repository"),
        "ref": state.get("ref"),
        "run_id": state.get("run_id"),
        "baseline_check": state.get("stages", {}).get("baseline_check"),
        "candidate_check": state.get("stages", {}).get("candidate_check"),
        "quickjs_no_run": state.get("stages", {}).get("quickjs_no_run"),
        "llvm_no_run": state.get("stages", {}).get("llvm_no_run"),
        "hir_no_run": state.get("stages", {}).get("hir_no_run"),
        "std_no_run": state.get("stages", {}).get("std_no_run"),
        "filter_counts": {
            status: sum(item.get("status") == status for item in state.get("filters", []))
            for status in sorted(ALLOWED_STATES)
        },
        "finished_at": state.get("finished_at"),
        "exit_code": state.get("exit_code"),
    }
    _atomic_json(evidence / "summary.json", summary)


def initialize_evidence(evidence: Path) -> dict[str, Any]:
    """Create a durable all-not-run state before dependency setup begins."""
    if evidence.exists() and not evidence.is_dir():
        raise RuntimeError(f"evidence path must be a directory or absent: {evidence}")
    evidence.mkdir(parents=True, exist_ok=True)
    state = _initial_state(evidence)
    _persist(evidence, state)
    return state


def finalize_incomplete(evidence: Path, reason: str) -> dict[str, Any]:
    """Mark remaining work blocked without overwriting completed or failed stages."""
    path = evidence / "validation.json"
    if path.is_file():
        try:
            state = json.loads(path.read_text(encoding="utf-8"))
        except Exception as exc:
            raise RuntimeError(f"cannot read existing validation evidence: {exc}") from exc
    else:
        state = _initial_state(evidence)
    reason = reason.strip() or "an earlier workflow step did not complete"
    reconstruction_path = evidence / "reconstruction.json"
    reconstruction = None
    if reconstruction_path.is_file():
        try:
            reconstruction = json.loads(reconstruction_path.read_text(encoding="utf-8"))
        except Exception:
            reconstruction = None
    identity = state.get("stages", {}).get("identity")
    if identity and identity.get("status") == "not-run" and isinstance(reconstruction, dict):
        if reconstruction.get("status") == "passed":
            identity.update({"status": "passed", "reason": "pre-dependency reconstruction and identity check passed"})
        elif reconstruction.get("status") == "failed":
            cause = reconstruction.get("error") or reason
            identity.update({"status": "failed", "reason": f"pre-dependency identity verification failed: {cause}"})
            reason = identity["reason"]
    for stage in state.get("stages", {}).values():
        if stage.get("status") == "not-run":
            stage.update({"status": "blocked", "reason": reason})
    for item in state.get("filters", []):
        if item.get("status") == "not-run":
            item.update({"status": "blocked", "reason": reason})
        for step in ("list", "run"):
            stage = item.get(step, {})
            if stage.get("status") == "not-run":
                stage.update({"status": "blocked", "reason": reason})
    previous_overall = state.get("overall", {}).get("status")
    identity_failed = state.get("stages", {}).get("identity", {}).get("status") == "failed"
    if identity_failed:
        state["status"] = "failed"
        state["overall"] = {"status": "failed", "reason": state["stages"]["identity"]["reason"]}
        state["exit_code"] = 1
    elif previous_overall not in ("passed", "failed"):
        state["status"] = "blocked"
        state["overall"] = {"status": "blocked", "reason": reason}
        state["exit_code"] = 1
    else:
        state["status"] = previous_overall
        if previous_overall == "failed":
            state["exit_code"] = 1
        else:
            state["exit_code"] = 0
    state["finished_at"] = state.get("finished_at") or _now()
    _persist(evidence, state)
    return state


def _normalize_result(result: dict[str, Any], argv: list[str], cwd: Path, log: Path, started: float) -> dict[str, Any]:
    if not isinstance(result, dict):
        raise RuntimeError("executor did not return a result mapping")
    return_code = result.get("exit_code", result.get("returncode"))
    if return_code is not None and not isinstance(return_code, int):
        raise RuntimeError(f"executor returned invalid exit code {return_code!r}")
    timed_out = bool(result.get("timed_out", False))
    out = result.get("stdout")
    err = result.get("stderr")
    if out is None and log.is_file():
        out = log.read_text(encoding="utf-8", errors="replace")
    if out is None:
        out = ""
    if err is None:
        err = ""
    if not log.exists():
        log.parent.mkdir(parents=True, exist_ok=True)
        with log.open("wb") as stream:
            stream.write(str(out).encode("utf-8", errors="replace"))
            if err:
                stream.write(str(err).encode("utf-8", errors="replace"))
    signal_number = result.get("signal") or _signal_from_return_code(return_code)
    if timed_out:
        status = "timed-out"
    else:
        status = "passed" if return_code == 0 else "failed"
    return {
        **result,
        "status": status,
        "argv": list(argv),
        "cwd": str(cwd),
        "exit_code": return_code,
        "signal": signal_number,
        "timed_out": timed_out,
        "duration_seconds": result.get("duration_seconds", round(time.monotonic() - started, 3)),
        "log": str(log),
        "log_sha256": _sha(log),
        "stdout": str(out),
        "stderr": str(err),
    }


def _budget_timeout(started_wall: float, max_seconds: float, command_timeout: float) -> tuple[float | None, str | None]:
    remaining = max_seconds - (time.monotonic() - started_wall)
    if remaining <= 0:
        return None, "whole-run wall budget expired before command start"
    return min(remaining, command_timeout), None


def _command(
    execute: Callable,
    state: dict[str, Any],
    evidence: Path,
    lane: str,
    cwd: Path,
    argv: list[str],
    target: Path,
    step_id: str,
    command_timeout: float,
    started_wall: float,
) -> dict[str, Any]:
    timeout, budget_reason = _budget_timeout(started_wall, WALL_BUDGET_SECONDS, command_timeout)
    if timeout is None:
        raise TimeoutError(budget_reason)
    env = os.environ.copy()
    env.update(
        {
            "CARGO_TARGET_DIR": str(target),
            "CARGO_BUILD_JOBS": "2",
            "CARGO_INCREMENTAL": "0",
            "CARGO_PROFILE_DEV_DEBUG": "0",
            "CARGO_PROFILE_TEST_DEBUG": "0",
            "RUST_MIN_STACK": "16777216",
        }
    )
    log = evidence / "logs" / f"{step_id}.log"
    before = time.monotonic()
    result = execute(argv, cwd, env, log, timeout)
    return _normalize_result(result, argv, cwd, log, before)


def _set_stage(state: dict[str, Any], name: str, result: dict[str, Any]) -> None:
    stage = state["stages"][name]
    stage.clear()
    stage.update({"name": name, **result})


def _block_stage(state: dict[str, Any], name: str, reason: str) -> None:
    stage = state["stages"][name]
    if stage["status"] == "not-run":
        stage.update({"status": "blocked", "reason": reason})


def _block_filters(state: dict[str, Any], start_index: int, reason: str) -> None:
    for item in state["filters"][start_index:]:
        if item["status"] == "not-run":
            item.update({"status": "blocked", "reason": reason})
        for step in ("list", "run"):
            if item[step]["status"] == "not-run":
                item[step].update({"status": "blocked", "reason": reason})


def _block_old_filters(state: dict[str, Any], start_index: int, reason: str) -> None:
    """Keep predecessor-lane fail-fast dispositions inside its original 30 groups."""
    for item in state["filters"][start_index:30]:
        if item["status"] == "not-run":
            item.update({"status": "blocked", "reason": reason})
        for step in ("list", "run"):
            if item[step]["status"] == "not-run":
                item[step].update({"status": "blocked", "reason": reason})


def _block_filter(item: dict[str, Any], reason: str) -> None:
    if item["status"] == "not-run":
        item.update({"status": "blocked", "reason": reason})
    for step in ("list", "run"):
        if item[step]["status"] == "not-run":
            item[step].update({"status": "blocked", "reason": reason})


def _run_stage(
    state: dict[str, Any],
    evidence: Path,
    execute: Callable,
    lane: str,
    cwd: Path,
    argv: list[str],
    target: Path,
    stage_name: str,
    timeout: float,
    started_wall: float,
) -> dict[str, Any]:
    try:
        result = _command(execute, state, evidence, lane, cwd, argv, target, stage_name, timeout, started_wall)
    except TimeoutError as exc:
        result = {"status": "blocked", "reason": str(exc), "argv": argv, "cwd": str(cwd), "log": None, "log_sha256": None}
    except Exception as exc:
        log = evidence / "logs" / f"{stage_name}.log"
        log.parent.mkdir(parents=True, exist_ok=True)
        with log.open("ab") as stream:
            stream.write(f"executor exception: {type(exc).__name__}: {exc}\n".encode("utf-8", errors="replace"))
        result = {
            "status": "failed",
            "reason": f"executor exception: {type(exc).__name__}: {exc}",
            "argv": argv,
            "cwd": str(cwd),
            "log": str(log),
            "log_sha256": _sha(log),
            "exit_code": None,
            "signal": None,
            "timed_out": False,
        }
    _set_stage(state, stage_name, result)
    return result


def _filter_list(output: str) -> list[str]:
    names = []
    for line in output.splitlines():
        match = re.match(r"^(.+): test\s*$", line)
        if match:
            names.append(match.group(1))
    return names


def _filter_run_counts(output: str, selected: list[str]) -> tuple[dict[str, int] | None, list[str], str | None]:
    summary = None
    pattern = re.compile(r"test result:\s+(?:ok|FAILED)\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored;")
    for line in output.splitlines():
        match = pattern.search(line)
        if match:
            summary = {"passed": int(match.group(1)), "failed": int(match.group(2)), "ignored": int(match.group(3))}
    if summary is None:
        return None, [], "actual cargo test output had no parseable Rust test summary"
    outcome_by_name: dict[str, str] = {}
    outcome_names: list[str] = []
    for line in output.splitlines():
        match = re.match(r"^test\s+(.+?)\s+\.\.\.\s+(ok|FAILED|ignored)\s*$", line)
        if match:
            outcome_names.append(match.group(1))
            outcome_by_name[match.group(1)] = match.group(2)
    missing = [name for name in selected if name not in outcome_by_name]
    if missing:
        return summary, outcome_names, f"actual cargo output omitted a selected test outcome: {missing[0]}"
    if len(outcome_names) != len(selected) or len(outcome_by_name) != len(selected):
        return summary, outcome_names, f"actual execution count {len(outcome_names)} differs from listed selection {len(selected)}"
    observed = {
        "passed": sum(outcome == "ok" for outcome in outcome_by_name.values()),
        "failed": sum(outcome == "FAILED" for outcome in outcome_by_name.values()),
        "ignored": sum(outcome == "ignored" for outcome in outcome_by_name.values()),
    }
    if observed != summary:
        return summary, outcome_names, f"per-test outcomes {observed} do not match Rust summary {summary}"
    if summary["passed"] <= 0 or summary["failed"] != 0 or summary["ignored"] != 0:
        return summary, outcome_names, f"control execution requires passed>0, failed=0, ignored=0; observed {summary}"
    return summary, outcome_names, None


def _run_filter(
    state: dict[str, Any],
    item: dict[str, Any],
    index: int,
    execute: Callable,
    evidence: Path,
    target: Path,
    candidate: Path,
    started_wall: float,
) -> bool:
    package = item["package"]
    filter_name = item["filter"]
    prefix = f"control_{index:02d}"
    base = ["cargo", "test", "--locked", "-p", package, "--lib", filter_name]
    list_argv = base + ["--", "--list"]
    try:
        listed = _command(execute, {}, evidence, "candidate", candidate, list_argv, target, prefix + "_list", TIMEOUTS["list"], started_wall)
    except TimeoutError as exc:
        item["list"].update({"status": "blocked", "reason": str(exc)})
        item["run"].update({"status": "blocked", "reason": "control execution requires a completed test listing"})
        item.update({"status": "blocked", "reason": str(exc)})
        _persist(evidence, state)
        return False
    except Exception as exc:
        log = evidence / "logs" / f"{prefix}_list.log"
        log.parent.mkdir(parents=True, exist_ok=True)
        with log.open("ab") as stream:
            stream.write(f"executor exception: {type(exc).__name__}: {exc}\n".encode("utf-8", errors="replace"))
        item["list"].update({"status": "failed", "reason": f"executor exception: {type(exc).__name__}: {exc}", "log": str(log), "log_sha256": _sha(log)})
        item["run"].update({"status": "blocked", "reason": "control execution requires a completed test listing"})
        item.update({"status": "failed", "reason": item["list"]["reason"]})
        _persist(evidence, state)
        return False
    item["list"].update(listed)
    _persist(evidence, state)
    if listed["status"] != "passed":
        reason = f"control list command {listed['status']}: exit={listed.get('exit_code')} signal={listed.get('signal')}"
        item.update({"status": "timed-out" if listed["status"] == "timed-out" else "failed", "reason": reason})
        item["run"].update({"status": "blocked", "reason": "control execution requires a successful nonempty test listing"})
        _persist(evidence, state)
        return False
    selected = _filter_list(listed.get("stdout", ""))
    item["selected_names"] = selected
    item["selected_count"] = len(selected)
    expected_full = verify.EXPECTED.get("expected_full_test_names", {}).get(filter_name)
    if expected_full is not None:
        exact = (
            isinstance(expected_full, list)
            and len(selected) == len(expected_full)
            and len(set(selected)) == len(selected)
            and len(set(expected_full)) == len(expected_full)
            and set(selected) == set(expected_full)
        )
        if not exact:
            item.update({"status": "failed", "reason": f"cargo --list full names/count differ from exact expected identifiers: expected {len(expected_full)} unique names, got {len(selected)}"})
            item["run"].update({"status": "blocked", "reason": "exact expected test listing was not established"})
            _persist(evidence, state)
            return False
    else:
        expected = verify.EXPECTED.get("expected_test_names", {}).get(filter_name)
        if expected is not None:
            observed_leaves = [name.rsplit("::", 1)[-1] for name in selected]
            exact = (
                len(selected) == len(expected)
                and len(set(expected)) == len(expected)
                and all("::" in name and name.split("::", 1)[0] for name in selected)
                and sorted(observed_leaves) == sorted(expected)
            )
            if not exact:
                item.update({"status": "failed", "reason": f"cargo --list names/count differ from exact expected identifiers: expected {len(expected)} unique names, got {len(selected)}"})
                item["run"].update({"status": "blocked", "reason": "exact expected test listing was not established"})
                _persist(evidence, state)
                return False
    if not selected:
        item.update({"status": "failed", "reason": "cargo --list selected zero tests"})
        item["run"].update({"status": "blocked", "reason": "cargo --list selected zero tests"})
        _persist(evidence, state)
        return False
    run_argv = base + ["--", "--test-threads=1"]
    try:
        executed = _command(execute, {}, evidence, "candidate", candidate, run_argv, target, prefix + "_run", TIMEOUTS["test"], started_wall)
    except TimeoutError as exc:
        item["run"].update({"status": "blocked", "reason": str(exc)})
        item.update({"status": "blocked", "reason": str(exc)})
        _persist(evidence, state)
        return False
    except Exception as exc:
        log = evidence / "logs" / f"{prefix}_run.log"
        log.parent.mkdir(parents=True, exist_ok=True)
        with log.open("ab") as stream:
            stream.write(f"executor exception: {type(exc).__name__}: {exc}\n".encode("utf-8", errors="replace"))
        item["run"].update({"status": "failed", "reason": f"executor exception: {type(exc).__name__}: {exc}", "log": str(log), "log_sha256": _sha(log)})
        item.update({"status": "failed", "reason": item["run"]["reason"]})
        _persist(evidence, state)
        return False
    item["run"].update(executed)
    _persist(evidence, state)
    counts, execution_names, output_error = _filter_run_counts(executed.get("stdout", ""), selected)
    item["execution"] = counts
    item["execution_names"] = execution_names
    if executed["status"] != "passed":
        reason = f"control command {executed['status']}: exit={executed.get('exit_code')} signal={executed.get('signal')}"
    elif output_error:
        reason = output_error
    else:
        reason = None
    status = "passed" if reason is None else "timed-out" if executed["status"] == "timed-out" else "failed"
    item.update({"status": status, "reason": reason or "all selected tests passed"})
    _persist(evidence, state)
    return reason is None


def _overall(state: dict[str, Any]) -> tuple[str, int, str]:
    required = [
        state["stages"][name]["status"]
        for name in ("baseline_check", "candidate_check", "quickjs_no_run", "llvm_no_run", "hir_no_run", "std_no_run")
    ]
    filters = [item["status"] for item in state["filters"]]
    if all(value == "passed" for value in required + filters):
        return "passed", 0, "baseline and candidate diagnostics plus all focused controls passed"
    if any(value == "failed" or value == "timed-out" for value in required + filters):
        return "failed", 1, "one or more required compilation or focused control stages failed"
    return "blocked", 1, "one or more required candidate stages did not complete"


def _run_validation_impl(
    baseline: Path,
    candidate: Path,
    evidence: Path,
    execute: Callable,
) -> dict[str, Any]:
    """Revalidate exact inputs, then run independent compile and gated focused stages."""
    verify._validate_independent_paths(baseline, candidate, PAYLOAD_DIR, evidence)
    started_wall = time.monotonic()
    state = initialize_evidence(evidence)
    try:
        identity = verify.verify_prepared(baseline, candidate, PAYLOAD_DIR)
        state["stages"]["identity"].update({"status": "passed", "reason": "prepared baseline/candidate/payload identities verified", "result": identity})
        _persist(evidence, state)
    except Exception as exc:
        reason = f"identity verification failed: {type(exc).__name__}: {exc}"
        state["stages"]["identity"].update({"status": "failed", "reason": reason})
        for stage in ("baseline_check", "candidate_check", "quickjs_no_run", "std_no_run", "llvm_no_run", "hir_no_run"):
            _block_stage(state, stage, reason)
        _block_filters(state, 0, reason)
        state["overall"] = {"status": "failed", "reason": reason}
        state["status"] = "failed"
        state["finished_at"] = _now()
        state["exit_code"] = 1
        _persist(evidence, state)
        return state

    target_root = evidence.parent / "cargo-targets"
    baseline_target = target_root / "baseline"
    candidate_target = target_root / "candidate"
    baseline_cmd = ["cargo", "check", "--locked", "-p", "thaw-llvm", "--lib"]
    candidate_cmd = ["cargo", "check", "--locked", "-p", "thaw-llvm", "--lib"]

    quickjs_result = _run_stage(
        state,
        evidence,
        execute,
        "candidate",
        candidate,
        ["cargo", "test", "--locked", "-p", "thaw-quickjs", "--lib", "--no-run"],
        candidate_target,
        "quickjs_no_run",
        TIMEOUTS["no_run"],
        started_wall,
    )
    _persist(evidence, state)
    std_result = _run_stage(
        state,
        evidence,
        execute,
        "candidate",
        candidate,
        ["cargo", "test", "--locked", "-p", "thaw-std", "--lib", "--no-run"],
        candidate_target,
        "std_no_run",
        TIMEOUTS["no_run"],
        started_wall,
    )
    _persist(evidence, state)

    # New QuickJS/std groups are independent: a failed focused command cannot
    # suppress another eligible group, and each package uses only its own gate.
    for index in range(30, len(state["filters"])):
        item = state["filters"][index]
        package = item["package"]
        preflight = quickjs_result if package == "thaw-quickjs" else std_result if package == "thaw-std" else None
        if preflight is None:
            _block_filter(item, f"no independent preflight is defined for package {package}")
        elif preflight["status"] != "passed":
            _block_filter(item, f"{package} test no-run {preflight['status']}: {preflight.get('reason') or preflight.get('exit_code')}")
        else:
            _run_filter(state, item, index + 1, execute, evidence, candidate_target, candidate, started_wall)
        _persist(evidence, state)

    baseline_result = _run_stage(state, evidence, execute, "baseline", baseline, baseline_cmd, baseline_target, "baseline_check", TIMEOUTS["check"], started_wall)
    _persist(evidence, state)
    candidate_result = _run_stage(state, evidence, execute, "candidate", candidate, candidate_cmd, candidate_target, "candidate_check", TIMEOUTS["check"], started_wall)
    _persist(evidence, state)

    # The predecessor lane retains its candidate LLVM-check, LLVM no-run, HIR
    # no-run and std-gate fail-fast behavior across exactly its original 30 groups.
    if candidate_result["status"] != "passed":
        reason = f"candidate check {candidate_result['status']}: {candidate_result.get('reason') or candidate_result.get('exit_code')}"
        _block_stage(state, "llvm_no_run", reason)
        _block_stage(state, "hir_no_run", reason)
        _block_old_filters(state, 0, reason)
    else:
        llvm_result = _run_stage(
            state,
            evidence,
            execute,
            "candidate",
            candidate,
            ["cargo", "test", "--locked", "-p", "thaw-llvm", "--lib", "--no-run"],
            candidate_target,
            "llvm_no_run",
            TIMEOUTS["no_run"],
            started_wall,
        )
        _persist(evidence, state)
        if llvm_result["status"] != "passed":
            reason = f"LLVM no-run {llvm_result['status']}: {llvm_result.get('reason') or llvm_result.get('exit_code')}"
            _block_stage(state, "hir_no_run", reason)
            _block_old_filters(state, 0, reason)
        else:
            hir_result = _run_stage(
                state,
                evidence,
                execute,
                "candidate",
                candidate,
                ["cargo", "test", "--locked", "-p", "thaw-hir", "--lib", "--no-run"],
                candidate_target,
                "hir_no_run",
                TIMEOUTS["no_run"],
                started_wall,
            )
            _persist(evidence, state)
            if hir_result["status"] != "passed":
                reason = f"HIR no-run {hir_result['status']}: {hir_result.get('reason') or hir_result.get('exit_code')}"
                _block_old_filters(state, 0, reason)
            elif std_result["status"] != "passed":
                reason = f"std no-run {std_result['status']}: {std_result.get('reason') or std_result.get('exit_code')}"
                _block_old_filters(state, 0, reason)
            else:
                for index, item in enumerate(state["filters"][:30]):
                    passed = _run_filter(state, item, index + 1, execute, evidence, candidate_target, candidate, started_wall)
                    _persist(evidence, state)
                    if not passed:
                        reason = item.get("reason", "focused control did not pass")
                        _block_old_filters(state, index + 1, f"prior focused control blocked sequence: {reason}")
                        _persist(evidence, state)
                        break

    # Recheck both source trees after all Cargo commands before recording a final verdict.
    try:
        verify.verify_prepared(baseline, candidate, PAYLOAD_DIR)
        state["post_run_identity"] = {"status": "passed", "reason": "baseline and candidate remain identical to their pinned identities"}
    except Exception as exc:
        state["post_run_identity"] = {"status": "failed", "reason": f"post-run identity check failed: {type(exc).__name__}: {exc}"}
    overall, exit_code, reason = _overall(state)
    if state.get("post_run_identity", {}).get("status") != "passed":
        overall, exit_code, reason = "failed", 1, state["post_run_identity"]["reason"]
    state["overall"] = {"status": overall, "reason": reason}
    state["status"] = overall
    state["exit_code"] = exit_code
    state["finished_at"] = _now()
    _persist(evidence, state)
    return state


def run_validation(
    baseline: Path,
    candidate: Path,
    evidence: Path,
    execute: Callable,
) -> dict[str, Any]:
    """Run diagnostics and guarantee an honest terminal summary when possible."""
    try:
        return _run_validation_impl(baseline, candidate, evidence, execute)
    except Exception as exc:
        validation_path = evidence / "validation.json"
        if not validation_path.is_file():
            raise
        try:
            state = json.loads(validation_path.read_text(encoding="utf-8"))
        except Exception as read_exc:
            raise RuntimeError(f"cannot recover validation evidence after {exc}: {read_exc}") from exc
        cause = f"validation runner exception: {type(exc).__name__}: {exc}"
        for stage in state.get("stages", {}).values():
            if stage.get("status") == "not-run":
                stage.update({"status": "blocked", "reason": cause})
        for item in state.get("filters", []):
            if item.get("status") == "not-run":
                item.update({"status": "blocked", "reason": cause})
            for name in ("list", "run"):
                if item.get(name, {}).get("status") == "not-run":
                    item[name].update({"status": "blocked", "reason": cause})
        state["status"] = "failed"
        state["overall"] = {"status": "failed", "reason": cause}
        state["error"] = cause
        state["exit_code"] = 1
        state["finished_at"] = _now()
        _persist(evidence, state)
        return state
    finally:
        validation_path = evidence / "validation.json"
        if validation_path.is_file():
            try:
                state = json.loads(validation_path.read_text(encoding="utf-8"))
                if state.get("finished_at") is None:
                    reason = state.get("error", "validation did not reach a terminal state")
                    for stage in state.get("stages", {}).values():
                        if stage.get("status") == "not-run":
                            stage.update({"status": "blocked", "reason": reason})
                    for item in state.get("filters", []):
                        if item.get("status") == "not-run":
                            item.update({"status": "blocked", "reason": reason})
                        for name in ("list", "run"):
                            if item.get(name, {}).get("status") == "not-run":
                                item[name].update({"status": "blocked", "reason": reason})
                    state["status"] = "failed"
                    state["overall"] = {"status": "failed", "reason": reason}
                    state["exit_code"] = 1
                    state["finished_at"] = _now()
                    _persist(evidence, state)
            except Exception:
                # If the evidence volume itself is failing, the exception from the
                # body/writer remains visible to the caller; no false pass is made.
                pass


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--evidence", type=Path, required=True)
    parser.add_argument("--initialize", action="store_true", help="write all-not-run evidence before setup")
    parser.add_argument("--finalize-if-incomplete", action="store_true", help="block any work left not-run after an earlier failure")
    parser.add_argument("--reason", default="an earlier workflow setup or diagnostic step did not complete")
    args = parser.parse_args(argv)
    try:
        if args.initialize:
            initialize_evidence(args.evidence)
            return 0
        if args.finalize_if_incomplete:
            finalize_incomplete(args.evidence, args.reason)
            return 0
        if not args.baseline or not args.candidate:
            parser.error("--baseline and --candidate are required unless --initialize or --finalize-if-incomplete is selected")
        result = run_validation(args.baseline, args.candidate, args.evidence, run_command)
        print(json.dumps(result, sort_keys=True, indent=2))
        return int(result.get("exit_code", 1))
    except Exception as exc:
        print(f"native candidate diagnostics failed: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
