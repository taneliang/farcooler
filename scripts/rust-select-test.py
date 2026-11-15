#!/usr/bin/env python3
"""Offline planner tests, including real Cargo metadata and planted consumers."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from copy import deepcopy
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("rust_select", Path(__file__).with_name("rust-select.py"))
shadow = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shadow)


class SelectTests(unittest.TestCase):
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

    def test_binaries_split_by_selection(self):
        run, skipped = shadow.test_binaries(self.metadata, {"leaf", "dev"})
        self.assertEqual(run, ["dev", "leaf"])
        self.assertIn("unrelated", skipped)
        self.assertIn("farcooler-cli", skipped)
        self.assertEqual(sorted(run + skipped), sorted(set(run + skipped)))
        self.assertEqual(len(run + skipped), len(self.names))
        # Integration tests and binaries get Nextest-style ids.
        self.write("leaf/tests/api.rs", "")
        self.write("leaf/src/main.rs", "fn main() {}")
        meta = json.loads(self.cargo("metadata", "--offline", "--all-features", "--format-version", "1").stdout)
        run, _ = shadow.test_binaries(meta, {"leaf"})
        self.assertEqual(run, ["leaf", "leaf::api", "leaf::bin/leaf"])

if __name__ == "__main__":
    unittest.main()
