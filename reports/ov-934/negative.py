#!/usr/bin/python3
"""One bounded regression control; always restore exact current source bytes."""
import hashlib, json, subprocess, sys
from pathlib import Path
root = Path(__file__).resolve().parents[2]
kind = sys.argv[1]
paths = ['crates/store/src/workflow_ownership.rs'] if kind == 'alias' else {
    'safety': ['crates/daemon/src/workflow_runtime.rs', 'crates/daemon/src/workflow_runtime/service_owner.rs'],
    'recovery': ['crates/daemon/src/workflow_runtime/coding_execute.rs'],
    'availability': ['crates/daemon/src/workflow_service.rs'],
}[kind]
original = {p:(root/p).read_bytes() for p in paths}
def sha(raw): return hashlib.sha256(raw).hexdigest()
def replace(path, old, new):
    p=root/path; text=p.read_text(); assert old in text, old
    p.write_text(text.replace(old,new))
record = {'kind':kind,'parent':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),
          'before':{p:sha(b) for p,b in original.items()}}
try:
    if kind == 'alias':
        p=paths[0]; text=(root/p).read_text()
        old=subprocess.check_output(['git','show','e69ec2bbad90561f33cb707e374495b0b949772d:'+p],cwd=root,text=True)
        begin=text.index('fn task('); end=text.index('pub(crate) fn available',begin)
        prior=old[old.index('fn task('):old.index('pub(crate) fn available')]
        prior=prior.replace('    let Ok(workspace)', '    use rusqlite::OptionalExtension;\n    let Ok(workspace)')
        (root/p).write_text(text[:begin]+prior+text[end:])
        record['mutation']='exact e69 task resolver (function-local OptionalExtension import only); retains new tests and error variant'
    elif kind == 'safety':
        p=paths[0]
        replace(p,'let selected = self.select_item_configuration(&workspace, item);',
            'self.select_item_configuration(&workspace, item)?;\n                let selected: Result<()> = Ok(());')
        p=paths[1]
        replace(p,'        self.validate_native_configuration(config)?;\n        let check', '        let check')
        replace(p,'                self.validate_native_configuration(config)\n', '                Ok(())\n')
        replace(p,'        self.base = self.coding.as_ref().map_or_else(|| "0".repeat(40), |c| c.base.clone());\n        Ok(())',
            '        self.base = self.coding.as_ref().map_or_else(|| "0".repeat(40), |c| c.base.clone());\n        self.validate_item_launch(workspace,item)?;\n        Ok(())')
        record['mutation']='e69 R2 behavior: publication lacks endpoint validation; mutable item configuration/resource errors propagate from pump and close receiver'
    elif kind == 'recovery':
        p=paths[0]
        replace(p,'config.validate_pinned_definition(&item.definition)?;', 'config.validate_definition(&item.definition)?;')
        # Keep trait import live elsewhere so the control is behavioral, not a warning/build error.
        replace(p,'use super::coding::{FileValidation, PinnedDefinition};','use super::coding::FileValidation;')
        replace(p,'self.bound_command(&original_binding, false)?','self.bound_command(&original_binding, true)?')
        replace(p,'self.bound_command(binding, false)?','self.bound_command(binding, true)?')
        record['mutation']='e69 recovery binding/expected-command path requires launchable current settings and executable'
    else:
        replace(paths[0],'.filter(|o| !o.task.is_finished() && !o.handle.is_closed()).map(|o| o.handle.clone())',
            '.map(|o| o.handle.clone())')
        record['mutation']='e69 Service availability returns a retained closed/failed owner handle'
    record['mutated']={p:sha((root/p).read_bytes()) for p in paths}
    result=subprocess.run(['/usr/bin/python3',str(root/'reports/ov-934/check.py'),kind,'negative-'+kind],cwd=root)
    record['exit']=result.returncode
    log=(root/f'reports/ov-934/negative-{kind}.log').read_text()
    record['behavioral_red']=result.returncode==101 and 'test result: FAILED.' in log and 'error[E' not in log
    assert record['behavioral_red'], 'control must fail at a regression assertion, not compile/admission'
finally:
    for p, raw in original.items(): (root/p).write_bytes(raw)
    record['restored']={p:sha((root/p).read_bytes()) for p in paths}
    record['exact_restoration']=record['before']==record['restored']
    (root/f'reports/ov-934/mutation-{kind}.json').write_text(json.dumps(record,indent=2)+'\n')
    assert record['exact_restoration']
print(json.dumps({'kind':kind,'behavioral_red':record['behavioral_red'],'exact_restoration':record['exact_restoration']}))
