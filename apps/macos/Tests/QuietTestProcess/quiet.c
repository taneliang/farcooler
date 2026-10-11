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
