package com.farcooler.ui

import java.io.File
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.*
import org.junit.Test

class WorkflowRunsTest {
    private fun cards(): List<String> {
        val root = generateSequence(File(System.getProperty("user.dir"))) { it.parentFile }
            .first { File(it, "crates/client/src/testdata/workflow-presentation.json").exists() }
        val fixtures = Json.parseToJsonElement(File(root, "crates/client/src/testdata/workflow-presentation.json").readText()) as JsonArray
        return fixtures.map { it.jsonObject.getValue("response").jsonObject.getValue("card").toString() }
    }
    @Test fun sharedGoldenAndExactDescriptors() = runBlocking {
        for (json in cards()) {
            val card = WorkflowPresentation.decode(json, "workspace").runs.single()
            val model = WorkflowRunsModel("runner", "workspace")
            var forwarded: String? = null
            val port = object : WorkflowPort {
                override suspend fun read(host: String, workspace: String): String {
                    assertEquals("runner", host); assertEquals("workspace", workspace); return json
                }
                override suspend fun dispatch(host: String, workspace: String, item: String, actionJson: String): String {
                    assertEquals(card.item, item); forwarded = actionJson; return "retained identity"
                }
            }
            model.refresh(port)
            assertTrue(model.view.value.ready)
            assertTrue(card.text("label")!!.isNotEmpty())
            for (action in card.actions) {
                model.perform(card.item, action, port)
                assertEquals(action.descriptor, Json.parseToJsonElement(forwarded!!))
            }
            model.disconnect(); assertFalse(model.view.value.ready)
            forwarded = null
            model.perform(card.item, card.actions.first(), port); assertNull(forwarded)
            model.refresh(port); assertTrue(model.view.value.ready)
        }
    }
    @Test fun disconnectFencesLateReadAndPendingPort() = runBlocking {
        val model = WorkflowRunsModel("runner", "workspace")
        val result = CompletableDeferred<String>()
        val started = CompletableDeferred<Unit>()
        val port = object : WorkflowPort {
            override suspend fun read(host: String, workspace: String): String { started.complete(Unit); return result.await() }
            override suspend fun dispatch(host: String, workspace: String, item: String, actionJson: String): String = error("must not submit")
        }
        val reading = async { model.refresh(port) }; started.await(); model.disconnect()
        result.complete(cards().first()); reading.await()
        assertFalse(model.view.value.ready); assertNull(model.view.value.presentation)
        model.refresh(null); assertFalse(model.view.value.ready); assertTrue(model.view.value.notice.contains("not connected"))
        try { WorkflowPresentation.decode(cards().first(), "other"); fail("wrong workspace") } catch (_: IllegalArgumentException) { }
    }
    @Test fun losslessFenceAndLongSharedCopy() {
        val json = cards().first().replace("\"fence\":1", "\"fence\":18446744073709551615")
        // JSON formatting is compact in cards(); no Double conversion is used.
        val run = WorkflowPresentation.decode(json, "workspace").runs.single()
        assertEquals("18446744073709551615", run.text("fence"))
        val raw = "{\"kind\":\"FutureAction\",\"guard\":\"exact\",\"label\":\"" + "Long shared label ".repeat(100) + "\"}"
        val action = WorkflowAction(Json.parseToJsonElement(raw).jsonObject)
        assertEquals("Long shared label ".repeat(100), action.label)
        assertEquals(Json.parseToJsonElement(raw), action.descriptor)
    }

    @Test fun selectionFencesReplyWithoutStrandingBusyOrRefresh() = runBlocking {
        for (backToOriginal in listOf(false, true)) {
            val json = cards().first()
            val model = WorkflowRunsModel("runner", "workspace")
            val card = WorkflowPresentation.decode(json, "workspace").runs.single()
            val result = CompletableDeferred<String>(); val started = CompletableDeferred<Unit>()
            var reads = 0
            val port = object : WorkflowPort {
                override suspend fun read(host: String, workspace: String): String { reads++; return json }
                override suspend fun dispatch(host: String, workspace: String, item: String, actionJson: String): String {
                    started.complete(Unit); return result.await()
                }
            }
            model.refresh(port)
            val sending = async { model.perform(card.item, card.actions.first(), port) }; started.await()
            model.clearInspection() // All Runs / another selected item
            if (backToOriginal) model.clearInspection() // Return to original before reply
            result.complete("old evidence"); sending.await()
            assertEquals("", model.view.value.inspection); assertFalse(model.view.value.busy)
            assertTrue(model.view.value.ready); assertEquals(2, reads)
        }
    }
    @Test fun replacedRunClearsInspectionOnRefreshAndPostActionRead() = runBlocking {
        for (replaceDuringAction in listOf(false, true)) {
            var json = cards().first()
            val model = WorkflowRunsModel("runner", "workspace")
            val card = WorkflowPresentation.decode(json, "workspace").runs.single()
            val replacement = json.replace("\"run\":\"${card.text("run")}\"", "\"run\":\"replacement\"")
            assertNotEquals(json, replacement)
            val port = object : WorkflowPort {
                override suspend fun read(host: String, workspace: String): String = json
                override suspend fun dispatch(host: String, workspace: String, item: String, actionJson: String): String {
                    if (replaceDuringAction) json = replacement
                    return "old evidence"
                }
            }
            model.refresh(port); model.perform(card.item, card.actions.first(), port)
            if (!replaceDuringAction) { assertEquals("old evidence", model.view.value.inspection); json = replacement; model.refresh(port) }
            assertEquals("", model.view.value.inspection)
        }
    }
    @Test fun failedActionReadCannotOverwriteDisconnectOrReconnect() = runBlocking {
        for (reconnect in listOf(false, true)) {
            val json = cards().first(); val model = WorkflowRunsModel("runner", "workspace")
            val card = WorkflowPresentation.decode(json, "workspace").runs.single()
            val read = CompletableDeferred<String>(); val started = CompletableDeferred<Unit>(); var reads = 0
            val old = object : WorkflowPort {
                override suspend fun read(host: String, workspace: String): String {
                    if (++reads == 1) return json
                    started.complete(Unit); return read.await()
                }
                override suspend fun dispatch(host: String, workspace: String, item: String, actionJson: String): String = error("lost reply")
            }
            model.refresh(old)
            val sending = async { model.perform(card.item, card.actions.first(), old) }; started.await(); model.disconnect()
            if (reconnect) model.refresh(object : WorkflowPort {
                override suspend fun read(host: String, workspace: String): String = json
                override suspend fun dispatch(host: String, workspace: String, item: String, actionJson: String): String = error("must not replay")
            })
            val notice = model.view.value.notice
            read.complete(json); sending.await()
            assertEquals(notice, model.view.value.notice); assertEquals(reconnect, model.view.value.ready)
        }
    }

    @Test fun disconnectFencesDelayedActionReply() = runBlocking {
        for (reconnect in listOf(false, true)) {
            val json = cards().first(); val model = WorkflowRunsModel("runner", "workspace")
            val card = WorkflowPresentation.decode(json, "workspace").runs.single()
            val reply = CompletableDeferred<String>(); val started = CompletableDeferred<Unit>()
            val port = object : WorkflowPort {
                override suspend fun read(host: String, workspace: String): String = json
                override suspend fun dispatch(host: String, workspace: String, item: String, actionJson: String): String {
                    started.complete(Unit); return reply.await()
                }
            }
            model.refresh(port)
            val sending = async { model.perform(card.item, card.actions.first(), port) }; started.await(); model.disconnect()
            if (reconnect) model.refresh(port)
            reply.complete("old evidence"); sending.await()
            assertEquals("", model.view.value.inspection); assertFalse(model.view.value.busy)
            assertEquals(reconnect, model.view.value.ready)
        }
    }

}
