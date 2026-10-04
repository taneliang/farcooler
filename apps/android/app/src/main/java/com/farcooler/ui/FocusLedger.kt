package com.farcooler.ui

import com.farcooler.data.ReviewStorage
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * Which pane each worktree shows, keyed `runner/worktree`, and where the
 * chosen ones are written down: the process-death copy ([read], [write]) and
 * the one a relaunch reads ([kept]) (ov-233).
 *
 * Out of `AppModel` so a JVM test reaches the save, the load and the prune: the
 * model needs an `Application`, and a restore that nothing can fail is the
 * restore that quietly stops working. See [Focus] for why only a choice is
 * written, and [Backstack.restoreFocus] for the two copies' order.
 */
class FocusLedger(
    private val read: () -> String?,
    private val write: (String) -> Unit,
    private val kept: ReviewStorage,
) {
    private val _focus = MutableStateFlow<Map<String, Focus>>(emptyMap())
    val focus: StateFlow<Map<String, Focus>> = _focus.asStateFlow()

    operator fun get(key: String): Focus? = _focus.value[key]

    /** Read what was kept, at launch, before anything is composed. */
    fun restore() {
        _focus.value = Backstack.restoreFocus(read(), kept)
    }

    /** Where somebody is, or [chosen] to be. Only a choice changes what is on disk. */
    fun record(key: String, pane: Pane, chosen: Boolean) {
        val existing = _focus.value[key]
        if (existing?.pane == pane && existing.chosen == chosen) return
        _focus.value = _focus.value + (key to Focus(pane, chosen))
        if (chosen) persist()
    }

    /**
     * Forget the tabs of what a runner no longer has.
     *
     * Only for a runner whose fleet has been READ on this link ([fleetRead]),
     * not merely one that is connected: a connected runner's rows are empty
     * until its first read, and pruning on them forgot every agent tab, which
     * is now written to disk. A worktree gone from a read fleet loses its entry
     * whatever the pane, Changes included; a worktree that is there loses an
     * agent tab whose terminal isn't.
     */
    fun prune(
        fleetRead: (host: String) -> Boolean,
        hasWorktree: (host: String, worktree: String) -> Boolean,
        hasTerminal: (host: String, worktree: String, terminalId: String) -> Boolean,
    ) {
        val pruned = Backstack.prune(
            _focus.value.filter { (key, _) ->
                val host = key.substringBefore('/')
                !fleetRead(host) || hasWorktree(host, key.substringAfter('/'))
            },
        ) { key, terminalId ->
            val host = key.substringBefore('/')
            !fleetRead(host) || hasTerminal(host, key.substringAfter('/'), terminalId)
        }
        if (pruned == _focus.value) return
        _focus.value = pruned
        persist()
    }

    private fun persist() {
        write(Backstack.encodeFocus(_focus.value))
        Backstack.keepFocus(_focus.value, kept)
    }
}
