package com.farcooler.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.model.BoardHistory
import com.farcooler.model.TaskStatus
import com.farcooler.net.Connection

/**
 * A finished status's History page (ov-103), pushed from the board's "All
 * Done" row: every task in Done or Canceled, grouped Today, Yesterday, This
 * Week and Earlier, searchable by key and title, narrowed by area chips. The
 * Mac's page searches note text too; a phone can't ask the runner for that yet.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun BoardHistoryScreen(
    connection: Connection,
    workspaceId: String,
    status: TaskStatus,
    onOpenTask: (taskId: String) -> Unit,
    onBack: () -> Unit,
) {
    val boards by connection.boards.collectAsStateWithLifecycle()
    var query by rememberSaveable { mutableStateOf("") }
    var area by rememberSaveable { mutableStateOf<String?>(null) }
    val all = boards[workspaceId]?.columns?.firstOrNull { it.status == status }?.rows.orEmpty()
    val found = BoardHistory.filter(all, query, area)
    val now = System.currentTimeMillis()
    val areas = BoardHistory.areas(all)

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(status.title) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
            )
        },
    ) { padding ->
        LazyColumn(Modifier.padding(padding).fillMaxSize().testTag("board-history")) {
            item(key = "search") {
                OutlinedTextField(
                    value = query,
                    onValueChange = { query = it },
                    placeholder = { Text("Search keys and titles") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)
                        .testTag("history-search"),
                )
            }
            if (areas.size > 1) {
                item(key = "areas") {
                    LazyRow(
                        contentPadding = PaddingValues(horizontal = 16.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        items(areas, key = { it }) { name ->
                            FilterChip(
                                selected = area == name,
                                onClick = { area = if (area == name) null else name },
                                label = { Text(name) },
                                modifier = Modifier.testTag("history-area-$name"),
                            )
                        }
                    }
                }
            }
            if (found.isEmpty()) {
                item(key = "empty") {
                    Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) {
                        Text(
                            if (all.isEmpty()) "Nothing here yet." else "No tasks match.",
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
            for (group in BoardHistory.groups(found, now)) {
                item(key = "period/${group.period.name}") {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp),
                    ) {
                        Text(group.period.title, style = MaterialTheme.typography.titleSmall)
                        Spacer(Modifier.weight(1f))
                        Text(
                            "${group.rows.size}",
                            style = MaterialTheme.typography.labelMedium.copy(fontFeatureSettings = "tnum"),
                            color = MaterialTheme.colorScheme.outline,
                        )
                    }
                }
                items(group.rows, key = { "task/${it.id}" }) { row ->
                    ListItem(
                        overlineContent = { Text(row.key, fontFamily = FontFamily.Monospace) },
                        headlineContent = { Text(row.title, maxLines = 2, overflow = TextOverflow.Ellipsis) },
                        supportingContent = { Text(BoardHistory.landed(row, now)) },
                        modifier = Modifier.clickable { onOpenTask(row.id) }.testTag("history-row-${row.key}"),
                    )
                    HorizontalDivider()
                }
            }
        }
    }
}
