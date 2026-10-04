package com.farcooler.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.InsertDriveFile
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.Link
import androidx.compose.material.icons.outlined.HelpOutline
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.farcooler.core.refusalWord
import com.farcooler.model.FileEntry
import com.farcooler.model.FilesCode
import com.farcooler.model.FilesContent
import com.farcooler.model.FilesDirectory
import com.farcooler.model.FilesExpecting
import com.farcooler.model.FilesLoader
import com.farcooler.model.FilesLocation
import com.farcooler.model.FilesPlace
import com.farcooler.model.CoreFilesSource
import com.farcooler.net.Connection
import kotlinx.coroutines.launch

// The read-only Files browser (ov-259): a worktree's files, or one of the
// runner's extra read-only folders, a screen to a directory and a screen to a
// file. What each screen shows is `FilesLoader`'s, in `model/Files.kt`, where the
// JVM tests reach it; this file only draws. Compose only, with no syntax coloring
// as the Mac's Files has none.

/** The place and path a route names. */
fun Route.Files.location(): FilesLocation = FilesLocation(
    place = folder?.let { FilesPlace.Folder(it) } ?: FilesPlace.Worktree(worktreeId.orEmpty()),
    path = path,
    expecting = FilesExpecting.parse(expecting),
)

/** The route that opens [location] on [hostId]. */
fun FilesLocation.route(hostId: String): Route.Files = Route.Files(
    hostId = hostId,
    worktreeId = (place as? FilesPlace.Worktree)?.id,
    folder = (place as? FilesPlace.Folder)?.name,
    path = path,
    expecting = expecting.wire,
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun FilesScreen(
    connection: Connection,
    route: Route.Files,
    onOpen: (Route.Files) -> Unit,
    onBack: () -> Unit,
) {
    val location = route.location()
    val source = remember(connection) {
        CoreFilesSource(
            call = { method, args ->
                connection.core.call(method, Connection.args(*args.map { it.key to it.value as Any }.toTypedArray()))
            },
            refusalWord = { it.refusalWord },
        )
    }
    var reloads by remember { mutableIntStateOf(0) }
    val content by produceState<FilesContent>(FilesContent.Loading, route, reloads) {
        value = FilesContent.Loading
        value = FilesLoader.load(location, source)
    }
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(location.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = {
                    if (location.path.isNotEmpty()) {
                        IconButton(
                            onClick = { scope.launch { clipboard.writeText("Path", location.path) } },
                            modifier = Modifier.testTag("files-copy-path"),
                        ) {
                            Icon(Icons.Outlined.ContentCopy, contentDescription = "Copy path")
                        }
                    }
                },
            )
        },
    ) { padding ->
        Box(Modifier.padding(padding).fillMaxSize()) {
            FilesContentView(
                content = content,
                onOpen = { onOpen(it.route(route.hostId)) },
                onRetry = { reloads += 1 },
            )
        }
    }
}

/** What a screen shows for [content]. Separate from [FilesScreen] so a capture draws the real thing. */
@Composable
internal fun FilesContentView(
    content: FilesContent,
    onOpen: (FilesLocation) -> Unit,
    onRetry: () -> Unit,
) {
    when (content) {
        FilesContent.Loading -> Box(Modifier.fillMaxSize().testTag("files-loading"), contentAlignment = Alignment.Center) {
            CircularProgressIndicator()
        }
        is FilesContent.Directory -> FilesDirectoryList(content.directory, onOpen)
        is FilesContent.Code -> FilesCodeView(content.code)
        is FilesContent.Message -> FilesNote(content.words)
        is FilesContent.Link -> FilesNote("This is a link to ${content.target}.") {
            content.destination?.let {
                Button(onClick = { onOpen(it) }, modifier = Modifier.testTag("files-open-link")) { Text("Open") }
            }
        }
        is FilesContent.Failed -> FilesNote(content.sentence) {
            OutlinedButton(onClick = onRetry, modifier = Modifier.testTag("files-try-again")) { Text("Try again") }
        }
    }
}

@Composable
private fun FilesDirectoryList(directory: FilesDirectory, onOpen: (FilesLocation) -> Unit) {
    LazyColumn(Modifier.fillMaxSize().testTag("files-list")) {
        directory.empty?.let {
            item(key = "empty") {
                Text(
                    it, color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(16.dp).testTag("files-empty"),
                )
            }
        }
        itemsIndexed(directory.rows, key = { _, row -> row.key }) { _, row ->
            val destination = row.destination
            ListItem(
                headlineContent = { Text(row.name, maxLines = 1, overflow = TextOverflow.MiddleEllipsis) },
                supportingContent = row.detail.takeIf { it.isNotEmpty() }?.let { detail ->
                    { Text(detail, maxLines = 1, overflow = TextOverflow.MiddleEllipsis) }
                },
                leadingContent = {
                    Icon(
                        when (row.kind) {
                            FileEntry.Kind.DIRECTORY -> Icons.Outlined.Folder
                            FileEntry.Kind.FILE -> Icons.AutoMirrored.Outlined.InsertDriveFile
                            FileEntry.Kind.LINK -> Icons.Outlined.Link
                            FileEntry.Kind.OTHER -> Icons.Outlined.HelpOutline
                        },
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                },
                trailingContent = if (destination != null && row.kind != FileEntry.Kind.FILE) {
                    {
                        Icon(
                            Icons.AutoMirrored.Outlined.KeyboardArrowRight, contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                } else null,
                modifier = Modifier
                    .then(if (destination != null) Modifier.clickable { onOpen(destination) } else Modifier)
                    .testTag("files-row-${row.name}"),
            )
        }
        directory.footer?.let {
            item(key = "footer") {
                Text(
                    it, style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(16.dp).testTag("files-footer"),
                )
            }
        }
    }
}

/**
 * A file's text, numbered, in a monospaced face, never wrapped: one horizontal
 * scroll over a lazy column, whose width is stated from the longest line so the
 * rows don't resize as they scroll by.
 */
@Composable
private fun FilesCodeView(code: FilesCode) {
    val size = 13.sp
    val column = with(LocalDensity.current) { (size * 0.62f).toDp() }
    val gutter = column * code.gutterDigits
    val widest = code.lines.maxOfOrNull { it.length } ?: 0
    Column(Modifier.fillMaxSize().testTag("files-code")) {
        Box(Modifier.weight(1f).horizontalScroll(rememberScrollState())) {
            LazyColumn(Modifier.width(gutter + 12.dp + column * widest + 32.dp)) {
                itemsIndexed(code.lines) { index, line ->
                    Row(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Text(
                            "${index + 1}", fontFamily = FontFamily.Monospace, fontSize = size,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            textAlign = TextAlign.End, modifier = Modifier.width(gutter),
                        )
                        Text(
                            line.ifEmpty { " " }, fontFamily = FontFamily.Monospace, fontSize = size,
                            maxLines = 1, softWrap = false,
                        )
                    }
                }
            }
        }
        if (code.anyCut) {
            Text(
                FilesCode.CUT_NOTE, style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.fillMaxWidth().padding(16.dp).testTag("files-cut-note"),
            )
        }
    }
}

/** A sentence in the middle of the screen, and what to do about it. */
@Composable
private fun FilesNote(words: String, actions: @Composable () -> Unit = {}) {
    Column(
        Modifier.fillMaxSize().padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically),
    ) {
        Text(
            words, textAlign = TextAlign.Center, color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.testTag("files-note"),
        )
        actions()
    }
}
