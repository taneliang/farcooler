package com.farcooler.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

// ov-925 attaches its CURRENT authenticated Session adapter at the app root.
val LocalWorkflowPort = compositionLocalOf<WorkflowPort?> { null }

@Composable
fun WorkflowRunsButton(host: String, workspace: String) {
    var showing by rememberSaveable(host, workspace) { mutableStateOf(false) }
    TextButton(onClick = { showing = true }) { Text("Workflow runs") }
    if (showing) Dialog(onDismissRequest = { showing = false }, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        androidx.compose.material3.Surface {
            WorkflowRunsView(host, workspace) { showing = false }
        }
    }
}
@Composable
fun WorkflowRunsView(host: String, workspace: String, close: () -> Unit) {
    val port = LocalWorkflowPort.current
    val model = remember(host, workspace) { WorkflowRunsModel(host, workspace) }
    val view by model.view.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()
    val preferences = LocalContext.current.getSharedPreferences("workflow-runs", 0)
    val key = "run.$host.$workspace"
    var selected by rememberSaveable(host, workspace) { mutableStateOf(preferences.getString(key, null)) }
    fun select(item: String?) { model.clearInspection(); selected = item; preferences.edit().putString(key, item).apply() }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(model, lifecycle, port) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) scope.launch { model.refresh(port) }
            if (event == Lifecycle.Event.ON_PAUSE) model.disconnect()
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer); model.disconnect() }
    }
    LaunchedEffect(model, port) { model.refresh(port) }
    BackHandler(selected != null) { select(null) }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        view.retained.forEach { entry ->
            TextButton(onClick = { select(entry.item); scope.launch { model.retry(entry, port) } }, enabled = view.ready && !view.busy) {
                Text("${if (entry.reinspect) "Reinspect unsubmitted action" else "Retry retained action"} · ${entry.item} · ${entry.label}")
            }
        }
        Text("Workflow runs", style = MaterialTheme.typography.headlineSmall)
        TextButton(onClick = close) { Text("Close") }
        Button(onClick = { scope.launch { model.refresh(port) } }, enabled = !view.busy) { Text("Refresh") }
        if (view.notice.isNotEmpty()) Text(view.notice)
        if (selected != null) TextButton(onClick = { select(null) }) { Text("All runs") }
        val run = view.presentation?.runs?.firstOrNull { it.item == selected }
        if (run != null) {
            WorkflowRunCard(run, view.ready && !view.busy && port != null, performReason = { action, reason ->
                scope.launch { model.perform(run.item, action, port, reason) }
            }) { action -> scope.launch { model.perform(run.item, action, port) } }
        } else {
            view.presentation?.runs?.forEach { card ->
                TextButton(onClick = { select(card.item) }) { Text("${card.text("label")} · ${card.item} · ${card.text("phase_label")}") }
            }
            if (view.ready && view.presentation?.runs?.isEmpty() == true) Text("No workflow runs.")
            if (view.ready && selected != null) Text("This run is no longer available.")
        }
        if (view.inspection.isNotEmpty()) SelectionContainer { Text(view.inspection) }
    }
}
@Composable
fun WorkflowRunCard(run: WorkflowRun, enabled: Boolean, performReason: (WorkflowAction, String) -> Unit = { _, _ -> }, perform: (WorkflowAction) -> Unit) {
    // Finite identity slots. Values remain lossless JsonPrimitive strings,
    // including u64 fence values, and content wraps without ellipsis.
    SelectionContainer {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(requireNotNull(run.text("label")), style = MaterialTheme.typography.titleLarge)
            Text(requireNotNull(run.text("phase_label")))
            listOf("item", "run", "definition_pin", "policy_pin", "candidate", "pending_effect", "iteration", "attempt", "fence", "reworks_remaining").forEach { key ->
                run.text(key)?.let { Text("$key: $it") }
            }
            run.value["latest_evidence"]?.takeUnless { it == JsonNull }?.jsonObject?.forEach { (key, value) -> Text("Evidence $key: ${value.jsonPrimitive.content}") }
            listOf("stage", "gate").forEach { key ->
                run.value[key]?.takeUnless { it == JsonNull }?.jsonObject?.let { value ->
                    listOf("label", "text", "progression_label", "principal").forEach { slot -> value[slot]?.let { Text(it.jsonPrimitive.content) } }
                }
            }
            (run.value["stages"] as JsonArray).forEach { stage ->
                val value = stage.jsonObject
                Text(listOf("id", "label", "progression_label").joinToString(" · ") { value.getValue(it).jsonPrimitive.content })
            }
            (run.value["reasons"] as JsonArray).forEach { reason ->
                Text(reason.jsonObject.getValue("label").jsonPrimitive.content, style = MaterialTheme.typography.titleMedium)
                Text(reason.jsonObject.getValue("text").jsonPrimitive.content)
            }
        }
    }
    (run.value["rework_history"] as? JsonArray)?.forEach { history ->
        Text(history.jsonObject.getValue("principal").jsonPrimitive.content)
        SelectionContainer { Text(history.jsonObject.getValue("reason").jsonPrimitive.content) }
    }
    run.actions.forEach { action ->
        val form = (run.value["forms"] as? JsonArray)?.firstOrNull {
            it.jsonObject.getValue("action") == action.descriptor.getValue("kind")
        }?.jsonObject
        if (form == null) Button(onClick = { perform(action) }, enabled = enabled) { Text(action.label) }
        else {
            var draft by rememberSaveable(run.item, run.text("run"), action.descriptor.getValue("guard").jsonPrimitive.content) { mutableStateOf("") }
            OutlinedTextField(value = draft, onValueChange = { draft = it }, enabled = enabled,
                label = { Text(form.getValue("label").jsonPrimitive.content) })
            Text(form.getValue("help").jsonPrimitive.content)
            Button(onClick = { performReason(action, draft) }, enabled = enabled) { Text(form.getValue("submit_label").jsonPrimitive.content) }
        }
    }
}
