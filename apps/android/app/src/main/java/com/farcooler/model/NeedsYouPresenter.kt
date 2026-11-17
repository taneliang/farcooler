package com.farcooler.model

import com.farcooler.core.NativeClient
import com.farcooler.core.coreIsAvailable
import com.farcooler.core.wire.NeedsYouAggregateView
import kotlinx.serialization.json.*

/** The existing view-model bridge carries the pure Rust aggregation request. */
object NeedsYouPresenter {
    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }

    fun view(runners: Map<String, List<NeedsYouItem>>): NeedsYouAggregateView {
        val request = buildJsonObject {
            put("schema", 1)
            putJsonArray("needs_you") {
                runners.forEach { (runner, items) ->
                    add(buildJsonObject {
                        put("runner_id", runner)
                        put("items", json.encodeToJsonElement(items))
                    })
                }
            }
        }
        check(coreIsAvailable)
        val answer = requireNotNull(NativeClient.nativePlanView(request.toString()))
        val result = requireNotNull(Json.parseToJsonElement(answer).jsonObject["needs_you"]) { "needs_you_invalid" }
        return json.decodeFromJsonElement(result)
    }

    fun view(items: List<RunnerNeedsYouItem>): NeedsYouAggregateView =
        view(items.groupBy { it.hostId }.mapValues { (_, rows) -> rows.map { it.item } })

    fun total(items: List<RunnerNeedsYouItem>): Int = view(items).counted_total.toInt()
}
