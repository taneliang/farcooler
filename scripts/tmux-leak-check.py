#!/usr/bin/env python3
"""Fail if a command leaves a tmux server or a farcoolerd running that it started (ov-207, ov-566).

A test tmux server once outlived its run by 15.5 hours at 92% CPU. Teardown now
ends its server by PID (`farcooler_tmux::reap_server`), but a test that never
calls it, or a new harness that forgets, leaks all the same, and nothing in the
test run would notice. This notices.

It gives the command a private `TMUX_TMPDIR` that it makes, so every tmux
server the command starts keeps its socket there and nowhere else, then looks
only in that directory afterwards. A socket with a live server behind it is a
leak. It never looks at, or signals, any other tmux server: not the owner's,
not another lane's, not one that was already running.

  Outside CI it only reports a leak (exit 1) and names the PID.
  With `CI` set it also ends the leaked servers, by exact PID.

A `farcoolerd` is a leak too (ov-566). The CLI starts a daemon on demand,
detached, so a test that runs `farcooler <command>` under a scratch home and
never stops the daemon leaves it running after the run, parented to launchd:
four were found after full Mac runs. A daemon the command started inherits the
private `TMUX_TMPDIR`, which is how this tells it from the owner's app, another
lane's daemon, or a developer's, none of which carries it; `ps eww` shows that
environment for a process that is not a platform binary. A daemon still alive
after the grace period is named, and ended in CI.

  ./scripts/tmux-leak-check.py -- cargo test -p farcooler-tmux
  ./scripts/tmux-leak-check.py --self-test
"""

import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time

GRACE_SECONDS = 5
# Every private dir `run` made, for the self-test to remove once it has ended
# the servers it deliberately leaked.
BASES = []


def server_pids(base):
    """PIDs of the servers behind the sockets under `base/tmux-<uid>/`."""
    sockets = os.path.join(base, f"tmux-{os.getuid()}")
    if not os.path.isdir(sockets):
        return set()
    paths = [os.path.realpath(os.path.join(sockets, name)) for name in os.listdir(sockets)]
    if not paths:
        return set()
    # One lsof for every socket: it ORs its file arguments, and each separate
    # run costs a scan of the whole file table (seconds on a busy Mac).
    out = subprocess.run(["lsof", "-t", "--", *paths], capture_output=True, text=True).stdout
    return {int(p) for p in out.split()}


# `.../farcoolerd`, `farcoolerd-local`: the daemon and its channel builds, in any mode.
DAEMON = re.compile(r"(^|/)farcoolerd(-[a-z]+)?( |$)")


def daemon_pids(base):
    """PIDs of the `farcoolerd` processes that carry `TMUX_TMPDIR=<base>` in their environment."""
    listing = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True).stdout
    candidates = []
    for line in listing.splitlines():
        pid, _, command = line.strip().partition(" ")
        if pid.isdigit() and int(pid) != os.getpid() and DAEMON.search(command.lstrip().split(" --", 1)[0] + " "):
            candidates.append(pid)
    if not candidates:
        return set()
    # `ps eww` prints each process's environment after its command line.
    shown = subprocess.run(["ps", "eww", "-p", ",".join(candidates)], capture_output=True, text=True).stdout
    marker = re.compile(r"(^|\s)TMUX_TMPDIR=" + re.escape(base) + r"(\s|$)")
    found = set()
    for line in shown.splitlines():
        head = line.split(None, 1)
        if head and head[0].isdigit() and marker.search(line):
            found.add(int(head[0]))
    return found


def leaked(base, grace=GRACE_SECONDS):
    """(servers, daemons) still behind a socket or alive under `base` after `grace` seconds."""
    deadline = time.time() + grace
    while True:
        servers, daemons = server_pids(base), daemon_pids(base)
        if not (servers or daemons) or time.time() >= deadline:
            return servers, daemons
        time.sleep(0.25)


def end(pids):
    for sig in (signal.SIGTERM, signal.SIGKILL):
        for pid in pids:
            try:
                os.kill(pid, sig)
            except ProcessLookupError:
                pass
        time.sleep(1)


def run(argv, kill, grace=GRACE_SECONDS):
    """Run `argv` with a private TMUX_TMPDIR; return (exit code, leaked PIDs)."""
    # Short and directly under /tmp: a unix socket path is limited to about
    # 100 bytes, and $TMPDIR on a Mac is already long.
    base = tempfile.mkdtemp(prefix="fc-leak-", dir="/tmp")
    BASES.append(base)
    try:
        status = subprocess.run(argv, env={**os.environ, "TMUX_TMPDIR": base}).returncode
        servers, daemons = leaked(base, grace)
        for pid in sorted(servers):
            print(f"tmux-leak-check: LEAKED tmux server {pid} (socket under {base})", file=sys.stderr)
        for pid in sorted(daemons):
            print(f"tmux-leak-check: LEAKED farcoolerd {pid} (started by the run; a test must stop the daemon it starts)", file=sys.stderr)
        pids = servers | daemons
        if kill:
            end(pids)
        return (1 if pids else status), pids
    finally:
        # Kept while a leaked server or daemon remains, so it can still be found.
        if not (server_pids(base) or daemon_pids(base)):
            shutil.rmtree(base, ignore_errors=True)


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


def self_test():
    tmux = shutil.which("tmux")
    if not tmux or not shutil.which("lsof"):
        print("tmux-leak-check: SKIP self-test, no tmux or lsof")
        return 0
    start = "{tmux} -L {name} -f /dev/null new-session -d -s x 'sleep 600'"
    failures = []
    pids = set()

    # A bystander on its own TMUX_TMPDIR: never looked at or touched.
    other = tempfile.mkdtemp(prefix="fc-other-", dir="/tmp")
    env = {**os.environ, "TMUX_TMPDIR": other}
    subprocess.run(["sh", "-c", start.format(tmux=tmux, name="bystander")], env=env, check=True)
    bystanders = server_pids(other)
    try:
        # Outside CI: a leak is reported, the server is left running.
        rc, pids = run(["sh", "-c", start.format(tmux=tmux, name=f"leak{os.getpid()}")], kill=False, grace=1)
        if rc != 1 or len(pids) != 1:
            failures.append(f"a leaked server was not caught (rc={rc}, pids={pids})")
        if not all(alive(p) for p in pids):
            failures.append("a leak was killed outside CI")
        # In CI: a leak is reported and ended.
        rc2, pids2 = run(["sh", "-c", start.format(tmux=tmux, name=f"leakci{os.getpid()}")], kill=True, grace=1)
        if rc2 != 1 or len(pids2) != 1 or any(alive(p) for p in pids2):
            failures.append(f"a leaked server was not ended in CI (rc={rc2}, pids={pids2})")
        # A command that cleans up is not a leak.
        clean = start.format(tmux=tmux, name=f"clean{os.getpid()}") + f"; {tmux} -L clean{os.getpid()} kill-server"
        rc3, pids3 = run(["sh", "-c", clean], kill=False, grace=1)
        if rc3 != 0 or pids3:
            failures.append(f"a clean command was flagged (rc={rc3}, pids={pids3})")
        # Dead sockets left behind are not leaks, and do not hide a live one
        # among them (all sockets are checked in a single lsof).
        n = os.getpid()
        stale = "".join(
            f"{start.format(tmux=tmux, name=f'st{n}{i}')}; kill -9 $({tmux} -L st{n}{i} display -p '#{{pid}}'); "
            for i in range(3)
        )
        rc4, pids4 = run(["sh", "-c", stale + start.format(tmux=tmux, name=f"live{n}")], kill=True, grace=1)
        if rc4 != 1 or len(pids4) != 1 or any(alive(p) for p in pids4):
            failures.append(f"a live leak among stale sockets was missed (rc={rc4}, pids={pids4})")
        rc5, pids5 = run(["sh", "-c", stale], kill=False, grace=1)
        if rc5 != 0 or pids5:
            failures.append(f"stale sockets were flagged (rc={rc5}, pids={pids5})")
        if not all(alive(p) for p in bystanders):
            failures.append("a server outside the private TMUX_TMPDIR was killed")
        # A daemon the command leaves running (ov-566): a python script named
        # farcoolerd, so its command line ends in `/farcoolerd`. (An argv[0] set
        # with `exec -a` does not survive Homebrew's python re-launching itself.)
        fakes = tempfile.mkdtemp(prefix="fc-fake-", dir="/tmp")
        fake = os.path.join(fakes, "farcoolerd")
        with open(fake, "w") as f:
            f.write("import time\ntime.sleep(600)\n")
        python = sys.executable
        leave = ["sh", "-c", f"{python} {fake} </dev/null >/dev/null 2>&1 &"]
        rc6, pids6 = run(leave, kill=False, grace=1)
        if rc6 != 1 or len(pids6) != 1:
            failures.append(f"a leaked farcoolerd was not caught (rc={rc6}, pids={pids6})")
        if not all(alive(p) for p in pids6):
            failures.append("a leaked farcoolerd was killed outside CI")
        end([p for p in pids6 if alive(p)])
        rc7, pids7 = run(leave, kill=True, grace=1)
        if rc7 != 1 or len(pids7) != 1 or any(alive(p) for p in pids7):
            failures.append(f"a leaked farcoolerd was not ended in CI (rc={rc7}, pids={pids7})")
        # One the command stops is no leak.
        stops = ["sh", "-c", f"{python} {fake} </dev/null >/dev/null 2>&1 & kill $!; wait $! 2>/dev/null; true"]
        rc8, pids8 = run(stops, kill=False, grace=1)
        if rc8 != 0 or pids8:
            failures.append(f"a stopped farcoolerd was flagged (rc={rc8}, pids={pids8})")
        # A farcoolerd outside the private TMUX_TMPDIR (the owner's, another
        # lane's) is never looked at, though it is running.
        lone = subprocess.Popen([python, fake], env={**os.environ, "TMUX_TMPDIR": other})
        try:
            rc9, pids9 = run([python, "-c", "pass"], kill=True, grace=1)
            if rc9 != 0 or pids9 or lone.poll() is not None:
                failures.append(f"a farcoolerd of another run was flagged or ended (rc={rc9}, pids={pids9})")
            # The environment is what finds it: this one is found by its own base.
            if lone.pid not in daemon_pids(other):
                failures.append("a running farcoolerd was not found by its TMUX_TMPDIR")
        finally:
            lone.kill()
            lone.wait()
        # And a process that only mentions the name is not one.
        talker = subprocess.Popen([python, "-c", "import time; time.sleep(60)", "tail", "/x/farcoolerd.log"],
                                  env={**os.environ, "TMUX_TMPDIR": other})
        try:
            if daemon_pids(other):
                failures.append("a process that only names farcoolerd was taken for one")
        finally:
            talker.kill()
            talker.wait()
            shutil.rmtree(fakes, ignore_errors=True)
    finally:
        end([p for p in pids if alive(p)])
        end(list(bystanders))
        for base in BASES:
            shutil.rmtree(base, ignore_errors=True)
        shutil.rmtree(other, ignore_errors=True)
    if failures:
        for f in failures:
            print(f"tmux-leak-check: self-test FAILED, {f}", file=sys.stderr)
        return 1
    print("tmux-leak-check: self-test ok")
    return 0


if __name__ == "__main__":
    args = sys.argv[1:]
    if args == ["--self-test"]:
        sys.exit(self_test())
    if args[:1] == ["--"] and len(args) > 1:
        sys.exit(run(args[1:], kill=bool(os.environ.get("CI")))[0])
    print(__doc__)
    sys.exit(2)
