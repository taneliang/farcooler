#!/usr/bin/env python3
"""ov-1182: prove the actual nextest hash partitions cover the full archive.

Every shard checks the full inventory against all four partition inventories
before running. Ignored tests retain their normal nextest policy; a missing,
filtered, changed, duplicated, or unexpected runnable test fails closed.
"""
import argparse
from collections import Counter
from copy import deepcopy
import json
from pathlib import Path
import subprocess
import sys
import unittest

SHARDS = 4
ROOT = Path(__file__).resolve().parent.parent


def cases(inventory):
    result = {}
    for binary, suite in inventory['rust-suites'].items():
        if suite['status'] not in ('listed', 'skipped'):
            raise ValueError(f'unknown suite status: {binary}')
        for name, case in suite['testcases'].items():
            status = case['filter-match']['status']
            if status not in ('matches', 'mismatch') or type(case['ignored']) is not bool:
                raise ValueError(f'invalid case: {binary} {name}')
            result[binary, name] = (case['ignored'], status)
    return result


def verify(full, partitions):
    full = cases(full)
    expected = {key for key, (_, match) in full.items() if match == 'matches'}
    if not expected or len(partitions) != SHARDS:
        raise ValueError('empty full inventory or wrong shard count')
    counts = Counter()
    for inventory in partitions:
        for key, (ignored, match) in cases(inventory).items():
            if key not in full or ignored != full[key][0]:
                raise ValueError(f'unknown or changed test: {key}')
            if match == 'matches':
                counts[key] += 1
    if set(counts) != expected or any(n != 1 for n in counts.values()):
        raise ValueError(f'shard coverage differs: missing={expected - counts.keys()}, '
                         f'extra={counts.keys() - expected}, duplicated='
                         f'{[key for key, n in counts.items() if n != 1]}')
    return len(expected)


def inventory(archive, out, partition=None):
    command = ['cargo', 'nextest', 'list', '--profile', 'ci',
               '--archive-file', str(archive), '--extract-to', '.',
               '--extract-overwrite', '--workspace-remap', str(ROOT),
               '--message-format', 'json']
    if partition:
        command += ['--partition', partition]
    result = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
    if result.returncode:
        sys.stderr.write(result.stderr)
        result.check_returncode()
    out.write_text(result.stdout)
    return json.loads(result.stdout)


class CoverageTests(unittest.TestCase):
    def fixture(self, names):
        return {'rust-suites': {'binary': {'status': 'listed', 'testcases': {
            name: {'ignored': name == 'ignored', 'filter-match': {'status':
                   'matches' if name in names else 'mismatch'}}
            for name in ('a', 'b', 'c', 'd', 'ignored')}}}}

    def test_real_partition_union_and_mutations(self):
        full = self.fixture('abcd')
        parts = [self.fixture(name) for name in 'abcd']
        self.assertEqual(verify(full, parts), 4)
        for mutation in ('missing', 'duplicate', 'extra', 'ignored', 'status', 'count', 'empty'):
            bad = deepcopy(parts)
            if mutation == 'missing':
                bad[0]['rust-suites']['binary']['testcases']['a']['filter-match']['status'] = 'mismatch'
            elif mutation == 'duplicate':
                bad[1] = deepcopy(bad[0])
            elif mutation == 'extra':
                bad[0]['rust-suites']['binary']['testcases']['ignored']['filter-match']['status'] = 'matches'
            elif mutation == 'ignored':
                bad[0]['rust-suites']['binary']['testcases']['a']['ignored'] = True
            elif mutation == 'status':
                bad[0]['rust-suites']['binary']['status'] = 'unknown'
            elif mutation == 'count':
                bad.pop()
            else:
                full = self.fixture('')
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                verify(full, bad)

    def test_workflow_runs_all_partitions_and_guard(self):
        workflow = (ROOT / '.github/workflows/ci.yml').read_text()
        self.assertIn('cargo nextest archive --locked --profile ci --workspace --archive-file', workflow)
        self.assertIn('shard: [1, 2, 3, 4]', workflow)
        self.assertIn('--partition hash:${{ matrix.shard }}/4', workflow)
        self.assertIn('python3 scripts/rust-shards.py --archive', workflow)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--archive', type=Path)
    parser.add_argument('--out', type=Path)
    args = parser.parse_args()
    if args.self_test:
        return not unittest.TextTestRunner().run(unittest.defaultTestLoader.loadTestsFromTestCase(CoverageTests)).wasSuccessful()
    if not args.archive or not args.out:
        parser.error('--archive and --out are required')
    args.out.mkdir(parents=True, exist_ok=True)
    full = inventory(args.archive, args.out / 'full.json')
    partitions = [inventory(args.archive, args.out / f'{i}.json', f'hash:{i}/{SHARDS}')
                  for i in range(1, SHARDS + 1)]
    count = verify(full, partitions)
    print(f'Partition coverage: {count} runnable tests, each in exactly one of {SHARDS} shards')
    return 0


if __name__ == '__main__':
    sys.exit(main())
