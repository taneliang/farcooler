#!/usr/bin/python3
"""Scoped checks with normal FIFO, an owned deadline and 10GiB reserve."""
import ast, hashlib, json, os, shutil, signal, subprocess, sys, time
from pathlib import Path
ROOT = Path(__file__).resolve().parents[2]
OUT = Path('/private/tmp/ov930-checks')
OUT.mkdir(exist_ok=True)
REPORT = ROOT / 'reports/ov-930'
mode = sys.argv[1]
label = sys.argv[2] if len(sys.argv) > 2 else mode
NODE = '/opt/homebrew/bin/node'
TS = '/Users/e-liang/.npm/_npx/67eb4586ca667318/node_modules/typescript/bin/tsc'
env = {**os.environ, 'PATH': '/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/Users/e-liang/.cargo/bin',
       'CARGO_BUILD_JOBS': '6', 'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_PROFILE_TEST_DEBUG': '0', 'CARGO_INCREMENTAL': '0'}
if mode == 'web':
    command = [NODE, TS, '--strict', '--target', 'es2022', '--module', 'commonjs', '--lib', 'es2022,dom,dom.iterable', '--outDir', str(OUT/'web'),
               *['services/web/src/client/'+p+'.ts' for p in ['workflow','workflow-view','workflow-session']]]
elif mode == 'browser-focus': command = [NODE,'reports/ov-926/review-fixes/browser-focus.mjs',str(OUT/'web')]
elif mode == 'web-baseline': command = [NODE,'reports/ov-930/web-baseline.cjs']
elif mode == 'negative': command = ['/usr/bin/python3','reports/ov-930/negative.py']
elif mode == 'web-test': command = [NODE, 'services/web/test/workflow-session.test.mjs', str(OUT/'web')]
elif mode == 'web-lifecycle': command = [NODE, 'services/web/test/workflow.test.mjs', str(OUT/'web')]
elif mode == 'swift':
    command = ['/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/swiftc', '-swift-version', '6', '-target', 'arm64-apple-macosx26.0', '-sdk', '/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX27.0.sdk', '-module-cache-path', str(OUT/'swift-cache'), '-j6',
               *['apps/shared/AgentKit/Sources/AgentKit/'+p+'.swift' for p in ['WorkflowRuns','WorkflowRunsView','WorkflowSession']],
               '-DWORKFLOW_STANDALONE', 'apps/shared/AgentKit/Tests/AgentKitTests/WorkflowLifecycleChecks.swift', 'reports/ov-930/swift-check.swift', '-o', str(OUT/'swift-check')]
elif mode == 'swift-test': command = [str(OUT/'swift-check'), 'crates/client/src/testdata/workflow-presentation.json']
elif mode in ('kotlin','kotlin-test','kotlin-compose'):
    # Reuse read-only, content-addressed compiler/dependency JARs listed by the
    # reviewed ov-926 evidence. Outputs are private to this task.
    old = ast.literal_eval((ROOT/'reports/ov-926/review-fixes/kotlin-compile.log').read_text().splitlines()[0][8:])
    if mode == 'kotlin-compose':
        old = ast.literal_eval((ROOT/'reports/ov-926/review-fixes/kotlin-compose.log').read_text().splitlines()[0][8:])
    if mode != 'kotlin-test':
        old[old.index('-d')+1] = str(OUT/('compose' if mode == 'kotlin-compose' else 'kotlin'))
        old.append('apps/android/app/src/main/java/com/farcooler/ui/WorkflowSession.kt')
        if mode == 'kotlin': old.append('apps/android/app/src/test/java/com/farcooler/ui/WorkflowSessionTest.kt')
        command = old
    else:
        deps = old[old.index('-classpath')+1]
        command = [old[0], '-Xmx512m', '-cp', str(OUT/'kotlin')+':'+deps, 'org.junit.runner.JUnitCore', 'com.farcooler.ui.WorkflowRunsTest', 'com.farcooler.ui.WorkflowSessionTest']
elif mode in ('rust','rust-lint','jni','wasm','browser','native'):
    target = ROOT.parent/'ov-923-workflow-runtime/target'
    # Pin/check provenance from reviewed ov-927 and verify the previous cargo
    # holder actually released its lock before entering normal machine FIFO.
    old = json.loads((ROOT/'reports/ov-927/final-client.json').read_text())
    assert Path(old['target']) == target and old['exit'] == 0 and old['source_unchanged']
    env['CARGO_TARGET_DIR'] = str(target)
    command = ['/Users/e-liang/.cargo/bin/cargo', *{
        'rust':['test','--offline','-p','farcooler-client','--lib','workflow_','--','--nocapture'],
        'rust-lint':['clippy','--offline','-p','farcooler-client','--lib','--','-D','warnings'],
        'jni':['check','--offline','-p','farcooler-jni'],
        'wasm':['check','--offline','--manifest-path','crates/web/Cargo.toml','--target','wasm32-unknown-unknown'],
        'browser':[],
        'native':[],
    }[mode]]
    if mode == 'browser': command = ['/usr/bin/python3','reports/ov-930/bridge.py']
    if mode == 'native': command = ['/usr/bin/python3','reports/ov-930/native.py']
    command = ['/usr/bin/python3','reports/ov-930/entry.py',str(target),*command]
elif mode == 'ios-generator':
    env['CLANG_MODULE_CACHE_PATH']=str(OUT/'generator-cache')
    env['SWIFT_MODULECACHE_PATH']=str(OUT/'generator-cache')
    command=['/usr/bin/python3','apps/ios/generate-project.py']
elif mode == 'swift-host-parse':
    command=['/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/swiftc','-frontend','-parse',
        'apps/ios/FarCooler/Connection.swift','apps/ios/FarCooler/ConnectionWorkflow.swift','apps/ios/FarCooler/WorkspaceScreen.swift',
        'apps/macos/Sources/FarCooler/NativeAgent/NativeAgents.swift','apps/macos/Sources/FarCooler/ContentView+WorkspaceDetail.swift']
elif mode == 'static': command = ['/usr/bin/python3', 'scripts/doc-comment-check.py']
else: raise RuntimeError(mode)
command = ['/usr/bin/python3', str(ROOT/'scripts/heavy'), '--load-limit', '20', '--', *command]
paths = subprocess.check_output(['git','diff','--name-only'],cwd=ROOT,text=True).splitlines()
paths += ['services/web/src/client/workflow-session.ts','apps/shared/AgentKit/Sources/AgentKit/WorkflowSession.swift','apps/android/app/src/main/java/com/farcooler/ui/WorkflowSession.kt']
def hashes(): return {p:hashlib.sha256((ROOT/p).read_bytes()).hexdigest() for p in paths if (ROOT/p).is_file()}
before=hashes(); start=time.monotonic(); minimum=shutil.disk_usage(ROOT).free; stopped=None
with (REPORT/(label+'.log')).open('w') as log:
    log.write('COMMAND '+repr(command)+'\n');log.flush()
    child=subprocess.Popen(command,cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    def stop():
        if child.poll() is None:
            os.killpg(child.pid,signal.SIGTERM)
            try:child.wait(timeout=5)
            except subprocess.TimeoutExpired:os.killpg(child.pid,signal.SIGKILL);child.wait()
    def interrupted(sig,_): stop();raise SystemExit(128+sig)
    for sig in (signal.SIGTERM,signal.SIGHUP,signal.SIGINT):signal.signal(sig,interrupted)
    try:
        while child.poll() is None:
            minimum=min(minimum,shutil.disk_usage(ROOT).free)
            if minimum<10*1024**3 or time.monotonic()-start>(900 if mode == 'native' else 600):stopped='deadline/reserve';break
            time.sleep(.25)
    finally:stop()
    log.write('\nEXIT '+str(child.returncode)+'\n')
record=dict(command=command,exit=child.returncode,stopped=stopped,seconds=time.monotonic()-start,minimum_free=minimum,before=before,after=hashes())
(REPORT/(label+'.json')).write_text(json.dumps(record,indent=2)+'\n')
print(label,child.returncode,stopped)
sys.exit(child.returncode if not stopped and before==hashes() else 1)
