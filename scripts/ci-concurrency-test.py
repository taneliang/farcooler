#!/usr/bin/env python3
"""Guard the CI workflow's main and branch concurrency policy."""
import pathlib
import re
import unittest

WORKFLOW = pathlib.Path(__file__).resolve().parent.parent / '.github/workflows/ci.yml'


class ConcurrencyTest(unittest.TestCase):
    def test_overlapping_runs(self):
        workflow = WORKFLOW.read_text()
        block = re.search(r'(?ms)^concurrency:\n((?:  [^\n]*\n|\n)+)', workflow)
        self.assertIsNotNone(block)
        # Pin the expressions whose behavior is modeled below. A change to either
        # policy must update this test and reconsider GitHub's pending-run limit.
        self.assertRegex(block.group(1), re.escape("group: ci-${{ github.ref }}-${{ github.ref == 'refs/heads/main' && github.run_id || 'shared' }}"))
        self.assertRegex(block.group(1), re.escape("cancel-in-progress: ${{ github.ref != 'refs/heads/main' }}"))

        def group(ref, run_id):
            return f"ci-{ref}-{run_id if ref == 'refs/heads/main' else 'shared'}"

        # GitHub keeps at most one pending run per group, even when running
        # jobs are not cancelled. Three main pushes must all have distinct groups.
        self.assertEqual(len({group('refs/heads/main', run) for run in (1, 2, 3)}), 3)
        for ref in ('refs/heads/feature', 'refs/pull/42/merge'):
            self.assertEqual(group(ref, 1), group(ref, 2))
            self.assertNotEqual(ref, 'refs/heads/main')  # cancel-in-progress is true
        self.assertNotEqual(group('refs/heads/feature', 1), group('refs/pull/42/merge', 1))


class CapacityAndYieldTests(unittest.TestCase):
    def test_capacity_and_yield(self):
        import subprocess
        import sys
        for script in ("ci-macos-capacity.py", "ci-yield-to-main.py"):
            with self.subTest(script=script):
                subprocess.run([sys.executable, str(pathlib.Path(__file__).resolve().parent / script), "--self-test"], check=True)


if __name__ == '__main__':
    unittest.main()
