package com.farcooler.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.farcooler.core.TerminalGrid
import com.farcooler.core.VtCore
import com.farcooler.data.Theme
import com.farcooler.model.AdapterInfo
import com.farcooler.model.AdapterTestOutcome
import com.farcooler.model.envToLines
import com.farcooler.model.linesToEnv
import com.farcooler.model.linesToList
import kotlinx.coroutines.launch

/**
 * Nineteen colours, over a terminal actually rendering them.
 *
 * All nineteen on a phone, which was the deliberate choice rather than the easy
 * one: the config format treats the sixteen ANSI colours as optional, so a
 * grounds-only editor would have been a first-class config and less work — and
 * would have sent you back to ssh for the other sixteen.
 *
 * Each colour is three sliders rather than a picker, because Android ships no
 * colour picker and a hand-rolled wheel is a lot of surface to get subtly wrong.
 * Sliders are dull and exact, and the preview above answers the only question
 * that actually matters.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ThemeEditorScreen(theme: Theme, onCancel: () -> Unit, onSave: (Theme) -> Unit) {
    var draft by remember(theme.name) { mutableStateOf(theme) }
    var expanded by remember { mutableStateOf<Int?>(null) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(if (theme.name.isBlank()) "New theme" else theme.name) },
                navigationIcon = { TextButton(onClick = onCancel) { Text("Cancel") } },
                actions = {
                    TextButton(onClick = { onSave(draft) }, enabled = draft.name.isNotBlank()) {
                        Text("Save")
                    }
                },
            )
        },
    ) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()),
        ) {
            ThemePreview(draft, Modifier.fillMaxWidth().height(150.dp))

            Column(
                Modifier.padding(horizontal = 20.dp, vertical = 12.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                OutlinedTextField(
                    value = draft.name,
                    onValueChange = { draft = draft.copy(name = it) },
                    label = { Text("Name") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text("Dark surfaces around the terminal", Modifier.weight(1f))
                    Switch(
                        checked = draft.dark,
                        onCheckedChange = { draft = draft.copy(dark = it) },
                    )
                }
                Text(
                    "Carried with the theme rather than guessed from the background, so a " +
                        "mid-grey ground is yours to decide about.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )

                HorizontalDivider()
                SectionTitle("Ground")
                ColorRow("Background", draft.background, expanded == GROUND_BACKGROUND, {
                    expanded = if (expanded == GROUND_BACKGROUND) null else GROUND_BACKGROUND
                }) { draft = draft.copy(background = it) }
                ColorRow("Text", draft.foreground, expanded == GROUND_TEXT, {
                    expanded = if (expanded == GROUND_TEXT) null else GROUND_TEXT
                }) { draft = draft.copy(foreground = it) }
                ColorRow("Cursor", draft.cursor, expanded == GROUND_CURSOR, {
                    expanded = if (expanded == GROUND_CURSOR) null else GROUND_CURSOR
                }) { draft = draft.copy(cursor = it) }

                HorizontalDivider()
                SectionTitle("Normal")
                for (index in 0..7) {
                    ansiRow(index, ANSI_NAMES[index], draft, expanded, { expanded = it }) {
                        draft = it
                    }
                }

                HorizontalDivider()
                SectionTitle("Bright")
                for (index in 8..15) {
                    ansiRow(
                        index, "Bright ${ANSI_NAMES[index - 8]}", draft, expanded,
                        { expanded = it },
                    ) { draft = it }
                }
                Text(
                    "Colours above these sixteen are arithmetic every terminal agrees on, so no " +
                        "theme sets them.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

// Negative, so the three grounds cannot collide with an ANSI index.
private const val GROUND_BACKGROUND = -1
private const val GROUND_TEXT = -2
private const val GROUND_CURSOR = -3

/** ANSI's own names, which are what an escape sequence actually means. */
private val ANSI_NAMES =
    listOf("Black", "Red", "Green", "Yellow", "Blue", "Magenta", "Cyan", "White")

@Composable
private fun ansiRow(
    index: Int,
    label: String,
    draft: Theme,
    expanded: Int?,
    onExpand: (Int?) -> Unit,
    onChange: (Theme) -> Unit,
) {
    ColorRow(
        label, draft.ansi[index], expanded == index,
        { onExpand(if (expanded == index) null else index) },
    ) { packed ->
        onChange(draft.copy(ansi = draft.ansi.toMutableList().also { it[index] = packed }))
    }
}

/** One colour: a swatch and its hex, which opens three channel sliders. */
@Composable
private fun ColorRow(
    label: String,
    packed: Int,
    open: Boolean,
    onToggle: () -> Unit,
    onChange: (Int) -> Unit,
) {
    Column {
        Row(
            Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Box(Modifier.size(24.dp).background(Color(OPAQUE or packed)))
            Text(label, Modifier.weight(1f))
            Text(
                String.format("#%06X", packed and 0xFFFFFF),
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            TextButton(onClick = onToggle) { Text(if (open) "Done" else "Change") }
        }
        if (open) {
            Channel("R", (packed shr 16) and 0xFF) {
                onChange((packed and 0x00FFFF) or (it shl 16))
            }
            Channel("G", (packed shr 8) and 0xFF) {
                onChange((packed and 0xFF00FF) or (it shl 8))
            }
            Channel("B", packed and 0xFF) { onChange((packed and 0xFFFF00) or it) }
        }
    }
}

@Composable
private fun Channel(name: String, value: Int, onChange: (Int) -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Text(name, Modifier.width(18.dp), style = MaterialTheme.typography.bodySmall)
        Slider(
            value = value.toFloat(),
            onValueChange = { onChange(it.toInt().coerceIn(0, 255)) },
            valueRange = 0f..255f,
            modifier = Modifier.weight(1f),
        )
        Text(
            value.toString(),
            Modifier.width(32.dp),
            style = MaterialTheme.typography.bodySmall,
            fontFamily = FontFamily.Monospace,
        )
    }
}

/** Opaque alpha, since the grid's colours carry none. */
private const val OPAQUE = 0xFF000000.toInt()

/**
 * A terminal rendering a fixture in the theme being edited.
 *
 * Through the same [VtCore] a live pane uses, fed the same kind of bytes. Cell
 * colours are resolved inside that core precisely so three renderers cannot
 * drift; a hand-drawn preview here would be a fourth drifting from all of them.
 */
@Composable
private fun ThemePreview(theme: Theme, modifier: Modifier = Modifier) {
    // Rebuilt on every change rather than recoloured in place: the fixture is a
    // few hundred bytes, and a fresh core cannot carry state from a palette that
    // is no longer chosen. Freed immediately — this core outlives nothing.
    val grid = remember(theme) {
        val core = VtCore(columns = 52, rows = 9)
        core.setPalette(theme.packed())
        core.feed(PREVIEW_FIXTURE.toByteArray())
        val snapshot = core.snapshot()
        core.free()
        snapshot
    }
    Box(modifier.background(Color(OPAQUE or theme.background))) {
        grid?.let { PreviewCanvas(it) }
    }
}

/**
 * The grid as coloured blocks.
 *
 * Backgrounds only, deliberately: at this size a glyph is four pixels tall and
 * unreadable, and what the eye is actually judging is whether these colours sit
 * together. The fixture's last two rows are solid blocks of all sixteen for
 * exactly that reason.
 */
@Composable
private fun PreviewCanvas(grid: TerminalGrid) {
    Canvas(Modifier.fillMaxSize()) {
        if (grid.columns <= 0 || grid.rows <= 0) return@Canvas
        val cellWidth = size.width / grid.columns
        val cellHeight = size.height / grid.rows
        for (row in 0 until grid.rows) {
            for (column in 0 until grid.columns) {
                // A written cell shows its text colour; a blank one shows the
                // ground. Without that, every row would be one flat rectangle.
                val packed =
                    if (grid.character(row, column) > 32) grid.foreground(row, column)
                    else grid.background(row, column)
                drawRect(
                    color = Color(packed),
                    topLeft = Offset(column * cellWidth, row * cellHeight),
                    size = Size(cellWidth, cellHeight),
                )
            }
        }
    }
}

/**
 * Output chosen to exercise what a theme has to get right: every colour, bold, a
 * prompt, a diff, and an agent waiting on you.
 */
private val PREVIEW_FIXTURE: String = buildString {
    val esc = "\u001B"
    append("$esc[H$esc[2J")
    append("$esc[1;32m~/project$esc[0m $esc[1;34mmain$esc[0m $ claude\r\n")
    append("$esc[2m? for shortcuts$esc[0m\r\n")
    append("$esc[1;35m*$esc[0m Editing src/main.rs\r\n")
    append("  $esc[32m+ let theme = Theme::from(config);$esc[0m\r\n")
    append("  $esc[31m- let theme = Theme::default();$esc[0m\r\n")
    append("$esc[33m!$esc[0m $esc[1mDo you want to make this edit?$esc[0m\r\n")
    append("  $esc[36m> 1. Yes$esc[0m\r\n")
    append(" ")
    for (code in 30..37) append("$esc[${code}m###$esc[0m")
    append("\r\n ")
    for (code in 90..97) append("$esc[${code}m###$esc[0m")
}

/**
 * One agent's adapter, with a button that proves the launch half works.
 *
 * Grouped Launch and Detection, and that split is the point: Test starts the
 * adapter and completes an ACP handshake, so it proves Launch. It cannot prove
 * Detection — those strings are matched against output only that agent produces,
 * and a wrong one does not fail, it stops the agent being recognized.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AdapterEditorScreen(
    adapter: AdapterInfo,
    isNew: Boolean,
    onTest: suspend (AdapterInfo) -> AdapterTestOutcome,
    onCancel: () -> Unit,
    onSave: (AdapterInfo) -> Unit,
) {
    val scope = rememberCoroutineScope()
    var preset by remember { mutableStateOf(adapter.preset) }
    var program by remember { mutableStateOf(adapter.program) }
    var argsText by remember { mutableStateOf(adapter.args.joinToString("\n")) }
    var envText by remember { mutableStateOf(envToLines(adapter.env)) }
    var commandsText by remember { mutableStateOf(adapter.commands.joinToString("\n")) }
    var identityText by remember { mutableStateOf(adapter.identity.joinToString("\n")) }
    var blockedText by remember { mutableStateOf(adapter.blocked.joinToString("\n")) }
    var workingText by remember { mutableStateOf(adapter.working.joinToString("\n")) }
    var testing by remember { mutableStateOf(false) }
    var outcome by remember { mutableStateOf<AdapterTestOutcome?>(null) }

    fun assembled() = adapter.copy(
        preset = preset.trim(),
        program = program.trim(),
        args = linesToList(argsText),
        env = linesToEnv(envText),
        commands = linesToList(commandsText),
        identity = linesToList(identityText),
        blocked = linesToList(blockedText),
        working = linesToList(workingText),
    )

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(if (isNew) "New agent" else adapter.preset) },
                navigationIcon = { TextButton(onClick = onCancel) { Text("Cancel") } },
                actions = {
                    TextButton(
                        onClick = { onSave(assembled()) },
                        enabled = preset.isNotBlank() && program.isNotBlank(),
                    ) { Text("Save") }
                },
            )
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            if (adapter.origin == "builtIn") {
                Text(
                    "Saving this writes an override on that machine. You can revert to the " +
                        "shipped one at any time.",
                    style = MaterialTheme.typography.bodySmall,
                )
            }

            OutlinedTextField(
                value = preset,
                onValueChange = { preset = it },
                label = { Text("Name") },
                singleLine = true,
                enabled = isNew,
                modifier = Modifier.fillMaxWidth(),
            )
            if (!isNew) {
                Text(
                    "The name identifies this agent in the config file and is matched against " +
                        "the process in a pane, so it is fixed once it exists.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            HorizontalDivider()
            SectionTitle("Launch")
            OutlinedTextField(
                value = program,
                onValueChange = { program = it },
                label = { Text("Program") },
                placeholder = { Text("npx") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            Multiline("Arguments", argsText) { argsText = it }
            Multiline("Environment (KEY=value)", envText) { envText = it }
            Text(
                "One per line. This is the half Test can prove.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            HorizontalDivider()
            SectionTitle("Detection")
            Multiline("Process names", commandsText) { commandsText = it }
            Multiline("Identity", identityText) { identityText = it }
            Multiline("Waiting for you", blockedText) { blockedText = it }
            Multiline("Working", workingText) { workingText = it }
            Text(
                "How Far Cooler recognizes this agent on a screen. Test cannot check these — a " +
                    "wrong value does not fail, the agent simply stops being recognized and its " +
                    "notifications stop arriving.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
            )

            HorizontalDivider()
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Button(
                    onClick = {
                        scope.launch {
                            testing = true
                            outcome = onTest(assembled())
                            testing = false
                        }
                    },
                    enabled = !testing && program.isNotBlank(),
                ) { Text(if (testing) "Testing…" else "Test") }
                if (testing) {
                    CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                }
            }
            outcome?.let {
                when (it) {
                    is AdapterTestOutcome.Worked ->
                        Text(
                            "Starts and speaks ACP — ${it.reported}",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.primary,
                        )
                    is AdapterTestOutcome.Failed ->
                        Text(
                            it.why,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error,
                        )
                }
            }
        }
    }
}

@Composable
private fun Multiline(label: String, value: String, onChange: (String) -> Unit) {
    OutlinedTextField(
        value = value,
        onValueChange = onChange,
        label = { Text(label) },
        minLines = 2,
        maxLines = 5,
        modifier = Modifier.fillMaxWidth(),
    )
}
