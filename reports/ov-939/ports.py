#!/usr/bin/python3
"""Finite cached native compiler/model checks; never launches an app or browser."""
import ast,hashlib,json,subprocess,sys
from pathlib import Path
root=Path(__file__).resolve().parents[2]
out=Path('/private/tmp/ov939-ports')
report=root/'reports/ov-939'
node='/opt/homebrew/bin/node'
ts='/Users/e-liang/.npm/_npx/67eb4586ca667318/node_modules/typescript/bin/tsc'
swift='/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/swiftc'
models=['WorkflowRuns','WorkflowRunsView','WorkflowSession']
kotlin=ast.literal_eval((root/'reports/ov-926/review-fixes/kotlin-compile.log').read_text().splitlines()[0][8:])
compose=ast.literal_eval((root/'reports/ov-926/review-fixes/kotlin-compose.log').read_text().splitlines()[0][8:])
deps=kotlin[kotlin.index('-classpath')+1]
for args,folder in [(kotlin,'kotlin'),(compose,'compose')]:
    args[args.index('-d')+1]=str(out/folder)
    args.append('apps/android/app/src/main/java/com/farcooler/ui/WorkflowSession.kt')
kotlin.append('apps/android/app/src/test/java/com/farcooler/ui/WorkflowSessionTest.kt')
commands=[
 ('ts',[node,ts,'--strict','--target','es2022','--module','commonjs','--lib','es2022,dom,dom.iterable','--outDir',str(out/'web'),
       *['services/web/src/client/'+p+'.ts' for p in ['workflow','workflow-view','workflow-session']]]),
 ('web-session',[node,'services/web/test/workflow-session.test.mjs',str(out/'web')]),
 ('web-lifecycle',[node,'services/web/test/workflow.test.mjs',str(out/'web')]),
 ('swift',[swift,'-swift-version','6','-target','arm64-apple-macosx26.0','-sdk','/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX27.0.sdk','-module-cache-path',str(out/'swift-cache'),'-j6',
          *['apps/shared/AgentKit/Sources/AgentKit/'+p+'.swift' for p in models],'-DWORKFLOW_STANDALONE',
          'apps/shared/AgentKit/Tests/AgentKitTests/WorkflowLifecycleChecks.swift','reports/ov-939/swift-check.swift','-o',str(out/'swift-check')]),
 ('swift-test',[str(out/'swift-check'),'crates/client/src/testdata/workflow-presentation.json']),
 ('kotlin',kotlin),
 ('kotlin-test',[kotlin[0],'-Xmx512m','-cp',str(out/'kotlin')+':'+deps,'org.junit.runner.JUnitCore','com.farcooler.ui.WorkflowRunsTest','com.farcooler.ui.WorkflowSessionTest']),
 ('kotlin-compose',compose),
]
inputs=set([node,ts,swift,kotlin[0]])
for args in [kotlin,compose]:
    for option in ['-classpath','-cp']:
        if option in args:inputs.update(args[args.index(option)+1].split(':'))
    inputs.update(a.split('=',1)[1] for a in args if a.startswith('-Xplugin='))
missing=[p for p in sorted(inputs) if not Path(p).exists()]
plan={'commands':commands,'missing':missing,'cached_inputs':{p:hashlib.sha256(Path(p).read_bytes()).hexdigest() for p in sorted(inputs) if Path(p).is_file()}}
if sys.argv[1:] == ['plan']:
    (report/'ports-plan.json').write_text(json.dumps(plan,indent=2)+'\n');print('cached compiler inputs',len(inputs),'missing',missing);sys.exit(bool(missing))
assert not missing,missing
out.mkdir(exist_ok=True)
start_at=sys.argv[1] if len(sys.argv)>1 else commands[0][0]
assert start_at in dict(commands)
for label,cmd in commands:
    if label==start_at:start_at=None
    if start_at:continue
    print('PORT',label,repr(cmd),flush=True)
    path=report/('port-'+label+'.log')
    # Retain originals; a corrected attempt gets its own explicit suffix.
    suffix=sys.argv[2] if len(sys.argv)>2 else ''
    if suffix:path=report/('port-'+label+'-'+suffix+'.log')
    with path.open('x') as log:
        log.write('COMMAND '+repr(cmd)+'\n');log.flush()
        result=subprocess.run(cmd,cwd=root,stdout=log,stderr=subprocess.STDOUT)
        log.write('\nEXIT '+str(result.returncode)+'\n')
    print(label,'exit',result.returncode,flush=True)
    if result.returncode:sys.exit(result.returncode)
    if len(sys.argv)>3 and label==sys.argv[3]:break
