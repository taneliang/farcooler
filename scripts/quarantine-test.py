#!/usr/bin/env python3
"""Self-test for scripts/quarantine.py (ov-1229): the lint, and real nextest runs of a scratch crate."""
import contextlib
import datetime as dt
import importlib.util
import io
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("quarantine", HERE / "quarantine.py")
q = importlib.util.module_from_spec(spec)
spec.loader.exec_module(q)

TODAY = dt.date(2026, 11, 15)
CRATE = """
#[cfg(test)]
mod tests {
    // Fails on its first run in a fresh state directory, passes after.
    #[test]
    fn fails_once() {
        let marker = std::path::Path::new(&std::env::var("QUARANTINE_STATE").unwrap()).join("once");
        if !marker.exists() {
            std::fs::write(&marker, "x").unwrap();
            panic!("first run");
        }
    }
    #[test]
    fn fails_always() { panic!("always"); }
    #[test]
    fn fails_unlisted() { panic!("unlisted"); }
}
"""


class Lint(unittest.TestCase):
    def check(self, text, today=TODAY):
        root = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, root)
        crate = root / "crates/x"
        (crate / "src/server").mkdir(parents=True)
        (crate / "tests").mkdir()
        (crate / "Cargo.toml").write_text('[package]\nname = "farcooler-x"\nversion = "0.0.0"\n')
        (crate / "src/lib.rs").write_text(
            '// #[test] fn in_a_comment() {}\nfn plain() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn real() {}\n'
            '    fn helper() {}\n    mod deep {\n        #[tokio::test]\n        async fn nested() {}\n    }\n}\n')
        (crate / "src/lib.rs").write_text((crate / "src/lib.rs").read_text() + "mod server;\n")
        (crate / "src/server/mod.rs").write_text("#[test]\nfn in_a_file_module() {}\n")
        (crate / "src/main.rs").write_text("mod cli;\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn in_main() {}\n}\n")
        (crate / "src/cli.rs").write_text("#[test]\nfn in_cli() {}\n")
        (crate / "src/orphan.rs").write_text("#[test]\nfn never_compiled() {}\n")
        (crate / "tests/a_bin.rs").write_text("#[test]\nfn integration() {}\n")
        entries, problems = q.parse(text, today)
        return problems + q.unknown(entries, root)

    def test_good_and_empty(self):
        self.assertEqual(self.check(""), [])
        self.assertEqual(self.check(
            "# c\n\nfarcooler-x tests::real ov-1 2026-11-16\nfarcooler-x tests::deep::nested ov-2 2026-12-01\n"
            "farcooler-x server::in_a_file_module ov-3 2026-12-01\nfarcooler-x::a_bin integration ov-4 2026-12-01\n"), [])

    def test_expired_malformed_fail(self):
        for text, word in [
            ("farcooler-x tests::real ov-1 2026-11-15\n", "expired"),
            ("farcooler-x tests::real ov-1 2026-01-01\n", "expired"),
            ("farcooler-x tests::real ov-1\n", "field"),
            ("farcooler-x tests::real ticket 2026-12-01\n", "ticket"),
            ("farcooler-x tests::real ov-1 soon\n", "date"),
            ("farcooler-x tests::real ov-1 2026-13-45\n", "date"),
            ("farcooler-x tests::real ov-1 2026-12-01\nfarcooler-x tests::real ov-1 2026-12-01\n", "twice"),
        ]:
            with self.subTest(text=text):
                self.assertTrue(any(word in p for p in self.check(text)), self.check(text))

    def test_a_name_matching_no_test_fails(self):
        for text in [
            "farcooler-x tests::gone ov-1 2026-12-01\n",                    # renamed or typo
            "invented tests::real ov-1 2099-01-01\n",                       # nonexistent package
            "invented::module main ov-1 2099-01-01\n",                      # nonexistent package and binary
            "farcooler-x invented::module::main ov-1 2099-01-01\n",         # nonexistent module
            "farcooler-x real ov-1 2026-12-01\n",                           # wrong module path
            "farcooler-x tests::helper ov-1 2026-12-01\n",                  # a fn, but not a test
            "farcooler-x tests::plain ov-1 2026-12-01\n",
            "farcooler-x in_a_comment ov-1 2026-12-01\n",                   # a #[test] inside a comment
            "farcooler-x::no_such_bin integration ov-1 2026-12-01\n",       # nonexistent test binary
            "farcooler-x::a_bin tests::real ov-1 2026-12-01\n",             # right test, wrong binary
            "farcooler-x integration ov-1 2026-12-01\n",                    # integration test under the lib binary
            "farcooler-x tests::in_main ov-1 2026-12-01\n",                 # a main.rs test under the library binary
            "farcooler-x::bin/farcooler-x tests::real ov-1 2026-12-01\n",   # a lib test under the binary
            "farcooler-x::bin/other tests::in_main ov-1 2026-12-01\n",      # wrong binary name
            "farcooler-x orphan::never_compiled ov-1 2026-12-01\n",         # a file no `mod` reaches
        ]:
            with self.subTest(text=text):
                self.assertTrue(any("no test" in p for p in self.check(text)), self.check(text))

    def test_the_checked_in_list_is_clean(self):
        # Whatever the list holds, it must be valid today. It may be empty or hold entries.
        self.assertEqual(q.lint(), [])

    def test_the_real_tree_is_read_like_nextest(self):
        real = q.real_tests(q.ROOT)
        self.assertIn(("farcooler-tmux::live_tmux", "resize_and_capture_target_the_exact_window"), real)
        self.assertTrue(any(b == "farcooler-tmux" and "::" in t for b, t in real))


@unittest.skipUnless(shutil.which("cargo-nextest") or (pathlib.Path.home() / ".cargo/bin/cargo-nextest").exists(), "needs cargo-nextest")
class Nextest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.dir = pathlib.Path(tempfile.mkdtemp())
        (cls.dir / "src").mkdir()
        (cls.dir / "Cargo.toml").write_text('[package]\nname = "scratch"\nversion = "0.0.0"\nedition = "2021"\n')
        (cls.dir / "src/lib.rs").write_text(CRATE)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.dir)

    def go(self, listed, only):
        state = pathlib.Path(tempfile.mkdtemp(dir=self.dir))
        os.environ["QUARANTINE_STATE"] = str(state)
        entries = [("scratch", t, "ov-1", "2026-12-01", 1) for t in listed]
        out = []
        cwd = os.getcwd()
        os.chdir(self.dir)
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                rc = q.run(["cargo", "nextest", "run", "--profile", "ci", "-E", only],
                           base="[profile.ci]\nfail-fast = false\n", entries=entries, say=out.append)
        finally:
            os.chdir(cwd)
        return rc, out

    def test_listed_flake_passes_and_is_reported(self):
        rc, out = self.go(["tests::fails_once"], "test(fails_once)")
        self.assertEqual(rc, 0)
        self.assertEqual(len(out), 1)
        self.assertIn("flaky (quarantined): scratch tests::fails_once", out[0])

    def test_listed_test_failing_every_rerun_still_fails(self):
        rc, out = self.go(["tests::fails_always"], "test(fails_always)")
        self.assertNotEqual(rc, 0)
        self.assertEqual(out, [])

    def test_unlisted_failure_gets_no_rerun_and_fails(self):
        # fails_once is not listed here, so its first failure is final.
        rc, out = self.go(["tests::fails_always"], "test(fails_once)")
        self.assertNotEqual(rc, 0)
        self.assertEqual(out, [])

    def test_listed_flake_beside_a_real_failure_still_fails(self):
        rc, out = self.go(["tests::fails_once"], "test(fails_once) or test(fails_unlisted)")
        self.assertNotEqual(rc, 0)
        self.assertEqual(len(out), 1)


if __name__ == "__main__":
    os.environ["PATH"] = str(pathlib.Path.home() / ".cargo/bin") + os.pathsep + os.environ["PATH"]
    unittest.main()
