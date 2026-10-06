package com.farcooler.ui

import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * The call sites the plan layer hangs on (ov-274), which no JVM test can drive
 * because the code around them needs the native core or a screen: each is read
 * out of its source, so removing the call turns this red. Their rules are tested
 * where they live (`PlanTest`, `PlanReadsTest`, `PlanListTest`).
 */
class PlanWiringTest {
    private fun source(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, "apps/android/app/src/main/java/com/farcooler/$relative")
            if (candidate.isFile) return candidate.readText()
            val here = File(directory, "app/src/main/java/com/farcooler/$relative")
            if (here.isFile) return here.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}.")
    }

    @Test
    fun `a runner notice reaches the plan layer, and a task or resync notice reads the plans again`() {
        val connection = source("net/Connection.kt")
        assertTrue("plan notices", "plans.noticed(notice, boardList(), isForeground)" in connection)
        assertTrue("task notices", "plans.reread(RunnerBoards.touched(moved, boardList()))" in connection)
        assertTrue("resync", "plans.reread(boardList())" in connection)
    }

    @Test
    fun `the board hands its Plan choice to the list, and the control is gated on the capability`() {
        val board = source("ui/BoardScreen.kt")
        assertTrue("the list stands down for Plan", "showsPlan = showsPlan)" in board)
        assertTrue("the control needs board_plan", "if (keepsPlan) {\n                        item(key = \"plan/switch\")" in board)
        assertTrue("the choice needs the capability", "PlanChoice.showing(keepsPlan, planChosen)" in board)
        assertTrue("the plan draws in place of the tasks", "if (showsPlan) {\n                        planItems(" in board)
    }

    @Test
    fun `rulings are gated on board_rulings, copied to the clipboard, and drawn after the plan`() {
        val board = source("ui/BoardScreen.kt")
        assertTrue("the capability", "val keepsRulings = daemon?.can(Capability.BOARD_RULINGS) == true" in board)
        // The hook is built once, for the Board and the Plan sheet alike (ov-300 review 5).
        assertTrue("no hook without it", "return if (keepsRulings) RulingsHook(" in board)
        assertTrue("the board's plan takes it", "rulings = rulingsHook," in board)
        assertTrue("so does the sheet", "rulings = rulings," in source("ui/PhoneTree.kt"))
        assertTrue("copy goes to the clipboard", "copy = { scope.launch { clipboard.writeText(\"Far Cooler\", it) } }," in board)
        // The owner's marks (ov-333): gated on their own capability, Keep to the runner, Reverse sent.
        assertTrue("its own capability", "val keepsRulingActions = daemon?.can(Capability.BOARD_RULING_ACTIONS) == true" in board)
        assertTrue("keep is the runner's mark", "connection.keepRuling(ruling, workspace)" in board)
        assertTrue("keep all is one call", "connection.keepAllRulings(workspace)" in board)
        assertTrue("reverse sends a message", "connection.agentPrompt(seat.terminal.id, text)" in board)
        assertTrue("discuss leaves a draft", "connection.composerHandoff.offer(seat.terminal.id, it)" in board)
        val screens = source("ui/PlanScreens.kt")
        assertTrue("drawn after the plan", "rulingItems(state.plan, rulings)" in screens)
        val rows = source("ui/PlanRulingRows.kt")
        assertTrue("nothing without the hook", "if (hook == null || plan.rulings.isEmpty()) return" in rows)
    }

    @Test
    fun `the owner's marks need the write grant, and Reverse asks before it sends`() {
        val board = source("ui/BoardScreen.kt")
        assertTrue("a Read grant offers no marks", "canMark = keepsRulingActions && daemon?.grantedScope != \"read\"," in board)
        val rows = source("ui/PlanRulingRows.kt")
        assertTrue("the menu asks first", "onClick = { open = false; onReverse() }," in rows)
        assertTrue("the dialog is what reverses", "TextButton(onClick = { confirming = false; hook.reverse(ruling) }" in rows)
        assertTrue("TalkBack asks first too", "CustomAccessibilityAction(RulingWords.REVERSE) { onReverse(); true }" in rows)
        assertTrue("nothing else calls reverse", rows.split("hook.reverse(ruling)").size == 2)
    }

    @Test
    fun `a plan page says which page is open, so a notice reads only that record`() {
        val page = source("ui/PlanPageScreens.kt")
        assertTrue("open", "connection.plans.openPage = page" in page)
        assertTrue("closed", "connection.plans.openPage = null" in page)
    }
}
