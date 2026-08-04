package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The Swift suite in `apps/shared/AgentKit/Sources/AgentKit/Composer.swift`'s
 * tests, translated. Getting this subtly different on two platforms means the
 * picker opens in one app and not the other for the same keystrokes.
 */
class ComposerTest {
    @Test
    fun aSlashAtTheStartOpensTheCommandPicker() {
        val token = activeToken("/mod", 4)
        assertEquals("mod", (token as ComposerToken.Slash).prefix)
    }

    @Test
    fun aSlashMidSentenceIsJustASlash() {
        // Otherwise typing a file path or "and/or" pops a command menu over the
        // text the user is writing.
        assertTrue(activeToken("read src/main.rs", 16) is ComposerToken.None)
    }

    @Test
    fun anAtSignOpensTheFilePickerAnywhere() {
        // Unlike a slash command, a file mention is legitimate mid-sentence.
        val token = activeToken("look at @src/ma", 15)
        assertEquals("src/ma", (token as ComposerToken.Mention).prefix)
    }

    @Test
    fun anEmailAddressDoesNotOpenTheFilePicker() {
        // An @ preceded by a word character is part of that word.
        assertTrue(activeToken("mail me@example.com", 19) is ComposerToken.None)
    }

    @Test
    fun aSpaceClosesTheToken() {
        assertTrue(activeToken("@src/main.rs and then", 21) is ComposerToken.None)
    }

    @Test
    fun aCaretRightAfterASpaceIsInsideNothing() {
        // The empty token. Swift reaches this through `token.first` being nil;
        // Kotlin has to say it outright, so it is asserted rather than assumed.
        assertTrue(activeToken("hello ", 6) is ComposerToken.None)
        assertTrue(activeToken("", 0) is ComposerToken.None)
    }

    @Test
    fun theRangeCoversTheWholeTokenIncludingItsSigil() {
        // What a completion replaces. Off by one at either end and accepting a
        // suggestion leaves a stray `@` behind or eats the character before it.
        val token = activeToken("look at @src/ma", 15) as ComposerToken.Mention
        assertEquals(8, token.range.first)
        assertEquals(14, token.range.last)
    }
}
