package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
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
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.core.TerminalPalette
import com.farcooler.core.Vt
import com.farcooler.model.Workspace
import com.farcooler.net.Connection
import com.farcooler.net.TerminalRef
import com.farcooler.net.TerminalSession
import kotlinx.coroutines.launch

/**
 * One terminal, live — or the chat behind the same pane.
 *
 * Which of the two is drawn comes from `terminal.isAgentPane`, which the daemon
 * sets. Never derived here, the same rule that keeps activity and agent mode as
 * reported rather than guessed at.
 *
 * ## One session per pane, and it is not re-pointed
 *
 * This was `TerminalScreen`: ONE composable for a whole workspace, holding one
 * [TerminalSession] built with `remember(ref.hostId)` and pointed at whichever
 * terminal the fleet-wide tab strip had last selected. F-3 in the parity
 * inventory. Every tab tap tore down the outgoing pane's emulator, its screen
 * and its stream, so the tab you came back to was one that had never been open —
 * "Loading…" on a pane you already had, a transcript starting at the top, and a
 * terminal renegotiating its size with tmux, which is the content jumping around
 * as it appears.
 *
 * Now this composable IS one pane, one session, mounted for as long as the pane
 * is mounted, and [WorkspaceScreen] holds however many of them the deck says. The
 * session's id is fixed at construction and `TerminalSession.switchTo` is gone.
 *
 * ## [live] is what this pane costs the runner
 *
 * Mounted is not the same as being read. A hidden tab, and every tab while the
 * app is backgrounded, holds no SSH stream, runs no agent poll and asserts no
 * tmux geometry — see `TerminalSession.stop` and `resume`, and `AgentScreen`.
 * Without that split, three open tabs in a pocket would be three second
 * channels.
 *
 * The tab strip lives at the BOTTOM, in thumb reach, and rises with the
 * keyboard. It belongs to [WorkspaceScreen] now, because it is the workspace's
 * strip rather than this pane's — but the position and the reason for it are
 * unchanged: under the title bar it would put the one control you use constantly
 * at the far end of the screen from the hand holding the phone.
 */
@Composable
fun TerminalPane(
    model: AppModel,
    ref: TerminalRef,
    connection: Connection,
    workspace: Workspace?,
    showRunner: Boolean,
    /**
     * Whether this pane is the one being read: the current tab, in an app that
     * is in the foreground.
     *
     * Everything that costs the host follows this and only this.
     */
    live: Boolean,
    onPickImage: () -> Unit,
    onOpenDrawer: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val clipboard = LocalClipboard.current

    // The URL a long press landed on, which is also what shows the sheet: a
    // dialog that can be presented with nothing to present is one that
    // eventually will be.
    var heldLink by remember { mutableStateOf<String?>(null) }

    val fontChoice by model.settings.font.collectAsStateWithLifecycle()
    val fontSize by model.settings.fontSize.collectAsStateWithLifecycle()
    val reshape by model.settings.reshapePanes.collectAsStateWithLifecycle()

    // The terminal as the daemon describes it RIGHT NOW.
    //
    // A stored copy is right for identity and wrong for anything that changes
    // underneath it — pane mode above all. Switching to chat left an iOS copy
    // still saying "terminal", so the button asked for the same switch every
    // time and the screen kept drawing a VT grid.
    val terminal = model.fleet.terminal(ref)
    val ordinal = workspace?.ordinals()?.get(ref.terminalId)
    val name = terminal?.displayName(ordinal) ?: "Terminal"

    // Keyed to nothing that can change. A pane's id is fixed for the life of
    // this composable — [WorkspaceScreen] gives each one its own `key` — so
    // there is nothing left for a `remember` key to guard against, and anything
    // put in one would be a way for this session to be rebuilt without the
    // pane being.
    val session = remember { TerminalSession(ref.terminalId, connection.core) }
    LaunchedEffect(reshape) { session.reshapeAllowed = reshape }

    // What this pane costs while nobody is reading it: nothing. `resume` and
    // not `relink` — relinking drops the emulator and puts "Loading…" over a tab
    // you had already opened, which is the exact opposite of what mounting
    // hidden panes is for. See `TerminalSession.resume`.
    LaunchedEffect(live) {
        if (live) session.resume() else session.stop()
    }

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
    // everything this pane had open went with it. [TerminalSession] survives
    // that on its own by falling back to polling, which is the right behaviour
    // and the slower path; this puts it back on the stream now that there is
    // one to be on. Skipped on the first composition, where the count is
    // whatever it already was and nothing has been replaced.
    //
    // A pane nobody is reading does not relink and does not need to: it holds no
    // channel to have lost, and `resume` opens a fresh one against whatever link
    // exists by then. It still records the count, or coming back to it would
    // relink a session that had just been opened.
    val reconnects by connection.reconnects.collectAsStateWithLifecycle()
    var seenReconnects by remember { mutableIntStateOf(reconnects) }
    LaunchedEffect(reconnects, live) {
        if (reconnects == seenReconnects) return@LaunchedEffect
        seenReconnects = reconnects
        if (live) session.relink()
    }
    DisposableEffect(session) {
        onDispose { session.dispose() }
    }

    var showMenu by remember { mutableStateOf(false) }
    var focusRequest by remember { mutableIntStateOf(0) }
    var dismissRequest by remember { mutableIntStateOf(0) }
    var ctrlArmed by remember { mutableStateOf(false) }
    var altArmed by remember { mutableStateOf(false) }

    // A dialog is a window, not part of this pane's layout, so a hidden pane
    // holding one would put it over whatever tab you had switched to. Dropped
    // rather than merely hidden: coming back to a pane and being asked again
    // about a link you long-pressed ten minutes ago is not resuming anything.
    LaunchedEffect(live) {
        if (!live) heldLink = null
    }

    // The software keyboard belongs to whichever pane is being read. The
    // terminal's is attached to a real Android `View`, which Compose's focus
    // manager cannot reach — so a pane going quiet has to put its own keyboard
    // away, or the next thing typed would be delivered to a terminal nobody can
    // see. The agent composer's field is Compose's and is cleared by the screen.
    LaunchedEffect(live) {
        if (!live) dismissRequest += 1
    }

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

    Column(
        Modifier
            .fillMaxSize()
            .background(Color(TerminalPalette.BACKGROUND))
    ) {
        WorkspaceTopBar(
            workspace = workspace,
            fallbackTitle = name,
            showRunner = showRunner,
            runnerLabel = connection.host.displayLabel,
            onOpenDrawer = onOpenDrawer,
        ) {
            // Terminal or chat, on the pane that can be either. Shown only
            // where it would work: `chatCapable` already reflects the daemon's
            // registry-backed check, so a pane whose agent has no adapter never
            // gets the button in the first place.
            if (terminal?.canSwitchPaneMode == true) {
                IconButton(onClick = {
                    scope.launch {
                        connection.setPaneMode(
                            terminal,
                            if (terminal.isAgentPane) "terminal" else "agent",
                        )
                    }
                }) {
                    Icon(
                        if (terminal.isAgentPane) Icons.Outlined.Terminal
                        else Icons.AutoMirrored.Filled.Chat,
                        contentDescription =
                            if (terminal.isAgentPane) "Show the terminal" else "Show the chat",
                    )
                }
            }
            Box {
                IconButton(onClick = { showMenu = true }) {
                    Icon(Icons.Filled.MoreVert, contentDescription = "More")
                }
                DropdownMenu(showMenu, onDismissRequest = { showMenu = false }) {
                    if (terminal?.isAgentPane != true) {
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
                            text = { Text("Send image") },
                            leadingIcon = { Icon(Icons.Filled.Image, null) },
                            onClick = {
                                showMenu = false
                                onPickImage()
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
        }

        Box(Modifier.weight(1f)) {
            if (terminal?.isAgentPane == true) {
                AgentScreen(
                    model = model,
                    ref = ref,
                    connection = connection,
                    live = live,
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
        //
        // Only while the keyboard is up: they are meaningless without one, and a
        // permanent row would cost the grid three lines it needs more.
        //
        // And only on the pane being read, which is new and is not cosmetic. The
        // IME is a fact about the WINDOW, so a keyboard raised for the agent
        // composer on the tab in front raises it for every mounted pane behind
        // it — each of which would reserve a key row nobody can press, measure
        // its canvas three lines short, and re-assert that wrong shape to tmux
        // the moment it was resumed.
        if (live && imeVisible && terminal?.isAgentPane != true) {
            TerminalKeyRow(
                ctrlArmed = ctrlArmed,
                altArmed = altArmed,
                onToggleCtrl = { ctrlArmed = !ctrlArmed },
                onToggleAlt = { altArmed = !altArmed },
                onKey = { key -> session.sendKey(key, consumeModifiers()) },
                onDismiss = { dismissRequest += 1 },
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

/**
 * The workspace's own title bar, worn by every tab.
 *
 * Drawn per pane rather than once above them, which is the one place this
 * screen deliberately differs from `WorkspaceView`. iOS puts its toolbar on the
 * workspace and has to route every tab's controls back up through it — its own
 * comment records what that cost when `ChangesView` contributed items from below
 * and SwiftUI merged the two trees in a different order per pane. Here the
 * controls belong to the tab that owns them: the terminal's overflow menu needs
 * that pane's session, and nothing has to be published upward for it.
 *
 * What it costs is a `TopAppBar` composed per mounted tab, which is three cheap
 * ones, all but one of them behind `alpha(0f)` and never recomposed because the
 * tab behind it is frozen.
 *
 * The task and its branch, not the tab's own name — a terminal already names
 * itself in its chip, and repeating it here would spend the one line of title
 * bar a phone has on something already on screen. The runner appears only when
 * more than one is connected.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun WorkspaceTopBar(
    workspace: Workspace?,
    fallbackTitle: String,
    showRunner: Boolean,
    runnerLabel: String,
    onOpenDrawer: () -> Unit,
    actions: @Composable RowScope.() -> Unit = {},
) {
    TopAppBar(
        colors = TopAppBarDefaults.topAppBarColors(
            containerColor = Color(TerminalPalette.BACKGROUND),
        ),
        title = {
            Column {
                Text(
                    workspace?.task?.ifBlank { null } ?: fallbackTitle,
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                val subtitle = buildString {
                    workspace?.branch?.takeIf { it.isNotBlank() }?.let { append(it) }
                    if (showRunner) {
                        if (isNotEmpty()) append(" · ")
                        append(runnerLabel)
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
        actions = actions,
    )
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
            transcript = current.transcript,
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

/**
 * [message] is prose this app wrote; [transcript] is what the host said.
 *
 * Drawn as two different kinds of thing on purpose. The host's answer used to
 * arrive as [message] and be set in the same line, so a lowercase fragment off
 * an SSH channel read as Far Cooler's own account of the pane. Nothing is
 * dropped — those words are the whole diagnosis of a pane nobody can read — it
 * simply goes where output goes.
 */
@Composable
private fun Status(
    spinner: Boolean = false,
    title: String,
    message: String? = null,
    transcript: String? = null,
) {
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
            // Bounded to a readable column rather than the full width of a
            // landscape phone, and left-aligned inside itself: a transcript
            // centered with the prose above it stops looking like output.
            if (transcript != null) {
                Spacer(Modifier.height(12.dp))
                DetailBox(
                    transcript,
                    modifier = Modifier.padding(horizontal = 32.dp).widthIn(max = 360.dp),
                )
            }
        }
    }
}
