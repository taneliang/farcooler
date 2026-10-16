package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * One pull request that wants a review, seen by two runners signed in as one
 * GitHub login, is one row and so one in the Needs You count (ov-589). The
 * count is the length of the merged list, never a sum of the runners' own.
 *
 * Mutation: `NeedsYouItems.merge` without its pr_review filter. Red: 3, not 2.
 */
class NeedsYouRunnerCountTest {
    private fun runner(host: String, vararg items: NeedsYouItem) =
        NeedsYouRunner(host, host, RunnerNeedsYou(items.toList()), Fleet(workspaces = emptyList()), emptyList())

    @Test
    fun `a review request on two runners counts once`() {
        val rows = NeedsYou.rows(
            listOf(
                runner("studio", NeedsYouItem(id = "pr-review:aaaa", kind = "pr_review", rank = 500), NeedsYouItem(id = "ask:1", kind = "ask", rank = 5)),
                runner("build-box", NeedsYouItem(id = "pr-review:aaaa", kind = "pr_review", rank = 400)),
            ),
        )
        assertEquals(2, rows.size)
        assertEquals(1, rows.count { it.item.kindValue == NeedsYouKind.PR_REVIEW })
    }
}
