#!/usr/bin/python3
"""One bounded foreground check with raw evidence and normal canonical FIFO."""
import hashlib,json,os,shutil,signal,subprocess,sys,time
from pathlib import Path
root=Path(__file__).resolve().parents[2]
report=root/'reports/ov-941'
label,*argv=sys.argv[1:]
assert label and argv and '/' not in label
assert not (report/(label+'.log')).exists(), 'preserve original evidence; use a new label'
target=root.parent/'ov-923-workflow-runtime/target'
env={**os.environ,'PATH':'/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/Users/e-liang/.cargo/bin',
     'CARGO_TARGET_DIR':str(target),'CARGO_BUILD_JOBS':'6','CARGO_PROFILE_DEV_DEBUG':'0',
     'CARGO_PROFILE_TEST_DEBUG':'0','CARGO_INCREMENTAL':'0','CARGO_NET_OFFLINE':'true','RUST_TEST_THREADS':'1'}
fixture=Path('/private/tmp')/f'ov941-check-{os.getpid()}'
fixture.mkdir(mode=0o700)
env.update(FARCOOLER_CONFIG=str(fixture/'config.toml'),CLAUDE_CONFIG_DIR=str(fixture/'claude'))
tracked=subprocess.check_output(['git','ls-files','--cached','--others','--exclude-standard','-z','crates','apps','services/web','Cargo.toml','Cargo.lock','scripts/heavy','scripts/heavy_lock.py'],cwd=root).decode().split('\0')
paths=set(p for p in tracked if p and (root/p).is_file())
paths.update(str(p.relative_to(root)) for p in (root/'crates').rglob('*.rs') if 'target' not in p.parts)
paths.update(str(p.relative_to(root)) for p in report.iterdir() if p.suffix in ('.py','.swift'))
def hashes():return {p:hashlib.sha256((root/p).read_bytes()).hexdigest() for p in sorted(paths)}
before=hashes()
inner=['/usr/bin/python3',str(report/'entry.py'),str(target),*argv] if Path(argv[0]).name=='cargo' or label.startswith('rust-') else argv
command=['/usr/bin/python3',str(root/'scripts/heavy'),'--load-limit','20','--',*inner]
start=time.monotonic();free_start=minimum=shutil.disk_usage(root).free;stopped=None
assert minimum>=10*1024**3
child=None
with (report/(label+'.log')).open('x') as log:
    log.write('COMMAND '+repr(command)+'\n');log.flush()
    def stop():
        if child and child.poll() is None:
            os.killpg(child.pid,signal.SIGTERM)
            try:child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid,signal.SIGKILL);child.wait()
    def interrupted(sig,_):
        global stopped
        stopped='signal:'+str(sig);stop()
    for sig in (signal.SIGTERM,signal.SIGHUP,signal.SIGINT):signal.signal(sig,interrupted)
    child=subprocess.Popen(command,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    try:
        while child.poll() is None:
            minimum=min(minimum,shutil.disk_usage(root).free)
            if minimum<10*1024**3 or time.monotonic()-start>1800:
                stopped='reserve/deadline';break
            time.sleep(.5)
    finally:stop()
    code=child.wait();log.write(f'\nEXIT {code} stopped={stopped}\n')
after=hashes()
record=dict(command=command,head=subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),
    target=str(target),profile='debug0/test0/incremental0/jobs6/offline/locked',exit=code,stopped=stopped,
    seconds=time.monotonic()-start,minimum_free=minimum,free_start=free_start,free_end=shutil.disk_usage(root).free,
    log_bytes=(report/(label+'.log')).stat().st_size,pid=child.pid,reaped=child.poll() is not None,source_unchanged=before==after,before=before,after=after)
(report/(label+'.json')).write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps({k:v for k,v in record.items() if k not in ('before','after')}),flush=True)
sys.exit(code if not stopped and before==after else 1)
