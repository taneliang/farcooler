#!/usr/bin/python3
import subprocess,sys
commands=[
 ['/usr/bin/python3','reports/ov-933/negative.py','owner-closed','freeze-owner-closed'],
 ['/usr/bin/python3','reports/ov-933/check.py','affected','affected-sealed'],
 ['/usr/bin/python3','reports/ov-933/check.py','lint','lint-sealed'],
]
for command in commands:
 print('sealed check:',command,flush=True)
 code=subprocess.call(command)
 if code:sys.exit(code)
