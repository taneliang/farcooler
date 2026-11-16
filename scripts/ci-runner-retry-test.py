#!/usr/bin/env python3
import copy
import importlib.util
import unittest
from unittest import mock
import json
import os
import tempfile

spec = importlib.util.spec_from_file_location("retry", "scripts/ci-runner-retry.py")
retry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(retry)


class RetryTest(unittest.TestCase):
    def setUp(self):
        self.event = {"workflow_run": {"id": 123, "head_branch": "main", "event": "push",
                                       "run_attempt": 1, "conclusion": "failure"}}
        self.jobs = [{"id": 456, "name": "Rust (macos-latest)", "conclusion": "cancelled", "steps": [], "runner_name": ""}]
        self.annotations = [{"message": retry.MESSAGE}]

    def api(self, path):
        return self.annotations if path.startswith("check-runs/") else {"jobs": self.jobs}

    def test_acquisition_failure_retries(self):
        self.assertEqual(retry.retry_job(self.event, self.api), 456)

    def test_sibling_cancellations_request_each_once(self):
        self.jobs = [dict(self.jobs[0], id=456, name="Rust native tests (macOS 1/2)"),
                     dict(self.jobs[0], id=789, name="Rust native tests (macOS 2/2)")]
        attempts = {1: self.jobs, 2: [dict(self.jobs[0], id=900)],
                    3: [dict(self.jobs[1], id=901)]}
        requests = []

        def check_output(args):
            path = args[-1]
            if "annotations" in path:
                return json.dumps([self.annotations]).encode()
            attempt = int(path.split("/attempts/")[1].split("/")[0])
            return json.dumps([{"jobs": attempts[attempt]}]).encode()

        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8") as event_file:
            with mock.patch.dict(os.environ, {"GITHUB_REPOSITORY": "owner/repo",
                                             "GITHUB_EVENT_PATH": event_file.name}), \
                 mock.patch.object(retry.subprocess, "check_output", side_effect=check_output), \
                 mock.patch.object(retry.subprocess, "run", side_effect=lambda args, check: requests.append(args)):
                for attempt in (1, 2, 3):
                    self.event["workflow_run"]["run_attempt"] = attempt
                    event_file.seek(0)
                    json.dump(self.event, event_file)
                    event_file.truncate()
                    event_file.flush()
                    retry.main()
        self.assertEqual(requests, [
            ["gh", "api", "--method", "POST", "repos/owner/repo/actions/jobs/456/rerun"],
            ["gh", "api", "--method", "POST", "repos/owner/repo/actions/jobs/789/rerun"],
        ])

    def test_does_not_retry_other_cases(self):
        for key, value in [("head_branch", "branch"), ("event", "pull_request"),
                           ("conclusion", "success")]:
            event = copy.deepcopy(self.event)
            event["workflow_run"][key] = value
            self.assertIsNone(retry.retry_job(event, self.api))
        for key, value in [("conclusion", "failure"), ("steps", [{"name": "test"}]),
                           ("runner_name", "runner")]:
            original = self.jobs[0][key]
            self.jobs[0][key] = value
            self.assertIsNone(retry.retry_job(self.event, self.api))
            self.jobs[0][key] = original
        self.annotations = [{"message": "test failed"}]
        self.assertIsNone(retry.retry_job(self.event, self.api))


if __name__ == "__main__":
    unittest.main()
