#!/usr/bin/env python3
"""Engine command contract over real clean repositories and subprocesses."""
import importlib.util
import json
import os
import pathlib
import shutil
import subprocess
import sys
import unittest

sys.dont_write_bytecode = True
ROOT = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("fixtures", ROOT / "gate-evidence-test.py")
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


class EngineGateTest(unittest.TestCase):
    def setUp(self):
        self.f = fixtures.EvidenceTest()
        self.f.setUp()
        self.addCleanup(self.f.doCleanups)
        shutil.copyfile(ROOT / "engine-gate.py", self.f.scripts / "engine-gate.py")
        self.f.git("add", ".")
        self.f.git("commit", "-qm", "engine adapter")
        self.f.env.update(FARCOOLER_BASE=self.f.base,
                          FARCOOLER_CANDIDATE=self.f.git("rev-parse", "HEAD").strip(),
                          FARCOOLER_DEFINITION_PIN="definition", FARCOOLER_POLICY_PIN="policy",
                          FARCOOLER_WORKSPACE="workspace", FARCOOLER_OPERATION_BINDING="invocation-1")
        self.root = self.f.root / "private-evidence"

    def invoke(self, code=0, reuse=True, trust="fixture-reviewed-epoch", env=None):
        p = subprocess.run([sys.executable, str(self.f.scripts / "engine-gate.py"),
                            "--inputs", str(self.f.inputs), "--evidence-root", str(self.root),
                            "--trust-scope", trust, *(["--enable-reuse"] if reuse else [])],
                           cwd=self.f.repo, env=env or self.f.env, text=True, capture_output=True)
        self.assertEqual(p.returncode, code, p.stdout + p.stderr)
        return p

    def metrics(self, **kwargs):
        p = self.invoke(**kwargs)
        self.assertTrue(p.stdout.splitlines(), p.stderr)
        return json.loads(p.stdout.splitlines()[-1])

    def green(self):
        self.assertEqual(self.metrics(code=1)["reused_jobs"], 0)
        self.assertEqual(self.metrics()["reused_jobs"], 1)

    def test_failure_retry_full_hit_and_default_disabled(self):
        self.green()
        before = {p: p.read_bytes() for p in self.root.rglob("*") if p.is_file()}
        self.f.env["FARCOOLER_OPERATION_BINDING"] = "invocation-3"
        hit = self.metrics()
        self.assertEqual((hit["reused_jobs"], hit["selected_jobs"], hit["binding"]), (2, 0, "invocation-3"))
        self.assertEqual((self.f.repo / ".fixture/wire-count").read_text(), "wire\n")
        self.assertTrue(all(p.read_bytes() == raw for p, raw in before.items()))
        self.assertEqual(self.metrics(reuse=False)["reused_jobs"], 0)
        self.assertEqual((self.f.repo / ".fixture/wire-count").read_text(), "wire\nwire\n")

    def test_workflow_policy_board_trust_and_environment_mismatches_rerun(self):
        self.green()
        for key in ("FARCOOLER_DEFINITION_PIN", "FARCOOLER_POLICY_PIN", "FARCOOLER_WORKSPACE", "LANG"):
            self.f.env[key] = self.f.env.get(key, "") + "changed"
            self.assertEqual(self.metrics()["reused_jobs"], 0, key)
        self.assertEqual(self.metrics(trust="new-config-epoch")["reused_jobs"], 0)

    def test_candidate_dirty_untracked_and_missing_context_refused(self):
        self.green()
        self.f.env["FARCOOLER_CANDIDATE"] = "a" * 40
        self.assertIn("candidate", self.invoke(code=1).stderr)
        self.f.env["FARCOOLER_CANDIDATE"] = self.f.git("rev-parse", "HEAD").strip()
        readme = self.f.repo / "README.md"
        original = readme.read_bytes()
        readme.write_text("dirty fix")
        self.invoke(code=1)
        readme.write_bytes(original)
        extra = self.f.repo / "untracked"
        extra.write_text("untracked input")
        self.invoke(code=1)
        extra.unlink()
        env = dict(self.f.env)
        del env["FARCOOLER_POLICY_PIN"]
        self.invoke(code=1, env=env)

    def test_legacy_partial_missing_and_tampered_evidence_never_hits(self):
        self.f.pointer.write_text("lane-gate-summary v1\nverdict: ok\n")
        self.assertEqual(self.metrics(code=1)["reused_jobs"], 0)
        self.assertEqual(self.metrics()["reused_jobs"], 1)
        parent = fixtures.evidence.pointer_parent(self.f.pointer)
        log = pathlib.Path(parent["directory"]) / "doc-comments-0.log"
        log.chmod(0o600)
        log.write_text("forged output")
        self.assertEqual(self.metrics()["reused_jobs"], 0)
        self.assertEqual(log.read_text(), "forged output")
        self.assertTrue(list(self.root.glob("history-*.pointer.txt")))
        current = fixtures.evidence.pointer_parent(self.f.pointer)
        (pathlib.Path(current["directory"]) / "wire-0.log").unlink()
        self.assertEqual(self.metrics()["reused_jobs"], 0)

    def test_adapter_must_run_from_candidate_checkout(self):
        external = self.f.root / "external-scripts"
        shutil.copytree(self.f.scripts, external)
        shutil.copytree(self.f.repo / ".github", self.f.root / ".github")
        adapter = external / "engine-gate.py"
        adapter.write_text(adapter.read_text() + "\n# not the candidate's adapter\n")
        p = subprocess.run([sys.executable, str(adapter), "--inputs", str(self.f.inputs),
                            "--evidence-root", str(self.root), "--trust-scope", "fixture"],
                           cwd=self.f.repo, env=self.f.env, text=True, capture_output=True)
        self.assertEqual(p.returncode, 1, p.stdout + p.stderr)
        self.assertIn("adapter must run from candidate checkout", p.stderr)
        self.assertFalse(self.f.pointer.exists())
        self.assertFalse((self.f.repo / ".fixture/wire-count").exists())

    def test_candidate_is_checked_again_under_capture_lock(self):
        adapter = self.f.scripts / "engine-gate.py"
        # Move HEAD after the initial candidate check but before locked preflight.
        # The instrumented adapter itself is committed before invocation.
        source = adapter.read_text().replace(
            "with evidence.capture_lock(lane, repo):",
            'with evidence.capture_lock(lane, repo):\n'
            '        (repo / "README.md").write_text("concurrent candidate\\n")\n'
            '        subprocess.run(["git", "commit", "-qam", "concurrent candidate"], check=True)')
        adapter.write_text(source)
        self.f.git("add", ".")
        self.f.git("commit", "-qm", "instrument candidate race")
        self.f.env["FARCOOLER_CANDIDATE"] = self.f.git("rev-parse", "HEAD").strip()
        p = self.invoke(code=1)
        self.assertIn("engine candidate differs from HEAD", p.stderr)
        self.assertFalse(self.f.pointer.exists())
        self.assertFalse((self.f.repo / ".fixture/wire-count").exists())

    def test_capture_cannot_resample_a_different_candidate(self):
        adapter = self.f.scripts / "engine-gate.py"
        source = adapter.read_text().replace(
            'code = evidence._run_capture(',
            '(repo / "README.md").write_text("candidate changed before capture\\n")\n'
            '            subprocess.run(["git", "commit", "-qam", "moved before capture"], check=True)\n'
            '            code = evidence._run_capture(')
        adapter.write_text(source)
        self.f.git("add", ".")
        self.f.git("commit", "-qm", "instrument capture race")
        self.f.env["FARCOOLER_CANDIDATE"] = self.f.git("rev-parse", "HEAD").strip()
        p = self.invoke(code=1)
        self.assertIn("inputs changed before capture", p.stderr)
        self.assertFalse(self.f.pointer.exists())
        self.assertFalse((self.f.repo / ".fixture/wire-count").exists())

    def test_clean_operational_pair_reports_observed_wall_saving(self):
        # A clean committed source with real tiny CI commands, not mocked
        # receipt producers. This measures the adapter, not application builds.
        state = self.f.repo / ".fixture"
        state.mkdir()
        (state / "attempt").touch()
        cold = self.metrics()
        hot = self.metrics()
        self.assertEqual((cold["reused_jobs"], hot["reused_jobs"]), (0, 2))
        self.f.checker(0)
        print(json.dumps({"operational_fixture": "clean-real-shell-pair",
                          "cold_seconds": cold["wall_seconds"], "hit_seconds": hot["wall_seconds"],
                          "observed_wall_saved_seconds": cold["wall_seconds"] - hot["wall_seconds"],
                          "repeat_job_hit_rate": hot["reused_jobs"] / hot["required_jobs"],
                          "pair_job_hit_rate": 0.5}, sort_keys=True), flush=True)


if __name__ == "__main__":
    unittest.main()
