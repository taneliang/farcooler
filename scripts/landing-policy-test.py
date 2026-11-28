#!/usr/bin/env python3
"""ov-1185: exercise local scheduling and real Cargo failure propagation offline."""
import argparse
import contextlib
import hashlib
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
    def script_reach(self, paths, extra_commands=None):
        changes = lane.load_module("script_ci_changes", ROOT / "scripts/ci-changes.py")
        workflows = {"ci": (ROOT / ".github/workflows/ci.yml").read_text(),
                     "ios-ui-build": (ROOT / ".github/actions/ios-ui-build/action.yml").read_text()}
        ctx = lane.make_ctx(ROOT, Path("/tmp/ov-1324-plan"), "HEAD")
        commands = {j: "\n".join(s.run for s in lane.steps_for(j, workflows, ctx) if not s.skip)
                    for j in lane.JOBS}
        commands.update(extra_commands or {})
        return set(lane.jobs_for(paths, changes.SITE, lambda _: True, commands, changes.families))

    def assert_script_map_complete(self, repo):
        # Include untracked additions and dotfiles, but not ignored build output.
        paths = subprocess.check_output(
            ["git", "-C", str(repo), "ls-files", "--cached", "--others",
             "--exclude-standard", "-z", "--", "scripts/", ".github/workflows/"], text=True)
        missing = set(paths.rstrip("\0").split("\0")) - policy.SCRIPT_USERS.keys()
        self.assertFalse(missing, f"unreviewed scripts (runtime defaults to full): {sorted(missing)}")

    def test_every_script_and_workflow_has_reviewed_users_or_explicit_full(self):
        self.assert_script_map_complete(ROOT)
        for path, users in policy.SCRIPT_USERS.items():
            with self.subTest(path=path):
                self.assertTrue(users is None or (isinstance(users, set) and users and users <= set(lane.ALL_JOBS)))

    def test_new_script_or_workflow_fails_coverage_and_defaults_full(self):
        for path in ("scripts/new-unknown.py", ".github/workflows/new-unknown.yml"):
            with self.subTest(path=path), tempfile.TemporaryDirectory() as tmp:
                repo = Path(tmp)
                subprocess.run(["git", "init", "-q", str(repo)], check=True)
                target = repo / path
                target.parent.mkdir(parents=True)
                target.write_text("# new input\n")
                with self.assertRaisesRegex(AssertionError, "unreviewed scripts"):
                    self.assert_script_map_complete(repo)
                self.assertEqual(self.script_reach([path]), set(lane.JOBS))

    def test_reviewed_script_selections(self):
        always = {"wire", "doc-comments"}
        expected = {
            "scripts/copy-lint.py": always,
            "scripts/proto-lint.py": always,
            "scripts/fresh-script-lint.py": always,
            "scripts/shared-core-baseline.json": always,
            "scripts/ci-macos-capacity.py": always,
            "scripts/ci-flake-tally.py": always,
            "scripts/fixtures/ci-flake-tally/manifest.tsv": always,
            "scripts/landing-timing.py": always,
            "scripts/doc-comment-check.py": always,
            "scripts/web-client-tree-check.py": always | {"web"},
            "scripts/swift-build-cache.py": always | {"swift", "swift-tests"},
            "scripts/generator-test.sh": always,
            "scripts/quarantine-test.py": always | {"rust"},
            "scripts/lane-gate.py": set(lane.JOBS),
            "scripts/landing_policy.py": set(lane.JOBS),
        }
        for path, jobs in expected.items():
            with self.subTest(path=path):
                self.assertEqual(self.script_reach([path]), jobs)
        # doc-comments is an unconditional floor; the lint itself feeds only wire.
        self.assertEqual(policy.script_users("scripts/copy-lint.py", {}), {"wire"})

    def test_retry_diff_runs_only_script_checks(self):
        paths = [".github/workflows/ci-runner-retry.yml", ".github/workflows/ci-runner-retry-recover.yml",
                 "scripts/ci-runner-retry.py", "scripts/ci-runner-retry-test.py"]
        for diff in ([p] for p in paths):
            with self.subTest(diff=diff):
                self.assertEqual(self.script_reach(diff), {"wire", "doc-comments"})
        chosen = self.script_reach(paths)
        self.assertEqual(chosen, {"wire", "doc-comments"})
        with patch.object(policy, "rust_selection", side_effect=AssertionError("Rust must not run")):
            proposal = lane.rust_proposal(ROOT, argparse.Namespace(full_rust=False), chosen, policy)
        self.assertEqual(proposal["reasons"], ["Rust not reached"])

    def test_reviewed_scripts_keep_new_direct_consumers(self):
        self.assertIn("rust", self.script_reach(["scripts/ci-runner-retry.py"],
                                               {"rust": "python3 scripts/ci-runner-retry.py"}))

    def test_workflow_changes_keep_lightweight_validation(self):
        plans = {"wire": []}
        policy.workflow_checks(plans, ["scripts/ci-runner-retry.py"], lane.Step)
        self.assertEqual(plans["wire"], [])
        policy.workflow_checks(plans, [".github/workflows/ci-runner-retry.yml"], lane.Step)
        self.assertEqual([s.run.strip() for s in plans["wire"]],
                         ["./scripts/actionlint.sh", "./scripts/ios-ui-shards.py --check"])

    def test_quick_checks_precede_fixture_compilation(self):
        workflows = {"ci": (ROOT / ".github/workflows/ci.yml").read_text()}
        ctx = lane.make_ctx(ROOT, ROOT, "HEAD")
        plans = {"wire": lane.steps_for("wire", workflows, ctx)}
        fixtures = [step for step in plans["wire"] if step.run.strip() in {
            "python3 scripts/rust-select-test.py", "python3 scripts/landing-gate-test.py",
            "python3 scripts/gate-test-fixture-test.py"}]
        self.assertEqual(len(fixtures), 3)
        for review in (False, True):
            with self.subTest(review=review):
                quick, remaining = policy.split_quick(plans, ["wire"], "HEAD", lane.Step,
                                                      typeshare_ready=True, review=review)
                self.assertEqual(quick["wire"][0].run.strip(), "./scripts/shared-core-lint.py")
                for fixture in fixtures:
                    self.assertNotIn(fixture, quick["wire"])
                    self.assertIn(fixture, remaining["wire"])

    def test_gate_script_consumers_and_unknowns_stay_covered(self):
        for path, required in (("scripts/lane-gate.py", set(lane.JOBS)),
                               ("apps/macos/test.sh", {"swift-tests"}),
                               ("scripts/macos-rust-tests.py", {"rust"}),
                               ("scripts/new-unknown.py", set(lane.JOBS)),
                               (".github/workflows/new-unknown.yml", set(lane.JOBS))):
            with self.subTest(path=path):
                self.assertTrue(self.script_reach([path]) >= required)
        self.assertEqual(self.script_reach(["scripts/ci-runner-retry.py", "scripts/new-unknown.py"]),
                         set(lane.JOBS))

    def test_engine_entry_point_keeps_optimized_policy_and_normal_admission(self):
        # Execute the published engine command against the real wrapper. Stand-ins
        # record the boundary calls; no actual machine lock or display is touched.
        with tempfile.TemporaryDirectory() as temporary:
            repo = Path(temporary)
            scripts = repo / "scripts"
            scripts.mkdir()
            shutil.copy(ROOT / "scripts/landing-gate", scripts / "landing-gate")
            events = repo / "events"
            (scripts / "lane-gate.py").write_text(
                'import json, os, sys\n'
                'with open(os.environ["EVENTS"], "a") as f: '
                'f.write(json.dumps(sys.argv[1:]) + "\\n")\n'
                'sys.exit(int(os.environ["GATE_EXIT"]))\n')
            for code in (0, 7):
                events.write_text("")
                done = subprocess.run(["/bin/sh", "-c", "scripts/landing-gate --base main --log-dir logs"],
                                      cwd=repo, env={**os.environ, "EVENTS": str(events), "GATE_EXIT": str(code)},
                                      capture_output=True, text=True)
                self.assertEqual(done.returncode, code, done.stdout + done.stderr)
                calls = events.read_text().splitlines()
                args = ["--fail-fast", "--landing", "--keep-target", "--wait-for-unlock",
                        "--base", "main", "--log-dir", "logs"]
                self.assertEqual([json.loads(call) for call in calls], [args])
                self.assertIn("landing-gate: PASS" if code == 0 else "landing-gate: FAIL", done.stdout)

    def test_mac_build_admission_waits_for_screen_and_takes_the_heavy_lock(self):
        import gate_disk
        events = []

        @contextlib.contextmanager
        def lock(*_):
            events.append("lock")
            yield
            events.append("release")

        def unlock(command, **_):
            self.assertEqual(command, [str(ROOT / "scripts/wait-for-unlock")])
            events.append("unlock")
            return subprocess.CompletedProcess(command, 0)

        args = argparse.Namespace(landing=True, wait_for_unlock=True, no_load_wait=True)
        with patch.dict(os.environ, {}, clear=True), \
                patch.object(lane.process_guard, "run", side_effect=unlock), \
                patch.object(lane, "machine_lock", side_effect=lock), \
                patch.object(gate_disk, "admit", side_effect=lambda *_: events.append("disk")), \
                patch.object(policy, "run", side_effect=lambda *_: events.append("build") or False):
            self.assertFalse(lane.run_jobs(["swift-tests"], {}, [], ROOT, ROOT, {}, ROOT, args))
        self.assertEqual(events, ["unlock", "lock", "disk", "build", "release"])

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
            # Exercise the actual planning entry point's first Git read,
            # before a worker exists. Plain mode keeps its original behavior.
            def first_git(*args):
                self.assertEqual(subprocess.check_output(
                    ["git", "status", "--porcelain"], cwd=repo), b"")
                self.assertEqual(subprocess.check_output(
                    ["git", "diff", "--binary", "HEAD"], cwd=repo), b"")
                raise RuntimeError("stop after first planning read")
            for landing in (True, False):
                stat = source.stat()
                os.utime(source, ns=(stat.st_atime_ns, stat.st_mtime_ns - 2_000_000_000))
                before = index.stat().st_mtime_ns
                with patch.dict(os.environ), patch.object(lane, "make_ctx", side_effect=first_git), \
                        patch.object(lane.tempfile, "mkdtemp", return_value=str(repo / ".git/run")):
                    os.environ.pop("GIT_OPTIONAL_LOCKS", None)
                    with self.assertRaisesRegex(RuntimeError, "first planning read"):
                        lane.gate(argparse.Namespace(worktree=str(ROOT), base="HEAD", landing=landing, quick_only=False))
                self.assertEqual(index.stat().st_mtime_ns == before, landing)

    def test_plain_runner_can_be_loaded_without_optional_landing_policy(self):
        # Evidence consumers copy the canonical plain runner and heavy lock,
        # not the opt-in landing implementation. Exercise their real import.
        with tempfile.TemporaryDirectory() as temporary:
            repo = Path(temporary)
            (repo / "scripts").mkdir()
            (repo / ".github/workflows").mkdir(parents=True)
            for name in ("lane-gate.py", "heavy_lock.py", "process_guard.py"):
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
        original = next(s.run for s in plans["rust"] if s.name == "Full Rust workspace (local)")
        self.assertIn("--no-fail-fast --archive-file target/rust-tests.tar.zst", original)
        self.assertNotIn("--partition", original)
        self.assertIn("apps/macos/test.sh", wf["ci"])
        policy.apply(plans, {"full": False, "packages": ["farcooler-fence"]})
        nextest = next(s.run for s in plans["rust"] if "--no-fail-fast --archive-file" in s.run)
        self.assertIn("test-leak-check.py -- ./scripts/tmux-leak-check.py -- python3 scripts/gate-flaky.py run -- python3 scripts/quarantine.py run -- cargo nextest", nextest)
        self.assertIn("-E 'package(=farcooler-fence)'", nextest)
        self.assertNotIn("rust-select.py", nextest)
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

    def test_real_workspace_leaf_narrows_and_shared_inputs_select_everything(self):
        """ov-1268: the lane gate skips Rust tests a change cannot affect, using this repo's real crate graph."""
        shadow = lane.load_module("rust_select_real", ROOT / "scripts/rust-select.py")
        metadata = json.loads(subprocess.run(
            ["cargo", "metadata", "--offline", "--locked", "--format-version", "1", "--all-features", "--no-deps"],
            cwd=ROOT, capture_output=True, text=True, check=True).stdout)
        everything = sorted(p["name"] for p in metadata["packages"] if p["id"] in set(metadata["workspace_members"]))
        leaf, _, full = shadow.select(ROOT, metadata, [("M", "crates/vt/src/lib.rs")])
        self.assertFalse(full)
        self.assertIn("farcooler-vt", leaf)
        self.assertLess(len(leaf), len(everything))
        for path in ("crates/protocol/src/lib.rs",          # the shared wire crate
                     "Cargo.lock", "Cargo.toml",
                     "crates/cli/build.rs", "crates/protocol/build.rs",  # build scripts
                     "rust-toolchain.toml", "rust-toolchain", ".cargo/config.toml"):
            with self.subTest(path=path):
                names, _, full = shadow.select(ROOT, metadata, [("M", path)])
                self.assertTrue(full)
                self.assertEqual(names, everything)
        # A narrow proposal reaches the plain lane gate's nextest step, and CI's workflow is untouched.
        with tempfile.TemporaryDirectory() as tmp:
            ctx = lane.make_ctx(ROOT, Path(tmp), "HEAD")
            wf = {"ci": (ROOT / ".github/workflows/ci.yml").read_text(),
                  "ios-ui-build": (ROOT / ".github/actions/ios-ui-build/action.yml").read_text()}
            steps = lane.steps_for("rust", wf, ctx)
        step = next(s for s in steps if s.name == "Full Rust workspace (local)")
        policy.apply_rust("rust", step, {"full": False, "packages": leaf})
        self.assertIn("-E ", step.run)
        self.assertIn("package(=farcooler-vt)", step.run)
        self.assertNotIn("package(=", wf["ci"])  # CI on main keeps the full suite
        # ov-1293: a docs-only change selects no packages, so the nextest step is skipped, never narrowed to nothing.
        docs_step = next(s for s in lane.steps_for("rust", wf, ctx) if s.name == "Full Rust workspace (local)")
        policy.apply_rust("rust", docs_step, {"full": False, "packages": []})
        self.assertIn("no Rust test input changed", docs_step.skip)


    def test_full_rust_forces_the_whole_suite_in_both_modes(self):
        """ov-1278: selection is the default in both modes; --full-rust beats it in both, and the evidence says so."""
        # A private clone with its own last commit touching Rust: the real HEAD may be any ticket's commit.
        tmp = tempfile.TemporaryDirectory(prefix="full-rust-fixture-")
        self.addCleanup(tmp.cleanup)
        repo = Path(tmp.name) / "repo"
        git_env = ["git", "-c", "commit.gpgsign=false", "-c", "user.name=Test", "-c", "user.email=t@t"]
        subprocess.run(["git", "clone", "-q", "--no-hardlinks", str(ROOT), str(repo)], check=True, capture_output=True)
        rs = subprocess.run(["git", "ls-files", "*.rs"], cwd=repo, capture_output=True, text=True, check=True).stdout.split()[0]
        with open(repo / rs, "a") as f:
            f.write("\n// fixture change\n")
        subprocess.run([*git_env, "commit", "-qam", "fixture: touch Rust"], cwd=repo, check=True, capture_output=True)
        def plan(*flags):
            out = subprocess.run([sys.executable, str(repo / "scripts/lane-gate.py"), "--dry-run", "--only", "rust",
                                  "--base", "HEAD~1", *flags], cwd=repo, capture_output=True, text=True)
            return out.stdout + out.stderr
        for flags in (("--landing", "--full-rust"), ("--full-rust",)):
            with self.subTest(flags=flags):
                out = plan(*flags)
                self.assertIn("Rust coverage: full suite (--full-rust)", out)
                self.assertNotIn("nextest runs only", out)
        self.assertNotIn("--select-rust", (ROOT / "scripts/lane-gate.py").read_text().split("def main")[1])

    def test_evidence_names_selected_and_skipped_binaries_and_reads_back(self):
        """ov-1278: each gate run saves the test binaries it selected and skipped; a red main is traced by reading them."""
        shadow = lane.load_module("rust_select_evidence", ROOT / "scripts/rust-select.py")
        metadata = json.loads(subprocess.run(
            ["cargo", "metadata", "--offline", "--locked", "--format-version", "1", "--all-features", "--no-deps"],
            cwd=ROOT, capture_output=True, text=True, check=True).stdout)
        names, reasons, full = shadow.select(ROOT, metadata, [("M", "crates/vt/src/lib.rs")])
        self.assertFalse(full)
        narrow = policy.with_binaries(shadow, metadata, {"full": False, "packages": names, "reasons": reasons})
        everything = policy.forced_full(ROOT, lane.load_module, "--full-rust")
        self.assertTrue(everything["full"])
        self.assertEqual(everything["skipped_binaries"], [])
        self.assertIn("farcooler-vt", narrow["selected_binaries"])
        self.assertTrue(narrow["skipped_binaries"])
        self.assertEqual(sorted(narrow["selected_binaries"] + narrow["skipped_binaries"]), everything["selected_binaries"])
        self.assertTrue(all(b.split("::")[0] in names for b in narrow["selected_binaries"]))
        self.assertFalse(any(b.split("::")[0] in names for b in narrow["skipped_binaries"]))
        with tempfile.TemporaryDirectory() as tmp:
            lane.write_rust_evidence(tmp, narrow)
            saved = json.loads((Path(tmp) / "rust-selection.json").read_text())
        self.assertEqual(saved["selected_binaries"], narrow["selected_binaries"])
        self.assertEqual(saved["skipped_binaries"], narrow["skipped_binaries"])

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

    def test_landing_fail_fast_stops_queued_families_and_apple_jobs(self):
        jobs = ["swift-tests", "swift", "ios", "wire"]
        with tempfile.TemporaryDirectory() as t, contextlib.redirect_stdout(io.StringIO()), \
                patch("gate_disk.admit"):  # scheduler fixtures do not consume a gate disk budget
            t = Path(t)
            marker = t / "later"
            plans = {j: [lane.Step(j, "later", f'echo {j} >> "{marker}"\n')] for j in jobs}
            plans["swift-tests"].insert(0, lane.Step("swift-tests", "failure", "exit 1\n"))
            args = argparse.Namespace(workers=1, strict_leaks=False, no_load_wait=True,
                                      no_lock=True, landing=True, fail_fast=True)
            rows = []
            self.assertTrue(lane.run_jobs(jobs, plans, rows, ROOT, t, lane.make_ctx(ROOT, t, "HEAD"), t, args))
            self.assertFalse(marker.exists())
            self.assertEqual([r[0] for r in rows if r[2].startswith("FAIL")], ["swift-tests", "landing"])

            args.fail_fast = False
            rows = []
            second = t / "second"
            second.mkdir()
            self.assertTrue(lane.run_jobs(jobs, plans, rows, ROOT, second, lane.make_ctx(ROOT, second, "HEAD"), t, args))
            self.assertEqual(marker.read_text().splitlines(), jobs)

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

    def test_three_file_fold_plan_uses_group_base_and_only_reached_rust_binaries(self):
        """ov-1305: execute the planner with a stale remote ref and a pinned group base."""
        with tempfile.TemporaryDirectory(prefix="three-file-fold-") as temporary:
            repo = Path(temporary).resolve()
            for folder in ("scripts", ".github"):
                shutil.copytree(ROOT / folder, repo / folder)
            (repo / "Cargo.toml").write_text('[workspace]\nresolver="2"\nmembers=["crates/*"]\n')
            for folder, name, dependencies in (("android", "leaf", ""), ("consumer", "consumer", '\n[dependencies]\nleaf={path="../android"}\n'),
                                                ("unrelated", "unrelated", "")):
                root = repo / "crates" / folder
                (root / "src").mkdir(parents=True)
                (root / "Cargo.toml").write_text(f'[package]\nname="{name}"\nversion="0.1.0"\nedition="2021"\n{dependencies}')
                (root / "src/lib.rs").write_text('#[test] fn contract() {}\n')
            def git(*args):
                return subprocess.check_output(["git", "-c", "user.name=Fixture", "-c", "user.email=t@t",
                                                "-c", "commit.gpgsign=false", *args], cwd=repo, text=True).strip()
            subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=repo, check=True, capture_output=True)
            git("init", "-q")
            git("add", "scripts", ".github", "Cargo.toml", "Cargo.lock", "crates")
            git("commit", "-qm", "runner startup")
            git("update-ref", "refs/remotes/origin/main", "HEAD")
            (repo / "README.md").write_text("main advanced before folding\n")
            with (repo / "Cargo.toml").open("a") as stream:
                stream.write("# historical shared input, outside this fold\n")
            git("add", "README.md", "Cargo.toml")
            git("commit", "-qm", "actual group base")
            base = git("rev-parse", "HEAD")
            for path in ("crates/android/src/lib.rs", "README.md", "REPORT.md"):
                with (repo / path).open("a") as stream:
                    stream.write("// folded member\n")
            git("add", "crates/android/src/lib.rs", "README.md", "REPORT.md")
            git("commit", "-qm", "three folded files")
            candidates = repo / ".git/green.ndjson"
            candidates.write_text(json.dumps({"head_sha": base}) + "\n")
            env = {**os.environ, "FARCOOLER_OPERATION_BINDING": "fixture", "FARCOOLER_BASE": base,
                   "CANDIDATES": str(candidates)}
            out = subprocess.run(["bash", "scripts/landing-gate", "--dry-run"], cwd=repo, env=env, capture_output=True, text=True)
            self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
            self.assertIn(f"3 changed file(s) against {base}; jobs: wire, doc-comments, site, tunnel, rust\n", out.stdout)
            self.assertIn("Rust coverage: nextest runs only consumer, leaf; 1 test binaries skipped", out.stdout)
            self.assertNotIn("nextest runs only consumer, leaf, unrelated", out.stdout)
            # ov-1318: the installed daemon still supplies its startup base.
            # Use the daemon's exact fold-index encoding, including workspace/item.
            env.update(FARCOOLER_WORKSPACE="fixture-workspace", FARCOOLER_WORKFLOW_ITEM="group-1",
                       FARCOOLER_CANDIDATE=git("rev-parse", "HEAD"))
            receipts = repo / ".git/receipts"
            receipts.mkdir()
            env["FARCOOLER_RECEIPT"] = str(receipts / "gate.process.json")
            identity = json.dumps([env["FARCOOLER_WORKSPACE"], env["FARCOOLER_WORKFLOW_ITEM"]],
                                  separators=(",", ":")).encode()
            record = receipts / ("fold-wf1:" + hashlib.sha256(identity).hexdigest()
                                 + "-" + env["FARCOOLER_CANDIDATE"] + ".json")
            fact = {"item": env["FARCOOLER_WORKFLOW_ITEM"], "base": base,
                    "folded": env["FARCOOLER_CANDIDATE"]}
            record.write_text(json.dumps(fact))
            stale = git("rev-parse", "origin/main")
            def plan(given):
                return subprocess.run(["bash", "scripts/landing-gate", "--dry-run"], cwd=repo,
                                      env={**env, "FARCOOLER_BASE": given}, capture_output=True, text=True)
            corrected = plan(stale)
            self.assertEqual(corrected.returncode, 0, corrected.stdout + corrected.stderr)
            self.assertIn(f"3 changed file(s) against {base}; jobs: wire, doc-comments, site, tunnel, rust\n",
                          corrected.stdout)
            self.assertIn("using fold base", corrected.stdout)
            correct = plan(base)
            self.assertEqual(correct.returncode, 0, correct.stdout + correct.stderr)
            self.assertIn(f"3 changed file(s) against {base}; jobs:", correct.stdout)
            self.assertNotIn("using fold base", correct.stdout)
            sibling = git("commit-tree", "HEAD^{tree}", "-p", stale, "-m", "unrelated sibling")
            for invalid in (None, "{bad json", {**fact, "item": "another-group"},
                            {**fact, "folded": stale}, {**fact, "base": "HEAD"},
                            {**fact, "base": "f" * 40}, {**fact, "base": sibling}):
                if invalid is None:
                    record.unlink()
                else:
                    record.write_text(invalid if isinstance(invalid, str) else json.dumps(invalid))
                fallback = plan(stale)
                self.assertEqual(fallback.returncode, 0, fallback.stdout + fallback.stderr)
                self.assertIn(f"against {stale}; jobs:", fallback.stdout)
                self.assertIn("fold base unavailable", fallback.stdout)
                self.assertIn("jobs: " + ", ".join(lane.JOBS), fallback.stdout)
            with patch.dict(os.environ, env):
                proposal = policy.rust_selection(repo, base, lambda name, path: lane.load_module(name, ROOT / "scripts" / path.name))
            self.assertEqual(proposal["packages"], ["consumer", "leaf"])
            self.assertEqual(proposal["selected_binaries"], ["consumer", "leaf"])
            self.assertEqual(proposal["skipped_binaries"], ["unrelated"])

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
            (repo / ".config/nextest.toml").write_text('[profile.gate]\nfail-fast=false\n')
            def gate(full=False):
                step = lane.Step("rust", "Full Rust workspace (local)",
                    './scripts/test-leak-check.py -- ./scripts/tmux-leak-check.py -- cargo nextest run --offline --profile gate --no-fail-fast --archive-file target/rust-tests.tar.zst\n')
                # Match CI's shape; offline is configured in env instead.
                step.run = step.run.replace("run --offline", "run")
                plans = {"rust": [step]}
                policy.apply(plans, {**proposal, "full": full})
                # This temporary Cargo workspace has no repository wrappers.
                step.run = step.run.replace("./scripts/test-leak-check.py -- ./scripts/tmux-leak-check.py -- ", "", 1)
                # No archive in this scratch workspace: run the same selection live.
                step.run = step.run.replace(" --archive-file target/rust-tests.tar.zst", "", 1)
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
