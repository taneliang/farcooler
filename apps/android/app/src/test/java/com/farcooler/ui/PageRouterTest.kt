package com.farcooler.ui

import com.farcooler.model.Fleet
import com.farcooler.model.PageDestination
import com.farcooler.model.PlanPage
import com.farcooler.model.Terminal
import com.farcooler.model.Worktree
import com.farcooler.net.TerminalRef
import org.junit.Assert.assertEquals
import org.junit.Test

/** Where a page's reference goes on Android (ov-285, review L5), and that a web link opens only after its check again. */
class PageRouterTest {
    private val fleet = Fleet(
        worktrees = listOf(
            Worktree(id = "wt-1", task = "integ-10", terminals = listOf(Terminal(id = "t-shell", title = "shell"), Terminal(id = "t-build", title = "build"))),
            Worktree(id = "wt-2", task = "empty"),
        ),
    )
    private val went = mutableListOf<String>()
    private val router = PageRouter(
        "host", fleet,
        onOpenTask = { went += "task $it" },
        onOpenPage = { went += "page ${it.kind} ${it.id}" },
        onNeedsYou = { went += "needs you" },
        onOpenTerminal = { went += "pane ${it.worktreeId} ${it.terminalId}" },
    )

    @Test
    fun `each reference opens what it names, and a question waiting goes to Needs You`() {
        router.open(PageDestination.Task("t1"))
        router.open(PageDestination.Ask("t2"))
        router.open(PageDestination.Lane("l1"))
        router.open(PageDestination.Theme("th1"))
        router.open(PageDestination.Page("spend"))
        router.open(PageDestination.Terminal("wt-1", "build"))
        router.open(PageDestination.Worktree("wt-1"))
        assertEquals(
            listOf("task t1", "needs you", "page lane l1", "page theme th1", "page page spend", "pane wt-1 t-build", "pane wt-1 t-shell"),
            went,
        )
    }

    @Test
    fun `a pane that's gone, a worktree with none, and a web link do nothing here`() {
        router.open(PageDestination.Terminal("wt-1", "gone"))
        router.open(PageDestination.Worktree("wt-2"))
        router.open(PageDestination.Worktree("wt-9"))
        router.open(PageDestination.Url("https://github.com/x"))
        assertEquals(emptyList<String>(), went)
    }

    @Test
    fun `a web link opens in the browser only over https, and the rest go to the app`() {
        val opened = mutableListOf<String>()
        val app = mutableListOf<PageDestination>()
        for (url in listOf("https://github.com/x", "http://github.com/x", "javascript:alert(1)", "https://user@github.com/")) {
            PageOpen.open(PageDestination.Url(url), { app += it }) { opened += it }
        }
        PageOpen.open(PageDestination.Task("t1"), { app += it }) { opened += it }
        assertEquals(listOf("https://github.com/x"), opened)
        assertEquals(listOf<PageDestination>(PageDestination.Task("t1")), app)
    }
}
