package com.farcooler.net

import com.farcooler.core.ClientCore
import com.farcooler.data.Runner
import com.farcooler.data.Identity
import com.farcooler.data.Theme
import com.farcooler.model.AdapterInfo
import com.farcooler.model.AdapterTestOutcome
import com.farcooler.model.DaemonBuild
import com.farcooler.model.Fleet
import com.farcooler.model.InboxReply
import com.farcooler.model.InboxRow
import com.farcooler.model.Repository
import com.farcooler.model.RepositoryList
import com.farcooler.model.Terminal
import com.farcooler.model.Workspace
import com.farcooler.model.toJson
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
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonPrimitive

/**
 * One runner's session and the state a screen renders from it.
 *
 * The rule the whole product rests on holds here too: this never computes a
 * terminal's state. It asks, and shows what the daemon derived. A phone that
 * re-derived could disagree with the daemon and with the Mac about the same
 * terminal, which is exactly the confusion the design removed everywhere else.
 *
 * One of these per runner, always — see [FleetRepository]. That is the same
 * shape the Mac arrived at, and it is why "this runner is down, here is why"
 * has somewhere natural to live instead of being a flag threaded through shared
 * code.
 */
class Connection(val host: Runner, private val scope: CoroutineScope) {

    sealed interface Phase {
        data object Connecting : Phase

        /** First contact: the host's fingerprint, awaiting a human. */
        data class NeedsApproval(val fingerprint: String) : Phase

        /**
         * Stopped, with a reason, and a next move that depends on which reason.
         *
         * [kind] is what that reason MEANS. Defaulted to reading it off
         * [message], which is right for everything the core hands back and
         * wrong for the sentences this app writes itself: [Identity]'s three
         * name the exact key step that failed and none of them contains the
         * substring [Failure.of] looks for, so they used to be filed under
         * "Can't connect" — an unclassified failure — with the app's own
         * diagnosis sitting in the slot reserved for a runner's output.
         */
        data class Failed(
            val message: String,
            val kind: Failure = Failure.of(message),
        ) : Phase

        data object Connected : Phase

        /**
         * There WAS a connection, it went away, and one is being made again.
         *
         * A fourth phase rather than a flag on [Connected], because the two
         * existing candidates are each wrong in a way that shows on screen.
         * [Connecting] means "there has never been a fleet"; [Failed] means
         * "stopped, waiting for you", and this is not stopped.
         *
         * Rows keep their place through this. A runner that stops answering
         * keeps its rows rather than dropping them — the rule this app already
         * follows, and the reason it can afford a phase that means "stale, on
         * purpose, for the moment".
         *
         * **They are not dimmed, though this comment and `FleetScreen`'s have
         * said so since they were written.** Nothing in `app/src/main` ever
         * applied an alpha to a row belonging to a runner in this phase; what
         * says a runner is stale is [RunnerStatusRow], which names it and
         * offers "Reconnect now". Recorded rather than quietly corrected
         * because the claim is a good one and the front door is where it would
         * matter most — its sections span every runner, so a stale row there
         * sits beside a live one with nothing between them. Doing it means one
         * alpha, in one place, applied on both surfaces at once, and no
         * emulator was available to judge whether the result reads as stale or
         * as broken.
         */
        data class Reconnecting(val attempt: Int) : Phase
    }

    /**
     * What a failure MEANS, as opposed to what it says.
     *
     * A failure screen that offers the same button for every failure is one
     * that is wrong most of the time: "Try again" fixes a runner that was
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
        /** The runner answered but does not know this device's key. */
        KEY_REJECTED,

        /**
         * The key presented is not the one we pinned. Retrying is guaranteed to
         * fail, and offering it would suggest this is a glitch rather than a
         * decision someone has to make.
         */
        HOST_KEY_CHANGED,

        /** Nothing answered: wrong address, host asleep, off the network. */
        UNREACHABLE,

        /** SSH worked; Far Cooler is not installed over there. */
        DAEMON_MISSING,

        /** This device has no usable key, so no runner will ever accept it. */
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

    /**
     * What each worktree on this runner has changed, by workspace id, or empty
     * until the first read.
     *
     * Keyed rather than kept as the list the wire sends, because every reader
     * arrives with one workspace in hand and wants that workspace's row: a
     * fleet of twenty rows each scanning a twenty-entry array is quadratic work
     * to answer a question a map answers once.
     *
     * Keyed by WORKSPACE alone and not by `host/workspace`, unlike almost
     * everything else in this app — because this map belongs to one runner and
     * dies with it. [FleetRepository] is where the runners are merged, and it
     * carries the counts out on a [FleetEntry], which already names the host.
     *
     * Held from the last successful read when a read fails, on the same terms
     * as [fleet] itself. A count that vanishes reads as "this worktree has no
     * changes any more", which is a claim, and a call that failed is not
     * entitled to make one.
     */
    private val _inbox = MutableStateFlow<Map<String, InboxRow>>(emptyMap())
    val inbox: StateFlow<Map<String, InboxRow>> = _inbox.asStateFlow()

    private val _daemon = MutableStateFlow<DaemonBuild?>(null)
    val daemon: StateFlow<DaemonBuild?> = _daemon.asStateFlow()

    /**
     * Not private: a terminal screen talks to the same runner through this
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

    /** The runner details the last [start] used, which a retry reconnects to. */
    private var current: Runner = host

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
            // if it holds this is one round trip nobody notices. Forced,
            // because every count on screen is a claim about a fleet nobody
            // has asked since.
            is Phase.Connected -> scope.launch { refresh(force = true) }
            is Phase.Reconnecting, is Phase.Failed -> reconnectNow()
            // Already in flight, or waiting on a person. Neither is helped by
            // starting over.
            is Phase.Connecting, is Phase.NeedsApproval -> Unit
        }
    }

    suspend fun start(withHost: Runner = host) {
        poller?.cancel()
        reconnector?.cancel()
        current = withHost
        attempt += 1
        val mine = attempt
        _phase.value = Phase.Connecting

        val key = Identity.privateKey()
        if (key == null) {
            if (mine == attempt) {
                // Named rather than classified: [Identity] already said which
                // step failed, and this side knows without reading its words
                // that the answer is "there is no key".
                //
                // Its sentence only. The platform's words about a Keystore
                // write belong on the screen that manages this device's key —
                // see `AuthorizeScreen`, which shows them — and not in a runner row
                // whose subject is a runner that never got asked anything.
                _phase.value = Phase.Failed(
                    Identity.lastError?.sentence
                        ?: "This device has no SSH key and one could not be generated.",
                    Failure.NO_IDENTITY,
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
        // Forced, so a fresh link reads the diff counts on its first poll
        // rather than up to [INBOX_EVERY] polls later. See [loadInboxIfDue].
        refresh(force = true)
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
     * into Wi-Fi range, you woke the host, or you can simply see that this
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
                Identity.lastError?.sentence
                    ?: "This device has no SSH key and one could not be generated.",
                Failure.NO_IDENTITY,
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
        refresh(force = true)
        // Re-read rather than trust what a previous session reported: a
        // runner that dropped and came back may have gained a repository, and
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

        when (next.kind) {
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
     * runner that is asleep shows the same indefinite spinner as one that is
     * about to succeed.
     */
    fun giveUp() {
        abandon("Stopped waiting for ${host.address}. It may be asleep or off the network.")
    }

    /**
     * Back out of the fingerprint question without answering it.
     *
     * Lands on the failure screen rather than the spinner, because that is the
     * screen with the runner switcher, the editor and this device's key on it.
     * The wording is what [Failure.KEY_NOT_TRUSTED] matches on.
     */
    fun declineHostKey() {
        abandon(
            "The key ${host.address} presented has not been trusted on this device. " +
                "Far Cooler won’t connect until it is."
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
            // Absent from a daemon older than capabilities. `DaemonBuild.can`
            // reads an empty set as the features that existed then, so an old
            // runner keeps working rather than going dark.
            capabilities = body["capabilities"]?.jsonArray
                ?.mapNotNull { it.jsonPrimitive.contentOrNull }
                ?.toSet()
                .orEmpty(),
        )
        // Read from the same call, which is already made once per connection.
        //
        // Defaulted to the daemon's own default rather than to no prefix: an
        // older daemon that does not send the key still prefixes its branches
        // that way, so assuming nothing here would have this phone create
        // differently-named branches than the Mac beside it.
        _branchPrefix.value =
            body["branchPrefix"]?.jsonPrimitive?.contentOrNull ?: DEFAULT_BRANCH_PREFIX
    }

    /**
     * What this runner says a derived branch name starts with.
     *
     * Applied on this side rather than by the daemon, because the composer shows
     * you the branch it is about to create — a prefix added on the far side
     * would make that preview a lie.
     */
    private val _branchPrefix = MutableStateFlow(DEFAULT_BRANCH_PREFIX)
    val branchPrefix: StateFlow<String> = _branchPrefix.asStateFlow()

    /** What a fleet refresh produced, so the app can announce it exactly once. */
    var onFleet: ((Fleet) -> Unit)? = null

    /**
     * Ask the runner what its fleet is doing, and — on the polls that are due
     * for it — what its worktrees have changed.
     *
     * [force] reads the inbox whatever the divisor says. Passed by the three
     * moments where the last read is not merely old but untrusted: a link that
     * has just come up, a link that has just come back, and an app returning to
     * the foreground. Pull-to-refresh forces it too — see
     * [FleetRepository.refreshAll] — because a person pulling the list down is
     * asking for everything on it, not for most of it.
     */
    suspend fun refresh(force: Boolean = false) {
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

        // The diff counts, on the same poll as the rows they belong to.
        //
        // OUTSIDE the `try` above, deliberately. That block treats a throw as
        // possible evidence the link is gone, and this call must never be able
        // to supply that evidence: a daemon too old to know `changes.inbox`
        // refuses it on every poll forever, and would otherwise reconnect a
        // perfectly good session every three seconds. [loadInbox] cannot throw
        // at all, and a failed fleet poll returns above without reaching it, so
        // a fleet nobody could read is never followed by counts describing it.
        loadInboxIfDue(force)
    }

    /**
     * How many fleet polls have gone by, so the inbox can ride every [INBOX_EVERY]
     * of them. Not a wall clock: what this bounds is round trips, and the poll
     * loop is what makes them.
     */
    private var polls = 0L

    /**
     * Read the inbox on the polls that are due for it.
     *
     * **This is where the fleet's cost is bounded.** iOS calls `changes.inbox`
     * on every one of its three-second polls, and can afford to: it holds one
     * connection. This app holds one per runner, so the same rule would put N
     * extra SSH round trips on a three-second timer and make the feature the
     * product is proudest of the most expensive thing on the phone's radio.
     *
     * The divisor is not a compromise, it is what the data is worth. The fleet
     * needs three seconds because it carries agent state — blocked, done — and
     * that is the perishable half of the front door. The inbox carries a diff
     * watermark, which is the durable half: it was true before the app was
     * opened and stays true until somebody reads it. Something that sits still
     * does not need a three-second clock.
     *
     * At three, one runner costs 20 fleet polls and 6.7 inbox polls a minute
     * against iOS's 20 and 20; three runners cost 60 and 20 — twice iOS's total
     * call rate for three times the fleet. The counter is per connection, so
     * runners drift out of phase with each other rather than firing together.
     */
    private suspend fun loadInboxIfDue(force: Boolean) {
        val due = force || polls % INBOX_EVERY == 0L
        polls += 1
        if (due) loadInbox()
    }

    /**
     * Read what every worktree on this runner has changed, in one call.
     *
     * Errors swallowed, as [loadRepositories] and [loadThemes] beside it are,
     * and with more reason than either: these numbers decorate rows that are
     * already correct without them. A runner whose daemon predates
     * `changes.inbox` fails this on every poll forever, and neither that nor
     * one dropped packet may cost the fleet its screen.
     *
     * The last good map is kept when a read fails, rather than blanked. See
     * [inbox].
     */
    private suspend fun loadInbox() {
        val data = attempt { core.call("changes.inbox") }.getOrNull() ?: return
        val reply = runCatching {
            json.decodeFromJsonElement(InboxReply.serializer(), data)
        }.getOrNull() ?: return
        // `associateBy` keeps the LAST of any duplicate key, which is what a
        // runner listing one worktree twice should collapse to — the same
        // uniquing iOS spells out.
        _inbox.value = reply.items.associateBy { it.workspaceId }
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
     * Merge whatever this runner defines into the picker.
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
     * Hide or unhide a workspace.
     *
     * The Mac has had this since workspace management landed and neither phone
     * app ever got it, which on a runner that adopts every worktree it already
     * has means a sidebar of twenty rows and no way to put nineteen away.
     * Hiding never touches git and is never refused for a running terminal — it
     * is a view preference, not a lifecycle step.
     */
    /**
     * Put a device's key into this runner's `~/.ssh/authorized_keys`.
     *
     * **The daemon owns the write**, and that is the whole reason this is one
     * call: that file is the one whose corruption costs somebody SSH access to
     * their own machine, so the write is descriptor-anchored, `O_NOFOLLOW`,
     * locked, atomic and `fsync`ed twice in `crates/daemon/src/enrollment.rs`.
     * Nothing in this app appends a line to anything.
     *
     * `scope` is `control` per the design — `read` is a narrowing done afterwards
     * in Settings › Devices. There is deliberately no argument for the options or
     * the forced command: `ClientEnroll` has no field for either, and the absence
     * of a way to ask for an unrestricted line is the guard rail.
     *
     * False means the line is not there, and this says nothing about why: from
     * here a runner asleep, a daemon not installed, a damaged fence and a client
     * core with no arm for this method all look identical, and a caller that
     * guessed would be guessing.
     */
    suspend fun enroll(publicKey: String, label: String, clientId: String): Boolean {
        // Answered at all is answered yes. `already_enrolled` comes back in that
        // result and is not a failure: it is the ordinary outcome of granting a
        // runner the device can already reach, and the key is in the file either
        // way, which is what was asked.
        return attempt {
            core.call(
                "client.enroll",
                args(
                    "publicKey" to publicKey,
                    "label" to label,
                    "clientId" to clientId,
                    "scope" to "control",
                ),
            )
        }.isSuccess
    }

    // ---- runner settings ----
    //
    // Editing what the connected runner's config.toml holds. Every write
    // answers with the file's new state, read back by the daemon rather than
    // echoed from the request, so a value the writer normalized is what this
    // phone ends up holding.
    //
    // Built with `buildJsonObject` rather than the `args` helper below: these
    // carry arrays and a map, and that helper handles scalars only.

    /**
     * Only the themes this runner's file defines.
     *
     * Not the merged list [com.farcooler.data.Themes.available], which includes
     * this phone's built-ins — a built-in shown in an editor as though the file
     * defined it would offer a delete that does nothing.
     */
    suspend fun hostThemes(): List<Theme> {
        val data = attempt { core.call("themes") }.getOrNull() ?: return emptyList()
        return runCatching {
            json.decodeFromJsonElement(HostThemes.serializer(), data).themes
        }.getOrDefault(emptyList())
    }

    suspend fun setBranchPrefix(prefix: String): String? {
        val data = attempt {
            core.call("settings.set_branch_prefix", args("prefix" to prefix))
        }.getOrNull() ?: return null
        val stored = data["branchPrefix"]?.jsonPrimitive?.contentOrNull ?: prefix
        _branchPrefix.value = stored
        return stored
    }

    suspend fun upsertTheme(theme: Theme): List<Theme>? {
        val data = attempt { core.call("theme.upsert", theme.toJson()) }.getOrNull() ?: return null
        return themesFrom(data)
    }

    suspend fun deleteTheme(name: String): List<Theme>? {
        val data = attempt {
            core.call("theme.delete", args("name" to name))
        }.getOrNull() ?: return null
        return themesFrom(data)
    }

    /**
     * Put the runner's themes back into the picker every screen reads.
     *
     * Without this, a theme you just made is missing from the one place you
     * would go to choose it.
     */
    suspend fun reloadThemes() {
        com.farcooler.data.Themes.merge(hostThemes())
    }

    suspend fun adapters(): List<AdapterInfo> {
        val data = attempt { core.call("adapters") }.getOrNull() ?: return emptyList()
        return adaptersFrom(data)
    }

    suspend fun upsertAdapter(adapter: AdapterInfo): List<AdapterInfo>? {
        val data = attempt {
            core.call("adapter.upsert", adapter.toJson())
        }.getOrNull() ?: return null
        return adaptersFrom(data)
    }

    suspend fun deleteAdapter(preset: String): List<AdapterInfo>? {
        val data = attempt {
            core.call("adapter.delete", args("preset" to preset))
        }.getOrNull() ?: return null
        return adaptersFrom(data)
    }

    /** Prove an adapter works, without saving it first. */
    suspend fun testAdapter(adapter: AdapterInfo): AdapterTestOutcome {
        val data = attempt { core.call("adapter.test", adapter.toJson()) }.getOrNull()
            // Nothing to put under the sentence: a bridge call that throws
            // throws a state rather than a message. The Mac's own arm has the
            // CLI's line and passes it — see `RunnerSettingsStore.test`.
            ?: return AdapterTestOutcome.Failed(AdapterTestOutcome.Reason.NoAnswer(null))
        return if (data["ok"]?.jsonPrimitive?.booleanOrNull == true) {
            AdapterTestOutcome.Worked(
                data["reported"]?.jsonPrimitive?.contentOrNull ?: "answered")
        } else {
            // Empty rather than a sentence when the field is missing: an outcome
            // with nothing to show says so by having no transcript, and the
            // words for that case are `AdapterTestOutcome`'s to choose rather
            // than this call site's.
            AdapterTestOutcome.Failed(
                AdapterTestOutcome.Reason.Refused(
                    data["failure"]?.jsonPrimitive?.contentOrNull.orEmpty()))
        }
    }

    private fun themesFrom(data: JsonObject): List<Theme> = runCatching {
        json.decodeFromJsonElement(HostThemes.serializer(), data).themes
    }.getOrDefault(emptyList())

    private fun adaptersFrom(data: JsonObject): List<AdapterInfo> = runCatching {
        json.decodeFromJsonElement(AdapterList.serializer(), data).adapters
    }.getOrDefault(emptyList())

    @kotlinx.serialization.Serializable
    private data class AdapterList(val adapters: List<AdapterInfo> = emptyList())

    suspend fun setHidden(workspace: Workspace, hidden: Boolean) {
        val method = if (hidden) "workspace.hide" else "workspace.unhide"
        attempt { core.call(method, args("workspace" to workspace.id)) }
        refresh()
    }

    /**
     * Create a worktree and branch, with a terminal already in it.
     *
     * [name] names the worktree's directory. The wire key is still `task`, which
     * is what it was called when a workspace carried a typed-out task alongside
     * its directory; renaming the key would strand every shipped app for nothing.
     *
     * `terminal` names what runs there; empty means none, which is what a caller
     * about to create its own agent terminal wants. A shell here, because a
     * worktree with nothing running in it is a directory.
     */
    suspend fun createWorkspace(
        repository: String,
        name: String,
        branch: String,
        terminal: String = "shell",
    ): String {
        val data = core.call(
            "workspace.create",
            args(
                "repository" to repository,
                "task" to name,
                "branch" to branch,
                "base" to "",
                "terminal" to terminal,
            ),
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
        /**
         * What a derived branch name starts with when the runner says nothing.
         *
         * Matches `farcooler_core::config::DEFAULT_BRANCH_PREFIX`. Stated here
         * as well because an older daemon does not send the key, and defaulting
         * to no prefix would have this phone name branches differently from
         * every other client talking to the same runner.
         */
        const val DEFAULT_BRANCH_PREFIX = "feat/"

        private const val POLL_INTERVAL_MS = 3_000L

        /**
         * One inbox read per this many fleet polls. See [loadInboxIfDue] for
         * why the two payloads do not deserve the same cadence.
         */
        private const val INBOX_EVERY = 3L

        /**
         * How long a runner that answered SSH but not Far Cooler waits.
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
         * runners recovering from one network event must not retry in
         * lockstep — and this app connects to every runner at once, so that is
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
