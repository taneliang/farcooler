#!/usr/bin/python3
"""Owned temporary mutations; each must fail the composed Session/socket suite."""
from pathlib import Path
import subprocess, json
root=Path(__file__).resolve().parents[2]
p=root/'crates/client/src/session/workflow.rs'
original=p.read_text()
mutations={
 'guard': ('    if !card.actions.contains(&selected) { return Err(stale()); }', '    // mutation: accept stale selection and substitute fresh evidence'),
 'bytes': ('self.workflow_wire(prepared.workspace, prepared.json.clone()).await', '''{
            let mut changed: serde_json::Value = serde_json::from_str(&prepared.json).unwrap();
            let revision = changed["mutation"]["revision"].as_u64().unwrap();
            changed["mutation"]["revision"] = serde_json::json!(revision + 1);
            self.workflow_wire(prepared.workspace, changed.to_string()).await
        }'''),
}
results={}
try:
 for name,(old,new) in mutations.items():
  assert original.count(old)==1
  p.write_text(original.replace(old,new))
  code=subprocess.run(['/usr/bin/python3',str(root/'reports/ov-927/check.py'),'client','mutation-'+name],cwd=root).returncode
  results[name]=code
  if code==0: raise SystemExit('mutation survived: '+name)
finally:
 p.write_text(original)
 (root/'reports/ov-927/mutations.json').write_text(json.dumps(results,indent=2)+'\n')
assert set(results)==set(mutations)
