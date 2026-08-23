package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.outlined.AccountTree
import androidx.compose.material.icons.outlined.Bookmark
import androidx.compose.material.icons.outlined.Build
import androidx.compose.material.icons.outlined.CheckCircleOutline
import androidx.compose.material.icons.outlined.ExpandMore
import androidx.compose.material.icons.outlined.FormatListNumbered
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material.icons.outlined.KeyboardArrowUp
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.UnfoldMore
import androidx.compose.material.icons.outlined.Warning
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.core.TerminalPalette
import com.farcooler.model.ChangeCommit
import com.farcooler.model.ChangedFile
import com.farcooler.model.ChangedFileStatus
import com.farcooler.model.ChangesRow
import com.farcooler.model.ChangesState
import com.farcooler.model.DiffComputation
import com.farcooler.model.DiffLayout
import com.farcooler.model.DiffScope
import com.farcooler.model.ReviewPosition
import com.farcooler.model.ReviewScroll
import com.farcooler.model.Workspace
import com.farcooler.net.ChangesStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch

/**
 * Reviewing what a worktree changed, on a phone.
 *
 * Phase 5b of the parity program, and the body `e23718c` reserved. Phase 5a
 * built the whole model — [ChangesState], [ChangesStore], [DiffLayout],
 * [ReviewPosition] — with nothing to look at; this is the looking. The screen is
 * shaped around the same ninety seconds `apps/ios/FarCooler/ChangesView.swift`
 * is shaped around: one hand, between sets, with the process very likely killed
 * in between, reading a branch an agent wrote overnight. So the controls that
 * move you through a diff are at the BOTTOM where a thumb is, exactly one file
 * is open at a time so that "next" means something, and where you were is
 * written to disk on every move because nothing will be alive to be asked.
 *
 * ## The two places this is not a port, both decided in 5a and honored here
 *
 * **[ChangesState.rows] IS this `LazyColumn`'s item list.** Compose has no
 * `scrollTo(id)`; `animateScrollToItem` takes an INDEX, and an index into a lazy
 * list is only knowable from the list itself. So the model owns the layout,
 * [ChangesState.indexOf] resolves a [com.farcooler.model.Jump]'s key against it,
 * and this file draws `rows` and scrolls by `indexOf` from one value in one
 * recomposition. **The consequence is a rule with no exceptions: this list emits
 * exactly one item per row and never a header, a footer or a spacer item.** A
 * single extra `item {}` anywhere in here silently moves every jump on the
 * screen down by one, and the failure looks like a bookmark that lands on the
 * file after the one it named.
 *
 * **The resume anchor is rebuilt from `layoutInfo`.** iOS hangs
 * `.onScrollVisibilityChange` off each heading; Compose has no such modifier,
 * and [ReviewScroll.topVisible] takes the shape `LazyListState` actually offers
 * instead. See its doc comment for why that shape is the better one.
 *
 * ## What the sticky file name is, and why it is not iOS's
 *
 * iOS pins the heading with `LazyVStack(pinnedViews: [.sectionHeaders])`, which
 * costs it two elements per file — a header and a body. **Two items per file is
 * exactly what the index contract above forbids**, so `stickyHeader` is not
 * available to this screen: a `stickyHeader` holding the whole card would pin
 * the patch as well as its name, and splitting the card in two would put the
 * layout back in two places.
 *
 * So the name is [PinnedFileName], one bar drawn over the list, and the
 * objection `b6e3114` raised against exactly that on iOS does not apply here:
 * it said an overlay would have to be told which file the scroll is inside,
 * which is the question the pinning answers for nothing. This app already
 * computes that answer every frame and hands it to the bookmark. The bar reads
 * the same [ReviewScroll] value, so the name at the top of the screen and the
 * file the bookmark will name cannot disagree.
 *
 * **Its background is opaque, and that is not `b6e3114`'s slab coming back.**
 * `cc1cc25` replaced opacity with `.regularMaterial` on iOS because the right
 * answer to "content must not read through a floating surface" is a blur, and
 * **Compose 1.11 has no backdrop material** — `Modifier.blur` blurs the
 * composable it is applied to, not what passes behind it, and there is no
 * `.regularMaterial`, no `GlassSurface` and no backdrop node to borrow. Doing it
 * by hand means rendering the list into a layer and sampling it every frame of
 * every scroll, which is the cost `ChangesSurface` on iOS already refuses for a
 * cheaper effect. What makes an opaque bar right rather than merely available is
 * that the SHAPE is different: iOS pinned forty card tops, so opacity turned
 * every card top into a slab. This is ONE bar, at the edge of the screen,
 * outside every card — the same object `NeedsYouScreen`'s section header already
 * is, with the same opaque ground and the same recorded reason. The cards
 * themselves keep their wash and are never opaque, so no card gets a step across
 * its middle and no gap between two of them becomes a stripe.
 *
 * ## Colour
 *
 * No accent text anywhere on this screen. Every control that could have been
 * accent-coloured words is a button with a FILL of its own, which is the fix
 * `b6e3114` had to make three times on iOS and the finding
 * `.claude/agent/done/ios-fleet-visual-critique.md` opens with: this pane sits
 * on the terminal's theme-chosen background rather than on a system one, so how
 * well eleven points of blue can be read depends on which theme is in force.
 *
 * That rules out `OutlinedButton` here, which is the trap: it looks like the
 * counterpart of SwiftUI's `.bordered` and is not. `.bordered` fills with a wash
 * of the tint and puts the label on THAT; `OutlinedButton` puts `primary` text
 * over whatever is behind it with a one-dp line around the outside — accent
 * words on the terminal's ground, which is precisely the shape being corrected.
 * `FilledTonalButton` is the one that carries `secondaryContainer` with it, so
 * every secondary control here is one, and `Continue` — the only prominent
 * button on the screen — is a filled `Button`.
 * Green and red are [DIFF_ADDED] and the scheme's `error`, the same pair the
 * fleet row, the front door and the Changes chip already spend.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ChangesPane(
    store: ChangesStore,
    workspace: Workspace?,
    showRunner: Boolean,
    runnerLabel: String,
    /** The terminal's own face and size — see [DiffLine] for why a diff wears them. */
    fontFamily: FontFamily,
    fontSize: Float,
    onOpenDrawer: () -> Unit,
) {
    val state by store.state.collectAsStateWithLifecycle()
    val listState = rememberLazyListState()
    val scope = rememberCoroutineScope()
    var refreshing by remember { mutableStateOf(false) }

    // Once per store, not once per appearance. `load` throws away every patch it
    // holds, and this tab is one chip away from the agents — glancing at one and
    // coming back must not empty the screen and refetch it over a cellular link.
    LaunchedEffect(store) { store.loadIfNeeded() }

    // Where the reader is, from what the list says it is showing.
    //
    // `derivedStateOf` rather than a plain read, because this recomputes on
    // every scroll frame and both of its readers — the pinned bar and the
    // bookmark below — want the same answer. Derived state computes once and
    // caches, so a frame costs one pass over at most a handful of visible rows.
    val top = remember(listState) {
        derivedStateOf {
            val info = listState.layoutInfo
            val visible = info.visibleItemsInfo.map {
                ReviewScroll.VisibleRow(it.key as? String ?: "", it.offset, it.size)
            }
            val key = ReviewScroll.topVisible(
                rows = visible,
                viewportStart = info.viewportStartOffset,
                viewportEnd = info.viewportEndOffset,
                isFile = ::isFileRow,
            ) ?: return@derivedStateOf null
            TopFile(
                key = key,
                pinned = ReviewScroll.scrolledPastHeading(
                    visible, key, info.viewportStartOffset),
            )
        }
    }

    // The bookmark. Distinct-until-changed rather than raw, because the value
    // above moves on every frame of a scroll and only its CHANGES are worth a
    // write; `noteTopFile` dedupes again on the far side, which is belt and
    // braces on a path that writes to disk.
    LaunchedEffect(store, top) {
        snapshotFlow { top.value?.key }
            .distinctUntilChanged()
            .collect { store.noteTopFile(it) }
    }

    // Every jump on this screen comes through here, so there is exactly one
    // place that can be wrong about scrolling: the resume, Next and Previous,
    // and opening a file all raise a `Jump` and this moves the list. Cleared by
    // SERIAL after acting — see `Jump` — so a jump raised while this animation
    // was running is not swallowed by the clear belonging to the last one, and
    // tapping the same file twice moves twice.
    LaunchedEffect(state.jump) {
        val jump = state.jump ?: return@LaunchedEffect
        // Null means the row it named has stopped existing, which an agent that
        // kept working overnight does routinely. Nothing to scroll to, and the
        // sentence explaining it is already on `ChangesState.resumeNote`.
        state.indexOf(jump.key)?.let { listState.animateScrollToItem(it) }
        store.clearJump(jump.serial)
    }

    val rows = state.rows

    Column(Modifier.fillMaxSize()) {
        WorkspaceTopBar(
            workspace = workspace,
            fallbackTitle = "Changes",
            showRunner = showRunner,
            runnerLabel = runnerLabel,
            onOpenDrawer = onOpenDrawer,
        ) {
            ReviewMenu(
                onMarkRead = { scope.launch { store.markRead() } },
                onRecompute = { scope.launch { store.load(fresh = true) } },
            )
        }

        Box(Modifier.weight(1f)) {
            // Asks the daemon to RECOMPUTE rather than answer from its cache.
            // This is the affordance that exists because no watcher is perfect
            // and somebody must always have a way to be certain.
            PullToRefreshBox(
                isRefreshing = refreshing,
                onRefresh = {
                    scope.launch {
                        refreshing = true
                        store.load(fresh = true)
                        refreshing = false
                    }
                },
            ) {
                LazyColumn(
                    state = listState,
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(horizontal = 12.dp, vertical = 10.dp),
                    // The gap between cards, which on iOS had to be smuggled
                    // into the heading because a section's header and its
                    // content are two elements of one stack and any spacing
                    // between them opened a seam down the middle of every card.
                    // A card here is one item, so the arrangement can simply say
                    // it, and the ten points land only between cards.
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                ) {
                    // ONE item per row. See this file's doc comment: an extra
                    // item here moves every jump on the screen by one.
                    items(rows, key = { it.key }, contentType = { it::class }) { row ->
                        when (row) {
                            is ChangesRow.Summary -> SummaryBlock(state, store, workspace)
                            is ChangesRow.GeneratedHeading -> GeneratedHeading(row)
                            is ChangesRow.File -> FileCard(
                                file = row.file,
                                state = state,
                                store = store,
                                fontFamily = fontFamily,
                                fontSize = fontSize,
                            )
                        }
                    }
                }
            }

            // `top` is handed in unread, so the state it holds is read one
            // composable further down. Read here, every crossing from one file
            // to the next would invalidate this whole function — which reads
            // `rows`, and therefore rebuilds the list's item lambda — for a
            // change that moves one bar.
            PinnedFileName(
                top = top,
                rows = rows,
                expanded = state.expandedFile,
                onToggle = { store.toggle(it) },
                modifier = Modifier.align(Alignment.TopCenter),
            )
        }

        ReviewBar(state, store, scope)
    }
}

/** The topmost file on screen, and whether its own heading has left with it. */
private data class TopFile(val key: String, val pinned: Boolean)

/**
 * Whether a row's key names a file rather than one of the two generated rows.
 *
 * The summary block sits at the top of every list and the generated heading in
 * the middle of some, and neither is a place in the diff — returning one would
 * make every bookmark say "the top" and the resume offer would have nothing to
 * restore. Compared against the two constants rather than derived from `rows`,
 * because this is called per visible row per scroll frame and rebuilding a set
 * of forty paths to answer it would be the expensive way to say the same thing.
 */
private fun isFileRow(key: String): Boolean =
    key != ChangesRow.TOP_KEY && key != ChangesRow.GENERATED_KEY

// ---- the file, held at the top of the screen ----

/**
 * The name of the file being read, over its own diff.
 *
 * On a long file a screenful of hunks looks like any other screenful of hunks,
 * which is the whole reason this exists. Drawn only once the file's own heading
 * has scrolled off — otherwise the name would be on the screen twice, once in
 * the card and once above it.
 *
 * The same composable as the card's heading, deliberately: a pinned name is the
 * same name pinned or not, so anything trimmed here would also be trimmed from
 * the forty headings somebody scrolls past on the way to it. It is also still
 * the fold control, which is the small thing pinning hands over for free — the
 * way to close a file you are done with is now at the top of the screen instead
 * of a thousand lines back up.
 *
 * See this file's doc comment for the background, which is the one decision here
 * that differs from iOS's on purpose.
 */
@Composable
private fun PinnedFileName(
    top: State<TopFile?>,
    rows: List<ChangesRow>,
    expanded: String?,
    onToggle: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val current = top.value?.takeIf { it.pinned } ?: return
    val file = (rows.firstOrNull { it.key == current.key } as? ChangesRow.File)?.file ?: return
    val onClick = { onToggle(file.path) }
    Column(modifier.fillMaxWidth().background(Color(TerminalPalette.BACKGROUND))) {
        Box(Modifier.padding(horizontal = 12.dp)) {
            Column(
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(12.dp))
                    .background(cardColor())
            ) {
                FileHeading(file, expanded == file.path, onClick)
            }
        }
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
    }
}

// ---- one file ----

/**
 * One file: its heading, which is always there, and its patch, which is there
 * while the file is open.
 *
 * One `Column` in one item, where iOS needs a `Section` of two elements — so the
 * `cardShape` / `bodyShape` pair it names to keep a wash and a material agreeing
 * about a corner has no counterpart here. There is one shape, clipped once,
 * around both halves.
 */
@Composable
private fun FileCard(
    file: ChangedFile,
    state: ChangesState,
    store: ChangesStore,
    fontFamily: FontFamily,
    fontSize: Float,
) {
    val expanded = state.isExpanded(file.path)

    // The scroll decides what gets read: a file's patch is fetched when the file
    // is opened, not when the change set loads. Keyed on the GENERATION as well
    // as the path, which is the load-bearing part — nothing else asks for a
    // file's diff a second time, so a card that stayed composed while the cache
    // was emptied underneath it would sit at "Reading…" forever. Every emptying
    // bumps the generation, a pull to refresh included.
    LaunchedEffect(file.path, state.generation, expanded) {
        if (expanded) store.ensure(file.path)
    }

    Column(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(cardColor())
    ) {
        FileHeading(file, expanded, onClick = { store.toggle(file.path) })
        if (expanded) {
            FileBody(file, state, fontFamily, fontSize)
        }
    }
}

/**
 * The status letter, the name, the two counts, and the fold.
 *
 * `A`, `D`, `R` and `T` are all real here whichever scope the row came from: a
 * commit's files arrive from `changes.commit_files`, which merges
 * `git diff --name-status` onto the counts, the branch's from
 * `change_set::numstat`, which has always done the same, and Local reads the
 * working tree's own porcelain codes. So a file a commit created is badged `A`
 * rather than `M`. The bullet is the no-status case only — a daemon old enough
 * to omit the field, which decodes as [ChangedFileStatus.UNKNOWN] rather than as
 * a wrong letter.
 */
@Composable
private fun FileHeading(file: ChangedFile, expanded: Boolean, onClick: () -> Unit) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 12.dp, vertical = 10.dp)
            // Spoken as one phrase and WITHOUT the directory. Pinned, this is
            // read on the way into every file, and the path in it would be
            // `crates/daemon/src` spelled out once per file on a forty-file
            // branch. `NeedsYouScreen`'s workspace header leaves the branch out
            // of its label for the same reason. The directory stays on screen
            // for the eye.
            .semantics(mergeDescendants = true) {
                heading()
                contentDescription = spokenHeading(file)
            },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            file.status.mark.ifEmpty { "•" },
            style = MaterialTheme.typography.labelSmall,
            fontFamily = FontFamily.Monospace,
            fontWeight = FontWeight.Bold,
            color = statusColor(file.status),
            modifier = Modifier.width(14.dp),
        )
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f)) {
            // Truncated in the MIDDLE, which matters more for the part that
            // stays on screen: a long leaf is `ChangesStore+Resume` or
            // `WorkspaceDetailViewController`, and both ends of it say more than
            // either end alone.
            Text(
                file.name,
                style = MaterialTheme.typography.labelLarge,
                maxLines = 1,
                overflow = TextOverflow.MiddleEllipsis,
            )
            if (file.directory.isNotEmpty()) {
                // Not monospaced — the Mac's file list draws a parent path in
                // the plain face and reserves monospace for the counts, and a
                // directory in Iosevka beside a proportional filename reads as
                // two different kinds of thing on one row. Truncated from the
                // HEAD, so what survives a narrow screen is the directory the
                // file is actually in rather than the `crates/` every path on
                // the branch begins with.
                Text(
                    file.directory,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.StartEllipsis,
                )
            }
        }
        Spacer(Modifier.width(8.dp))
        FileCounts(file)
        Icon(
            if (expanded) Icons.Outlined.ExpandMore
            else Icons.AutoMirrored.Outlined.KeyboardArrowRight,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(start = 4.dp).size(18.dp),
        )
    }
}

@Composable
private fun FileCounts(file: ChangedFile) {
    if (file.binary) {
        Text(
            "binary",
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        return
    }
    Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        if (file.insertions > 0) {
            Text(
                "+${file.insertions}",
                style = MaterialTheme.typography.labelSmall,
                fontFamily = FontFamily.Monospace,
                color = DIFF_ADDED,
            )
        }
        if (file.deletions > 0) {
            Text(
                "-${file.deletions}",
                style = MaterialTheme.typography.labelSmall,
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.error,
            )
        }
    }
}

/**
 * The patch, or the honest account of why there isn't one.
 *
 * The order of these branches is the whole of their correctness. A file being
 * READ has no lines yet and is not an empty file; a file the daemon would not
 * render has no lines and is not an empty file either; and a file git has never
 * seen cannot be given a diff at all, because `git diff` compares against
 * something recorded and nothing is recorded for a file only just written. An
 * empty card in any of those cases reads as a bug.
 *
 * [ChangesState.fileNotices] is a separate thing again, and the distinction is
 * the one an earlier audit got backwards: `truncated` and `firstParentOfMerge`
 * are notices AROUND a patch that is really there, not reasons there is none.
 * They are drawn above the hunks, not instead of them.
 */
@Composable
private fun FileBody(
    file: ChangedFile,
    state: ChangesState,
    fontFamily: FontFamily,
    fontSize: Float,
) {
    val path = file.path
    Column(Modifier.fillMaxWidth().padding(bottom = 8.dp)) {
        for (notice in state.fileNotices[path].orEmpty()) {
            FileNotice(notice)
        }
        when {
            path in state.loadingFiles -> Row(
                Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 2.dp)
                Spacer(Modifier.width(8.dp))
                Text(
                    "Reading…",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            state.unsupported[path] != null -> Text(
                state.unsupported.getValue(path),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
            )

            state.isUntracked(path) -> Text(
                "New file — git has no earlier version to compare against.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
            )

            else -> Patch(state.fileDiffs[path].orEmpty(), fontFamily, fontSize)
        }
    }
}

@Composable
private fun FileNotice(text: String) {
    Row(
        Modifier.padding(horizontal = 12.dp, vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            Icons.Outlined.Warning,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(14.dp),
        )
        Spacer(Modifier.width(6.dp))
        Text(
            text,
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

// ---- the patch ----

/**
 * A file's lines, cut into hunks and folded.
 *
 * **A budget, which iOS does not have, and it is a Compose constraint rather
 * than a preference.** A `LazyColumn` realizes items, not the insides of one, so
 * a file card is composed whole — and a four-thousand-line generated file is
 * four thousand `Row`s measured in a single frame while somebody is mid-scroll.
 * SwiftUI's `LazyVStack` has the same property and iOS gets away with it; a
 * dropped frame budget on a mid-range Android phone is not the same budget. The
 * alternative — a file contributing many items so the list can page it — is the
 * one thing this screen may not do, because [ChangesState.rows] IS the item list
 * and a jump resolves against it.
 *
 * So a long patch draws its first [PATCH_BUDGET] lines and says exactly how many
 * it is holding back, in a row that reveals the rest. Said rather than silently
 * done, for the reason `DetailBox` gives about truncated output: output that
 * stops early without saying so is output somebody can draw the wrong conclusion
 * from. It fires on almost nothing a person wrote — see [PATCH_BUDGET].
 */
@Composable
private fun Patch(lines: List<DiffComputation.Line>, fontFamily: FontFamily, fontSize: Float) {
    var whole by remember(lines) { mutableStateOf(false) }
    val hunks = remember(lines, whole) {
        DiffLayout.hunks(if (whole) lines else lines.take(PATCH_BUDGET))
    }

    Column(Modifier.fillMaxWidth()) {
        for (hunk in hunks) {
            key(hunk.id) { Hunk(hunk, fontFamily, fontSize) }
        }
        if (!whole && lines.size > PATCH_BUDGET) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .clickable { whole = true }
                    .padding(horizontal = 12.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(
                    Icons.Outlined.UnfoldMore,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(16.dp),
                )
                Spacer(Modifier.width(6.dp))
                Text(
                    remainingLines(lines.size - PATCH_BUDGET),
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

/**
 * How many lines of one file are drawn before the rest is offered rather than
 * taken.
 *
 * Six hundred, which is around eight screenfuls at the default size — far more
 * than anybody reads in one sitting on a phone, and well past every hand-written
 * patch this repository has. What it catches is the case it exists for: a
 * lockfile or a generated client, where the whole file is one added hunk and the
 * fold in [DiffLayout] never fires because there are no unchanged lines in it.
 */
private const val PATCH_BUDGET = 600

/**
 * One hunk, with its own sideways scroll.
 *
 * The scroll is PER HUNK, which is the fix for the thing that makes long lines
 * miserable on a phone: with one scroll around the whole file, dragging to see
 * the end of a 200-character line in the third hunk drags the first two hunks
 * off the screen with it, and coming back means dragging all of them back. Per
 * hunk, a long line moves only its own neighborhood.
 *
 * Wrapping instead is not an option: a wrapped diff line breaks the one property
 * a diff has, that a line is a line, and on a phone almost every line of real
 * code would wrap.
 */
@Composable
private fun Hunk(hunk: DiffLayout.Hunk, fontFamily: FontFamily, fontSize: Float) {
    // Folds the reader has opened, by segment. Local to the hunk and lost when
    // the card is folded, which is right: the reason to open a fold is the
    // question being asked right now.
    var revealed by remember(hunk) { mutableStateOf(emptySet<Int>()) }

    Column(Modifier.fillMaxWidth()) {
        hunk.rangeLabel?.let { range ->
            Text(
                range,
                style = MaterialTheme.typography.labelSmall,
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(start = 12.dp, top = 6.dp, bottom = 2.dp),
            )
        }
        Column(
            Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .padding(horizontal = 12.dp)
        ) {
            for (segment in remember(hunk) { DiffLayout.segments(hunk) }) {
                val folded = segment.folded
                if (folded != null && segment.id !in revealed) {
                    Row(
                        Modifier
                            .clickable { revealed = revealed + segment.id }
                            .padding(vertical = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(
                            Icons.Outlined.UnfoldMore,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.size(14.dp),
                        )
                        Spacer(Modifier.width(6.dp))
                        Text(
                            unchangedLines(folded),
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                } else {
                    for (line in segment.lines) {
                        DiffLine(line, fontFamily, fontSize)
                    }
                }
            }
        }
    }
}

/**
 * One line of a patch, in the terminal's own face and size.
 *
 * A diff is code, and this app already has an answer to "what does code look
 * like here" — one the user chose in Settings. Drawing it in the system
 * monospace instead would mean the review pane and the terminal one chip away
 * disagreed about the width of a tab stop and the shape of a zero, which is
 * exactly the comparison a diff exists to support.
 */
@Composable
private fun DiffLine(line: DiffComputation.Line, fontFamily: FontFamily, fontSize: Float) {
    Row(
        Modifier.background(
            when (line.kind) {
                DiffComputation.Kind.ADDED -> DIFF_ADDED.copy(alpha = 0.10f)
                DiffComputation.Kind.REMOVED -> MaterialTheme.colorScheme.error.copy(alpha = 0.10f)
                DiffComputation.Kind.CONTEXT -> Color.Transparent
            }
        )
    ) {
        // Two points down and dimmed: the gutter is for orientation, not for
        // reading, and at the body size it competes with the code beside it.
        Text(
            (line.newNumber ?: line.oldNumber)?.toString().orEmpty(),
            fontFamily = fontFamily,
            fontSize = (fontSize - 2).sp,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.End,
            maxLines = 1,
            modifier = Modifier.width(34.dp),
        )
        Spacer(Modifier.width(8.dp))
        Text(
            when (line.kind) {
                DiffComputation.Kind.ADDED -> "+"
                DiffComputation.Kind.REMOVED -> "-"
                DiffComputation.Kind.CONTEXT -> " "
            } + line.text,
            fontFamily = fontFamily,
            fontSize = fontSize.sp,
            maxLines = 1,
            color = when (line.kind) {
                DiffComputation.Kind.ADDED -> DIFF_ADDED
                DiffComputation.Kind.REMOVED -> MaterialTheme.colorScheme.error
                DiffComputation.Kind.CONTEXT -> MaterialTheme.colorScheme.onSurfaceVariant
            },
        )
    }
}

// ---- everything above the files ----

/**
 * The summary card, the resume offer and whichever notice applies — one row of
 * the list, because it is one block that is either at the top of the screen or
 * scrolled past, and because "land at the top and say why" needs somewhere to
 * land. See [ChangesRow.Summary].
 */
@Composable
private fun SummaryBlock(state: ChangesState, store: ChangesStore, workspace: Workspace?) {
    val scope = rememberCoroutineScope()
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Card {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(
                    Icons.Outlined.AccountTree,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(16.dp),
                )
                Spacer(Modifier.width(8.dp))
                Text(
                    state.changeSet.branch.ifBlank { workspace?.branch.orEmpty() }
                        .ifBlank { "This worktree" },
                    style = MaterialTheme.typography.titleSmall,
                    fontFamily = FontFamily.Monospace,
                    maxLines = 1,
                    overflow = TextOverflow.MiddleEllipsis,
                    modifier = Modifier.weight(1f),
                )
                if (state.loading) {
                    Spacer(Modifier.width(8.dp))
                    CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 2.dp)
                }
            }
            Spacer(Modifier.height(6.dp))
            // The card's lower half is the one thing on this screen that says
            // WHAT is being compared, so a commit replaces it outright rather
            // than being squeezed in beside the branch's base and counts — which
            // would then be describing something nobody is looking at.
            if (state.scope.commitSha != null) {
                CommitHeader(state, store, scope)
            } else {
                ComparisonHeader(state, store, scope)
            }
        }

        state.resume?.let { ResumeCard(state, it, store, scope) }

        state.resumeNote?.let { note ->
            // Tap to dismiss. It has said its piece the moment it is read, and a
            // sentence about a file that moved has no business still being on
            // the screen three commits later.
            Notice(
                icon = Icons.Outlined.Bookmark,
                text = note,
                modifier = Modifier.clickable { store.clearResumeNote() },
            )
        }

        val error = state.error
        when {
            error != null -> Notice(
                icon = Icons.Outlined.Warning,
                tint = MaterialTheme.colorScheme.error,
                text = error.sentence,
                detail = error.transcript,
            )
            // Ahead of the empty case on purpose: a commit that could not be
            // read also has no files, and "nothing changed here" is the one
            // sentence that must not be said about it.
            state.commitUnreadable -> Notice(
                icon = Icons.Outlined.Warning,
                tint = MaterialTheme.colorScheme.error,
                text = "Couldn’t read this commit. It might not be on this branch anymore. " +
                    "Choose another, or go back to the whole branch.",
            )
            state.files.isEmpty() && !state.loading -> Notice(
                icon = Icons.Outlined.CheckCircleOutline,
                text = nothingHere(state.scope),
            )
        }
    }
}

/**
 * What is being compared, its counts, and the ways into the commits.
 *
 * The top line follows the SEGMENT, which is the whole of this. `vs main` under
 * Uncommitted was not merely stale, it was the wrong comparison: uncommitted
 * work is what the worktree has that HEAD does not, and the base does not come
 * into it.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ComparisonHeader(
    state: ChangesState,
    store: ChangesStore,
    scope: CoroutineScope,
) {
    val local = state.scope == DiffScope.Local
    val total =
        if (local) state.uncommittedInsertions to state.uncommittedDeletions
        else state.changeSet.insertions to state.changeSet.deletions
    // The comparison's own totals while nothing was generated, and the
    // hand-written subtotal once something was. See [GeneratedNote].
    val shown =
        if (state.generatedFiles.isEmpty()) total
        else state.writtenInsertions to state.writtenDeletions

    Row(
        verticalAlignment = Alignment.CenterVertically,
        // Read aloud, the parts are "vs main", "plus 82", "minus 13" — three
        // fragments that never say which comparison they belong to, and the
        // control that would have said it is a separate element below. So the
        // label names the comparison it is the total of, the same fix the rows
        // that lead here got in `NeedsYouScreen` and `FleetScreen`.
        modifier = Modifier.semantics(mergeDescendants = true) {
            contentDescription = spokenComparison(state, shown.first, shown.second)
        },
    ) {
        val subject = if (local) "vs HEAD" else state.changeSet.baseRef.let {
            if (it.isEmpty()) "" else "vs $it"
        }
        if (subject.isNotEmpty()) {
            Text(
                subject,
                style = MaterialTheme.typography.labelSmall,
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
            )
        }
        Spacer(Modifier.weight(1f))
        Counts(shown.first, shown.second)
    }

    GeneratedNote(state)

    // Only a GUESSED base is called out. The others are recorded facts; a guess
    // is the one that can silently produce a wrong diff that looks exactly like
    // a right one. Under Uncommitted the guess cannot have produced anything —
    // that comparison never reaches for the base — so leaving the warning on
    // would be telling somebody a diff against HEAD may be wrong, which is the
    // one thing about it that cannot be.
    if (state.changeSet.baseIsGuessed && !local) {
        Spacer(Modifier.height(4.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(
                Icons.Outlined.Warning,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.error,
                modifier = Modifier.size(14.dp),
            )
            Spacer(Modifier.width(6.dp))
            Text(
                "Base branch was guessed, so this diff may be wrong.",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.error,
            )
        }
    }

    // Which question the list below is answering. A segmented control rather
    // than only a menu item, because the two scopes are read one after the other
    // and a menu makes that two taps each way. Two segments and not three: a
    // Commit segment cannot answer its own question — tapping it says nothing
    // about WHICH commit — so it would have to open the picker directly beneath
    // it. See `DiffScope.offered`.
    Spacer(Modifier.height(8.dp))
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
        DiffScope.offered.forEachIndexed { index, option ->
            SegmentedButton(
                selected = state.scope == option,
                onClick = {
                    when (option) {
                        is DiffScope.Local -> store.showUncommitted()
                        else -> store.showWholeBranch()
                    }
                },
                shape = SegmentedButtonDefaults.itemShape(index, DiffScope.offered.size),
            ) {
                Text(option.label, maxLines = 1)
            }
        }
    }

    CommitEntry(state, store, scope)
}

/**
 * The way into reading a branch one commit at a time.
 *
 * A row rather than only a menu item, because a control nobody can see is a
 * feature nobody has — and because "start at the beginning and keep going" is
 * the reading an agent-authored branch is actually legible in. Each commit is
 * one intention, and an agent's intentions only make sense forwards: the third
 * fixes what the second introduced, and read backwards it is a repair to
 * something that has not happened yet.
 *
 * The history PICKER is a sheet and belongs to phase 5c. This row does not need
 * it: it starts at the first commit, and the header that replaces this one
 * carries Previous and Next, so the whole branch is walkable without one.
 */
@Composable
private fun CommitEntry(
    state: ChangesState,
    store: ChangesStore,
    scope: CoroutineScope,
) {
    val count = state.changeSet.commits.size
    Spacer(Modifier.height(6.dp))
    if (count == 0) {
        // Said rather than left blank. A branch with nothing on it yet is the
        // ordinary state of a worktree an agent has only just started in, and a
        // control that does nothing when tapped is worse than one that says why
        // it is quiet.
        Text(
            "No commits yet.",
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        return
    }
    FilledTonalButton(
        onClick = { scope.launch { store.startAtFirstCommit() } },
        modifier = Modifier.fillMaxWidth(),
    ) {
        Icon(
            Icons.Outlined.FormatListNumbered,
            contentDescription = null,
            modifier = Modifier.size(16.dp),
        )
        Spacer(Modifier.width(8.dp))
        Text("Review commit by commit", maxLines = 1)
        Spacer(Modifier.weight(1f))
        Text(commitCount(count), style = MaterialTheme.typography.labelSmall)
    }
}

/**
 * Which commit is on screen, what it said it was doing, and the ways on.
 *
 * The counts come from [ChangeCommit] when the daemon sent them and from the
 * commit's own file list otherwise — see [ChangesState.commitCounts]. They are
 * the same number either way; `--shortstat` is the sum of the same commit's
 * `--numstat`.
 */
@Composable
private fun CommitHeader(
    state: ChangesState,
    store: ChangesStore,
    scope: CoroutineScope,
) {
    val sha = state.scope.commitSha ?: return
    val known = state.selectedCommitInfo

    Row(verticalAlignment = Alignment.Top) {
        Text(
            sha.take(8),
            style = MaterialTheme.typography.labelSmall,
            fontFamily = FontFamily.Monospace,
            fontWeight = FontWeight.SemiBold,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.width(8.dp))
        // Up to two lines: a subject is one line of prose written for a
        // terminal, and truncating it to a phone's width regularly cuts it
        // before the verb.
        if (known != null && known.subject.isNotEmpty()) {
            Text(
                known.subject,
                style = MaterialTheme.typography.labelLarge,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }

    Spacer(Modifier.height(4.dp))
    Row(verticalAlignment = Alignment.CenterVertically) {
        if (known != null) {
            Text(
                "${known.author} · ${known.age(System.currentTimeMillis() / 1_000)}",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                // The one thing on this line that is allowed to give way. A
                // second weight here — a spacer alongside it — would split the
                // row in half and truncate a short author name on a wide phone.
                modifier = Modifier.weight(1f),
            )
        } else {
            Spacer(Modifier.weight(1f))
        }
        state.commitCounts?.let { Counts(it.first, it.second) }
    }

    GeneratedNote(state)

    // The rationale, which for an agent's commit is usually the only one written
    // down anywhere — and the cheapest context there is before a line of diff is
    // read, which between two sets is very often the only context there is time
    // for. Keyed on the sha so moving to the next commit starts it folded again
    // rather than inheriting however far the last one was opened.
    known?.bodyText?.let { body ->
        Spacer(Modifier.height(6.dp))
        CommitBody(body, sha)
    }

    // The change set no longer lists this sha, so there is no subject and no
    // author to show — an amend or a rebase during the read does exactly that.
    // The patch below is usually still right, because the object it names is
    // still in the repository, so this warns rather than blanking the pane.
    if (known == null) {
        Spacer(Modifier.height(6.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(
                Icons.Outlined.Warning,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.error,
                modifier = Modifier.size(14.dp),
            )
            Spacer(Modifier.width(6.dp))
            Text(
                "This commit isn’t on the branch anymore. It was probably amended or " +
                    "rebased while you were reading.",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.error,
            )
        }
    }

    // Where this commit sits in the branch, and the way to the ones either side
    // of it. This is what makes commit-by-commit a path: the reader who has
    // finished one commit's files takes one tap to the next intention, and the
    // position says how many are left — which a sheet, opened and dismissed,
    // never could.
    state.commitPositionLabel?.let { position ->
        Spacer(Modifier.height(6.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconButton(
                onClick = { scope.launch { store.showPreviousCommit() } },
                enabled = state.previousCommit != null,
            ) {
                Icon(
                    Icons.AutoMirrored.Outlined.KeyboardArrowLeft,
                    contentDescription = "Previous commit",
                )
            }
            Text(
                position,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            IconButton(
                onClick = { scope.launch { store.showNextCommit() } },
                enabled = state.nextCommit != null,
            ) {
                Icon(
                    Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                    contentDescription = "Next commit",
                )
            }
            Spacer(Modifier.weight(1f))
            // The obvious way back, said in words. The segmented control is not
            // drawn while a commit is, both because a segmented picker whose
            // selection matches no segment is a control with nothing selected
            // and because the space it wanted is what the subject is using.
            FilledTonalButton(onClick = { store.showWholeBranch() }) {
                Text("Whole branch", maxLines = 1)
            }
        }
    }
}

/**
 * A commit body, folded until it is asked for.
 *
 * An agent's body runs to paragraphs and this card sits above the file list —
 * left open, a good commit message would push the diff off the screen. Four
 * lines is enough for the first sentence of the rationale, which is the part
 * that decides whether the rest is worth reading.
 */
@Composable
private fun CommitBody(text: String, sha: String) {
    var expanded by remember(sha) { mutableStateOf(false) }
    // Measured crudely rather than with a text layout pass: the cost of being
    // wrong is a "More" button that reveals nothing, and the cost of measuring
    // properly on every redraw of a scrolling list is real.
    val long = text.contains("\n\n") || text.length > 200
    Column {
        Text(
            text,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = if (expanded) Int.MAX_VALUE else 4,
            overflow = TextOverflow.Ellipsis,
        )
        if (long) {
            Spacer(Modifier.height(2.dp))
            // A real button style rather than accent-coloured words. This is one
            // of the three places `b6e3114` had to fix on iOS: a card here sits
            // on the terminal's theme-chosen ground rather than on a system
            // background, so accent text with nothing behind it reads differently
            // under every theme.
            FilledTonalButton(onClick = { expanded = !expanded }) {
                Text(if (expanded) "Less" else "More")
            }
        }
    }
}

/**
 * What the two numbers at the top are not counting.
 *
 * The whole reason `isGenerated` exists. A branch that touched eleven source
 * files and regenerated a lockfile reads as four thousand lines changed, and
 * somebody with ninety seconds cannot tell that from a branch that really did
 * rewrite four thousand lines. Split, the headline is the work and this line is
 * the lockfile — and the two still add up to what the daemon counted, which is
 * why this says the numbers rather than hiding them.
 */
@Composable
private fun GeneratedNote(state: ChangesState) {
    if (state.generatedFiles.isEmpty()) return
    Spacer(Modifier.height(4.dp))
    Row(verticalAlignment = Alignment.CenterVertically) {
        Icon(
            Icons.Outlined.Build,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(13.dp),
        )
        Spacer(Modifier.width(6.dp))
        Text(
            generatedAside(state.generatedFiles.size),
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.width(6.dp))
        Text(
            "+${state.generatedInsertions} -${state.generatedDeletions}",
            style = MaterialTheme.typography.labelSmall,
            fontFamily = FontFamily.Monospace,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/**
 * The line that separates what somebody wrote from what a tool wrote.
 *
 * Its own row of the list — see [ChangesRow.GeneratedHeading], and note that it
 * being a row is why `Cargo.lock` is at index 4 rather than 3.
 */
@Composable
private fun GeneratedHeading(row: ChangesRow.GeneratedHeading) {
    Row(
        Modifier.fillMaxWidth().padding(start = 4.dp, end = 4.dp, top = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            Icons.Outlined.Build,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(14.dp),
        )
        Spacer(Modifier.width(8.dp))
        Text(
            generatedHeading(row.count),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.weight(1f))
        Text(
            "${row.lines} lines",
            style = MaterialTheme.typography.labelSmall,
            fontFamily = FontFamily.Monospace,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/**
 * The offer to go back where the last window ended.
 *
 * An offer and NOT a jump. The app was almost certainly killed between the two
 * windows and the agent has probably kept working; restoring silently would drop
 * somebody who opened this tab to glance at one thing into the middle of a
 * patch, and — worse — into a diff that has changed shape underneath the
 * position being restored. So it says where it thinks they were, in the words of
 * the branch rather than in path-and-sha, and waits to be asked.
 */
@Composable
private fun ResumeCard(
    state: ChangesState,
    saved: ReviewPosition,
    store: ChangesStore,
    scope: CoroutineScope,
) {
    Card {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(
                Icons.Outlined.Bookmark,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(16.dp),
            )
            Spacer(Modifier.width(8.dp))
            Text("Continue where you stopped", style = MaterialTheme.typography.titleSmall)
        }
        Spacer(Modifier.height(6.dp))
        Text(
            resumeDescription(saved, state.changeSet.commits),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(10.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = { scope.launch { store.applyResume() } }) { Text("Continue") }
            FilledTonalButton(onClick = { store.dismissResume() }) { Text("Not now") }
        }
    }
}

/** One sentence in a card, for the states that are not a list of files. */
@Composable
private fun Notice(
    icon: ImageVector,
    text: String,
    modifier: Modifier = Modifier,
    tint: Color = MaterialTheme.colorScheme.onSurfaceVariant,
    detail: String? = null,
) {
    Card(modifier) {
        Row(verticalAlignment = Alignment.Top) {
            Icon(
                icon,
                contentDescription = null,
                tint = tint,
                modifier = Modifier.size(16.dp),
            )
            Spacer(Modifier.width(10.dp))
            Text(
                text,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        // Inside the card rather than beneath it, because the words and the
        // sentence are one report; and in a `DetailBox` rather than a second
        // `Text`, because a box is what marks output as somebody else's.
        if (!detail.isNullOrEmpty()) {
            Spacer(Modifier.height(8.dp))
            DetailBox(detail)
        }
    }
}

// ---- the bar at the bottom ----

/**
 * Where you are in the diff, and the controls that move you through it.
 *
 * At the bottom, in thumb reach, and never in the title bar. This is the whole
 * of "moving through a large diff" and it has to be reachable by the hand
 * already holding the phone — a control you have to shuffle your grip to press
 * is a control that does not get pressed between sets. It also stays put while
 * the diff scrolls behind it, so "how far in am I" is answerable without
 * scrolling anywhere, which is the one question a phone's scrollbar cannot
 * answer: it appears while you drag and vanishes while you read.
 *
 * The Next button is one control with two meanings and changes its face when it
 * changes its meaning: while a commit has files left it goes to the next file,
 * and on the last file of a commit it becomes Next commit. That join is what
 * turns a stack of commits into something you can walk end to end.
 *
 * **What is not here yet.** iOS puts a Files button at the leading edge, opening
 * the index sheet, and an outbox row above the bar once notes have been written.
 * Both of those are sheets, and sheets are phase 5c; a control that opens
 * nothing is worse than no control, so neither is drawn. The position label
 * takes the leading edge in the meantime.
 */
@Composable
private fun ReviewBar(
    state: ChangesState,
    store: ChangesStore,
    scope: CoroutineScope,
) {
    Column(Modifier.fillMaxWidth()) {
        HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
        Row(
            Modifier.fillMaxWidth().padding(start = 16.dp, end = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                state.positionLabel,
                style = MaterialTheme.typography.labelLarge,
                fontFamily = FontFamily.Monospace,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
            )
            Spacer(Modifier.weight(1f))
            // A glyph is not a tap target; the button is. Material's own
            // `IconButton` is 48dp square, which clears the 44 the visual
            // critique found almost nothing on the phones clearing.
            IconButton(
                onClick = { store.showPreviousFile() },
                enabled = state.hasPreviousFile,
            ) {
                Icon(Icons.Outlined.KeyboardArrowUp, contentDescription = "Previous file")
            }
            if (state.nextIsCommit) {
                FilledTonalButton(
                    onClick = { scope.launch { store.showNextCommit() } },
                    modifier = Modifier.padding(vertical = 6.dp),
                ) {
                    Text("Next commit", maxLines = 1)
                    Spacer(Modifier.width(4.dp))
                    Icon(
                        Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                        contentDescription = null,
                        modifier = Modifier.size(16.dp),
                    )
                }
            } else {
                IconButton(
                    onClick = { store.showNextFile() },
                    enabled = state.hasNextFile,
                ) {
                    Icon(Icons.Outlined.KeyboardArrowDown, contentDescription = "Next file")
                }
            }
        }
    }
}

/**
 * The two writes this tab can make without a sheet.
 *
 * Marking a worktree read is what clears its badge on the front door and in the
 * fleet list, and it needs nothing but a tap; recomputing is the same thing pull
 * to refresh does, kept here for the reader who is a thousand lines down and
 * would have to fling back to the top to reach the gesture. `changes.set_base`
 * is the third write and needs a branch picker, so it waits for phase 5c.
 */
@Composable
private fun ReviewMenu(onMarkRead: () -> Unit, onRecompute: () -> Unit) {
    var open by remember { mutableStateOf(false) }
    IconButton(onClick = { open = true }) {
        Icon(Icons.Filled.MoreVert, contentDescription = "Review options")
    }
    DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
        DropdownMenuItem(
            text = { Text("Mark as reviewed") },
            leadingIcon = { Icon(Icons.Outlined.CheckCircleOutline, contentDescription = null) },
            onClick = {
                open = false
                onMarkRead()
            },
        )
        DropdownMenuItem(
            text = { Text("Recompute") },
            leadingIcon = { Icon(Icons.Outlined.Refresh, contentDescription = null) },
            onClick = {
                open = false
                onRecompute()
            },
        )
    }
}

// ---- the shared bits of chrome ----

/**
 * What a card on this screen sits on.
 *
 * Lifted off the ground rather than tinted, and derived from the THEME rather
 * than from Material's surface roles: this pane is drawn on the terminal's own
 * background, and a `surfaceContainer` grey beside a Nord or Gruvbox ground is
 * the "cards read as recessed wells" finding the iOS critique files under
 * global. Only "slightly lighter than whatever is behind" holds for both
 * polarities, which is why it is a wash and not a colour.
 *
 * The same six and four percent iOS's `ChangesSurface.card` uses, so the two
 * apps' cards sit at the same distance from the same palette.
 */
@Composable
private fun cardColor(): Color {
    // Read off the scheme rather than off `Themes.revision`, and that is not
    // taste: this is called once per card and a flow collected per card would be
    // forty subscriptions on a list that is being scrolled. `FarCoolerTheme`
    // already pins `surfaceContainerLowest` to the terminal's own background and
    // rebuilds the scheme when the theme changes, so the ground is here for
    // free, reactively, and cannot disagree with what the pane is drawn on.
    val ground = MaterialTheme.colorScheme.surfaceContainerLowest
    return if (ground.luminance() < 0.5f) Color.White.copy(alpha = 0.06f)
    else Color.Black.copy(alpha = 0.04f)
}

@Composable
private fun Card(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Column(
        modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(cardColor())
            .padding(12.dp),
        content = content,
    )
}

@Composable
private fun Counts(insertions: Int, deletions: Int) {
    Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(
            "+$insertions",
            style = MaterialTheme.typography.labelMedium,
            fontFamily = FontFamily.Monospace,
            color = DIFF_ADDED,
        )
        Text(
            "-$deletions",
            style = MaterialTheme.typography.labelMedium,
            fontFamily = FontFamily.Monospace,
            color = MaterialTheme.colorScheme.error,
        )
    }
}

/**
 * The colour of a file's status letter.
 *
 * Deliberately only two of the eight get a colour of their own, and they are the
 * two the diff's own green and red already mean: a file that appeared and a file
 * that went. Everything else is the neutral. Spending a third hue on `R` or `T`
 * would be spending it on the rarest rows in the list, and the letter itself
 * already says which is which.
 */
@Composable
private fun statusColor(status: ChangedFileStatus): Color = when (status) {
    ChangedFileStatus.ADDED, ChangedFileStatus.UNTRACKED -> DIFF_ADDED
    ChangedFileStatus.DELETED -> MaterialTheme.colorScheme.error
    ChangedFileStatus.CONFLICTED -> MaterialTheme.colorScheme.error
    else -> MaterialTheme.colorScheme.onSurfaceVariant
}

// ---- the words, where a test can read them ----
//
// Pure and out here rather than built inline, which is the shape
// `changesDescription` and `NeedsYouScreen`'s reassurance copy already use on
// this app: there is no emulator for this phase, so a sentence built inside a
// composable is a sentence nothing can check.

/**
 * Which nothing this is, when the list is empty.
 *
 * The commit case is the one worth writing down. A commit is compared against
 * its FIRST parent here — `Selector::Commit` in the daemon's `file_diff.rs` —
 * and a merge that only joined two branches genuinely changed nothing against
 * that side while changing plenty against the other. Naming the comparison is
 * the difference between a fact and a claim this screen cannot make.
 */
internal fun nothingHere(scope: DiffScope): String = when (scope) {
    is DiffScope.Branch -> "This branch hasn’t committed anything yet."
    is DiffScope.Local -> "Nothing uncommitted. The workspace is clean."
    is DiffScope.Commit ->
        "Nothing changed against this commit’s first parent, which is also what a " +
            "clean merge looks like."
}

/**
 * Where the bookmark says you were, said the way the branch says it.
 *
 * A subject rather than a sha wherever one is known: "you were reading `push.ts`
 * in *handle retries on 429*" is a place somebody recognizes, and
 * `local/a1b2c3d4` is a place they have to decode.
 */
internal fun resumeDescription(saved: ReviewPosition, commits: List<ChangeCommit>): String {
    val sha = ReviewPosition.sha(saved.scope)
    val place = when {
        sha != null -> {
            val known = commits.firstOrNull { it.sha == sha }
            if (known != null && known.subject.isNotEmpty()) "in “${known.subject}”"
            else "in commit ${sha.take(8)}"
        }
        saved.scope == DiffScope.Local.wire -> "in the uncommitted work"
        else -> "on the whole branch"
    }
    val file = saved.file ?: saved.topFile ?: return "You were $place."
    return "You were at ${file.substringAfterLast('/')}, $place."
}

/** The comparison as one phrase, for a screen reader. See [ComparisonHeader]. */
internal fun spokenComparison(state: ChangesState, insertions: Int, deletions: Int): String {
    val subject = when {
        state.scope == DiffScope.Local -> "Uncommitted, against HEAD"
        state.changeSet.baseRef.isEmpty() -> "Branch"
        else -> "Branch, against ${state.changeSet.baseRef}"
    }
    return "$subject, $insertions added, $deletions removed"
}

/**
 * A file heading as one phrase, and without the directory. See [FileHeading].
 */
internal fun spokenHeading(file: ChangedFile): String {
    val parts = mutableListOf(file.name, file.status.label)
    if (file.binary) {
        parts.add("binary")
    } else {
        if (file.insertions > 0) parts.add("${file.insertions} added")
        if (file.deletions > 0) parts.add("${file.deletions} removed")
    }
    return parts.joinToString(", ")
}

internal fun generatedHeading(count: Int): String =
    if (count == 1) "1 generated file" else "$count generated files"

internal fun generatedAside(count: Int): String =
    if (count == 1) "plus 1 generated file" else "plus $count generated files"

internal fun commitCount(count: Int): String =
    if (count == 1) "1 commit" else "$count commits"

internal fun unchangedLines(count: Int): String =
    if (count == 1) "1 unchanged line" else "$count unchanged lines"

internal fun remainingLines(count: Int): String =
    if (count == 1) "Show 1 more line" else "Show $count more lines"
