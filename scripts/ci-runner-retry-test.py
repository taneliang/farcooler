#!/usr/bin/env python3
import importlib.util
import json
import os
import pathlib
import subprocess
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location("retry", "scripts/ci-runner-retry.py")
retry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(retry)


class RetryTest(unittest.TestCase):
    def setUp(self):
        self.event = {"workflow_run": {"id": 123, "head_branch": "main", "event": "push",
                                       "run_attempt": 2, "conclusion": "failure"}}
        self.shards = [dict(id=i, name=f"Rust native tests (macOS {n}/2)",
                            conclusion="cancelled", steps=[], runner_name="")
                       for n, i in [(1, 456), (2, 789)]]
        self.attempts = {1: [dict(j, conclusion="success", steps=[{"name": "test"}],
                                  runner_name="runner") for j in self.shards],
                         2: self.shards}
        self.markers = set()

    def api(self, path):
        if path.startswith("check-runs/"):
            return [{"message": retry.MESSAGE}]
        if path.startswith("actions/artifacts?"):
            return {"artifacts": [{"name": name, "expired": False} for name in self.markers]}
        attempt = int(path.split("/attempts/")[1].split("/")[0])
        return {"jobs": self.attempts[attempt]}

    def test_later_attempt_siblings_and_once_bound(self):
        # Execute the workflow's actual POST command against a recording gh.
        workflow = pathlib.Path('.github/workflows/ci-runner-retry.yml').read_text()
        command = next(line.strip()[5:] for line in workflow.splitlines()
                       if line.strip().startswith('run: gh api --method POST'))
        with tempfile.TemporaryDirectory() as d:
            gh = pathlib.Path(d) / 'gh'
            log = pathlib.Path(d) / 'requests'
            gh.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$LOG"\n')
            gh.chmod(0o755)
            for expected in (456, 789):
                job, name = retry.retry_job(self.event, self.api)
                self.assertEqual(job, expected)
                self.markers.add(name)  # upload-artifact precedes POST
                subprocess.run(['/bin/sh', '-c', command], check=True,
                               env={'PATH': d, 'LOG': str(log), 'JOB': str(job),
                                    'GITHUB_REPOSITORY': 'owner/repo'})
                self.event['workflow_run']['run_attempt'] += 1
                self.attempts[self.event['workflow_run']['run_attempt']] = [
                    dict(self.shards[0 if job == 456 else 1], id=job + 1000)]
            self.assertIsNone(retry.retry_job(self.event, self.api))
            self.assertEqual(log.read_text().splitlines(), [
                'api --method POST repos/owner/repo/actions/jobs/456/rerun',
                'api --method POST repos/owner/repo/actions/jobs/789/rerun'])

    def test_first_failure_reruns_failed_jobs_once(self):
        self.event['workflow_run']['run_attempt'] = 1
        self.attempts[1] = [dict(self.shards[0], conclusion='failure'),
                            dict(self.shards[1], conclusion='success')]
        jobs, name = retry.failed_retry(self.event, self.api)
        self.assertEqual([job['id'] for job in jobs], [456])
        self.markers.add(name)
        self.assertIsNone(retry.failed_retry(self.event, self.api))
        self.markers.clear()
        self.event['workflow_run']['run_attempt'] = 2
        self.attempts[2] = [dict(self.shards[0], conclusion='failure')]
        self.assertIsNone(retry.failed_retry(self.event, self.api))
        workflow = pathlib.Path('.github/workflows/ci-runner-retry.yml').read_text()
        self.assertIn('actions/runs/$RUN/rerun-failed-jobs', workflow)

    def test_failed_retry_excludes_branches_and_pull_requests(self):
        self.event['workflow_run']['run_attempt'] = 1
        self.attempts[1] = [dict(self.shards[0], conclusion='failure')]
        for key, value in [('head_branch', 'other'), ('event', 'pull_request')]:
            old = self.event['workflow_run'][key]
            self.event['workflow_run'][key] = value
            self.assertIsNone(retry.failed_retry(self.event, self.api))
            self.event['workflow_run'][key] = old

    def test_summary_and_second_failure(self):
        self.event['workflow_run']['run_attempt'] = 1
        job = dict(self.shards[0], conclusion='failure')
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            (root / 'event').write_text(json.dumps(self.event))
            env = {'GITHUB_REPOSITORY': 'owner/repo', 'GITHUB_EVENT_PATH': str(root / 'event'),
                   'GITHUB_OUTPUT': str(root / 'output'), 'GITHUB_STEP_SUMMARY': str(root / 'summary')}

            def gh(args, **kwargs):
                path = args[-1]
                if path.endswith('/logs'):
                    return 'test example::broken ... FAILED\n'
                if 'artifacts?' in path:
                    return json.dumps([{'artifacts': []}]).encode()
                return json.dumps([{'jobs': [job]}]).encode()

            old = os.getcwd()
            try:
                os.chdir(d)
                with mock.patch.dict(os.environ, env), mock.patch.object(retry.subprocess, 'check_output', side_effect=gh):
                    retry.main()
                    self.assertIn(job['name'], (root / 'summary').read_text())
                    self.assertIn('example::broken', (root / 'summary').read_text())
                    self.assertIn('failed_run=123', (root / 'output').read_text())
                    (root / 'output').unlink()
                    self.event['workflow_run']['run_attempt'] = 2
                    (root / 'event').write_text(json.dumps(self.event))
                    retry.main()
                    self.assertFalse((root / 'output').exists())
            finally:
                os.chdir(old)

    def test_parse_real_failure_lines(self):
        # Run 43985211994, job 133713551594, including nextest ANSI colors.
        log = ("2026-11-17T00:23:01.9992469Z     test workflow_service_admitted_ci_recovers_original_bundle_after_reconfiguration ... FAILED\n"
               "2026-11-17T00:23:12.0084320Z \x1b[31;1m        FAIL\x1b[0m [   0.333s] (1470/1811) \x1b[35;1mfarcooler-daemon::workflow_service_over_socket\x1b[0m \x1b[34;1mworkflow_service_admitted_ci_recovers_original_bundle_after_reconfiguration\x1b[0m\n")
        self.assertEqual(retry.failed_tests(log), [
            'farcooler-daemon::workflow_service_over_socket workflow_service_admitted_ci_recovers_original_bundle_after_reconfiguration',
            'workflow_service_admitted_ci_recovers_original_bundle_after_reconfiguration'])

    def test_unrelated_failures_are_not_retried(self):
        for key, value in [('head_branch', 'other'), ('event', 'pull_request'), ('conclusion', 'success')]:
            old = self.event['workflow_run'][key]
            self.event['workflow_run'][key] = value
            self.assertIsNone(retry.retry_job(self.event, self.api))
            self.event['workflow_run'][key] = old
        self.attempts[2] = [dict(j, conclusion='failure', steps=[{'name': 'test'}], runner_name='runner')
                            for j in self.shards]
        self.assertIsNone(retry.retry_job(self.event, self.api))


if __name__ == '__main__':
    unittest.main()
