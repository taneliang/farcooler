#!/usr/bin/env python3
import importlib.util
import pathlib
import subprocess
import tempfile
import unittest

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
