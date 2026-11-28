#!/usr/bin/env python3
"""Load for `seconds`: `spin` busy loops and `churn` loops that fork and exec
without pause. It starts its own process group and kills only that."""
import os, signal, subprocess, sys, time
spin, churn, seconds = map(int, sys.argv[1:4])
kids = [subprocess.Popen(["/usr/bin/yes"], stdout=subprocess.DEVNULL, start_new_session=True) for _ in range(spin)]
kids += [subprocess.Popen(["/bin/sh", "-c", "while :; do /usr/bin/true; done"], start_new_session=True) for _ in range(churn)]
def stop(*_):
    for k in kids:
        try: os.killpg(k.pid, signal.SIGKILL)
        except (ProcessLookupError, PermissionError): k.kill()  # macOS says EPERM for a group already gone
    sys.exit(0)
signal.signal(signal.SIGTERM, stop); signal.signal(signal.SIGINT, stop)
try: time.sleep(seconds)
finally: stop()
