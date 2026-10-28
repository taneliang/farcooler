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

# ---- light runs (ov-843)
#
# A reviewer's two-minute filtered test waited an hour behind whole lane-gates. `scripts/heavy --light -- <command>`
# skips the gate queue and takes one of LIGHT_SLOTS (2) light slots instead, still waiting for the load to be under the
# limit. It is an explicit flag, not detection: guessing "this looks short" from a command line fails open (a missed
# spelling slips a whole suite past the queue), while a flag the caller must type fails closed. The flag is also
# checked: light_refusal() accepts only a recognized filtered shape (cargo test or nextest with -p and a filter, vitest
# run with a file, swift test --filter) in a checkout whose target/ is already built (a cold build is not light), and
# says why when it refuses. Full gates and everything else stay one at a time in the queue.

LIGHT_SLOTS = 2
LIGHT_SLOTS_ENV = "HEAVY_LIGHT_SLOTS"
_CARGO_VALUE_FLAGS = {"-p", "--package", "-E", "--filter-expr", "--features", "-F", "--profile", "--cargo-profile", "-P", "--target",
                      "--test", "--bin", "--example", "--bench", "--manifest-path", "-j", "--jobs", "--exclude", "--target-dir",
                      "--config", "-Z", "--color", "--message-format", "--run-ignored", "--partition", "--test-threads",
                      "--retries", "--max-fail", "-T", "--threads", "--build-jobs"}
_WHOLE_FLAGS = {"--workspace", "--all"}


def _positionals(args):
    """Non-flag arguments of a cargo test or nextest command line, before and after `--`."""
    before, after, i, dashed = [], [], 0, False
    while i < len(args):
        a = args[i]
        if dashed:
            if not a.startswith("-"):
                after.append(a)
        elif a == "--":
            dashed = True
        elif a.startswith("-"):
            if a in _CARGO_VALUE_FLAGS:
                i += 1
        else:
            before.append(a)
        i += 1
    return before, after


def _manifest_arg(args):
    """The --manifest-path value of a cargo command line (either spelling), or None. Stops at `--`."""
    for i, a in enumerate(args):
        if a == "--":
            return None
        if a == "--manifest-path":
            return args[i + 1] if i + 1 < len(args) else ""
        if a.startswith("--manifest-path="):
            return a.split("=", 1)[1]
    return None


def _has_table(manifest, table):
    try:
        return any(line.strip() == table for line in pathlib.Path(manifest).read_text().splitlines())
    except OSError:
        return False


def _manifest_target(manifest, top):
    """Where cargo puts target/ for `manifest` (ov-847): the target/ of the nearest workspace root at or above the
    crate's directory, up to the repo top, else the crate's own. Crates/web is a workspace of its own."""
    d = manifest.parent
    while True:
        if _has_table(d / "Cargo.toml", "[workspace]"):
            return d / "target"
        if d == top or d.parent == d:
            return manifest.parent / "target"
        d = d.parent


def light_refusal(cmd, top=None):
    """None when `cmd` may run as a light run, else the reason it may not (a plain sentence)."""
    tool = os.path.basename(cmd[0]) if cmd else ""
    if tool not in ("cargo", "vitest", "swift", "npx"):
        return "only a direct cargo, vitest or swift test command can be light; anything else (a script, sh -c) queues"
    rest = list(cmd[1:])
    if tool == "cargo":
        if rest[:1] and rest[0].startswith("+"):
            rest = rest[1:]
        if rest[:2] in (["nextest", "run"], ["nextest", "r"]):
            rest, nextest = rest[2:], True
        elif rest[:1] in (["test"], ["t"]):
            rest, nextest = rest[1:], False
        else:
            return "light cargo is `cargo test` or `cargo nextest run`; builds, clippy and the rest queue"
        if any(a in _WHOLE_FLAGS or a == "--exclude" or a.startswith("--exclude=") for a in rest):
            return "--workspace, --all and --exclude mean a whole-workspace run; it queues"
        manifest = _manifest_arg(rest)
        target = None
        if manifest is not None:
            # A crate outside the root workspace (crates/web) is named by its manifest, not by -p (ov-847). The path must
            # resolve (symlinks followed) to a Cargo.toml under the repo, and name a package, not a virtual workspace.
            if top is None:
                return "--manifest-path needs the repo top-level to check it against"
            real_top = pathlib.Path(top).resolve()
            mp = pathlib.Path(manifest).resolve()
            if not manifest or mp.name != "Cargo.toml" or not mp.is_file():
                return f"--manifest-path {manifest or '(empty)'} is not a Cargo.toml that exists; it queues"
            if real_top != mp and real_top not in mp.parents:
                return f"--manifest-path {manifest} is outside this repo; it queues"
            if not _has_table(mp, "[package]"):
                return f"--manifest-path {manifest} is a virtual workspace, which would build every member; it queues"
            target = _manifest_target(mp, real_top)
        elif not any(a in ("-p", "--package") or a.startswith("--package=") for a in rest):
            return "light cargo needs -p <crate> or --manifest-path <crate>/Cargo.toml; without them cargo may build the whole workspace"
        before, after = _positionals(rest)
        if not (before or after or (nextest and any(a in ("-E", "--filter-expr") or a.startswith("--filter-expr=") for a in rest))):
            return "light cargo needs a test-name filter (or nextest -E); a whole crate's suite queues"
        if top is not None and not ((target or pathlib.Path(top) / "target") / "debug").is_dir():
            return "target/debug is not built here, so this would be a cold build; it queues"
        return None
    if tool == "swift":
        if rest[:1] != ["test"] or not any(a == "--filter" or a.startswith("--filter=") for a in rest):
            return "light swift is `swift test --filter <name>`"
        return None
    if tool == "npx":
        if rest[:1] != ["vitest"]:
            return "light npx is `npx vitest run <file>`"
        rest = rest[1:]
    if rest[:1] != ["run"] or not [a for a in rest[1:] if not a.startswith("-")]:
        return "light vitest is `vitest run <file or name filter>`"
    return None


def light_dir(path=None):
    path = path or lock_path()
    return path.with_name(path.name + ".light")


@contextlib.contextmanager
def light_slot(job, path=None, slots=None, limit=LOAD_LIMIT, load=load5, poll=LOCK_POLL, announce=60, say=print):
    """Hold one of `slots` light slots (default 2) for the block, taking it only while load() is under `limit`.

    The count and the load are checked together under one flock, so two arrivals cannot both take the last slot.
    A slot whose process died is dropped. The gate queue is never consulted."""
    d = light_dir(path)
    d.mkdir(parents=True, exist_ok=True)
    slots = int(os.environ.get(LIGHT_SLOTS_ENV, LIGHT_SLOTS)) if slots is None else slots
    mine = d / f"{os.getpid()}-{time.time_ns()}.slot"
    started, announced = time.time(), -1
    while True:
        why = None
        with open(d / ".enter", "a+") as enter:
            fcntl.flock(enter, fcntl.LOCK_EX)
            live = []
            for s in sorted(d.glob("*.slot")):
                if pid_alive(int(s.name.split("-")[0])):
                    live.append(s)
                else:
                    s.unlink(missing_ok=True)
            if len(live) >= slots:
                why = f"all {slots} light slots are in use"
            elif load() >= limit:
                why = f"5-minute load average {load():.1f} is {limit:.0f} or more"
            else:
                mine.write_text(f"PID {os.getpid()}, running {job}\n")
                break
        if time.time() - started >= (announced + 1) * announce:
            announced += 1
            say(f"  light run waiting: {why} ({job})", flush=True)
        time.sleep(poll)
    try:
        yield
    finally:
        mine.unlink(missing_ok=True)


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

        # Light runs (ov-843): a filtered run goes while a full gate holds the lock; the gate queue itself stays one at a time.
        # The load is fixed by HEAVY_LOAD_LIMIT (100000 = never busy, 0 = always busy); the slot count by HEAVY_LIGHT_SLOTS.
        lenv = {**env, "HEAVY_LIGHT_SLOTS": "2", "HEAVY_ANNOUNCE": "0.1"}
        lt = t / "lightwt"
        (lt / "target" / "debug").mkdir(parents=True)
        bind = t / "lbin"
        bind.mkdir()
        (bind / "cargo").write_text(f'#!/bin/sh\necho "$$ $*" >> "{t}/light-marks"\nsleep "${{LIGHT_SLEEP:-0}}"\n')
        (bind / "cargo").chmod(0o755)
        lenv["PATH"] = f"{bind}{os.pathsep}{os.environ['PATH']}"
        lcmd = ["cargo", "test", "-p", "x", "name_filter"]

        def start_light(more=None, cmd=lcmd, **kw):
            return subprocess.Popen([heavy, "--light", "--", *cmd], cwd=lt, env={**lenv, **(more or {})}, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, **kw)

        def light_ran():
            f = t / "light-marks"
            return len(f.read_text().split("\n")) - 1 if f.exists() else 0

        # 1. a light run starts while a (fake) full gate holds the lock
        (t / "light-marks").unlink(missing_ok=True)
        marks.write_text("")
        gate = subprocess.Popen([heavy, "--", "sh", "-c", f'echo gate-in >> "{marks}"; while [ ! -e "{t}/release" ]; do sleep 0.05; done; echo gate-out >> "{marks}"'], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = time.time() + 5
        while "gate-in" not in marks.read_text() and time.time() < deadline:
            time.sleep(0.02)
        lp = start_light()
        try:
            lo = lp.communicate(timeout=10)[0]
        except subprocess.TimeoutExpired:  # it queued behind the gate: that is the failure this check exists for
            lp.kill()
            lo = lp.communicate()[0]
        expect("a light run starts and finishes while a full gate holds the lock", (lp.returncode, light_ran(), "gate-out" in marks.read_text()), (0, 1, False))
        (t / "release").write_text("")
        gate.wait()

        # 2. two full gates never overlap (the wrappers' check above), and a light run never takes a gate's place in the queue
        expect("a light run leaves no gate ticket and no slot behind", (list(queue_dir(lock).glob("*.ticket")), list(light_dir(lock).glob("*.slot"))), ([], []))

        # 3. a light run waits when the load is at the limit, and when both slots are taken
        (t / "light-marks").unlink(missing_ok=True)
        lp = start_light(more={"HEAVY_LOAD_LIMIT": "0"})
        time.sleep(0.6)
        waiting = (lp.poll() is None, light_ran())
        lp.terminate()
        lo = lp.communicate()[0]
        expect("a light run waits while the load is at the limit", waiting, (True, 0))
        expect("... and says so", "light run waiting" in lo and "load average" in lo, True)
        (t / "light-marks").unlink(missing_ok=True)
        long_ones = [start_light(more={"LIGHT_SLEEP": "2"}) for _ in range(2)]
        deadline = time.time() + 5
        while light_ran() < 2 and time.time() < deadline:
            time.sleep(0.02)
        third = start_light()
        time.sleep(0.6)
        waiting = (third.poll() is None, light_ran())
        for p in long_ones:
            p.communicate(timeout=10)
        to = third.communicate(timeout=10)[0]
        expect("a third light run waits while both slots are taken, then runs", (waiting, third.returncode, light_ran()), ((True, 2), 0, 3))
        expect("... and says the slots are full", "light slots are in use" in to, True)

        # A light flag never carries a whole-suite or unrecognized command past the queue.
        shapes = {
            "filtered cargo test": (["cargo", "test", "-p", "x", "f"], True), "cargo test with a filter after --": (["cargo", "test", "-p", "x", "--", "f"], True),
            "nextest with a filter": (["cargo", "nextest", "run", "-p", "x", "f"], True), "nextest -E": (["cargo", "nextest", "run", "-p", "x", "-E", "test(f)"], True),
            "toolchain": (["cargo", "+stable", "test", "-p", "x", "f"], True), "vitest file": (["vitest", "run", "a.test.ts"], True),
            "swift filter": (["swift", "test", "--filter", "A.b"], True),
            "workspace": (["cargo", "test", "--workspace", "-p", "x", "f"], False), "no -p": (["cargo", "test", "f"], False),
            "no filter": (["cargo", "test", "-p", "x"], False), "-p value is not a filter": (["cargo", "nextest", "run", "-p", "x", "--profile", "ci"], False),
            "build": (["cargo", "build", "-p", "x", "f"], False), "bare vitest": (["vitest", "run"], False), "bare swift test": (["swift", "test"], False),
            "a script": (["sh", "-c", "cargo test"], False), "lane-gate": (["scripts/lane-gate.py"], False),
        }
        for name, (c, ok) in shapes.items():
            expect(f"light accepts/refuses: {name}", light_refusal(c, lt) is None, ok)
        # --manifest-path (ov-847): a crate outside the root workspace runs light, judged by its own warm target/.
        web = lt / "crates" / "web"
        web.mkdir(parents=True)
        (web / "Cargo.toml").write_text('[package]\nname = "w"\n\n[workspace]\n')
        (lt / "Cargo.toml").write_text('[workspace]\nmembers = []\n')
        mcmd = ["cargo", "test", "--manifest-path", "crates/web/Cargo.toml", "f"]
        cwd0 = os.getcwd()
        os.chdir(lt)
        try:
            cold = light_refusal(mcmd, lt)
            expect("--manifest-path to a crate with a cold target/ queues, whatever the root's target/ is", cold is not None and "cold" in cold, True)
            (web / "target" / "debug").mkdir(parents=True)
            expect("--manifest-path <crate>/Cargo.toml with a filter is light on that crate's own warm target/", light_refusal(mcmd, lt), None)
            expect("--manifest-path=<crate>/Cargo.toml is light too", light_refusal(["cargo", "test", "--manifest-path=crates/web/Cargo.toml", "f"], lt), None)
            expect("--manifest-path still needs a filter", light_refusal(["cargo", "test", "--manifest-path", "crates/web/Cargo.toml"], lt) is not None, True)
            expect("--manifest-path with --workspace still queues", light_refusal([*mcmd, "--workspace"], lt) is not None, True)
            outside = t / "elsewhere"
            outside.mkdir()
            (outside / "Cargo.toml").write_text('[package]\nname = "o"\n')
            (outside / "target" / "debug").mkdir(parents=True)
            why = light_refusal(["cargo", "test", "--manifest-path", str(outside / "Cargo.toml"), "f"], lt)
            expect("a --manifest-path outside the repo is refused", why is not None and "outside" in why, True)
            why = light_refusal(["cargo", "test", f"--manifest-path={outside}/Cargo.toml", "f"], lt)
            expect("... in the = spelling too", why is not None and "outside" in why, True)
            why = light_refusal(["cargo", "test", "--manifest-path", "../elsewhere/Cargo.toml", "f"], lt)
            expect("... and by a relative .. path", why is not None and "outside" in why, True)
            (lt / "crates" / "link").symlink_to(outside)
            why = light_refusal(["cargo", "test", "--manifest-path", "crates/link/Cargo.toml", "f"], lt)
            expect("a symlink under the repo that points outside is refused", why is not None and "outside" in why, True)
            (lt / "Cargo.toml").write_text('[workspace]\nmembers = []\n')
            expect("a virtual workspace manifest is refused", light_refusal(["cargo", "test", "--manifest-path", "Cargo.toml", "f"], lt) is not None, True)
        finally:
            os.chdir(cwd0)
        expect("a cold target/ is not light", light_refusal(lcmd, t / "nowhere") is not None, True)
        refused = subprocess.run([heavy, "--light", "--", "cargo", "test", "--workspace"], cwd=lt, env=lenv, capture_output=True, text=True, timeout=10)
        expect("scripts/heavy --light refuses a whole-workspace run with exit 2 and runs nothing", (refused.returncode, "queue" in refused.stderr), (2, True))
        (t / "light-marks").unlink(missing_ok=True)
        wp = start_light(cmd=mcmd)
        wp.communicate(timeout=10)
        expect("scripts/heavy --light -- cargo test --manifest-path crates/web/Cargo.toml f runs light", (wp.returncode, light_ran()), (0, 1))
        (t / "light-marks").unlink(missing_ok=True)
        wo = subprocess.run([heavy, "--light", "--", "cargo", "test", "--manifest-path", str(outside / "Cargo.toml"), "f"], cwd=lt, env=lenv, capture_output=True, text=True, timeout=10)
        expect("scripts/heavy --light refuses a --manifest-path outside the repo with exit 2 and runs nothing", (wo.returncode, light_ran()), (2, 0))
    if fails:
        for line in fails:
            print(f"heavy_lock self-test: {line}", file=sys.stderr)
        return 1
    print("heavy_lock self-test: ok")
    return 0


if __name__ == "__main__":
    sys.exit(self_test() if sys.argv[1:] == ["--self-test"] else 2)
