#!/usr/bin/env python3
"""Validate the captured spike evidence without running a product build or test."""
import datetime
import hashlib
import json
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import zipfile

root = pathlib.Path(__file__).resolve().parent
result = subprocess.run([sys.executable, str(root / 'analyze.py')], capture_output=True, text=True, check=True)
assert result.stdout == (root / 'tables.md').read_text()
print('PASS: tables regenerate byte-for-byte from captured receipts')
build = json.loads((root / 'build-results.json').read_text())
assert [run['exit'] for run in build['runs']] == [0, 0]
assert build['runs'][0]['seconds'] > 100 and build['runs'][1]['seconds'] < 1
assert all(max(row[1:]) < 20 for row in build['load_samples'])
assert max(build['runs'][1]['load_end']) < 20
units = []
for name in ['cold', 'warm']:
    text = zipfile.ZipFile(root / 'timings.zip').read(f'{name}-timing.html').decode()
    units.append(json.loads(re.search(r'const UNIT_DATA = (\[.*?\]);', text, re.S)[1]))
assert len(units[0]) > 400 and all(unit['duration'] == 0 for unit in units[1])
print('PASS: cold/warm builds exited 0; warm units all fresh/zero-time; sampled load below 20')
for name in ['fresh-gate-preflight.json', 'resume-preflight.json']:
    preflight = json.loads((root / name).read_text())
    assert preflight['decision'] == 'not_started_load_limit', name
    assert preflight['limit'] == 20
    assert preflight['command'] == [
        'scripts/landing-gate', '--base',
        'd986f3083730db7cda3b91caf5e536ba321b7c1e', '--keep-target']
    samples = preflight['samples']
    assert len(samples) >= 6
    times = [datetime.datetime.fromisoformat(row['utc']) for row in samples]
    assert all(a < b for a, b in zip(times, times[1:]))
    assert (times[-1] - times[0]).total_seconds() >= 150
    assert all(len(row['load_1_5_15']) == 3
               and max(row['load_1_5_15']) >= preflight['limit'] for row in samples)
print('PASS: both full-gate preflights remained above the load limit; neither is a gate result')
for name, code in [('test-preflight-failure.json', 126), ('test-results.json', 254)]:
    attempt = json.loads((root / name).read_text())
    assert len(attempt['runs']) == 1 and attempt['runs'][0]['exit'] == code
print('PASS: failed/interrupted nextest attempts remain excluded from passing baselines')
for ref in json.loads((root / 'ci-runs.json').read_text()):
    run = json.loads((root / f"ci-{ref['databaseId']}.json").read_text())
    assert run['headSha'] == ref['headSha'] and str(ref['databaseId']) in run['url']
print('PASS: five CI run IDs and source SHAs match requested runs')
summary = json.loads((root / 'ci-test-times.json').read_text())
raw = (root / 'slow-tests-extract.log').read_text().splitlines()
for field in ['top20_observed', 'top20_rust']:
    for row in summary[field]:
        assert any(row['job'] + '\t' in line and row['test'] in line
                   and f"{row['seconds']:.3f}" in line for line in raw), row
print('PASS: top-20 observed and Rust rows match timestamped raw extracts')
sources = json.loads((root / 'evidence-sources.json').read_text())
assert hashlib.sha256((root / 'historical-gate.txt').read_bytes()).hexdigest() == sources['historical_gate_sha256']
print('PASS: historical gate copy matches original receipt hash')
followups = json.loads((root / 'followups.json').read_text())
assert [task['key'] for task in followups] == ['ov-1116', 'ov-1117', 'ov-1118', 'ov-1119']
assert all(task['status'] == 'backlog' and len(task['acceptance']) == 3 for task in followups)
print('PASS: four follow-up creation receipts have backlog status and acceptance')
with tempfile.TemporaryDirectory(prefix='ov1110-validation-', dir='/private/tmp') as temporary:
    dest = pathlib.Path(temporary)
    for path in root.iterdir():
        if path.is_file():
            shutil.copyfile(path, dest / path.name)
    for path in dest.glob('ci-*.json'):
        if re.fullmatch(r'ci-\d+\.json', path.name):
            data = json.loads(path.read_text())
            data['jobs'] = [job for job in data['jobs'] if job['name'] != 'Android']
            path.write_text(json.dumps(data))
    red = subprocess.run([sys.executable, str(dest / 'analyze.py')], capture_output=True, text=True)
    assert red.returncode != 0 and 'missing CI job family' in red.stderr
print('PASS: removing Android receipts makes analyzer fail, not report zero cost')
for path in root.glob('*.md'):
    for _, target in re.findall(r'\[([^\]]+)\]\(([^)]+)\)', path.read_text()):
        if '://' not in target:
            assert (path.parent / target.split('#')[0]).exists(), (path, target)
print('PASS: relative report links resolve')
