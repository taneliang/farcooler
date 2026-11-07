#!/usr/bin/python3
"""Selected focused tests; run only under the outer canonical FIFO supervisor."""
import subprocess,sys
commands=[
 ['test','--offline','--locked','-p','farcooler-workflow','--test','workflow','workflow_reasoned','--','--nocapture'],
 ['test','--offline','--locked','-p','farcooler-client','-p','farcooler-protocol','-p','farcooler-daemon','--lib','workflow_reasoned','--','--nocapture'],
 ['test','--offline','--locked','-p','farcooler-cli','--bin','farcooler','workflow_reasoned','--','--nocapture'],
]
if sys.argv[1:] == ['remaining']:
    commands=commands[1:]
    commands[0]=[a for a in commands[0] if a!='farcooler-client']
    commands[0].remove('-p')
for args in commands:
    cmd=['/Users/e-liang/.cargo/bin/cargo',*args]
    print('SELECTED',repr(cmd),flush=True)
    code=subprocess.call(cmd)
    if code:sys.exit(code)
