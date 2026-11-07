#!/usr/bin/python3
"""Explicit affected targets; stop immediately on the first failed phase."""
import subprocess,sys
cargo='/Users/e-liang/.cargo/bin/cargo'
commands=[
 ['test','--offline','-p','farcooler-daemon','-p','farcooler-store','-p','farcooler-protocol','--lib','workflow_','--','--nocapture'],
 ['test','--offline','-p','farcooler-cli','--bin','farcooler','workflow_','--','--nocapture'],
 ['test','--offline','-p','farcooler-daemon','--test','workflow_service_over_socket','--','--nocapture'],
 ['test','--offline','-p','farcooler-daemon','--test','workflow_commands_over_socket','--','--nocapture'],
]
for args in commands:
 print('affected phase:',args,flush=True)
 code=subprocess.call([cargo,*args])
 if code:sys.exit(code)
