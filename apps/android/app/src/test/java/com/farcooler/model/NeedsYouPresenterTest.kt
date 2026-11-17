package com.farcooler.model

import kotlinx.serialization.json.Json
import org.junit.After
import org.junit.Assert.*
import org.junit.Test

class NeedsYouPresenterTest {
    @After fun reset() { NeedsYouPresenter.reset() }

    @Test fun `duplicate ids leave the usable row visible`() {
        NeedsYouPresenter.bridge = { request ->
            if (request.windowed("\"id\":\"ask\"".length).count { it == "\"id\":\"ask\"" } > 1)
                """{"schema":1,"error":"duplicate_item"}"""
            else """{"schema":1,"needs_you":{"schema":1,"rows":[],"yours":[],"others":[],"split":false,"counted_total":0,"runner_counts":[],"workspace_counts":[],"unclaimed_counts":[]}}"""
        }
        val data = Json.parseToJsonElement("""{"items":[{"id":"ask","kind":"ask","rank":1},{"id":"ask","kind":"ask","rank":1}]}""")
        assertEquals(1, NeedsYouPresenter.read(data, "host")?.items?.size)
    }

    @Test fun `unavailable core leaves composition safe and has no reading`() {
        NeedsYouPresenter.bridge = { null }
        assertNull(NeedsYouPresenter.read(Json.parseToJsonElement("{\"items\":[]}"), "host"))
        val runners = listOf(NeedsYouRunner("host", "Runner", null))
        val rows = NeedsYou.rows(runners)
        assertTrue(rows.isEmpty())
        assertEquals(emptyList<NeedsYouRow>() to emptyList<NeedsYouRow>(), NeedsYou.grouped(rows))
        assertFalse(NeedsYou.nothingNeedsYou(runners, rows))
        assertEquals(0, NeedsYouPresenter.total(emptyList()))
        val item = NeedsYouItem("ask", kind = "ask", rank = 1)
        assertFalse(item.isCounted)
        assertFalse(item.isOthers)
    }

    @Test fun `invalid rank fails at the read boundary and never crashes composition`() {
        for (rank in listOf("-1", "4294967296")) {
            val data = Json.parseToJsonElement("{\"items\":[{\"id\":\"ask\",\"kind\":\"ask\",\"rank\":$rank}]}")
            assertNull(NeedsYouPresenter.read(data, "host"))
            val invalid = NeedsYouItem("ask", kind = "ask", rank = rank.toLong())
            assertNull(NeedsYouPresenter.tryView(mapOf("host" to listOf(invalid))))
            assertTrue(NeedsYou.rows(listOf(NeedsYouRunner("host", "Runner", RunnerNeedsYou(listOf(invalid))))).isEmpty())
        }
        assertNotNull(NeedsYouPresenter.read(Json.parseToJsonElement("{\"items\":[{\"id\":\"ask\",\"kind\":\"ask\",\"rank\":1}]}"), "host"))
    }

    @Test fun `bridge failures reject a populated reading and leave composition safe`() {
        val failures: List<(String) -> String?> = listOf(
            { null },
            { throw UnsatisfiedLinkError("native core unavailable") },
            { "{\"schema\":1,\"error\":\"invalid\"}" },
            { "not json" },
            { "{\"schema\":1,\"needs_you\":{}}" },
        )
        val data = Json.parseToJsonElement("{\"items\":[{\"id\":\"ask\",\"kind\":\"ask\",\"rank\":1}]}")
        val item = NeedsYouItem("ask", kind = "ask", rank = 1)
        val runners = listOf(NeedsYouRunner("host", "Runner", RunnerNeedsYou(listOf(item))))
        val rows = NeedsYou.rows(runners)
        assertEquals(1, rows.size)
        for (failure in failures) {
            NeedsYouPresenter.bridge = failure
            assertNull(NeedsYouPresenter.read(data, "host"))
            assertNull(NeedsYouPresenter.tryView(mapOf("host" to listOf(item))))
            assertTrue(NeedsYou.rows(runners).isEmpty())
            assertEquals(emptyList<NeedsYouRow>() to emptyList<NeedsYouRow>(), NeedsYou.grouped(rows))
            assertEquals(0, NeedsYouPresenter.total(listOf(RunnerNeedsYouItem("host", item))))
            assertFalse(item.isCounted)
            assertFalse(item.isOthers)
        }
    }
}
