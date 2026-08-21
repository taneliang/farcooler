package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Chat
import androidx.compose.material.icons.filled.ContentPaste
import androidx.compose.material.icons.filled.Image
import androidx.compose.material.icons.filled.Keyboard
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.outlined.Menu
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.core.TerminalPalette
import com.farcooler.core.Vt
import com.farcooler.net.TerminalRef
import com.farcooler.net.TerminalSession
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * One terminal, live — or the chat behind the same pane.
 *
 * Which of the two is drawn comes from `terminal.isAgentPane`, which the daemon
 * sets. Never derived here, the same rule that keeps activity and agent mode as
 * reported rather than guessed at.
 *
 * The tab strip lives at the BOTTOM, in thumb reach, and rises with the
 * keyboard. Under the title bar it would put the one control you use
 * constantly — switching terminal — at the far end of the screen from the hand
 * holding the phone.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TerminalScreen(model: AppModel, ref: TerminalRef, onOpenDrawer: () -> Unit) {
    val connection = model.fleet.connection(ref) ?: run {
        // The runner this pane was on is gone — removed in settings, or its
        // connection torn down and rebuilt. Saying so beats a blank screen.
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            Text("That runner is no longer connected.")
        }
        return
    }

    val scope = rememberCoroutineScope()
    val clipboard = LocalClipboard.current

    // The URL a long press landed on, which is also what shows the sheet: a
    // dialog that can be presented with nothing to present is one that
    // eventually will be.
    var heldLink by remember { mutableStateOf<String?>(null) }

    val fontChoice by model.settings.font.collectAsStateWithLifecycle()
    val fontSize by model.settings.fontSize.collectAsStateWithLifecycle()
    val reshape by model.settings.reshapePanes.collectAsStateWithLifecycle()
    val entries by model.fleet.entries.collectAsStateWithLifecycle()
    val connections by model.fleet.active.collectAsStateWithLifecycle()
    connection.fleet.collectAsStateWithLifecycle()

    // The terminal as the daemon describes it RIGHT NOW.
    //
    // A stored copy is right for identity and wrong for anything that changes
    // underneath it — pane mode above all. Switching to chat left an iOS copy
    // still saying "terminal", so the button asked for the same switch every
    // time and the screen kept drawing a VT grid.
    val live = model.fleet.terminal(ref)
    val workspace = model.fleet.workspace(ref)
    val ordinal = workspace?.ordinals()?.get(ref.terminalId)
    val name = live?.displayName(ordinal) ?: "Terminal"

    val session = remember(ref.hostId) { TerminalSession(ref.terminalId, connection.core) }
    // `start` for the pane this session was built around, `switchTo` for every
    // one after it. `switchTo` deliberately does nothing when the id has not
    // changed, so it cannot be what opens the first one.
    LaunchedEffect(session) { session.start() }
    LaunchedEffect(ref.terminalId) { session.switchTo(ref.terminalId) }
    LaunchedEffect(reshape) { session.reshapeAllowed = reshape }
    // A program putting text on the clipboard (OSC 52). Collected here because
    // this is where a Context — and therefore the clipboard — exists at all.
    // On a phone this is the only way anything on screen reaches the clipboard:
    // there is no text selection in the renderer.
    LaunchedEffect(session) {
        session.copied.collect { clipboard.writeText("Far Cooler", it) }
    }
    // The link under this pane was replaced.
    //
    // A stream is a second SSH channel on the session that just died, so
    // everything this screen had open went with it. [TerminalSession] survives
    // that on its own by falling back to polling, which is the right behaviour
    // and the slower path; this puts it back on the stream now that there is
    // one to be on. Skipped on the first composition, where the count is
    // whatever it already was and nothing has been replaced.
    val reconnects by connection.reconnects.collectAsStateWithLifecycle()
    var seenReconnects by remember(ref.hostId) { mutableIntStateOf(reconnects) }
    LaunchedEffect(reconnects) {
        if (reconnects == seenReconnects) return@LaunchedEffect
        seenReconnects = reconnects
        session.relink()
    }
    DisposableEffect(session) {
        onDispose { session.dispose() }
    }

    // Which pane is on screen, so a banner about THIS one is suppressed while
    // banners about the others still arrive — and so `done` is ended for it,
    // which is what stops an agent you have read staying orange forever.
    DisposableEffect(ref.terminalId) {
        connection.visibleTerminal = ref.terminalId
        model.notifier.visibleTerminal = ref.terminalId
        scope.launch { connection.markVisibleSeen() }
        onDispose {
            connection.visibleTerminal = null
            model.notifier.visibleTerminal = null
        }
    }

    var showMenu by remember { mutableStateOf(false) }

    // Images on their way into this pane, and the picker that starts one.
    // Keyed to nothing, so switching terminals in place does not lose a
    // transfer that is still running.
    val pastes = remember { ImagePasteQueue() }
    val resolver = LocalContext.current.contentResolver
    val pickImage = rememberLauncherForActivityResult(
        ActivityResultContracts.PickVisualMedia()
    ) { uri ->
        if (uri == null) return@rememberLauncherForActivityResult
        scope.launch {
            val picked = withContext(Dispatchers.IO) { readPickedImage(resolver, uri) }
            if (picked == null) {
                pastes.reject("Far Cooler couldn’t read that image.")
                return@launch
            }
            pastes.send(
                picked.data,
                picked.name,
                picked.mime,
                picked.thumbnail,
                ref.terminalId,
                connection.core,
                scope,
            )
        }
    }
    var focusRequest by remember { mutableIntStateOf(0) }
    var dismissRequest by remember { mutableIntStateOf(0) }
    var ctrlArmed by remember { mutableStateOf(false) }
    var altArmed by remember { mutableStateOf(false) }

    /**
     * Ctrl and Alt are toggles, not held keys — there is nothing on a
     * touchscreen that behaves like holding a modifier down. So each applies to
     * exactly the next key and then clears itself, the same shape as shift-lock
     * on a keyboard with only one hand.
     */
    fun consumeModifiers(): Int {
        var modifiers = 0
        if (ctrlArmed) modifiers = modifiers or Vt.MOD_CTRL
        if (altArmed) modifiers = modifiers or Vt.MOD_ALT
        ctrlArmed = false
        altArmed = false
        return modifiers
    }

    val imeVisible = WindowInsets.ime.getBottom(androidx.compose.ui.platform.LocalDensity.current) > 0

    Scaffold(
        containerColor = Color(TerminalPalette.BACKGROUND),
        topBar = {
            TopAppBar(
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = Color(TerminalPalette.BACKGROUND),
                ),
                title = {
                    // The task and its branch, not the terminal's own name —
                    // the terminal already names itself in its tab strip chip,
                    // and repeating it here would waste the one line of title
                    // bar a phone has on something already on screen. The
                    // runner appears only when more than one is connected.
                    Column {
                        Text(
                            workspace?.task?.ifBlank { null } ?: name,
                            style = MaterialTheme.typography.titleMedium,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        val subtitle = buildString {
                            workspace?.branch?.takeIf { it.isNotBlank() }?.let { append(it) }
                            if (connections.size > 1) {
                                if (isNotEmpty()) append(" · ")
                                append(connection.host.displayLabel)
                            }
                        }
                        if (subtitle.isNotEmpty()) {
                            Text(
                                subtitle,
                                style = MaterialTheme.typography.labelSmall,
                                fontFamily = FontFamily.Monospace,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                },
                navigationIcon = {
                    IconButton(onClick = onOpenDrawer) {
                        Icon(Icons.Outlined.Menu, contentDescription = "Show the fleet")
                    }
                },
                actions = {
                    // Terminal or chat, on the pane that can be either. Shown
                    // only where it would work: `chatCapable` already reflects
                    // the daemon's registry-backed check, so a pane whose agent
                    // has no adapter never gets the button in the first place.
                    if (live?.canSwitchPaneMode == true) {
                        IconButton(onClick = {
                            scope.launch {
                                connection.setPaneMode(
                                    live,
                                    if (live.isAgentPane) "terminal" else "agent",
                                )
                            }
                        }) {
                            Icon(
                                if (live.isAgentPane) Icons.Outlined.Terminal
                                else Icons.AutoMirrored.Filled.Chat,
                                contentDescription =
                                    if (live.isAgentPane) "Show the terminal" else "Show the chat",
                            )
                        }
                    }
                    Box {
                        IconButton(onClick = { showMenu = true }) {
                            Icon(Icons.Filled.MoreVert, contentDescription = "More")
                        }
                        DropdownMenu(showMenu, onDismissRequest = { showMenu = false }) {
                            if (live?.isAgentPane != true) {
                                DropdownMenuItem(
                                    text = { Text("Paste") },
                                    leadingIcon = { Icon(Icons.Filled.ContentPaste, null) },
                                    onClick = {
                                        showMenu = false
                                        scope.launch {
                                            clipboard.readText()?.let { session.paste(it) }
                                        }
                                    },
                                )
                                DropdownMenuItem(
                                    text = { Text("Send Image") },
                                    leadingIcon = { Icon(Icons.Filled.Image, null) },
                                    onClick = {
                                        showMenu = false
                                        pickImage.launch(
                                            PickVisualMediaRequest(
                                                ActivityResultContracts.PickVisualMedia.ImageOnly
                                            )
                                        )
                                    },
                                )
                                DropdownMenuItem(
                                    text = { Text("Show keyboard") },
                                    leadingIcon = { Icon(Icons.Filled.Keyboard, null) },
                                    onClick = {
                                        showMenu = false
                                        focusRequest += 1
                                    },
                                )
                            }
                        }
                    }
                },
            )
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .imePadding()
        ) {
            Box(Modifier.weight(1f)) {
                // Over the terminal, and gone the moment the path is typed.
                // Nothing about a transfer is ever written into the pane
                // itself: the path is the only thing that reaches the program.
                ImagePasteChips(pastes, Modifier.align(Alignment.BottomCenter))
                if (live?.isAgentPane == true) {
                    AgentScreen(
                        model = model,
                        ref = ref,
                        connection = connection,
                    )
                } else {
                    TerminalSurface(
                        session = session,
                        name = name,
                        fontFamily = TerminalFonts.family(fontChoice),
                        fontSize = fontSize,
                        onTap = { focusRequest += 1 },
                        onLongPress = { column, row ->
                            // Over a link, the link actions. Anywhere else, the
                            // paste this gesture has always meant: a phone has no
                            // other way to get a command it did not type into a
                            // terminal, and the menu is two taps away at the top
                            // of a screen whose whole point is one-handed use.
                            //
                            // So the new behavior only appears where there is
                            // something to act on, and the old one is untouched
                            // everywhere else.
                            val link = session.urlAt(row, column)
                            if (link != null) {
                                heldLink = link
                            } else {
                                scope.launch { clipboard.readText()?.let { session.paste(it) } }
                            }
                        },
                    )
                    TerminalKeyboardAnchor(
                        focusRequest = focusRequest,
                        dismissRequest = dismissRequest,
                        onText = { text -> session.send(text, consumeModifiers()) },
                        onKey = { key, modifiers ->
                            session.sendKey(key, modifiers or consumeModifiers())
                        },
                    )
                }
            }

            // The keys a terminal needs and a phone's keyboard does not have.
            // Only while the keyboard is up: they are meaningless without one,
            // and a permanent row would cost the grid three lines it needs
            // more.
            if (imeVisible && live?.isAgentPane != true) {
                TerminalKeyRow(
                    ctrlArmed = ctrlArmed,
                    altArmed = altArmed,
                    onToggleCtrl = { ctrlArmed = !ctrlArmed },
                    onToggleAlt = { altArmed = !altArmed },
                    onKey = { key -> session.sendKey(key, consumeModifiers()) },
                    onDismiss = { dismissRequest += 1 },
                )
            }

            TerminalTabStrip(
                entries = entries,
                showRunner = connections.size > 1,
                current = ref,
                onSelect = { model.open(it) },
                modifier = Modifier.navigationBarsPadding(),
            )
        }
    }

    // The link a long press landed on. Titled with the URL itself, because
    // "Open Link" without saying which link asks you to trust output an agent
    // produced without showing you what you are trusting.
    heldLink?.let { link ->
        val uri = LocalUriHandler.current
        AlertDialog(
            onDismissRequest = { heldLink = null },
            title = { Text("Link") },
            text = { Text(link) },
            confirmButton = {
                TextButton(onClick = {
                    heldLink = null
                    runCatching { uri.openUri(link) }
                }) { Text("Open") }
            },
            dismissButton = {
                TextButton(onClick = {
                    heldLink = null
                    scope.launch { clipboard.writeText("Far Cooler link", link) }
                }) { Text("Copy") }
            },
        )
    }
}

@Composable
private fun TerminalSurface(
    session: TerminalSession,
    name: String,
    fontFamily: FontFamily,
    fontSize: Float,
    onTap: () -> Unit,
    onLongPress: (column: Int, row: Int) -> Unit,
) {
    val phase by session.phase.collectAsStateWithLifecycle()
    val grid by session.grid.collectAsStateWithLifecycle()

    when (val current = phase) {
        is TerminalSession.Phase.Connecting -> Status(spinner = true, title = "Loading $name…")

        is TerminalSession.Phase.NotLive -> Status(
            title = "Not live",
            message = "$name has no running pane right now.",
        )

        is TerminalSession.Phase.Failed -> Status(
            title = "Could not load",
            message = current.message,
        )

        is TerminalSession.Phase.Live -> {
            val current = grid
            if (current == null) {
                Status(spinner = true, title = "Loading $name…")
            } else {
                TerminalCanvas(
                    grid = current,
                    fontFamily = fontFamily,
                    fontSize = fontSize,
                    onSize = { columns, rows -> session.configure(columns, rows) },
                    onTap = onTap,
                    onLongPress = onLongPress,
                    onScroll = { lines, column, row -> session.scroll(lines, column, row) },
                )
            }
        }
    }
}

@Composable
private fun Status(spinner: Boolean = false, title: String, message: String? = null) {
    Box(
        Modifier.fillMaxSize().background(Color(TerminalPalette.BACKGROUND)),
        contentAlignment = Alignment.Center,
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            if (spinner) {
                CircularProgressIndicator(Modifier.size(28.dp), strokeWidth = 2.dp)
                Spacer(Modifier.height(12.dp))
            }
            Text(title, style = MaterialTheme.typography.titleMedium, color = Color.White)
            if (message != null) {
                Spacer(Modifier.height(6.dp))
                Text(
                    message,
                    style = MaterialTheme.typography.bodyMedium,
                    color = Color.White.copy(alpha = 0.7f),
                    textAlign = TextAlign.Center,
                    modifier = Modifier.padding(horizontal = 32.dp),
                )
            }
        }
    }
}
