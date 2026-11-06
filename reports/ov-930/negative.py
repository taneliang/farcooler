#!/usr/bin/python3
"""Mutation checks edit private generated JS copies only; production stays pinned."""
import json, shutil, subprocess
from pathlib import Path
root=Path(__file__).resolve().parents[2]
base=Path('/private/tmp/ov930-checks/web')
results=[]
original=(base/'workflow-session.js').read_text()
mutations={
 'opaque-bytes':('entry.prepared = reply.prepared;', 'entry.prepared = {...reply.prepared, json: reply.prepared.json + " "};'),
 'captured-session':('const snapshot = workflowJSON(', 'this.initialSession ??= current.session; const snapshot = workflowJSON('),
 'missing-persistence':('this.storage.setItem(key, JSON.stringify({ schema: 1, payload, checksum: await this.checksum(payload) }));','void payload;'),
}
for label,(old,new) in mutations.items():
    path=Path('/private/tmp/ov930-negative-'+label);path.mkdir(exist_ok=True)
    for source in base.glob('*.js'):shutil.copy2(source,path/source.name)
    assert old in original,label
    changed=original.replace(old,new)
    if label=='captured-session':
        old='const raw = await latest.session.workflow_call('
        assert old in changed
        changed=changed.replace(old,'const raw = await this.initialSession.workflow_call(')
    (path/'workflow-session.js').write_text(changed)
    result=subprocess.run(['/opt/homebrew/bin/node','services/web/test/workflow-session.test.mjs',str(path)],cwd=root,capture_output=True,text=True,timeout=30)
    (root/f'reports/ov-930/negative-{label}.log').write_text(result.stdout+result.stderr)
    assert result.returncode!=0,label+' escaped'
    results.append({'mutation':label,'exit':result.returncode,'production_unchanged':(base/'workflow-session.js').read_text()==original})
(root/'reports/ov-930/negative.json').write_text(json.dumps(results,indent=2)+'\n')
print('all three meaningful byte/session/persistence mutations rejected; production unchanged')
