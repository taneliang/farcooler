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
    fun `a plan page says which page is open, so a notice reads only that record`() {
        val page = source("ui/PlanPageScreens.kt")
        assertTrue("open", "connection.plans.openPage = page" in page)
        assertTrue("closed", "connection.plans.openPage = null" in page)
    }
}
