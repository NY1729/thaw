"""Static, dependency-free contract tests for the branch-local CI workflow."""

from __future__ import annotations

from pathlib import Path
import re
import json
import unittest


STAGING = Path(__file__).resolve().parents[3]
WORKFLOW = STAGING / ".github/workflows/ci.yml"


class WorkflowContractTests(unittest.TestCase):
    def read_workflow(self) -> str:
        self.assertTrue(WORKFLOW.is_file(), "staging/.github/workflows/ci.yml must replace inherited CI")
        return WORKFLOW.read_text(encoding="utf-8")

    def test_workflow_exists_and_is_the_only_branch_validation_route(self):
        text = self.read_workflow()
        pins = json.loads((STAGING / ".ci/native-return-v1/pins.json").read_text(encoding="utf-8"))
        self.assertEqual("validation/native-return", pins["validation_branch"])
        self.assertRegex(
            text,
            r"(?ms)^on:\n  push:\n    branches:\n      - validation/native-return\n    paths:\n      - \.github/workflows/ci\.yml\n      - \.ci/native-return-v1/\*\*\s*$",
        )
        for trigger in ("pull_request", "pull_request_target", "workflow_run", "schedule:", "workflow_dispatch:"):
            self.assertNotIn(trigger, text)

    def test_workflow_permissions_jobs_and_actions_are_minimal_and_pinned(self):
        text = self.read_workflow()
        self.assertIn("permissions:\n  contents: read", text)
        self.assertNotIn("contents: write", text)
        self.assertEqual(1, len(re.findall(r"(?m)^jobs:$", text)))
        uses = re.findall(r"(?m)^\s+uses:\s*(\S+)", text)
        self.assertEqual(
            [
                "actions/checkout@11d5960a326750d5838078e36cf38b85af677262",
                "actions/checkout@11d5960a326750d5838078e36cf38b85af677262",
                "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02",
            ],
            uses,
        )
        self.assertEqual(2, text.count("persist-credentials: false"))
        self.assertIn("8b353a99d4995c8217b9e73cd308ea85d2d6e5d8", text)
        self.assertIn("ref: ${{ github.sha }}", text)
        self.assertIn("timeout-minutes: 90", text)
        self.assertNotIn("matrix:", text)
        self.assertNotIn("services:", text)

    def test_workflow_has_no_secret_inputs_and_always_finalizes_evidence(self):
        text = self.read_workflow()
        self.assertNotIn("secrets.", text)
        self.assertNotIn("GH_TOKEN", text)
        self.assertRegex(text, r"(?m)^\s+if:\s+always\(\)\s*$")
        self.assertIn("if-no-files-found: error", text)
        self.assertRegex(text, r"(?m)^\s+retention-days:\s+14\s*$")
        self.assertIn("--finalize-if-incomplete", text)
        self.assertIn("Finalize any blocked or unfinished stages", text)
        self.assertIn("python3 -m unittest discover", text)

    def test_workflow_setup_stays_within_approved_runner_only_diagnostics(self):
        text = self.read_workflow()
        self.assertIn("LLVM-22.1.8-Linux-X64.tar.xz", text)
        self.assertIn("df0e1ecf16caf3489a272a5eea4eec9b0d82878f6477fa309504f918a0006384", text)
        self.assertIn("libuv1-dev", text)
        self.assertIn("LLVM_SYS_221_PREFIX", text)
        self.assertIn("CARGO_BUILD_JOBS: \"2\"", text)
        self.assertIn("CARGO_INCREMENTAL: \"0\"", text)
        self.assertNotIn("cargo fmt", text)
        self.assertNotIn("cargo clippy", text)
        self.assertNotIn("cargo build --release", text)
        self.assertNotIn("npm", text)
        self.assertNotIn("postgres", text.lower())
        self.assertNotIn("sudo rm -rf", text)


if __name__ == "__main__":
    unittest.main()
