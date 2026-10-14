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
    private fun page(fixture: PlanFixture, folds: PlanFolds = PlanFolds.Defaults, columns: Int = 1, agents: Boolean = true): @Composable () -> Unit = {
        FarCoolerTheme {
            LazyColumn(Modifier.fillMaxSize()) {
                planItems(
                    PlanReadState.Loaded(fixture.plan), fixture.statuses, onOpen = {}, onRetry = {}, folds = folds,
                    needsYou = fixture.needsYou, onNeedsYou = {}, agentsSupported = agents, themeColumns = columns,
                )
            }
        }
    }

    private fun drawn(fixture: PlanFixture, folds: PlanFolds = PlanFolds.Defaults, columns: Int = 1, agents: Boolean = true) =
        SemanticsProbe.described(content = page(fixture, folds, columns, agents))

    @Test fun `the sections come in the design's order, under the band`() {
        val tags = drawn(PlanFixtures.long())
        val order = listOf(
            "plan-band", "plan-needs-you", "plan-landing", "plan-in-review", "plan-building", "plan-waiting",
            "plan-on-landed-train", "plan-agents", "plan-themes",
        )
        for (tag in order) assertTrue("$tag is drawn: ${tags.keys}", tag in tags)
        val tops = order.map { tags.getValue(it).bounds.top }
        assertEquals("top to bottom: $order", tops.sorted(), tops)
    }

    @Test fun `the band says Now and Next, and Needs you says who`() {
        val tags = drawn(PlanFixtures.long())
        assertTrue(tags.getValue("plan-band-now").words, "Train 89" in tags.getValue("plan-band-now").words)
        assertTrue(tags.getValue("plan-band-next").words, "Onboarding" in tags.getValue("plan-band-next").words)
        // The Needs you section follows the band and says it, so the band doesn't say it twice.
        assertFalse("plan-band-needs-you" in tags)
        assertTrue(tags.getValue("plan-needs-you-row").words, "2 need you" in tags.getValue("plan-needs-you-row").words)
    }

    @Test fun `Landing draws each live train as Train N with its title under it, furthest along first, with its ladder compact`() {
        val tags = drawn(PlanFixtures.long())
        val trains = listOf("integ-89", "integ-86", "integ-84").map { tags.getValue("plan-train-$it").bounds.top }
        assertEquals("Checking on CI, then waiting for lanes, then stalled", trains.sorted(), trains)
        assertEquals("Train 89", tags.getValue("plan-train-integ-89-label").words.substringBefore(" |"))
        assertTrue(tags.getValue("plan-train-integ-89-subtitle").words, tags.getValue("plan-train-integ-89-subtitle").words.startsWith("The agents table"))
        assertEquals("Checking on CI · step 2 of 3", tags.getValue("plan-train-integ-89-phase").words.substringBefore(" |"))
        assertEquals("Waiting for lanes · step 1 of 3", tags.getValue("plan-train-integ-86-phase").words.substringBefore(" |"))
        assertEquals("Stalled · step 1 of 3", tags.getValue("plan-train-integ-84-phase").words.substringBefore(" |"))
        assertTrue("a lane simply aboard says nothing", "plan-train-lane-team-dir-note" !in tags)
        assertTrue(tags.getValue("plan-train-integ-86-held").words, "Waiting for ios-overwrite" in tags.getValue("plan-train-integ-86-held").words)
        val facts = tags.getValue("plan-train-integ-89-words").words.replace('\u00a0', ' ')
        assertTrue(facts, "3f78754f" in facts && "2 of 5 jobs done" in facts && "Integrator" in facts)
        assertTrue("a landed train is not in Landing", "plan-train-integ-83" !in tags)
        assertTrue("no lane says it holds the train", tags.values.none { "holding the train" in it.words.lowercase() })
        assertTrue("lanes sit inside their train", "plan-train-lane-team-dir" in tags && "plan-train-lane-ios-overwrite" in tags)
    }

    @Test fun `In review and Building rows carry their agent and the landed train they still name`() {
        val tags = drawn(PlanFixtures.long())
        for (lane in listOf("fleet-ui", "agent-rows", "fix-482", "tile-grid", "lane-474")) assertTrue("$lane is a row", "plan-lane-$lane" in tags)
        assertTrue(tags.getValue("plan-lane-tile-grid-agent").words, "Builder · Sonnet" in tags.getValue("plan-lane-tile-grid-agent").words)
        assertFalse("the reviewer finished, so the lane has no agent line", "plan-lane-fleet-ui-agent" in tags)
        assertTrue(tags.getValue("plan-lane-fix-482-landed").words, "Names Train 83, which landed" in tags.getValue("plan-lane-fix-482-landed").words)
        val review = tags.getValue("plan-in-review").bounds.top
        val building = tags.getValue("plan-building").bounds.top
        assertTrue(tags.getValue("plan-lane-fleet-ui").bounds.top in review..building)
        assertTrue(tags.getValue("plan-lane-tile-grid").bounds.top > building)
    }

    @Test fun `Waiting ranks Next up, then names Not ranked`() {
        val tags = drawn(PlanFixtures.long())
        assertTrue("plan-next-up" in tags && "plan-not-ranked" in tags)
        val ranked = listOf("onboarding-copy", "tailcat-default", "hero-shot", "docs-pass").map { tags.getValue("plan-lane-$it").bounds.top }
        assertEquals(ranked.sorted(), ranked)
    }

    @Test fun `On a landed train is folded until opened`() {
        val closed = drawn(PlanFixtures.long())
        assertFalse("its row hides", "plan-lane-old-queued" in closed)
        val open = drawn(PlanFixtures.long(), PlanFolds(listOf("landed-train")) {})
        assertTrue("plan-lane-old-queued" in open)
        assertTrue(open.getValue("plan-lane-old-queued-landed").words, "Names Train 85" in open.getValue("plan-lane-old-queued-landed").words)
    }

    @Test fun `Agents lists who exists and what it costs, with no Finished fold`() {
        val drawn = drawn(PlanFixtures.long())
        assertTrue(drawn.getValue("plan-agents-header").words, "4 working" in drawn.getValue("plan-agents-header").words)
        assertTrue("agents are rows", "plan-agent-pane:orch" in drawn)
        val row = drawn.getValue("plan-agent-pane:orch").words
        assertTrue(row, "Orchestrator · Opus" in row && "2.1M" in row && "≈" in row)
        assertFalse("there is no Finished fold", "plan-agents-finished" in drawn)
        assertFalse("a finished reviewer is not listed", drawn.keys.any { it.startsWith("plan-agent-") && "Reviewer" in drawn.getValue(it).words })
        val resumed = drawn.getValue("plan-agent-agent:s1").words
        assertTrue("a subagent resumed for another round reads Working: $resumed", "Working" in resumed && "Not on a lane" in resumed)
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
        assertEquals("No train is landing.", sections.emptyWords(PlanSections.Kind.LANDING))
        assertTrue(tags.getValue("plan-landing-empty").words, "No train is landing." in tags.getValue("plan-landing-empty").words)
        assertTrue("plan-waiting" in tags)
        val agents = drawn(PlanFixtures.case("board-agents-empty"))
        assertTrue(agents.getValue("plan-agents-empty").words, "No agents running." in agents.getValue("plan-agents-empty").words)
        // A runner without board_agents draws no Agents section at all.
        assertFalse("plan-agents" in drawn(PlanFixtures.case("board-old-runner"), agents = false))
        val building = drawn(PlanFixtures.case("next-ranked"))
        assertTrue(building.getValue("plan-building-empty").words, "Nothing is building." in building.getValue("plan-building-empty").words)
        assertTrue("plan-waiting-empty" !in building)
    }

    @Test fun `an old runner's board still draws, from state alone`() {
        val tags = drawn(PlanFixtures.case("board-old-runner"), agents = false)
        assertTrue("plan-landing" in tags && "plan-themes" in tags)
        assertFalse("plan-agents" in tags)
    }

    // The pictures: the whole plan, tall, on a phone and a tablet pane, in light and dark.
    @Test fun phone() = Capture.both("p7-phone-plan") { page(PlanFixtures.long())() }

    @Test @Config(qualifiers = "w320dp-h4200dp-xhdpi")
    fun phoneNarrow() = Capture.both("p7-phone-plan-320") { page(PlanFixtures.long())() }

    @Test @Config(qualifiers = "w480dp-h4200dp-xhdpi")
    fun tabletPane() = Capture.both("p7-tablet-plan-pane-480") { page(PlanFixtures.long(), columns = 2)() }

    @Test fun phoneFolded() = Capture.both("p7-phone-plan-open-folds") {
        page(PlanFixtures.long(), PlanFolds(listOf("landed-train")) {})()
    }

    @Test @Config(qualifiers = "w411dp-h1400dp-xhdpi")
    fun phoneOldRunner() = Capture.both("p7-phone-plan-old-runner") { page(PlanFixtures.case("board-old-runner"), agents = false)() }

    @Test @Config(qualifiers = "w411dp-h1000dp-xhdpi")
    fun phoneEmptySections() = Capture.both("p7-phone-plan-empty-sections") { page(PlanFixtures.case("next-empty"))() }
}
