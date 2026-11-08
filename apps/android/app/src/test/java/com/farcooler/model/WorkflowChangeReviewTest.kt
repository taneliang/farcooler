package com.farcooler.model

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The Review screen reads the question the daemon's own producer wrote (ov-978). */
class WorkflowChangeReviewTest {
    private fun real(): String {
        var dir: File? = File("").absoluteFile
        while (dir != null && !File(dir, "crates/client/src/testdata/workflow-change-question.txt").exists()) dir = dir.parentFile
        return File(requireNotNull(dir) { "no repository root above ${File("").absolutePath}" }, "crates/client/src/testdata/workflow-change-question.txt").readText()
    }

    @Test fun `the real question is read in full`() {
        val text = real()
        val review = WorkflowChangeReview.parse(text)!!
        assertEquals("Publish this workflow? 4 checks must pass, each ticket opens a pull request, you approve each ticket.", review.headline)
        assertEquals(2, review.intro.size)
        assertTrue(review.intro[0].startsWith("The orchestrator’s reason:"))
        assertEquals(listOf("What runs as you", "Checks that must pass", "How code lands", "Who approves", "In full, exactly as it runs"), review.parts.map { it.title })
        assertEquals("approve a1b2c3", review.approval)
        // Nothing is dropped between the producer and the screen.
        val bullets = text.lines().filter { it.startsWith("- ") }.map { it.removePrefix("- ") }
        assertTrue(bullets.size >= 20)
        assertEquals(bullets, review.parts.flatMap { it.items })
    }

    @Test fun `a question that is not a change is never given an Approve button`() {
        val text = real()
        assertNull(WorkflowChangeReview.parse("Which branch should I use?"))
        assertNull(WorkflowChangeReview.parse(text.replace("Who approves:\n- You approve each ticket before it lands.\n\n", "")))
        assertNull(WorkflowChangeReview.parse(text.replace("How code lands:", "How it lands:")))
        assertNull(WorkflowChangeReview.parse(text.replace("\n\nTo publish", "\n\nAnd also this.\n\nTo publish")))
        assertNull(WorkflowChangeReview.parse(text.replace("approve a1b2c3", "approve now")))
        assertNull(WorkflowChangeReview.parse(text.replace("answer `decline`", "answer `no`")))
    }
}
