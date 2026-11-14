#!/usr/bin/env python3
"""Real-process regressions for a gate losing its owner (ov-1138)."""
import importlib.util
import os
from pathlib import Path
import signal
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

SCRIPTS = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), SCRIPTS / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def running(pid):
    # An orphan can be an exited zombie until init reaps it on Linux.
    out = subprocess.run(['ps', '-p', str(pid), '-o', 'stat='], capture_output=True, text=True)
    return bool(out.stdout.strip()) and not out.stdout.strip().startswith('Z')


def until(predicate, seconds=5):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if predicate():
            return True
        time.sleep(0.05)
    return False


class ProcessGuardTests(unittest.TestCase):
    def exercise(self, entry, killed):
        with tempfile.TemporaryDirectory(prefix='fc-owner-', dir='/tmp') as scratch:
            root = Path(scratch)
            # A nextest stand-in and a test that ignores TERM. Both publish
            # their own PIDs; cleanup never discovers victims by name.
            leaf = root / 'test.py'
            leaf.write_text("import os, pathlib, signal, time\n"
                            "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
                            "pathlib.Path('leaf').write_text(str(os.getpid()))\n"
                            "while True: time.sleep(1)\n")
            runner = root / 'nextest.py'
            runner.write_text("import os, pathlib, subprocess, sys, time\n"
                              "pathlib.Path('runner').write_text(str(os.getpid()))\n"
                              "if 'TMUX_TMPDIR' in os.environ: pathlib.Path('base').write_text(os.environ['TMUX_TMPDIR'])\n"
                              "subprocess.Popen([sys.executable, 'test.py'], preexec_fn=os.setpgrp)\n"
                              "while not pathlib.Path('leaf').exists(): time.sleep(.01)\n"
                              "if os.environ.get('FC_GUARD_FINISH'): sys.exit(0)\n"
                              "while True: time.sleep(1)\n")
            if entry in ('gate', 'nested'):
                command = shlex.join([sys.executable, 'nextest.py'])
                if entry == 'nested':
                    command = shlex.join([sys.executable, str(SCRIPTS / 'tmux-leak-check.py'),
                                          '--', sys.executable, 'nextest.py'])
                body = ("m = load('lane-gate')\n"
                        f"m.run_step(m.Step('rust', 'test', {command!r}), "
                        "Path.cwd(), dict(os.environ), Path.cwd(), Path.cwd())\n")
            else:
                body = ("m = load('tmux-leak-check')\n"
                        "m.run([sys.executable, 'nextest.py'], kill=False, grace=0)\n")
            owner_code = (f"import sys, os\nfrom pathlib import Path\nsys.path.insert(0, {str(SCRIPTS)!r})\n"
                          "from process_guard_test import load\n" + body)
            bystander = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
            owner = subprocess.Popen([sys.executable, '-c', owner_code], cwd=root,
                                     env={**os.environ, 'FC_GUARD_FINISH': '1' if killed == 'finished' else ''},
                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            pids = []
            try:
                self.assertTrue(until(lambda: (root / 'leaf').exists() and (root / 'leaf').read_text()),
                                'test never started')
                pids = [int((root / name).read_text()) for name in ('runner', 'leaf')]
                if killed == 'runner':
                    os.kill(pids[0], signal.SIGKILL)
                elif killed == 'runner-group':
                    os.killpg(os.getpgid(pids[0]), signal.SIGKILL)
                elif killed != 'finished':
                    os.kill(owner.pid, killed)
                owner.wait(timeout=8)
                self.assertTrue(until(lambda: not any(running(pid) for pid in pids)),
                                f'{entry} left test processes alive after {killed}: {pids}')
                self.assertIsNone(bystander.poll(), 'another run was signaled')
            finally:
                for pid in pids:
                    if running(pid):
                        os.kill(pid, signal.SIGKILL)
                if owner.poll() is None:
                    owner.kill()
                owner.wait()
                bystander.kill()
                bystander.wait()
                if (root / 'base').exists():
                    shutil.rmtree((root / 'base').read_text(), ignore_errors=True)

    def test_killed_gate_reaps_its_tests(self):
        for sig in (signal.SIGTERM, signal.SIGKILL):
            with self.subTest(signal=sig):
                self.exercise('gate', sig)

    def test_killed_nextest_wrapper_reaps_its_tests(self):
        self.exercise('wrapper', signal.SIGKILL)

    def test_killed_nextest_reaps_its_test(self):
        self.exercise('wrapper', 'runner')

    def test_killed_nextest_group_reaps_its_test(self):
        self.exercise('wrapper', 'runner-group')

    def test_killed_gate_reaps_nested_wrapper_tests(self):
        self.exercise('nested', signal.SIGKILL)

    def test_finished_runner_reaps_its_remaining_test(self):
        self.exercise('wrapper', 'finished')

    def test_normal_result_and_environment_survive(self):
        import process_guard
        with tempfile.TemporaryDirectory(prefix='fc-result-', dir='/tmp') as scratch:
            out = Path(scratch) / 'out'
            with out.open('w') as stream:
                result = process_guard.run(
                    [sys.executable, '-c', "import os; print(os.getcwd(), os.environ['FC_GUARD_TEST']); exit(7)"],
                    cwd=scratch, env={**os.environ, 'FC_GUARD_TEST': 'inherited'},
                    stdout=stream, stderr=subprocess.STDOUT,
                )
            self.assertEqual(result.returncode, 7)
            self.assertEqual(out.read_text().strip(), f'{Path(scratch).resolve()} inherited')


if __name__ == '__main__':
    unittest.main()
