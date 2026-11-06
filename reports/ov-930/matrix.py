#!/usr/bin/python3
"""Each scoped job enters the normal FIFO independently; no reservation bypass."""
import subprocess,sys
for mode in ('rust','rust-lint','jni','wasm','browser'):
    result=subprocess.run(['/usr/bin/python3','reports/ov-930/check.py',mode,'final-'+mode])
    if result.returncode:sys.exit(result.returncode)
