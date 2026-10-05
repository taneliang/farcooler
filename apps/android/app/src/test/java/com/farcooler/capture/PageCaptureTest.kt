package com.farcooler.capture

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.ui.Modifier
import com.farcooler.model.BoardPage
import com.farcooler.model.Fleet
import com.farcooler.model.Plan
import com.farcooler.model.PlanPage
import com.farcooler.model.PlanReadState
import com.farcooler.model.TaskRow
import com.farcooler.model.TaskStatus
import com.farcooler.net.PageListState
import com.farcooler.ui.OrchestratorPage
import com.farcooler.ui.PagesHook
import com.farcooler.ui.PlanPageBody
import com.farcooler.ui.PlanSwitch
import com.farcooler.ui.pageWorld
import com.farcooler.ui.planItems
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Orchestrator pages on Android (ov-285): the design's three mockups, drawn by
 * the app's own composables from a real board. `test/fixtures/pages-seeded.json`
 * is the CLI's own `plan --json`, `page list --json` and `task list --json` for
 * the board `.claude/agent/reports/ov-284/seed-pages.sh` seeds. Twelve minutes
 * after the pages were written, so "Updated" reads as a person would see it.
 * Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class PageCaptureTest {
    private val seeded = Json.parseToJsonElement(repositoryFile("test/fixtures/pages-seeded.json")).jsonObject
    private val plan = Plan.decode(seeded["plan"]!!.jsonObject)
    private val pages = BoardPage.list(seeded["pages"]!!.jsonObject)
    private val now = pages.maxOf { it.updatedAtMs } + 12 * 60_000
    private val rows = seeded["tasks"]!!.jsonArray.map {
        val t = it.jsonObject
        TaskRow(
            id = t["id"]!!.jsonPrimitive.content, key = t["key"]!!.jsonPrimitive.content, title = t["title"]!!.jsonPrimitive.content,
            status = TaskStatus.parse(t["status"]!!.jsonPrimitive.content)!!, statusSince = t["status_since"]!!.jsonPrimitive.long,
        )
    }
    private val world = pageWorld(rows, plan, pages, Fleet(), now)

    private fun page(slot: String) = pages.first { it.slot == slot }

    @Test fun pagesSection() = Capture.both("pages-section") {
        LazyColumn(Modifier.fillMaxSize()) {
            item { PlanSwitch(showsPlan = true, onChange = {}) }
            planItems(
                PlanReadState.Loaded(plan.copy(order = emptyList(), lanes = emptyList())), emptyMap(), onOpen = {}, onRetry = {},
                pages = PagesHook(PageListState.Loaded(pages), now) {},
            )
        }
    }

    @Test fun train() = Capture.both("pages-train") { OrchestratorPage(page("train"), world) {} }

    /** The train page's lower half, from its Waiting On list: a capture is one screen tall. */
    @Test fun trainLower() = Capture.both("pages-train-lower") {
        val train = page("train")
        OrchestratorPage(train.copy(doc = train.doc!!.copy(blocks = train.doc!!.blocks.drop(5))), world) {}
    }

    @Test fun spend() = Capture.both("pages-spend") { OrchestratorPage(page("spend"), world) {} }

    @Test fun risks() = Capture.both("pages-risks") {
        val theme = plan.themes.first { it.name == "Visual language" }
        PlanPageBody(plan, PlanPage.Theme(theme.id), record = null, rows = rows.associateBy { it.id }, onOpenTask = {}, onOpenPage = {}, pages = pages, world = world)
    }

    @Test fun unavailable() = Capture.both("pages-unavailable") {
        LazyColumn(Modifier.fillMaxSize()) {
            planItems(PlanReadState.Loaded(plan.copy(order = emptyList(), lanes = emptyList())), emptyMap(), onOpen = {}, onRetry = {}, pages = PagesHook(PageListState.Unavailable, now) {})
        }
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
