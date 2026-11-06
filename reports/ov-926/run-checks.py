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
ROOT = Path(__file__).resolve().parents[2]
OUT = Path('/private/tmp/ov926-checks')
OUT.mkdir(exist_ok=True)
REPORT = ROOT / 'reports/ov-926'

def run(name, command, timeout=180):
    if shutil.disk_usage(ROOT).free < 10 * 1024**3:
        raise RuntimeError('10GiB reserve')
    with (REPORT / (name + '.log')).open('w') as log:
        log.write('COMMAND ' + repr(command) + '\n'); log.flush()
        child = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
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
    if child.returncode: raise RuntimeError(name)

TS = '/Users/e-liang/.npm/_npx/67eb4586ca667318/node_modules/typescript/bin/tsc'
NODE = '/opt/homebrew/bin/node'
mode = sys.argv[1]
if mode == 'web':
    run('web-typecheck', [NODE, TS, '--strict', '--target', 'es2022', '--module', 'commonjs', '--lib', 'es2022,dom,dom.iterable', '--outDir', str(OUT/'web'), 'services/web/src/client/workflow.ts', 'services/web/src/client/workflow-view.ts'])
    run('web-fixtures', [NODE, 'services/web/test/workflow.test.mjs', str(OUT/'web')])
    run('web-baseline', [NODE, 'reports/ov-926/web-baseline.cjs'])
elif mode == 'swift':
    # SDK27 shipped prebuilt modules match installed Swift6.4. The older
    # AgentKit SDK26.5 cache is incompatible and deliberately NOT copied.
    swift = '/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/swiftc'
    sdk = '/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX27.0.sdk'
    run('swift-compile', [swift, '-swift-version', '6', '-target', 'arm64-apple-macosx26.0', '-sdk', sdk, '-module-cache-path', str(OUT/'swift-cache'), '-j6', 'apps/shared/AgentKit/Sources/AgentKit/WorkflowRuns.swift', 'apps/shared/AgentKit/Sources/AgentKit/WorkflowRunsView.swift', 'reports/ov-926/swift-check.swift', '-o', str(OUT/'swift-check')], 180)
    run('swift-fixtures', [str(OUT/'swift-check'), 'crates/client/src/testdata/workflow-presentation.json'])
    run('swift-test-module', [swift, '-swift-version', '6', '-target', 'arm64-apple-macosx26.0', '-sdk', sdk, '-module-cache-path', str(OUT/'swift-cache'), '-emit-module', '-enable-testing', '-module-name', 'AgentKit', 'apps/shared/AgentKit/Sources/AgentKit/WorkflowRuns.swift', 'apps/shared/AgentKit/Sources/AgentKit/WorkflowRunsView.swift', '-emit-module-path', str(OUT/'AgentKit.swiftmodule')])
    run('swift-test-typecheck', [swift, '-swift-version', '6', '-target', 'arm64-apple-macosx26.0', '-sdk', sdk, '-module-cache-path', str(OUT/'swift-cache'), '-typecheck', '-plugin-path', '/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/host/plugins/testing', '-F', '/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/Library/Frameworks', '-I', str(OUT), 'apps/shared/AgentKit/Tests/AgentKitTests/WorkflowRunsTests.swift'])
elif mode == 'swift-tests':
    swift = '/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin/swiftc'
    sdk = '/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX27.0.sdk'
    run('swift-test-typecheck', [swift, '-swift-version', '6', '-target', 'arm64-apple-macosx26.0', '-sdk', sdk, '-module-cache-path', str(OUT/'swift-cache'), '-typecheck', '-plugin-path', '/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/host/plugins/testing', '-F', '/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/Library/Frameworks', '-I', str(OUT), 'apps/shared/AgentKit/Tests/AgentKitTests/WorkflowRunsTests.swift'])
elif mode in ('kotlin', 'kotlin-view'):
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
        run('kotlin-compile', [java, '-Xmx1g', '-cp', ':'.join(compiler + deps), 'org.jetbrains.kotlin.cli.jvm.K2JVMCompiler', '-no-stdlib', '-no-reflect', '-classpath', ':'.join(deps), '-d', str(OUT/'kotlin'), 'apps/android/app/src/main/java/com/farcooler/ui/WorkflowRuns.kt', 'apps/android/app/src/test/java/com/farcooler/ui/WorkflowRunsTest.kt'])
        run('kotlin-fixtures', [java, '-Xmx512m', '-cp', ':'.join([str(OUT/'kotlin')] + deps), 'org.junit.runner.JUnitCore', 'com.farcooler.ui.WorkflowRunsTest'])
