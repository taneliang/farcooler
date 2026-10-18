package com.farcooler.ui

import android.content.Context
import androidx.browser.customtabs.CustomTabsIntent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
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
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.farcooler.model.ClaudeLogin
import com.farcooler.model.ClaudeLoginState
import com.farcooler.model.ClaudeSignIn
import com.farcooler.model.ClaudeSignInApi
import com.farcooler.model.ClaudeSignInFlow
import com.farcooler.model.ClaudeSignInStep
import com.farcooler.model.PageOpen
import com.farcooler.net.Connection
import com.farcooler.net.SignInReply
import com.farcooler.net.cancelClaudeSignIn
import com.farcooler.net.claudeLogin
import com.farcooler.net.finishClaudeSignIn
import com.farcooler.net.signOutOfClaude
import com.farcooler.net.startClaudeSignIn
import kotlinx.coroutines.launch

/**
 * Opens the sign-in page in a Custom Tab, after checking it again: the phone
 * checks the address it is about to open, whoever handed it over. Refused when it
 * isn't Claude's page, [PageOpen.NO_BROWSER] when no browser takes it.
 */
internal fun openSignInPage(context: Context, url: String): PageOpen {
    if (!ClaudeSignIn.isSignInPage(url)) return PageOpen.REFUSED
    return try {
        CustomTabsIntent.Builder().setShowTitle(false).build().launchUrl(context, android.net.Uri.parse(url))
        PageOpen.OPENED
    } catch (_: Exception) {
        PageOpen.NO_BROWSER
    }
}

/**
 * The Claude section of Runner Settings (ov-584): this member's own login on
 * this runner, and the sheet that signs them in. Drawn only for a member's own
 * device on a runner that offers it ([ClaudeSignIn.offered]); the caller asks.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ClaudeSignInSection(connection: Connection, you: String) {
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val runner = connection.host.displayLabel
    var login by remember(connection, you) { mutableStateOf<ClaudeLogin?>(null) }
    var signingOut by remember { mutableStateOf(false) }
    var signOutFailed by remember { mutableStateOf(false) }
    var confirmingSignOut by remember { mutableStateOf(false) }
    val flow = remember(connection, you) {
        ClaudeSignInFlow(
            scope = scope,
            api = object : ClaudeSignInApi {
                override suspend fun start() = connection.startClaudeSignIn(you)
                override suspend fun finish(session: String, code: String) = connection.finishClaudeSignIn(session, code)
                override suspend fun cancel(session: String) = connection.cancelClaudeSignIn(session)
            },
            runner = runner,
            onLogin = { login = it },
        )
    }
    val step by flow.step.collectAsStateWithLifecycle()
    val code by flow.code.collectAsStateWithLifecycle()
    val codeNote by flow.codeNote.collectAsStateWithLifecycle()

    LaunchedEffect(connection, you) { login = connection.claudeLogin(you) }

    ClaudeSection(
        login = login,
        runner = runner,
        signingOut = signingOut,
        onSignIn = flow::start,
        notice = if (signOutFailed) ClaudeSignIn.SIGN_OUT_FAILED else null,
        onSignOut = { confirmingSignOut = true },
    )

    if (confirmingSignOut) {
        AlertDialog(
            onDismissRequest = { confirmingSignOut = false },
            title = { Text(ClaudeSignIn.signOutTitle(runner)) },
            text = { Text(ClaudeSignIn.SIGN_OUT_MESSAGE) },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmingSignOut = false
                        signingOut = true
                        signOutFailed = false
                        scope.launch {
                            val reply = connection.signOutOfClaude(you)
                            if (reply is SignInReply.Ok) login = reply.value else signOutFailed = true
                            signingOut = false
                        }
                    },
                    modifier = Modifier.testTag("claude-sign-out-confirm"),
                ) { Text(ClaudeSignIn.SIGN_OUT, color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { confirmingSignOut = false }) { Text(ClaudeSignIn.CANCEL) } },
        )
    }

    val showing = step
    if (showing != null) {
        ModalBottomSheet(
            onDismissRequest = flow::close,
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            modifier = Modifier.testTag("claude-sign-in-sheet"),
        ) {
            ClaudeSignInSheet(
                step = showing,
                runner = runner,
                code = code,
                codeNote = codeNote,
                onCode = flow::edit,
                onOpenPage = { url -> flow.opened(url, openSignInPage(context, url)) },
                onSubmit = flow::submit,
                onRestart = flow::start,
                onClose = flow::close,
            )
        }
    }
}

/**
 * The section itself, stateless. Not signed in: why, the button, and what
 * keeping the login means. Signed in: whose, and the two ways to change it.
 * Nothing before the runner has said ([login] null).
 */
@Composable
internal fun ClaudeSection(
    login: ClaudeLogin?,
    runner: String,
    signingOut: Boolean = false,
    notice: String? = null,
    onSignIn: () -> Unit,
    onSignOut: () -> Unit,
) {
    if (login == null) return
    Column(Modifier.fillMaxWidth().testTag("claude-section"), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        SectionTitle(ClaudeSignIn.SECTION_TITLE)
        when (login.state) {
            ClaudeLoginState.SIGNED_IN, ClaudeLoginState.STOPPED_WORKING -> {
                Text(
                    if (login.state == ClaudeLoginState.SIGNED_IN) login.email else ClaudeSignIn.STOPPED_WORKING,
                    style = MaterialTheme.typography.bodyLarge,
                    maxLines = 1,
                    overflow = TextOverflow.MiddleEllipsis,
                    modifier = Modifier.testTag("claude-row"),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                    OutlinedButton(onClick = onSignIn, enabled = !signingOut, modifier = Modifier.testTag("claude-sign-in-again")) {
                        Text(ClaudeSignIn.SIGN_IN_AGAIN)
                    }
                    TextButton(onClick = onSignOut, enabled = !signingOut, modifier = Modifier.testTag("claude-sign-out")) {
                        Text(ClaudeSignIn.SIGN_OUT, color = MaterialTheme.colorScheme.error)
                    }
                    if (signingOut) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                }
                notice?.let {
                    Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("claude-notice"))
                }
            }
            ClaudeLoginState.NONE -> {
                Text(ClaudeSignIn.BODY, style = MaterialTheme.typography.bodyMedium)
                Button(onClick = onSignIn, modifier = Modifier.testTag("claude-sign-in")) { Text(ClaudeSignIn.SIGN_IN) }
            }
        }
        Text(
            ClaudeSignIn.trust(runner),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.testTag("claude-trust"),
        )
        // This section's own rule, drawn only with it, as HealthSection does.
        Separator()
    }
}

/** The sheet's contents for [step], stateless. */
@Composable
internal fun ClaudeSignInSheet(
    step: ClaudeSignInStep,
    runner: String,
    code: String,
    codeNote: String? = null,
    onCode: (String) -> Unit,
    onOpenPage: (String) -> Unit,
    onSubmit: (String) -> Unit,
    onRestart: () -> Unit,
    onClose: () -> Unit,
) {
    Column(
        Modifier
            .fillMaxWidth()
            .navigationBarsPadding()
            .imePadding()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 24.dp)
            .padding(bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(ClaudeSignIn.SHEET_TITLE, style = MaterialTheme.typography.titleLarge, modifier = Modifier.testTag("claude-sheet-title"))
        when (step) {
            ClaudeSignInStep.Starting -> {
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                    Text(ClaudeSignIn.starting(runner), style = MaterialTheme.typography.bodyMedium)
                }
            }
            is ClaudeSignInStep.OpenPage -> {
                OpenPageStep(step.url, onOpenPage, primary = true)
            }
            is ClaudeSignInStep.Code -> {
                OpenPageStep(step.url, onOpenPage, primary = false)
                CodeStep(code, onCode, null, codeNote, enabled = true) { onSubmit(step.url) }
            }
            ClaudeSignInStep.Finishing -> {
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                    Text(ClaudeSignIn.FINISHING, style = MaterialTheme.typography.bodyMedium)
                }
            }
            is ClaudeSignInStep.Done -> {
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(ClaudeSignIn.SIGNED_IN_AS, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.testTag("claude-done-text"))
                    // One line, cut in the middle: an address has no spaces to break at.
                    Text(
                        step.email,
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.MiddleEllipsis,
                        modifier = Modifier.testTag("claude-done-email"),
                    )
                }
                Button(onClick = onClose, modifier = Modifier.fillMaxWidth().testTag("claude-done")) { Text(ClaudeSignIn.DONE) }
            }
            is ClaudeSignInStep.Failed -> {
                if (step.failure == ClaudeSignIn.Failure.Incomplete && step.url != null) {
                    OpenPageStep(step.url, onOpenPage, primary = false)
                    CodeStep(code, onCode, step.failure.sentence, codeNote, enabled = true) { onSubmit(step.url) }
                } else {
                    Text(
                        step.failure.sentence,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.error,
                        modifier = Modifier.testTag("claude-failure"),
                    )
                    step.failure.action?.let { action ->
                        Button(onClick = onRestart, modifier = Modifier.fillMaxWidth().testTag("claude-restart")) { Text(action) }
                    }
                }
            }
        }
        if (step !is ClaudeSignInStep.Done) {
            // On a failure that ends the attempt the sentence is the thing to read.
            val failed = step as? ClaudeSignInStep.Failed
            if (failed == null || failed.failure == ClaudeSignIn.Failure.Incomplete) {
                Text(
                    ClaudeSignIn.trust(runner),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.testTag("claude-sheet-trust"),
                )
            }
            TextButton(onClick = onClose, modifier = Modifier.fillMaxWidth().testTag("claude-cancel")) { Text(ClaudeSignIn.CANCEL) }
        }
    }
}

@Composable
private fun OpenPageStep(url: String, onOpen: (String) -> Unit, primary: Boolean) {
    Text(ClaudeSignIn.OPEN_PAGE_BODY, style = MaterialTheme.typography.bodyMedium)
    val modifier = Modifier.fillMaxWidth().testTag("claude-open-page")
    if (primary) {
        Button(onClick = { onOpen(url) }, modifier = modifier) { Text(ClaudeSignIn.OPEN_PAGE) }
    } else {
        OutlinedButton(onClick = { onOpen(url) }, modifier = modifier) { Text(ClaudeSignIn.OPEN_PAGE) }
    }
}

@Composable
private fun CodeStep(code: String, onCode: (String) -> Unit, error: String?, note: String?, enabled: Boolean, onSubmit: () -> Unit) {
    Text(ClaudeSignIn.CODE_BODY, style = MaterialTheme.typography.bodyMedium)
    OutlinedTextField(
        value = code,
        onValueChange = onCode,
        label = { Text(ClaudeSignIn.CODE_LABEL) },
        textStyle = MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace),
        keyboardOptions = KeyboardOptions(
            capitalization = KeyboardCapitalization.None,
            autoCorrectEnabled = false,
            keyboardType = KeyboardType.Ascii,
        ),
        isError = error != null,
        // The paste note is supporting text; only a failure turns the field red.
        supportingText = (error ?: note)?.let { { Text(it, modifier = Modifier.testTag(if (error != null) "claude-code-error" else "claude-code-note")) } },
        minLines = 2,
        maxLines = 4,
        modifier = Modifier.fillMaxWidth().testTag("claude-code"),
    )
    Button(
        onClick = onSubmit,
        enabled = enabled && ClaudeSignIn.code(code) != null,
        modifier = Modifier.fillMaxWidth().testTag("claude-finish"),
    ) { Text(ClaudeSignIn.FINISH) }
}
