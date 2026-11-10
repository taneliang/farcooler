package com.farcooler.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
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
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
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
    val watcher = LocalAgentWatch.current
    var watching by rememberSaveable(host, workspace) { mutableStateOf<String?>(null) }
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
        val agent = watching
        if (agent != null && watcher != null) Dialog(onDismissRequest = { watching = null }, properties = DialogProperties(usePlatformDefaultWidth = false)) {
            androidx.compose.material3.Surface { AgentWatchView(workspace, agent, watcher) { watching = null } }
        }
        if (run != null) {
            WorkflowRunCard(run, view.ready && !view.busy && port != null,
                onWatch = if (watcher != null && run.pendingEffect != null) ({ watching = run.item }) else null,
                performReason = { action, reason ->
                scope.launch { model.perform(run.item, action, port, reason) }
            }) { action -> scope.launch { model.perform(run.item, action, port) } }
        } else {
            view.presentation?.runs?.forEach { card ->
                TextButton(onClick = { select(card.item) }) { Text("${card.label} · ${card.item} · ${card.phaseLabel}") }
            }
            if (view.ready && view.presentation?.runs?.isEmpty() == true) Text("No workflow runs.")
            if (view.ready && selected != null) Text("This run is no longer available.")
        }
        if (view.inspection.isNotEmpty()) SelectionContainer { Text(view.inspection) }
    }
}
@Composable
fun WorkflowRunCard(run: WorkflowRun, enabled: Boolean, detailsOpen: Boolean = false, onWatch: (() -> Unit)? = null, performReason: (WorkflowAction, String) -> Unit = { _, _ -> }, perform: (WorkflowAction) -> Unit) {
    // Finite identity slots, content wrapping without ellipsis.
    SelectionContainer {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            val clipboard = LocalClipboardManager.current
            Text(run.page.ticket, style = MaterialTheme.typography.titleLarge)
            Text(run.page.outcome, style = MaterialTheme.typography.titleMedium)
            Text(run.page.reason)
            run.page.more.forEach { Text(it) }
            run.actions.forEach { action ->
                val form = run.forms?.firstOrNull { it.action == action.kind }
                if (form == null) Button(onClick = { perform(action) }, enabled = enabled) { Text(action.label) }
                else {
                    var draft by rememberSaveable(run.item, run.run, action.guard) { mutableStateOf("") }
                    OutlinedTextField(value = draft, onValueChange = { draft = it }, enabled = enabled,
                        label = { Text(form.label) })
                    Text(form.help)
                    Button(onClick = { performReason(action, draft) }, enabled = enabled) { Text(form.submitLabel) }
                }
            }
            if (onWatch != null) Button(onClick = onWatch) { Text("Watch agent") }
            run.page.steps.forEach { step -> Text("${step.label} · ${step.state}") }
            var open by rememberSaveable(run.item, run.run) { mutableStateOf(detailsOpen) }
            Text(run.page.detailsLabel, color = MaterialTheme.colorScheme.primary, modifier = Modifier.clickable(role = Role.Button) { open = !open })
            if (open) run.page.details.forEach { row ->
                Text("${row.title}: ${row.value}")
                row.whole?.let { whole -> Text(run.page.copyLabel, color = MaterialTheme.colorScheme.primary, modifier = Modifier.clickable(role = Role.Button) { clipboard.setText(AnnotatedString(whole)) }) }
            }
        }
    }
    run.reworkHistory?.forEach { history ->
        Text(history.principal)
        SelectionContainer { Text(history.reason) }
    }
}
