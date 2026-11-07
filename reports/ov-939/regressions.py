#!/usr/bin/python3
"""Changed workflow contract regressions, including historical feature A and goldens."""
import subprocess,sys
commands=[
 ['test','--offline','--locked','-p','farcooler-workflow','--test','workflow'],
 ['test','--offline','--locked','-p','farcooler-client','--lib','workflow_','--','--nocapture'],
 ['test','--offline','--locked','-p','farcooler-protocol','--lib','workflow_'],
 ['test','--offline','--locked','-p','farcooler-daemon','--lib','workflow_runtime::wire_commands::','--','--nocapture'],
 ['test','--offline','--locked','-p','farcooler-cli','--bin','farcooler','workflow_'],
]
for args in commands:
    cmd=['/Users/e-liang/.cargo/bin/cargo',*args];print('REGRESSION',repr(cmd),flush=True)
    code=subprocess.call(cmd)
    if code:sys.exit(code)
