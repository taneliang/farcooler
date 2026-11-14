#!/usr/bin/env python3
"""Verify real wrapper timing and reject incomplete/failed performance evidence."""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import sys

spec = importlib.util.spec_from_file_location("benchmark", Path(__file__).with_name("landing-benchmark.py"))
benchmark = importlib.util.module_from_spec(spec)
spec.loader.exec_module(benchmark)


class BenchmarkTests(unittest.TestCase):
    def test_step_timings_survive_without_a_final_summary(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            script = ('import sys; from pathlib import Path; import landing_policy; '
                      'step=type("Step",(),{"label":lambda s:"nextest", "run":"cargo nextest"})(); '
                      'p=Path(sys.argv[1]); p.mkdir(); '
                      'landing_policy.record_timing(p,"rust",step,"pass",1,3.25)')
            command = [sys.executable, "-c", script, str(root / "trial/steps")]
            result = benchmark.measure(command, Path(__file__).parent, root / "trial", 5)
            self.assertEqual(result["exit_code"], 0)
            self.assertEqual(result["steps"][0]["seconds"], 2.25)
            self.assertFalse(result["complete_green"])

    def test_wrapper_wait_counts_and_stale_summary_does_not(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "lane-gate-summary.txt").write_text("lane-gate: all steps passed\nlanding-gate: PASS\n")
            command = ["bash", "-c", "sleep 0.1; echo '| rust | nextest | pass | 0.02 | 1 passed |'; "
                       "echo 'lane-gate: all steps passed'; echo 'landing-gate: PASS'"]
            result = benchmark.measure(command, root, root / "trial", 5)
            self.assertGreaterEqual(result["wall_seconds"], 0.1)
            self.assertTrue(result["complete_green"])
            self.assertEqual(result["steps"][0]["seconds"], 0.02)
            self.assertTrue(result["load_samples"])
            empty = benchmark.measure(["bash", "-c", "true"], root, root / "empty", 5)
            self.assertFalse(empty["complete_green"])

    def test_nonzero_partial_and_deadline_never_count_as_fast_green(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for i, script in enumerate(("echo 'lane-gate: all steps passed'; echo 'landing-gate: PASS'; exit 1",
                                       "echo PARTIAL; echo 'lane-gate: all steps passed'; echo 'landing-gate: PASS'",
                                       "sleep 10")):
                result = benchmark.measure(["bash", "-c", script], root, root / str(i), 0.1)
                self.assertFalse(result["complete_green"])
            self.assertTrue(result["timed_out"])

    def test_acceptance_requires_three_stable_green_wrappers_under_limit(self):
        good = {"wall_seconds": 899, "complete_green": True}
        self.assertTrue(benchmark.summarize([good] * 3, True)["acceptance_1"])
        self.assertFalse(benchmark.summarize([good] * 2, True)["acceptance_1"])
        self.assertFalse(benchmark.summarize([good] * 3, False)["acceptance_1"])
        bad = {"wall_seconds": 1, "complete_green": False}
        self.assertFalse(benchmark.summarize([good, good, bad], True)["acceptance_1"])
        slow = {"wall_seconds": 900, "complete_green": True}
        self.assertFalse(benchmark.summarize([good, slow, slow], True)["acceptance_1"])

    def test_per_step_before_after_keeps_missing_steps_and_skip_reasons(self):
        before = benchmark.steps('| rust | nextest a\\|b | pass | 665 | 1 passed |\n')
        after = benchmark.steps('| rust | nextest a\\|b | pass | 700 | 1 passed |\n'
                                '| ios | new CI command | pass | 50 | |\n'
                                '| swift | bundle | skipped: CI | | |\n')
        report = benchmark.report([{"steps": after}] * 3, before)
        self.assertIn("665.0 | 700.0 (pass)", report)
        self.assertIn("not in capture | 50.0 (pass)", report)
        self.assertIn("skipped: CI", report)
        self.assertIn(r"nextest a\|b", report)

    def test_source_identity_includes_dirty_and_untracked_inputs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            def git(*args):
                subprocess.run(["git", "-c", "commit.gpgsign=false", "-c", "user.name=t",
                                "-c", "user.email=t@t", *args], cwd=root, check=True, capture_output=True)
            git("init", "-q")
            (root / "code").write_text("before")
            git("add", "code")
            git("commit", "-qm", "initial")
            initial = benchmark.identity(root)
            index = root / ".git/index"
            before = index.stat().st_mtime_ns
            stat = (root / "code").stat()
            os.utime(root / "code", ns=(stat.st_atime_ns, stat.st_mtime_ns - 2_000_000_000))
            self.assertEqual(initial, benchmark.identity(root))
            self.assertEqual(index.stat().st_mtime_ns, before)
            (root / "code").write_text("after")
            dirty = benchmark.identity(root)
            self.assertEqual(initial["head"], dirty["head"])
            self.assertNotEqual(initial, dirty)
            (root / "new").write_text("untracked")
            self.assertNotEqual(dirty, benchmark.identity(root))

    def test_report_preserves_repeated_labels_and_matches_split_lints(self):
        before = benchmark.steps('| swift | app bundle | pass | 1 | |\n'
                                 '| swift | app bundle | pass | 2 | |\n'
                                 '| rust | cargo clippy --workspace | pass | 3 | |\n')
        after = benchmark.steps('| swift | app bundle | skipped: CI | | |\n'
                                '| swift | app bundle | skipped: CI | | |\n'
                                '| rust-lints | cargo clippy --workspace | pass | 4 | |\n')
        report = benchmark.report([{"steps": after}] * 3, before)
        self.assertIn('app bundle | 1.0 | skipped: CI', report)
        self.assertIn('app bundle | 2.0 | skipped: CI', report)
        self.assertIn('cargo clippy --workspace | 3.0 | 4.0 (pass)', report)


if __name__ == "__main__":
    unittest.main()
