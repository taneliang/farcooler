#!/usr/bin/python3
from pathlib import Path
import subprocess,json,sys
root=Path(__file__).resolve().parents[2]
results={}
for mode in ['client','identity','client-lint']:
 code=subprocess.run(['/usr/bin/python3',str(root/'reports/ov-927/check.py'),mode,'final-'+mode],cwd=root).returncode
 results[mode]=code
 (root/'reports/ov-927/final-checks.json').write_text(json.dumps(results,indent=2)+'\n')
 if code:sys.exit(code)
