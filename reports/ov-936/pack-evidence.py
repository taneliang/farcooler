#!/usr/bin/python3
"""Byte-preserving signed copies; original private evidence is left intact."""
import gzip,hashlib,json,os
from pathlib import Path
root=Path(__file__).resolve().parents[2]
assert os.environ.get('HEAVY_LOCK_HELD')=='1'
folder=root/'reports/ov-936/fixtures'
records=[]
for path in sorted(folder.rglob('*')):
 if path.suffix not in ['.json','.sqlite']:continue
 raw=path.read_bytes();packed=gzip.compress(raw,compresslevel=6,mtime=0);destination=path.with_suffix(path.suffix+'.gz')
 if destination.exists():assert destination.read_bytes()==packed
 else:destination.write_bytes(packed)
 assert gzip.decompress(destination.read_bytes())==raw
 records.append(dict(path=str(path.relative_to(root)),sha256=hashlib.sha256(raw).hexdigest(),bytes=len(raw),signed_copy=str(destination.relative_to(root)),gzip_sha256=hashlib.sha256(packed).hexdigest(),gzip_bytes=len(packed)))
(folder/'.gitignore').write_text('*.json\n*.sqlite\n')
(root/'reports/ov-936/fixture-index.json').write_text(json.dumps(records,indent=2)+'\n')
print(json.dumps(dict(evidence_files=len(records),raw_bytes=sum(r['bytes'] for r in records),signed_gzip_bytes=sum(r['gzip_bytes'] for r in records),originals_retained=True)),flush=True)
