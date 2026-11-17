package com.farcooler.model

import com.farcooler.core.NativeClient
import com.farcooler.core.coreIsAvailable
import com.farcooler.core.wire.NeedsYouAggregateView
import kotlinx.serialization.json.*

/** The existing view-model bridge carries the pure Rust aggregation request. */
object NeedsYouPresenter {
    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }

    private val core: (String) -> String? = { request -> if (coreIsAvailable) NativeClient.nativePlanView(request) else null }
    internal var bridge: (String) -> String? = core
    internal fun reset() { bridge = core }

    // No native library means no reading; let the existing diagnostics remain reachable.
    // This is an unavailable view, never a second implementation of aggregation.
    private val unavailable = NeedsYouAggregateView(1u, emptyList(), emptyList(), emptyList(), false, 0u, emptyList(), emptyList(), emptyList())

    fun view(runners: Map<String, List<NeedsYouItem>>): NeedsYouAggregateView {
        tryView(runners)?.let { return it }
        System.err.println("Needs You presenter rejected the list") // casing ok: name; platform copy: developer diagnostic
        val unique = runners.mapValues { (_, items) -> items.distinctBy { it.id } }
        if (unique != runners) return tryView(unique) ?: unavailable
        return unavailable
    }

    /** Null on an unavailable core or invalid input, just like the other native view-model boundaries. */
    fun tryView(runners: Map<String, List<NeedsYouItem>>): NeedsYouAggregateView? = runCatching {
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
        val answer = bridge(request.toString()) ?: return null
        val result = Json.parseToJsonElement(answer).jsonObject["needs_you"] ?: return null
        json.decodeFromJsonElement<NeedsYouAggregateView>(result)
    }.getOrNull()

    /** Validate with Rust before a read replaces the last good list. */
    fun read(data: JsonElement, runnerId: String): NeedsYouList? = runCatching {
        val list = json.decodeFromJsonElement<NeedsYouList>(data)
        if (tryView(mapOf(runnerId to list.items)) != null) return list
        System.err.println("Needs You presenter rejected the list for $runnerId") // casing ok: name; platform copy: developer diagnostic
        val unique = list.items.distinctBy { it.id }
        if (unique.size == list.items.size || tryView(mapOf(runnerId to unique)) == null) return null
        list.copy(items = unique)
    }.getOrNull()

    fun view(items: List<RunnerNeedsYouItem>): NeedsYouAggregateView =
        view(items.groupBy { it.hostId }.mapValues { (_, rows) -> rows.map { it.item } })

    fun total(items: List<RunnerNeedsYouItem>): Int = view(items).counted_total.toInt()
}
