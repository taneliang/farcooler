package com.farcooler.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.outlined.HelpOutline
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.AgentConversation
import com.farcooler.model.AgentRow
import com.farcooler.net.NativePaneModel

/** What a row's buttons need from its pane (ov-370): how the last answer went, and where the next goes. */
class NativeAnswer(
    val answering: String?,
    val issues: Map<String, String>,
    val send: (ask: AgentRow.Ask, option: String, answers: Map<String, String>) -> Unit,
    /** The agent asking, as the row's title names it. */
    val agent: String = "Claude",
)

/** The rows' answering, where the runner takes answers. */
fun nativeAnswer(model: NativePaneModel): NativeAnswer? {
    if (model.answers == null) return null
    return NativeAnswer(model.answering, model.answerIssues, { ask, option, answers -> model.answer(ask, option, answers) }, model.agent)
}

/**
 * Claude asking: a permission, a question, or a plan to approve (ov-370), as
 * the iPhone's `NativeAskRow` draws it.
 *
 * While the runner's hook holds the ask, the row answers it: Allow or Deny; the
 * question's options, then Send answer; or Approve plan or Keep planning. The
 * answer goes to the hook, never as keys into claude's dialog, and the first
 * from any device wins. Once the hold ends, only the terminal can answer, and
 * the row offers Show terminal.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun NativeAskRow(ask: AgentRow.Ask, answer: NativeAnswer?, showTerminal: () -> Unit) {
    var picked by remember(ask.held) { mutableStateOf<Map<Int, Set<String>>>(emptyMap()) }
    var typed by remember(ask.held) { mutableStateOf<Map<Int, String>>(emptyMap()) }
    // The hold this row last answered, so what became of the answer is still
    // said once the hold ends (review 1 M1).
    var sentFor by remember { mutableStateOf<String?>(null) }
    val waiting = !ask.answered && ask.answeredBy == null
    val canAnswer = answer != null && AgentConversation.answerable(ask)
    val sending = ask.held != null && answer?.answering == ask.held
    val drawnBelow = waiting && if (ask.kind == "Question") ask.questions.isNotEmpty() else ask.kind == "PlanExit" && ask.plan != null

    Column(attentionCard(waiting), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Icon(if (waiting) Icons.Outlined.HelpOutline else Icons.Filled.Check, null, Modifier.size(18.dp))
            Text(AgentConversation.askTitle(ask, answer?.agent ?: "Claude"), style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium)
        }
        if (!drawnBelow) {
            Text(ask.text, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 4, overflow = TextOverflow.Ellipsis)
        }
        val plan = ask.plan
        if (waiting && ask.kind == "PlanExit" && plan != null) {
            MarkdownText(plan, modifier = Modifier.fillMaxWidth().testTag("native-ask-plan"))
        }
        if (waiting && ask.kind == "Question") {
            ask.questions.forEachIndexed { i, question ->
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    if (question.header.isNotEmpty()) {
                        Text(question.header, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    Text(question.question, style = MaterialTheme.typography.bodyMedium)
                    question.options.forEachIndexed { o, option ->
                        val on = picked[i]?.contains(option.label) == true
                        val toggle = { picked = picked + (i to AgentConversation.pick(option.label, question, picked[i].orEmpty())) }
                        Row(
                            Modifier.fillMaxWidth().clickable(enabled = canAnswer && !sending, onClick = toggle).testTag("native-ask-option-$i-$o"),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            if (question.multiSelect) {
                                Checkbox(checked = on, onCheckedChange = { toggle() }, enabled = canAnswer && !sending)
                            } else {
                                RadioButton(selected = on, onClick = toggle, enabled = canAnswer && !sending)
                            }
                            Column {
                                Text(option.label, style = MaterialTheme.typography.bodyMedium)
                                if (option.description.isNotEmpty()) {
                                    Text(option.description, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                            }
                        }
                    }
                    if (canAnswer) {
                        OutlinedTextField(
                            value = typed[i].orEmpty(),
                            onValueChange = { typed = typed + (i to it) },
                            label = { Text("Other") },
                            singleLine = true,
                            enabled = !sending,
                            modifier = Modifier.fillMaxWidth().testTag("native-ask-other-$i"),
                        )
                    }
                }
            }
        }
        if (canAnswer && answer != null) {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                val send = { option: String, given: Map<String, String> ->
                    sentFor = ask.held
                    answer.send(ask, option, given)
                }
                when (ask.kind) {
                    "Question" -> {
                        val given = AgentConversation.answers(ask.questions, picked, typed)
                        Button(onClick = { send(AgentConversation.ANSWER, given.orEmpty()) }, enabled = given != null && !sending, modifier = Modifier.testTag("native-ask-send-answer")) {
                            Text("Send answer")
                        }
                    }
                    "PlanExit" -> {
                        Button(onClick = { send(AgentConversation.ALLOW, emptyMap()) }, enabled = !sending, modifier = Modifier.testTag("native-ask-approve")) {
                            Text("Approve plan")
                        }
                        OutlinedButton(onClick = { send(AgentConversation.DENY, emptyMap()) }, enabled = !sending, modifier = Modifier.testTag("native-ask-keep-planning")) {
                            Text("Keep planning")
                        }
                    }
                    else -> {
                        Button(onClick = { send(AgentConversation.ALLOW, emptyMap()) }, enabled = !sending, modifier = Modifier.testTag("native-ask-allow")) {
                            Text("Allow")
                        }
                        OutlinedButton(onClick = { send(AgentConversation.DENY, emptyMap()) }, enabled = !sending, modifier = Modifier.testTag("native-ask-deny")) {
                            Text("Deny")
                        }
                    }
                }
                OutlinedButton(onClick = showTerminal, modifier = Modifier.testTag("native-ask-show-terminal")) { Text("Show terminal") }
                if (sending) CircularProgressIndicator(Modifier.size(20.dp).align(Alignment.CenterVertically), strokeWidth = 2.dp)
            }
        } else if (waiting) {
            OutlinedButton(onClick = showTerminal, modifier = Modifier.testTag("native-ask-show-terminal")) { Text("Show terminal") }
        }
        val held = ask.held ?: sentFor
        val issue = if (held != null) answer?.issues?.get(held) else null
        if (issue != null) {
            Text(issue, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.testTag("native-ask-issue"))
        }
    }
}
