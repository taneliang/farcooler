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
    fun aSentenceBecomesAWorktreeNameTheRunnerWillAccept() {
        // A name is a directory now: too long, or nothing left after sanitizing,
        // and the create call fails somewhere in the middle of Quick Task rather
        // than at the start of it.
        assertEquals("fix-the-parser", TaskSlug.name("  Fix the parser  "))
        assertEquals("ship-it", TaskSlug.name("Ship it!!!"))
        assertTrue(TaskSlug.name("a".repeat(200)).length <= 60)
        // A sentence with nothing sanitizable in it still names a directory.
        assertEquals("task", TaskSlug.name("!!!"))
        assertEquals("task", TaskSlug.sanitize(TaskSlug.name("!!!")))

        // Kotlin calls 写 a letter and the runner does not, so a description
        // with no ASCII in it has to fall back rather than hand over a name
        // every character of which the daemon will dash away to nothing.
        assertEquals("task", TaskSlug.name("写代码"))
        // And an accent is dropped rather than transliterated: "caf", not "cafe".
        // The runner would have done exactly this to it.
        assertEquals("caf", TaskSlug.name("Café"))
        assertTrue(
            "every generated name must survive the runner's own sanitizing",
            TaskSlug.sanitize(TaskSlug.name("写代码")).isNotEmpty(),
        )
    }

    @Test
    fun sanitizingAgreesWithTheRunnerAboutWhereANameLands() {
        // Runs collapse and edges are trimmed, so a typed sentence and the slug
        // of that sentence land in the same directory.
        assertEquals("Rate-Limiting", TaskSlug.sanitize("Rate  Limiting!"))
        assertEquals("Rate-Limiting", TaskSlug.sanitize("Rate-Limiting"))
        // Case is information typed on purpose, and underscores are legal.
        assertEquals("Keep_This", TaskSlug.sanitize("Keep_This"))
        // Nothing left is a name the runner refuses, so the forms check for it.
        assertEquals("", TaskSlug.sanitize("!!!"))
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
