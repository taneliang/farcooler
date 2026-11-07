#!/usr/bin/python3
"""Apply one finite source mutation, or restore its byte-exact saved source."""
import hashlib,json,sys
from pathlib import Path
root=Path(__file__).resolve().parents[2]
kind,action=sys.argv[1:]
seams={
 'config':('crates/daemon/src/workflow_runtime/service_owner.rs','self.store.workflow_bundle(&item.policy_pin)?','self.store.workflow_configuration(workspace)?'),
 'owner':('crates/store/src/workflow.rs','if result.is_ok() && !crate::workflow_ownership::available(tx, input)? {','if result.is_ok() && false && !crate::workflow_ownership::available(tx, input)? {'),
}
name,original,mutant=seams[kind];path=root/name
backup=Path('/private/tmp')/('ov931-'+kind+'-original.rs')
hash=lambda b:hashlib.sha256(b).hexdigest()
record=root/'reports/ov-931/mutations.json';data=json.loads(record.read_text()) if record.exists() else {}
if action=='apply':
 content=path.read_bytes();assert content.count(original.encode())==1
 backup.write_bytes(content);replacement=content.replace(original.encode(),mutant.encode());path.write_bytes(replacement)
 data[kind]={'file':name,'original':hash(content),'mutant':hash(replacement),'restored':False}
else:
 content=backup.read_bytes();assert hash(content)==data[kind]['original'];path.write_bytes(content)
 data[kind]['restored']=hash(path.read_bytes())==data[kind]['original']
record.write_text(json.dumps(data,indent=2)+'\n')
print(kind,action,data[kind])
