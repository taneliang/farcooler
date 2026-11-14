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
    def check(self, text, today=TODAY, files=None):
        root = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, root)
        (root / "crates/x/tests").mkdir(parents=True)
        (root / "crates/x/src.rs").write_text("fn real_test() {}\n")
        (root / "crates/x/tests/a_bin.rs").write_text("fn other() {}\n")
        entries, problems = q.parse(text, today)
        return problems + q.unknown(entries, root)

    def test_good_and_empty(self):
        self.assertEqual(self.check(""), [])
        self.assertEqual(self.check("# c\n\nx::a_bin m::other ov-1 2026-12-01\nx m::real_test ov-2 2026-11-16\n"), [])

    def test_expired_malformed_unknown_fail(self):
        for text, word in [
            ("x m::real_test ov-1 2026-11-15\n", "expired"),
            ("x m::real_test ov-1 2026-01-01\n", "expired"),
            ("x m::real_test ov-1\n", "field"),
            ("x m::real_test ticket 2026-12-01\n", "ticket"),
            ("x m::real_test ov-1 soon\n", "date"),
            ("x m::real_test ov-1 2026-13-45\n", "date"),
            ("x m::gone ov-1 2026-12-01\n", "no test function"),
            ("x::no_such_bin m::other ov-1 2026-12-01\n", "no test binary"),
            ("x m::real_test ov-1 2026-12-01\nx m::real_test ov-1 2026-12-01\n", "twice"),
        ]:
            with self.subTest(text=text):
                self.assertTrue(any(word in p for p in self.check(text)), self.check(text))

    def test_the_checked_in_list_is_clean_and_empty(self):
        self.assertEqual(q.lint(), [])
        self.assertEqual(q.parse(q.LIST.read_text(), TODAY)[0], [])


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
