package com.farcooler.model

import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test

class LiteralOperationTest {
    private fun metadata() = Json.parseToJsonElement("""{"version":1,"provenance":"literal_intent","kind":"task_note_decision","target":"ov-893","preview":"Exact  spacing; λ","preview_clipped":false,"lifecycle":"unconfirmed","input_clipped":false,"output_available":false,"output_clipped":false}""").jsonObject
    private fun tool(operation: JsonElement?): AgentRow.Tool {
        val payload = buildJsonObject {
            put("name", "Bash"); put("summary", "original generic summary"); put("status", "Done"); put("input", "original input excerpt"); put("diff", JsonArray(emptyList()))
            operation?.let { put("operation", it) }
        }
        val row = buildJsonObject { put("id", "tool:call"); put("ord", 0); put("rev", 1); put("kind", buildJsonObject { put("Tool", payload) }) }
        return (AgentRowJson.row(row)!!.kind as AgentRow.Kind.OfTool).tool
    }
    @Test fun wire_preserves_exact_preview_original_input_and_uncertain_presentation() {
        val value = tool(metadata())
        assertEquals("Bash", value.name); assertEquals("original input excerpt", value.input)
        assertEquals("Exact  spacing; λ", value.operation?.preview)
        assertEquals("Far Cooler", value.presentationName)
        assertTrue(value.presentationAccessibility.contains("recognized command"))
        assertEquals(AgentRow.Status.Ended("Outcome unconfirmed"), value.presentationStatus)
        assertTrue(value.opens)
    }
    @Test fun malformed_optional_metadata_and_old_rows_remain_generic() {
        val variants = mutableListOf<JsonElement>(JsonPrimitive("forged"), JsonArray(emptyList()))
        for ((key, bad) in listOf("version" to JsonPrimitive(2), "version" to JsonPrimitive("1"), "provenance" to JsonPrimitive("authenticated_receipt"), "lifecycle" to JsonPrimitive("delivered"), "preview" to JsonPrimitive("x".repeat(281)), "preview_clipped" to JsonPrimitive("false"), "target" to JsonPrimitive("ov-893\n"))) {
            variants.add(JsonObject(metadata() + (key to bad)))
        }
        variants.add(JsonObject(metadata() + ("unknown" to JsonPrimitive("x".repeat(1024)))))
        for (variant in variants) {
            val value = tool(variant)
            assertNull(value.operation); assertEquals("Bash", value.name); assertEquals("original input excerpt", value.input)
        }
        assertNull(tool(null).operation)
    }
    @Test fun mixed_cached_model_groups_keep_provenance_without_success_or_body_rewriting() {
        val semantic = tool(metadata()); val generic = tool(null)
        // Android retains decoded row models in memory, not a separate Codable/disk shape.
        val retained = listOf(generic, semantic).toList()
        assertEquals("Recognized commands · Outcome unconfirmed", AgentConversation.recognizedGroupLabel(retained))
        assertEquals(AgentRow.Status.Ended("Outcome unconfirmed"), AgentConversation.groupStatus(retained))
        assertFalse(AgentConversation.groupPurpose(retained).contains("Exact  spacing"))
        assertEquals("Add decision · ov-893", semantic.presentationSummary)
        assertNull(AgentConversation.recognizedGroupLabel(listOf(generic)))
    }
    @Test fun encoded_limit_and_direction_controls_are_exact_and_fail_closed() {
        val controls = tool(JsonObject(metadata() + ("preview" to JsonPrimitive("λ\u202EExact"))))
        assertTrue(controls.operation!!.containsControls); assertEquals("λ\u202EExact", controls.operation!!.preview)
        assertNull(tool(JsonObject(metadata() + ("preview" to JsonPrimitive("🐱".repeat(280))))).operation)
    }
}
