#!/usr/bin/env python3
"""Own a command's session even when its calling gate is killed (ov-1138).

A finally block cannot run after SIGKILL. A separate supervisor owns the
command and watches a pipe held only by the gate. EOF means the gate went
away; a finished command means its remaining children must go too. Only the
new session created here is signaled, including nextest's separate test groups.
Daemons/tmux servers that leave the session remain the responsibility of
tmux-leak-check's private TMUX_TMPDIR checks.
"""
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import time


def run(argv, *, cwd=None, env=None, stdout=None, stderr=None):
    """A subprocess.run result, with an out-of-process lifetime owner."""
    read, write = os.pipe()
    supervisor = None
    try:
        # The writer is CLOEXEC and is never passed on: children cannot keep
        # the owner alive accidentally. The supervisor gets only the reader.
        supervisor = subprocess.Popen(
            [sys.executable, str(Path(__file__).resolve()), '--supervise', str(read), *argv],
            cwd=cwd, env=env, stdout=stdout, stderr=stderr, pass_fds=(read,), start_new_session=True,
        )
        os.close(read)
        read = None
        return subprocess.CompletedProcess(argv, supervisor.wait())
    finally:
        os.close(write)
        if read is not None:
            os.close(read)
        if supervisor is not None:
            # KeyboardInterrupt also closes the lease; wait for cleanup before
            # allowing a caller to release its heavy-job lock.
            supervisor.wait()


def run_self_test(argv):
    """Wait for fixture cleanup on catchable signals, and leave a lease for SIGKILL."""
    def interrupted(signum, _frame):
        for sig in previous:
            signal.signal(sig, signal.SIG_IGN)
        raise SystemExit(128 + signum)

    previous = {sig: signal.signal(sig, interrupted)
                for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP, signal.SIGQUIT)}
    try:
        return run(argv).returncode
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)


def self_test_ready(*pids):
    """Publish exact fixture identities for the lifetime regressions."""
    if ready_file := os.environ.get('FC_SELF_TEST_READY'):
        ready = Path(ready_file)
        pending = ready.with_suffix('.tmp')
        pending.write_text(' '.join(map(str, (os.getpid(), *pids))))
        first_report = not ready.exists()
        pending.replace(ready)
        if first_report and os.environ.get('FC_SELF_TEST_PAUSE'):
            os.kill(os.getpid(), signal.SIGSTOP)
        if os.environ.get('FC_SELF_TEST_FAIL'):
            raise AssertionError('injected self-test failure after fixture startup')


def members(session):
    """Exact PIDs still in our session, even after nextest has been reparented."""
    listing = subprocess.run(['ps', '-axo', 'pid='], capture_output=True, text=True, check=True)
    found = []
    for value in listing.stdout.split():
        pid = int(value)
        try:
            if pid != os.getpid() and os.getsid(pid) == session:
                found.append(pid)
        except ProcessLookupError:
            pass
    return found


def end_session(child):
    """Nextest uses setpgid per test; ending only its own group misses them."""
    # The supervisor itself leads this session and remains alive throughout
    # cleanup, pinning the ID even after the command's PID has been reaped.
    session = os.getsid(0)
    for sig, grace in ((signal.SIGTERM, 1), (signal.SIGKILL, 1)):
        deadline = time.monotonic() + grace
        while True:
            child.poll()
            pids = members(session)
            if not pids:
                break
            for pid in pids:
                try:
                    # Recheck immediately before signaling: never signal a
                    # recycled PID now belonging to someone else's session.
                    if os.getsid(pid) == session:
                        os.kill(pid, sig)
                except ProcessLookupError:
                    pass
            if time.monotonic() >= deadline:
                break
            time.sleep(0.05)
    child.wait()


def supervise(read, argv):
    child = None
    try:
        # Cancellation may target the runner's whole process group. Keep the
        # supervisor outside that group so it can still reap separate test
        # groups. Stay in our session, which bounds cleanup to this run.
        # This standalone supervisor has no threads; preexec_fn also supports
        # the system Python 3.9 (Popen's process_group requires Python 3.11).
        child = subprocess.Popen(argv, preexec_fn=os.setpgrp)
        while child.poll() is None:
            if select.select([read], [], [], 0.1)[0] and not os.read(read, 1):
                return 125  # owner gone; no caller remains to receive this
        return child.returncode if child.returncode >= 0 else 128 - child.returncode
    except OSError as error:
        print(f'process-guard: {error}', file=sys.stderr)
        return 127
    finally:
        if child is not None:
            end_session(child)
        os.close(read)


if __name__ == '__main__':
    if len(sys.argv) > 3 and sys.argv[1] == '--supervise':
        sys.exit(supervise(int(sys.argv[2]), sys.argv[3:]))
    sys.exit('process_guard.py is imported by lane-gate and tmux-leak-check')
