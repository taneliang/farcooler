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
import errno
import fcntl
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import time

LOCK_POLL = 5  # seconds between a waiter's tries
LOAD_LIMIT = 20.0
HELD_ENV = "HEAVY_LOCK_HELD"  # set in the child of scripts/heavy: nested takers do not queue behind their own parent


class _Survivor:
    """A text stream that goes quiet instead of raising when its far end is gone.

    A waiting gate prints "waiting for the lock" lines for as long as it is queued, which can be hours. If
    the reader has gone (a closed pipe, a hung-up terminal, a log on a full disk), the next print raises
    BrokenPipeError, EIO or ENOSPC inside the queue loop; the gate dies, loses its place, and Python exits 120
    with a traceback nobody sees (ov-839). Output is a courtesy; the wait is the job. On the first failure the
    descriptor is pointed at /dev/null, so the later flush at exit succeeds too."""

    def __init__(self, stream, fd):
        self._stream, self._fd, self.dead = stream, fd, False

    def _quiet(self):
        self.dead = True
        try:
            null = os.open(os.devnull, os.O_WRONLY)
            os.dup2(null, self._fd)
            os.close(null)
        except OSError:
            pass

    def write(self, text):
        if not self.dead:
            try:
                return self._stream.write(text)
            except (OSError, ValueError):
                self._quiet()
        return len(text)

    def flush(self):
        if not self.dead:
            try:
                self._stream.flush()
            except (OSError, ValueError):
                self._quiet()

    def __getattr__(self, name):
        return getattr(self._stream, name)


def survive_dead_output():
    """Make stdout and stderr survive their reader going away. A stream that is closed or not a file is left alone."""
    for name, fd in (("stdout", 1), ("stderr", 2)):
        stream = getattr(sys, name)
        try:
            if stream is not None and stream.fileno() == fd and not isinstance(stream, _Survivor):
                setattr(sys, name, _Survivor(stream, fd))
        except (OSError, ValueError, AttributeError):
            pass  # no descriptor behind it (a StringIO, or closed): nothing to protect


def disk_full(e):
    """True for an error that means the volume has no room."""
    return isinstance(e, OSError) and e.errno in (errno.ENOSPC, errno.EDQUOT)


# ---- a lane's Rust target/ stays small (ov-837)
#
# Each lane worktree grew its own target/, 10-84 GB apiece (about 230 GB across five lanes on Oct 27), and the disk
# filled. A shared CARGO_TARGET_DIR was measured and rejected: each checkout still builds its own copies of the
# workspace crates, so two lanes shared only the registry dependencies (28.3 GB for two lanes after one edit each, against
# about 30 GB apart), and cargo's mtime check let one lane's build stand in for another's older checkout: B's
# transport crate failed with E0063 against the protocol crate built for A. sccache cannot cache incremental
# builds and leaves every lane its own target/. What does work, per lane worktree:
#   1. no debug info in the dev and test profiles: 12.6 GB to 7.0 GB for daemon+client, cold build 212 s to 132 s,
#      an edit rebuild unchanged at 22 s. (Incremental caches stay: dropping them halves the size again but makes an
#      edit rebuild 2.4 times slower.)
#   2. a ceiling: over FARCOOLER_TARGET_MAX_GB (20) the incremental caches go, then the whole target/.
#   3. scripts/lane-gate.py removes target/ after a fully green gate, when the lane is done.
# Lanes only (a linked worktree): the main checkout and CI build as before. Opt out with FARCOOLER_LEAN_BUILD=off.

LEAN_ENV = "FARCOOLER_LEAN_BUILD"
TARGET_MAX_ENV = "FARCOOLER_TARGET_MAX_GB"
TARGET_MAX_GB = 20.0


def is_linked_worktree(top):
    """True in a `git worktree add` checkout, false in the main checkout."""
    def git(flag):
        out = subprocess.run(["git", "-C", str(top), "rev-parse", flag], capture_output=True, text=True).stdout.strip()
        return (pathlib.Path(top) / out).resolve() if out else None
    git_dir, common = git("--git-dir"), git("--git-common-dir")
    return bool(git_dir and common and git_dir != common)


def lean_wanted(top):
    return not (os.environ.get("CI") or os.environ.get(LEAN_ENV) == "off") and is_linked_worktree(top)


def lean_cargo_env(top):
    """Environment that keeps a lane's target/ small: no debug info unless the caller chose a level. {} outside a lane."""
    if not lean_wanted(top) or "CARGO_PROFILE_DEV_DEBUG" in os.environ:
        return {}
    return {"CARGO_PROFILE_DEV_DEBUG": "0"}


def runs_cargo(cmd):
    return "cargo" in [os.path.basename(a) for a in cmd]


def target_kb(target):
    out = subprocess.run(["du", "-sk", str(target)], capture_output=True, text=True).stdout.split()
    return int(out[0]) if out else 0


def cargo_is_building(target):
    """True when a cargo process holds target/debug/.cargo-lock (a build outside scripts/heavy, say)."""
    for lock in pathlib.Path(target).glob("*/.cargo-lock"):
        try:
            with open(lock, "a+") as f:
                try:
                    fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError:
                    return True
        except OSError:
            continue
    return False


def trim_target(target, say=print):
    """Over the ceiling: drop the incremental caches, then everything. Call with the machine lock held."""
    target = pathlib.Path(target)
    try:
        cap = float(os.environ.get(TARGET_MAX_ENV, TARGET_MAX_GB))
    except ValueError:
        say(f"  {TARGET_MAX_ENV}={os.environ[TARGET_MAX_ENV]!r} is not a number; using {TARGET_MAX_GB:g} GB")
        cap = TARGET_MAX_GB
    limit = cap * 1024 * 1024  # KB
    if not target.is_dir() or target_kb(target) <= limit:
        return
    if cargo_is_building(target):
        say(f"  {target} is over {cap:g} GB, but cargo is building in it right now; not clearing it")
        return
    for inc in target.glob("*/incremental"):
        shutil.rmtree(inc, ignore_errors=True)
    if target_kb(target) > limit:
        say(f"  {target} is over {limit / 1024 / 1024:g} GB; clearing it (the next build starts cold)")
        shutil.rmtree(target, ignore_errors=True)


def prepare_lane_build(top, say=print):
    """What a lane's cargo command gets: the lean environment, after the ceiling is enforced."""
    env = lean_cargo_env(top)
    if lean_wanted(top):
        trim_target(pathlib.Path(top) / "target", say)
    return env


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
    if os.environ.get(HELD_ENV):  # an ancestor (scripts/heavy) already holds the place; waiting on ourselves would deadlock
        yield
        return
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

        # Nested: a command run by scripts/heavy that itself takes the lock (train-gate.sh running lane-gate) does not wait on its parent.
        out = subprocess.run([heavy, "--", sys.executable, str(here / "heavy"), "--", "echo", "nested-ok"], env=env, capture_output=True, text=True, timeout=10).stdout
        expect("a nested wrapper does not deadlock behind its parent", out.strip(), "nested-ok")

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

        # A waiter whose reader goes away (a closed pipe, a hung-up terminal) keeps its place and runs when its turn
        # comes (ov-839). The control run leaves out survive_dead_output() and must die, so this check can fail.
        for protected in (True, False):
            marks.write_text("")
            driver = ("import sys; sys.path.insert(0, %r); import heavy_lock as hl\n%s"
                      "with hl.machine_lock('/wt/gate', 'gate: x', poll=0.05, announce=0.05):\n"
                      "    open(%r, 'a').write('gate-in\\n')\n") % (str(here), "hl.survive_dead_output()\n" if protected else "", str(marks))
            holder = subprocess.Popen([heavy, "--", "sh", "-c", f'echo wrapper-in >> "{marks}"; sleep 1.5; echo wrapper-out >> "{marks}"'],
                                      env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            deadline = time.time() + 5
            while "wrapper-in" not in marks.read_text() and time.time() < deadline:
                time.sleep(0.02)
            w = subprocess.Popen([sys.executable, "-c", driver], env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
            w.stdout.readline()  # it is waiting and has said so
            w.stdout.close()     # ... and now nobody is listening
            rc = w.wait(timeout=10)
            holder.wait()
            if protected:
                expect("a waiter whose output pipe closes still takes its turn and exits 0", (rc, marks.read_text().split()), (0, ["wrapper-in", "wrapper-out", "gate-in"]))
            else:
                expect("control: without survive_dead_output a closed pipe kills the waiter", rc != 0 and "gate-in" not in marks.read_text(), True)
        # scripts/heavy waits the same way: its wait lines go to a closed pipe and the command still runs.
        marks.write_text("")
        holder = subprocess.Popen([heavy, "--", "sh", "-c", f'echo wrapper-in >> "{marks}"; sleep 1.5; echo wrapper-out >> "{marks}"'],
                                  env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = time.time() + 5
        while "wrapper-in" not in marks.read_text() and time.time() < deadline:
            time.sleep(0.02)
        w = subprocess.Popen([heavy, "--", "sh", "-c", f'echo second-in >> "{marks}"'], env={**env, "HEAVY_ANNOUNCE": "0.05"},
                             stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        w.stderr.readline()
        w.stderr.close()
        rc = w.wait(timeout=10)
        holder.wait()
        expect("scripts/heavy with its stderr pipe closed still runs its command", (rc, marks.read_text().split()), (0, ["wrapper-in", "wrapper-out", "second-in"]))

        # A lane's Rust target/ stays small (ov-837): linked worktrees build without debug info and under a ceiling; the main checkout does not.
        def git(*a, cwd):
            return subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *a], cwd=cwd, capture_output=True, text=True, check=True)
        main_wt, lane_wt = t / "main", t / "lane"
        main_wt.mkdir()
        git("init", "-q", cwd=main_wt)
        git("commit", "-q", "--allow-empty", "-m", "x", cwd=main_wt)
        git("worktree", "add", "-q", "--detach", str(lane_wt), cwd=main_wt)
        knobs = ("CI", "CARGO_PROFILE_DEV_DEBUG", LEAN_ENV, TARGET_MAX_ENV)
        saved = {k: os.environ.pop(k, None) for k in knobs}
        lean = {"CARGO_PROFILE_DEV_DEBUG": "0"}
        try:
            expect("the main checkout builds as before", lean_cargo_env(main_wt), {})
            expect("a lane worktree builds without debug info", lean_cargo_env(lane_wt), lean)
            os.environ["CARGO_PROFILE_DEV_DEBUG"] = "1"
            expect("a debug level the caller chose wins", lean_cargo_env(lane_wt), {})
            del os.environ["CARGO_PROFILE_DEV_DEBUG"]
            os.environ["CI"] = "1"
            expect("CI builds as before", lean_cargo_env(lane_wt), {})
            del os.environ["CI"]
            os.environ[LEAN_ENV] = "off"
            expect("off opts out", lean_cargo_env(lane_wt), {})
            del os.environ[LEAN_ENV]
            expect("only cargo commands are lean", [runs_cargo(c) for c in (["cargo", "test"], ["./scripts/test-leak-check.py", "--", "cargo", "nextest"], ["swift", "test"], ["apps/macos/test.sh"])], [True, True, False, False])

            # Through scripts/heavy with a fake cargo and swift that print what they were given.
            bindir = t / "bin"
            bindir.mkdir()
            for tool in ("cargo", "swift"):
                (bindir / tool).write_text('#!/bin/sh\necho "debug=${CARGO_PROFILE_DEV_DEBUG:-none}"\n')
                (bindir / tool).chmod(0o755)
            e3 = {**env, "PATH": f"{bindir}{os.pathsep}{os.environ['PATH']}"}
            def via_heavy(wt, *cmd, extra=(), more=None):
                return subprocess.run([heavy, *extra, "--", *cmd], cwd=wt, env={**e3, **(more or {})}, capture_output=True, text=True, timeout=20).stdout.strip()
            expect("scripts/heavy gives a lane's cargo the lean build", via_heavy(lane_wt, "cargo", "build"), "debug=0")
            expect("... not the main checkout's", via_heavy(main_wt, "cargo", "build"), "debug=none")
            expect("... not a swift build", via_heavy(lane_wt, "swift", "test"), "debug=none")

            # The ceiling: over it, incremental caches go first, then the whole target/. The wrapper enforces it before the command runs.
            tgt = lane_wt / "target"
            (tgt / "debug" / "incremental").mkdir(parents=True)
            (tgt / "debug" / "deps").mkdir()
            (tgt / "debug" / "incremental" / "big").write_bytes(b"x" * 2_000_000)
            (tgt / "debug" / "deps" / "small").write_bytes(b"x" * 100_000)
            os.environ[TARGET_MAX_ENV] = "0.001"  # about 1 MB
            trim_target(tgt, say=lambda *a, **k: None)
            expect("over the ceiling, incremental caches go and the dependencies stay", ((tgt / "debug" / "incremental").exists(), (tgt / "debug" / "deps" / "small").exists()), (False, True))
            (tgt / "debug" / "deps" / "big").write_bytes(b"x" * 2_000_000)
            via_heavy(lane_wt, "cargo", "build", more={TARGET_MAX_ENV: "0.001"})
            expect("still over, scripts/heavy clears the whole target/ before it builds", tgt.exists(), False)
            tgt.mkdir(exist_ok=True)
            (tgt / "keep").write_bytes(b"x")
            via_heavy(lane_wt, "cargo", "build", more={TARGET_MAX_ENV: "0.001"})
            expect("under the ceiling, nothing is touched", (tgt / "keep").exists(), True)
            # Cargo holds target/debug/.cargo-lock while it builds: the trim must wait for another day, and say why.
            (tgt / "debug").mkdir(parents=True, exist_ok=True)
            (tgt / "debug" / "big").write_bytes(b"x" * 2_000_000)
            holder = subprocess.Popen([sys.executable, "-c", "import fcntl,sys,time; f=open(sys.argv[1],'a+'); fcntl.flock(f, fcntl.LOCK_EX); print('held', flush=True); time.sleep(30)", str(tgt / "debug" / ".cargo-lock")], stdout=subprocess.PIPE, text=True)
            try:
                holder.stdout.readline()
                told = []
                trim_target(tgt, say=told.append)
                expect("a target/ cargo holds is not trimmed, and the line says why", ((tgt / "debug" / "big").exists(), bool(told) and "cargo is building" in told[0]), (True, True))
            finally:
                holder.kill()
                holder.wait()
            trim_target(tgt, say=lambda *a, **k: None)
            expect("... and once cargo lets go it is trimmed", tgt.exists(), False)
            # A cap that is not a number falls back to 20 GB with a warning, not a traceback.
            tgt.mkdir()
            (tgt / "keep").write_bytes(b"x")
            os.environ[TARGET_MAX_ENV] = "lots"
            told = []
            try:
                trim_target(tgt, say=told.append)
                err = None
            except ValueError as e:
                err = e
            expect("a non-numeric cap warns and keeps the default", (err, bool(told) and "not a number" in told[0], (tgt / "keep").exists()), (None, True, True))
            os.environ.pop(TARGET_MAX_ENV)
            shutil.rmtree(tgt)
            main_tgt = main_wt / "target"
            main_tgt.mkdir()
            (main_tgt / "big").write_bytes(b"x" * 2_000_000)
            via_heavy(main_wt, "cargo", "build", more={TARGET_MAX_ENV: "0.001"})
            expect("the main checkout's target/ is never trimmed", (main_tgt / "big").exists(), True)
        finally:
            for k, v in saved.items():
                os.environ.pop(k, None)
                if v is not None:
                    os.environ[k] = v
    if fails:
        for line in fails:
            print(f"heavy_lock self-test: {line}", file=sys.stderr)
        return 1
    print("heavy_lock self-test: ok")
    return 0


if __name__ == "__main__":
    sys.exit(self_test() if sys.argv[1:] == ["--self-test"] else 2)
