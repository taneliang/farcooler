package com.farcooler.ui

import androidx.compose.material.icons.outlined.Circle
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.FolderOpen
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.model.GlanceMarkSize
import com.farcooler.model.GlancePalette
import com.farcooler.model.NeedsYouRunner
import com.farcooler.model.RepositoryWorkspaces
import com.farcooler.model.RunnerLink
import com.farcooler.model.WorkspaceRow
import com.farcooler.net.Connection

// The Workspaces list: each runner's repositories, each repository's
// workspaces with their orchestrators and counts, then its Unclaimed and
// Hidden worktrees (spec §6). Drawn twice — under the items on Needs You, and
// in the drawer — from one set of composables, so the two can't disagree.

/**
 * Every connected runner as the front door reads it: its reading, its fleet
 * and its repositories, collected here so a screen subscribes once.
 */
@Composable
fun rememberNeedsYouRunners(connections: List<Connection>): List<Pair<Connection, NeedsYouRunner>> =
    connections.map { connection ->
        key(connection.host.id) {
            val reading by connection.needsYou.collectAsStateWithLifecycle()
            val fleet by connection.fleet.collectAsStateWithLifecycle()
            val repositories by connection.repositories.collectAsStateWithLifecycle()
            val link by connection.link.collectAsStateWithLifecycle()
            connection to NeedsYouRunner(
                hostId = connection.host.id,
                label = connection.host.displayLabel,
                reading = reading,
                fleet = fleet,
                repositories = repositories,
                answering = link == RunnerLink.ANSWERING,
            )
        }
    }

/**
 * The list's rows: a heading per repository ("overnight", and its runner when
 * there are several), its workspaces, then Unclaimed and Hidden when there are
 * any.
 */
fun LazyListScope.workspaceItems(
    sections: List<Pair<Connection, RepositoryWorkspaces>>,
    namesRunners: Boolean,
    onOpenWorkspace: (WorkspaceRow) -> Unit,
    onOpenWorktrees: (hostId: String, repository: String, hidden: Boolean) -> Unit,
) {
    for ((connection, section) in sections) {
        item(key = "repository/${section.key}") {
            RepositoryHeading(section.name, if (namesRunners) connection.host.displayLabel else null)
        }
        items(section.workspaces, key = { "workspace/${it.key}" }) { row ->
            val link by connection.link.collectAsStateWithLifecycle()
            WorkspaceListRow(row, answering = link == RunnerLink.ANSWERING, onOpen = { onOpenWorkspace(row) })
        }
        if (section.unclaimed.isNotEmpty()) {
            item(key = "unclaimed/${section.key}") {
                ListItem(
                    headlineContent = { Text("Unclaimed") },
                    supportingContent = { Text(worktreeCount(section.unclaimed.size)) },
                    leadingContent = {
                        Icon(Icons.Outlined.FolderOpen, null, Modifier.size(20.dp),
                            tint = MaterialTheme.colorScheme.onSurfaceVariant)
                    },
                    trailingContent = { if (section.unclaimedCount > 0) Count(section.unclaimedCount) },
                    modifier = Modifier
                        .clickable { onOpenWorktrees(section.hostId, section.repository, false) }
                        .testTag("unclaimed-${section.key}"),
                )
            }
        }
        if (section.hidden.isNotEmpty()) {
            item(key = "hidden/${section.key}") {
                ListItem(
                    headlineContent = { Text("Hidden") },
                    supportingContent = { Text(worktreeCount(section.hidden.size)) },
                    leadingContent = {
                        Icon(Icons.Outlined.VisibilityOff, null, Modifier.size(20.dp),
                            tint = MaterialTheme.colorScheme.onSurfaceVariant)
                    },
                    modifier = Modifier
                        .clickable { onOpenWorktrees(section.hostId, section.repository, true) }
                        .testTag("hidden-${section.key}"),
                )
            }
        }
    }
}

/** Each connected runner's shared folders as its build says them, collected once. */
@Composable
fun rememberSharedFolders(connections: List<Connection>): List<Pair<Connection, List<String>>> =
    connections.map { connection ->
        key(connection.host.id) {
            val build by connection.daemon.collectAsStateWithLifecycle()
            connection to build?.sharedFolders.orEmpty()
        }
    }

/**
 * Each runner's extra read-only folders, by name (ov-232, ov-259): the logs and
 * notes its owner chose to share, never addable from here. A heading and a row
 * each, from a runner that has any and that this grant may read; nothing from
 * a runner without. Drawn on Needs You and in the drawer beside the workspaces.
 */
fun LazyListScope.folderItems(
    runners: List<Pair<Connection, List<String>>>,
    namesRunners: Boolean,
    onOpenFolder: (hostId: String, name: String) -> Unit,
) {
    for ((connection, names) in runners) {
        if (names.isEmpty()) continue
        item(key = "folders/${connection.host.id}") {
            RepositoryHeading(
                "Folders", if (namesRunners) connection.host.displayLabel else null)
        }
        items(names, key = { "folder/${connection.host.id}/$it" }) { name ->
            ListItem(
                headlineContent = { Text(name) },
                leadingContent = {
                    Icon(Icons.Outlined.Folder, null, Modifier.size(20.dp),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant)
                },
                modifier = Modifier
                    .clickable { onOpenFolder(connection.host.id, name) }
                    .testTag("folder-row-$name"),
            )
        }
    }
}

/** "1 worktree", "3 worktrees". */
fun worktreeCount(count: Int): String = if (count == 1) "1 worktree" else "$count worktrees"

@Composable
private fun RepositoryHeading(name: String, runner: String?) {
    Text(
        listOfNotNull(name, runner).joinToString(" · "),
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
        modifier = Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surface)
            .padding(start = 16.dp, end = 16.dp, top = 20.dp, bottom = 4.dp),
    )
}

/**
 * One workspace: its name, its orchestrator's mark (a hollow ring when none
 * is running), and its count in amber when anything needs you there.
 */
@Composable
fun WorkspaceListRow(row: WorkspaceRow, answering: Boolean, onOpen: () -> Unit) {
    val orchestrator = row.orchestrator
    ListItem(
        headlineContent = { Text(row.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        leadingContent = {
            Box(Modifier.size(20.dp), contentAlignment = Alignment.Center) {
                if (orchestrator != null) {
                    AgentMarkView(
                        terminal = orchestrator,
                        size = GlanceMarkSize.ROW,
                        decorative = true,
                        answering = answering,
                    )
                } else {
                    Icon(
                        Icons.Outlined.Circle,
                        contentDescription = null,
                        modifier = Modifier.size(12.dp),
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        },
        trailingContent = { if (row.count > 0) Count(row.count) },
        modifier = Modifier
            .clickable(onClick = onOpen)
            .testTag("workspace-row-${row.key}")
            .semantics(mergeDescendants = true) {
                contentDescription = listOfNotNull(
                    row.name,
                    if (orchestrator == null) "No orchestrator" else "Orchestrator ${orchestrator.activityLabel}",
                    row.count.takeIf { it > 0 }?.let { if (it == 1) "1 needs you" else "$it need you" },
                ).joinToString(", ")
            },
    )
}

@Composable
private fun Count(count: Int) {
    Text(
        "$count",
        style = MaterialTheme.typography.labelLarge,
        fontFamily = FontFamily.Monospace,
        fontWeight = FontWeight.SemiBold,
        color = glanceColor(GlancePalette.amber),
    )
}

/** The title of one repository's Unclaimed or Hidden worktrees. */
fun worktreesTitle(hidden: Boolean): String = if (hidden) "Hidden" else "Unclaimed"
