#!/usr/bin/python3
"""One guard mutation at a time; always restore exact bytes before exiting."""
import hashlib,json,subprocess,sys
from pathlib import Path
root=Path(__file__).resolve().parents[2]
mode=sys.argv[1]
label=sys.argv[2] if len(sys.argv)>2 else mode
specs={
 'admission':('crates/store/src/workflow_notices.rs',
 'target_fingerprint(&tx,proof.workspace,proof.terminal,&proof.thread)?.as_ref()!=Some(&proof.fingerprint)',
 'false', 'notices-native'),
 'aba':('crates/store/src/workflow_notices.rs',
 'COALESCE((SELECT generation FROM workflow_notice_target_generations WHERE workspace=p.workspace_id),0)',
 '0', 'notices-native'),
 'atomic':('crates/store/src/workflow.rs',
 '                crate::workflow_notices::select(tx, &admitted, &state, &transition.state)?;',
 '                // injected: omit atomic notification production', 'notices-store'),
 'target-outcome':('crates/store/src/workflow_notices.rs',
 "AND target=?3 AND state='prepared'", "AND ?3 IS NOT NULL AND state='prepared'",'notices-store'),
 'owner-closed':('crates/daemon/src/workflow_notices.rs',
 '*stop.borrow() || stop.has_changed().is_err()',
 '*stop.borrow()', 'notices-native'),
 'inventory':('crates/tmux/src/inventory.rs',
 'if changed { self.targets.send_modify(|v| *v = v.wrapping_add(1)); }',
 'if changed && false { self.targets.send_modify(|v| *v = v.wrapping_add(1)); }','notices-native'),
 'inventory-silence':('crates/tmux/src/inventory.rs',
 'let changed = target_keys(&view) != target_keys(&snapshot);',
 'let changed = true;', 'notices-native'),
 'metadata':('crates/daemon/src/log_join.rs',
 '.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).ok()?;',
 '.open(path).ok()?;', 'notices-native'),
 'ack':('crates/daemon/src/workflow_runtime.rs',
 '        self.next_lease = None;',
 '        if !self.store.workflow_pending_notices(0)?.is_empty() { return Ok(0); }\n        self.next_lease = None;', 'negative-config'),
}
relative,old,new,check=specs[mode]
path=root/relative
original=path.read_bytes();s=original.decode();assert s.count(old)==1
mutant=s.replace(old,new).encode()
snapshots=root/'reports/ov-933/negative-sources';snapshots.mkdir(exist_ok=True)
(snapshots/(label+'.original.txt')).write_bytes(original)
(snapshots/(label+'.mutant.txt')).write_bytes(mutant)
record={'mode':mode,'label':label,'path':relative,'original':hashlib.sha256(original).hexdigest(),'mutant':hashlib.sha256(mutant).hexdigest()}
try:
 path.write_bytes(mutant)
 code=subprocess.call(['/usr/bin/python3',str(root/'reports/ov-933/check.py'),check,'negative-'+label])
 record['negative_exit']=code
 assert code!=0,'guard mutation survived'
finally:
 path.write_bytes(original)
 record['restored']=hashlib.sha256(path.read_bytes()).hexdigest()==record['original']
 (root/f'reports/ov-933/mutation-{label}.json').write_text(json.dumps(record,indent=2)+'\n')
assert record['restored']
