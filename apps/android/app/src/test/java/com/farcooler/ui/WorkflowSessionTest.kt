package com.farcooler.ui

import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test
import java.io.File

class WorkflowSessionTest {
    @Test fun retainedBytesSurviveRestorationAndCurrentSessionChange() = runBlocking {
        val root = kotlin.io.path.createTempDirectory("ov930-journal").toFile()
        try {
            val fixture = Json.parseToJsonElement(File("crates/client/src/testdata/workflow-presentation.json").readText()).jsonArray[0]
                .jsonObject.getValue("response").jsonObject.getValue("card").jsonObject
            val view = JsonObject(fixture + mapOf("revision" to Json.parseToJsonElement("18446744073709551614"), "epoch" to Json.parseToJsonElement("18446744073709551615")))
            val workspace = view.getValue("workspace").jsonPrimitive.content
            val run = WorkflowPresentation.decode(view.toString(), workspace).runs.first()
            var account = "account"; var generation = 1; var prepares = 0
            val submits = mutableListOf<Pair<Int, String>>()
            val prepared = buildJsonObject { put("schema", 1); put("workspace", workspace); put("json", "{\"epoch\":18446744073709551615}") }
            val getter: (String) -> WorkflowSessionPort.Current? = {
                val capturedGeneration = generation
                WorkflowSessionPort.Current(account, "actual-host") { method, args ->
                    when (method) {
                        "workflow.snapshot" -> buildJsonObject { put("host", "actual-host"); put("presentation", view) }
                        "workflow.prepare_selection" -> {
                            prepares++
                            assertEquals("18446744073709551614", args.getValue("revision").jsonPrimitive.content)
                            assertEquals("18446744073709551615", args.getValue("epoch").jsonPrimitive.content)
                            assertEquals(run.actions.first().descriptor, args["action"])
                            buildJsonObject { put("kind", "Mutation"); put("prepared", prepared) }
                        }
                        else -> {
                            assertTrue(root.listFiles()!!.single().readText().contains("prepared"))
                            assertEquals(prepared, args.getValue("prepared"))
                            submits.add(capturedGeneration to args.toString())
                            error("discarded reply")
                        }
                    }
                }
            }
            var port = WorkflowSessionPort(root, getter)
            port.read("host", workspace)
            try { port.dispatch("host", workspace, run.item, run.actions.first().descriptor.toString()); fail() } catch (_: IllegalStateException) { }
            val pending = port.pending("host", workspace).single()
            generation = 2
            port.read("host", workspace); assertEquals(1, submits.size)
            try { port.retry("host", workspace, pending.id); fail() } catch (_: IllegalStateException) { }
            port = WorkflowSessionPort(root, getter); generation = 3
            port.read("host", workspace)
            try { port.retry("host", workspace, pending.id); fail() } catch (_: IllegalStateException) { }
            assertEquals(1, prepares); assertEquals(3, submits.size)
            assertEquals(submits[0].second, submits[2].second); assertEquals(2, submits[1].first)
            assertEquals(3, submits[2].first)
            assertEquals(1, port.pending("host", workspace).size)
            try { port.retry("host", workspace, pending.id); fail() } catch (_: IllegalStateException) { }
            assertEquals(4, submits.size); assertEquals(submits[0].second, submits[3].second)
            account = "other"; assertTrue(port.pending("host", workspace).isEmpty())
        } finally { root.deleteRecursively() }
    }
    @Test fun lostPrepareRecoveryAndCorruptionRefuseImplicitMutation() = runBlocking {
        val root = kotlin.io.path.createTempDirectory("ov930-recovery").toFile()
        try {
            val card = Json.parseToJsonElement(File("crates/client/src/testdata/workflow-presentation.json").readText()).jsonArray[0]
                .jsonObject.getValue("response").jsonObject.getValue("card").jsonObject
            val workspace = card.getValue("workspace").jsonPrimitive.content
            val run = WorkflowPresentation.decode(card.toString(), workspace).runs.first()
            var prepares = 0
            val getter: (String) -> WorkflowSessionPort.Current? = {
                WorkflowSessionPort.Current("account", "actual-host") { method, _ ->
                    if (method == "workflow.snapshot") buildJsonObject { put("host", "actual-host"); put("presentation", card) }
                    else { assertEquals("workflow.prepare_selection", method); prepares++; error("prepare answer lost") }
                }
            }
            var port = WorkflowSessionPort(root, getter)
            port.read("host", workspace)
            try { port.dispatch("host", workspace, run.item, run.actions.first().descriptor.toString()); fail() } catch (_: IllegalStateException) { }
            port = WorkflowSessionPort(root, getter); port.read("host", workspace)
            val marker = port.pending("host", workspace).single(); assertTrue(marker.reinspect)
            assertTrue(port.retry("host", workspace, marker.id).contains("No action was submitted")); assertEquals(1, prepares)
            assertTrue(port.pending("host", workspace).isEmpty())
            try { port.dispatch("host", workspace, run.item, run.actions.first().descriptor.toString()); fail() } catch (_: IllegalStateException) { }
            assertEquals(2, prepares)
            val path = root.listFiles()!!.single(); path.writeText(path.readText().replace("preparing", "abandoned"))
            try { port.retry("host", workspace, marker.id); fail("corrupt reset") } catch (_: IllegalStateException) { }
        } finally { root.deleteRecursively() }
    }

}
