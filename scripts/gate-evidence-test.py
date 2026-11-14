#!/usr/bin/env python3
"""Tiny real-process v2 fixtures; never runs app jobs, a heavy queue or targets."""
import copy
import importlib.util
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from types import SimpleNamespace
from unittest.mock import patch

sys.dont_write_bytecode = True
sys.path.insert(0, str(pathlib.Path(__file__).parent))
import gate_evidence as evidence

ROOT = pathlib.Path(__file__).resolve().parent.parent


class EvidenceTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="ov903-fixture-")
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name).resolve()
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.scripts = self.repo / "scripts"
        self.scripts.mkdir()
        for name in ("gate_evidence.py", "lane-gate.py", "heavy_lock.py", "ci-changes.py", "ios-ui-plan.py", "gate-summary-check.py"):
            shutil.copyfile(ROOT / "scripts" / name, self.scripts / name)
        (self.repo / ".github/actions/ios-ui-build").mkdir(parents=True)
        (self.repo / ".github/actions/ios-ui-build/action.yml").write_text("runs:\n  steps:\n")
        (self.repo / ".github/workflows").mkdir()
        jobs = []
        for job in [j for j in evidence.load_lane().JOBS if j != "doc-comments"] + ["ios-ui-plan"]:
            jobs.append(f"  {job}:\n    steps:\n      - run: printf '{job} output\\n'\n")
        # The local Rust plan is derived from CI's archive consumer even when
        # this fixture executes only wire/doc-comments jobs.
        jobs.append("  rust-tests:\n    steps:\n      - name: Full Rust workspace shard\n"
                    "        run: ./scripts/test-leak-check.py -- ./scripts/tmux-leak-check.py -- cargo nextest run --profile ci --no-fail-fast --archive-file target/tests.tar.zst --partition hash:1/4\n")
        jobs[0] = "  wire:\n    steps:\n      - run: mkdir -p .fixture; printf 'wire\\n' >> .fixture/wire-count\n      - run: printf 'raw wire output\\n'\n"
        (self.repo / ".github/workflows/ci.yml").write_text("jobs:\n" + "".join(jobs))
        # The producer is deliberately stateful, like a transient first-attempt
        # failure. Its state is an ignored fixture output, never reused as a pass.
        doc = self.scripts / "doc-comment-check.py"
        doc.write_text("#!/bin/sh\nmkdir -p .fixture\nprintf '%s\\n' \"$*\" >> .fixture/doc-count\n"
                       "if [ -e .fixture/pause ]; then touch .fixture/started; sleep 2; fi\n"
                       "if [ \"$1\" = --self-test ] && [ ! -e .fixture/attempt ]; then\n"
                       "  touch .fixture/attempt; printf 'first attempt failed\\n'; exit 7\nfi\n"
                       "printf 'doc passed\\n'\n")
        doc.chmod(0o755)
        (self.repo / ".gitignore").write_text(".fixture/\n__pycache__/\n")
        (self.repo / "README.md").write_text("base\n")
        self.git("init", "-q")
        self.git("config", "user.name", "fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("config", "commit.gpgsign", "false")
        self.git("add", ".")
        self.git("commit", "-qm", "base")
        self.base = self.git("rev-parse", "HEAD").strip()
        (self.repo / "README.md").write_text("head\n")
        self.git("commit", "-qam", "head")
        self.inputs = self.root / "public-inputs.json"
        self.inputs.write_text(json.dumps({"schema": 1, "jobs": {
            j: {"tools": ["sh"], "sdk": [], "cache": [], "native": []}
            for j in ("wire", "doc-comments")}}))
        self.env = {**os.environ, "PATH": "/usr/bin:/bin:/usr/sbin:/sbin:" + os.environ.get("PATH", ""),
                    "PYTHONDONTWRITEBYTECODE": "1"}
        self.pointer = self.repo / ".git/lane-gate-summary.txt"

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.repo), *args], text=True, stderr=subprocess.STDOUT)

    def cli(self, *args, code=0):
        p = subprocess.run([sys.executable, str(self.scripts / "gate_evidence.py"),
                            "--worktree", str(self.repo), "--base", self.base,
                            "--inputs", str(self.inputs), *map(str, args)],
                           env=self.env, capture_output=True, text=True)
        self.assertEqual(p.returncode, code, p.stdout + p.stderr)
        return p

    def checker(self, code, *args):
        p = subprocess.run([sys.executable, str(self.scripts / "gate-summary-check.py"),
                            "--worktree", str(self.repo), *args],
                           env=self.env, capture_output=True, text=True)
        self.assertEqual(p.returncode, code, p.stdout + p.stderr)
        return p

    def first(self):
        self.cli("--run", "--evidence-dir", self.root / "first", code=1)
        original = self.root / "first-pointer.txt"
        original.write_bytes(self.pointer.read_bytes())
        self.assertEqual(json.loads((self.root / "first/manifest.json").read_text())["jobs"]["doc-comments"][0]["exit"], 7)
        return original

    def green(self):
        original = self.first()
        self.cli("--run", "--evidence-dir", self.root / "second", "--resume", original)
        return original

    def rewrite(self, directory, data):
        """Rebind a fixture, so semantic reds cannot pass just on digest checks."""
        summary = directory / "summary.txt"
        summary.chmod(0o600)
        summary.write_text(evidence.summary_text(data))
        data["summary_sha256"] = evidence.file_digest(summary)
        manifest = directory / "manifest.json"
        manifest.chmod(0o600)
        manifest.write_bytes(evidence.canonical(data))
        self.pointer.write_text(f"lane-gate-summary v2\nevidence: {directory}\nsha256: {evidence.file_digest(manifest)}\n")

    def test_real_failed_job_retry_and_private_immutable_chain(self):
        self.cli("--plan")
        self.assertFalse((self.root / "first").exists())
        original = self.first()
        before = {p.name: p.read_bytes() for p in (self.root / "first").iterdir()}
        self.checker(1)
        waived = self.checker(1, "--waive", "fixture cannot waive v2")
        self.assertIn("cannot be waived", waived.stderr)
        self.assertFalse((self.repo / ".git/gate-waivers.log").exists())
        planned = json.loads(self.cli("--plan", "--resume", original).stdout)
        self.assertEqual(planned["execute"], ["doc-comments"])
        self.cli("--run", "--evidence-dir", self.root / "second", "--resume", original)
        self.checker(0)
        self.assertEqual((self.repo / ".fixture/wire-count").read_text(), "wire\n")
        self.assertEqual(len((self.repo / ".fixture/doc-count").read_text().splitlines()), 4)
        self.assertEqual(before, {p.name: p.read_bytes() for p in (self.root / "first").iterdir()})
        data = json.loads((self.root / "second/manifest.json").read_text())
        self.assertEqual(data["selected"], ["doc-comments"])
        self.assertIn("--base " + self.base, data["jobs"]["doc-comments"][1]["command"])
        self.assertEqual((self.root / "first").stat().st_mode & 0o777, 0o700)
        for p in (self.root / "first").iterdir():
            self.assertEqual(p.stat().st_mode & 0o777, 0o400)

    def test_v1_missing_unsealed_and_existing_directory_refused(self):
        v1 = self.root / "v1.txt"
        v1.write_text("lane-gate-summary v1\nhead: x\nverdict: ok\n")
        self.cli("--plan", "--resume", v1, code=1)
        parent = self.first()
        self.cli("--run", "--evidence-dir", self.root / "first", "--resume", parent, code=1)
        manifest = self.root / "first/manifest.json"
        manifest.unlink()
        self.cli("--plan", "--resume", parent, code=1)

    def test_current_source_base_ci_environment_and_inputs_drift(self):
        self.green()
        self.checker(0)
        readme = self.repo / "README.md"
        readme.write_text("uncommitted fix\n")
        self.checker(1)
        self.cli("--plan", code=1)
        self.git("checkout", "--", "README.md")
        extra = self.repo / "untracked"
        extra.write_text("untracked input")
        self.checker(1)
        extra.unlink()
        self.git("add", "README.md")
        self.git("commit", "--allow-empty", "-qm", "new SHA same tree")
        self.checker(1)

    def test_stale_symbolic_base_and_prerequisite_bytes(self):
        self.git("branch", "fixture-base", self.base)
        self.base = "fixture-base"
        sdk = self.root / "sdk-public-version"
        sdk.write_text("version 1")
        config = json.loads(self.inputs.read_text())
        config["jobs"]["wire"]["sdk"] = [str(sdk)]
        self.inputs.write_text(json.dumps(config))
        self.green()
        self.checker(0)
        sdk.write_text("version 2")
        self.checker(1)
        sdk.write_text("version 1")
        self.checker(0)
        self.git("branch", "-f", "fixture-base", "HEAD")
        self.checker(1)

    def test_environment_and_ci_plan_change(self):
        self.green()
        self.checker(0)
        old = self.env.get("LANG")
        self.env["LANG"] = "C.fixture"
        self.checker(1)
        if old is None:
            self.env.pop("LANG")
        else:
            self.env["LANG"] = old
        ci = self.repo / ".github/workflows/ci.yml"
        ci.write_text(ci.read_text().replace("raw wire output", "different required command"))
        self.checker(1)

    def test_semantic_counterexamples_even_when_rehashed(self):
        self.green()
        directory = self.root / "second"
        original = json.loads((directory / "manifest.json").read_text())
        mutations = [
            ("missing row", lambda d: d["jobs"]["doc-comments"].pop()),
            ("unknown exit", lambda d: d["jobs"]["doc-comments"][0].update(exit=None)),
            ("boolean exit", lambda d: d["jobs"]["doc-comments"][0].update(exit=False)),
            ("protocol failure with numeric zero", lambda d: d["jobs"]["doc-comments"][0].update(error="protocol failed")),
            ("nonzero exit", lambda d: d["jobs"]["doc-comments"][0].update(exit=9)),
            ("changed command", lambda d: d["jobs"]["doc-comments"][0].update(command="true")),
            ("changed cwd", lambda d: d["jobs"]["doc-comments"][0].update(cwd="elsewhere")),
            ("changed env", lambda d: d["jobs"]["doc-comments"][0]["env"].update(CI="false")),
            ("duplicate row", lambda d: d["jobs"]["doc-comments"].append(d["jobs"]["doc-comments"][0])),
            ("out of order", lambda d: d["jobs"]["doc-comments"][0].update(index=1)),
            ("partial jobs", lambda d: d.update(selected=[])),
            ("run error", lambda d: d.update(error="interrupted")),
            ("boundary drift", lambda d: d.update(invalidated=True)),
        ]
        for name, mutate in mutations:
            with self.subTest(name=name):
                data = copy.deepcopy(original)
                mutate(data)
                self.rewrite(directory, data)
                self.checker(1)
        self.rewrite(directory, original)
        self.checker(0)

    def test_missing_tampered_parent_log_summary_and_start(self):
        self.green()
        self.checker(0)
        for relative in ("first/wire-0.log", "first/summary.txt", "first/start.json", "first/manifest.json", "second/previous-pointer.txt"):
            with self.subTest(relative=relative):
                path = self.root / relative
                saved = path.read_bytes()
                path.chmod(0o600)
                path.write_bytes(saved + b"tamper")
                self.checker(1)
                path.write_bytes(saved)
                path.unlink()
                self.checker(1)
                path.write_bytes(saved)
        self.checker(0)

    def test_dirty_input_never_attests_committed_sha(self):
        (self.repo / "README.md").write_text("dirty fix")
        self.cli("--run", "--evidence-dir", self.root / "dirty", code=1)
        self.assertFalse((self.root / "dirty").exists())
        self.git("add", "README.md")
        self.cli("--plan", code=1)

    def test_hidden_index_flags_and_ignored_mode_drift_cannot_attest_head(self):
        for flag, undo in (("--assume-unchanged", "--no-assume-unchanged"), ("--skip-worktree", "--no-skip-worktree")):
            with self.subTest(flag=flag):
                self.git("update-index", flag, "README.md")
                (self.repo / "README.md").write_text("hidden uncommitted fix")
                self.cli("--plan", code=1)
                self.git("update-index", undo, "README.md")
                self.git("checkout", "--", "README.md")
        self.git("config", "core.fileMode", "false")
        (self.repo / "README.md").chmod(0o755)
        self.cli("--plan", code=1)

    def test_stat_cache_cannot_hide_same_size_dirty_fix(self):
        path = self.repo / "README.md"
        old = time.time() - 30
        os.utime(path, (old, old))
        self.git("update-index", "--refresh")
        saved = path.stat()
        self.git("config", "core.trustctime", "false")
        path.write_text("evil\n")  # same byte count as committed head\n
        os.utime(path, ns=(saved.st_atime_ns, saved.st_mtime_ns))
        self.assertEqual(self.git("status", "--porcelain"), "")
        self.cli("--plan", code=1)

    def test_saved_parent_retry_with_shared_history_is_a_dag(self):
        original = self.green()
        self.cli("--run", "--evidence-dir", self.root / "third", "--resume", original)
        self.checker(0)
        # A fresh full capture audits the old graph without importing coverage.
        self.cli("--run", "--evidence-dir", self.root / "fourth")
        self.checker(0)
        log = self.root / "second/doc-comments-0.log"
        log.unlink()
        self.checker(1)

    def test_missing_declared_prerequisite_fails_before_execution(self):
        config = json.loads(self.inputs.read_text())
        config["jobs"]["wire"]["native"] = [str(self.root / "absent.so")]
        self.inputs.write_text(json.dumps(config))
        self.cli("--run", "--evidence-dir", self.root / "absent", code=1)
        self.assertFalse((self.repo / ".fixture").exists())
        self.assertFalse((self.root / "absent").exists())

    def test_shell_failure_unknown_start_and_environment_outputs(self):
        step = {"run": "printf 'raw error\\n'; exit 23", "cwd": None, "env": {}}
        tmp = self.root / "step-temp"
        tmp.mkdir()
        env = {"PATH": "/usr/bin:/bin"}
        row = evidence.execute_step(step, 0, env, self.repo, tmp, self.root / "exit.log")
        self.assertEqual(row["exit"], 23)
        self.assertEqual((self.root / "exit.log").read_text(), "raw error\n")
        with patch.object(evidence.subprocess, "Popen", side_effect=OSError("cannot spawn")):
            row = evidence.execute_step(step, 0, env, self.repo, tmp, self.root / "unknown.log")
        self.assertIsNone(row["exit"])
        self.assertIn("cannot spawn", row["error"])
        step["run"] = 'printf "UNRECOGNIZED=value\\n" > "$GITHUB_ENV"'
        row = evidence.execute_step(step, 0, env, self.repo, tmp, self.root / "env.log")
        self.assertEqual(row["exit"], 0)
        self.assertIsNotNone(row["error"])
        self.assertEqual(row["updates"], {})
        step["run"] = 'printf "\\377" > "$GITHUB_ENV"'
        row = evidence.execute_step(step, 0, env, self.repo, tmp, self.root / "malformed.log")
        self.assertEqual(row["exit"], 0)
        self.assertIsNotNone(row["error"])
        self.assertTrue(row["log_sha256"])

    def test_android_public_sdk_ndk_metadata_preflight(self):
        sdk = self.root / "sdk"
        ndk = sdk / "ndk/29.0.14206865"
        host = "darwin-x86_64" if sys.platform == "darwin" else "linux-x86_64"
        bin_dir = ndk / "toolchains/llvm/prebuilt" / host / "bin"
        public = [ndk / "source.properties", sdk / "platforms/android-37.1/android.jar",
                  sdk / "build-tools/37.0.0/source.properties"]
        public += [sdk / "build-tools/37.0.0" / n for n in ("aapt2", "zipalign", "apksigner")]
        public += [bin_dir / n for n in ("aarch64-linux-android37-clang", "clang", "llvm-ar")]
        for path in public:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("public fixture metadata\n")
            path.chmod(0o755)
        shutil.copyfile(ROOT / "scripts/android-ndk.sh", self.scripts / "android-ndk.sh")
        lane = evidence.load_lane()
        plans = {"android": [lane.Step("android", "SDK install", "sdkmanager 'ndk;29.0.14206865' 'platforms;android-37.1' 'build-tools;37.0.0'", skip="CI setup"),
                             lane.Step("android", "native build", "./scripts/build-android-libs.sh")]}
        env = evidence.android_environment({"PATH": self.env["PATH"], "ANDROID_HOME": str(sdk)}, plans)
        self.assertEqual(env["ANDROID_NDK_HOME"], str(ndk))
        with self.assertRaisesRegex(ValueError, "differs"):
            evidence.android_environment({**env, "ANDROID_NDK_HOME": str(self.root / "another-ndk")}, plans)
        declared = self.root / "declared-public-input"
        declared.write_text("immutable SDK/native/cache fixture")
        declaration = {"schema": 1, "jobs": {"android": {"tools": ["sh"], "sdk": [str(declared)],
                                                       "cache": [str(declared)], "native": [str(declared)]}}}
        # Isolate SDK discovery without invoking Rust/Go/JDK tools. Production
        # mandatory-tool policy stays intact; this fixture never builds anything.
        with patch.dict(evidence.JOB_TOOLS, {"android": ["sh"]}):
            original = evidence.prerequisites(lane, self.repo, ["android"], plans, {"android": env}, declaration)
            compiler = bin_dir / "clang"
            compiler.write_text("different selected compiler")
            changed = evidence.prerequisites(lane, self.repo, ["android"], plans, {"android": env}, declaration)
            self.assertNotEqual(original, changed)
            for path in (sdk / "platforms/android-37.1/android.jar", sdk / "build-tools/37.0.0/aapt2", bin_dir / "llvm-ar", declared):
                with self.subTest(missing=str(path)):
                    saved = path.read_bytes()
                    path.unlink()
                    with self.assertRaises((ValueError, subprocess.CalledProcessError)):
                        evidence.prerequisites(lane, self.repo, ["android"], plans, {"android": env}, declaration)
                    path.write_bytes(saved)
                    path.chmod(0o755)

    def test_pending_capture_and_lock_refuse_old_green(self):
        self.green()
        self.checker(0)
        lane = evidence.load_lane()
        with evidence.capture_lock(lane, self.repo):
            self.checker(1, "--waive", "still running")
        self.checker(0)
        spec = importlib.util.spec_from_file_location("fixture_checker", self.scripts / "gate-summary-check.py")
        checker = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(checker)
        def while_checking(repo):
            with self.assertRaises(BlockingIOError):
                with evidence.capture_lock(lane, repo):
                    self.fail("writer acquired lock during verification")
            return "fixture verification completed"
        with patch.object(checker, "_problem", side_effect=while_checking):
            self.assertEqual(checker.problem(self.repo), "fixture verification completed")
        marker = self.repo / ".git/gate-evidence.lock"
        marker.unlink()
        calls = []
        def first_activation(repo):
            calls.append(True)
            if len(calls) == 1:
                marker.touch()
                return "old unlocked result"
            return while_checking(repo)
        with patch.object(checker, "_problem", side_effect=first_activation):
            self.assertEqual(checker.problem(self.repo), "fixture verification completed")
            self.assertEqual(len(calls), 2)
        evidence.replace_pointer(self.pointer, "lane-gate-summary v2 pending\nevidence: /missing/unsealed\n")
        self.checker(1, "--waive", "killed attempt")
        self.pointer.unlink()
        self.checker(1, "--waive", "missing v2 pointer")
        self.pointer.write_text("lane-gate-summary v1\nhead: " + self.git("rev-parse", "HEAD").strip() + "\nverdict: ok\n")
        self.checker(1, "--waive", "downgrade to legacy")

    def test_sigkill_new_capture_cannot_leave_previous_green(self):
        self.green()
        self.checker(0)
        (self.repo / ".fixture/pause").touch()
        p = subprocess.Popen([sys.executable, str(self.scripts / "gate_evidence.py"),
                              "--worktree", str(self.repo), "--base", self.base,
                              "--inputs", str(self.inputs), "--run", "--evidence-dir", str(self.root / "killed")],
                             env=self.env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            deadline = time.monotonic() + 10
            while not (self.repo / ".fixture/started").exists() and p.poll() is None and time.monotonic() < deadline:
                time.sleep(0.02)
            self.assertTrue((self.repo / ".fixture/started").exists())
            self.checker(1)
            p.kill()
            p.communicate(timeout=5)
            self.assertIn("v2 pending", self.pointer.read_text())
            self.assertFalse((self.root / "killed/manifest.json").exists())
            self.checker(1, "--waive", "SIGKILL")
        finally:
            if p.poll() is None:
                p.kill()
            p.communicate(timeout=5)


if __name__ == "__main__":
    unittest.main()
