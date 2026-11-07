#!/usr/bin/python3
"""Only the final changed Session envelope validator and CLI producer/retained path."""
import subprocess,sys
for args in [
 ['test','--offline','--locked','-p','farcooler-client','--lib','workflow_','--','--nocapture'],
 ['test','--offline','--locked','-p','farcooler-cli','--bin','farcooler','workflow_','--','--nocapture'],
]:
    cmd=['/Users/e-liang/.cargo/bin/cargo',*args];print('FINAL DELTA',repr(cmd),flush=True)
    code=subprocess.call(cmd)
    if code:sys.exit(code)
