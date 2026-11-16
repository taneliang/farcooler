#!/usr/bin/env python3
import copy
import importlib.util
import unittest

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

    def test_does_not_retry_other_cases(self):
        for key, value in [("head_branch", "branch"), ("event", "pull_request"),
                           ("run_attempt", 2), ("conclusion", "success")]:
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
