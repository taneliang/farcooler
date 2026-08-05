package com.farcooler.net

import com.farcooler.core.ClientCore
import com.farcooler.data.Host
import com.farcooler.data.Identity
import com.farcooler.model.DaemonBuild
import com.farcooler.model.Fleet
import com.farcooler.model.Repository
import com.farcooler.model.RepositoryList
import com.farcooler.model.Terminal
import com.farcooler.model.Workspace
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.JsonUnquotedLiteral
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive

/**
 * One machine's session and the state a screen renders from it.
 *
 * The rule the whole product rests on holds here too: this never computes a
 * terminal's state. It asks, and shows what the daemon derived. A phone that
 * re-derived could disagree with the daemon and with the Mac about the same
 * terminal, which is exactly the confusion the design removed everywhere else.
 *
 * One of these per machine, always — see [FleetRepository]. That is the same
 * shape the Mac arrived at, and it is why "this machine is down, here is why"
 * has somewhere natural to live instead of being a flag threaded through shared
 * code.
 */
class Connection(val host: Host, private val scope: CoroutineScope) {

    sealed interface Phase {
        data object Connecting : Phase

        /** First contact: the host's fingerprint, awaiting a human. */
        data class NeedsApproval(val fingerprint: String) : Phase

        data class Failed(val message: String) : Phase

        data object Connected : Phase

        /**
         * There WAS a connection, it went away, and one is being made again.
         *
         * A fourth phase rather than a flag on [Connected], because the two
         * existing candidates are each wrong in a way that shows on screen.
         * [Connecting] means "there has never been a fleet"; [Failed] means
         * "stopped, waiting for you", and this is not stopped.
         *
         * Rows keep their place through this. A machine that stops answering
         * keeps its rows, dimmed, rather than dropping them — the rule this
         * app already follows, and the reason it can afford a phase that means
         * "stale, on purpose, for the moment".
         */
        data class Reconnecting(val attempt: Int) : Phase
    }

    /**
     * What a failure MEANS, as opposed to what it says.
     *
     * A failure screen that offers the same button for every failure is one
     * that is wrong most of the time: "Try again" fixes a machine that was
     * asleep and fixes nothing at all about a key this device was never
     * authorised with, or a host key that changed underneath us. Each of these
     * has exactly one useful next move and they are not the same move.
     *
     * Read off the message rather than a typed error because the message is all
     * that crosses the FFI boundary — the core hands back Rust's `Display`
     * output as a string and there is no code to switch on. The substrings are
     * the ones in `crates/client/src/ssh.rs` and `session.rs`; each is a
     * distinctive phrase from the middle of its message rather than a prefix,
     * so wrapping the error in more context does not stop it matching.
     */
    enum class Failure {
        /** The machine answered but does not know this device's key. */
        KEY_REJECTED,

        /**
         * The key presented is not the one we pinned. Retrying is guaranteed to
         * fail, and offering it would suggest this is a glitch rather than a
         * decision someone has to make.
         */
        HOST_KEY_CHANGED,

        /** Nothing answered: wrong address, machine asleep, off the network. */
        UNREACHABLE,

        /** SSH worked; Far Cooler is not installed over there. */
        DAEMON_MISSING,

        /** This device has no usable key, so no machine will ever accept it. */
        NO_IDENTITY,

        /**
         * The user was shown a fingerprint and did not say yes. Not a fault at
         * all — a decision that has been deferred — and the way back is the
         * same screen again, not a retry that pretends something broke.
         */
        KEY_NOT_TRUSTED,

        /** The user stopped waiting. Also not a fault. */
        STOPPED,

        OTHER;

        /**
         * Whether "Try Again" belongs below the primary action as a second
         * option. False where retrying is already the primary action (it would
         * then appear twice) and false where it cannot work at all.
         */
        val worthRetryingAsAlternative: Boolean get() = this == KEY_REJECTED

        companion object {
            fun of(message: String): Failure = when {
                message.contains("rejected this key") -> KEY_REJECTED
                message.contains("is not the one Far Cooler has recorded") -> HOST_KEY_CHANGED
                message.contains("cannot reach") -> UNREACHABLE
                message.contains("did not answer") -> DAEMON_MISSING
                message.contains("no SSH key") -> NO_IDENTITY
                message.contains("has not been trusted") -> KEY_NOT_TRUSTED
                message.contains("Stopped waiting") -> STOPPED
                else -> OTHER
            }
        }
    }

    enum class Action { RESTART, STOP, DISMISS_LOST }

    private val _phase = MutableStateFlow<Phase>(Phase.Connecting)
    val phase: StateFlow<Phase> = _phase.asStateFlow()

    private val _fleet = MutableStateFlow(Fleet.EMPTY)
    val fleet: StateFlow<Fleet> = _fleet.asStateFlow()

    private val _repositories = MutableStateFlow<List<Repository>>(emptyList())
    val repositories: StateFlow<List<Repository>> = _repositories.asStateFlow()

    private val _daemon = MutableStateFlow<DaemonBuild?>(null)
    val daemon: StateFlow<DaemonBuild?> = _daemon.asStateFlow()

    /**
     * Not private: a terminal screen talks to the same machine through this
     * same core, rather than opening a second SSH session just to watch one
     * pane.
     */
    val core = ClientCore()

    /**
     * Bumped every time a session is replaced by a new one.
     *
     * What a live terminal stream watches. A stream is a second SSH channel on
     * the session that just died, and [TerminalSession] recovers from losing
     * one by falling back to polling — correct, and slower than it needs to be
     * once there is a link to stream over again. This is how it finds out
     * there is.
     */
    private val _reconnects = MutableStateFlow(0)
    val reconnects: StateFlow<Int> = _reconnects.asStateFlow()

    private val json = Json { ignoreUnknownKeys = true }
    private var poller: Job? = null

    /**
     * The armed retry, or the attempt in flight. One slot, so a second request
     * to reconnect replaces the first rather than running alongside it — the
     * same rule the Mac's `DaemonClient.retryTask` follows.
     */
    private var reconnector: Job? = null

    /** The machine details the last [start] used, which a retry reconnects to. */
    private var current: Host = host

    /**
     * Which connection attempt is current.
     *
     * [start] awaits the core, and that wait cannot be interrupted from here:
     * the call is a ticket the core resolves whenever the network gets round to
     * it, and a routable address with nothing listening takes as long as the
     * OS's TCP timeout — over a minute — to say so. Giving up therefore cannot
     * stop the work. What it can do is stop this object caring about the
     * answer, which is what the counter is for: every write to [_phase] is
     * guarded on the attempt that produced it still being the current one, so
     * an attempt someone abandoned two screens ago cannot reach up later and
     * change what they are looking at now.
     */
    private var attempt = 0

    /**
     * Which terminal is on screen right now, or null.
     *
     * Set by the terminal screen. Two readers, deliberately one register: a
     * banner about this pane is suppressed, and the same fact is what ends
     * `done`. Suppressing a notification and marking something read are the
     * same judgement — "you are looking at this" — and answering it in two
     * places is how they come to disagree.
     */
    var visibleTerminal: String? = null

    /**
     * Whether this connection is allowed to poll. False while backgrounded.
     *
     * Set through [setForeground] rather than assigned, because coming back is
     * the moment a backoff timer cannot predict and something has to act on
     * it. Still readable directly: [markVisibleSeen] and the poll loop only
     * ask the question.
     */
    @Volatile
    var isForeground: Boolean = true
        private set

    /**
     * The app came to the foreground, or left it.
     *
     * The single most common way this will be experienced: a phone in a pocket
     * for two hours, the process frozen, the sockets dying, and the first
     * thing anyone sees on unlock being a stale screen.
     */
    fun setForeground(foreground: Boolean) {
        val was = isForeground
        isForeground = foreground
        if (!foreground || was) return

        when (_phase.value) {
            // After two hours away, "connected" is a claim rather than a fact.
            // Testing it now beats waiting out a poll interval to find out, and
            // if it holds this is one round trip nobody notices.
            is Phase.Connected -> scope.launch { refresh() }
            is Phase.Reconnecting, is Phase.Failed -> reconnectNow()
            // Already in flight, or waiting on a person. Neither is helped by
            // starting over.
            is Phase.Connecting, is Phase.NeedsApproval -> Unit
        }
    }

    suspend fun start(withHost: Host = host) {
        poller?.cancel()
        reconnector?.cancel()
        current = withHost
        attempt += 1
        val mine = attempt
        _phase.value = Phase.Connecting

        val key = Identity.privateKey()
        if (key == null) {
            if (mine == attempt) {
                _phase.value = Phase.Failed(
                    Identity.lastError
                        ?: "This device has no SSH key and one could not be generated."
                )
            }
            return
        }

        try {
            core.connect(withHost.config(key))
        } catch (e: Exception) {
            e.rethrowIfCancellation()
            if (mine == attempt) _phase.value = classify(e.message.orEmpty())
            return
        }

        if (mine != attempt) return
        _phase.value = Phase.Connected
        refresh()
        loadRepositories()
        loadThemes()
        startPolling()
    }

    // ---- staying connected ----

    /**
     * A call came back saying the link is gone.
     *
     * Detected in [refresh] alone. Every other call site swallows its errors,
     * and chasing all of them would buy nothing: the poller runs every three
     * seconds, so the drop is noticed within one poll of whichever call first
     * hit it, from one place instead of twenty.
     */
    private fun linkDropped() {
        if (_phase.value !is Phase.Connected) return
        poller?.cancel()
        poller = null
        scheduleReconnect(attempt = 1, afterMs = backoffMs(1))
    }

    private fun scheduleReconnect(attempt: Int, afterMs: Long) {
        _phase.value = Phase.Reconnecting(attempt)
        reconnector?.cancel()
        reconnector = scope.launch {
            delay(afterMs)
            reconnect(attempt)
        }
    }

    /**
     * Retry at once, whatever the backoff had planned.
     *
     * The escape hatch for everything a timer cannot know: you walked back
     * into Wi-Fi range, you woke the machine, or you can simply see that this
     * is stuck. Deliberately available while [Phase.Connected] too — that is
     * the case where the app believes it is fine and the person holding it
     * knows better.
     */
    fun reconnectNow() {
        poller?.cancel()
        poller = null
        reconnector?.cancel()
        // A start may still be waiting on the network — this is reachable from
        // Connecting, and a routable address with nothing listening takes over
        // a minute to say so. Bumping the counter is what stops its answer
        // landing on top of this one; see [attempt].
        attempt += 1
        _phase.value = Phase.Reconnecting(0)
        reconnector = scope.launch { reconnect(0) }
    }

    private suspend fun reconnect(attempt: Int) {
        if (_phase.value !is Phase.Reconnecting) return
        val key = Identity.privateKey()
        if (key == null) {
            _phase.value = Phase.Failed(
                Identity.lastError
                    ?: "This device has no SSH key and one could not be generated."
            )
            return
        }

        try {
            core.connect(current.config(key))
        } catch (e: Exception) {
            e.rethrowIfCancellation()
            // A start, or a second reconnectNow, landed while this attempt was
            // crossing the network. Its answer is the current one; this one
            // must not reach up and overwrite it.
            if (_phase.value !is Phase.Reconnecting) return
            retryOrGiveUp(e.message.orEmpty(), attempt)
            return
        }

        if (_phase.value !is Phase.Reconnecting) return
        _phase.value = Phase.Connected
        // Before the reads below, so anything watching for a new link learns
        // about it in the same turn the link exists.
        _reconnects.value += 1
        refresh()
        // Re-read rather than trust what a previous session reported: a
        // machine that dropped and came back may have gained a repository, and
        // staying invisible to the pickers until relaunch is the failure the
        // Mac's `onReconnect` seeding exists to prevent.
        loadRepositories()
        loadThemes()
        startPolling()
    }

    /**
     * What to do about an attempt that failed: wait longer, wait much longer,
     * or stop and say why.
     *
     * Stopping is not a dead end — the row's button, the app coming back to
     * the foreground and the network returning all still reach [reconnectNow].
     * It is the difference between a screen that explains what to fix and one
     * that spins forever over something retrying will never fix.
     */
    private fun retryOrGiveUp(message: String, attempt: Int) {
        val next = classify(message)
        if (next !is Phase.Failed) {
            // The host key is unknown again, which is a question for a human
            // and not something to retry past.
            _phase.value = next
            return
        }

        when (Failure.of(next.message)) {
            Failure.KEY_REJECTED,
            Failure.HOST_KEY_CHANGED,
            Failure.NO_IDENTITY,
            Failure.KEY_NOT_TRUSTED,
            -> _phase.value = next

            // Kept at the same rung: `attempt` drives the fast schedule, means
            // nothing at this cadence, and letting it climb would leave a
            // later, genuinely transient failure starting at the ceiling.
            Failure.DAEMON_MISSING -> scheduleReconnect(attempt, SLOW_RETRY_MS)

            Failure.UNREACHABLE, Failure.STOPPED, Failure.OTHER ->
                scheduleReconnect(attempt + 1, backoffMs(attempt + 1))
        }
    }

    /**
     * Stop waiting, and say so.
     *
     * Not a way to abort the SSH attempt — see [attempt] — but a way off the
     * spinner, which is the thing that was actually missing. A connection to a
     * machine that is asleep shows the same indefinite spinner as one that is
     * about to succeed.
     */
    fun giveUp() {
        abandon("Stopped waiting for ${host.address}. It may be asleep or off the network.")
    }

    /**
     * Back out of the fingerprint question without answering it.
     *
     * Lands on the failure screen rather than the spinner, because that is the
     * screen with the machine switcher, the editor and this device's key on it.
     * The wording is what [Failure.KEY_NOT_TRUSTED] matches on.
     */
    fun declineHostKey() {
        abandon(
            "The key ${host.address} presented has not been trusted on this device. " +
                "Far Cooler won't connect until it is."
        )
    }

    private fun abandon(message: String) {
        attempt += 1
        poller?.cancel()
        // The armed retry too. "Stop waiting" that leaves a backoff ticking
        // underneath would put the spinner back thirty seconds later, which is
        // the opposite of what was asked for.
        reconnector?.cancel()
        _phase.value = Phase.Failed(message)
    }

    /**
     * Turn the core's message into a phase a screen can act on.
     *
     * The unknown-host case is not a failure — it is a question — and it has to
     * be told apart from one, or the user is shown "try again" for something
     * retrying will never fix.
     */
    private fun classify(message: String): Phase {
        fingerprint(message)?.let { return Phase.NeedsApproval(it) }
        return Phase.Failed(message)
    }

    private fun fingerprint(message: String): String? {
        if (!message.contains("is unknown")) return null
        return message.split(" ").firstOrNull { it.startsWith("SHA256:") }
    }

    /**
     * What the daemon on the other end is, asked once per connection.
     *
     * Cached because it cannot change while connected — a daemon that restarted
     * is a connection that dropped — and because the settings screen should not
     * cost a round trip every time it opens.
     */
    suspend fun loadDaemonBuild() {
        if (_phase.value !is Phase.Connected || _daemon.value != null) return
        val body = attempt { core.call("host") }.getOrNull() ?: return
        _daemon.value = DaemonBuild(
            version = body["daemonVersion"]?.jsonPrimitive?.contentOrNull ?: "unknown",
            matches = body["buildsMatch"]?.jsonPrimitive?.booleanOrNull ?: true,
            platform = body["platform"]?.jsonPrimitive?.contentOrNull.orEmpty(),
        )
    }

    /** What a fleet refresh produced, so the app can announce it exactly once. */
    var onFleet: ((Fleet) -> Unit)? = null

    suspend fun refresh() {
        if (_phase.value !is Phase.Connected) return
        try {
            val data = core.call("fleet")
            val fleet = json.decodeFromJsonElement(Fleet.serializer(), data)
            _fleet.value = fleet
            onFleet?.invoke(fleet)
            // And end `done` for whatever is on screen, from the same fleet.
            // Here as well as on the taps, because an agent finishing while you
            // sit reading it is not a tap — it is a poll, and this is the poll.
            markVisibleSeen()
        } catch (e: Exception) {
            e.rethrowIfCancellation()
            // A failed poll is not a disconnection — unless the core says it
            // is. That distinction did not exist before: this swallowed every
            // error, which is right about one poll and wrong about the
            // hundredth, and left the app with no path out of Connected at
            // all. Either way the last known fleet stays on screen rather than
            // blanking the screen someone is reading.
            if (e is com.farcooler.core.DisconnectedException) linkDropped()
            return
        }
    }

    /**
     * Terminals with a `terminal.seen` already in flight, so a poll landing
     * while the last one is still crossing the SSH link does not send a second.
     */
    private val markingSeen = mutableSetOf<String>()

    /**
     * End `done` for the terminal on screen, if anyone is there to see it.
     *
     * `done` is finished-and-UNSEEN. Gated on the app being in the foreground,
     * which is the distinction the feature rests on: an agent finishing while
     * the phone is in a pocket is exactly what the push notification is for,
     * and a screen that happens to still be composed behind a locked phone has
     * not been read by anyone.
     *
     * Only `done`. `blocked` is an agent waiting on an ANSWER — looking at a
     * question does not answer it — and the daemon would refuse to clear it
     * anyway, so sending it would be a round trip spent to be told no.
     *
     * Nothing is applied locally afterwards. The next poll brings the daemon's
     * answer, and this client does not compute a terminal's state.
     */
    suspend fun markVisibleSeen() {
        if (_phase.value !is Phase.Connected || !isForeground) return
        val id = visibleTerminal ?: return
        val terminal = _fleet.value.workspaces.flatMap { it.terminals }.firstOrNull { it.id == id }
        if (terminal?.agent != com.farcooler.model.AgentActivity.DONE) return
        synchronized(markingSeen) { if (!markingSeen.add(id)) return }
        try {
            core.call("terminal.seen", args("terminal" to id))
        } catch (e: Exception) {
            e.rethrowIfCancellation()
        } finally {
            synchronized(markingSeen) { markingSeen.remove(id) }
        }
    }

    /**
     * Merge whatever this machine defines into the picker.
     *
     * Read on every connection and reconnection, alongside repositories, so a
     * `[themes.*]` table added to the host's config.toml does not stay
     * invisible until the app is relaunched.
     */
    private suspend fun loadThemes() {
        val data = attempt { core.call("themes") }.getOrNull() ?: return
        val parsed = runCatching {
            json.decodeFromJsonElement(HostThemes.serializer(), data)
        }.getOrNull() ?: return
        com.farcooler.data.Themes.merge(parsed.themes)
    }

    @kotlinx.serialization.Serializable
    private data class HostThemes(val themes: List<com.farcooler.data.Theme> = emptyList())

    private suspend fun loadRepositories() {
        val data = attempt { core.call("repositories") }.getOrNull() ?: return
        _repositories.value =
            runCatching { json.decodeFromJsonElement(RepositoryList.serializer(), data) }
                .getOrNull()?.repositories ?: emptyList()
    }

    /**
     * Poll while something is watching.
     *
     * Three seconds, not sub-second: every poll is an SSH round trip, and this
     * is a phone with a battery. The states that matter here change on the
     * scale of an agent finishing a task, not a keystroke.
     */
    private fun startPolling() {
        poller = scope.launch {
            while (isActive) {
                delay(POLL_INTERVAL_MS)
                if (isForeground) refresh()
            }
        }
    }

    suspend fun act(action: Action, terminal: Terminal) {
        val method = when (action) {
            Action.RESTART -> "terminal.restart"
            Action.STOP -> "terminal.stop"
            Action.DISMISS_LOST -> "terminal.dismiss_lost"
        }
        attempt { core.call(method, args("terminal" to terminal.id)) }
        refresh()
    }

    /**
     * Hide or unhide a worktree.
     *
     * The Mac has had this since worktree management landed and neither phone
     * app ever got it, which on a machine that adopts every worktree it already
     * has means a sidebar of twenty rows and no way to put nineteen away.
     * Hiding never touches git and is never refused for a running terminal — it
     * is a view preference, not a lifecycle step.
     */
    suspend fun setHidden(workspace: Workspace, hidden: Boolean) {
        val method = if (hidden) "workspace.hide" else "workspace.unhide"
        attempt { core.call(method, args("workspace" to workspace.id)) }
        refresh()
    }

    suspend fun createWorkspace(repository: String, task: String, branch: String): String {
        val data = core.call(
            "workspace.create",
            args("repository" to repository, "task" to task, "branch" to branch, "base" to ""),
        )
        return data["id"]?.jsonPrimitive?.contentOrNull
            ?: throw com.farcooler.core.CoreException("The host created a worktree but did not name it.")
    }

    suspend fun createTerminal(workspace: String, title: String, preset: String): String {
        val data = core.call(
            "terminal.create",
            args("workspace" to workspace, "title" to title, "preset" to preset),
        )
        return data["id"]?.jsonPrimitive?.contentOrNull
            ?: throw com.farcooler.core.CoreException("The host started a terminal but did not name it.")
    }

    /**
     * Send exact bytes to a terminal. [hex] must already be lowercase hex —
     * this does no encoding of its own, because the caller that exists needs
     * the sentence and the carriage return sent as two separate calls, not two
     * strings joined into one payload here.
     */
    suspend fun writeRaw(terminal: String, hex: String) {
        core.call("terminal.write", args("terminal" to terminal, "hex" to hex))
    }

    /**
     * Switch a pane between its terminal and its chat.
     *
     * Refreshes afterwards rather than guessing: the daemon respawns the pane,
     * and what comes back — a new epoch, a different pane mode, possibly a
     * refusal because a turn was in flight — is its answer to give, not this
     * client's to assume.
     */
    suspend fun setPaneMode(terminal: Terminal, mode: String) {
        attempt {
            core.call("terminal.set_pane_mode", args("terminal" to terminal.id, "paneMode" to mode))
        }
        refresh()
    }

    fun terminal(id: String): Terminal? =
        _fleet.value.workspaces.flatMap { it.terminals }.firstOrNull { it.id == id }

    fun workspaceOf(terminalId: String): Workspace? =
        _fleet.value.workspaces.firstOrNull { workspace ->
            workspace.terminals.any { it.id == terminalId }
        }

    suspend fun close() {
        poller?.cancel()
        core.close()
    }

    companion object {
        private const val POLL_INTERVAL_MS = 3_000L

        /**
         * How long a machine that answered SSH but not Far Cooler waits.
         *
         * Five minutes, matching the Mac. No amount of retrying installs a
         * daemon, so the exponential schedule — which exists to survive a
         * burst of transient failures quickly — is the wrong tool and would
         * just be noise every thirty seconds forever. Not giving up either:
         * installing it later should be noticed without relaunching the app.
         */
        private const val SLOW_RETRY_MS = 300_000L

        /**
         * How long to wait before the next attempt.
         *
         * The same schedule as the Mac's `DaemonClient.backoffSeconds` and the
         * iPhone's `Connection.backoff`, deliberately: "how long until it comes
         * back" should have one answer across the three apps. Doubling from two
         * seconds to a thirty second ceiling, with jitter, because several
         * machines recovering from one network event must not retry in
         * lockstep — and this app connects to every machine at once, so that is
         * the ordinary case here rather than the unlucky one.
         */
        fun backoffMs(attempt: Int): Long {
            val ceiling = minOf(30.0, Math.pow(2.0, attempt.toDouble()))
            return (ceiling * (0.8 + Math.random() * 0.4) * 1000).toLong()
        }

        /**
         * A JSON object from string pairs, which is every call this makes.
         *
         * `ULong` is spelled out rather than falling through to `toString`,
         * because the difference is a quoted string versus a number and the
         * host reads these with `as_u64()` — which answers `None` for a string
         * and silently defaults. See [ScreenResponse.revision] for the value
         * that needs the full unsigned range.
         */
        @OptIn(ExperimentalSerializationApi::class)
        fun args(vararg pairs: Pair<String, Any>): JsonObject = JsonObject(
            pairs.associate { (key, value) ->
                key to when (value) {
                    is Int -> JsonPrimitive(value)
                    is Long -> JsonPrimitive(value)
                    is Boolean -> JsonPrimitive(value)
                    // Above `Long.MAX_VALUE` there is no `Number` that holds
                    // the value, so it goes out as an unquoted literal — a JSON
                    // number that Kotlin never had a type for.
                    is ULong -> JsonUnquotedLiteral(value.toString())
                    else -> JsonPrimitive(value.toString())
                }
            }
        )
    }
}
