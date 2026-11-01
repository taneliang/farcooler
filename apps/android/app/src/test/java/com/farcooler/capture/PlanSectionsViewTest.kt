package com.farcooler.capture

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import com.farcooler.model.PlanReadState
import com.farcooler.model.PlanSections
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.PlanFolds
import com.farcooler.ui.planItems
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The Plan's sections on a phone and a tablet (ov-503, design 7), drawn by the app's own `planItems` from the shared
 * cases (`test/fixtures/plan-sections-cases.json`) and the long-titled board (`plan-android-long.json`). The windows
 * are tall so that every row is composed and the tests can read what is where. Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h4200dp-xhdpi")
class PlanSectionsViewTest {
    private fun page(fixture: PlanFixture, folds: PlanFolds = PlanFolds.Defaults, columns: Int = 1): @Composable () -> Unit = {
        FarCoolerTheme {
            LazyColumn(Modifier.fillMaxSize()) {
                planItems(
                    PlanReadState.Loaded(fixture.plan), fixture.statuses, onOpen = {}, onRetry = {}, folds = folds,
                    needsYou = fixture.needsYou, onNeedsYou = {}, themeColumns = columns,
                )
            }
        }
    }

    private fun drawn(fixture: PlanFixture, folds: PlanFolds = PlanFolds.Defaults, columns: Int = 1) =
        SemanticsProbe.described(content = page(fixture, folds, columns))

    @Test fun `the sections come in the design's order, under the band`() {
        val tags = drawn(PlanFixtures.long())
        val order = listOf(
            "plan-band", "plan-needs-you", "plan-landing", "plan-in-review", "plan-building", "plan-waiting",
            "plan-agents", "plan-themes",
        )
        for (tag in order) assertTrue("$tag is drawn: ${tags.keys}", tag in tags)
        val tops = order.map { tags.getValue(it).bounds.top }
        assertEquals("top to bottom: $order", tops.sorted(), tops)
    }

    @Test fun `the band says Now and Next, and Needs you says who`() {
        val tags = drawn(PlanFixtures.long())
        assertTrue(tags.getValue("plan-band-now").words, "Landing 89" in tags.getValue("plan-band-now").words)
        assertTrue(tags.getValue("plan-band-next").words, "Onboarding" in tags.getValue("plan-band-next").words)
        // The Needs you section follows the band and says it, so the band doesn't say it twice.
        assertFalse("plan-band-needs-you" in tags)
        assertTrue(tags.getValue("plan-needs-you-row").words, "2 need you" in tags.getValue("plan-needs-you-row").words)
    }

    @Test fun `Landing draws each live landing as Landing N with its title under it, furthest along first, with its ladder compact`() {
        val tags = drawn(PlanFixtures.long())
        val landings = listOf(89, 86, 84).map { tags.getValue("plan-landing-$it").bounds.top }
        assertEquals("Checking on CI, then waiting for main, then stalled", landings.sorted(), landings)
        assertEquals("Landing 89", tags.getValue("plan-landing-89-label").words.substringBefore(" |"))
        assertTrue(tags.getValue("plan-landing-89-subtitle").words, tags.getValue("plan-landing-89-subtitle").words.startsWith("The agents table"))
        assertEquals("Checking on CI · step 2 of 3", tags.getValue("plan-landing-89-phase").words.substringBefore(" |"))
        assertEquals("Waiting for main's checks to pass · step 1 of 3", tags.getValue("plan-landing-86-phase").words.substringBefore(" |"))
        assertEquals("Stalled · step 1 of 3", tags.getValue("plan-landing-84-phase").words.substringBefore(" |"))
        assertTrue("a ticket simply aboard says nothing", "plan-landing-ticket-ov-485-note" !in tags)
        assertTrue("no landing says who holds it", tags.keys.none { it.endsWith("-held") })
        val facts = tags.getValue("plan-landing-89-words").words.replace('\u00a0', ' ')
        assertTrue(facts, "3f78754f" in facts && "2 of 5 jobs done" in facts && "Lander" in facts)
        assertTrue("a landed landing is not in Landing", "plan-landing-83" !in tags)
        assertTrue("tickets sit inside their landing", "plan-landing-ticket-ov-487" in tags && "plan-landing-ticket-ov-485" in tags)
    }

    @Test fun `In review and Building rows carry their agent`() {
        val tags = drawn(PlanFixtures.long())
        for (ticket in listOf("ov-311", "ov-312", "ov-310", "ov-474")) assertTrue("$ticket is a row", "plan-ticket-$ticket" in tags)
        assertTrue(tags.getValue("plan-ticket-ov-310-agent").words, "Builder · Sonnet" in tags.getValue("plan-ticket-ov-310-agent").words)
        val review = tags.getValue("plan-in-review").bounds.top
        val building = tags.getValue("plan-building").bounds.top
        assertTrue(tags.getValue("plan-ticket-ov-311").bounds.top in review..building)
        assertTrue(tags.getValue("plan-ticket-ov-310").bounds.top > building)
    }

    @Test fun `Waiting ranks Next up, then names Not ranked`() {
        val tags = drawn(PlanFixtures.long())
        assertTrue("plan-next-up" in tags && "plan-not-ranked" in tags)
        val ranked = listOf("onboarding-copy", "tailcat-default", "ov-302").map { tags.getValue("plan-ticket-$it").bounds.top }
        assertEquals(ranked.sorted(), ranked)
    }

    @Test fun `Agents lists who exists and what it costs, with no Finished fold`() {
        val drawn = drawn(PlanFixtures.long())
        assertTrue(drawn.getValue("plan-agents-header").words, "working" in drawn.getValue("plan-agents-header").words)
        assertTrue("agents are rows", "plan-agent-pane:orch" in drawn)
        val row = drawn.getValue("plan-agent-pane:orch").words
        assertTrue(row, "Orchestrator · Opus" in row && "2.1M" in row && "≈" in row)
        assertFalse("there is no Finished fold", "plan-agents-finished" in drawn)
        val resumed = drawn.getValue("plan-agent-agent:s1").words
        assertTrue("a subagent resumed for another round reads Working: $resumed", "Working" in resumed && "Not on a ticket" in resumed)
    }

    @Test fun `Themes are one column on a phone and two on a tablet, the finished ones in one tile`() {
        val one = drawn(PlanFixtures.long(), columns = 1)
        val tiles = one.keys.filter { it.startsWith("plan-theme-tile-") }
        assertTrue("three live themes: $tiles", tiles.size == 3)
        assertEquals("one column: every tile at the same left", 1, tiles.map { one.getValue(it).bounds.left }.distinct().size)
        assertTrue("plan-finished-tile" in one)
        assertTrue(one.getValue("plan-finished-tile").words, "Mac look and feel" in one.getValue("plan-finished-tile").words)
        val two = drawn(PlanFixtures.long(), columns = 2)
        val lefts = two.keys.filter { it.startsWith("plan-theme-tile-") }.map { two.getValue(it).bounds.left }.distinct()
        assertEquals("two columns: two lefts", 2, lefts.size)
        // The owner's ask is the one amber line on its tile.
        assertTrue(one.getValue("plan-theme-tile-Marketing").words, "Needs you: How do we ship the site?" in one.getValue("plan-theme-tile-Marketing").words)
    }

    @Test fun `the empty sections say so`() {
        val fixture = PlanFixtures.case("next-empty")
        val sections = PlanSections.of(fixture.plan, fixture.needsYou, fixture.statuses)
        val tags = drawn(fixture)
        assertEquals("Nothing is landing right now.", sections.emptyWords(PlanSections.Kind.LANDING))
        assertTrue(tags.getValue("plan-landing-empty").words, "Nothing is landing right now." in tags.getValue("plan-landing-empty").words)
        assertTrue("plan-waiting" in tags)
        val agents = drawn(PlanFixtures.case("board-agents-empty"))
        assertTrue(agents.getValue("plan-agents-empty").words, "No agents running." in agents.getValue("plan-agents-empty").words)
        val building = drawn(PlanFixtures.case("next-ranked"))
        assertTrue(building.getValue("plan-building-empty").words, "Nothing is building." in building.getValue("plan-building-empty").words)
        assertTrue("plan-waiting-empty" !in building)
    }

    @Test fun `a board with no phases and no agents still draws, from stage alone`() {
        val tags = drawn(PlanFixtures.case("board-no-phases"))
        assertTrue("plan-landing" in tags && "plan-themes" in tags)
        assertTrue("plan-agents-empty" in tags)
    }

    // The pictures: the whole plan, tall, on a phone and a tablet pane, in light and dark.
    @Test fun phone() = Capture.both("p7-phone-plan") { page(PlanFixtures.long())() }

    @Test @Config(qualifiers = "w320dp-h4200dp-xhdpi")
    fun phoneNarrow() = Capture.both("p7-phone-plan-320") { page(PlanFixtures.long())() }

    @Test @Config(qualifiers = "w480dp-h4200dp-xhdpi")
    fun tabletPane() = Capture.both("p7-tablet-plan-pane-480") { page(PlanFixtures.long(), columns = 2)() }

    @Test fun phoneFolded() = Capture.both("p7-phone-plan-open-folds") {
        page(PlanFixtures.long(), PlanFolds(listOf("landed-today")) {})()
    }

    @Test @Config(qualifiers = "w411dp-h1400dp-xhdpi")
    fun phoneNoPhases() = Capture.both("p7-phone-plan-no-phases") { page(PlanFixtures.case("board-no-phases"))() }

    @Test @Config(qualifiers = "w411dp-h1000dp-xhdpi")
    fun phoneEmptySections() = Capture.both("p7-phone-plan-empty-sections") { page(PlanFixtures.case("next-empty"))() }
}
