package com.farcooler.capture

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.ui.Modifier
import com.farcooler.model.Plan
import com.farcooler.model.PlanGlanceSentence
import com.farcooler.model.PlanReadState
import com.farcooler.model.PlanSections
import com.farcooler.model.PlanStrip
import com.farcooler.model.Terminal
import com.farcooler.ui.BandShown
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.PlanGlanceBand
import com.farcooler.ui.WorkspaceCards
import com.farcooler.ui.peekBand
import com.farcooler.ui.planItems
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

/**
 * The plan peek says Now and Next once, and its card's chevron points down while it is open (ov-659). The card behind
 * the sheet is the short form of the plan's own band, so with the sheet up the band is not drawn again under it.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class PlanPeekViewTest {
    private val plan = Plan.decode(Json.parseToJsonElement(repositoryFile("test/fixtures/plan-seeded.json")).jsonObject["plan"]!!.jsonObject)
    private val strip = PlanStrip.of(plan, 2, Terminal(id = "o", state = "running", activity = "working", role = "orchestrator", line = "Dispatching"))

    private fun peek(open: Boolean, band: BandShown) = SemanticsProbe.described {
        FarCoolerTheme {
            Column(Modifier.fillMaxSize()) {
                WorkspaceCards(strip, AgentsFixtures.running(), onOpenPlan = {}, onOpenAgents = {}, planOpen = open)
                LazyColumn(Modifier.fillMaxSize()) {
                    planItems(PlanReadState.Loaded(plan), emptyMap(), onOpen = {}, onRetry = {}, needsYou = strip.needsYou, band = band)
                }
            }
        }
    }

    @Test fun `an open peek says Now and Next once, in the card`() {
        val tags = peek(open = true, band = peekBand(1f, 411, false))
        assertTrue("the card says them", "plan-band-now" in tags && "plan-band-next" in tags)
        assertFalse("the sheet's own band is not drawn under it: ${tags.keys}", "plan-band" in tags)
    }

    @Test fun `the plan home beside the chat still opens with its band`() {
        val tags = peek(open = false, band = BandShown.ALL)
        assertTrue("plan-band" in tags)
    }

    @Test fun `the card's chevron points down while the peek is open and up when it is closed`() {
        val open = peek(open = true, band = BandShown.NONE)
        assertTrue("plan-strip-chevron-down" in open && "plan-strip-chevron-up" !in open)
        val closed = peek(open = false, band = BandShown.NONE)
        assertTrue("plan-strip-chevron-up" in closed && "plan-strip-chevron-down" !in closed)
    }

    @Test fun `at large font sizes the card leaves Next to the sheet, so the sheet says Next and not Now again`() {
        assertEquals(BandShown.NONE, peekBand(1f, 411, false))
        assertEquals(BandShown.NEXT_ONLY, peekBand(1.3f, 411, false))
        assertEquals("a narrow card leaves Next to the sheet", BandShown.NEXT_ONLY, peekBand(1f, 320, false))
        assertEquals("a sheet open over the card says both", BandShown.ALL, peekBand(1f, 411, true))
        assertEquals(BandShown.ALL, peekBand(1.5f, 320, true))
        val tags = SemanticsProbe.described {
            FarCoolerTheme { PlanGlanceBand(PlanGlanceSentence.of(PlanSections.of(plan, 0, emptyMap())), 0, shown = BandShown.NEXT_ONLY) }
        }
        assertTrue("plan-band-next" in tags)
        assertFalse("plan-band-now" in tags)
    }

    private fun repositoryFile(relative: String): String {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val candidate = File(directory, relative)
            if (candidate.isFile) return candidate.readText()
            directory = directory.parentFile
        }
        throw AssertionError("Could not find $relative above ${System.getProperty("user.dir")}.")
    }
}
