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

    /** The workspace list — what a runner with nothing running shows. */
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
            is Onboarding, is Fleet, is Terminal -> false
        }
}

/**
 * Which pane a workspace is showing, and whether a person chose it.
 *
 * The second half is not bookkeeping. Only a chosen focus is written down —
 * see [Backstack.encodeFocus] — because a notification tap and a fleet-list row
 * are places somebody was SENT, and a 3am ping about an agent that got itself
 * blocked must not decide where the workspace opens tomorrow morning. iOS draws
 * the same line in `09b1e1f`, one writer and three deliberate non-writers.
 */
data class Focus(val terminalId: String, val chosen: Boolean)

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

    /** The root every degraded stack falls back to. */
    val ROOT: Route = Route.Fleet

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
        val chosen = focus.filterValues { it.chosen }.mapValues { it.value.terminalId }
        return json.encodeToString(focusFormat, chosen)
    }

    fun decodeFocus(saved: String?): Map<String, Focus> {
        if (saved.isNullOrBlank()) return emptyMap()
        val decoded = runCatching { json.decodeFromString(focusFormat, saved) }
            .getOrNull() ?: return emptyMap()
        return decoded.mapValues { Focus(it.value, chosen = true) }
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
        focus.filter { (key, value) -> lives(key, value.terminalId) }

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
    fun chooseFocus(terminals: List<Terminal>, focus: Focus?): String? {
        val wanted = focus?.terminalId
        if (wanted != null && terminals.any { it.id == wanted }) return wanted
        return rule(terminals)
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
    fun rule(terminals: List<Terminal>): String? = terminals.landingTerminal?.id

    private val stackFormat = ListSerializer(Route.serializer())

    private val focusFormat = MapSerializer(String.serializer(), String.serializer())
}
