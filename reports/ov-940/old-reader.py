#!/usr/bin/python3
"""Real pre-cutover reader control: build the immutable parent commit's Store and
open (a) a registry-65 source and (b) the file left by transfer::begin."""
import json, os, subprocess, sys, shutil, tempfile
from pathlib import Path
root = Path(__file__).resolve().parents[2]
OLD = 'd43e8f99cc00d81ec5f2ed4e606483e9352fe3a3'
scratch = Path(tempfile.mkdtemp(prefix='ov940-oldreader-', dir='/private/tmp'))
fixtures = scratch / 'fixtures'; fixtures.mkdir()
src = scratch / 'oldsrc'; src.mkdir()
env = dict(os.environ)
def run(cmd, cwd, **kw):
    r = subprocess.run(cmd, cwd=cwd, env={**env, **kw.get('env', {})}, capture_output=True, text=True)
    print(r.stdout[-2000:], r.stderr[-2000:], flush=True)
    return r
tar = subprocess.run(['git', 'archive', OLD], cwd=root, capture_output=True, check=True).stdout
subprocess.run(['tar', '-x', '-C', str(src)], input=tar, check=True)
(src / 'crates/store/examples').mkdir(exist_ok=True)
(src / 'crates/store/examples/old_open.rs').write_text(
    'fn main(){let p=std::env::args().nth(1).unwrap();match farcooler_store::Store::open(&p){Ok(_)=>println!("OPENED"),Err(e)=>println!("REFUSED {e:?}")}}\n')
cargo = '/Users/e-liang/.cargo/bin/cargo'
w = run([cargo, 'test', '--offline', '-p', 'farcooler-store', '--lib', 'old_reader_fixture', '--', '--nocapture'], root,
        env={'OV940_OLD_READER_DIR': str(fixtures)})
assert w.returncode == 0 and (fixtures / 'begun.sqlite').exists(), 'fixture writer failed'
b = run([cargo, 'build', '--offline', '-p', 'farcooler-store', '--example', 'old_open'], src)
assert b.returncode == 0, 'old reader build failed'
exe = Path(os.environ['CARGO_TARGET_DIR']) / 'debug/examples/old_open'
results = {}
for name in ['v65.sqlite', 'begun.sqlite']:
    copy = scratch / ('open-' + name); shutil.copy(fixtures / name, copy)
    results[name] = subprocess.run([str(exe), str(copy)], capture_output=True, text=True).stdout.strip()
print(json.dumps({'old_commit': OLD, 'results': results}, indent=2))
(root / 'reports/ov-940/old-reader-result.json').write_text(json.dumps({'old_commit': OLD, 'results': results}, indent=2) + '\n')
assert results['v65.sqlite'] == 'OPENED', 'control is vacuous: old reader cannot open its own schema'
assert results['begun.sqlite'].startswith('REFUSED'), 'old reader opened the upgraded frozen file'
