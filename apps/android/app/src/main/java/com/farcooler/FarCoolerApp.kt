package com.farcooler

import android.app.Application
import com.farcooler.diagnostics.ProcessExit
import kotlin.concurrent.thread

class FarCoolerApp : Application() {
    override fun onCreate() {
        super.onCreate()

        // Why the last process went away, which until this existed nothing in
        // the app could say. `PaneDeck.MOUNT_LIMIT` was raised from three to
        // five on the understanding that we would watch for real memory
        // pressure instead of designing around a predicted one, and a
        // low-memory kill is silent — so this is the half of that decision that
        // makes the other half a plan rather than a hope. See [ProcessExit].
        //
        // On a plain thread rather than the main one: it is a binder round trip
        // to the system server, it runs before the first frame, and nothing
        // waits on the answer. A coroutine would mean standing up a scope in
        // `Application.onCreate` for one fire-and-forget call that outlives
        // nothing.
        thread(name = "process-exit-report", isDaemon = true) {
            ProcessExit.report(this)
        }
    }
}
