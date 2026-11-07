#!/usr/bin/python3
import os, subprocess
assert os.environ.get('HEAVY_LOCK_HELD') == '1'
for name in ['FARCOOLER_HOME','FARCOOLER_CONFIG','CLAUDE_CONFIG_DIR','TMUX_TMPDIR']:
    assert os.environ[name].startswith('/private/tmp/ov940-check-')
for selected in ['workflow_runtime_', 'workflow_notice']:
    subprocess.run(['/Users/e-liang/.cargo/bin/cargo','test','--offline','-p','farcooler-daemon','--lib',selected,'--','--nocapture'],check=True)
