package com.farcooler.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Warning
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.farcooler.data.Theme
import com.farcooler.data.Themes
import com.farcooler.model.AdapterInfo
import com.farcooler.model.AdapterTestOutcome
import com.farcooler.model.HostHealth
import com.farcooler.model.Repository
import com.farcooler.model.RepositoryRoot
import com.farcooler.model.Trouble
import com.farcooler.net.Connection
import com.farcooler.net.rethrowIfCancellation
import kotlinx.coroutines.launch

/**
 * One runner's `config.toml`, from a phone.
 *
 * The reason this belongs on a phone: the runner holding the file is frequently
 * a Linux box with no Far Cooler app on it, reached over ssh. Changing its branch
 * prefix used to mean an ssh session and a text editor, which is not a thing
 * anybody does from a phone — and the theme section's own help text used to say
 * to go and do exactly that.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RunnerSettingsScreen(connection: Connection, onBack: () -> Unit) {
    val scope = rememberCoroutineScope()

    var prefix by remember { mutableStateOf("") }
    var storedPrefix by remember { mutableStateOf("") }
    var themes by remember { mutableStateOf<List<Theme>>(emptyList()) }
    var adapters by remember { mutableStateOf<List<AdapterInfo>>(emptyList()) }
    var health by remember { mutableStateOf<HostHealth?>(null) }
    var roots by remember { mutableStateOf<List<RepositoryRoot>>(emptyList()) }
    var loading by remember { mutableStateOf(true) }
    var failure by remember { mutableStateOf<String?>(null) }
    var addingRepository by remember { mutableStateOf(false) }
    // The two sections whose read is `Scope::HostAdmin` while this app enrolls
    // at `control`. Their own trouble rather than the screen's one `failure`
    // line, because a denial is about ONE section and the rest of the screen is
    // still true — and because [Trouble] carries the runner's words, which a
    // `String?` this screen wrote itself cannot.
    var rootsTrouble by remember { mutableStateOf<Trouble?>(null) }
    var adaptersTrouble by remember { mutableStateOf<Trouble?>(null) }

    // The projects this runner already knows, off the fleet poll rather than
    // out of a call of its own. `repositories` is `Scope::Read` and is already
    // being refreshed every three seconds for the fleet, so a section that
    // re-read it here would be a second answer that can disagree with the
    // sidebar's.
    val repositories by connection.repositories.collectAsStateWithLifecycle()

    // Whether this connection may ask for the calls that change the runner —
    // its branch prefix, its themes, its agents. Learned in the handshake, so
    // it is known before anything on this screen is pressed, which is the whole
    // difference between this and the trouble boxes below: those report a call
    // that already failed, this dims a call that would.
    //
    // Null while `loadDaemonBuild` is still on its way, and that reads as
    // "yes" — the same direction [DaemonBuild.mayAdministerRunner] takes for a
    // grant this build has no word for. A screen that starts out dimmed and
    // un-dims a moment later is worse than one that never flickers, and the
    // controls are honest either way: the runner is the one enforcing this, not
    // the app.
    val daemon by connection.daemon.collectAsStateWithLifecycle()
    val mayAdminister = daemon?.mayAdministerRunner() ?: true

    var editingTheme by remember { mutableStateOf<Theme?>(null) }
    var editingAdapter by remember { mutableStateOf<AdapterInfo?>(null) }
    var adapterIsNew by remember { mutableStateOf(false) }

    // Its own function because two things ask for it: the first load, and a
    // repository having just been added under a folder that may or may not have
    // been watched before. Both have to set the trouble as well as the list, or
    // a second read that failed would leave the first read's rows on screen with
    // nothing saying they are stale.
    suspend fun reloadRoots() {
        roots = try {
            rootsTrouble = null
            connection.repositoryRoots()
        } catch (e: Exception) {
            e.rethrowIfCancellation()
            rootsTrouble = Trouble(deniedSentence("watched folders"), e.message)
            emptyList()
        }
    }

    suspend fun reload() {
        // No round trip after the first: the handshake already carried both the
        // capabilities and this session's grant, and this is cached per
        // connection. Asked here anyway so the screen never draws a control
        // live that the connection cannot use, whatever order it was reached in.
        connection.loadDaemonBuild()
        storedPrefix = connection.branchPrefix.value
        prefix = storedPrefix
        // First, because it is what says whether the rest of the screen can be
        // trusted — and because it is `Scope::Read`, unlike everything under it.
        // A phone the runner has given read scope gets a health section and
        // three empty ones, which is a truer picture than a screen that fails
        // whole.
        health = connection.health()
        themes = connection.hostThemes()
        adapters = try {
            adaptersTrouble = null
            connection.adapters()
        } catch (e: Exception) {
            e.rethrowIfCancellation()
            adaptersTrouble = Trouble(deniedSentence("agents"), e.message)
            emptyList()
        }
        reloadRoots()
        loading = false
    }

    LaunchedEffect(connection) { reload() }

    // The editors take the whole screen rather than a bottom sheet: nineteen
    // colour rows and seven fields do not fit one, and a sheet that scrolls
    // behind the keyboard is worse than a screen that does not have to.
    editingTheme?.let { theme ->
        ThemeEditorScreen(
            theme = theme,
            onCancel = { editingTheme = null },
            onSave = { edited ->
                editingTheme = null
                scope.launch {
                    connection.upsertTheme(edited)?.let { themes = it }
                    // Every picker reads the merged list, so a saved theme has
                    // to reach it — otherwise the thing you just made is
                    // missing from the one place you would choose it.
                    connection.reloadThemes()
                }
            })
        return
    }
    editingAdapter?.let { adapter ->
        AdapterEditorScreen(
            adapter = adapter,
            isNew = adapterIsNew,
            onTest = { connection.testAdapter(it) },
            onCancel = { editingAdapter = null },
            onSave = { edited ->
                editingAdapter = null
                scope.launch { connection.upsertAdapter(edited)?.let { adapters = it } }
            })
        return
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(connection.host.displayLabel) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
                actions = {
                    if (loading) {
                        CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                    }
                })
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            // What this runner is and whether it is well.
            //
            // Absent entirely on a runner too old to answer, rather than drawn
            // as zeroes — a version of "unknown" and a version of "0.0.0" are
            // not the same claim. iOS's `healthSection` does the same with the
            // same nil.
            health?.let { HealthSection(it) }

            // Each of these draws its own trailing rule, the way [HealthSection]
            // does, because either can be absent: two rules stacked with nothing
            // between them is what a section that decided not to draw leaves
            // behind when the rule belongs to the caller.
            RepositoriesSection(repositories)
            WatchedFoldersSection(roots, rootsTrouble) { addingRepository = true }

            SectionTitle("Branches")
            // The field itself, not just Save. `settings.set_branch_prefix` is
            // the only thing this section does, so a field that still typed
            // would collect an edit with nowhere to go — and with it disabled
            // the draft can never differ from the stored value, which is what
            // takes Save away below without a second condition.
            OutlinedTextField(
                value = prefix,
                onValueChange = { prefix = it },
                label = { Text("Branch prefix") },
                placeholder = { Text("feat/") },
                singleLine = true,
                enabled = mayAdminister,
                modifier = Modifier.fillMaxWidth(),
            )
            Text(
                "Goes in front of a branch name made from a task description. " +
                    "Leave it empty for no prefix.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (!mayAdminister) RestrictedNote()
            if (prefix != storedPrefix) {
                Button(onClick = {
                    scope.launch {
                        val stored = connection.setBranchPrefix(prefix)
                        if (stored == null) {
                            failure = "That runner didn’t accept the change."
                        } else {
                            storedPrefix = stored
                            prefix = stored
                            failure = null
                        }
                    }
                }) { Text("Save") }
            }

            HorizontalDivider()
            SectionTitle("Themes on this runner")
            Text(
                "Only themes this runner defines are listed. Shipped themes have nothing to " +
                    "delete; saving one under a shipped name overrides it here.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            // The list still loads: `theme.list` is `Scope::Read` and only the
            // writes underneath it are not. So this section reads normally and
            // dims its three buttons, rather than emptying — which is the same
            // rule `adapters()` follows one section down for the opposite case.
            if (!mayAdminister) RestrictedNote()
            for (theme in themes) {
                Row(
                    Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    ThemeStrip(theme)
                    Text(theme.name, Modifier.weight(1f))
                    TextButton(
                        onClick = { editingTheme = theme },
                        enabled = mayAdminister,
                    ) { Text("Edit") }
                    TextButton(
                        onClick = {
                            scope.launch { connection.deleteTheme(theme.name)?.let { themes = it }
                                connection.reloadThemes() }
                        },
                        enabled = mayAdminister,
                    ) { Text("Delete") }
                }
            }
            OutlinedButton(enabled = mayAdminister, onClick = {
                val taken = themes.map { it.name }.toSet()
                var name = "${Themes.current.name} Copy"
                var n = 2
                while (taken.contains(name)) {
                    name = "${Themes.current.name} Copy $n"
                    n += 1
                }
                editingTheme = Themes.current.copy(name = name)
            }) { Text("Duplicate the current theme") }

            HorizontalDivider()
            SectionTitle("Agents")
            Text(
                "An adapter lets Far Cooler show an agent as a chat instead of its terminal.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            // Every runner has adapters — `adapter_origin` merges the shipped
            // table into this list — so an empty section is a read that did not
            // happen, and this is what makes the difference visible. `adapters()`
            // used to swallow it and answer the empty list, which said the
            // opposite of the truth in the runner's name.
            //
            // Both halves, and they are not the same statement. The note says
            // what this connection may not ask for, ahead of asking. The box
            // says what actually came back — `adapter.list` is host_admin too,
            // so on this runner it is usually the same cause, but "usually" is
            // the reason neither one stands in for the other.
            if (!mayAdminister) RestrictedNote()
            adaptersTrouble?.let { SheetFailure(it) }
            for (adapter in adapters) {
                Row(
                    Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Column(Modifier.weight(1f)) {
                        Text(adapter.preset)
                        Text(
                            adapter.subtitle(),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                        )
                    }
                    TextButton(
                        onClick = {
                            adapterIsNew = false
                            editingAdapter = adapter
                        },
                        enabled = mayAdminister,
                    ) { Text("Edit") }
                    // Only what the file owns can be removed. A built-in has no
                    // table to delete, so offering it would do nothing.
                    if (adapter.origin == "override" || adapter.origin == "user") {
                        TextButton(
                            onClick = {
                                scope.launch {
                                    connection.deleteAdapter(adapter.preset)?.let { adapters = it }
                                }
                            },
                            enabled = mayAdminister,
                        ) { Text(if (adapter.origin == "override") "Revert" else "Delete") }
                    }
                }
            }
            OutlinedButton(enabled = mayAdminister, onClick = {
                val taken = adapters.map { it.preset }.toSet()
                var name = "my-agent"
                var n = 2
                while (taken.contains(name)) {
                    name = "my-agent-$n"
                    n += 1
                }
                adapterIsNew = true
                editingAdapter = AdapterInfo(preset = name)
            }) { Text("Add an agent") }

            failure?.let {
                HorizontalDivider()
                Text(
                    it,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.error,
                )
            }
        }
    }

    if (addingRepository) {
        AddRepositorySheet(
            connection = connection,
            // Re-read rather than appended to: registering a repository inside
            // a root that was already watched adds no root at all, so a row
            // guessed here would be one the runner does not have. The
            // repositories list needs nothing — it is the fleet's, and
            // `AddRepositorySheet` refreshes that before it closes.
            onAdded = { scope.launch { reloadRoots() } },
            onDismiss = { addingRepository = false },
        )
    }
}

/**
 * The projects this runner can start work in.
 *
 * Read-only, and that is the daemon's shape rather than a decision here: a
 * repository is registered by pointing at one, and there is no RPC to
 * unregister it. Removing the ROOT above it is the only thing that takes a
 * project back out of the fleet, which is the second half of why these are two
 * lists and not one.
 *
 * Absent entirely when there are none, rather than drawn as an empty heading —
 * the same rule the health section follows one section up, and the same one iOS
 * follows at `repositoriesSection`. A runner with no projects is the state
 * every runner starts in, and a heading over nothing reads as a list that
 * failed to load.
 */
@Composable
private fun RepositoriesSection(repositories: List<Repository>) {
    if (repositories.isEmpty()) return
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        SectionTitle("Repositories")
        for (repository in repositories) {
            Column {
                Text(repository.displayName, style = MaterialTheme.typography.bodyMedium)
                // The remote, when git had one to summarize. Monospaced because
                // it is a URL and not prose, and middle-elided because the half
                // that identifies a remote is the end.
                if (repository.remote.isNotEmpty()) {
                    Text(
                        repository.remote,
                        style = MaterialTheme.typography.labelSmall,
                        fontFamily = FontFamily.Monospace,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 1,
                        overflow = TextOverflow.MiddleEllipsis,
                    )
                }
            }
        }
        HorizontalDivider(Modifier.padding(top = 6.dp))
    }
}

/**
 * Where this runner is allowed to go looking, and what that costs.
 *
 * **A standing permission, not a project.** Adding one scans and registers
 * nothing; it grants the daemon the right to operate under a directory tree, and
 * that is why the footer says what it says. Registering a repository is the
 * separate call that follows — see [Connection.addRepository].
 *
 * ## Why there is no Remove here
 *
 * It used to be that no app could send this call at all: `crates/daemon` refuses
 * a `repository_root.remove` that carries anything but a `TypedConfirmation`
 * payload, and `Session::remove_repository_root` took no `confirm` argument and
 * sent `None`, so the request went out carrying `Payload::Empty` and came back
 * `InvalidArgument { what: "payload" }` before scope, the root or a typed name
 * was ever looked at. iOS's swipe on this same screen went through that arm and
 * cannot once have succeeded. Only the Mac's worked, by shelling out to the CLI,
 * which built the payload itself.
 *
 * That is fixed. `crates/client`'s `actions::remove_repository_root` builds the
 * payload for every client, and the FFI's arm takes a `confirm` argument, so the
 * call is reachable from here today.
 *
 * What is missing now is this screen's half, and it is not a small one: the
 * runner asks for the folder's own name typed back, EVERY time, because removal
 * revokes Far Cooler's permission over a whole directory tree. A client cannot
 * invent that on the user's behalf — that is the entire point of typing it — so
 * a Remove here means a confirmation sheet, which iOS's `RemoveRootConfirmSheet`
 * now has and this screen does not. A button that opens nothing is still the
 * thing `07e75e8` is a fix for, so there still isn't one; the note below names
 * where it can be done instead.
 *
 * [trouble] is the honest answer to a scope denial rather than a guess at one.
 * `repository_root.list` is `Scope::HostAdmin` and this app enrolls at
 * `control`, and nothing on this side can predict which of those the runner will
 * apply — see [Connection.removeWorktree] for why. So the runner's own words go
 * in a box, and the rest of the screen is left alone.
 */
@Composable
private fun WatchedFoldersSection(
    roots: List<RepositoryRoot>,
    trouble: Trouble?,
    onAdd: () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        SectionTitle("Watched folders")
        Text(
            "Far Cooler looks for repositories in these. Removing one stops the search and " +
                "deletes nothing on disk.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        // Only when there is something to remove. On a runner watching nothing
        // this is a paragraph about a control that would not be there anyway,
        // in front of the one button that matters on this section.
        if (roots.isNotEmpty()) {
            Text(
                rootRemovalNote(),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        trouble?.let { SheetFailure(it) }
        for (root in roots) {
            Text(
                root.displayPath ?: "Hidden",
                style = MaterialTheme.typography.bodyMedium,
                fontFamily = if (root.displayPath == null) null else FontFamily.Monospace,
                color =
                    if (root.displayPath == null) MaterialTheme.colorScheme.onSurfaceVariant
                    else MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                // The head, not the tail: two roots under one home directory are
                // told apart by their last segment, and eliding that would make
                // them the same row.
                overflow = TextOverflow.StartEllipsis,
            )
        }
        OutlinedButton(onClick = onAdd) { Text("Add a repository") }
        HorizontalDivider(Modifier.padding(top = 6.dp))
    }
}

/**
 * Why this screen lists watched folders and cannot remove one.
 *
 * It says the client is the limit, not the permission, and that distinction is
 * the whole value of the sentence: every other refusal on this screen is a scope
 * question the app deliberately does not pre-empt, and somebody who read this
 * absence as another one would go looking for a scope to widen and find that
 * widening it changed nothing.
 *
 * What the limit IS has changed. It used to be the client core — it sent no
 * confirmation payload and the runner refused every such request, from every
 * app. That is fixed; the missing half is now this screen's, which has nowhere
 * to type the folder's name the runner asks for. The sentence says "this app"
 * either way, which is why it did not need rewording when the core was fixed —
 * only the list of places that work got longer.
 *
 * Names the places that CAN do it, because a control that is missing without an
 * alternative is just a dead end. All three work: iOS asks for the name in a
 * sheet, the Mac shells out to the CLI, and the CLI takes it as `--confirm`.
 */
internal fun rootRemovalNote(): String =
    "Removing one takes the Mac app, the iPhone app or the runner’s own command line — this " +
        "app has nowhere to type the folder’s name the runner asks for."

/**
 * What a section says when the runner would not answer it.
 *
 * One sentence for every such section, because the app genuinely knows the same
 * amount about each: this phone enrolls at `control`, these reads are
 * `Scope::HostAdmin`, and a key somebody added to `authorized_keys` by hand
 * carries no scope line and reads as host_admin — so two phones with identical
 * settings get different answers and neither can tell which it is from here. It
 * does not claim a denial, because a runner asleep, a daemon too old and a socket
 * that went away all arrive here looking the same. It says what is missing and
 * lets the runner's own words, in the box underneath, say why.
 */
internal fun deniedSentence(what: String): String =
    "Couldn’t read this runner’s $what."

/**
 * Why a section's own buttons are dimmed rather than live.
 *
 * The other half of [deniedSentence], and deliberately a different sentence
 * about a different moment. That one is spoken AFTER a call came back with
 * nothing, where a runner asleep, a daemon too old and a socket that went away
 * all look identical — so it names what is missing and blames nothing. This one
 * is spoken BEFORE anything is pressed, and it can afford to say why: the
 * handshake carried what this session may ask for, so the app knows in advance
 * that these calls are not for this device. Neither sentence replaces the other,
 * and the failure one is untouched.
 *
 * Says what to do about it, because a dimmed button with no way forward teaches
 * exactly as little as a hidden one. The runner's own command line is named
 * rather than the Mac app, and that is the honest half: the Mac's Add Device
 * flow adds a phone with the same access this one already has, so sending
 * somebody there would send them in a circle.
 *
 * **It does not promise the change takes effect at once, and must not.** sshd
 * reads `authorized_keys` when a connection authenticates and never again, so a
 * device added again stays dimmed on its current connection until it
 * reconnects. "Add it again" is true; "and these light up" would not be.
 *
 * Written in plain product words on purpose, and pinned that way by
 * `RunnerSettingsCopyTest` — the wire's own vocabulary for this means nothing to
 * the person reading it, and half of it would be a claim about the runner this
 * app has no business making.
 */
internal fun restrictedSentence(): String =
    "This device can’t change this runner’s settings. Add it again from the runner’s own " +
        "command line, with full access, to change them from here."

/** [restrictedSentence], in the voice the rest of a section's help text uses. */
@Composable
private fun RestrictedNote() {
    Text(
        restrictedSentence(),
        style = MaterialTheme.typography.labelSmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
}

/**
 * Whether this runner says it is well, and what it is.
 *
 * **[HostHealth.reasons] is the reason this section exists.** Everything else in
 * it is on the phone already — the fleet poll carries the same healthy flag and
 * the same live count, and `loadDaemonBuild` caches the same platform and
 * version — but nothing carried the daemon's account of WHY, and the fleet
 * footer's three words ("tmux unavailable") are the whole of what this app could
 * previously say about a degraded runner. They are also fleet-wide: that footer
 * only colours when EVERY runner is down, so one bad runner out of three said
 * nothing at all anywhere. This screen is per-runner by construction.
 *
 * The reasons are printed one to a line, unsummarized and in the runner's own
 * voice. Summarizing them would throw away the only part that says what to do,
 * and rewriting them would make a machine's words read as Far Cooler's — the
 * split `Trouble` exists for.
 */
@Composable
private fun HealthSection(health: HostHealth) {
    // Its own Column, tighter than the screen's 12dp. Everything in here is one
    // paragraph about one runner; at the screen's spacing the eight lines read
    // as eight separate settings.
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        SectionTitle("This runner")
        Row(verticalAlignment = Alignment.CenterVertically) {
            Icon(
                if (health.healthy) Icons.Outlined.CheckCircle else Icons.Outlined.Warning,
                contentDescription = null,
                // Red for degraded, not amber: amber on this phone means an agent
                // is waiting on you and nothing else. `7e4a4f7` settled that for
                // iOS and `FleetScreen`'s footer already follows it.
                tint = if (health.healthy) MaterialTheme.colorScheme.onSurfaceVariant
                else MaterialTheme.colorScheme.error,
                modifier = Modifier.size(18.dp),
            )
            Spacer(Modifier.width(8.dp))
            Text(if (health.healthy) "Healthy" else "Degraded")
        }
        for (reason in health.reasons) {
            DetailBox(reason)
        }
        HealthFact("Far Cooler", health.daemonVersion.ifBlank { "unknown" })
        HealthFact("Platform", health.platform.ifBlank { "unknown" })
        HealthFact("Live panes", health.livePanes.toString())
        // The one fact here that is not also somewhere else in the app, apart from
        // the reasons. Small and last, because it is what you read when somebody
        // asks for it and never otherwise.
        if (health.protocolVersion > 0) {
            Text(
                "Protocol ${health.protocolVersion}.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        HorizontalDivider(Modifier.padding(top = 6.dp))
    }
}

/** A label and its value, for the health section's short list of facts. */
@Composable
private fun HealthFact(label: String, value: String) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(
            label,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.weight(1f))
        Text(value, style = MaterialTheme.typography.bodySmall)
    }
}

/** A theme's colours as one strip, for a list row. */
@Composable
fun ThemeStrip(theme: Theme) {
    Row(horizontalArrangement = Arrangement.spacedBy(1.dp)) {
        // The ground, the text, and the eight normal colours. Enough to tell two
        // themes apart at a glance, which is all a row has to do.
        val preview = listOf(theme.background, theme.foreground) + theme.ansi.take(8)
        for (packed in preview) {
            Box(
                Modifier
                    .width(5.dp)
                    .height(18.dp)
                    // Opaque alpha, since the grid's colours carry none.
                    .background(Color(0xFF000000.toInt() or packed)))
        }
    }
}
