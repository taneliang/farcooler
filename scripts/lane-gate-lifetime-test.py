#!/usr/bin/env python3
"""End-to-end lifetime checks for lane-gate's self-test process tree."""
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest

SCRIPTS = Path(__file__).resolve().parent
GATE = SCRIPTS / 'lane-gate.py'
START_CEILING = 300
EXIT_CEILING = 300
REAP_CEILING = 120


def processes():
    result = subprocess.run(['ps', '-axo', 'pid=,ppid=,command='], capture_output=True, text=True, check=True)
    rows = {}
    for line in result.stdout.splitlines():
        fields = line.strip().split(None, 2)
        if len(fields) == 3:
            rows[int(fields[0])] = (int(fields[1]), fields[2])
    return rows


def observe(root, known):
    rows = processes()
    parents = {root} | {pid for pid, cmd in known.items() if pid in rows and rows[pid][1] == cmd}
    while True:
        children = {pid for pid, (ppid, _) in rows.items() if ppid in parents} - parents
        if not children:
            break
        parents.update(children)
    for pid in parents - {root}:
        if pid in rows:
            known[pid] = rows[pid][1]
    return rows


class SelfTestLifetime(unittest.TestCase):
    def exercise(self, kill):
        with tempfile.TemporaryDirectory(prefix='lane-lifetime-') as temp:
            ready = Path(temp) / 'ready'
            log = Path(temp) / 'output'
            env = {**os.environ, 'TMPDIR': temp, 'HEAVY_LOCK_HELD': '1',
                   'FC_SELF_TEST_READY': str(ready)}
            env.pop('FC_SELF_TEST_PAUSE', None)
            if kill:
                env['FC_SELF_TEST_PAUSE'] = '1'
            known = {}
            with log.open('w') as output:
                owner = subprocess.Popen([sys.executable, str(GATE), '--self-test'], env=env,
                                         stdout=output, stderr=subprocess.STDOUT)
                try:
                    deadline = time.monotonic() + START_CEILING
                    while not ready.exists() and owner.poll() is None and time.monotonic() < deadline:
                        observe(owner.pid, known)
                        time.sleep(.05)
                    self.assertTrue(ready.exists(), f'real guard fixture never started\n{log.read_text()}')
                    runner_pid, fixture_pid = map(int, ready.read_text().split())
                    rows = processes()
                    if fixture_pid in rows:
                        known[fixture_pid] = rows[fixture_pid][1]
                    if kill:
                        # The runner publishes PIDs just before it stops itself.
                        deadline = time.monotonic() + START_CEILING
                        while True:
                            status = subprocess.run(['ps', '-p', str(runner_pid), '-o', 'stat='],
                                                    capture_output=True, text=True).stdout.strip()
                            if status.startswith('T'):
                                break
                            if owner.poll() is not None or time.monotonic() >= deadline:
                                self.fail(f'fixture runner did not stop\n{log.read_text()}')
                            time.sleep(.05)
                        owner.kill()
                    # Observe descendants until the owner's exit so survivors
                    # that are subsequently reparented can still be checked.
                    deadline = time.monotonic() + EXIT_CEILING
                    while owner.poll() is None and time.monotonic() < deadline:
                        observe(owner.pid, known)
                        time.sleep(.1)
                    self.assertEqual(owner.poll(), -signal.SIGKILL if kill else 0, log.read_text())
                    deadline = time.monotonic() + REAP_CEILING
                    while True:
                        rows = observe(owner.pid, known)
                        alive = {pid: cmd for pid, cmd in known.items()
                                 if pid in rows and rows[pid][1] == cmd and
                                 any(name in cmd for name in ('guard_fixture', 'nextest.py', 'cargo test'))}
                        if not alive or time.monotonic() >= deadline:
                            break
                        time.sleep(.1)
                    self.assertFalse(alive, f'self-test left processes alive: {alive}')
                finally:
                    if owner.poll() is None:
                        owner.kill()
                    owner.wait()
                    rows = observe(owner.pid, known)
                    for pid, cmd in known.items():
                        if pid in rows and rows[pid][1] == cmd:
                            try:
                                os.kill(pid, signal.SIGKILL)
                            except ProcessLookupError:
                                pass

    def test_killed_self_test(self):
        self.exercise(True)

    def test_normal_self_test(self):
        self.exercise(False)


class FixtureProcessGroups(unittest.TestCase):
    def exercise(self, args, outcome):
        with tempfile.TemporaryDirectory(prefix='fixture-lifetime-') as tmp:
            ready = Path(tmp) / 'ready'
            env = {**os.environ, 'FC_SELF_TEST_READY': str(ready), 'FC_SELF_TEST_PAUSE': '1',
                   'FC_SELF_TEST_FAIL': '1' if outcome == 'fail' else ''}
            with tempfile.TemporaryFile(mode='w+') as log:
                owner = subprocess.Popen([sys.executable, *args], env=env,
                                         stdout=log, stderr=subprocess.STDOUT,
                                         start_new_session=True)
                groups = {owner.pid}

                def observe_groups():
                    listing = subprocess.check_output(
                        ['ps', '-axo', 'pid=,ppid=,pgid=,stat='], text=True)
                    rows = {}
                    for line in listing.splitlines():
                        pid, parent, group, state = line.split()
                        rows[int(pid)] = (int(parent), int(group), state)
                    # Follow children of the owner and of previously observed
                    # groups, even after those groups have been reparented.
                    tracked = {pid for pid, (_, group, _) in rows.items() if group in groups}
                    while True:
                        children = {pid for pid, (parent, _, _) in rows.items()
                                    if parent in tracked} - tracked
                        if not children:
                            break
                        tracked.update(children)
                    groups.update(rows[pid][1] for pid in tracked)
                    return rows

                def output():
                    log.seek(0)
                    return log.read()

                try:
                    deadline = time.monotonic() + START_CEILING
                    while owner.poll() is None and time.monotonic() < deadline:
                        rows = observe_groups()
                        if ready.exists():
                            worker, *fixtures = map(int, ready.read_text().split())
                            if worker in rows and rows[worker][2].startswith('T'):
                                self.assertTrue(fixtures)
                                for pid in fixtures:
                                    self.assertIn(pid, rows)
                                    self.assertFalse(rows[pid][2].startswith('Z'))
                                    groups.add(rows[pid][1])
                                break
                        time.sleep(.05)
                    else:
                        self.fail('self-test never paused with live fixtures\n' + output())

                    if isinstance(outcome, signal.Signals):
                        owner.send_signal(outcome)
                    else:
                        os.kill(worker, signal.SIGCONT)
                    expected = {'pass': 0, 'fail': 1, signal.SIGTERM: 143, signal.SIGKILL: -9}[outcome]
                    deadline = time.monotonic() + EXIT_CEILING
                    while owner.poll() is None and time.monotonic() < deadline:
                        observe_groups()
                        time.sleep(.1)
                    self.assertEqual(owner.poll(), expected, output())

                    deadline = time.monotonic() + 15
                    while True:
                        live = {group for _, group, state in observe_groups().values()
                                if group in groups and not state.startswith('Z')}
                        if not live or time.monotonic() >= deadline:
                            break
                        time.sleep(.1)
                    self.assertFalse(live, f'self-test left process groups alive: {sorted(live)}')
                finally:
                    if owner.poll() is None:
                        owner.kill()
                    owner.wait()
                    for group in {g for _, g, state in observe_groups().values()
                                  if g in groups and not state.startswith('Z')}:
                        try:
                            os.killpg(group, signal.SIGKILL)
                        except ProcessLookupError:
                            pass

    @unittest.skipUnless(shutil.which('tmux') and shutil.which('lsof'), 'needs tmux and lsof')
    def test_tmux_self_test(self):
        for outcome in ('pass', 'fail', signal.SIGTERM, signal.SIGKILL):
            with self.subTest(outcome=outcome):
                self.exercise([str(SCRIPTS / 'tmux-leak-check.py'), '--self-test'], outcome)

    @unittest.skipUnless(shutil.which('cargo'), 'needs cargo')
    def test_process_guard_self_test(self):
        for outcome in ('pass', 'fail', signal.SIGTERM, signal.SIGKILL):
            with self.subTest(outcome=outcome):
                self.exercise([str(SCRIPTS / 'process_guard_test.py'), '--failfast',
                               'ProcessGuardTests.test_real_cargo_test_dies_with_owner'], outcome)


if __name__ == '__main__':
    unittest.main()
