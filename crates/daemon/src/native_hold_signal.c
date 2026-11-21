#include <signal.h>
#include <stdatomic.h>
#include <stdbool.h>

// A signal can interrupt any thread. A store to this flag must never take a lock.
_Static_assert(ATOMIC_BOOL_LOCK_FREE == 2, "the hangup flag must always be lock-free");
static atomic_bool hangup = false;

// Keep this callback private to C and limited to an async-signal-safe store.
static void on_hangup(int signal_number) {
    (void)signal_number;
    atomic_store(&hangup, true);
}

void farcooler_native_hold_install_signals(void) {
    signal(SIGHUP, on_hangup);
    signal(SIGTERM, on_hangup);
}

bool farcooler_native_hold_hangup_pending(bool clear) {
    return clear ? atomic_exchange(&hangup, false) : atomic_load(&hangup);
}
