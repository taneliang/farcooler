#!/usr/bin/python3
"""Read-only cache-holder probe after normal FIFO admission; no cache trimming."""
import os, subprocess, sys
from pathlib import Path
assert os.environ.get('HEAVY_LOCK_HELD') == '1'
for lock in [Path(sys.argv[1])/'debug/.cargo-lock', Path('/Users/e-liang/.cargo/.package-cache')]:
    probe=subprocess.run(['/usr/sbin/lsof',str(lock)],capture_output=True,text=True)
    print('holder probe:',str(lock),'exit',probe.returncode,probe.stdout,probe.stderr,flush=True)
    if probe.returncode != 1: sys.exit('refusing occupied or uninspectable cache')
print('load5',os.getloadavg()[1], 'argv',sys.argv[2:],flush=True)
if os.getloadavg()[1] >= 20: sys.exit('load5 over admission bound')
os.execv(sys.argv[2],sys.argv[2:])
