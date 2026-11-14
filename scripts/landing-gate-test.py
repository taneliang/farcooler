#!/usr/bin/env python3
"""ov-1189: combined group selection, CI coverage floor and build-free reports."""
import importlib.util
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), ROOT / "scripts" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


lane = load("lane-gate")
changes = load("ci-changes")


def selected(paths):
    return set(lane.jobs_for(paths, changes.SITE, lambda _: False, {}, changes.families))


class LandingTests(unittest.TestCase):
    def test_ci_families_are_the_minimum_for_every_path_rule(self):
        # Independent mapping: dropping any family-to-job edge must go red.
        jobs = {"rust": {"rust"}, "apple": {"swift", "swift-tests", "ios"},
                "android": {"android"}, "web": {"web"}, "relay": {"relay"}}
        paths = ["apps/macos/a.swift", "apps/ios/a.swift", "apps/shared/a.swift",
                 "apps/android/a.kt", "services/relay/a.ts", "services/relay/test/fixtures/a.json",
                 "services/common/a.ts", "services/web/a.ts", "services/site/a.ts",
                 "crates/android/src/lib.rs", "crates/cli/src/main.rs", "crates/web/src/lib.rs",
                 ".config/nextest.toml", "test/live_agents.rs", "scripts/swift-test-guard.sh",
                 ".github/actions/ios-ui-build/action.yml", "unknown"]
        for path in paths:
            with self.subTest(path=path):
                flags = changes.families([path])
                required = set().union(*(v for k, v in jobs.items() if flags[k]))
                self.assertLessEqual(required, selected([path]))

    def test_gate_covers_ci_job_conditions_including_unconditional_jobs(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("      non_docs: ${{ steps.changes.outputs.non_docs }}", workflow.splitlines())
        for path in ["apps/android/a.kt", "apps/macos/a.swift", "services/site/a.ts",
                     "services/common/a.ts", "services/relay/a.ts", "scripts/site-proof.py",
                     "crates/android/src/lib.rs", "Cargo.lock", "docs/spike.md",
                     "reports/result.txt", ".claude/agent/reports/result.md"]:
            flags = changes.families([path])
            required = set()
            for job in lane.JOBS:
                if job == "doc-comments":  # Separate, unconditional workflow.
                    required.add(job)
                    continue
                block = lane.job_block(workflow, job)
                condition = next((line.strip()[4:] for line in block
                                  if line.startswith("    if:")), None)
                if condition is None:
                    required.add(job)
                    continue
                match = re.fullmatch(
                    r"\$\{\{ !cancelled\(\) && needs.changes.outputs.([a-z_]+) != 'false' \}\}",
                    condition)
                self.assertIsNotNone(match, f"unrecognized CI condition for {job}: {condition}")
                if job != "ios":
                    self.assertIn("    needs: changes", block)
                if flags[match[1]]:
                    required.add(job)
            with self.subTest(path=path):
                self.assertLessEqual(required, selected([path]))

    def test_cross_cutting_paths_select_every_suite(self):
        for path in ["Cargo.lock", "Cargo.toml", "crates/protocol/src/lib.rs",
                     "proto/farcooler.proto", ".github/workflows/ci.yml",
                     ".github/workflows/canary.yml", "scripts/unknown.py"]:
            with self.subTest(path=path):
                self.assertEqual(selected([path]), set(lane.JOBS))

    def test_mixed_group_unions_families(self):
        self.assertEqual(selected(["apps/android/a.kt", "apps/macos/a.swift", "docs/spike.md"]),
                         {"wire", "doc-comments", "site", "tunnel", "android", "relay", "swift", "swift-tests", "ios"})
        for path in ["docs/spike.md", ".claude/agent/reports/ov-1189/result.md", "reports/ov-1189/run.txt"]:
            self.assertEqual(selected([path]), {"wire", "doc-comments"})

    def test_crate_selector_is_used_and_never_filters_ci_rust_steps(self):
        shadow = load("rust-shadow")
        result = subprocess.CompletedProcess([], 0, stdout="{}")
        with patch.object(lane, "load_module", return_value=shadow), \
                patch.object(lane.subprocess, "run", return_value=result) as metadata, \
                patch.object(shadow, "select", return_value=(["farcooler-cli", "farcooler-daemon"], ["closure"], False)) as choose:
            impact = lane.rust_impact(ROOT, [("M", "crates/cli/src/main.rs")])
        self.assertIn("farcooler-daemon", impact)
        choose.assert_called_once_with(ROOT, {}, [("M", "crates/cli/src/main.rs")])
        self.assertIn("--offline", metadata.call_args.args[0])
        self.assertIn("--all-features", metadata.call_args.args[0])
        with patch.object(lane.subprocess, "run", side_effect=OSError("missing cargo")):
            self.assertIn("full workspace", lane.rust_impact(ROOT, [("M", "crates/cli/src/main.rs")]))

    def test_real_wrapper_uses_pinned_combined_diff_and_never_builds_reports(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)
            def git(*args):
                return subprocess.check_output(["git", "-C", tmp, "-c", "commit.gpgsign=false",
                                                "-c", "user.name=t", "-c", "user.email=t@t", *args], text=True).strip()
            def write(name, text):
                p = repo / name
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_text(text)
            git("init", "-q")
            for name in ["landing-gate", "lane-gate.py", "ci-changes.py", "ios-ui-plan.py", "heavy_lock.py"]:
                target = repo / "scripts" / name
                target.parent.mkdir(exist_ok=True)
                shutil.copyfile(ROOT / "scripts" / name, target)
            # Any accidental heavy/unlock invocation or build fails this real gate.
            for name in ["heavy", "wait-for-unlock"]:
                write("scripts/" + name, "#!/bin/sh\necho unexpected-build >&2\nexit 91\n")
                (repo / "scripts" / name).chmod(0o755)
            write("scripts/doc-comment-check.py", "#!/bin/sh\nexit 0\n")
            (repo / "scripts/doc-comment-check.py").chmod(0o755)
            write(".github/workflows/ci.yml", "jobs:\n" + "".join(
                f"  {job}:\n    steps:\n      - run: {'true' if job == 'wire' else 'exit 92'}\n"
                for job in lane.JOBS if job != "doc-comments") + "  ios-ui-plan:\n    steps: []\n")
            write(".github/actions/ios-ui-build/action.yml", "runs:\n  steps: []\n")
            git("add", ".")
            git("commit", "-qm", "base")
            base = git("rev-parse", "HEAD")
            write("docs/spike.md", "findings\n")
            git("add", ".")
            git("commit", "-qm", "first ticket")
            write(".claude/agent/reports/ov-1189/result.md", "report\n")
            git("add", "-f", ".claude/agent/reports/ov-1189/result.md")
            git("commit", "-qm", "second ticket")
            # origin/main already moved: selecting against it would miss both tickets.
            git("update-ref", "refs/remotes/origin/main", "HEAD")
            env = {**os.environ, "FARCOOLER_BASE": base}
            out = subprocess.run(["bash", str(repo / "scripts/landing-gate"), "--worktree", tmp,
                                  "--keep-target", "--no-load-wait"], env=env,
                                 capture_output=True, text=True)
            if out.returncode:
                out.stdout += subprocess.run([sys.executable, str(repo / "scripts/lane-gate.py"), "--worktree", tmp, "--dry-run"], env=env, capture_output=True, text=True).stdout
            self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
            self.assertIn("2 changed file(s) against " + base, out.stdout)
            self.assertIn("jobs: wire, doc-comments", out.stdout)
            self.assertIn("no builds selected", out.stdout)
            self.assertIn("landing-gate: PASS", out.stdout)
            write("apps/android/a.kt", "code\n")
            git("add", ".")
            git("commit", "-qm", "third ticket")
            write("apps/macos/a.swift", "code\n")  # dirty fourth ticket also counts
            self.assertEqual(set(lane.changed_files(repo, base)), {
                "docs/spike.md", ".claude/agent/reports/ov-1189/result.md",
                "apps/android/a.kt", "apps/macos/a.swift"})
            # --base still overrides the engine default, including in dry runs.
            dry = subprocess.run(["bash", str(repo / "scripts/landing-gate"), "--worktree", tmp,
                                  "--base", "HEAD", "--dry-run"], env=env,
                                 capture_output=True, text=True)
            self.assertEqual(dry.returncode, 0, dry.stderr)
            self.assertIn("1 changed file(s) against HEAD; jobs: wire, doc-comments, site, tunnel, swift-tests, swift, ios", dry.stdout)
            git("add", ".")
            git("commit", "-qm", "fourth ticket")
            git("mv", "apps/android/a.kt", "docs/moved.kt")
            self.assertIn(("D", "apps/android/a.kt"), lane.changed_inputs(repo, "HEAD"))
            self.assertIn("android", selected(lane.changed_files(repo, "HEAD")))
            with self.assertRaises(SystemExit):
                lane.changed_files(repo, "missing-base")


if __name__ == "__main__":
    unittest.main()
