#!/usr/bin/python3
"""Final allocated inventory guards and restored affected checks, sequential."""
import subprocess,sys
commands=[
 ['/usr/bin/python3','reports/ov-933/negative.py','inventory','freeze-inventory'],
 ['/usr/bin/python3','reports/ov-933/negative.py','inventory-silence','freeze-inventory-silence'],
 ['/usr/bin/python3','reports/ov-933/check.py','affected','affected-final'],
]
for command in commands:
 print('freeze check:',command,flush=True)
 code=subprocess.call(command)
 if code:sys.exit(code)
