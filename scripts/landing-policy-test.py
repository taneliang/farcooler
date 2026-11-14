#!/usr/bin/env python3
"""ov-1185: exercise local scheduling and real Cargo failure propagation offline."""
import argparse
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import patch

import landing_policy as policy

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("lane_gate", ROOT / "scripts/lane-gate.py")
lane = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lane)


class LandingTests(unittest.TestCase):
    def test_ci_unchanged_and_local_bundle_consumers_explicit(self):
        with tempfile.TemporaryDirectory() as tmp:
            ctx = lane.make_ctx(ROOT, Path(tmp), "HEAD")
            wf = {"ci": (ROOT / ".github/workflows/ci.yml").read_text(),
                  "ios-ui-build": (ROOT / ".github/actions/ios-ui-build/action.yml").read_text()}
            plans = {j: lane.steps_for(j, wf, ctx) for j in lane.JOBS}
        original = next(s.run for s in plans["rust"] if "python3 scripts/rust-shadow.py --out " in s.run)
        self.assertIn("--workspace --no-fail-fast", original)
        self.assertIn("apps/macos/test.sh", wf["ci"])
        policy.apply(plans, {"full": False, "packages": ["farcooler-fence"]})
        nextest = next(s.run for s in plans["rust"] if "--workspace --no-fail-fast" in s.run)
        self.assertIn("test-leak-check.py -- ./scripts/tmux-leak-check.py -- cargo nextest", nextest)
        self.assertIn("-E 'package(=farcooler-fence)'", nextest)
        self.assertNotIn("rust-shadow.py", nextest)
        for s in plans["swift"]:
            if "apps/macos/build/$APP_NAME.app" in s.run or "build-app.sh" in s.run:
                self.assertIsNotNone(s.skip)
        entitlements = next(s for s in plans["swift"] if "macos-entitlements-test.sh" in s.run)
        self.assertIsNone(entitlements.skip)
        self.assertEqual(entitlements.run.strip(), "./scripts/macos-entitlements-test.sh")
        # Never filter feature-specific tests, clippy, doctests, or Apple tests.
        self.assertTrue(any("cargo clippy --workspace" in s.run and not s.skip for s in plans["rust"]))
        self.assertTrue(any("cargo test --workspace --doc" in s.run and not s.skip for s in plans["rust"]))
        self.assertTrue(any("apps/macos/test.sh" in s.run and not s.skip for s in plans["swift-tests"]))

    def test_parallelism_is_bounded_and_apple_is_serial(self):
        self.assertEqual(policy.families(["rust", "swift-tests", "swift", "ios", "android"]),
                         [["rust"], ["swift-tests", "swift", "ios"], ["android"]])
        barrier = threading.Barrier(2, timeout=5)
        active = set()
        lock = threading.Lock()
        def job(j, plans, rows, repo, tmp, *rest):
            self.assertEqual(tmp.name, j)
            with lock:
                active.add(j)
            barrier.wait()
            rows.append((j, "test", "FAIL" if j == "rust" else "pass", "0", ""))
            return j == "rust"
        args = argparse.Namespace(workers=2)
        with tempfile.TemporaryDirectory() as t:
            rows = []
            failed = policy.run(["rust", "android"], {"rust": [], "android": []}, rows, ROOT, Path(t), {}, Path(t), args, job)
        self.assertTrue(failed)
        self.assertEqual(active, {"rust", "android"})
        self.assertEqual([r[0] for r in rows], ["rust", "android", "landing"])
        self.assertEqual([r[1] for r in rows[:2]], ["test", "test"])

    def test_worker_exception_is_red_and_other_rows_survive(self):
        def job(j, plans, rows, *rest):
            if j == "relay":
                raise OSError("planted runner failure")
            rows.append((j, "test", "pass", "0", ""))
            return False
        with tempfile.TemporaryDirectory() as t:
            rows = []
            self.assertTrue(policy.run(["relay", "site"], {}, rows, ROOT, Path(t), {}, Path(t),
                                       argparse.Namespace(workers=2), job))
        self.assertIn("planted runner failure", rows[0][2])
        self.assertEqual(rows[1][2], "pass")

    def test_clippy_is_independent_and_failure_is_required(self):
        lint = lane.Step("rust", "clippy", "cargo clippy --workspace --all-targets -- -D warnings")
        suite = lane.Step("rust", "tests", "cargo nextest run --workspace")
        seen = []
        def job(j, plans, rows, repo, tmp, *rest):
            seen.append((j, [s.name for s in plans[j]]))
            rows.append((j, "lint/test", "FAIL" if j == "rust-lints" else "pass", "0", ""))
            return j == "rust-lints"
        with tempfile.TemporaryDirectory() as t:
            rows = []
            self.assertTrue(policy.run(["rust"], {"rust": [lint, suite]}, rows, ROOT, Path(t), {},
                                       Path(t), argparse.Namespace(workers=2), job))
        self.assertEqual(sorted(seen), [("rust", ["tests"]), ("rust-lints", ["clippy"])])
        self.assertTrue(any(r[2] == "FAIL" for r in rows))

    def test_environment_files_are_private_and_steps_keep_exports(self):
        jobs = ["relay", "site"]
        plans = {j: [lane.Step(j, "export", f'echo FAMILY={j} >> "$GITHUB_ENV"\n'),
                     lane.Step(j, "consume", f'test "$FAMILY" = {j}\n')] for j in jobs}
        args = argparse.Namespace(workers=2, strict_leaks=False, no_load_wait=True)
        with tempfile.TemporaryDirectory() as t, contextlib.redirect_stdout(io.StringIO()):
            t = Path(t)
            rows = []
            self.assertFalse(policy.run(jobs, plans, rows, ROOT, t, lane.make_ctx(ROOT, t, "HEAD"),
                                        t, args, lane.run_one_job))
            self.assertEqual(len([r for r in rows if r[2] == "pass"]), 5)

    def test_real_affected_failure_and_unaffected_ci_failure(self):
        with tempfile.TemporaryDirectory(prefix="landing-selection-") as t:
            repo = Path(t).resolve()
            (repo / "Cargo.toml").write_text('[workspace]\nresolver="2"\nmembers=["leaf","consumer","unrelated"]\n')
            for name in ("leaf", "consumer", "unrelated"):
                (repo / name / "src").mkdir(parents=True)
                dep = '\n[dependencies]\nleaf={path="../leaf"}\n' if name == "consumer" else ""
                (repo / name / "Cargo.toml").write_text(f'[package]\nname="{name}"\nversion="0.1.0"\nedition="2021"\n{dep}')
                (repo / name / "src/lib.rs").write_text('#[test] fn contract() {}\n')
            def git(*args):
                return subprocess.run(["git", "-c", "commit.gpgsign=false", "-c", "user.name=Test",
                                       "-c", "user.email=t@t", *args], cwd=repo, capture_output=True, text=True, check=True).stdout.strip()
            subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=repo, check=True, capture_output=True)
            git("init", "-q")
            git("add", "Cargo.toml", "Cargo.lock", "leaf", "consumer", "unrelated")
            git("commit", "-qm", "green base")
            green = git("rev-parse", "HEAD")
            (repo / "leaf/src/lib.rs").write_text('// changed\n#[test] fn contract() {}\n')
            git("add", "leaf/src/lib.rs")
            git("commit", "-qm", "leaf change")
            candidates = repo / ".git/green.ndjson"
            candidates.write_text(json.dumps({"head_sha": green}) + "\n")
            # Use the real selector while loading its code from this checkout.
            def loader(name, path):
                return lane.load_module(name, ROOT / "scripts" / path.name)
            with patch.dict(os.environ, {"CANDIDATES": str(candidates)}):
                proposal = policy.rust_selection(repo, "HEAD", loader)
                self.assertFalse(proposal["full"])
                self.assertEqual(proposal["packages"], ["consumer", "leaf"])
                # Unknown untracked inputs must not disappear from the plan.
                (repo / "unknown").write_text("input")
                self.assertTrue(policy.rust_selection(repo, "HEAD", loader)["full"])
                (repo / "unknown").unlink()
                # Missing green evidence is full, never an arbitrary --base.
                candidates.write_text("")
                self.assertTrue(policy.rust_selection(repo, "HEAD", loader)["full"])
            (repo / ".config").mkdir()
            (repo / ".config/nextest.toml").write_text('[profile.ci]\nfail-fast=false\n')
            def gate(full=False):
                step = lane.Step("rust", "Full Rust tests and shadow measurement",
                    'python3 scripts/rust-shadow.py --out unused -- cargo nextest run --offline --profile ci --workspace --no-fail-fast\n')
                # Match CI's shape; offline is configured in env instead.
                step.run = step.run.replace("run --offline", "run")
                plans = {"rust": [step]}
                policy.apply(plans, {**proposal, "full": full})
                tmp = repo / ".git/run"
                tmp.mkdir(exist_ok=True)
                env = dict(os.environ, CARGO_NET_OFFLINE="true")
                result = lane.run_step_safely(step, repo, env, tmp, tmp)
                return result[3], result[5]
            (repo / "consumer/src/lib.rs").write_text('#[test] fn planted_affected() { panic!("affected"); }\n')
            ok, output = gate()
            self.assertFalse(ok, output)
            self.assertIn("planted_affected", output)
            (repo / "consumer/src/lib.rs").write_text('#[test] fn contract() {}\n')
            (repo / "unrelated/src/lib.rs").write_text('#[test] fn planted_unaffected() { panic!("unaffected"); }\n')
            self.assertTrue(gate()[0])
            ok, output = gate(full=True)
            self.assertFalse(ok, output)
            self.assertIn("planted_unaffected", output)
            print("plant proof: affected selected gate FAIL; unaffected selected PASS; CI full workspace FAIL")


if __name__ == "__main__":
    unittest.main()
