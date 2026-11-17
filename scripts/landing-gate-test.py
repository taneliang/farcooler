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


class DiskTests(unittest.TestCase):
    def test_low_disk_wrapper_refuses_within_a_minute_without_deleting(self):
        with tempfile.TemporaryDirectory() as temporary:
            repo = Path(temporary).resolve()
            for name in ("landing-gate", "lane-gate.py", "landing_policy.py", "gate_disk.py",
                         "ci-changes.py", "ios-ui-plan.py", "heavy_lock.py", "process_guard.py",
                         "gate_evidence.py"):
                target = repo / "scripts" / name
                target.parent.mkdir(exist_ok=True)
                shutil.copyfile(ROOT / "scripts" / name, target)
            for name in (".github/workflows/ci.yml", ".github/actions/ios-ui-build/action.yml"):
                target = repo / name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / name, target)
            for args in (("init", "-q"), ("add", "scripts", ".github"),
                         ("-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false",
                          "commit", "-qm", "fixture")):
                subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True)
            sentinel = repo / "target/unrelated"
            sentinel.parent.mkdir()
            sentinel.write_text("keep")
            (repo / "scripts/changed.py").write_text("# changed\n")
            # Inject capacity in each real CLI process; any removal is a failure.
            (repo / "sitecustomize.py").write_text("""
import os, shutil, sys, tempfile
tempfile.tempdir = '/tmp'
shutil.disk_usage = lambda _: type('Usage', (), {'free': 21 * 1024 ** 3})()
def audit(event, args):
    if event in ('os.remove', 'os.rmdir', 'shutil.rmtree'):
        raise AssertionError('UNEXPECTED DELETION: ' + event)
sys.addaudithook(audit)
""")
            env = {k: v for k, v in os.environ.items() if not k.startswith("FARCOOLER_")}
            env.update(PYTHONPATH=str(repo), PYTHONDONTWRITEBYTECODE="1")
            done = subprocess.run(["bash", str(repo / "scripts/landing-gate"),
                                   "--worktree", str(repo), "--base", "HEAD", "--only", "rust"],
                                  cwd=repo, env=env, capture_output=True, text=True, timeout=60)
            self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
            self.assertIn("free 21.00 GB; needed 70.00 GB", done.stdout, done.stderr)
            self.assertNotIn("UNEXPECTED", done.stdout + done.stderr)
            self.assertEqual(sentinel.read_text(), "keep")

    def test_admission_boundary(self):
        import gate_disk as disk
        for free in (disk.NEEDED - 1, disk.NEEDED, disk.NEEDED + 1):
            with self.subTest(free=free), patch.object(disk.shutil, "disk_usage") as usage:
                usage.return_value.free = free
                if free < disk.NEEDED:
                    with self.assertRaises(ValueError):
                        disk.admit(ROOT, say=lambda *a, **k: None)
                else:
                    disk.admit(ROOT, say=lambda *a, **k: None)

    def test_failed_extraction_keeps_archive_and_unrelated_files(self):
        import gate_disk as disk
        for failure in (subprocess.CalledProcessError(1, "extract"), None):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temporary:
                repo = Path(temporary).resolve()
                archive = repo / "target/rust-tests.tar.zst"
                archive.parent.mkdir()
                archive.write_text("compressed")
                keep = archive.parent / "keep"
                keep.write_text("unrelated")
                with patch.object(disk.subprocess, "run", side_effect=failure):
                    with self.assertRaises((subprocess.CalledProcessError, ValueError)):
                        disk.extract_archive(repo, ["cargo", "nextest", "run", "--archive-file", str(archive)])
                self.assertEqual(archive.read_text(), "compressed")
                self.assertEqual(keep.read_text(), "unrelated")

    def test_archive_removal_refuses_other_names_and_symlink_escapes(self):
        import gate_disk as disk
        with tempfile.TemporaryDirectory() as temporary:
            repo = Path(temporary).resolve()
            target = repo / "target"
            target.mkdir()
            keep = target / "unrelated.tar.zst"
            keep.write_text("keep")
            archive = target / "rust-tests.tar.zst"
            archive.symlink_to(keep)
            with patch.object(disk.subprocess, "run") as extract:
                for path in (keep, archive):
                    with self.assertRaises(ValueError):
                        disk.extract_archive(repo, ["cargo", "nextest", "run", "--archive-file", str(path)])
                extract.assert_not_called()
            self.assertEqual(keep.read_text(), "keep")
            self.assertTrue(archive.is_symlink())

    def test_archive_deleted_after_extraction_and_reruns_use_metadata(self):
        import gate_disk as disk
        flaky = load('gate-flaky')
        with tempfile.TemporaryDirectory() as temporary:
            repo = Path(temporary).resolve()
            archive = repo / 'target/rust-tests.tar.zst'
            archive.parent.mkdir()
            archive.write_text('compressed')
            unrelated = archive.parent / 'unrelated.txt'
            unrelated.write_text('keep')
            destination = repo
            def extract(command, **kwargs):
                self.assertIn('--extract-to', command)
                self.assertTrue(archive.exists())
                metadata = destination / 'target/nextest'
                metadata.mkdir(parents=True)
                for name in ('cargo-metadata.json', 'binaries-metadata.json'):
                    (metadata / name).write_text('{}')
            def run(command, *a):
                self.assertFalse(archive.exists())
                self.assertTrue(destination.exists())
                self.assertNotIn('--archive-file', command)
                self.assertIn('--binaries-metadata', command)
                self.assertNotIn('--extract-to', command)
                self.assertNotIn('--extract-overwrite', command)
                self.assertEqual(command.count('--workspace-remap'), 1)
                self.assertEqual(command[-2:], ['-E', 'package(=test)'])
                return 7
            actual = disk.subprocess.run
            def subprocess_run(command, **kwargs):
                return extract(command, **kwargs) if command[:3] == ['cargo', 'nextest', 'list'] else actual(command, **kwargs)
            with patch.dict(os.environ, {'GATE_DISK_REPO': str(repo)}), \
                    patch.object(disk.subprocess, 'run', side_effect=subprocess_run), \
                    patch.object(flaky, 'run_extracted', side_effect=run):
                self.assertEqual(flaky.run(['cargo', 'nextest', 'run', '--archive-file', str(archive),
                                            '--extract-to', '.', '--extract-overwrite', '--workspace-remap', str(repo),
                                            '-E', 'package(=test)'], say=lambda *a, **k: None), 7)
            self.assertTrue((destination / 'target/nextest/binaries-metadata.json').exists())
            self.assertEqual(unrelated.read_text(), 'keep')


class LandingTests(unittest.TestCase):
    def setUp(self):
        environment = patch.dict(os.environ)
        environment.start()
        self.addCleanup(environment.stop)
        for key in list(os.environ):
            if key.startswith("FARCOOLER_"):
                os.environ.pop(key)

    def test_wire_step_refuses_a_field_renamed_since_the_recorded_baseline(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        block = lane.job_block(workflow, "wire")
        self.assertIn("          fetch-depth: 0", block)
        commands = [run.strip() for _, run, _, _ in lane.parse_steps(block)]
        command = "./scripts/proto-lint.py --channel canary --since-baseline"
        self.assertIn(command, commands)
        # The landing gate consumes these same CI run steps. Reproduce the gap:
        # the baseline predates tag 3, but a later main commit already named it.
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)

            def git(*args):
                return subprocess.run(["git", "-C", str(repo), "-c", "user.name=t",
                                       "-c", "user.email=t@t", "-c", "commit.gpgsign=false", *args],
                                      check=True, capture_output=True, text=True).stdout.strip()

            for name in ["scripts/proto-lint.py", "crates/protocol/src/lib.rs",
                         "crates/daemon/src/rpc.rs"]:
                dest = repo / name
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_text((ROOT / name).read_text())
            proto = repo / "proto/farcooler.proto"
            proto.parent.mkdir()
            original = "message TaskList { optional bytes next_page_after = 3; }\n"
            proto.write_text("message TaskList {}\n")
            git("init", "--quiet")
            git("add", "proto/farcooler.proto")
            git("commit", "--quiet", "-m", "baseline before paging")
            recorded = git("rev-parse", "HEAD")
            baseline = repo / "proto/baseline/canary.proto"
            baseline.parent.mkdir()
            baseline.write_text(f"// Shipped by Canary at {recorded}.\n" + proto.read_text())
            proto.write_text(original)
            git("add", "proto/farcooler.proto")
            git("commit", "--quiet", "-m", "add paging cursor")
            git("commit", "--quiet", "--allow-empty", "-m", "candidate")

            def run(strict):
                args = [sys.executable, str(repo / "scripts/proto-lint.py"), "--channel", "canary"]
                return subprocess.run(args + (["--since-baseline"] if strict else []),
                                      cwd=repo, capture_output=True, text=True)

            self.assertEqual(run(True).returncode, 0)
            proto.write_text(original.replace("next_page_after", "next_after"))
            self.assertEqual(run(False).returncode, 0)  # The old CI check missed it.
            refused = run(True)
            self.assertEqual(refused.returncode, 1, refused.stdout + refused.stderr)
            self.assertIn("TaskList tag 3 was renamed from next_page_after to next_after",
                          refused.stderr)

    def test_canary_recovery_survives_fold_and_fresh_clone(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo, fresh = Path(tmp) / "repo", Path(tmp) / "fresh"
            repo.mkdir()

            def git(root, *args, check=True):
                return subprocess.run(
                    ["git", "-C", str(root), "-c", "user.name=t", "-c", "user.email=t@t",
                     "-c", "commit.gpgsign=false", *args], check=check,
                    capture_output=True, text=True)

            def commit(message, *paths):
                git(repo, "add", *paths)
                git(repo, "commit", "--quiet", "-m", message)
                return git(repo, "rev-parse", "HEAD").stdout.strip()

            git(repo, "init", "--quiet", "-b", "main")
            for name in ["scripts/proto-lint.py", "crates/protocol/src/lib.rs",
                         "crates/daemon/src/rpc.rs"]:
                dest = repo / name
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_text((ROOT / name).read_text())
            proto = repo / "proto/farcooler.proto"
            proto.parent.mkdir()
            shipped_proto = "message TaskList {}\n"
            original = "message TaskList { optional bytes next_page_after = 3; }\n"
            renamed = original.replace("next_page_after", "next_after")
            proto.write_text(shipped_proto)
            shipped = commit("last shipped wire", "scripts", "crates", "proto")
            baseline = repo / "proto/baseline/canary.proto"
            baseline.parent.mkdir()

            def snapshot(anchor, wire=original):
                return f"// Shipped by Canary at {anchor}.\n" + wire

            baseline.write_text(snapshot(shipped, shipped_proto))
            proto.write_text(original)
            commit("unshipped paging cursor", "proto")
            proto.write_text(renamed)
            main_tip = commit("unshipped rename", "proto")
            git(repo, "checkout", "--quiet", "-b", "candidate")
            proto.write_text(original)
            # Verify restoration against the original addition before recovering
            # history. Merely keeping the renamed schema is not a repair.
            wire = load("proto-lint")
            self.assertEqual(wire.compare(wire.parse(original), wire.parse(proto.read_text())), [])
            self.assertTrue(wire.compare(wire.parse(original), wire.parse(renamed)))
            repair = commit("restore original field name", "proto/farcooler.proto")
            # With the original history window, only the unshipped renamed
            # version objects to restoration; the original addition accepts it.
            historical, _ = wire.since_baseline_problems(repo, baseline.read_text(), original)
            self.assertTrue(historical)
            self.assertTrue(all("renamed from next_after to next_page_after" in p for p in historical), historical)
            # The snapshot enforces the original name. Only the history anchor
            # skips the unshipped rename, using an existing main commit whose ID
            # survives fold's -x picks.
            baseline.write_text(snapshot(main_tip))
            recovery = commit("recover baseline", "proto/baseline/canary.proto")
            git(repo, "checkout", "--quiet", "main")
            git(repo, "cherry-pick", "-x", repair, recovery)
            self.assertNotEqual(git(repo, "rev-parse", "HEAD").stdout.strip(), recovery)
            # Clone only main over file://: neither alternates nor the candidate
            # ref may hide a dependency on its otherwise unreachable objects.
            git(repo, "clone", "--quiet", "--single-branch", "--branch", "main",
                repo.as_uri(), str(fresh))
            for candidate in (repair, recovery):
                self.assertNotEqual(git(fresh, "cat-file", "-e", candidate, check=False).returncode, 0)

            def lint():
                return subprocess.run(
                    [sys.executable, str(fresh / "scripts/proto-lint.py"),
                     "--channel", "canary", "--since-baseline"],
                    cwd=fresh, capture_output=True, text=True)

            out = lint()
            self.assertEqual(out.returncode, 0, out.stdout + out.stderr)
            fresh_baseline = fresh / "proto/baseline/canary.proto"
            fresh_baseline.write_text(snapshot(repair))
            refused = lint()  # Reproduce the returned build's candidate-only anchor.
            self.assertNotEqual(refused.returncode, 0)
            self.assertIn("which is not in this history", refused.stderr)
            fresh_baseline.write_text(snapshot(main_tip))
            (fresh / "proto/farcooler.proto").write_text(
                original.replace("next_page_after", "next_cursor"))
            refused = lint()
            self.assertEqual(refused.returncode, 1, refused.stdout + refused.stderr)
            self.assertIn("TaskList tag 3 was renamed from next_page_after to next_cursor",
                          refused.stderr)

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
                     "services/common/a.ts", "services/relay/a.ts", "scripts/unreviewed.py",
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
                # Skipped only for main's push of a commit CI already passed on its landing
                # branch (ov-1269), which a local gate never is: these always run here.
                if condition is None or condition == "${{ !cancelled() && needs.changes.outputs.proven != 'true' }}":
                    required.add(job)
                    continue
                if condition == "${{ always() }}":
                    # An aggregate status job (the android gate): it must run the
                    # moment any job it waits on does, so follow their conditions.
                    needed = re.search(r"    needs: \[([^]]+)\]", "\n".join(block))
                    self.assertIsNotNone(needed, f"{job}: always() without needs")
                    upstream = []
                    for dep in (d.strip() for d in needed[1].split(",")):
                        if dep == "changes":
                            continue
                        cond = next((l.strip()[4:] for l in lane.job_block(workflow, dep)
                                     if l.startswith("    if:")), "")
                        m = re.fullmatch(r"\$\{\{ !cancelled\(\) && needs.changes.outputs.([a-z_]+) != 'false' \}\}", cond)
                        self.assertIsNotNone(m, f"unrecognized CI condition for {dep}: {cond}")
                        upstream.append(m[1])
                    self.assertTrue(upstream, f"{job}: aggregates no conditional job")
                    if any(flags[f] for f in upstream):
                        required.add(job)
                    continue
                match = re.fullmatch(
                    r"\$\{\{ !cancelled\(\) && needs.changes.outputs.([a-z_]+) != 'false' \}\}",
                    condition)
                self.assertIsNotNone(match, f"unrecognized CI condition for {job}: {condition}")
                if job != "ios":
                    needs = next((l.strip()[len("needs:"):].strip() for l in block
                                  if l.startswith("    needs:")), "")
                    self.assertIn("changes", [d.strip() for d in needs.strip("[]").split(",")],
                                  f"{job}: must need changes")
                if flags[match[1]]:
                    required.add(job)
            with self.subTest(path=path):
                self.assertLessEqual(required, selected([path]))

    def test_reviewed_script_users_override_the_ci_non_docs_floor(self):
        # ov-1326: the wire self-test consumes the generator; site/tunnel do not.
        self.assertEqual(selected(["scripts/site-proof.py"]), {"wire", "doc-comments"})

    def test_cross_cutting_paths_select_every_suite(self):
        for path in ["Cargo.lock", "Cargo.toml", "crates/protocol/src/lib.rs",
                     "proto/farcooler.proto", ".github/workflows/ci.yml",
                     ".github/workflows/canary.yml", "scripts/unknown.py"]:
            with self.subTest(path=path):
                self.assertEqual(selected([path]), set(lane.JOBS))

    def test_mixed_group_unions_families(self):
        self.assertEqual(selected(["apps/android/a.kt", "apps/macos/a.swift", "docs/spike.md"]),
                         {"wire", "doc-comments", "site", "tunnel", "android", "relay", "swift", "swift-tests", "ios"})
        for path in ["docs/spike.md", "docs/second.md", ".claude/agent/reports/ov-1189/result.md", "reports/ov-1189/run.txt"]:
            self.assertEqual(selected([path]), {"wire", "doc-comments"})

    def test_crate_selector_is_used_and_never_filters_ci_rust_steps(self):
        shadow = load("rust-select")
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
            for name in ["landing-gate", "lane-gate.py", "ci-changes.py", "ios-ui-plan.py", "heavy_lock.py", "process_guard.py", "landing_policy.py", "gate_evidence.py", "gate_disk.py"]:
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
                for job in lane.JOBS if job != "doc-comments") + "".join(f"  {j}:\n    steps:\n      - run: exit 92\n" for j in ("tunnel-helper", "android-build", "android-ui")) + "  rust-build:\n    steps:\n      - name: Build the runtime binaries and test archive\n        run: ./scripts/rust-archive.sh x.tar.zst\n  rust-tests:\n    steps:\n      - name: Full Rust workspace shard\n        run: ./scripts/test-leak-check.py -- ./scripts/tmux-leak-check.py -- cargo nextest run --profile ci --no-fail-fast --archive-file x.tar.zst --partition hash:${{ matrix.shard }}/4\n" + "  ios-ui-plan:\n    steps: []\n")
            write(".github/actions/ios-ui-build/action.yml", "runs:\n  steps: []\n")
            git("add", ".")
            git("commit", "-qm", "base")
            base = git("rev-parse", "HEAD")
            write("docs/spike.md", "findings\n")
            write("docs/second.md", "second finding\n")
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
            self.assertIn("3 changed file(s) against " + base, out.stdout)
            self.assertIn("jobs: wire, doc-comments", out.stdout)
            self.assertIn("no builds selected", out.stdout)
            self.assertIn("landing-gate: PASS", out.stdout)
            engine = {**env, "FARCOOLER_OPERATION_BINDING": "fixture-engine"}
            for bad_env, flags, message in (
                ({k: v for k, v in engine.items() if k != "FARCOOLER_BASE"}, [], "engine requires FARCOOLER_BASE"),
                (engine, ["--base", "HEAD"], "differs from engine FARCOOLER_BASE"),
            ):
                refused = subprocess.run(["bash", str(repo / "scripts/landing-gate"), "--worktree", tmp,
                                          "--dry-run", *flags], env=bad_env, capture_output=True, text=True)
                self.assertNotEqual(refused.returncode, 0)
                self.assertIn(message, refused.stderr)
            # A divergent sibling has a merge base, but must never silently widen the diff to it.
            git("checkout", "-qb", "sibling", base)
            write("docs/sibling.md", "sibling\n")
            git("add", "docs/sibling.md")
            git("commit", "-qm", "sibling")
            sibling = git("rev-parse", "HEAD")
            git("checkout", "-q", "-")
            refused = subprocess.run(["bash", str(repo / "scripts/landing-gate"), "--worktree", tmp, "--dry-run"],
                                      env={**engine, "FARCOOLER_BASE": sibling}, capture_output=True, text=True)
            self.assertNotEqual(refused.returncode, 0)
            self.assertIn("not an ancestor of candidate HEAD", refused.stderr)
            write("apps/android/a.kt", "code\n")
            git("add", ".")
            git("commit", "-qm", "third ticket")
            write("apps/macos/a.swift", "code\n")  # dirty fourth ticket also counts
            self.assertEqual(set(lane.changed_files(repo, base)), {
                "docs/spike.md", "docs/second.md", ".claude/agent/reports/ov-1189/result.md",
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
