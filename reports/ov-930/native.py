#!/usr/bin/python3
"""One isolated native graphical evidence batch, normal FIFO, owned 900s."""
import hashlib,json,os,plistlib,shutil,signal,subprocess,time,urllib.request
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
OUT=Path('/private/tmp/ov930-native-'+str(os.getpid()))
REPORT=ROOT/'reports/ov-930'
children=[]
def eligible(label):
    raw=subprocess.check_output(['/usr/sbin/ioreg','-l','-n','Root','-d','1'],text=True)
    required=['"IOConsoleLocked" = No','"kCGSSessionOnConsoleKey"=Yes','"kCGSessionLoginDoneKey"=Yes','"kCGSSessionUserIDKey"=502']
    record=dict(label=label,time=time.time(),uid=os.getuid(),unlocked=all(s in raw for s in required))
    with (REPORT/'native-eligibility.jsonl').open('a') as f:f.write(json.dumps(record)+'\n')
    assert record['uid']==502 and record['unlocked'],'locked/unknown console'
def stop_process(p):
    if p.poll() is None:
        os.killpg(p.pid,signal.SIGTERM)
        try:p.wait(timeout=5)
        except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);p.wait()
def interrupted(sig,_):
    for p in reversed(children):stop_process(p)
    raise SystemExit(128+sig)
for sig in (signal.SIGTERM,signal.SIGHUP,signal.SIGINT):signal.signal(sig,interrupted)
try:
    eligible('pre-creation')
    OUT.mkdir()
    (REPORT/'native-location.json').write_text(json.dumps({'root':str(OUT),'bundle':str(OUT/'Ov930 Native Workflow.app')})+'\n')
    env={**os.environ,'OV930_BROWSER_BRIDGE':str(OUT),'OV930_NATIVE_ROOT':str(OUT),
         'CARGO_TARGET_DIR':str(ROOT.parent/'ov-923-workflow-runtime/target')}
    log=(REPORT/'native-bridge.log').open('w')
    child=subprocess.Popen(['/Users/e-liang/.cargo/bin/cargo','test','--offline','-p','farcooler-client','--lib','workflow_host_browser_bridge','--','--ignored','--nocapture'],cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True);children.append(child)
    deadline=time.monotonic()+120
    while not (OUT/'ready.json').exists():
        assert child.poll() is None and time.monotonic()<deadline,'fixture startup'
        time.sleep(.1)
    app=OUT/'Ov930 Native Workflow.app';(app/'Contents/MacOS').mkdir(parents=True)
    plistlib.dump({'CFBundleExecutable':'Ov930 Native Workflow','CFBundleIdentifier':'dev.farcooler.ov930-native-check','CFBundleName':'Ov930 Native Workflow','CFBundlePackageType':'APPL','LSUIElement':False,'NSHighResolutionCapable':True},(app/'Contents/Info.plist').open('wb'))
    sources=['apps/shared/AgentKit/Sources/AgentKit/'+s+'.swift' for s in ['WorkflowRuns','WorkflowRunsView','WorkflowSession']]+['reports/ov-930/native-check.swift']
    command=['/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/swiftc','-swift-version','6','-target','arm64-apple-macosx26.0','-sdk','/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX27.0.sdk','-module-cache-path','/private/tmp/ov930-checks/swift-cache','-j6',*sources,'-o',str(app/'Contents/MacOS/Ov930 Native Workflow')]
    subprocess.run(command,cwd=ROOT,env=env,check=True,timeout=120)
    binding={s:hashlib.sha256((ROOT/s).read_bytes()).hexdigest() for s in sources}
    binding['executable']=hashlib.sha256((app/'Contents/MacOS/Ov930 Native Workflow').read_bytes()).hexdigest()
    (REPORT/'native-build-binding.json').write_text(json.dumps({'source':'9b80d9a4ad5ed3f3b31d04b3049b0a81731360a3','command':command,'hashes':binding},indent=2)+'\n')
    subprocess.run(['/usr/bin/codesign','--force','--sign','-',str(app)],check=True,timeout=30)
    eligible('pre-launch')
    process=subprocess.Popen([str(app/'Contents/MacOS/Ov930 Native Workflow')],env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True);children.append(process)
    print('PRIVATE_NATIVE_READY '+str(app),flush=True)
    deadline=time.monotonic()+300
    while not (OUT/'finish').exists():
        assert process.poll() is None and child.poll() is None and time.monotonic()<deadline,'native owned deadline/exit'
        eligible('running')
        time.sleep(2)
    eligible('post-capture')
    ready=json.loads((OUT/'ready.json').read_text())
    request=urllib.request.Request('http://127.0.0.1:'+str(ready['port'])+'/call',data=b'{"method":"stop","args":{}}',method='POST')
    urllib.request.urlopen(request,timeout=5).read()
    assert child.wait(timeout=15)==0
    shutil.copy2(OUT/'result.json',REPORT/'native-session-result.json')
    shutil.copy2(OUT/'calls.json',REPORT/'native-calls.json')
    print('native graphical fixture final assertions passed',flush=True)
finally:
    for p in reversed(children):stop_process(p)
    # Keep isolated build/source binding artifacts for review. No installed app.
