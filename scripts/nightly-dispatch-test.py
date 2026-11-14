#!/usr/bin/env python3
"""Dry-check Nightly's dispatch selections and macOS dependency chain."""
import re
from pathlib import Path

text = (Path(__file__).resolve().parents[1] / '.github/workflows/nightly.yml').read_text()
jobs = dict(re.findall(r'^  ([\w-]+):\n(.*?)(?=^  [\w-]+:\n|\Z)', text.split('\njobs:\n', 1)[1], re.M | re.S))
options = re.search(r'        options:\n((?:          - .+\n)+)', text).group(1)
options = re.findall(r'          - (.+)', options)
assert options[0] == 'all' and len(options) == len(set(options))
assert 'default: all' in text
assert 'needs: [ios, yield-ui]' in jobs['ios-ui']
assert 'needs: [ios, ios-ui, yield-captures]' in jobs['captures']
assert 'needs: [captures, yield-perf]' in jobs['mac-perf']
assert "!cancelled()" in jobs['ios-ui'] and "!cancelled()" in jobs['captures'] and "!cancelled()" in jobs['mac-perf']
matrices = {}
for job in ('ios-ui', 'captures'):
    body = jobs[job]
    assert 'max-parallel: 1' in body
    matrices[job] = re.search(r'^        shard: \[([^]]+)\]', body, re.M).group(1).split(', ')
    exclusions = re.findall(r"^          - shard: \$\{\{ inputs.shard != '' && inputs.shard != 'all' && inputs.shard != '([^']+)' && '([^']+)' \|\| '__none__' \}\}$", body, re.M)
    assert exclusions == [(s, s) for s in matrices[job]], (job, exclusions)
assert options[1:] == matrices['ios-ui'] + matrices['captures']
assert "inputs.shard == '' || inputs.shard == 'all' || !startsWith(inputs.shard, 'captures')" in jobs['ios-ui']
assert "inputs.shard == '' || inputs.shard == 'all' || startsWith(inputs.shard, 'captures')" in jobs['captures']
for job in ('budget', 'claude-rewake', 'daemon-production-timing', 'mac-perf', 'report'):
    assert "inputs.shard == '' || inputs.shard == 'all'" in jobs[job]
for choice in ['', *options]:
    full = choice in ('', 'all')
    selected = [s for shards in matrices.values() for s in shards if full or s == choice]
    assert selected == (options[1:] if full else [choice]), (choice, selected)
    print(f"{choice or 'schedule'}: iOS -> {', '.join(selected)}" + (' -> Mac perf' if full else '') + '; peak macOS runners: 1')
