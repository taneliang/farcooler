#!/usr/bin/python3
"""Safe old-semantics mutations; only exact non-hazardous tests run."""
import hashlib,json,subprocess,sys
from pathlib import Path
root=Path(__file__).resolve().parents[2]
mode=sys.argv[1]
prefix=sys.argv[2] if len(sys.argv)>2 else ""
relative,old,new={
 'binding': ('crates/daemon/src/runtime_selection.rs','if !selected.is_explicit() {','if true {'),
 'selector': ('crates/daemon/src/paths.rs','if text.is_empty() || text.chars().any(char::is_control) || !root.is_absolute()', 'if false && (text.is_empty() || text.chars().any(char::is_control) || !root.is_absolute()'),
}[mode]
p=root/relative
original=p.read_bytes()
source=original.decode()
assert source.count(old)==1
mutated=source.replace(old,new)
if mode=='selector':
    mutated=mutated.replace('> MAX_SOCKET_PATH\n        {', '> MAX_SOCKET_PATH)\n        {',1)
try:
    p.write_text(mutated)
    code=subprocess.call(['/usr/bin/python3',str(root/'reports/ov-937/check.py'),'red-'+mode,prefix+'red-'+mode])
finally:
    p.write_bytes(original)
assert p.read_bytes()==original
record=dict(mode=mode,source=relative,before=hashlib.sha256(original).hexdigest(),mutant=hashlib.sha256(mutated.encode()).hexdigest(),exit=code,restored=True)
(root/f'reports/ov-937/{prefix}negative-{mode}.json').write_text(json.dumps(record,indent=2)+'\n')
assert code==101, record
print(json.dumps(record))
