package com.farcooler.ui

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.CancellationException
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject

// Schema 2 display data only. The runs are the Rust core's presentation, generated as `core/wire/WireTypes.kt` (ov-1035);
// these are the app's names for them. An action goes back to the core exactly as it came, guard included.
typealias WorkflowAction = com.farcooler.core.wire.Action
typealias WorkflowRun = com.farcooler.core.wire.RunCard
typealias WorkflowPresentation = com.farcooler.core.wire.Presentation

private val wire = Json { ignoreUnknownKeys = true; explicitNulls = false }

/** The action as the core sent it, which is what it validates. */
fun WorkflowAction.json(): JsonObject = wire.encodeToJsonElement(WorkflowAction.serializer(), this).jsonObject

object WorkflowPresentations {
    fun decode(json: String, workspace: String): WorkflowPresentation {
        val value = wire.decodeFromString(WorkflowPresentation.serializer(), json)
        require(value.schema == 2u)
        require(value.workspace == workspace)
        return value
    }
}
data class RetainedWorkflow(val id: String, val item: String, val run: String, val label: String, val reinspect: Boolean = false)
interface WorkflowPort {
    suspend fun pending(host: String, workspace: String): List<RetainedWorkflow> = emptyList()
    suspend fun retry(host: String, workspace: String, id: String): String = error("No retained action")
    suspend fun read(host: String, workspace: String): String
    // Current authenticated Rust Session re-reads State/auth and validates the
    // opaque descriptor; return inspection identity/result text for a text slot.
    suspend fun dispatch(host: String, workspace: String, item: String, actionJson: String): String
}
data class WorkflowView(
    val retained: List<RetainedWorkflow> = emptyList(),
    val presentation: WorkflowPresentation? = null, val ready: Boolean = false,
    val busy: Boolean = false, val notice: String = "", val inspection: String = "",
)
// Main-thread display controller. Owns no engine, event ACK or durable receipt.
class WorkflowRunsModel(val host: String, val workspace: String) {
    private val mutable = MutableStateFlow(WorkflowView())
    val view = mutable.asStateFlow()
    private var generation = 0L
    private var selection = 0L
    fun clearInspection() { selection++; mutable.value = mutable.value.copy(inspection = "") }
    fun disconnect() {
        generation++
        mutable.value = mutable.value.copy(ready = false, busy = false, inspection = "", notice = "Reconnect to refresh workflow runs.")
    }
    // Recheck this completion ticket after the caller resumes from suspension.
    suspend fun refresh(port: WorkflowPort?): Long? {
        if (mutable.value.busy) return null
        val ticket = ++generation
        mutable.value = mutable.value.copy(ready = false)
        if (port == null) {
            mutable.value = mutable.value.copy(notice = "Workflow runs are not connected on this runner yet.")
            return null
        }
        try {
            val presentation = WorkflowPresentations.decode(port.read(host, workspace), workspace)
            val retained = port.pending(host, workspace)
            if (ticket == generation) {
                if (mutable.value.presentation?.runs?.any { old ->
                    presentation.runs.none { it.item == old.item && it.run == old.run }
                } == true) clearInspection()
                mutable.value = mutable.value.copy(presentation = presentation, retained = retained, ready = true, notice = "")
            }
        } catch (cancel: CancellationException) { throw cancel }
        catch (_: Exception) {
            if (ticket == generation) mutable.value = mutable.value.copy(notice = "Couldn’t refresh workflow runs. Reconnect, then refresh again.")
        }
        return if (ticket == generation) ticket else null
    }
    suspend fun perform(item: String, action: WorkflowAction, port: WorkflowPort?, reason: String? = null) {
        val current = mutable.value
        if (port == null || !current.ready || current.busy ||
            current.presentation?.runs?.firstOrNull { it.item == item }?.actions?.contains(action) != true) return
        val ticket = generation
        val display = selection
        val run = current.presentation?.runs?.firstOrNull { it.item == item }?.run
        mutable.value = current.copy(busy = true, inspection = "")
        var failed = false
        try {
            val submitted = if (reason == null) action.json() else kotlinx.serialization.json.buildJsonObject {
                put("action", action.json()); put("reason", kotlinx.serialization.json.JsonPrimitive(reason))
            }
            val result = port.dispatch(host, workspace, item, submitted.toString())
            if (ticket == generation && display == selection &&
                mutable.value.presentation?.runs?.any { it.item == item && it.run == run } == true)
                mutable.value = mutable.value.copy(inspection = result)
        } catch (cancel: CancellationException) {
            disconnect(); throw cancel
        } catch (_: Exception) {
            failed = true
        }
        if (ticket == generation) {
            mutable.value = mutable.value.copy(busy = false, ready = false)
            val currentRefresh = refresh(port)
            if (failed && currentRefresh != null && currentRefresh == generation) mutable.value = mutable.value.copy(notice = "The action wasn’t confirmed. Check the refreshed run before trying again.")
        }
    }
    suspend fun retry(entry: RetainedWorkflow, port: WorkflowPort?) {
        if (port == null || !mutable.value.ready || mutable.value.busy) return
        val ticket = generation; val display = selection
        var failed = false
        mutable.value = mutable.value.copy(busy = true, inspection = "")
        try {
            val result = port.retry(host, workspace, entry.id)
            if (ticket == generation && display == selection && mutable.value.presentation?.runs?.any {
                it.item == entry.item && it.run == entry.run } == true)
                mutable.value = mutable.value.copy(inspection = result)
        } catch (cancel: CancellationException) { disconnect(); throw cancel }
        catch (_: Exception) { failed = true }
        if (ticket == generation) {
            mutable.value = mutable.value.copy(busy = false, ready = false)
            val currentRefresh = refresh(port)
            if (failed && currentRefresh != null && currentRefresh == generation) mutable.value = mutable.value.copy(notice = "The action wasn’t confirmed. Check the refreshed run before trying again.")
        }
    }

}
