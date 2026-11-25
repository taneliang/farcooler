#!/usr/bin/env python3
"""Regression: gate self-tests do not depend on the host's disk capacity."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent


class FixtureTests(unittest.TestCase):
    def test_self_test_with_one_gib_free(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "sitecustomize.py").write_text(
                "import shutil\n"
                "shutil.disk_usage = lambda path: shutil._ntuple_diskusage(1024**3, 0, 1024**3)\n"
            )
            # Reproduce the engine scratch path, which is too long for Unix sockets.
            scratch = root / ("engine-scratch-" + "x" * 100)
            scratch.mkdir()
            env = {**os.environ, "PYTHONPATH": str(root), "TMPDIR": str(scratch),
                   "FARCOOLER_SCRATCH": str(scratch)}
            env.pop("LANE_GATE_TEST_FIXTURE", None)
            result = subprocess.run([sys.executable, str(ROOT / "scripts/lane-gate.py"), "--self-test"],
                                    cwd=ROOT, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("lane-gate self-test: ok", result.stdout)


if __name__ == "__main__":
    unittest.main()
