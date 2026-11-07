#!/usr/bin/python3
"""Freeze final-source guard evidence; run sequentially, each via normal FIFO."""
import subprocess,sys
for mode in ['admission','aba','atomic','target-outcome','metadata','ack']:
 print('final-source negative:',mode,flush=True)
 code=subprocess.call(['/usr/bin/python3','reports/ov-933/negative.py',mode,'final-'+mode])
 if code:sys.exit(code)
