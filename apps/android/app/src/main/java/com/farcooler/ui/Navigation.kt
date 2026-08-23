package com.farcooler.ui

import com.farcooler.model.Terminal
import com.farcooler.model.landingTerminal
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.builtins.MapSerializer
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.json.Json

/**
 * Where the app is.
 *
 * A serializable closed hierarchy, because the whole stack is written into
 * `SavedStateHandle` on every navigation and read back after the process is
 * killed — see [Backstack] for the encoding and [Backstack.truncate] for what
 * happens to a route that no longer names anything.
 *
 * **Every route that names something on a runner carries the runner.** Ids are
 * minted per daemon, so a workspace id or a terminal id is meaningless without
 * the host beside it; `net/FleetRepository.kt` makes the same argument about
 * [com.farcooler.net.TerminalRef] and it is the reason this app can be
 * connected to every runner at once while iOS makes you pick one.
 *
 * The `@SerialName` on each case is deliberate and is not decoration: without
 * it the discriminator is the Kotlin class name, so moving [Route] to another
 * package or renaming a case would silently invalidate every stack anybody has
 * saved. These strings are the wire format for a phone's own state.
 */
@Serializable
sealed interface Route {
    /** No runners yet, so there is nothing else to show. */
    @Serializable
    @SerialName("onboarding")
    data object Onboarding : Route

    /**
     * The front door: what needs a person, across every connected runner.
     *
     * The root, and the app's answer to three of the four situations
     * `docs/jobs-to-be-done.md` names. The app used to open into a TERMINAL —
     * `FleetRepository.landing()` picked one on connect and the workspace list
     * was the fallback for a fleet with nothing running. That is the right
     * front door for exactly one of those situations, on the couch about to
     * drive an agent; in the other three — in transit, standing with the phone
     * in hand, at the gym between sets — the first question is *what needs me*,
     * and a terminal is an answer to a question nobody asked. iOS deleted the
     * same shape in `1be6264`.
     *
     * No host and no workspace on it, unlike every other route that names
     * something: this one names the whole fleet, which is the property the
     * screen exists for.
     */
    @Serializable
    @SerialName("needs-you")
    data object NeedsYou : Route

    /**
     * The workspace list — every worktree on every runner, hidden ones
     * included.
     *
     * No longer the root. It is pushed from the front door's Workspaces row,
     * and it is also what the navigation drawer holds, which is deliberate
     * duplication rather than an oversight: the drawer is reachable by an edge
     * swipe with no target to hit, and the row is reachable by reading. See
     * `RootScreen`.
     */
    @Serializable
    @SerialName("fleet")
    data object Fleet : Route

    /**
     * One workspace, on one runner — and deliberately NOT which pane of it.
     *
     * The pane lives in [Focus], beside the stack, and that separation is the
     * whole point. iOS learned it in `09b1e1f`: writing the focused tab into a
     * path element changes that element's VALUE, and SwiftUI is free to rebuild
     * a destination whose value changed, discarding every mounted pane with its
     * scroll position, its half-typed message and its open stream. Compose does
     * not rebuild on a value change by itself, but it is the route that
     * everything downstream keys on — `key(...)`, `remember(...)`,
     * `rememberSaveable(...)`, a `SaveableStateHolder` — so a route that
     * changes value on every tab tap is a subtree that resets on every tab tap
     * the moment anyone writes one of those. `RootScreen` keys the pane on
     * `hostId` and `workspaceId` for exactly that reason.
     */
    @Serializable
    @SerialName("terminal")
    data class Terminal(val hostId: String, val workspaceId: String) : Route

    @Serializable
    @SerialName("settings")
    data object Settings : Route

    @Serializable
    @SerialName("authorize")
    data object Authorize : Route

    /**
     * This device showing a code, to be added by one that is already trusted.
     *
     * The short road out of [Authorize], which stays: pasting a key is what
     * works with no trusted device to scan with.
     */
    @Serializable
    @SerialName("join")
    data object Join : Route

    /** The other side of the ceremony: granting runners to a device being added. */
    @Serializable
    @SerialName("add-device")
    data object AddDevice : Route

    /** Everything this account has registered, and how to revoke it. */
    @Serializable
    @SerialName("devices")
    data object Devices : Route

    /**
     * One runner's own config.toml.
     *
     * Carries the host id rather than the connection, so a route survives the
     * connection being replaced underneath it — a reconnect builds a new
     * `Connection` and a route holding the old one would edit a dead session.
     * That was true before this route had to serialize, and serializing is a
     * second reason for the same shape: a live object has no encoding.
     */
    @Serializable
    @SerialName("runner-settings")
    data class RunnerSettings(val hostId: String) : Route

    /**
     * Whether this route is drawn OVER the workspace rather than instead of it.
     *
     * Every pushed screen is. The workspace underneath stays composed, which is
     * what lets a predictive back gesture preview something real, and — the
     * reason that matters more — what lets the terminal session, the transcript
     * scroll and the composer draft survive a trip into settings. Before this,
     * opening settings left `TerminalScreen` composition and disposed the SSH
     * stream on the way out.
     */
    val isOverlay: Boolean
        get() = when (this) {
            is Settings, is RunnerSettings, is Authorize, is Join, is AddDevice, is Devices -> true
            // The three GROUND routes. A terminal is one of them and not an
            // overlay, even though it is now pushed onto the front door rather
            // than replacing it: `isOverlay` also decides whether the drawer's
            // edge swipe is live, and a terminal that gave that edge to the
            // back gesture would take the fleet drawer away from the one screen
            // it is most used from. So back out of a terminal is the plain
            // handler in `RootScreen`, and predictive back stays where it
            // already was.
            is Onboarding, is NeedsYou, is Fleet, is Terminal -> false
        }
}

/**
 * One tab of a workspace.
 *
 * Two kinds, and the second one has nothing behind it on the runner. Every
 * `changes.*` RPC takes a workspace id and nothing else — `Session::change_set`
 * and `Session::file_diff` in `crates/client/src/session.rs` pass `None` where
 * a terminal-scoped call passes an id — so the diff is a fact about the
 * worktree that this app can ask for whether or not anybody ever opened a
 * `changes` pane in it. That is what lets Changes be a tab at no cost on the
 * daemon side.
 *
 * A `changes` pane the host DOES have folds into [Changes] rather than
 * becoming a tab of its own, and that fold is where the defect the parity
 * inventory found gets fixed: `TerminalScreen` routed only on
 * [com.farcooler.model.Terminal.isAgentPane], so a `changes` pane created from
 * the Mac fell through to the VT renderer and was drawn as a grid of whatever
 * bytes are on a pane that is not a tty. It cannot reach that renderer any
 * more — [Backstack.chooseFocus] and [Backstack.rule] both fold it, so every
 * road into it now arrives at the Changes tab.
 *
 * [id] is NAMESPACED, because a terminal id and the word "changes" are
 * different kinds of thing and a collision between them would silently give two
 * tabs one Compose identity — which Compose resolves by drawing one of them.
 * It is also what a `SaveableStateHolder` buckets a tab's saved state under, so
 * a collision there would hand one pane's half-typed message to another.
 */
sealed interface Pane {
    /** Stable, namespaced, and the only thing anything downstream keys on. */
    val id: String

    /** One pane on the runner, by id. */
    data class Terminal(val terminalId: String) : Pane {
        override val id: String get() = "$TERMINAL_PREFIX$terminalId"
    }

    /** The worktree's own diff. Needs nothing on the runner to exist. */
    data object Changes : Pane {
        override val id: String get() = CHANGES_ID
    }

    companion object {
        const val CHANGES_ID = "changes"
        const val TERMINAL_PREFIX = "terminal:"

        /** The tab a pane on the runner belongs on. See the type's own note on folding. */
        fun of(terminal: com.farcooler.model.Terminal): Pane =
            if (terminal.isChangesPane) Changes else Terminal(terminal.id)

        /**
         * Read back an [id].
         *
         * The third arm is a MIGRATION and not a fallback. Phases 2 and 3 wrote
         * the focus map as bare terminal ids, and those strings are sitting in
         * the saved state of every phone that has run one of those builds; read
         * strictly they would each cost somebody the tab they last chose. A
         * value that carries neither namespace is exactly what those builds
         * wrote, so it is read as what it was.
         */
        fun parse(id: String): Pane = when {
            id == CHANGES_ID -> Changes
            id.startsWith(TERMINAL_PREFIX) -> Terminal(id.removePrefix(TERMINAL_PREFIX))
            else -> Terminal(id)
        }
    }
}

/**
 * Which tab a workspace is showing, and whether a person chose it.
 *
 * The second half is not bookkeeping. Only a chosen focus is written down —
 * see [Backstack.encodeFocus] — because a notification tap and a fleet-list row
 * are places somebody was SENT, and a 3am ping about an agent that got itself
 * blocked must not decide where the workspace opens tomorrow morning. iOS draws
 * the same line in `09b1e1f`, one writer and three deliberate non-writers.
 *
 * A [Pane] rather than a terminal id since the workspace screen gained a
 * Changes tab: "where I was in this worktree" has an answer that is not a pane
 * on the runner, and a map that could only hold terminal ids could not say it.
 */
data class Focus(val pane: Pane, val chosen: Boolean)

/**
 * The navigation stack, and everything about it that is pure.
 *
 * Separated from [AppModel] so it can be tested without a device: the encoding
 * and the resolve-or-truncate rule are exactly the parts of state restoration
 * that a JVM unit test can pin, and the parts a phone cannot be asked about
 * from here.
 */
object Backstack {
    /**
     * Tolerant on the way in, for the reason `net/Connection.kt` is tolerant
     * about a fleet: a saved stack is read by a build that may be newer than
     * the one that wrote it, and one unknown key must not cost somebody their
     * place. An unknown route TYPE is not survivable that way and is handled
     * by [decodeStack] instead.
     */
    private val json = Json { ignoreUnknownKeys = true }

    /**
     * The root every degraded stack falls back to.
     *
     * The front door, since phase 3. It was [Route.Fleet], which was correct
     * while the app landed on a terminal and used the workspace list as its
     * fallback; now the fallback and the front door are the same screen, and it
     * is the one screen in the app that needs nothing from any runner to be
     * worth showing.
     */
    val ROOT: Route = Route.NeedsYou

    /** One runner's one workspace, which is the grain a focus is remembered at. */
    fun key(hostId: String, workspaceId: String) = "$hostId/$workspaceId"

    fun encodeStack(stack: List<Route>): String =
        json.encodeToString(stackFormat, stack)

    /**
     * A saved stack, or null if there is nothing usable to restore.
     *
     * Null rather than an exception, and null rather than a partial stack: a
     * stack written by a build that has since renamed a route decodes to
     * nothing at all, and starting from the root is the honest answer. The
     * alternative — recovering the routes before the unknown one — would be
     * guessing at what somebody meant from a format this app no longer speaks.
     */
    fun decodeStack(saved: String?): List<Route>? {
        if (saved.isNullOrBlank()) return null
        val decoded = runCatching { json.decodeFromString(stackFormat, saved) }
            .getOrNull() ?: return null
        return decoded.ifEmpty { null }
    }

    /**
     * The chosen tabs, as `runner/workspace` to terminal id.
     *
     * Only the chosen ones. An entry nobody chose is where the app put someone,
     * and writing that down would make the memory a record of the app's own
     * guesses — which is the one thing it must not be, because it is read back
     * in preference to the rule that would make the guess again, better, with
     * today's fleet in front of it.
     */
    fun encodeFocus(focus: Map<String, Focus>): String {
        val chosen = focus.filterValues { it.chosen }.mapValues { it.value.pane.id }
        return json.encodeToString(focusFormat, chosen)
    }

    fun decodeFocus(saved: String?): Map<String, Focus> {
        if (saved.isNullOrBlank()) return emptyMap()
        val decoded = runCatching { json.decodeFromString(focusFormat, saved) }
            .getOrNull() ?: return emptyMap()
        return decoded.mapValues { Focus(Pane.parse(it.value), chosen = true) }
    }

    /**
     * Cut the stack at the first route that no longer names anything.
     *
     * The Compose answer to what iOS does with `path.removeSubrange(gone...)`.
     * Truncating rather than filtering, because a stack is a story: if the
     * workspace you were reading was merged away, the settings screen you had
     * pushed on top of it is not where you meant to end up either. Everything
     * from the first dead route onwards goes.
     *
     * And a stack with nothing left degrades to [ROOT], never to empty. There
     * is no such thing as being nowhere — an empty stack is a blank screen with
     * a back gesture that does not work, which is the broken screen this rule
     * exists to avoid.
     *
     * Called on every fleet change, not only at launch, so it covers both the
     * restored stack and the workspace that disappears under someone while they
     * are looking at it. One rule, two moments.
     */
    fun truncate(stack: List<Route>, resolves: (Route) -> Boolean): List<Route> {
        // Before the search, not after it: a stack that arrives empty has no
        // first dead route to find, so the guard below would never run and the
        // one thing this rule promises — never empty — would be broken by the
        // one input that needs it most.
        if (stack.isEmpty()) return listOf(ROOT)
        val gone = stack.indexOfFirst { !resolves(it) }
        if (gone < 0) return stack
        return stack.take(gone).ifEmpty { listOf(ROOT) }
    }

    /**
     * Drop remembered tabs for anything that is gone.
     *
     * Not what makes the app correct — [chooseFocus] degrades a dead entry to
     * the rule anyway. What it stops is the stored value growing a line per
     * worktree anybody ever merged away, forever, in a string that is written
     * to the activity's saved state on every navigation.
     */
    fun prune(focus: Map<String, Focus>, lives: (key: String, terminalId: String) -> Boolean) =
        focus.filter { (key, value) ->
            // The Changes tab is never pruned, and cannot be: nothing on the
            // runner has to exist for it, so nothing can stop existing. iOS
            // makes the same exception in `WorkspaceView.prune`.
            when (val pane = value.pane) {
                is Pane.Changes -> true
                is Pane.Terminal -> lives(key, pane.terminalId)
            }
        }

    /**
     * Which pane a workspace shows, in an order of authority.
     *
     * 1. **The focus**, if it still names a live pane. Somebody either chose it
     *    or was sent to it; either way it is the most recent thing anyone
     *    asked for.
     * 2. **The rule**, below — whatever needs you now.
     *
     * The first degrades into the second rather than to nothing, so a
     * remembered agent that died overnight falls through to whatever is
     * blocked this morning instead of resolving to a blank pane.
     */
    fun chooseFocus(terminals: List<Terminal>, focus: Focus?): Pane? =
        resolve(focus?.pane, terminals) ?: rule(terminals)

    /**
     * The tab a remembered or requested pane actually resolves to, or null when
     * it names nothing this workspace has.
     *
     * Split out of [chooseFocus] because the workspace screen needs the first
     * half WITHOUT the second. That screen watches the focus map so a
     * notification tap or a fleet row moves the tab you are looking at — but it
     * must not follow the RULE, which changes on every poll: an agent finishing
     * somewhere else in the worktree would otherwise yank a screen somebody is
     * reading. The rule gets exactly one turn, when the screen opens. iOS
     * writes the same restriction on `WorkspaceView.select`.
     *
     * Null while a runner has not answered, which is the honest result rather
     * than a gap: the pane may well exist, and a tap that arrives before the
     * fleet does gets honored on the poll that brings it.
     */
    fun resolve(pane: Pane?, terminals: List<Terminal>): Pane? = when (pane) {
        null -> null
        // Answerable with no fleet at all, which is the property the tab has:
        // the diff is asked for by workspace id. So a remembered Changes tab
        // comes back during a handshake, where a remembered terminal cannot.
        is Pane.Changes -> Pane.Changes
        // Folded, not returned as itself: the pane named may have been switched
        // to `changes` mode from the Mac since anyone last looked. See [Pane].
        is Pane.Terminal -> terminals.firstOrNull { it.id == pane.terminalId }?.let { Pane.of(it) }
    }

    /**
     * Which pane a workspace opens on when nobody said and nobody ever chose.
     *
     * Blocked agent, then whatever is running, then the first pane there is —
     * and the ordering is `model/Model.kt`'s, not a third copy of it. This is
     * the same question the fleet-wide landing rule asks, narrowed to one
     * workspace, and the two must not be able to disagree about which pane
     * matters.
     */
    fun rule(terminals: List<Terminal>): Pane? {
        // A `changes` pane is never landed ON as a terminal — it is the Changes
        // tab — so it is out of the running before the ordering is applied, and
        // is the floor underneath it when it is the only thing there is.
        val panes = terminals.filterNot { it.isChangesPane }
        panes.landingTerminal?.let { return Pane.Terminal(it.id) }
        if (terminals.any { it.isChangesPane }) return Pane.Changes
        return null
    }

    private val stackFormat = ListSerializer(Route.serializer())

    private val focusFormat = MapSerializer(String.serializer(), String.serializer())
}
