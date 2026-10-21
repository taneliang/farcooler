package com.farcooler.capture

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import com.farcooler.model.LaneTree
import com.farcooler.model.OneTree
import com.farcooler.model.PlanSections
import com.farcooler.model.ThemeTree
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.PlanFolds
import com.farcooler.ui.SidebarActions
import com.farcooler.ui.SidebarChoice
import com.farcooler.ui.SidebarModels
import com.farcooler.ui.laneListItems
import com.farcooler.ui.sidebarPaneItems
import com.farcooler.ui.sidebarRootRows
import com.farcooler.ui.themeListItems
import com.farcooler.ui.TreeFilterChips
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The sidebar as two lists (ov-503, design 3), on a phone and a tablet: Themes › Cards and Lanes › Agents, drawn by the
 * app's own rows from `ThemeTree` and `LaneTree`, over the long-titled board. A card and the lane working it light
 * each other. The windows are tall so every row is composed. Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h2600dp-xhdpi")
class SidebarListsViewTest {
    private val fixture = PlanFixtures.long()
    private val plan = fixture.plan
    private fun models(filter: OneTree.Filter = OneTree.Filter.OPEN) = SidebarModels(
        ThemeTree.of(plan, filter, fixture.statuses),
        LaneTree.of(plan, PlanSections.of(plan, fixture.needsYou, fixture.statuses)),
    )

    private val opened = mutableListOf<String>()
    private val actions = SidebarActions(
        onOpenTask = { opened += "task:$it" }, onOpenLane = { opened += "lane:$it" }, onOpenTheme = { opened += "theme:$it" },
        onOpenTerminal = { opened += "terminal:$it"; true },
    )

    private fun themes(choice: SidebarChoice? = null, folds: PlanFolds = PlanFolds.Defaults, filter: OneTree.Filter = OneTree.Filter.OPEN): @Composable () -> Unit = {
        val m = models(filter)
        FarCoolerTheme { LazyColumn(Modifier.fillMaxSize()) { themeListItems(m.themes, folds, choice, m.lanes, actions) {} } }
    }

    private fun lanes(choice: SidebarChoice? = null, folds: PlanFolds = PlanFolds.Defaults): @Composable () -> Unit = {
        val m = models()
        FarCoolerTheme { LazyColumn(Modifier.fillMaxSize()) { laneListItems(m.lanes, m.themes, folds, choice, actions) {} } }
    }

    private fun stacked(choice: SidebarChoice? = null): @Composable () -> Unit = {
        val m = models()
        FarCoolerTheme {
            LazyColumn(Modifier.fillMaxSize()) {
                sidebarPaneItems(m, { TreeFilterChips(OneTree.Filter.OPEN) {} }, PlanFolds.Defaults, choice, actions) {}
            }
        }
    }

    private fun drawn(content: @Composable () -> Unit) = SemanticsProbe.described(content = content)

    @Test fun `Themes lists each theme with its progress, and the cards of the ones a lane works`() {
        val tags = drawn(themes())
        assertTrue(tags.keys.toString(), "theme-row-Trustworthy agent view: every harness message renders, and nothing is lost" in tags)
        assertTrue(tags.getValue("theme-row-Teams and accounts, so more than one person can run the same board").words.contains("2/5"))
        // A theme a lane works opens by itself; Marketing's cards have no lane in motion and a lane on a train counts, so check both.
        assertTrue("a worked card shows", "card-row-ov-487" in tags)
        assertTrue(tags.getValue("card-row-ov-487").words, "team-dir" in tags.getValue("card-row-ov-487").words)
        assertFalse("a finished theme isn't listed under Not done", tags.keys.any { "Mac look and feel" in it })
    }

    @Test fun `the filter applies to cards, and In review shows only what is in review`() {
        val inReview = drawn(themes(filter = OneTree.Filter.IN_REVIEW, folds = PlanFolds(listOf(), {})))
        assertTrue("card-row-ov-311" in inReview)
        assertFalse("card-row-ov-310" in inReview)
        val all = drawn(themes(filter = OneTree.Filter.ALL))
        assertTrue("a done card is there under All", "card-row-ov-486" in all)
    }

    @Test fun `Lanes groups Landing first with its trains inside it, then In review and Building, with Waiting folded and agents under their lane`() {
        val tags = drawn(lanes())
        val groups = listOf("Landing", "In review", "Building", "Waiting", "Waiting for the next landing")
            .map { g -> tags.keys.first { it.startsWith("lane-group-") && it.removePrefix("lane-group-").startsWith(g) } }
        val tops = groups.map { tags.getValue(it).bounds.top }
        assertEquals("Landing first, then the groups, in the Plan's order", tops.sorted(), tops)
        val trains = listOf("Train 89", "Train 86", "Train 84").map { tags.getValue("lane-train-$it").bounds.top }
        assertEquals("the trains sit inside Landing, furthest along first", trains.sorted(), trains)
        assertTrue("Landing comes first", trains.first() > tags.getValue("lane-group-Landing").bounds.top && trains.last() < tags.getValue("lane-group-In review").bounds.top)
        assertTrue(tags.getValue("lane-train-Train 89").words, "The agents table" in tags.getValue("lane-train-Train 89").words)
        assertTrue("where it stands leads the line", tags.getValue("lane-train-Train 89-line").words.startsWith("Checking on CI"))
        assertFalse("no train-named group sits beside Building or Waiting", tags.keys.any { it.startsWith("lane-group-Train") })
        assertTrue("Waiting starts folded", tags.getValue("lane-group-Waiting").words.contains("Collapsed"))
        assertFalse("its lanes are hidden", "lane-row-hero-shot" in tags)
        assertTrue("lane-row-fleet-ui" in tags)
        assertTrue(tags.keys.toString(), tags.keys.any { it.startsWith("agent-row-") && "Builder · Sonnet" in tags.getValue(it).words && "380K" in tags.getValue(it).words })
        val open = drawn(lanes(folds = PlanFolds(listOf("group:Waiting"), {})))
        assertTrue("hero-shot shows once Waiting opens", "lane-row-hero-shot" in open)
    }

    @Test fun `choosing a card lights the lane working it, and choosing a lane lights each of its cards`() {
        val fromCard = drawn(lanes(choice = SidebarChoice.Card("t-ov-310")))
        assertTrue(fromCard.getValue("lane-row-tile-grid").words, "Works the card you chose" in fromCard.getValue("lane-row-tile-grid").words)
        assertFalse("only that lane", "Works the card you chose" in fromCard.getValue("lane-row-fleet-ui").words)
        val laneId = plan.lanes.first { it.name == "tile-grid" }.id
        val fromLane = drawn(themes(choice = SidebarChoice.Lane(laneId)))
        assertTrue(fromLane.getValue("card-row-ov-310").words, "Shares a lane" in fromLane.getValue("card-row-ov-310").words)
        assertFalse("never the chosen side", "Shares a lane" in fromLane.getValue("card-row-ov-311").words)
    }

    @Test fun `a tablet's list pane stacks both lists, and a phone's root has two rows`() {
        val pane = drawn(stacked())
        assertTrue("sidebar-themes" in pane && "sidebar-lanes" in pane)
        assertTrue(pane.getValue("sidebar-themes").bounds.top < pane.getValue("sidebar-lanes").bounds.top)
        val root = drawn {
            FarCoolerTheme { LazyColumn(Modifier.fillMaxSize()) { sidebarRootRows(models()) { opened += "level:$it" } } }
        }
        assertTrue(root.getValue("sidebar-row-themes").words, "themes" in root.getValue("sidebar-row-themes").words)
        assertTrue(root.getValue("sidebar-row-lanes").words, "lanes" in root.getValue("sidebar-row-lanes").words)
        assertFalse("no list in place on a phone", "sidebar-themes" in root)
    }

    @Test fun `an empty board says so in each list`() {
        val empty = SidebarModels(ThemeTree.of(com.farcooler.model.Plan()), LaneTree.of(com.farcooler.model.Plan()))
        val tags = drawn { FarCoolerTheme { LazyColumn(Modifier.fillMaxSize()) {
            themeListItems(empty.themes, PlanFolds.Defaults, null, empty.lanes, actions) {}
            laneListItems(empty.lanes, empty.themes, PlanFolds.Defaults, null, actions) {}
        } } }
        assertTrue("themes-empty" in tags && "lanes-empty" in tags)
    }

    // The pictures.
    @Test fun phoneThemes() = Capture.both("p7-phone-themes-list") { themes()() }

    @Test fun phoneLanes() = Capture.both("p7-phone-lanes-list") { lanes(folds = PlanFolds(listOf("group:Waiting"), {}))() }

    @Test @Config(qualifiers = "w411dp-h700dp-xhdpi")
    fun phoneRoot() = Capture.both("p7-phone-tree-root") {
        FarCoolerTheme { LazyColumn(Modifier.fillMaxSize()) { sidebarRootRows(models()) {} } }
    }

    @Test @Config(qualifiers = "w280dp-h2600dp-xhdpi")
    fun tabletListPane() = Capture.both("p7-tablet-list-pane") { stacked(SidebarChoice.Card("t-ov-310"))() }

    @Test @Config(qualifiers = "w280dp-h2600dp-xhdpi")
    fun tabletListPaneLane() = Capture.both("p7-tablet-list-pane-lane-chosen") { stacked(SidebarChoice.Lane(plan.lanes.first { it.name == "team-dir" }.id))() }
}
