package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A phone and a Mac describing the same task have to land on the same branch
 * name, or the same sentence produces two worktrees. The rules are copied
 * verbatim from `apps/macos/Sources/FarCooler/QuickCreate.swift`; these assert
 * the copy still behaves like the original.
 */
class QuickTaskTest {
    @Test
    fun aSentenceBecomesAGitSafeBranchName() {
        assertEquals("add-oauth-to-the-login-flow", TaskSlug.slug("Add OAuth to the login flow"))
    }

    @Test
    fun punctuationCollapsesRatherThanAccumulating() {
        // A branch name is something people type, paste into a PR title and see
        // in a CI log; one carrying punctuation from a sentence is a small tax
        // paid repeatedly.
        assertEquals("fix-the-parser", TaskSlug.slug("Fix -- the (parser)!!!"))
    }

    @Test
    fun leadingAndTrailingSeparatorsAreDropped() {
        assertEquals("hello", TaskSlug.slug("   hello   "))
        assertEquals("hello", TaskSlug.slug("...hello..."))
    }

    @Test
    fun aSlugIsNeverEmpty() {
        // git refuses an empty ref, and a workspace with no branch is a failure
        // in the middle of a flow rather than at the start of it.
        assertEquals("task", TaskSlug.slug("!!!"))
        assertEquals("task", TaskSlug.slug(""))
    }

    @Test
    fun aSlugIsBounded() {
        val slug = TaskSlug.slug("a".repeat(200))
        assertTrue(slug.length <= 48)
    }

    @Test
    fun aShortSentenceIsItsOwnTitle() {
        assertEquals("Fix the parser", TaskSlug.title("  Fix the parser  "))
    }

    @Test
    fun aLongSentenceIsCutOnAWordBoundary() {
        val title = TaskSlug.title(
            "Rewrite the authentication layer so that every request carries a token"
        )
        assertTrue(title.endsWith("…"))
        assertTrue(title.length <= 43)
        // Cut on a space, so the last word is whole rather than sliced.
        assertTrue(title.dropLast(1).last().isLetterOrDigit())
    }

    @Test
    fun aPresetIsTheAgentAloneUnlessAModelWasChosen() {
        assertEquals("claude", QuickAgents.preset("claude", ""))
        assertEquals("claude:opus", QuickAgents.preset("claude", "opus"))
    }

    @Test
    fun anUnknownAgentFallsBackRatherThanCrashing() {
        // The stored preference outlives the list it was chosen from.
        assertEquals("claude", QuickAgents.agent("something-removed").id)
    }
}
