#!/usr/bin/env python3
"""Offline planner tests, including real Cargo metadata and planted consumers."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("rust_shadow", Path(__file__).with_name("rust-shadow.py"))
shadow = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shadow)


class ShadowTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.repo = Path(self.tmp.name).resolve()
        self.names = ["leaf", "dev", "build", "target", "optional", "transitive", "farcooler-cli", "farcooler-daemon", "unrelated"]
        self.write("Cargo.toml", '[workspace]\nresolver="2"\nmembers=' + json.dumps(self.names))
        for name in self.names:
            dep = {
                "dev": '[dev-dependencies]\nleaf={path="../leaf"}',
                "build": '[build-dependencies]\nleaf={path="../leaf"}',
                "target": '[target.\'cfg(target_os="windows")\'.dependencies]\nleaf={path="../leaf"}',
                "optional": '[dependencies]\nleaf={path="../leaf",optional=true}',
                "transitive": '[dependencies]\ndev={path="../dev"}',
            }.get(name, "")
            self.write(f"{name}/Cargo.toml", f'[package]\nname="{name}"\nversion="0.1.0"\nedition="2021"\n{dep}\n')
            self.write(f"{name}/src/lib.rs", "// good\n")
        self.metadata = json.loads(self.cargo("metadata", "--offline", "--all-features", "--format-version", "1").stdout)

    def write(self, path, text):
        p = self.repo / path
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text)

    def cargo(self, *args):
        return subprocess.run(["cargo", *args], cwd=self.repo, capture_output=True, text=True)

    def choose(self, *changes):
        return shadow.select(self.repo, self.metadata, changes)

    def test_reverse_all_dependency_kinds_and_transitive(self):
        names, _, full = self.choose(("M", "leaf/src/lib.rs"))
        self.assertEqual(set(names), {"leaf", "dev", "build", "target", "optional", "transitive"})
        self.assertFalse(full)

    def test_cli_daemon_and_mac_consumer(self):
        names, _, _ = self.choose(("M", "farcooler-cli/src/lib.rs"))
        self.assertEqual(names, ["farcooler-cli", "farcooler-daemon"])
        # Plant a contract break in the CLI. Its own suite stays green; the
        # out-of-graph daemon and Mac-style consumer see the broken input.
        self.write("farcooler-cli/src/lib.rs", "// broken CLI contract\n")
        self.write("farcooler-daemon/src/lib.rs", '''#[test] fn cli_contract() {
            assert!(include_str!("../../farcooler-cli/src/lib.rs").contains("// good"));
        }''')
        selected_rc = self.cargo("test", "--offline", "-p", "farcooler-cli").returncode
        full_rc = self.cargo("test", "--offline", "--workspace").returncode
        self.assertEqual(selected_rc, 0)
        self.assertNotEqual(full_rc, 0)
        self.assertFalse(shadow.accepted(full_rc, selected_rc, True))
        mac = subprocess.run(["sh", "-c", "grep -q '// good' farcooler-cli/src/lib.rs"], cwd=self.repo)
        self.assertNotEqual(mac.returncode, 0)
        # The shipping workflow must retain its Mac suite and doctests.
        workflow = (shadow.ROOT / ".github/workflows/ci.yml").read_text()
        self.assertIn("cargo test --workspace --doc", workflow)
        self.assertIn("apps/macos/test.sh", workflow)

    def test_cross_crate_include(self):
        self.write("unrelated/src/lib.rs", 'const INPUT: &str = include_str!("../../leaf/src/lib.rs");')
        self.assertIn("unrelated", self.choose(("M", "leaf/src/lib.rs"))[0])

    def test_global_unknown_deleted_renamed_inputs(self):
        for status, path in [("D", "leaf/src/lib.rs"), ("R100", "leaf/src/lib.rs"),
                             ("M", "leaf/build.rs"), ("M", "leaf/Cargo.toml"),
                             ("M", "leaf/fixtures/input.json"), ("M", "proto/farcooler.proto"),
                             ("M", "test/fixtures/input.json"), ("M", "rust-toolchain.toml"),
                             ("M", ".cargo/config.toml"), ("M", "unknown.rs"),
                             ("M", "Cargo.lock"), ("M", "services/relay/test/fixtures/input.json")]:
            with self.subTest(path=path, status=status):
                self.assertTrue(self.choose((status, path))[2])
        self.assertTrue(self.choose()[2])

    def test_verified_base_not_previous_commit(self):
        def git(*args):
            return shadow.capture(self.repo, "git", *args).strip()
        git("init", "-q")
        git("config", "user.email", "test@example.com")
        git("config", "user.name", "Test")
        commits = []
        for i in range(3):
            self.write("leaf/src/lib.rs", f"// {i}\n")
            git("add", ".")
            git("-c", "commit.gpgsign=false", "commit", "-qm", str(i))
            commits.append(git("rev-parse", "HEAD"))
        self.assertEqual(shadow.green_base(self.repo, commits[2], [commits[0]]), commits[0])
        self.assertIsNone(shadow.green_base(self.repo, commits[2], [commits[2], "1" * 40]))

    def test_failure_and_inventory_evidence(self):
        self.assertTrue(shadow.accepted(0, 0, True))
        for full, selected, inventory in [(1, 0, True), (0, 1, True), (0, 0, False)]:
            self.assertFalse(shadow.accepted(full, selected, inventory))

    def test_runner_always_runs_full_and_publishes_rejection(self):
        # Exercise the actual subprocess runner, not just the decision helper.
        # This tiny nextest stand-in plants a failure only outside the filter.
        self.write("bin/cargo", '''#!/bin/sh
case "$2" in
  list) echo '{"rust-test-suites": {}}'; exit 0;;
  run) echo "$*" >> "$SHADOW_COMMAND_LOG"; exit "$SHADOW_FULL_EXIT";;
esac
exit 2
''')
        (self.repo / "bin/cargo").chmod(0o755)
        import argparse
        args = argparse.Namespace(out=str(self.repo / "report"), command=[
            "cargo", "nextest", "run", "--profile", "ci", "--workspace", "--no-fail-fast"])
        proposal = {"full": False, "packages": ["farcooler-cli"], "reasons": ["test"], "skips_enabled": False}
        candidates = self.repo / "green.ndjson"
        candidates.write_text('{"head_sha":"green"}\n')
        env = {"PATH": str(self.repo / "bin") + os.pathsep + os.environ["PATH"],
               "CANDIDATES": str(candidates), "SHADOW_FULL_EXIT": "1",
               "SHADOW_COMMAND_LOG": str(self.repo / "commands")}
        for full_exit in (1, 0):
            env["SHADOW_FULL_EXIT"] = str(full_exit)
            (self.repo / "commands").unlink(missing_ok=True)
            with patch.dict(os.environ, env), patch.object(Path, "cwd", return_value=self.repo), patch.object(shadow, "plan", return_value=proposal):
                self.assertEqual(shadow.run(args), full_exit)
            commands = (self.repo / "commands").read_text().splitlines()
            self.assertEqual(commands[0], "nextest run --profile ci --workspace --no-fail-fast")
            self.assertEqual(len(commands), 1 if full_exit else 2)
            if not full_exit:
                self.assertTrue(commands[1].endswith("-E package(=farcooler-cli)"))
            report = json.loads((self.repo / "report/measurement.json").read_text())
            self.assertEqual(report["selection_accepted"], not full_exit)
            self.assertFalse(report["skips_enabled"])
            if full_exit:
                self.assertIsNone(report["warm_wall_savings_seconds"])
        for name in ("full", "selected"):
            self.assertTrue((self.repo / f"report/{name}-inventory.json").exists())


if __name__ == "__main__":
    unittest.main()
