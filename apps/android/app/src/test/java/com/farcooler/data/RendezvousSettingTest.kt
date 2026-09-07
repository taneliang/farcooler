package com.farcooler.data

import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * What a person may type into the rendezvous field, and what is kept.
 *
 * The setting exists for one day — the day the rendezvous the app ships with
 * stops answering — and every way it can go wrong is silent. A tunneled runner
 * that meets nowhere times out; it does not refuse, and no screen in this app
 * would say why. So the rule is refuse rather than repair, and this holds the
 * Kotlin half of it to the same answers `Account.derpMapSetting` gives on the
 * Apple apps (`apps/shared/AgentKit/Tests/AgentKitTests/RendezvousSettingTests`).
 *
 * The `https` cases are the security ones. A map fetched over cleartext is a
 * map anybody on the path can rewrite, and rewriting it moves BOTH ends of a
 * tunnel onto a rendezvous of the attacker's choosing.
 */
class RendezvousSettingTest {
    /** Empty is the default and empty must never quietly become a URL. */
    @Test
    fun nothingTypedIsNothingKept() {
        assertEquals("", Settings.derpMapSetting(null))
        assertEquals("", Settings.derpMapSetting(""))
        assertEquals("", Settings.derpMapSetting("   "))
        assertEquals("\n was taken", "", Settings.derpMapSetting("\n"))
    }

    @Test
    fun anHttpsUrlIsKeptAsTyped() {
        assertEquals(
            "https://derp.example/derpmap.json",
            Settings.derpMapSetting("https://derp.example/derpmap.json"),
        )
    }

    /** Padding around the whole value is a paste, not a second field. */
    @Test
    fun surroundingWhitespaceIsTrimmedAway() {
        assertEquals(
            "https://derp.example/derpmap.json",
            Settings.derpMapSetting("  https://derp.example/derpmap.json\n"),
        )
    }

    /**
     * Anything that is not an `https` URL comes back empty.
     *
     * Empty means the rendezvous the app ships with, so a value this could not
     * read leaves the device exactly where it was rather than somewhere nobody
     * chose.
     */
    @Test
    fun anythingThatIsNotAnHttpsUrlIsRefused() {
        assertEquals("http was taken", "", Settings.derpMapSetting("http://derp.example/derpmap.json"))
        assertEquals("ftp was taken", "", Settings.derpMapSetting("ftp://derp.example/derpmap.json"))
        assertEquals("a bare host was taken", "", Settings.derpMapSetting("derp.example/derpmap.json"))
        assertEquals("a hostless url was taken", "", Settings.derpMapSetting("https:///derpmap.json"))
        // Refused rather than lowercased. A scheme nobody looked at is a value
        // nobody deliberately chose, which is the whole rule here.
        assertEquals("a shouted scheme was taken", "", Settings.derpMapSetting("HTTPS://derp.example/derpmap.json"))
    }

    /**
     * A space INSIDE the value is refused, not trimmed out of the middle.
     *
     * `farcooler_tailcat::set_derp_map_url` refuses a URL carrying whitespace,
     * because one of its backends sends this to a subprocess over a line
     * protocol whose fields are separated by spaces — so a second field would
     * arrive there as a command nobody issued. Refusing here is what stops such
     * a value being saved, ignored, and never mentioned again.
     */
    @Test
    fun aSecondFieldIsRefusedRatherThanTrimmedOutOfTheMiddle() {
        assertEquals("", Settings.derpMapSetting("https://derp.example/map.json allow bbbb"))
        assertEquals("", Settings.derpMapSetting("https://derp.example/a b.json"))
        assertEquals("", Settings.derpMapSetting("https://derp.example/a\tb.json"))
    }
}
