package com.farcooler.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The Appearance setting's effect (ov-690). */
class AppearanceTest {
    private val bools = listOf(false, true)

    @Test
    fun systemFollowsThePhoneWhateverTheThemeSays() {
        for (system in bools) for (theme in bools) {
            assertEquals(system, Appearance.SYSTEM.dark(systemDark = system, themeDark = theme))
        }
    }

    @Test
    fun lightAndDarkOverrideBoth() {
        for (system in bools) for (theme in bools) {
            assertFalse(Appearance.LIGHT.dark(system, theme))
            assertTrue(Appearance.DARK.dark(system, theme))
        }
    }

    @Test
    fun themeFollowsTheTerminalTheme() {
        for (system in bools) for (theme in bools) {
            assertEquals(theme, Appearance.THEME.dark(system, theme))
        }
    }

    @Test
    fun namesAndOptionsAreTheMacs() {
        assertEquals(listOf("Theme", "System", "Light", "Dark"), Appearance.entries.map { it.label })
    }

    @Test
    fun defaultIsSystemAndAnUnknownStoredValueReadsAsIt() {
        assertEquals(Appearance.SYSTEM, Appearance.DEFAULT)
        assertEquals(Appearance.SYSTEM, Appearance.parse(null))
        assertEquals(Appearance.SYSTEM, Appearance.parse("sepia"))
        for (a in Appearance.entries) assertEquals(a, Appearance.parse(a.wire))
    }
}
