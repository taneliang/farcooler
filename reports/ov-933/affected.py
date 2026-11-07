#!/usr/bin/python3
"""Exact affected suites; all run inside one exclusive FIFO lease."""
import subprocess,sys
cargo='/Users/e-liang/.cargo/bin/cargo'
commands=[
 ['test','--offline','-p','farcooler-tmux','--lib','inventory::','--','--nocapture'],
 ['test','--offline','-p','farcooler-store','--lib','workflow_','--','--nocapture'],
 ['test','--offline','-p','farcooler-protocol','--lib','workflow_','--','--nocapture'],
 ['test','--offline','-p','farcooler-cli','--bin','farcooler','workflow_','--','--nocapture'],
 ['test','--offline','-p','farcooler-daemon','--lib','workflow_','--','--nocapture'],
 ['test','--offline','-p','farcooler-daemon','--lib','log_join::tests::','--','--nocapture'],
 ['test','--offline','-p','farcooler-daemon','--lib','watch::answer_wake::codex_turn::','--','--nocapture'],
 ['test','--offline','-p','farcooler-daemon','--test','workflow_service_over_socket','--','--nocapture'],
 ['test','--offline','-p','farcooler-daemon','--test','workflow_commands_over_socket','--','--nocapture'],
]
for args in commands:
 print('affected phase:',args,flush=True)
 code=subprocess.call([cargo,*args])
 if code:sys.exit(code)
sys.exit(subprocess.call(['/usr/bin/python3','scripts/file-size-budget.py']))
