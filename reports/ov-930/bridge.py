#!/usr/bin/python3
"""Owned real Session bridge + isolated cached browser under the caller's FIFO."""
import json, os, shutil, signal, subprocess, time, urllib.request
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
OUT=Path('/private/tmp/ov930-browser-'+str(os.getpid()));OUT.mkdir()
target=ROOT.parent/'ov-923-workflow-runtime/target'
env={**os.environ,'OV930_BROWSER_BRIDGE':str(OUT),'CARGO_TARGET_DIR':str(target)}
command=['/Users/e-liang/.cargo/bin/cargo','test','--offline','-p','farcooler-client','--lib','workflow_host_browser_bridge','--','--ignored','--nocapture']
child=None;browser=None;ready=None
log=(ROOT/'reports/ov-930/browser-bridge.log').open('w')
def stop_process(p):
    if p is not None and p.poll() is None:
        os.killpg(p.pid,signal.SIGTERM)
        try:p.wait(timeout=5)
        except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);p.wait()
def stop(sig,_):
    stop_process(browser);stop_process(child);raise SystemExit(128+sig)
for sig in (signal.SIGTERM,signal.SIGHUP,signal.SIGINT):signal.signal(sig,stop)
try:
    child=subprocess.Popen(command,cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    deadline=time.monotonic()+180
    while not (OUT/'ready.json').exists():
        if child.poll() is not None or time.monotonic()>deadline:raise RuntimeError('bridge startup')
        time.sleep(.1)
    ready=json.loads((OUT/'ready.json').read_text())
    browser=subprocess.Popen(['/opt/homebrew/bin/node','reports/ov-930/browser-session.mjs','/private/tmp/ov930-checks/web',str(OUT/'ready.json')],cwd=ROOT,env=env,start_new_session=True)
    if browser.wait(timeout=120):raise RuntimeError('browser check')
    request=urllib.request.Request('http://127.0.0.1:'+str(ready['port'])+'/call',data=b'{"method":"stop","args":{}}',method='POST')
    urllib.request.urlopen(request,timeout=5).read()
    if child.wait(timeout=15):raise RuntimeError('bridge final assertions')
    shutil.copy2(OUT/'result.json',ROOT/'reports/ov-930/browser-session-result.json')
finally:
    stop_process(browser);stop_process(child);log.close()
    shutil.rmtree(OUT)
