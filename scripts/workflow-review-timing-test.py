#!/usr/bin/env python3
"""Regression checks for pairing real persisted stage observations."""
import importlib.util
import json
import sqlite3
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('timing', Path(__file__).with_name('workflow-review-timing.py'))
timing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(timing)


class TimingTest(unittest.TestCase):
    def test_candidate_pairing_across_stage_runs_and_rebuilds(self):
        db = sqlite3.connect(':memory:')
        db.executescript('''CREATE TABLE workflow_definitions(pin,definition);
            CREATE TABLE workflow_outbox(workspace,payload);
            CREATE TABLE workflow_events(workspace,revision,payload,notices);''')
        db.execute('INSERT INTO workflow_definitions VALUES (?,?)', ('d', json.dumps({'stages': []})))

        def observe(revision, kind, candidate, now, joined=False, workspace='w'):
            effect = dict(id=str(revision), item='ticket', run=f'run-{revision}', definition_pin='d',
                          kind={'Execute': kind}, candidate=candidate)
            event = dict(now_ms=now, command={'Observe': dict(effect=str(revision),
                         outcome={'Passed': dict(candidate=candidate)})})
            db.execute('INSERT INTO workflow_outbox VALUES (?,?)', (workspace, json.dumps(effect)))
            db.execute('INSERT INTO workflow_events VALUES (?,?,?,?)',
                       (workspace, revision, json.dumps(event), json.dumps(['ticket:WaitingJoin'] if joined else [])))

        observe(1, 'Build', 'old', 100)
        observe(2, 'Build', 'new', 200)
        observe(3, 'Route', 'new', 450, True)
        observe(4, 'Route', 'new', 500, True)  # Do not count the same completion twice.
        observe(5, 'Build', 'wrong', 600)
        observe(6, 'Route', 'other', 800, True)  # Another candidate has no matching build.
        observe(7, 'Build', 'snapshot', 0, workspace='snapshot')
        observe(8, 'Route', 'snapshot', 1000, True, workspace='snapshot')
        result = timing.measure(db, 'w')
        self.assertEqual(result['before']['count'], 1)
        self.assertEqual(result['before']['median_ms'], 250)
        self.assertNotEqual(result['before']['samples'][0]['build_run'], result['before']['samples'][0]['join_run'])
        self.assertEqual(result['after']['count'], 0)
        self.assertIsNone(result['after']['median_ms'])


if __name__ == '__main__':
    unittest.main()
