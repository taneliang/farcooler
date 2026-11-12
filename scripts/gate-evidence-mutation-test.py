#!/usr/bin/env python3
"""Prove key v2 counterexamples go red when guards are removed, in temp copies."""
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
MUTATIONS = [
    ("incomplete coverage", "len(receipts) == len(plan)", "0 < len(receipts) <= len(plan)",
     "test_semantic_counterexamples_even_when_rehashed"),
    ("nonzero exit promoted", 'r["exit"] == 0', 'r["exit"] in (0, 7)',
     "test_real_failed_job_retry_and_private_immutable_chain"),
    ("prerequisite bytes unbound", '"sha256": file_digest(path)', '"sha256": "unbound"',
     "test_stale_symbolic_base_and_prerequisite_bytes"),
    ("tracked bytes trusted from status", 'all(blob == index[name][1] for name, blob in zip(ordered, hashes))', 'True',
     "test_stat_cache_cannot_hide_same_size_dirty_fix"),
]


def main():
    for label, before, after, test in MUTATIONS:
        with tempfile.TemporaryDirectory(prefix="ov903-mutant-") as temporary:
            root = pathlib.Path(temporary)
            scripts = root / "scripts"
            scripts.mkdir()
            for name in ("gate_evidence.py", "gate-evidence-test.py", "lane-gate.py", "heavy_lock.py",
                         "ci-changes.py", "ios-ui-plan.py", "gate-summary-check.py", "android-ndk.sh"):
                shutil.copyfile(ROOT / "scripts" / name, scripts / name)
            workflow = root / ".github/workflows/ci.yml"
            workflow.parent.mkdir(parents=True)
            shutil.copyfile(ROOT / ".github/workflows/ci.yml", workflow)
            source = scripts / "gate_evidence.py"
            text = source.read_text()
            if text.count(before) != 1:
                raise RuntimeError(f"mutation anchor changed: {label}")
            source.write_text(text.replace(before, after))
            process = subprocess.run([sys.executable, str(scripts / "gate-evidence-test.py"), f"EvidenceTest.{test}"],
                                     capture_output=True, text=True,
                                     env={**os.environ, "PYTHONDONTWRITEBYTECODE": "1"})
            # A test assertion must fail, not an import/syntax/setup error.
            if process.returncode != 1 or "FAIL:" not in process.stderr or "AssertionError" not in process.stderr:
                print(process.stdout + process.stderr)
                raise RuntimeError(f"mutant was not caught by a meaningful assertion: {label}")
            print(f"mutation RED observed: {label}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
