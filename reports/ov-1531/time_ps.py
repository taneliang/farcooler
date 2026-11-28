#!/usr/bin/env python3
"""Time the three `ps` reads `pane_ps` makes, spawn to exit, on a pty of its
own with a process in front of it."""
import os, pty, signal, subprocess, sys, time
n = int(sys.argv[1])
pid, fd = pty.fork()
if pid == 0:
    os.execv("/bin/sleep", ["sleep", "3600"])
time.sleep(0.2)
tty = subprocess.check_output(["ps", "-o", "tty=", "-p", str(pid)], text=True).strip()
reads = {
    "list (ps -t)": ["ps", "-t", tty, "-o", "pid=,ppid=,stat=,comm="],
    "args (ps -p)": ["ps", "-o", "args=", "-p", str(pid)],
    "env (ps -E)": ["ps", "-E", "-ww", "-o", "command=", "-p", str(pid)],
}
try:
    print("load average at start: %.1f %.1f %.1f, %d cpus" % (*os.getloadavg(), os.cpu_count()))
    for name, cmd in reads.items():
        ms = []
        for _ in range(n):
            t = time.monotonic()
            subprocess.run(cmd, stdin=subprocess.DEVNULL, capture_output=True, check=True)
            ms.append((time.monotonic() - t) * 1000)
        ms.sort()
        at = lambda q: ms[min(n - 1, int(q * n))]
        print(f"{name:14} n={n} p50={at(.5):.0f}ms p95={at(.95):.0f}ms p99={at(.99):.0f}ms max={ms[-1]:.0f}ms over500ms={sum(m > 500 for m in ms)}")
    print("load average at end:   %.1f %.1f %.1f" % os.getloadavg())
finally:
    os.kill(pid, signal.SIGKILL)
