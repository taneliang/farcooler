#!/usr/bin/env python3
"""The machine-wide heavy-job lock, shared by scripts/lane-gate.py and scripts/heavy (ov-831, ov-836).

Several lanes share one Mac. Two builds or test suites at once push the load past
100 and make timing tests fail for no reason, so anything heavy takes the lock on
lane-gate.lock in the temp dir first: a first-come, first-served queue of ticket files in
lane-gate.lock.queue/. The oldest live ticket runs; a waiter says who holds it (worktree,
PID, start time, what it runs) and how many are ahead. A holder that dies loses its place.

    scripts/heavy_lock.py --self-test

Locking is off under CI (separate runners) and with --no-lock.
"""
import contextlib
import fcntl
import os
import pathlib
import subprocess
import sys
import tempfile
import time

LOCK_POLL = 5  # seconds between a waiter's tries
LOAD_LIMIT = 20.0


def lock_path():
    return pathlib.Path(tempfile.gettempdir()) / "lane-gate.lock"


def lock_wanted(no_lock=False):
    """Locking is on unless --no-lock, or CI is set (separate runners there)."""
    return not (no_lock or os.environ.get("CI"))


def load1():
    return os.getloadavg()[0]


def load5():
    return os.getloadavg()[1]


def wait_for_load(limit=LOAD_LIMIT, poll=30, load=load1, say=print):
    """Wait while `load()` is at or above the limit; say so now and every five minutes."""
    started = time.time()
    announced = 0
    while load() >= limit:
        if time.time() - started >= announced * 300:
            say(f"  waiting: load average {load():.1f} is {limit:.0f} or more", flush=True)
            announced += 1
        time.sleep(poll)


def queue_dir(path):
    return path.with_name(path.name + ".queue")


def pid_alive(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def live_tickets(qd):
    """The queue, oldest first: (ticket path, text). A ticket whose process is gone is deleted."""
    out = []
    for t in sorted(qd.glob("*.ticket")):
        try:
            pid = int(t.name.split(".")[0].split("-")[1])
            if not pid_alive(pid):
                t.unlink(missing_ok=True)
                continue
            out.append((t, t.read_text().strip()))
        except (OSError, ValueError, IndexError):
            continue
    return out


def read_holder(path):
    """Who is at the head of the queue: the one running now."""
    held = live_tickets(queue_dir(path))
    return held[0][1] if held else "an unknown holder"


@contextlib.contextmanager
def machine_lock(worktree, job, path=None, poll=LOCK_POLL, announce=60, say=print):
    """Wait in a first-come, first-served queue for the machine-wide heavy-job lock; hold it for the block.

    A ticket file named by arrival time and PID is the place in line; the oldest ticket whose
    process is alive runs, and a ticket left by a dead process is deleted. A flock is not a
    queue (the releaser retakes it before a waiter polls), which is why this is tickets.
    A waiter names the holder and how many are ahead.
    """
    path = path or lock_path()
    qd = queue_dir(path)
    qd.mkdir(parents=True, exist_ok=True)
    text = f"worktree {worktree}, PID {os.getpid()}, started {time.strftime('%Y-%m-%d %H:%M:%S')}, running {job}\n"
    with open(qd / ".enter", "a+") as enter:  # taking a number is one step at a time, so numbers match arrival
        fcntl.flock(enter, fcntl.LOCK_EX)
        ticket = qd / f"{time.time_ns():020d}-{os.getpid()}.ticket"
        tmp = qd / f".new-{os.getpid()}-{time.time_ns()}"
        tmp.write_text(text)
        os.rename(tmp, ticket)
    started = time.time()
    announced = -1
    try:
        while True:
            queue = live_tickets(qd)
            if not queue or queue[0][0] == ticket:
                break
            if time.time() - started >= (announced + 1) * announce:
                announced += 1
                ahead = [i for i, (t, _) in enumerate(queue) if t == ticket]
                say(f"  waiting for the machine-wide heavy-job lock ({job}); held by {queue[0][1]} ({ahead[0] if ahead else len(queue)} in line ahead of you)", flush=True)
            time.sleep(poll)
        yield
    finally:
        ticket.unlink(missing_ok=True)


def self_test():
    import re
    import threading

    fails = []

    def expect(what, got, want):
        if got != want:
            fails.append(f"{what}: got {got!r}, want {want!r}")

    here = pathlib.Path(__file__).resolve().parent
    heavy = str(here / "heavy")
    with tempfile.TemporaryDirectory() as t:
        t = pathlib.Path(t)
        lock = t / "lane-gate.lock"
        env = {**os.environ, "TMPDIR": str(t), "HEAVY_POLL": "0.05", "HEAVY_LOAD_LIMIT": "100000"}
        env.pop("CI", None)

        # In-process: a second taker waits, names the first, and the note goes with the lock.
        said, order = [], []
        held, go = threading.Event(), threading.Event()

        def first():
            with machine_lock("/wt/first", "rust", path=lock, poll=0.05):
                order.append("first in")
                held.set()
                go.wait(5)
                order.append("first out")

        def second():
            with machine_lock("/wt/second", "swift", path=lock, poll=0.05, announce=0.1, say=lambda m, **k: said.append(m)):
                order.append("second in")

        a = threading.Thread(target=first)
        a.start()
        held.wait(5)
        b = threading.Thread(target=second)
        b.start()
        time.sleep(0.4)
        expect("the second taker has not started while the first holds the lock", order, ["first in"])
        go.set()
        a.join(5)
        b.join(5)
        expect("the second taker ran after the first let go", order, ["first in", "first out", "second in"])
        expect("the waiter names the holder's worktree, PID and start time",
               bool(said) and "/wt/first" in said[0] and f"PID {os.getpid()}" in said[0]
               and re.search(r"started \d{4}-\d\d-\d\d \d\d:\d\d:\d\d", said[0]) is not None, True)
        expect("the ticket goes when the lock does", list(queue_dir(lock).glob("*.ticket")), [])
        expect("CI turns the lock off", (os.environ.update(CI="1"), lock_wanted())[1], False)
        os.environ.pop("CI")
        expect("--no-lock turns the lock off", lock_wanted(True), False)
        expect("otherwise it is on", lock_wanted(), True)

        # First come, first served: five takers arrive 0.1s apart while the first holds; they run in arrival order.
        # (A bare flock would let whichever polled first win.)
        order.clear()
        gate = threading.Event()
        def taker(n):
            with machine_lock(f"/wt/{n}", f"job{n}", path=lock, poll=0.01, say=lambda *a, **k: None):
                order.append(n)
                if n == 0:
                    gate.wait(5)
                time.sleep(0.02)
        ths = []
        for n in range(5):
            th = threading.Thread(target=taker, args=(n,))
            th.start()
            ths.append(th)
            time.sleep(0.1)
        gate.set()
        for th in ths:
            th.join(5)
        expect("takers run in arrival order", order, [0, 1, 2, 3, 4])

        # A ticket left by a dead process is skipped (and cleaned up), not waited on.
        dead = subprocess.Popen([sys.executable, "-c", "pass"])
        dead.wait()
        qd = queue_dir(lock)
        (qd / f"{1:020d}-{dead.pid}.ticket").write_text("worktree /wt/dead, PID gone\n")
        got = []
        th = threading.Thread(target=lambda: [got.append("ran") for _ in [machine_lock("/wt/live", "x", path=lock, poll=0.01, say=print).__enter__()]])
        th.start()
        th.join(2)
        expect("a dead holder's ticket is skipped", (got, list(qd.glob(f"*-{dead.pid}.ticket"))), (["ran"], []))
        for t in qd.glob("*.ticket"):
            t.unlink()

        # Through the real wrapper: the wrapper holds the lock, so an in-process taker waits and names it.
        marks = t / "marks"
        p = subprocess.Popen([heavy, "--", "sh", "-c", f'echo wrapper-in >> "{marks}"; sleep 1; echo wrapper-out >> "{marks}"; exit 7'],
                             env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        deadline = time.time() + 5
        while not marks.exists() and time.time() < deadline:
            time.sleep(0.02)
        said2 = []
        with machine_lock("/wt/gate", "lane-gate job", path=lock, poll=0.05, announce=0.1, say=lambda m, **k: said2.append(m)):
            with open(marks, "a") as m:
                m.write("gate-in\n")
        out = p.communicate()[0]
        expect("lane-gate's lock waits for scripts/heavy", marks.read_text().split(), ["wrapper-in", "wrapper-out", "gate-in"])
        expect("... and names the wrapper's command", bool(said2) and "sh -c" in said2[0] and f"PID {p.pid}" in said2[0], True)
        expect("the wrapper passes the command's exit status through", p.returncode, 7)

        # Two wrappers at once: the second prints who holds the lock, and they never overlap.
        marks.write_text("")
        cmd = lambda n: [heavy, "--", "sh", "-c", f'echo {n}-in >> "{marks}"; sleep 0.6; echo {n}-out >> "{marks}"']
        env2 = {**env, "HEAVY_ANNOUNCE": "0.1"}
        p1 = subprocess.Popen(cmd("one"), env=env2, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        time.sleep(0.3)
        p2 = subprocess.Popen(cmd("two"), env=env2, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        o1, o2 = p1.communicate()[0], p2.communicate()[0]
        expect("two wrappers do not overlap", marks.read_text().split(), ["one-in", "one-out", "two-in", "two-out"])
        expect("the waiting wrapper names the holder", "held by worktree" in o2 and f"PID {p1.pid}" in o2, True)
        expect("the holder says nothing of waiting", "waiting" in o1, False)

        # --no-lock and CI let them overlap.
        for how, e, extra in (("--no-lock", env2, ["--no-lock"]), ("CI", {**env2, "CI": "true"}, [])):
            marks.write_text("")
            c = lambda n: [heavy, *extra, "--", "sh", "-c", f'echo {n}-in >> "{marks}"; sleep 0.6; echo {n}-out >> "{marks}"']
            p1 = subprocess.Popen(c("one"), env=e)
            time.sleep(0.3)
            p2 = subprocess.Popen(c("two"), env=e)
            p1.wait()
            p2.wait()
            expect(f"{how} lets two wrappers overlap", marks.read_text().split()[:2], ["one-in", "two-in"])

        # The wrapper waits for load: a limit of 0 never clears, so the command must not have started.
        marks.write_text("")
        p = subprocess.Popen([heavy, "--load-limit", "0", "--", "sh", "-c", f'echo ran >> "{marks}"'], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        time.sleep(0.6)
        still_waiting = p.poll() is None
        p.terminate()
        o = p.communicate()[0]
        expect("the wrapper waits while the load is over the limit", (still_waiting, marks.read_text()), (True, ""))
        expect("... and says so", "load average" in o, True)
    if fails:
        for line in fails:
            print(f"heavy_lock self-test: {line}", file=sys.stderr)
        return 1
    print("heavy_lock self-test: ok")
    return 0


if __name__ == "__main__":
    sys.exit(self_test() if sys.argv[1:] == ["--self-test"] else 2)
