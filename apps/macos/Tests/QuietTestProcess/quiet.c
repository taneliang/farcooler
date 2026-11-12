// The test process may never come to the front (ov-473).
//
// `swift test` loads every test target into one process, and an unbundled
// process that shows a window can be activated by AppKit, taking the
// frontmost spot, the menu bar and the keyboard from whatever the person is
// using. NSApplicationActivationPolicyProhibited makes activation a no-op:
// `NSApp.activate()`, `makeKeyAndOrderFront` and `orderFrontRegardless` all
// leave the person's app in front. Swift has no load-time hook and
// swift-testing no whole-run setup, so a C constructor sets it when the test
// bundle loads, before any test runs.
#include "QuietTestProcess.h"

#include <dispatch/dispatch.h>
#include <objc/message.h>
#include <objc/runtime.h>
#include <pthread.h>
#include <stdlib.h>

// NSApplicationActivationPolicyProhibited.
static const long prohibited = 2;

static void quiet(void *unused) {
    (void)unused;
    Class application = objc_getClass("NSApplication");
    if (application == NULL) return;
    id app = ((id (*)(Class, SEL))objc_msgSend)(application, sel_registerName("sharedApplication"));
    ((signed char (*)(id, SEL, long))objc_msgSend)(app, sel_registerName("setActivationPolicy:"), prohibited);
}

void farcooler_quiet_test_process(void) {
    // NSApplication belongs to the main thread; a bundle loaded from another
    // thread hands the call over rather than making it there.
    if (pthread_main_np()) quiet(NULL);
    else dispatch_async_f(dispatch_get_main_queue(), NULL, quiet);
}

__attribute__((constructor)) static void quiet_at_load(void) { farcooler_quiet_test_process(); }

// Nor may it run an installed Far Cooler's CLI (ov-1091). The test process is
// a Local build, so `CLI.binary` fell through to `~/.local/bin/farcooler-local`
// and every test whose `DaemonClient` had no stub ran the owner's installed
// Local app against the owner's Local install, where an old one wrote its tmux
// config into the leak check's TMPDIR. CI has no app installed; this makes a
// Mac look the same. Here, beside the constructor above, because this file is
// one the linker keeps (`TestWindow` calls into it).
__attribute__((constructor)) static void no_installed_cli_at_load(void) { setenv("FARCOOLER_NO_INSTALLED_CLI", "1", 1); }
