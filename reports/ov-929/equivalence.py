#!/usr/bin/python3
"""Prove the inverse of the approved extraction reconstructs exact base bytes."""
import hashlib, json, subprocess
from pathlib import Path
root=Path(__file__).resolve().parents[2]
base='7cae4421e9d2c3aae62ed06153c3f7f627267bd5'
path='crates/daemon/src/rpc.rs'
old=subprocess.check_output(['git','show',base+':'+path],cwd=root)
new=(root/path).read_bytes()
module=(root/'crates/daemon/src/rpc/runner_settings.rs').read_bytes()
prefix=b'use super::{DomainError, Result, Rpc, Service, agents_found_now, wire};\n\nimpl Rpc {\n'
assert module.startswith(prefix) and module.endswith(b'}\n')
block=module[len(prefix):-2]
assert block.count(b'pub(super) ')==4
block=block.replace(b'pub(super) ',b'')
assert new.count(b'mod runner_settings;\n')==1
assert new.count(b'    async fn dispatch(')==1
inverse=new.replace(b'mod runner_settings;\n',b'',1).replace(b'    async fn dispatch(',block+b'    async fn dispatch(',1)
assert inverse==old, 'inverse differs from base'
def sha(data): return hashlib.sha256(data).hexdigest()
record={'base':base,'base_rpc_sha256':sha(old),'rpc_sha256':sha(new),'module_sha256':sha(module),
        'base_lines':len(old.splitlines()),'rpc_lines':len(new.splitlines()),'module_lines':len(module.splitlines()),
        'mapping':'base rpc.rs:763-840 -> rpc/runner_settings.rs:4-81; inverse removes four pub(super) tokens and module/import/impl wrapper, reinserts block before dispatch',
        'exact_inverse_equal':True,'all_other_rpc_bytes_unchanged':True}
print(json.dumps(record,indent=2))
