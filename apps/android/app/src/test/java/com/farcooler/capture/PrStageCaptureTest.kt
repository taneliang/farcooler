package com.farcooler.capture

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.ui.Modifier
import com.farcooler.model.NeedsYouList
import com.farcooler.model.NeedsYouRow
import com.farcooler.model.Plan
import com.farcooler.model.PlanPage
import com.farcooler.model.PlanReadState
import com.farcooler.model.PrActions
import com.farcooler.model.RunnerNeedsYouItem
import com.farcooler.model.TaskStatus
import com.farcooler.ui.NeedsYouItemRow
import com.farcooler.ui.NeedsYouMay
import com.farcooler.ui.PlanPageBody
import com.farcooler.ui.PlanSwitch
import com.farcooler.ui.ThreadReplyDialog
import com.farcooler.ui.planItems
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Where a lane's pull request stands, and what a phone does about it (ov-317), drawn from the plan the iPhone's UI test
 * shares (`test/fixtures/plan-phones-ov317.json`) and the Needs You items the runner sends
 * (`test/fixtures/needs-you.json`). The composables are the app's own (`planItems`, `PlanPageBody`, `NeedsYouItemRow`,
 * `ThreadReplyDialog`), with nothing built for the picture. Run only under `-Pfarcooler.captures`.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [37], qualifiers = "w411dp-h891dp-xxhdpi")
class PrStageCaptureTest {
    private val json = Json { ignoreUnknownKeys = true }
    private val plan = Plan.decode(json.parseToJsonElement(repositoryFile("test/fixtures/plan-phones-ov317.json")).jsonObject["plan"]!!.jsonObject)
    private val statuses = mapOf<String, TaskStatus>()

    private fun lane(name: String) = plan.lanes.first { it.name == name }

    @Test fun rows() = Capture.both("pr-stage-rows") {
        LazyColumn(Modifier.fillMaxSize()) {
            item { PlanSwitch(showsPlan = true, onChange = {}) }
            planItems(PlanReadState.Loaded(plan), statuses, onOpen = {}, onRetry = {})
        }
    }

    @Test fun draftLane() = Capture.both("pr-stage-draft-lane") {
        PlanPageBody(plan, PlanPage.Lane(lane("lane-2").id), null, rows = emptyMap(), onOpenTask = {}, onOpenPage = {}, markReady = { _, _ -> null })
    }

    @Test fun attentionLane() = Capture.both("pr-stage-attention-lane") {
        PlanPageBody(plan, PlanPage.Lane(lane("lane-8").id), null, rows = emptyMap(), onOpenTask = {}, onOpenPage = {}, markReady = { _, _ -> null })
    }

    @Test fun longLogin() = Capture.both("pr-stage-long-login") {
        PlanPageBody(plan, PlanPage.Lane(lane("lane-5").id), null, rows = emptyMap(), onOpenTask = {}, onOpenPage = {}, markReady = { _, _ -> null })
    }

    @Test fun needsYou() = Capture.both("pr-stage-needs-you") {
        val root = json.parseToJsonElement(repositoryFile("test/fixtures/needs-you.json")).jsonObject
        val items = root.getValue("runners").jsonArray[0].jsonObject.getValue("needs_you").let {
            json.decodeFromJsonElement(NeedsYouList.serializer(), it).items
        }.filter { it.kind == "pr_thread" || it.kind == "pr_review" }
        Column {
            for (item in items) {
                NeedsYouItemRow(
                    NeedsYouRow(RunnerNeedsYouItem("studio", item), place = item.workspaceName.ifEmpty { "Pull request" }, runner = null),
                    connection = null, onOpen = {}, may = NeedsYouMay(answer = true, reply = true),
                )
            }
        }
    }

    @Test fun replyDialog() = Capture.bothWithDialog("pr-stage-reply") {
        ThreadReplyDialog(
            question = "alice asked, says the agent: “Why not retry on 5xx?”",
            preview = { Result.success(PrActions.ReplyPreview("acme/billing", 213, "e-liang", inThread = true, noteId = "n", digest = "d")) },
            send = { _, _ -> null },
            onDismiss = {},
        )
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
