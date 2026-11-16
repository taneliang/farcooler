#!/usr/bin/env python3
"""Offline planner tests, including real Cargo metadata and planted consumers."""
import importlib.util
import json
import os
import re
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

class DocsAndWorkflowTests(unittest.TestCase):
    """ov-1293: docs select nothing, ci.yml is judged by what changed, every reason is printed."""
    @classmethod
    def setUpClass(cls):
        cls.metadata = json.loads(subprocess.run(
            ["cargo", "metadata", "--offline", "--locked", "--format-version", "1", "--all-features", "--no-deps"],
            cwd=shadow.ROOT, capture_output=True, text=True, check=True).stdout)

    def choose(self, *changes, texts=None):
        return shadow.select(shadow.ROOT, self.metadata, list(changes), texts)

    def test_docs_select_nothing(self):
        for path in ["reports/ov-1/README.md", "docs/x/y.md", "README.md", "crates/cli/README.md", "reports/ov-1/shot.png"]:
            with self.subTest(path=path):
                names, reasons, full = self.choose(("A", path))
                self.assertEqual((names, full), ([], False))
                self.assertIn(path, reasons[0])

    def test_included_markdown_still_selects_its_reader(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp).resolve()
            (repo / "leaf/src").mkdir(parents=True)
            (repo / "leaf/Cargo.toml").write_text('[package]\nname="leaf"\nversion="0.1.0"\nedition="2021"\n')
            (repo / "leaf/src/lib.rs").write_text('const D: &str = include_str!("../DOC.md");')
            (repo / "Cargo.toml").write_text('[workspace]\nmembers=["leaf"]\n')
            meta = json.loads(subprocess.run(["cargo", "metadata", "--offline", "--format-version", "1", "--no-deps"],
                                             cwd=repo, capture_output=True, text=True, check=True).stdout)
            self.assertEqual(shadow.select(repo, meta, [("M", "leaf/DOC.md")])[0], ["leaf"])

    def test_everything_still_selects_everything(self):
        for path in ["Cargo.lock", "Cargo.toml", "crates/cli/build.rs", "rust-toolchain.toml", ".config/nextest.toml",
                     "test/fixtures/a.json", "crates/cli/fixtures/a.json", "unmapped/thing.bin", "crates/protocol/src/lib.rs"]:
            with self.subTest(path=path):
                names, reasons, full = self.choose(("M", path), ("A", "reports/ov-1/README.md"))
                self.assertTrue(full)
                self.assertEqual(len(names), len(self.metadata["workspace_members"]))
                self.assertEqual(reasons, [f"full: M {path}"])

    def test_every_reason_is_listed(self):
        _, reasons, full = self.choose(("M", "Cargo.lock"), ("M", "rust-toolchain.toml"), ("D", "crates/cli/src/lib.rs"),
                                       ("A", "reports/ov-1/README.md"), ("M", "Cargo.lock"))
        self.assertTrue(full)
        self.assertEqual(reasons, ["full: M Cargo.lock", "full: M rust-toolchain.toml", "full: D crates/cli/src/lib.rs"])

    WORKFLOW = "name: CI\non:\n  push:\nenv:\n  A: 1\njobs:\n  rust:\n    steps:\n      - run: cargo test\n  swift:\n    steps:\n      - run: swift test\n  android:\n    steps:\n      - run: gradle\n"

    def workflow(self, old, new):
        return self.choose(("M", shadow.CI_WORKFLOW), texts=lambda path: (old, new))

    def test_workflow_by_what_changed(self):
        base = self.WORKFLOW
        self.assertEqual(self.workflow(base, base.replace("swift test", "swift test --parallel"))[::2], ([], False))
        self.assertEqual(self.workflow(base, base.replace("gradle", "gradle build"))[::2], ([], False))
        for old, new in [("cargo test", "cargo nextest run"), ("A: 1", "A: 2"), ("push:", "push:\n    branches: [main]")]:
            with self.subTest(change=new):
                names, reasons, full = self.workflow(base, base.replace(old, new))
                self.assertTrue(full)
                self.assertIn(shadow.CI_WORKFLOW, reasons[0])
        added = base + "  newjob:\n    steps:\n      - run: echo\n"
        self.assertTrue(self.workflow(base, added)[2])
        removed_rust = base.replace("  rust:\n    steps:\n      - run: cargo test\n", "")
        self.assertTrue(self.workflow(base, removed_rust)[2])
        self.assertTrue(self.choose(("M", shadow.CI_WORKFLOW))[2])  # no way to read the diff: everything

    def test_last_ten_landings(self):
        """Replay the changed-file lists of the 10 newest landings on main when ov-1293 was written (no git read)."""
        landings = json.loads(Path(shadow.ROOT, "scripts/rust-select-landings.json").read_text())
        self.assertEqual(len(landings), 10)
        rows = []
        for landing in landings:
            changes = [tuple(c) for c in landing["changes"]]
            def texts(path, jobs=landing["ci_jobs_touched"]):
                old = "name: CI\non:\njobs:\n" + "".join(f"  {j}:\n    a\n" for j in jobs if j) 
                new = old.replace("    a\n", "    b\n")
                if "" in jobs:
                    old, new = "env: 1\n" + old, "env: 2\n" + new
                return old, new
            before_full = any(st not in ("A", "M") or not re.fullmatch(r"crates/(?!protocol/)[^/]+/(?:src|tests|examples)/.+\.rs", p)
                              for st, p in changes)
            names, reasons, full = shadow.select(shadow.ROOT, self.metadata, changes, texts)
            rows.append((landing["commit"], "full" if before_full else "narrow", "full" if full else f"{len(names)} crates", reasons))
            if all(p.startswith(("reports/", "docs/")) or p.endswith(".md") for _, p in changes):
                self.assertEqual((names, full), ([], False), landing["subject"])
        for row in rows:
            print("replay", *row[:3], "|", "; ".join(row[3])[:110])
        self.assertTrue(any(r[1] == "full" and r[2] != "full" for r in rows), "at least one landing narrows that was full before")


if __name__ == "__main__":
    unittest.main()
