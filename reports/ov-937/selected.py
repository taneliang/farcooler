#!/usr/bin/python3
import subprocess,sys
cargo='/Users/e-liang/.cargo/bin/cargo'
filters=['runtime_selection_', 'log_watch::tests::a_codex_home_elsewhere_is_a_root', 'log_watch::tests::the_runners_claude_dir_is_the_claude_root', 'paths::tests::']
for name in filters:
    args=[cargo,'test','--offline','-p','farcooler-daemon','--lib',name,'--','--nocapture']
    print('private test:',args,flush=True)
    code=subprocess.call(args)
    if code: sys.exit(code)
code=subprocess.call([cargo,'test','--offline','-p','farcooler-cli','--bin','farcooler','runtime_selection_','--','--nocapture'])
if code: sys.exit(code)
sys.exit(subprocess.call(['/usr/bin/python3','scripts/file-size-budget.py']))
