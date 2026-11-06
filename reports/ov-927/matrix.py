#!/usr/bin/python3
"""Sequential checks; each owns its FIFO slot, bounds and reserve monitor."""
from pathlib import Path
import subprocess,sys,json
root=Path(__file__).resolve().parents[2]
results={}
for mode in ['client','green','socket','jni','web','wasm','client-lint','lint']:
 code=subprocess.run(['/usr/bin/python3',str(root/'reports/ov-927/check.py'),mode],cwd=root).returncode
 results[mode]=code
 (root/'reports/ov-927/matrix.json').write_text(json.dumps(results,indent=2)+'\n')
 if code: sys.exit(code)
