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
        self.event = {"workflow_run": {"id": 123, "name": "CI", "head_branch": "main", "event": "push",
                                       "status": "completed",
                                       "run_attempt": 2, "conclusion": "failure"}}
        self.shards = [dict(id=i, name=f"Rust native tests (macOS {n}/2)",
                            conclusion="cancelled", steps=[], runner_name="",
                            completed_at="2026-11-23T14:29:20Z")
                       for n, i in [(1, 456), (2, 789)]]
        self.attempts = {1: [dict(j, conclusion="success", steps=[{"name": "test"}],
                                  runner_name="runner") for j in self.shards],
                         2: self.shards}

    def api(self, path):
        if path.startswith("check-runs/"):
            return [{"message": retry.MESSAGE}]
        attempt = int(path.split("/attempts/")[1].split("/")[0])
        return {"jobs": self.attempts[attempt]}

    def test_later_attempt_siblings_and_retry_bound(self):
        for expected in (456, 789):
            job = retry.retry_job(self.event, self.api)
            self.assertEqual(job, expected)
            self.event['workflow_run']['run_attempt'] += 1
            self.attempts[self.event['workflow_run']['run_attempt']] = [
                dict(self.shards[0 if job == 456 else 1], id=job + 1000,
                     completed_at="2026-11-23T14:59:20Z")]
        self.assertEqual(retry.retry_job(self.event, self.api), 1456)
        self.event['workflow_run']['run_attempt'] = retry.MAX_ATTEMPTS
        self.assertIsNone(retry.retry_job(self.event, self.api))

    def test_first_failure_reruns_failed_jobs_once(self):
        self.event['workflow_run']['run_attempt'] = 1
        self.attempts[1] = [dict(self.shards[0], conclusion='failure'),
                            dict(self.shards[1], conclusion='success')]
        jobs = retry.failed_retry(self.event, self.api)
        self.assertEqual([job['id'] for job in jobs], [456])
        self.event['workflow_run']['run_attempt'] = 2
        self.attempts[2] = [dict(self.shards[0], conclusion='failure')]
        self.assertIsNone(retry.failed_retry(self.event, self.api))

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
                   'GITHUB_STEP_SUMMARY': str(root / 'summary')}
            current = self.event['workflow_run'].copy()
            posts = []

            def gh(args, **kwargs):
                path = args[-1]
                if path.endswith('/runs/123'):
                    return json.dumps(current).encode()
                if path.endswith('/logs'):
                    return 'test example::broken ... FAILED\n'
                if '--method' in args:
                    posts.append(path)
                    current.update(run_attempt=2, status="queued", conclusion=None)
                    return b''
                return json.dumps([{'jobs': [job]}]).encode()

            with mock.patch.dict(os.environ, env), mock.patch.object(retry.subprocess, 'check_output', side_effect=gh), mock.patch.object(retry.time, 'sleep'):
                retry.main()
                self.assertIn(job['name'], (root / 'summary').read_text())
                self.assertIn('example::broken', (root / 'summary').read_text())
                retry.main()  # Duplicate completion for the original attempt.
                current.update(status="completed", conclusion="failure")
                self.event['workflow_run'].update(current)
                (root / 'event').write_text(json.dumps(self.event))
                retry.main()
            self.assertEqual(posts, ['repos/owner/repo/actions/runs/123/rerun-failed-jobs'])

    def test_agentkit_removal_carried_cancellation_gets_a_retry(self):
        # Run 45191956221. Attempt 2 copied the build's cancellation without
        # its check annotation. Only the Mac tests had actually been rerun.
        build = dict(id=137752581918, name="Swift (app build + bundle checks)",
                     conclusion="cancelled", steps=[], runner_name="",
                     completed_at="2026-11-23T14:29:20Z")
        tests = dict(id=137752581792, name="Swift (AgentKit + Mac tests)",
                     conclusion="cancelled", steps=[], runner_name="",
                     completed_at=build["completed_at"])
        copied = dict(build, id=137765399570, runner_name=None)
        self.event["workflow_run"].update(id=45191956221, conclusion="cancelled")
        self.attempts = {
            1: [tests, build],
            2: [dict(tests, id=137765397217, conclusion="success",
                     steps=[{"name": "Set up job"}], runner_name="GitHub Actions 1000021234"), copied],
        }

        def api(path):
            if path.startswith("check-runs/"):
                return [{"message": retry.MESSAGE}] if str(build["id"]) in path else []
            return self.api(path)

        self.assertEqual(retry.retry_job(self.event, api), copied["id"])

        # Attempt 2 is still current after three rejected submissions. The
        # independent recovery controller must select the same copied job.
        current = self.event["workflow_run"].copy()
        posts = []
        with tempfile.TemporaryDirectory() as d:
            event = pathlib.Path(d) / "event"
            event.write_text(json.dumps(self.event))
            env = {"GITHUB_REPOSITORY": "owner/repo", "GITHUB_EVENT_PATH": str(event)}
            recovery = {"workflow_run": {"display_title": "Retry CI 45191956221 attempt 2"}}

            def gh(args, **kwargs):
                path = args[-1].removeprefix("repos/owner/repo/")
                if "--method" in args:
                    posts.append(path)
                    if len(posts) <= retry.SUBMISSION_TRIES:
                        raise subprocess.CalledProcessError(1, args)
                    current.update(run_attempt=3, status="queued", conclusion=None)
                    raise subprocess.TimeoutExpired(args, kwargs["timeout"])
                if path == "actions/runs/45191956221":
                    return json.dumps(current).encode()
                # Simulate a stale artifact from a controller that died
                # between reserving and submitting the rerun.
                if path.startswith("actions/artifacts?"):
                    return json.dumps([{"artifacts": [{
                        "name": "runner-retry-45191956221-attempt-2", "expired": False
                    }]}]).encode()
                return json.dumps([api(path)]).encode()

            with mock.patch.dict(os.environ, env), mock.patch.object(retry.subprocess, 'check_output', side_effect=gh), mock.patch.object(retry.time, 'sleep'):
                with self.assertRaisesRegex(RuntimeError, "Could not confirm"):
                    retry.main()
                self.assertEqual(current["run_attempt"], 2)
                event.write_text(json.dumps(recovery))
                with mock.patch.object(retry.sys, "argv", ["ci-runner-retry.py", "--recover"]):
                    retry.main()
                    retry.main()  # Redelivery cannot resubmit the uncertain POST.
            self.assertEqual(posts, [f"actions/jobs/{copied['id']}/rerun"] * (retry.SUBMISSION_TRIES + 1))
            self.assertEqual(current["run_attempt"], 3)

        # A later, unrelated cancellation of the same job cannot inherit the
        # original acquisition annotation.
        self.attempts[3] = [dict(copied, id=137778665576, completed_at="2026-11-23T15:10:34Z")]
        self.event["workflow_run"]["run_attempt"] = 3
        self.assertIsNone(retry.retry_job(self.event, api))
        self.attempts[3][0].update(conclusion="success", steps=[{"name": "Set up job"}],
                                  runner_name="GitHub Actions 1000021238")
        self.event["workflow_run"].update(run_attempt=3, conclusion="success")
        self.assertIsNone(retry.retry_job(self.event, api))

    def test_stale_completion_does_not_reserve_a_retry(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            event = root / "event"
            event.write_text(json.dumps(self.event))
            env = {"GITHUB_REPOSITORY": "owner/repo", "GITHUB_EVENT_PATH": str(event)}
            with mock.patch.dict(os.environ, env), mock.patch.object(
                    retry.subprocess, "check_output", return_value=b'{"status":"completed","run_attempt":3}') as gh:
                retry.main()
            gh.assert_called_once_with(["gh", "api", "repos/owner/repo/actions/runs/123"], timeout=retry.REQUEST_TIMEOUT)

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
