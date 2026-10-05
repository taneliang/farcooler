package com.farcooler.net

import com.farcooler.model.BoardPage
import com.farcooler.model.PlanReadState
import com.farcooler.model.WorkspaceSummary
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

/** What a phone holds of a board's pages: the list, or a read that failed with nothing in hand. */
sealed interface PageListState {
    data class Loaded(val pages: List<BoardPage>) : PageListState

    /** Refused, failed or never answered: [com.farcooler.model.PageWords.COULDNT_READ], with Try again. */
    data object Unavailable : PageListState
}

/**
 * Orchestrator pages, as one connection reads them (ov-285): each board's
 * pages with their documents, in one `page.list`, which the client core answers
 * in the shape `crates/client/src/page_json.rs` writes. The iPhone's
 * `ConnectionPages.swift`, on Android.
 *
 * EXPERIMENTAL, behind `board_pages`, and only ever in the Plan view: nothing
 * is asked of a runner without it, or before someone picks Plan on a board.
 */
class PageReads(
    private val call: suspend (method: String, args: JsonObject) -> JsonObject,
    /** Whether the runner advertises `board_pages`; null before its build is read. */
    private val runnerCan: () -> Boolean?,
    private val timeoutMs: Long = PlanReadState.TIMEOUT_MS,
) {
    private val _lists = MutableStateFlow<Map<String, PageListState>>(emptyMap())

    /** Each board's pages, by workspace id. */
    val lists: StateFlow<Map<String, PageListState>> = _lists.asStateFlow()

    /** Whether this runner keeps pages: only then is there a Pages section. */
    val keeps: Boolean get() = runnerCan() == true

    /**
     * Read [workspace]'s board's pages. A read that fails over a list in hand
     * keeps the list, as the plan's does; one with nothing in hand says so.
     */
    suspend fun read(workspace: WorkspaceSummary) {
        val board = workspace.boardWorkspace
        if (!keeps || board == null) {
            if (_lists.value.containsKey(workspace.id)) _lists.value = _lists.value - workspace.id
            return
        }
        val answer = try {
            withTimeoutOrNull(timeoutMs) { call("page.list", buildJsonObject { put("workspace", board) }) }
        } catch (e: kotlinx.coroutines.CancellationException) {
            throw e
        } catch (e: Exception) {
            null
        }
        val pages = answer?.let { runCatching { BoardPage.list(it) }.getOrNull() }
        val next = when {
            pages != null -> PageListState.Loaded(pages)
            _lists.value[workspace.id] is PageListState.Loaded -> return
            else -> PageListState.Unavailable
        }
        if (_lists.value[workspace.id] != next) _lists.value = _lists.value + (workspace.id to next)
    }

    /** A board's pages, or none before a read or after one that failed. */
    fun pages(workspaceId: String): List<BoardPage> = (_lists.value[workspaceId] as? PageListState.Loaded)?.pages.orEmpty()

    /**
     * A runner notice: a `pages` line names the board a page was written or
     * removed on. Boards whose pages were read read them again, while the app
     * is in front; the rest wait until someone picks Plan.
     */
    suspend fun noticed(notice: JsonObject, boards: List<WorkspaceSummary>, foreground: Boolean) {
        if (notice["event"]?.jsonPrimitive?.contentOrNull != "pages" || !foreground) return
        val named = notice["workspace"]?.jsonPrimitive?.contentOrNull ?: return
        for (workspace in boards) {
            val matches = workspace.id.equals(named, ignoreCase = true) || workspace.boardWorkspace.equals(named, ignoreCase = true)
            if (matches && _lists.value.containsKey(workspace.id)) read(workspace)
        }
    }
}
