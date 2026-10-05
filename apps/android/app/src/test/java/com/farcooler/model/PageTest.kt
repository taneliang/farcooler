package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Orchestrator pages on Android (ov-285), against real bytes: `page.json` is the
 * Rust client's `page_json` output, held there by the client crate's test and
 * the CLI's, and `pages-seeded.json` is a seeded daemon's own `plan --json`,
 * `page list --json` and `task list --json`. Each assertion names a value the
 * renderer draws or TalkBack says, spelled out here, never read back off the
 * fixture. AgentKit's `PageModelTests` and `PageLiveTests` hold the iPhone's
 * reader to the same words.
 */
class PageTest {
    /** The count reference "open" is "Not done" (Android's sentence case), as the Mac's tree filter and the iPhone say "Not Done" (ov-330, R-13). */
    @Test
    fun `an open card count is called Not done, never Open`() {
        assertEquals("Not done", PageWorld.statusName("open"))
    }

    private val seeded = Json.parseToJsonElement(repositoryFile("test/fixtures/pages-seeded.json")).jsonObject
    private val plan = Plan.decode(seeded["plan"]!!.jsonObject)
    private val pages = BoardPage.list(seeded["pages"]!!.jsonObject)
    private val tasks = seeded["tasks"]!!.jsonArray.map {
        val t = it.jsonObject
        TaskRow(
            id = t["id"]!!.jsonPrimitive.content, key = t["key"]!!.jsonPrimitive.content, title = t["title"]!!.jsonPrimitive.content,
            status = TaskStatus.parse(t["status"]!!.jsonPrimitive.content)!!, statusSince = t["status_since"]!!.jsonPrimitive.long,
        )
    }
    private val world = PageWorld(tasks, plan, pages, mapOf("integ-10" to "wt-1"), setOf(PageWorld.terminalKey("wt-1", "build")), nowMs = pages[0].updatedAtMs + 12 * 60_000)

    private fun page(slot: String) = pages.first { it.slot == slot }

    @Test
    fun `the Rust client's page decodes whole`() {
        val page = BoardPage.decode(Json.parseToJsonElement(repositoryFile("test/fixtures/page.json")).jsonObject)
        assertEquals("train", page.slot)
        assertEquals("Train integ-10", page.title)
        assertEquals("In review · 3 of 4 lanes green", page.summary)
        assertEquals("00000000-0000-0000-0000-000000001001", page.themeAnchor)
        assertEquals(3L, page.revision)
        assertEquals("manager", page.actor)
        assertEquals(1_791_151_320_000L, page.updatedAtMs)
        val doc = page.doc!!
        assertEquals(120, doc.staleAfterMin)
        assertEquals(10, doc.blocks.size)
    }

    @Test
    fun `the train mockup decodes to what the design draws`() {
        val blocks = page("train").doc!!.blocks
        assertEquals(PageBlock.Text("Phones' Plan view and the LFS record, one build, one review."), blocks[0])
        assertEquals(PageStat("Fixing", "1", tone = PageTone.ATTENTION), (blocks[1] as PageBlock.Stats).items[2])
        assertEquals(PageStat("Build", "41 min", "one Mac slot"), (blocks[1] as PageBlock.Stats).items[3])
        assertEquals(listOf("done", "done", "active", "todo"), (blocks[2] as PageBlock.Steps).steps.map { it.state.wire })
        assertEquals(PageBlock.Heading("Lanes"), blocks[3])
        val table = blocks[4] as PageBlock.Table
        assertEquals(listOf("Lane", "Cards", "Gate", "State"), table.columns.map { it.title })
        assertTrue(table.columns[2].grow)
        assertEquals(PageCell(ref = PageRef(PageTarget.Lane("ov-274-phones")), show = PageShow.STATE), table.rows[0][3])
        val waiting = (blocks[6] as PageBlock.ListBlock).items
        assertEquals(PageRef(PageTarget.Ask("ov-274")), waiting[0].ref)
        assertEquals(PageState.WAITING, waiting[0].state)
        val timeline = blocks[7] as PageBlock.Timeline
        // The runner stores RFC 3339 as milliseconds: 15:02 at -07:00.
        assertEquals(1_791_151_320_000L, timeline.entries[0].at)
        assertEquals(PageBlock.Progress("Cards closed", 2, 4), blocks[8])
        assertEquals(PageTarget.Terminal("integ-10", "build"), (blocks[9] as PageBlock.Links).refs[0].target)
    }

    @Test
    fun `references draw live from the board and the plan`() {
        assertEquals(
            PageResolved("ov-274", "Needs decision", destination = PageDestination.Task(tasks.first { it.key == "ov-274" }.id),
                spoken = "ov-274, The Plan view on the phones, Needs decision"),
            world.resolve(PageRef(PageTarget.Task("OV-274"))),
        )
        val lane = plan.lanes.first { it.name == "ov-274-phones" }
        assertEquals(PageResolved("ov-274-phones", "In review · in integ-10", destination = PageDestination.Lane(lane.id)), world.resolve(PageRef(PageTarget.Lane("ov-274-phones"))))
        val theme = world.resolve(PageRef(PageTarget.Theme("Visual language"), label = "Visual"))
        assertEquals("Visual", theme.name)
        assertEquals("0 of 4 done", theme.status)
        assertEquals(PageResolved("Spend", destination = PageDestination.Page("spend")), world.resolve(PageRef(PageTarget.Page("spend"))))
        assertEquals(PageDestination.Terminal("wt-1", "build"), world.resolve(PageRef(PageTarget.Terminal("integ-10", "build"))).destination)
        assertEquals("In review · in integ-10", world.cellText(PageCell(ref = PageRef(PageTarget.Lane("ov-274-phones")), show = PageShow.STATE)))
        assertEquals("Not reported", world.cellText(PageCell(ref = PageRef(PageTarget.Lane("mac-ux")), show = PageShow.SPEND)))
    }

    @Test
    fun `a question still waiting reads Needs you in amber and goes to Needs You, an answered one opens its task`() {
        val open = world.resolve(PageRef(PageTarget.Ask("ov-274")))
        assertEquals("Needs you", open.status)
        assertEquals(PageTone.ATTENTION, open.statusTone)
        assertTrue(open.destination is PageDestination.Ask)
        val answered = world.copy(tasks = tasks.map { if (it.key == "ov-274") it.copy(status = TaskStatus.IN_PROGRESS) else it })
            .resolve(PageRef(PageTarget.Ask("ov-274")))
        assertEquals("Answered", answered.status)
        assertEquals(PageTone.NEUTRAL, answered.statusTone)
        assertTrue(answered.destination is PageDestination.Task)
    }

    @Test
    fun `a reference to something gone draws its label or name as plain text`() {
        for (ref in listOf(
            PageRef(PageTarget.Lane("dropped-lane")), PageRef(PageTarget.Task("zz-9")), PageRef(PageTarget.Page("gone")),
            PageRef(PageTarget.Worktree("nowhere")), PageRef(PageTarget.Terminal("integ-10", "shell")), PageRef(PageTarget.Unknown),
        )) {
            val r = world.resolve(ref)
            assertNull("$ref opens", r.destination)
            assertEquals(ref.target.rawName, r.name)
        }
        assertEquals("Build terminal", world.resolve(PageRef(PageTarget.Worktree("gone"), label = "Build terminal")).name)
        // Without the plan layer, lanes and themes are words; cards still draw.
        val noPlan = world.copy(plan = null)
        assertNull(noPlan.resolve(PageRef(PageTarget.Lane("ov-274-phones"))).destination)
        assertTrue(noPlan.resolve(PageRef(PageTarget.Task("ov-274"))).destination != null)
    }

    @Test
    fun `a web link opens only over https with a plain domain, and says where it goes`() {
        val ci = world.resolve(PageRef(PageTarget.Url("https://github.com/example/overnight/actions/runs/812"), label = "CI"))
        assertEquals(PageResolved("CI", "github.com", destination = PageDestination.Url("https://github.com/example/overnight/actions/runs/812"), spoken = "CI, link to github.com"), ci)
        assertEquals("Link to github.com", world.resolve(PageRef(PageTarget.Url("https://github.com/x"))).spoken)
        for (raw in listOf(
            "http://github.com/", "javascript:alert(1)", "https://user@github.com/", "https://user:pw@github.com/",
            "file:///etc/passwd", "https://gіthub.com/", "/relative", "https://",
        )) {
            assertNull(raw, PageLinks.https(raw))
            assertNull(raw, world.resolve(PageRef(PageTarget.Url(raw))).destination)
        }
        assertEquals("xn--gthub-n4a.com", PageLinks.domain("https://xn--gthub-n4a.com/"))
    }

    @Test
    fun `a text link keeps https only, with its domain after its label`() {
        val runs = PageMarkdown.inline("See [the run](https://github.com/x/actions/1), [github.com](https://github.com/y) and [bad](http://evil.example/).")
        assertEquals(listOf("See ", "the run", " github.com", ", ", "github.com", " and ", "bad", "."), runs.map { it.span.text })
        assertEquals(listOf(false, false, true, false, false, false, false, false), runs.map { it.domain })
        assertEquals("https://github.com/x/actions/1", runs[1].span.link)
        assertNull("an http link opens", runs[6].span.link)
        assertEquals(
            listOf(PageMarkdown.Piece.Prose("One"), PageMarkdown.Piece.Item("•", "a", 0), PageMarkdown.Piece.Item("•", "b", 1), PageMarkdown.Piece.Plain("Head")),
            PageMarkdown.pieces("One\n\n- a\n  - b\n\n# Head"),
        )
    }

    @Test
    fun `a wide table stacks on a phone and TalkBack reads each row with its titles`() {
        assertTrue(PageLayout.stacks(4, 411))
        assertFalse(PageLayout.stacks(3, 411))
        assertFalse(PageLayout.stacks(4, 600))
        assertFalse("a width not measured is wide", PageLayout.stacks(4, null))
        assertTrue(PageLayout.stepsDown(411))
        val table = page("train").doc!!.blocks[4] as PageBlock.Table
        assertEquals(
            "Lane, ov-274-phones. Cards, ov-274. Gate, iOS UI class, Android captures. State, In review · in integ-10.",
            PageLayout.spokenRow(table.columns, table.rows[0], world),
        )
        assertEquals(listOf("Open ov-274-phones", "Open ov-274"), world.actions(table.rows[0]).map { it.first })
    }

    @Test
    fun `a table cell's link says its domain to TalkBack, in the row and in its action`() {
        val cell = PageCell(text = "Click here", ref = PageRef(PageTarget.Url("https://evil.example/x")))
        val columns = listOf(PageColumn("Lane"), PageColumn("Run"))
        assertEquals("Lane, mac-ux. Run, Click here, link to evil.example.", PageLayout.spokenRow(columns, listOf(PageCell("mac-ux"), cell), PageWorld()))
        assertEquals(listOf("Open Click here, link to evil.example"), PageWorld().actions(listOf(cell)).map { it.first })
        assertEquals(listOf("Open github.com"), PageWorld().actions(listOf(PageCell(ref = PageRef(PageTarget.Url("https://github.com/x"))))).map { it.first })
    }

    @Test
    fun `TalkBack says each state's word, never the glyph alone`() {
        val risks = (page("risks").doc!!.blocks[1] as PageBlock.ListBlock).items
        assertEquals(
            "Blocked, Sidebar tint vs. terminal theme is undecided, Holds ov-222's last pass, ov-222, Sidebar tint follows the terminal theme, Needs you",
            PageSpeech.item(risks[0], risks[0].ref?.let(world::resolve)),
        )
        assertEquals("Done, Increase Contrast outline on attention cards", PageSpeech.item(risks[3], null))
        assertEquals("Review, Active", PageSpeech.step(PageStep("Review", PageState.ACTIVE)))
        assertEquals("Land, To do", PageSpeech.step(PageStep("Land", PageState.TODO)))
        assertEquals("2 of 4", PageSpeech.progress(PageBlock.Progress("Cards closed", 2, 4)))
        assertEquals("Page, Train integ-10, In review · 3 of 4 lanes green, Updated 12 min ago", PageSpeech.row(page("train"), world.nowMs))
    }

    @Test
    fun `updated reads its age, and past the page's own limit, how long it's been`() {
        val spend = page("spend")
        assertEquals("Updated 40 min ago", PageWords.updated(spend, spend.updatedAtMs + 40 * 60_000))
        assertEquals("Not updated for 5 hours", PageWords.updated(spend, spend.updatedAtMs + 5 * 3_600_000))
    }

    @Test
    fun `a timeline is newest first unless the orchestrator gave its order`() {
        val entries = listOf(PageEntry(1, "a"), PageEntry(3, "c"), PageEntry(2, "b"), PageEntry(3, "c2"))
        assertEquals(listOf("c", "c2", "b", "a"), PageLayout.ordered(entries, given = false).map { it.text })
        assertEquals(listOf("a", "c", "b", "c2"), PageLayout.ordered(entries, given = true).map { it.text })
    }

    @Test
    fun `an anchored page draws inside its live theme, and falls back to the Pages section when it's gone`() {
        val theme = plan.themes.first { it.name == "Visual language" }
        assertEquals(listOf("train", "spend"), PageShelf.listed(pages, plan).map { it.slot })
        assertEquals(listOf("risks"), PageShelf.anchored(pages, theme.id, plan).map { it.slot })
        val dropped = plan.copy(themes = plan.themes.map { it.copy(state = "dropped") })
        assertEquals(listOf("train", "spend", "risks"), PageShelf.listed(pages, dropped).map { it.slot })
        assertEquals(emptyList<BoardPage>(), PageShelf.anchored(pages, theme.id, dropped))
        // Under its own header, a first heading repeating the title is left out.
        assertEquals(PageBlock.ListBlock::class, page("risks").doc!!.under("Risks").blocks.first()::class)
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
