#!/usr/bin/python3
"""One exclusive final lease: affected lifecycle, size, then daemon Clippy."""
import subprocess,sys
commands=[['/usr/bin/python3','reports/ov-938/affected.py'],
          ['/Users/e-liang/.cargo/bin/cargo','clippy','--offline','-p','farcooler-daemon','--all-targets','--','-D','warnings']]
for command in commands:
    print('final phase:',command,flush=True)
    code=subprocess.call(command)
    if code:sys.exit(code)
