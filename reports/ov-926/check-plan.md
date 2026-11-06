# Bounded verification plan

No UI launch/capture, simulator or device allocation requested. Source first.
Disk preflight: 40 GiB available; preserve >=10 GiB and stop/reap owned check
process if reserve crossed. No dependency downloads or all-platform rebuild.

Use normal scripts/heavy FIFO (5-minute load <20), jobs <=6 and native
/usr/bin/python3 supervision for compiler/test executions. Use cached TS5.9.3
at ~/.npm/_npx/67eb4586ca667318/node_modules/typescript, Node26.13.0. No npm install.
Node tests compile only two new modules to /private/tmp/ov926-checks then run
fixture/controller/DOM tests without opening a browser.

Swift: baseline AgentKit debug description records Xcode.app SDK26.5 and its
ModuleCache (164 MiB). Clone this cache once via APFS cp -cR into owned /tmp,
then compile/typecheck only the two new shared files plus focused harness,
using the exact recorded SDK/toolchain and no full AgentKit dependency build.
A cache/toolchain mismatch must be reported rather than relabelled warm proof.

Kotlin: reuse cached Kotlin2.4.10 compiler and kotlinx1.9 serialization/coroutines
jars in /private/tmp/ov892-gate-1/gradle, compiling only WorkflowRuns model and
focused test. Android Compose/app builds require a proven matching warm build
allocation; do not copy/download a cold app tree to manufacture full build proof.

Pure fixture/compiler proof is distinct from graphical app build, actual
navigation, narrow-layout screenshot, accessibility/input and reconnect/runtime
proof. Unavailable checks stay explicit in the handoff.

Preflight correction: existing AgentKit cache was Swift6.3.3/SDK26.5, but current
Xcode.app is Swift6.4/SDK27 (26.5 SDK removed). Do NOT reuse/relabel that cache.
Current Xcode ships matching prebuilt SwiftUI/Foundation/Combine modules under
Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx/prebuilt-modules/27.0.
Use those SDK modules for only the two-file focused executable/typecheck, with
an owned temporary Clang cache, 180s timeout, no SwiftPM/full app build. This is
a bounded focused compile; it is not a warm full-AgentKit/app build claim.

After cached Kotlin model tests pass, compile only WorkflowRuns{,View}.kt
with cached Compose compiler2.4.10, extracted AndroidX AAR classes in that SAME
Gradle9.6.1 cache and installed Android37 android.jar. No app build/native JNI
link/Gradle resolution/device execution; output remains owned /tmp/ov926-checks.
