#!/usr/bin/python3
"""Run inside scripts/heavy; bound owned processes, disk and output."""
import glob
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time
ROOT = Path(__file__).resolve().parents[3]
OUT = Path('/private/tmp/ov926-review-checks')
OUT.mkdir(exist_ok=True)
REPORT = ROOT / 'reports/ov-926/review-fixes'

def run(name, command, timeout=180, expected=0, env=None):
    if shutil.disk_usage(ROOT).free < 10 * 1024**3:
        raise RuntimeError('10GiB reserve')
    with (REPORT / (name + '.log')).open('w') as log:
        log.write('COMMAND ' + repr(command) + '\n'); log.flush()
        child = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True, env=env)
        start = time.monotonic()
        def stop_owned():
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGTERM)
                try: child.wait(timeout=5)
                except subprocess.TimeoutExpired: os.killpg(child.pid, signal.SIGKILL); child.wait()
        previous = {}
        def interrupted(signum, _frame):
            stop_owned()
            raise SystemExit(128 + signum)
        for signum in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
            previous[signum] = signal.signal(signum, interrupted)
        try:
            while child.poll() is None:
                if time.monotonic() - start > timeout or shutil.disk_usage(ROOT).free < 10 * 1024**3:
                    stop_owned()
                    raise RuntimeError('owned timeout/reserve stop: ' + name)
                time.sleep(.25)
        finally:
            stop_owned()
            for signum, handler in previous.items(): signal.signal(signum, handler)
        log.write('\nEXIT ' + str(child.returncode) + '\n')
    print(name, child.returncode, flush=True)
    if child.returncode != expected: raise RuntimeError(name)

TS = '/Users/e-liang/.npm/_npx/67eb4586ca667318/node_modules/typescript/bin/tsc'
NODE = '/opt/homebrew/bin/node'
mode = sys.argv[1]
if mode == 'suite':
    for check in ('web', 'swift', 'kotlin', 'kotlin-view', 'negative', 'negative-swift', 'negative-kotlin', 'browser'):
        subprocess.run(['/usr/bin/python3', __file__, check], check=True)
elif mode == 'web':
    run('web-typecheck', [NODE, TS, '--strict', '--target', 'es2022', '--module', 'commonjs', '--lib', 'es2022,dom,dom.iterable', '--outDir', str(OUT/'web'), 'services/web/src/client/workflow.ts', 'services/web/src/client/workflow-view.ts'])
    run('web-fixtures', [NODE, 'services/web/test/workflow.test.mjs', str(OUT/'web')])

elif mode == 'negative':
    old = OUT / 'old-web'
    old.mkdir(exist_ok=True)
    for name in ('workflow.ts', 'workflow-view.ts'):
        (old / name).write_bytes(subprocess.check_output(['git', 'show', 'db43f6393:services/web/src/client/' + name], cwd=ROOT))
    run('negative-web-compile', [NODE, TS, '--strict', '--target', 'es2022', '--module', 'commonjs', '--lib', 'es2022,dom,dom.iterable', '--outDir', str(old), str(old/'workflow.ts'), str(old/'workflow-view.ts')])
    for name, match in [('r1-selection', 'late inspection'), ('r1-replacement', 'run replacement'), ('r2-focus', 'stable focus'), ('r3-disconnect', 'old failed action')]:
        run('negative-' + name, [NODE, 'services/web/test/workflow.test.mjs', str(old)], expected=1, env={**os.environ, 'WORKFLOW_TEST_FILTER': match})
elif mode == 'browser':
    run('browser-focus', [NODE, 'reports/ov-926/review-fixes/browser-focus.mjs', str(OUT/'web')], 60)
elif mode in ('swift', 'negative-swift'):
    # SDK27 shipped prebuilt modules match installed Swift6.4. The older
    # AgentKit SDK26.5 cache is incompatible and deliberately NOT copied.
    swift = '/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/swiftc'
    sdk = '/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX27.0.sdk'
    model = 'apps/shared/AgentKit/Sources/AgentKit/WorkflowRuns.swift'
    view = 'apps/shared/AgentKit/Sources/AgentKit/WorkflowRunsView.swift'
    negative = mode == 'negative-swift'
    if negative:
        for source in (model, view):
            (OUT / Path(source).name).write_bytes(subprocess.check_output(['git', 'show', 'db43f6393:' + source], cwd=ROOT))
        model, view = str(OUT/'WorkflowRuns.swift'), str(OUT/'WorkflowRunsView.swift')
    prefix = 'negative-' if negative else ''
    run(prefix + 'swift-compile', [swift, '-swift-version', '6', '-target', 'arm64-apple-macosx26.0', '-sdk', sdk, '-module-cache-path', str(OUT/'swift-cache'), '-j6', model, view, '-DWORKFLOW_STANDALONE', 'apps/shared/AgentKit/Tests/AgentKitTests/WorkflowLifecycleChecks.swift', 'reports/ov-926/review-fixes/swift-check.swift', '-o', str(OUT/'swift-check')], 180)
    run(prefix + 'swift-fixtures', [str(OUT/'swift-check'), 'crates/client/src/testdata/workflow-presentation.json'], expected=1 if negative else 0)
    if not negative:
        run('swift-test-module', [swift, '-swift-version', '6', '-target', 'arm64-apple-macosx26.0', '-sdk', sdk, '-module-cache-path', str(OUT/'swift-cache'), '-emit-module', '-enable-testing', '-module-name', 'AgentKit', model, view, '-emit-module-path', str(OUT/'AgentKit.swiftmodule')])
        run('swift-test-typecheck', [swift, '-swift-version', '6', '-target', 'arm64-apple-macosx26.0', '-sdk', sdk, '-module-cache-path', str(OUT/'swift-cache'), '-typecheck', '-plugin-path', '/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/host/plugins/testing', '-F', '/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/Library/Frameworks', '-I', str(OUT), 'apps/shared/AgentKit/Tests/AgentKitTests/WorkflowRunsTests.swift', 'apps/shared/AgentKit/Tests/AgentKitTests/WorkflowLifecycleChecks.swift'])
elif mode == 'swift-tests':
    swift = '/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/swiftc'
    sdk = '/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX27.0.sdk'
    run('swift-test-typecheck', [swift, '-swift-version', '6', '-target', 'arm64-apple-macosx26.0', '-sdk', sdk, '-module-cache-path', str(OUT/'swift-cache'), '-typecheck', '-plugin-path', '/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/host/plugins/testing', '-F', '/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/Library/Frameworks', '-I', str(OUT), 'apps/shared/AgentKit/Tests/AgentKitTests/WorkflowRunsTests.swift', 'apps/shared/AgentKit/Tests/AgentKitTests/WorkflowLifecycleChecks.swift'])
elif mode in ('kotlin', 'kotlin-view', 'negative-kotlin'):
    cache = '/private/tmp/ov892-gate-1/gradle/caches/modules-2/files-2.1/'
    def jar(group, artifact, version):
        found = glob.glob(cache + group + '/' + artifact + '/' + version + '/*/*.jar')
        if len(found) != 1: raise RuntimeError(found)
        return found[0]
    std = jar('org.jetbrains.kotlin', 'kotlin-stdlib', '2.4.10')
    coroutines = jar('org.jetbrains.kotlinx', 'kotlinx-coroutines-core-jvm', '1.11.0')
    compiler = [jar('org.jetbrains.kotlin', 'kotlin-compiler-embeddable', '2.4.10'), std, coroutines,
        jar('org.jetbrains.kotlin', 'kotlin-script-runtime', '2.4.10'), jar('org.jetbrains.kotlin', 'kotlin-reflect', '1.6.10')]
    deps = [std, coroutines, jar('org.jetbrains.kotlinx', 'kotlinx-serialization-json-jvm', '1.9.0'), jar('org.jetbrains.kotlinx', 'kotlinx-serialization-core-jvm', '1.9.0'), jar('junit', 'junit', '4.13.2'), jar('org.hamcrest', 'hamcrest-core', '1.3'), jar('org.jetbrains', 'annotations', '23.0.0')]
    java = '/opt/homebrew/opt/openjdk@21/bin/java'
    if mode == 'kotlin-view':
        ui = sorted(glob.glob('/private/tmp/ov892-gate-1/gradle/caches/9.6.1/transforms/*/transformed/*/jars/classes.jar'))
        ui += glob.glob(cache + 'androidx.*/*/*/*/*.jar')
        ui.append('/Users/e-liang/Library/Android/sdk/platforms/android-37.0/android.jar')
        run('kotlin-compose', [java, '-Xmx1g', '-cp', ':'.join(compiler + deps), 'org.jetbrains.kotlin.cli.jvm.K2JVMCompiler', '-no-stdlib', '-no-reflect', '-Xplugin=' + jar('org.jetbrains.kotlin', 'kotlin-compose-compiler-plugin-embeddable', '2.4.10'), '-classpath', ':'.join(deps + ui), '-d', str(OUT/'compose'), 'apps/android/app/src/main/java/com/farcooler/ui/WorkflowRuns.kt', 'apps/android/app/src/main/java/com/farcooler/ui/WorkflowRunsView.kt'])
    else:
        negative = mode == 'negative-kotlin'
        model = 'apps/android/app/src/main/java/com/farcooler/ui/WorkflowRuns.kt'
        if negative:
            (OUT/'WorkflowRuns.kt').write_bytes(subprocess.check_output(['git', 'show', 'db43f6393:' + model], cwd=ROOT))
            model = str(OUT/'WorkflowRuns.kt')
        prefix = 'negative-' if negative else ''
        run(prefix + 'kotlin-compile', [java, '-Xmx1g', '-cp', ':'.join(compiler + deps), 'org.jetbrains.kotlin.cli.jvm.K2JVMCompiler', '-no-stdlib', '-no-reflect', '-classpath', ':'.join(deps), '-d', str(OUT/'kotlin'), model, 'apps/android/app/src/test/java/com/farcooler/ui/WorkflowRunsTest.kt'])
        run(prefix + 'kotlin-fixtures', [java, '-Xmx512m', '-cp', ':'.join([str(OUT/'kotlin')] + deps), 'org.junit.runner.JUnitCore', 'com.farcooler.ui.WorkflowRunsTest'], expected=1 if negative else 0)
