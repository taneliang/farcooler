package com.farcooler.capture

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import com.farcooler.model.OneTree
import com.farcooler.model.PlanSections
import com.farcooler.model.ThemeTree
import com.farcooler.model.TicketTree
import com.farcooler.ui.FarCoolerTheme
import com.farcooler.ui.PlanFolds
import com.farcooler.ui.SidebarActions
import com.farcooler.ui.SidebarChoice
import com.farcooler.ui.SidebarModels
import com.farcooler.ui.ticketListItems
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
 * The sidebar as two lists (ov-503, design 3), on a phone and a tablet: Themes › Cards and Tickets › Agents, drawn by the
 * app's own rows from `ThemeTree` and `TicketTree`, over the long-titled board. A card and its ticket light
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
        TicketTree.of(plan, PlanSections.of(plan, fixture.needsYou, fixture.statuses)),
    )

    private val opened = mutableListOf<String>()
    private val actions = SidebarActions(
        onOpenTask = { opened += "task:$it" }, onOpenTicket = { opened += "ticket:$it" }, onOpenTheme = { opened += "theme:$it" },
        onOpenTerminal = { opened += "terminal:$it"; true },
    )

    private fun themes(choice: SidebarChoice? = null, folds: PlanFolds = PlanFolds.Defaults, filter: OneTree.Filter = OneTree.Filter.OPEN): @Composable () -> Unit = {
        val m = models(filter)
        FarCoolerTheme { LazyColumn(Modifier.fillMaxSize()) { themeListItems(m.themes, folds, choice, m.tickets, actions) {} } }
    }

    private fun tickets(choice: SidebarChoice? = null, folds: PlanFolds = PlanFolds.Defaults): @Composable () -> Unit = {
        val m = models()
        FarCoolerTheme { LazyColumn(Modifier.fillMaxSize()) { ticketListItems(m.tickets, m.themes, folds, choice, actions) {} } }
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

    @Test fun `Themes lists each theme with its progress, and the cards of the ones with a ticket in motion`() {
        val tags = drawn(themes())
        assertTrue(tags.keys.toString(), "theme-row-Trustworthy agent view: every harness message renders, and nothing is lost" in tags)
        assertTrue(tags.getValue("theme-row-Teams and accounts, so more than one person can run the same board").words.contains("2/5"))
        // A theme with a ticket on the line opens by itself; Marketing's cards have none in motion, so check both.
        assertTrue("a worked card shows", "card-row-ov-487" in tags)
        assertTrue(tags.getValue("card-row-ov-487").words, "Landing" in tags.getValue("card-row-ov-487").words)
        assertFalse("a finished theme isn't listed under Not done", tags.keys.any { "Mac look and feel" in it })
    }

    @Test fun `the filter applies to cards, and In review shows only what is in review`() {
        val inReview = drawn(themes(filter = OneTree.Filter.IN_REVIEW, folds = PlanFolds(listOf(), {})))
        assertTrue("card-row-ov-311" in inReview)
        assertFalse("card-row-ov-310" in inReview)
        val all = drawn(themes(filter = OneTree.Filter.ALL))
        assertTrue("a done card is there under All", "card-row-ov-486" in all)
    }

    @Test fun `Tickets groups Landing first with its landings inside it, then In review and Building, with Waiting folded and agents under their ticket`() {
        val tags = drawn(tickets())
        val groups = listOf("Landing", "In review", "Building", "Waiting")
            .map { g -> tags.keys.first { it.startsWith("ticket-group-") && it.removePrefix("ticket-group-").startsWith(g) } }
        val tops = groups.map { tags.getValue(it).bounds.top }
        assertEquals("Landing first, then the groups, in the Plan's order", tops.sorted(), tops)
        val landings = listOf("Landing 89", "Landing 86", "Landing 84").map { tags.getValue("ticket-landing-$it").bounds.top }
        assertEquals("the landings sit inside Landing, furthest along first", landings.sorted(), landings)
        assertTrue("Landing comes first", landings.first() > tags.getValue("ticket-group-Landing").bounds.top && landings.last() < tags.getValue("ticket-group-In review").bounds.top)
        assertTrue(tags.getValue("ticket-landing-Landing 89").words, "The agents table" in tags.getValue("ticket-landing-Landing 89").words)
        assertTrue("where it stands leads the line", tags.getValue("ticket-landing-Landing 89-line").words.startsWith("Checking on CI"))
        assertFalse("no landing-named group sits beside Building or Waiting", tags.keys.any { it.startsWith("ticket-group-Landing ") })
        assertTrue("Waiting starts folded", tags.getValue("ticket-group-Waiting").words.contains("Collapsed"))
        assertFalse("its tickets are hidden", "ticket-row-ov-302" in tags)
        assertTrue("ticket-row-ov-311" in tags)
        assertTrue(tags.keys.toString(), tags.keys.any { it.startsWith("agent-row-") && "Builder · Sonnet" in tags.getValue(it).words })
        val open = drawn(tickets(folds = PlanFolds(listOf("group:Waiting"), {})))
        assertTrue("ov-302 shows once Waiting opens", "ticket-row-ov-302" in open)
    }

    @Test fun `choosing a card lights its ticket, and choosing a ticket lights its card`() {
        val fromCard = drawn(tickets(choice = SidebarChoice.Card("t-ov-310")))
        assertTrue(fromCard.getValue("ticket-row-ov-310").words, "Is the card you chose" in fromCard.getValue("ticket-row-ov-310").words)
        assertFalse("only that ticket", "Is the card you chose" in fromCard.getValue("ticket-row-ov-311").words)
        val ticketId = plan.tickets.first { it.key == "ov-310" }.task
        val fromTicket = drawn(themes(choice = SidebarChoice.Ticket(ticketId)))
        assertTrue(fromTicket.getValue("card-row-ov-310").words, "On the ticket you chose" in fromTicket.getValue("card-row-ov-310").words)
        assertFalse("never the chosen side", "On the ticket you chose" in fromTicket.getValue("card-row-ov-311").words)
    }

    @Test fun `a tablet's list pane stacks both lists, and a phone's root has two rows`() {
        val pane = drawn(stacked())
        assertTrue("sidebar-themes" in pane && "sidebar-tickets" in pane)
        assertTrue(pane.getValue("sidebar-themes").bounds.top < pane.getValue("sidebar-tickets").bounds.top)
        val root = drawn {
            FarCoolerTheme { LazyColumn(Modifier.fillMaxSize()) { sidebarRootRows(models()) { opened += "level:$it" } } }
        }
        assertTrue(root.getValue("sidebar-row-themes").words, "themes" in root.getValue("sidebar-row-themes").words)
        assertTrue(root.getValue("sidebar-row-tickets").words, "tickets" in root.getValue("sidebar-row-tickets").words)
        assertFalse("no list in place on a phone", "sidebar-themes" in root)
    }

    @Test fun `an empty board says so in each list`() {
        val empty = SidebarModels(ThemeTree.of(com.farcooler.model.Plan()), TicketTree.of(com.farcooler.model.Plan()))
        val tags = drawn { FarCoolerTheme { LazyColumn(Modifier.fillMaxSize()) {
            themeListItems(empty.themes, PlanFolds.Defaults, null, empty.tickets, actions) {}
            ticketListItems(empty.tickets, empty.themes, PlanFolds.Defaults, null, actions) {}
        } } }
        assertTrue("themes-empty" in tags && "tickets-empty" in tags)
    }

    // The pictures.
    @Test fun phoneThemes() = Capture.both("p7-phone-themes-list") { themes()() }

    @Test fun phoneTickets() = Capture.both("p7-phone-tickets-list") { tickets(folds = PlanFolds(listOf("group:Waiting"), {}))() }

    @Test @Config(qualifiers = "w411dp-h700dp-xhdpi")
    fun phoneRoot() = Capture.both("p7-phone-tree-root") {
        FarCoolerTheme { LazyColumn(Modifier.fillMaxSize()) { sidebarRootRows(models()) {} } }
    }

    @Test @Config(qualifiers = "w280dp-h2600dp-xhdpi")
    fun tabletListPane() = Capture.both("p7-tablet-list-pane") { stacked(SidebarChoice.Card("t-ov-310"))() }

    @Test @Config(qualifiers = "w280dp-h2600dp-xhdpi")
    fun tabletListPaneTicket() = Capture.both("p7-tablet-list-pane-ticket-chosen") { stacked(SidebarChoice.Ticket(plan.tickets.first { it.key == "ov-487" }.task))() }
}
