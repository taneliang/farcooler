#!/usr/bin/env python3
"""Verify the hosted experiment's CI parity and print its recorded job timings."""
from datetime import datetime
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
ci = (ROOT / '.github/workflows/ci.yml').read_text().split('\n  rust:\n', 1)[1].split('\n  # TWO JOBS, NOT ONE', 1)[0]
timing = (HERE / 'hosted-timing.yml').read_text().split('\n  rust:\n', 1)[1]
# Undo only the experiment's declared matrix, dependency and coverage switches.
timing = timing.replace('name: Rust (${{ matrix.os }}, ${{ matrix.coverage }})', 'name: Rust (${{ matrix.os }})')
timing = timing.replace('    runs-on:', "    needs: changes\n    if: ${{ !cancelled() && needs.changes.outputs.rust != 'false' }}\n    runs-on:", 1)
timing = timing.replace('os: [macos-latest]\n        coverage: [full, selected]', 'os: [ubuntu-24.04, macos-latest]')
timing = timing.replace("if: matrix.coverage == 'full'\n        env:", "if: runner.os == 'Linux'\n        env:", 1)
start = timing.index('      - name: macOS full Rust tests (before)\n')
end = timing.index('      - name: macOS native Rust tests\n')
full_step = timing[start:end]
# The baseline uses the exact original full-suite command, including shadow.
original = ci.split('      - name: Full Rust tests and shadow measurement\n', 1)[1].split('      - name: macOS native Rust tests\n', 1)[0]
assert full_step.split('        env:\n', 1)[1] == original.split('        env:\n', 1)[1]
timing = timing[:start] + timing[end:]
timing = timing.replace("if: runner.os == 'macOS' && matrix.coverage == 'selected'", "if: runner.os == 'macOS'")
timing = timing.replace("if: always() && matrix.coverage == 'full'", "if: always() && runner.os == 'Linux'")
assert timing == ci, 'Hosted job differs from ci.yml beyond the declared experiment switches'
print('PASS: hosted workflow matches the complete CI Rust job and original full-suite command')


def seconds(start, end):
    return (datetime.fromisoformat(end.replace('Z', '+00:00')) - datetime.fromisoformat(start.replace('Z', '+00:00'))).total_seconds()


jobs = json.loads((HERE / 'hosted-jobs.json').read_text())['jobs']
run = json.loads((HERE / 'hosted-run.json').read_text())
assert len(jobs) == 2
assert all(job['status'] == 'completed' and job['conclusion'] == 'success' for job in jobs)
assert all(job['head_sha'] == run['head_sha'] for job in jobs)
for job in sorted(jobs, key=lambda job: job['name']):
    print(f"{job['name']}: {job['conclusion']}, {seconds(job['started_at'], job['completed_at']):.0f} s, {job['html_url']}")
    for step in job['steps']:
        if step['conclusion'] != 'skipped':
            print(f"  {seconds(step['started_at'], step['completed_at']):6.0f} s  {step['conclusion']:7}  {step['name']}")
