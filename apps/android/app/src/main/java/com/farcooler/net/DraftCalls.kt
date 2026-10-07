package com.farcooler.net

import com.farcooler.model.AskAboutTask
import com.farcooler.model.DraftHold
import com.farcooler.net.Connection.Companion.args

// Ask the Orchestrator's paste and the hold behind a dialog (ov-241, ov-385),
// out of `Connection.kt` for its size ceiling.

/**
 * Ask the runner to paste [text] into a terminal orchestrator's box, pressing
 * no Enter (Ask the orchestrator, ov-241). DECLINED means nothing was typed;
 * UNKNOWN that no answer came in time, so it may have been; HELD that a dialog
 * was up and the runner pastes it once the dialog closes (ov-385), which the
 * pane's [com.farcooler.ui.HeldDraftBar] then says.
 */
suspend fun Connection.draftPrompt(terminal: String, text: String): AskAboutTask.DraftResult =
    attempt { core.call("terminal.draft_prompt", args("terminal" to terminal, "text" to text)) }
        .fold(
            onSuccess = {
                if (DraftHold.held(it) != null) {
                    // Read the hold now, so the pane says it waits.
                    refresh()
                    AskAboutTask.DraftResult.HELD
                } else {
                    AskAboutTask.DraftResult.PASTED
                }
            },
            onFailure = { AskAboutTask.DraftResult.of(it) },
        )

/**
 * Stop the runner pasting the draft [hold] it holds on [terminal] (ov-385).
 * False when it didn't reach the runner: the pane still says it waits, and
 * Withdraw is there to press again.
 */
suspend fun Connection.withdrawDraft(terminal: String, hold: String): Boolean {
    val reached = attempt { core.call("terminal.draft_withdraw", args("terminal" to terminal, "hold" to hold)) }.isSuccess
    refresh()
    return reached
}
