#!/usr/bin/env python3
"""ov-1185: exercise local scheduling and real Cargo failure propagation offline."""
import argparse
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
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
    def test_fixture_hooks_do_not_inherit_the_agents_private_route(self):
        with tempfile.TemporaryDirectory() as temporary, \
                patch.dict(os.environ, {"FARCOOLER_NATIVE_HOOK_SOCKET": "/tmp/real-agent.sock"}), \
                contextlib.redirect_stdout(io.StringIO()):
            root = Path(temporary)
            plans = {"relay": [lane.Step("relay", "isolated", 'test -z "${FARCOOLER_NATIVE_HOOK_SOCKET:-}"\n'),
                               lane.Step("relay", "explicit fixture", 'test "$FARCOOLER_NATIVE_HOOK_SOCKET" = /tmp/fixture.sock\n',
                                         env={"FARCOOLER_NATIVE_HOOK_SOCKET": "/tmp/fixture.sock"})]}
            args = argparse.Namespace(no_load_wait=True, strict_leaks=False, landing=False)
            rows = []
            self.assertFalse(lane.run_one_job("relay", plans, rows, ROOT, root,
                                             lane.make_ctx(ROOT, root, "HEAD"), root, args))
            self.assertEqual([r[2] for r in rows], ["pass", "pass"])

    def test_cpu_budget_reaches_real_steps_without_changing_the_checks(self):
        command = ('test "$CARGO_BUILD_JOBS" = 4 && test "$GOMAXPROCS" = 2 && '
                   'test "$GIT_OPTIONAL_LOCKS" = 0 && '
                   'test "$NEXTEST_TEST_THREADS" = 6 && test "$GOFLAGS" = "-p=2" && '
                   'test "$GRADLE_OPTS" = "-Dorg.gradle.workers.max=2" && '
                   'test "$CARGO_PROFILE_DEV_DEBUG" = 0\n')
        args = argparse.Namespace(workers=4, strict_leaks=False, no_load_wait=True, landing=True)
        with tempfile.TemporaryDirectory() as temporary, patch.object(policy.os, "cpu_count", return_value=10), \
                patch.object(lane.hl, "lean_cargo_env", return_value={"CARGO_PROFILE_DEV_DEBUG": "0"}), \
                patch.dict(os.environ, {"PATH": os.environ["PATH"], "HOME": os.environ["HOME"]}, clear=True), \
                contextlib.redirect_stdout(io.StringIO()):
            root = Path(temporary)
            rows = []
            self.assertFalse(lane.run_one_job("relay", {"relay": [lane.Step("relay", "budget", command)]},
                                             rows, ROOT, root, lane.make_ctx(ROOT, root, "HEAD"), root, args))
            self.assertEqual(rows[0][2], "pass")
        with patch.object(policy.os, "cpu_count", return_value=10):
            env = {"CARGO_BUILD_JOBS": "1", "GOMAXPROCS": "1", "NEXTEST_TEST_THREADS": "1",
                   "GOFLAGS": "-mod=readonly", "GRADLE_OPTS": "-Dkeep=true"}
            policy.budget_worker(env, 4)
            self.assertEqual(env["CARGO_BUILD_JOBS"], "1")
            self.assertEqual(env["GOMAXPROCS"], "1")
            self.assertEqual(env["NEXTEST_TEST_THREADS"], "1")
            self.assertEqual(env["GOFLAGS"], "-mod=readonly -p=2")
            self.assertEqual(env["GRADLE_OPTS"], "-Dkeep=true -Dorg.gradle.workers.max=2")

    def test_git_reads_keep_the_watched_index_stable_and_real_writes_work(self):
        with tempfile.TemporaryDirectory() as temporary:
            repo = Path(temporary)
            env = dict(os.environ)
            env.pop("GIT_OPTIONAL_LOCKS", None)
            policy.budget_worker(env, 4)
            def git(*args, optional=None):
                return subprocess.check_output(
                    ["git", "-c", "commit.gpgsign=false", "-c", "user.name=Test",
                     "-c", "user.email=t@t", *args], cwd=repo,
                    env=env if optional is None else {**env, "GIT_OPTIONAL_LOCKS": optional},
                    stderr=subprocess.DEVNULL)
            git("init", "-q")
            source = repo / "input"
            source.write_text("same")
            git("add", "input")
            git("commit", "-qm", "initial")
            git("status", "--porcelain", optional="1")
            index = repo / ".git/index"
            before = index.stat().st_mtime_ns
            stat = source.stat()
            os.utime(source, ns=(stat.st_atime_ns, stat.st_mtime_ns - 2_000_000_000))
            self.assertEqual(git("status", "--porcelain"), b"")
            self.assertEqual(index.stat().st_mtime_ns, before)
            self.assertEqual(git("status", "--porcelain", optional="1"), b"")
            self.assertNotEqual(index.stat().st_mtime_ns, before)
            source.write_text("changed")
            self.assertIn(b" M input", git("status", "--porcelain"))
            git("add", "input")
            git("commit", "-qm", "real write")
            self.assertEqual(git("show", "HEAD:input"), b"changed")

    def test_plain_runner_can_be_loaded_without_optional_landing_policy(self):
        # Evidence consumers copy the canonical plain runner and heavy lock,
        # not the opt-in landing implementation. Exercise their real import.
        with tempfile.TemporaryDirectory() as temporary:
            repo = Path(temporary)
            (repo / "scripts").mkdir()
            (repo / ".github/workflows").mkdir(parents=True)
            for name in ("lane-gate.py", "heavy_lock.py"):
                shutil.copyfile(ROOT / "scripts" / name, repo / "scripts" / name)
            shutil.copyfile(ROOT / ".github/workflows/ci.yml", repo / ".github/workflows/ci.yml")
            result = subprocess.run([sys.executable, str(repo / "scripts/lane-gate.py"), "--help"],
                                    cwd=repo, env={**os.environ, "PYTHONPATH": ""},
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

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
        wasm = next(s for s in plans["web"] if "GOOS=js GOARCH=wasm go test" in s.run)
        self.assertIsNone(wasm.skip)
        self.assertEqual(wasm.env["FASTLANE_SESSION"], "")
        self.assertEqual(wasm.env["GOMAXPROCS"], "1")
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
        args = argparse.Namespace(workers=2, strict_leaks=False, no_load_wait=True, landing=True)
        with tempfile.TemporaryDirectory() as t, contextlib.redirect_stdout(io.StringIO()):
            t = Path(t)
            rows = []
            self.assertFalse(policy.run(jobs, plans, rows, ROOT, t, lane.make_ctx(ROOT, t, "HEAD"),
                                        t, args, lane.run_one_job))
            self.assertEqual(len([r for r in rows if r[2] == "pass"]), 5)
            for job in jobs:
                timings = [json.loads(line) for line in (t / f"{job}-timings.jsonl").read_text().splitlines()]
                self.assertEqual([r["result"] for r in timings], ["pass", "pass"])
                self.assertTrue(all(r["seconds"] >= 0 for r in timings))

    def test_real_affected_failure_and_unaffected_ci_failure(self):
        self.real_failure_proof(use_nextest=shutil.which("cargo-nextest") is not None)

    def test_real_failure_proof_without_optional_nextest(self):
        self.real_failure_proof(use_nextest=False)

    def real_failure_proof(self, use_nextest):
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
                if not use_nextest:
                    # The wire job has Cargo but does not install nextest.
                    # Still exercise real filtered/workspace failures there;
                    # Rust-equipped local gates also exercise the exact
                    # production nextest command above.
                    step.run = "cargo test --no-fail-fast"
                    step.run += (" --workspace" if full else
                                 "".join(" -p " + p for p in proposal["packages"]))
                    step.run += "\n"
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
