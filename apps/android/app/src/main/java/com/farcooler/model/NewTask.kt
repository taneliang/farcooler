package com.farcooler.model

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/**
 * New Task… on a board: filing a task from the phone, as everything except
 * the drawing of it.
 *
 * The phone's `farcooler task create`, through the client core's
 * `task.create` (`crates/client/src/ffi.rs`), which files it as `user`. The
 * rules are the Mac's `TaskBoardWrites` (`apps/macos/.../TaskBoard.swift`):
 * offered below nothing but Read scope, and a title held to the runner's own
 * `checked_title`. The sheet adds Details, which is the task's intent.
 *
 * Pure, so a JVM can prove it; `NewTaskTest` does.
 */
object NewTask {
    /** The runner's cap on a title, in Unicode scalars. */
    const val TITLE_LIMIT = 200

    /**
     * Whether a board on [build]'s runner offers New Task…. Filing a task is
     * a Control-scope write, so a connection granted only Read gets no
     * button. No answer yet, or `unspecified`, is not a refusal: see
     * [DaemonBuild.grantedScope].
     */
    fun offered(build: DaemonBuild?): Boolean = build?.grantedScope != "read"

    /**
     * Whether the runner will take [title]: not empty once trimmed, and at
     * most [TITLE_LIMIT] code points. Code points and not chars, as the
     * runner counts: an emoji is two chars and one scalar.
     */
    fun titleFits(title: String): Boolean {
        val trimmed = title.trim()
        return trimmed.isNotEmpty() && trimmed.codePointCount(0, trimmed.length) <= TITLE_LIMIT
    }

    /**
     * `task.create`'s arguments: [workspace]'s board, the title trimmed, and
     * [details] as the intent when there are any. A runner without
     * workspaces names none, and the task goes on its repository's board.
     */
    fun request(workspace: WorkspaceSummary, title: String, details: String): JsonObject = buildJsonObject {
        put("repository", workspace.repository ?: workspace.id)
        workspace.boardWorkspace?.let { put("workspace", it) }
        put("title", title.trim())
        details.trim().takeIf { it.isNotEmpty() }?.let { put("intent", it) }
    }

    /**
     * The one line a failed create leaves on the sheet, from the runner's
     * [word] and [what]. Never the runner's own words. Null [word] is a link
     * that dropped rather than a runner that said no.
     */
    fun refusal(word: String?, what: String?): String = when {
        what == "title" -> "A title can be at most $TITLE_LIMIT characters."
        word == "scope-denied" -> "This device can only look at this runner, so it can’t add tasks."
        word == "capability-unsupported" ->
            "This runner’s Far Cooler is too old to add tasks from a phone. Update it there, then try again."
        word == "not-found" -> "This board isn’t on the runner anymore."
        word != null ->
            "The runner couldn’t add that task. That’s a problem in the app, not in anything you typed."
        else -> "Couldn’t add that task. Check that the runner is reachable, then try again."
    }
}
