#!/usr/bin/python3
"""Only native notice lifecycle and existing group-helper callers affected here."""
import subprocess,sys
cargo='/Users/e-liang/.cargo/bin/cargo'
filters=['workflow_notice_',
         'workflow_runtime_process_timeout_and_cancel_kill_owned_group',
         'workflow_service_native_stop_ignores_launch_drift_with_original_owner',
         'workflow_service_availability_tracks_failed_owner_and_join_before_restart',
         'workflow_coding_cancel_running_owned_process_settles_original_then_stop']
for name in filters:
    command=[cargo,'test','--offline','-p','farcooler-daemon','--lib',name,'--','--nocapture']
    print('affected phase:',command,flush=True)
    code=subprocess.call(command)
    if code:sys.exit(code)
sys.exit(subprocess.call(['/usr/bin/python3','scripts/file-size-budget.py']))
