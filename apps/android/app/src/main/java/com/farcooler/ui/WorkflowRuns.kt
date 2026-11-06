package com.farcooler.ui

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.CancellationException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

// Schema 2 display data only. Preserve the WHOLE action JSON, including guard.
// No semantic IDs are mapped to domain rules or native action enums.
data class WorkflowAction(val descriptor: JsonObject) {
    val label: String get() = descriptor.getValue("label").jsonPrimitive.content
}
data class WorkflowRun(val value: JsonObject) {
    fun text(key: String): String? = value[key]?.takeUnless { it == JsonNull }?.jsonPrimitive?.content
    val item: String get() = requireNotNull(text("item"))
    val actions: List<WorkflowAction> get() = value.getValue("actions").let { it as JsonArray }.map { WorkflowAction(it.jsonObject) }
}
data class WorkflowPresentation(val value: JsonObject) {
    val runs: List<WorkflowRun> get() = (value.getValue("runs") as JsonArray).map { WorkflowRun(it.jsonObject) }
    companion object {
        fun decode(json: String, workspace: String): WorkflowPresentation {
            val value = Json.parseToJsonElement(json).jsonObject
            require(value.getValue("schema").jsonPrimitive.content == "2")
            require(value.getValue("workspace").jsonPrimitive.content == workspace)
            return WorkflowPresentation(value)
        }
    }
}
interface WorkflowPort {
    suspend fun read(host: String, workspace: String): String
    // Current authenticated Rust Session re-reads State/auth and validates the
    // opaque descriptor; return inspection identity/result text for a text slot.
    suspend fun dispatch(host: String, workspace: String, item: String, actionJson: String): String
}
data class WorkflowView(
    val presentation: WorkflowPresentation? = null, val ready: Boolean = false,
    val busy: Boolean = false, val notice: String = "", val inspection: String = "",
)
// Main-thread display controller. Owns no engine, event ACK or durable receipt.
class WorkflowRunsModel(val host: String, val workspace: String) {
    private val mutable = MutableStateFlow(WorkflowView())
    val view = mutable.asStateFlow()
    private var generation = 0L
    fun clearInspection() { mutable.value = mutable.value.copy(inspection = "") }
    fun disconnect() {
        generation++
        mutable.value = mutable.value.copy(ready = false, busy = false, inspection = "", notice = "Reconnect to refresh workflow runs.")
    }
    suspend fun refresh(port: WorkflowPort?) {
        if (mutable.value.busy) return
        val ticket = ++generation
        mutable.value = mutable.value.copy(ready = false)
        if (port == null) {
            mutable.value = mutable.value.copy(notice = "Workflow runs are not connected on this runner yet.")
            return
        }
        try {
            val presentation = WorkflowPresentation.decode(port.read(host, workspace), workspace)
            if (ticket == generation) mutable.value = mutable.value.copy(presentation = presentation, ready = true, notice = "")
        } catch (cancel: CancellationException) { throw cancel }
        catch (_: Exception) {
            if (ticket == generation) mutable.value = mutable.value.copy(notice = "Couldn’t refresh workflow runs. Reconnect or try Refresh.")
        }
    }
    suspend fun perform(item: String, action: WorkflowAction, port: WorkflowPort?) {
        val current = mutable.value
        if (port == null || !current.ready || current.busy ||
            current.presentation?.runs?.firstOrNull { it.item == item }?.actions?.contains(action) != true) return
        val ticket = generation
        mutable.value = current.copy(busy = true, inspection = "")
        var failed = false
        try {
            val result = port.dispatch(host, workspace, item, action.descriptor.toString())
            if (ticket == generation) mutable.value = mutable.value.copy(inspection = result)
        } catch (cancel: CancellationException) {
            disconnect(); throw cancel
        } catch (_: Exception) {
            failed = true
        }
        if (ticket == generation) {
            mutable.value = mutable.value.copy(busy = false, ready = false)
            refresh(port)
            if (failed) mutable.value = mutable.value.copy(notice = "The action wasn’t confirmed. Check the refreshed run before trying again.")
        }
    }
}
