#!/usr/bin/env python3
"""Write the temporary branch-push workflow that times the production Rust jobs.

The jobs are copied from ci.yml unchanged except for the path-selection job and
the Swift waits, which a branch push without `changes` and the Apple jobs
cannot satisfy. Delete the output after measuring (as ov-1182 did)."""
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[2]
branch = sys.argv[1]
ci = (ROOT / '.github/workflows/ci.yml').read_text()
start = ci.index('\n  rust:\n') + 1
end = ci.index('  # TWO JOBS, NOT ONE', start)
jobs = ci[start:end]
for old, new in [
    ('    needs: changes\n', ''),
    ('    needs: [changes, rust-macos-build]\n', '    needs: rust-macos-build\n'),
    ('    needs: [changes, rust-build]\n', '    needs: rust-build\n'),
    ('    needs: [changes, rust-macos-build, swift, swift-tests]\n', '    needs: rust-macos-build\n'),
    ("    if: ${{ !cancelled() && needs.changes.outputs.rust != 'false' }}\n", ''),
    ("    if: ${{ !cancelled() && needs.changes.outputs.rust != 'false' && needs.rust-build.result == 'success' }}\n", ''),
    ("    if: ${{ !cancelled() && needs.changes.outputs.rust != 'false' && needs.rust-macos-build.result == 'success' }}\n", ''),
]:
    jobs = jobs.replace(old, new)
assert 'needs.changes' not in jobs and 'swift-tests' not in jobs.split('jobs:')[-1].split('rust-macos-tests:')[-1].split('steps:')[0], 'unresolved needs'
head = f"name: ov-1210 Rust timing\non:\n  push:\n    branches: [{branch}]\nenv:\n  CARGO_TERM_COLOR: always\njobs:\n"
out = ROOT / '.github/workflows/ov-1210-timing.yml'
out.write_text(head + jobs)
print(out)
