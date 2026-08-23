package com.farcooler.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.data.Runner
import com.farcooler.model.QuickAgents
import com.farcooler.model.TaskSlug
import com.farcooler.model.TerminalPresets
import com.farcooler.model.Trouble
import com.farcooler.model.Workspace
import com.farcooler.net.Connection
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Add a runner, or correct one that was typed in wrong.
 *
 * One sheet for both, because they are the same four fields and because the
 * second is what makes an unreachable runner survivable: the app opens onto a
 * runner, so a mistyped address is a screen you can never get past and never
 * fix. That made "Remove" the other thing this had to grow.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RunnerEditorSheet(
    existing: Runner?,
    onSave: (Runner) -> Unit,
    onRemove: ((Runner) -> Unit)?,
    onDismiss: () -> Unit,
) {
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    var label by remember { mutableStateOf(existing?.label.orEmpty()) }
    var address by remember { mutableStateOf(existing?.address.orEmpty()) }
    var user by remember { mutableStateOf(existing?.user.orEmpty()) }
    var port by remember { mutableStateOf((existing?.port ?: 22).toString()) }
    var confirmingRemove by remember { mutableStateOf(false) }

    val valid = address.isNotBlank() && user.isNotBlank()

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = state) {
        Column(
            Modifier
                .padding(horizontal = 20.dp)
                .padding(bottom = 20.dp)
                .imePadding()
                .navigationBarsPadding(),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                if (existing == null) "Add a runner" else "Edit runner",
                style = MaterialTheme.typography.headlineSmall,
            )

            // Labelled fields rather than bare placeholders. A placeholder names
            // a field only while it is empty, which is fine for adding and
            // useless for editing: four filled rows reading "Demo host /
            // 10.0.0.4 / me / 22" leave you to work out which is which.
            OutlinedTextField(
                value = label,
                onValueChange = { label = it },
                label = { Text("Name") },
                placeholder = { Text("Optional") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            OutlinedTextField(
                value = address,
                onValueChange = { address = it },
                label = { Text("Address") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                modifier = Modifier.fillMaxWidth(),
            )
            OutlinedTextField(
                value = user,
                onValueChange = { user = it },
                label = { Text("User") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            OutlinedTextField(
                value = port,
                onValueChange = { port = it.filter(Char::isDigit) },
                label = { Text("Port") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                modifier = Modifier.fillMaxWidth(),
            )

            Text(
                "Far Cooler connects over SSH. This device must be authorized on the " +
                    "runner first.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    onClick = {
                        val trimmed = address.trim()
                        onSave(
                            Runner(
                                id = existing?.id ?: java.util.UUID.randomUUID().toString(),
                                label = label.trim().ifBlank { trimmed },
                                address = trimmed,
                                port = port.toIntOrNull() ?: 22,
                                user = user.trim(),
                                fingerprint = existing?.fingerprint,
                            )
                        )
                        onDismiss()
                    },
                    enabled = valid,
                ) {
                    Text(if (existing == null) "Add" else "Save")
                }
                TextButton(onClick = onDismiss) { Text("Cancel") }
                if (existing != null && onRemove != null) {
                    Spacer(Modifier.weight(1f))
                    TextButton(onClick = { confirmingRemove = true }) { Text("Remove") }
                }
            }
        }
    }

    if (confirmingRemove && existing != null && onRemove != null) {
        AlertDialog(
            onDismissRequest = { confirmingRemove = false },
            title = { Text("Remove ${existing.displayLabel}?") },
            text = { Text("Removes it from this device only. Nothing on the runner changes.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmingRemove = false
                    onRemove(existing)
                    onDismiss()
                }) { Text("Remove") }
            },
            dismissButton = {
                TextButton(onClick = { confirmingRemove = false }) { Text("Cancel") }
            },
        )
    }
}

/**
 * Start a task by describing it.
 *
 * The Mac's Quick Create exists because the old flow made you supply four
 * things — a name, a branch, an agent, a first message — when only the last one
 * is actually a decision; the rest are derivable from it. That argument applies
 * at least as strongly on a phone, where typing a name AND a branch AND a
 * message is a worse tax, not a smaller one.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun QuickTaskSheet(model: AppModel, onDismiss: () -> Unit) {
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    val connections by model.fleet.active.collectAsStateWithLifecycle()

    var text by remember { mutableStateOf("") }
    var runnerIndex by remember { mutableStateOf(0) }
    var repositoryId by remember { mutableStateOf("") }
    var agentId by remember { mutableStateOf("claude") }
    var model_ by remember { mutableStateOf("") }
    var phase by remember { mutableStateOf<String?>(null) }
    var failure by remember { mutableStateOf<Trouble?>(null) }
    var working by remember { mutableStateOf(false) }

    val connected = connections.filter { it.phase.value is Connection.Phase.Connected }
    val connection = connected.getOrNull(runnerIndex.coerceIn(0, (connected.size - 1).coerceAtLeast(0)))
    val repositories by (connection?.repositories
        ?: kotlinx.coroutines.flow.MutableStateFlow(emptyList())).collectAsStateWithLifecycle()
    // Per runner, because the branch is created on the one holding the project
    // and that runner's convention is the one that matters.
    val branchPrefix by (connection?.branchPrefix
        ?: kotlinx.coroutines.flow.MutableStateFlow(Connection.DEFAULT_BRANCH_PREFIX))
        .collectAsStateWithLifecycle()

    LaunchedEffect(repositories) {
        if (repositories.none { it.id == repositoryId }) {
            repositoryId = repositories.firstOrNull()?.id.orEmpty()
        }
    }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = state) {
        Column(
            Modifier
                .padding(horizontal = 20.dp)
                .padding(bottom = 20.dp)
                .imePadding()
                .navigationBarsPadding(),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("Quick task", style = MaterialTheme.typography.headlineSmall)

            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                label = { Text("What do you want done?") },
                minLines = 3,
                enabled = !working,
                modifier = Modifier.fillMaxWidth(),
            )

            if (text.isNotBlank()) {
                Text(
                    TaskSlug.slug(text, branchPrefix),
                    style = MaterialTheme.typography.labelSmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            // Only shown with a choice to make. A picker with one option is not
            // a picker, it is a label that looks tappable.
            if (connected.size > 1) {
                Picker(
                    label = "Runner",
                    options = connected.map { it.host.id to it.host.displayLabel },
                    selected = connection?.host?.id.orEmpty(),
                    enabled = !working,
                ) { id -> runnerIndex = connected.indexOfFirst { it.host.id == id } }
            }

            if (repositories.size > 1) {
                Picker(
                    label = "Project",
                    options = repositories.map { it.id to it.displayName },
                    selected = repositoryId,
                    enabled = !working,
                ) { repositoryId = it }
            }

            Picker(
                label = "Agent",
                options = QuickAgents.all.map { it.id to it.name },
                selected = agentId,
                enabled = !working,
            ) {
                agentId = it
                model_ = ""
            }

            Picker(
                label = "Model",
                options = listOf("" to "Default") + QuickAgents.agent(agentId).models.map { it to it },
                selected = model_,
                enabled = !working,
            ) { model_ = it }

            phase?.let {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(Modifier.height(16.dp).width(16.dp), strokeWidth = 2.dp)
                    Spacer(Modifier.width(8.dp))
                    Text(it, style = MaterialTheme.typography.bodySmall)
                }
            }
            failure?.let { SheetFailure(it) }

            Button(
                onClick = {
                    val description = text.trim()
                    val target = connection ?: return@Button
                    val repository = repositoryId.ifEmpty { return@Button }
                    working = true
                    failure = null
                    scope.launch {
                        // Each step's failure is reported at that step rather
                        // than folded into one generic error, because "could not
                        // create a worktree" and "the worktree exists but the
                        // agent never started" call for different next actions
                        // from whoever is reading this on a phone.
                        phase = "Creating worktree…"
                        val workspaceId = runCatching {
                            target.createWorkspace(
                                repository,
                                TaskSlug.name(description),
                                TaskSlug.slug(description, branchPrefix),
                                // Its own agent terminal is created a few lines
                                // below, so the worktree must not also come up
                                // with an unused shell beside it.
                                terminal = "",
                            )
                        }.getOrElse {
                            // One sentence about the step, and the runner's
                            // answer below it rather than joined to it with a
                            // colon. No cause named: from here this could be a
                            // path that is not a repository, a branch that
                            // exists, or a runner that stopped answering
                            // mid-call, and picking one would send somebody to
                            // fix something that was never wrong.
                            //
                            // Word for word the phone's, in `TaskComposer`:
                            // this sheet and that one are the same flow, and
                            // two spellings of one failure is the drift the
                            // whole rule exists to prevent.
                            failure = Trouble("Couldn’t create the worktree.", it.message)
                            phase = null
                            working = false
                            return@launch
                        }
                        target.refresh()

                        val agentName = QuickAgents.agent(agentId).name
                        phase = "Starting $agentName…"
                        val terminalId = runCatching {
                            target.createTerminal(
                                workspaceId,
                                agentId,
                                QuickAgents.preset(agentId, model_),
                            )
                        }.getOrElse {
                            // What DID happen stays in the sentence — the
                            // worktree exists, and somebody who reads only this
                            // line still knows there is one to go back to.
                            failure = Trouble(
                                "Created the worktree, but couldn’t start $agentName.",
                                it.message,
                            )
                            phase = null
                            working = false
                            return@launch
                        }

                        // Up to a minute, polling the same activity the fleet
                        // list shows: a cold agent on a slow runner is not a
                        // failure, and there is no fixed delay that is both
                        // short enough to feel quick and long enough to never
                        // be wrong.
                        var ready = false
                        for (attempt in 0 until 60) {
                            delay(1_000)
                            target.refresh()
                            val current = target.terminal(terminalId) ?: continue
                            when (current.agent) {
                                com.farcooler.model.AgentActivity.IDLE -> {
                                    ready = true
                                }
                                // It asked something before we got a word in — a
                                // trust prompt, or a resume dialog. Stop rather
                                // than typing a task description into a question
                                // it hasn't finished asking.
                                com.farcooler.model.AgentActivity.BLOCKED -> {
                                    // Cause and next action, both known. No
                                    // box: nothing came back to show, and
                                    // nothing needs to.
                                    failure = Trouble(
                                        "$agentName is waiting on a question. Open it to answer.")
                                    phase = null
                                    working = false
                                    return@launch
                                }
                                else -> continue
                            }
                            if (ready) break
                        }
                        if (!ready) {
                            failure = Trouble(
                                "$agentName was not ready within a minute. Nothing was sent — " +
                                    "open it to check on it.")
                            phase = null
                            working = false
                            return@launch
                        }

                        phase = "Sending the task…"
                        runCatching {
                            target.writeRaw(terminalId, hexEncode(description))
                            // The return as its own write, deliberately: an
                            // agent's composer treats a newline that arrives in
                            // the same packet as pasted text, not as submit.
                            target.writeRaw(terminalId, "0d")
                        }.onFailure {
                            failure = Trouble(
                                "Started $agentName, but couldn’t send the task.",
                                it.message,
                            )
                            phase = null
                            working = false
                            return@launch
                        }

                        working = false
                        phase = null
                        text = ""
                        onDismiss()
                    }
                },
                enabled = !working && text.isNotBlank() && repositoryId.isNotEmpty(),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(if (working) "Starting…" else "Start")
            }
        }
    }
}

/** The form next to Quick Task, for a workspace you want to name yourself. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NewWorkspaceSheet(model: AppModel, onDismiss: () -> Unit) {
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    val connections by model.fleet.active.collectAsStateWithLifecycle()
    val connected = connections.filter { it.phase.value is Connection.Phase.Connected }

    var runnerId by remember { mutableStateOf(connected.firstOrNull()?.host?.id.orEmpty()) }
    val connection = connected.firstOrNull { it.host.id == runnerId } ?: connected.firstOrNull()
    val repositories by (connection?.repositories
        ?: kotlinx.coroutines.flow.MutableStateFlow(emptyList())).collectAsStateWithLifecycle()

    var repositoryId by remember { mutableStateOf("") }
    var name by remember { mutableStateOf("") }
    var branch by remember { mutableStateOf("") }
    var working by remember { mutableStateOf(false) }
    var failure by remember { mutableStateOf<Trouble?>(null) }

    // Per runner, because the branch is created on the one holding the project.
    val branchPrefix by (connection?.branchPrefix
        ?: kotlinx.coroutines.flow.MutableStateFlow(Connection.DEFAULT_BRANCH_PREFIX))
        .collectAsStateWithLifecycle()

    val trimmedName = name.trim()
    // The name IS the worktree's directory now, and nothing encourages naming
    // one carefully while the thing being named is invisible.
    val folder = TaskSlug.sanitize(trimmedName)
    // Sixty is the runner's cap on a name.
    val tooLong = trimmedName.codePointCount(0, trimmedName.length) > 60

    // This form used to make you type a branch by hand, which meant the
    // runner's branch prefix — the whole point of the setting — could not reach
    // the one place on this screen that names a branch. Now it matches the Mac's
    // sheet: type nothing and get the suggestion.
    val suggestedBranch =
        if (trimmedName.isEmpty()) "" else TaskSlug.slug(trimmedName, branchPrefix)
    val effectiveBranch = branch.trim().ifEmpty { suggestedBranch }

    LaunchedEffect(repositories) {
        if (repositories.none { it.id == repositoryId }) {
            repositoryId = repositories.firstOrNull()?.id.orEmpty()
        }
    }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = state) {
        Column(
            Modifier
                .padding(horizontal = 20.dp)
                .padding(bottom = 20.dp)
                .imePadding()
                .navigationBarsPadding(),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("New workspace", style = MaterialTheme.typography.headlineSmall)

            if (connected.size > 1) {
                Picker(
                    label = "Runner",
                    options = connected.map { it.host.id to it.host.displayLabel },
                    selected = connection?.host?.id.orEmpty(),
                    enabled = !working,
                ) { runnerId = it }
            }
            Picker(
                label = "Project",
                options = repositories.map { it.id to it.displayName },
                selected = repositoryId,
                enabled = !working,
            ) { repositoryId = it }

            OutlinedTextField(
                value = name,
                onValueChange = { name = it },
                label = { Text("Name") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            // What the name becomes on disk, or why it cannot become anything.
            // Both refusals are spelled out rather than left as a dimmed Create
            // button, which says a name is wrong without saying which rule it
            // broke.
            if (trimmedName.isNotEmpty()) {
                when {
                    tooLong -> Text(
                        "A name can be at most 60 characters.",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                    folder.isEmpty() -> Text(
                        "A name needs a letter or a number in it.",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                    else -> Text(
                        folder,
                        style = MaterialTheme.typography.labelSmall,
                        fontFamily = FontFamily.Monospace,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            OutlinedTextField(
                value = branch,
                onValueChange = { branch = it },
                label = { Text("Branch") },
                placeholder = {
                    Text(suggestedBranch.ifEmpty { branchPrefix + "my-worktree" })
                },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )

            Text(
                "A workspace is one Git worktree and one branch. Its name is the worktree’s " +
                    "folder, so it can’t be changed later.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            failure?.let { SheetFailure(it) }

            Button(
                onClick = {
                    val target = connection ?: return@Button
                    working = true
                    scope.launch {
                        runCatching {
                            target.createWorkspace(repositoryId, trimmedName, effectiveBranch)
                        }.onFailure {
                            // It used to be the whole red line, so whatever the
                            // core said about a path or a branch was set in the
                            // face this sheet writes its own refusals in — the
                            // two above this button among them. Same sentence
                            // as Quick Task's, because it is the same failure.
                            failure = Trouble("Couldn’t create the worktree.", it.message)
                            working = false
                            return@launch
                        }
                        target.refresh()
                        working = false
                        onDismiss()
                    }
                },
                enabled = !working && repositoryId.isNotEmpty() &&
                    folder.isNotEmpty() && !tooLong && effectiveBranch.isNotBlank(),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Create")
            }
        }
    }
}

/**
 * A second pane in a workspace that already has one.
 *
 * The Mac offers this on every workspace; iOS only ever creates a terminal as
 * part of Quick Task, so a workspace that needed an agent AND a shell to watch
 * it — the ordinary layout on the Mac — could not get one from a phone at all.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NewTerminalSheet(connection: Connection, workspace: Workspace, onDismiss: () -> Unit) {
    val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()
    var presetId by remember { mutableStateOf(TerminalPresets.all.first().id) }
    var model_ by remember { mutableStateOf("") }
    var working by remember { mutableStateOf(false) }

    val models = QuickAgents.all.firstOrNull { it.id == presetId }?.models ?: emptyList()

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = state) {
        Column(
            Modifier
                .padding(horizontal = 20.dp)
                .padding(bottom = 20.dp)
                .navigationBarsPadding(),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("New terminal", style = MaterialTheme.typography.headlineSmall)
            Text(
                workspace.task.ifBlank { workspace.branch },
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            Picker(
                label = "Preset",
                options = TerminalPresets.all.map { it.id to it.name },
                selected = presetId,
                enabled = !working,
            ) {
                presetId = it
                model_ = ""
            }

            if (models.isNotEmpty()) {
                Picker(
                    label = "Model",
                    options = listOf("" to "Default") + models.map { it to it },
                    selected = model_,
                    enabled = !working,
                ) { model_ = it }
            }

            Button(
                onClick = {
                    working = true
                    scope.launch {
                        runCatching {
                            connection.createTerminal(
                                workspace.id,
                                presetId,
                                if (models.isEmpty()) presetId
                                else QuickAgents.preset(presetId, model_),
                            )
                        }
                        connection.refresh()
                        working = false
                        onDismiss()
                    }
                },
                enabled = !working,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text("Start")
            }
        }
    }
}

/** One labelled choice, as the platform's own dropdown. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun Picker(
    label: String,
    options: List<Pair<String, String>>,
    selected: String,
    enabled: Boolean = true,
    onSelect: (String) -> Unit,
) {
    var expanded by remember { mutableStateOf(false) }
    val current = options.firstOrNull { it.first == selected }?.second.orEmpty()

    ExposedDropdownMenuBox(
        expanded = expanded && enabled,
        onExpandedChange = { if (enabled) expanded = it },
    ) {
        OutlinedTextField(
            value = current,
            onValueChange = {},
            readOnly = true,
            enabled = enabled,
            label = { Text(label) },
            trailingIcon = { ExposedDropdownMenuDefaults.TrailingIcon(expanded) },
            modifier = Modifier
                .fillMaxWidth()
                .menuAnchor(androidx.compose.material3.ExposedDropdownMenuAnchorType.PrimaryNotEditable),
        )
        ExposedDropdownMenu(expanded = expanded && enabled, onDismissRequest = { expanded = false }) {
            for ((id, name) in options) {
                DropdownMenuItem(
                    text = { Text(name) },
                    onClick = {
                        expanded = false
                        onSelect(id)
                    },
                )
            }
        }
    }
}

/**
 * One failure, drawn the way this app draws them: the sentence, then the
 * transcript under it.
 *
 * One composable rather than a copy in each sheet, so two sheets reporting the
 * same kind of failure cannot come to render it differently. Its Apple twin is
 * `SheetFailureSection` in `apps/ios/FarCooler/FleetView.swift`.
 *
 * Internal rather than private since the review's sheets landed: the outbox
 * reports a send that did not go and the base picker reports a branch list that
 * could not be read, and both are the same two fields under the same rule — the
 * sentence this app wrote, and only where it has none of its own, the runner's
 * words in a box beneath it.
 */
@Composable
internal fun SheetFailure(trouble: Trouble) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(
            trouble.sentence,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.error,
        )
        trouble.transcript?.takeIf { it.isNotEmpty() }?.let { DetailBox(it) }
    }
}

private fun hexEncode(text: String): String =
    text.toByteArray(Charsets.UTF_8).joinToString("") { "%02x".format(it) }
