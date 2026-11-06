#!/usr/bin/python3
"""Exact reviewed patch union and source/evidence digests; no source mutation."""
import hashlib,json,subprocess
from pathlib import Path
root=Path(__file__).resolve().parents[2]
out=root/'reports/ov-930'
def git(*args):return subprocess.check_output(['git',*args],cwd=root)
def digest(blob):return hashlib.sha256(blob).hexdigest()
base='7cae4421e9d2c3aae62ed06153c3f7f627267bd5'
renderer='b34e94d0d1e6c82d4dc74b3a1c094630f75f96b9'
composition='b736f1d89f59faa7c81b93a7dc92b4fbe3fcbcf3'
pairs=list(zip(['db43f6393','26b1665ac','32a008112','b34e94d0d'],['1502f74db','39dc7785c','ab557b5cf','b736f1d89']))
commits=[]
for source,forward in pairs:
    patches=[]
    for commit in (source,forward):
        p=subprocess.run(['git','patch-id','--stable'],cwd=root,input=git('show','--pretty=format:',commit),capture_output=True,check=True)
        patches.append(p.stdout.decode().split()[0])
    assert patches[0]==patches[1]
    commits.append(dict(source=git('rev-parse',source).decode().strip(),forward=git('rev-parse',forward).decode().strip(),patch_id=patches[0]))
paths=git('diff','--name-only','72ccadf5c9987b851972780409fcccc9cf6edc37',renderer).decode().splitlines()
union={}
for path in paths:
    original=git('show',renderer+':'+path);composed=git('show',composition+':'+path)
    assert original==composed,path
    union[path]=digest(composed)
(out/'composition.json').write_text(json.dumps(dict(base=base,reviewed_renderer=renderer,composition=composition,commits=commits,reviewed_source_union=union),indent=2)+'\n')
changed=set(git('diff','--name-only',composition).decode().splitlines())
changed.update(git('ls-files','--others','--exclude-standard').decode().splitlines())
sources=sorted(p for p in changed if not p.startswith(('.codex/','.cursor/','reports/')) and (root/p).is_file())
(out/'source-sha256.json').write_text(json.dumps({p:digest((root/p).read_bytes()) for p in sources},indent=2)+'\n')
evidence=sorted(p for p in out.rglob('*') if p.is_file() and p.name!='evidence-sha256.json')
(out/'evidence-sha256.json').write_text(json.dumps({str(p.relative_to(root)):digest(p.read_bytes()) for p in evidence},indent=2)+'\n')
print(len(commits),'equal patch IDs,',len(union),'exact renderer union paths,',len(sources),'source paths')
