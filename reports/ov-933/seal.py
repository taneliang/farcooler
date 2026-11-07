#!/usr/bin/python3
"""Bind final source, gate inputs, guard failures and retained evidence."""
import ast,hashlib,json,re,subprocess,sys
from pathlib import Path
root=Path(__file__).resolve().parents[2]
report=root/'reports/ov-933'
affected=sys.argv[1] if len(sys.argv)>1 else 'affected-freeze'
lint=sys.argv[2] if len(sys.argv)>2 else 'lint-freeze'
def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def git(*args):return subprocess.check_output(['git',*args],cwd=root,text=True).strip()
tracked=git('diff','--name-only','HEAD').splitlines()
new=git('ls-files','--others','--exclude-standard').splitlines()
products=sorted(p for p in set(tracked+new) if (p=='Cargo.lock' or p.startswith('crates/')) and (root/p).is_file())
source={p:sha(root/p) for p in products}
gates={}
for name in [affected,lint]:
 record=json.loads((report/(name+'.json')).read_text())
 assert record['exit']==0 and record['stopped'] is None and record['source_unchanged']
 assert record['minimum_free']>=10*1024**3
 assert record['before']==record['after']
 assert all(sha(root/p)==h for p,h in record['after'].items()),name+' current input drift'
 assert all(record['after'].get(p)==h for p,h in source.items()),name+' missing final source'
 gates[name]={'exit':0,'input_count':len(record['after']),'seconds':record['seconds'],'minimum_free':record['minimum_free'],'source_unchanged':True}
blocks=(report/(affected+'.log')).read_text().split('affected phase: ')[1:]
suites=[]
for block in blocks:
 command=ast.literal_eval(block.splitlines()[0])
 summaries=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;',block)
 assert summaries
 passed,failed,ignored=map(int,summaries[-1]);assert failed==0
 suites.append({'command':command,'passed':passed,'ignored':ignored,'nested_summary_count':len(summaries)-1})
assert 'file-size-budget: ok.' in (report/(affected+'.log')).read_text()
labels=['final-admission','final-aba','final-atomic','final-target-outcome','final-metadata','final-ack','freeze-inventory','freeze-inventory-silence']
if (report/'mutation-freeze-owner-closed.json').exists(): labels.append('freeze-owner-closed')
negative={}
for label in labels:
 record=json.loads((report/('mutation-'+label+'.json')).read_text())
 check=json.loads((report/('negative-'+label+'.json')).read_text())
 log=(report/('negative-'+label+'.log')).read_text()
 assert record['negative_exit']!=0 and record['restored'] and check['source_unchanged'] and check['stopped'] is None
 assert 'test result: FAILED.' in log and 'panicked at' in log and 'error[E' not in log
 assert sha(report/('negative-sources/'+label+'.original.txt'))==record['original']==source[record['path']]
 assert sha(report/('negative-sources/'+label+'.mutant.txt'))==record['mutant']
 deltas={p:{'tested':h,'frozen':sha(root/p)} for p,h in check['before'].items() if sha(root/p)!=h}
 negative[label]={**record,'guard_file_matches_final':True,'input_graph_deltas':deltas}
old=report/'negative-sources/owner-closed.old.txt'
old_record=json.loads((report/'owner-closed-red.json').read_text())
assert old_record['exit']!=0 and old_record['source_unchanged'] and old_record['before']['crates/daemon/src/workflow_notices.rs']==sha(old)
assert 'closed owner cannot admit another native attempt' in (report/'owner-closed-red.log').read_text()
(report/'negative-sources/owner-closed.restored.txt').write_bytes((root/'crates/daemon/src/workflow_notices.rs').read_bytes())
(report/'product-files.txt').write_text('\n'.join(products)+'\n')
diff=subprocess.run(['git','diff','HEAD','--check','--',*products],cwd=root,capture_output=True,text=True)
assert diff.returncode==0
(report/'product-diff-check.log').write_text(diff.stdout+diff.stderr+'exit=0\n')
manifest={'task':'ov-933','base':'77250986dcb4b1dbb6b63e06d0b11dc1b92d88a3','head_before_commit':git('rev-parse','HEAD'),'branch':git('branch','--show-current'),
 'product_files':source,'gates':gates,'main_suites':suites,'main_pass_count':sum(s['passed'] for s in suites),'main_ignored_count':sum(s['ignored'] for s in suites),
 'negative_guards':negative,'closed_owner_old_source_sha256':sha(old),'closed_owner_restored_sha256':source['crates/daemon/src/workflow_notices.rs']}
manifest['evidence_files']={str(p.relative_to(root)):sha(p) for p in sorted(report.rglob('*')) if p.is_file() and p.name!='freeze.json'}
(report/'freeze.json').write_text(json.dumps(manifest,indent=2)+'\n')
print(json.dumps({'products':len(source),'main_passed':manifest['main_pass_count'],'main_ignored':manifest['main_ignored_count'],'negative_guards':len(negative),'gates':gates},indent=2))
