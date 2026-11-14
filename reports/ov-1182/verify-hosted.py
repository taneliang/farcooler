#!/usr/bin/env python3
"""Check measured workflow parity and two successful sub-15-minute Rust runs."""
from datetime import datetime
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent


def measured_jobs(ci):
    start = ci.index('\n  rust:\n') + 1
    end = ci.index('  # TWO JOBS, NOT ONE', start)
    jobs = ci[start:end]
    jobs = jobs.replace('    needs: changes\n', '')
    jobs = jobs.replace('    needs: [changes, rust-build]\n', '    needs: rust-build\n')
    jobs = jobs.replace("    if: ${{ !cancelled() && needs.changes.outputs.rust != 'false' }}\n", '')
    return jobs.replace("    if: ${{ !cancelled() && needs.changes.outputs.rust != 'false' && needs.rust-build.result == 'success' }}\n", '')


def main():
    actual = (HERE / 'hosted-timing.yml').read_text().split('\njobs:\n', 1)[1]
    assert actual == measured_jobs((ROOT / '.github/workflows/ci.yml').read_text()), 'measured Rust jobs differ from production'
    print('Measured Rust jobs match production; only path-selection gates removed')
    if len(sys.argv) == 1:
        return
    assert len(sys.argv) == 3, 'provide job JSON for two distinct runs'
    expected = {'Rust (ubuntu-24.04)', 'Rust (macos-latest)', 'Rust test archive', 'Rust native tests (macOS)',
                *(f'Rust tests ({i}/4)' for i in range(1, 5))}
    seen_ids = set()
    for file in sys.argv[1:]:
        data = json.loads(Path(file).read_text())
        jobs = data['jobs']
        assert {j['name'] for j in jobs} == expected
        ids = {j['databaseId'] for j in jobs}
        assert not seen_ids.intersection(ids), 'same run supplied twice'
        seen_ids.update(ids)
        for job in jobs:
            assert job['conclusion'] == 'success', job['name']
            seconds = (datetime.fromisoformat(job['completedAt'].replace('Z', '+00:00'))
                       - datetime.fromisoformat(job['startedAt'].replace('Z', '+00:00'))).total_seconds()
            assert 0 < seconds < 900, (job['name'], seconds)
            print(f"{Path(file).name}: {job['name']}: {seconds:.0f}s")


if __name__ == '__main__':
    main()
