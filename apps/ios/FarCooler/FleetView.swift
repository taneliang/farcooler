import SwiftUI

/// The fleet on one host.
///
/// Was the screen between a host and a terminal; now it is mostly a
/// hand-off. A host with a terminal already running goes straight to it
/// (`landing`, decided once per connection — see below) and this screen is
/// what a host with nothing running shows instead, and what `TerminalView`'s
/// switcher sheet reuses to list every worktree. Every state shown here is
/// still DERIVED by the daemon at the moment of asking — the phone never
/// computes a terminal's state, because a client that re-derives can
/// disagree with the daemon and with the Mac about the same terminal.
@MainActor
struct FleetView: View {
    let host: Host
    let store: HostStore

    @StateObject private var connection = Connection()

    /// Which terminal to open automatically, decided once per connection
    /// attempt and then left alone.
    ///
    /// Recomputing this on every poll would mean a terminal finishing its
    /// work while this screen is open — an ordinary thing to happen while
    /// someone is reading it — yanks them onto a different pane mid-read.
    /// `nil` with `landingDecided` true means the fleet genuinely has no
    /// terminals, which is what falls back to `list` below; `nil` with it
    /// false means a connection attempt hasn't finished yet.
    @State private var landing: Terminal?
    @State private var landingDecided = false

    /// Pushed when a terminal is tapped in the fallback list — the only
    /// place this screen still pushes, since the landing terminal above
    /// takes the direct route and the switcher sheet (`WorkspaceListView`
    /// from `TerminalView`) selects in place instead of navigating.
    @State private var pushed: Terminal?

    /// Whether to offer a way off the spinner yet. See `waitedLongEnough`.
    @State private var stalled = false

    @Environment(\.scenePhase) private var scenePhase

    /// Open when correcting this machine's details, from any phase that has a
    /// reason to doubt them.
    @State private var editing = false

    var body: some View {
        Group {
            switch connection.phase {
            case .connecting:
                escapable { connecting }

            case .needsApproval(let fingerprint):
                escapable { approval(fingerprint) }

            case .failed(let message):
                escapable { failure(message) }

            // Reconnecting renders exactly what connected renders. The fleet on
            // screen is the last one this machine sent, and it is a better
            // answer than a spinner while the link comes back — see
            // `Connection.Phase.reconnecting`. The status chip in the bar is
            // where the difference shows.
            case .connected, .reconnecting:
                connected
            }
        }
        .navigationDestination(item: $pushed) { terminal in
            TerminalView(terminal: terminal, connection: connection, hosts: store)
        }
        .sheet(isPresented: $editing) {
            HostEditorView(
                existing: host,
                onSave: { store.update($0) },
                onRemove: { store.remove($0) })
        }
        .task { await connect(host) }
        // The app coming back is the moment a backoff timer cannot predict.
        //
        // Here rather than in `RootView`, because this is where the connection
        // is: the same reason the host switcher moved down out of the
        // connected screen. `.background` is passed on too, so a phone in a
        // pocket stops polling — which is both a battery question and one
        // plausible way the session died in the first place.
        .onChange(of: scenePhase) { _, phase in
            connection.setActive(phase == .active)
        }
    }

    /// Every screen shown BEFORE a connection exists, wrapped in the ways out of
    /// it.
    ///
    /// This is the bug those screens all had. `FleetView` is the root of the
    /// app's only navigation stack — the app opens onto a machine rather than a
    /// list of them — so it has no back button, and the host switcher lives
    /// inside `WorkspaceListView`, which only exists once a connection has
    /// succeeded. Any phase short of `.connected` was therefore a room with no
    /// doors: "Could not connect" offered "Try again" and nothing else, and if
    /// trying again could not work — the wrong address, a machine that never
    /// authorized this phone — there was no way to reach another machine, add
    /// one, fix this one, or even see this device's key. Force-quitting was the
    /// only exit.
    ///
    /// So the switcher comes out of the connected screen and goes under all four
    /// phases instead, in the same place with the same behavior. The bar is
    /// what makes each of these a screen you can leave.
    private func escapable<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        content()
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .safeAreaInset(edge: .bottom, spacing: 0) {
                HostSwitcherBar(hosts: store, connection: connection)
            }
            .navigationTitle(host.label)
            .navigationBarTitleDisplayMode(.inline)
    }

    /// The wait, and a way to end it.
    ///
    /// The button is held back for a few seconds rather than shown immediately:
    /// a healthy connection resolves well inside that, and a "Stop waiting"
    /// flashing up during every successful launch would read as though something
    /// were wrong every time. After that it is the honest offer, because an
    /// address that routes nowhere takes over a minute to fail on its own — see
    /// `Connection.giveUp(on:)`.
    private var connecting: some View {
        VStack(spacing: 12) {
            ProgressView()
            Text("Connecting to \(host.address)…")
                .font(.callout)
                .foregroundStyle(.secondary)

            if stalled {
                Button("Stop Waiting") { connection.giveUp(on: host) }
                    .buttonStyle(.bordered)
                    .padding(.top, 8)
                    .transition(.opacity)
            }
        }
        .task(id: host.id) { await waitedLongEnough() }
    }

    private func waitedLongEnough() async {
        stalled = false
        try? await Task.sleep(for: .seconds(4))
        guard !Task.isCancelled else { return }
        withAnimation { stalled = true }
    }

    @ViewBuilder
    private var connected: some View {
        if let landing {
            TerminalView(terminal: landing, connection: connection, hosts: store)
        } else if landingDecided {
            WorkspaceListView(connection: connection, onSelect: { pushed = $0 }, hosts: store)
                .navigationTitle(host.label)
                .navigationBarTitleDisplayMode(.inline)
        } else {
            ProgressView()
                .navigationTitle(host.label)
                .navigationBarTitleDisplayMode(.inline)
        }
    }

    /// Connect, then decide `landing` from whatever fleet that connection
    /// produced. Shared by the initial `.task` and every retry below — the
    /// approval screen's "Trust this host" and the failure screen's "Try
    /// again" each start a fresh connection of their own, and each one needs
    /// the same decision made afterwards, not the one left over from a
    /// connection attempt that never got this far.
    private func connect(_ target: Host) async {
        await connection.start(host: target)
        landing = connection.fleet.landingTerminal
        landingDecided = true
    }

    // MARK: - Phases

    /// First contact. The fingerprint is shown and refused until a human says
    /// yes, because silently trusting an unknown key is what makes an
    /// interception invisible.
    private func approval(_ fingerprint: String) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Unrecognized Machine", systemImage: "questionmark.circle")
                .font(.headline)
            Text("\(host.address) presented this key:")
                .font(.callout)
            Text(fingerprint)
                .font(.system(.footnote, design: .monospaced))
                .textSelection(.enabled)
                .padding(12)
                .background(Color(.secondarySystemBackground))
                .clipShape(RoundedRectangle(cornerRadius: 10))
            Text(
                "Check it against the machine: ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub"
            )
            .font(.footnote)
            .foregroundStyle(.secondary)

            Button("Trust This Machine") {
                store.trust(host, fingerprint: fingerprint)
                var trusted = host
                trusted.fingerprint = fingerprint
                Task { await connect(trusted) }
            }
            .buttonStyle(.borderedProminent)

            // The other answer. Saying no used to have nowhere to go — this
            // screen had one button on it — which made "I am not sure about this
            // fingerprint" and "yes, trust it" the same tap for anyone who just
            // wanted out. It leaves the host untrusted and lands on the failure
            // screen, which is where the switcher and the editor are.
            Button("Not Now") {
                connection.declineHostKey(host)
            }
            .buttonStyle(.bordered)

            Spacer()
        }
        .padding()
    }

    /// What went wrong, and the one thing worth doing about it.
    ///
    /// The old version of this screen said "Could not connect" and offered "Try
    /// again" whatever had happened. That is right for a machine that was
    /// asleep and wrong for everything else: retrying cannot authorize a key the
    /// host has never seen, cannot install a daemon, and must not be the offered
    /// response to a host key that changed — the one failure where doing it
    /// again is guaranteed to fail and the appearance of a glitch hides a
    /// decision someone needs to make. See `Connection.Failure`.
    /// Laid out like the first-run screen, because it is the same kind of
    /// screen: a mark, a headline, a sentence, and the actions anchored at the
    /// bottom where a thumb is. It used to center three pill buttons of three
    /// different widths in the middle of the view, which read as a ragged
    /// staircase and gave the eye no line to follow — and left the bottom third
    /// of a very tall screen empty while the controls floated in the middle of
    /// it.
    ///
    /// One full-width prominent action, then plain text for the alternatives.
    /// Three bordered pills gave three things the same visual weight when only
    /// one of them is the thing to do.
    private func failure(_ message: String) -> some View {
        let kind = Connection.Failure(message: message)

        return VStack(spacing: 0) {
            Spacer()

            // Quiet by default, and red only for the key change. An orange
            // warning triangle over "Not authorized yet" shouts about a step
            // you simply have not taken yet; the headline already carries what
            // this is, and alarm is worth reserving for the one case that
            // genuinely warrants it.
            Image(systemName: symbol(kind))
                .font(.system(size: 42, weight: .thin))
                .foregroundStyle(kind == .hostKeyChanged ? AnyShapeStyle(.red) : AnyShapeStyle(.tertiary))
                .padding(.bottom, 22)

            Text(headline(kind))
                .font(.title2.weight(.semibold))
                .multilineTextAlignment(.center)
                .padding(.bottom, 8)

            Text(detail(kind, message))
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .textSelection(.enabled)
                .frame(maxWidth: 320)

            Spacer()

            VStack(spacing: 18) {
                primaryAction(kind)

                if kind.worthRetryingAsAlternative {
                    Button("Try Again") { Task { await connect(host) } }
                }
                if kind != .noIdentity {
                    Button("Edit This Machine…") { editing = true }
                }
            }
            .padding(.horizontal, 40)
            .padding(.bottom, 40)
        }
        .padding(.horizontal)
    }

    /// The one action that fits what happened, full width and prominent.
    @ViewBuilder
    private func primaryAction(_ kind: Connection.Failure) -> some View {
        switch kind {
        case .keyRejected:
            // The fix is on the screen this links to: the public key, and the
            // one line to paste on the machine. It was already in the app and
            // unreachable from the only screen that ever sends you looking for
            // it.
            NavigationLink {
                AuthorizeView()
            } label: {
                Text("Authorize This Device").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)

        case .hostKeyChanged:
            Button(role: .destructive) {
                store.forgetKey(host)
                var untrusted = host
                untrusted.fingerprint = nil
                Task { await connect(untrusted) }
            } label: {
                Text("Review the New Key").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)

        case .keyNotTrusted:
            // Straight back to the fingerprint. Deliberately not "Try again":
            // nothing failed, the question is simply still open.
            Button {
                var untrusted = host
                untrusted.fingerprint = nil
                Task { await connect(untrusted) }
            } label: {
                Text("Show the Key Again").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)

        case .unreachable, .daemonMissing, .noIdentity, .stopped, .other:
            Button {
                Task { await connect(host) }
            } label: {
                Text("Try Again").frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)
        }
    }

    private func symbol(_ kind: Connection.Failure) -> String {
        switch kind {
        case .keyRejected, .noIdentity: return "key.slash"
        case .hostKeyChanged: return "exclamationmark.shield"
        case .keyNotTrusted: return "key"
        case .unreachable: return "network.slash"
        case .daemonMissing: return "square.and.arrow.down"
        case .stopped: return "clock"
        case .other: return "exclamationmark.triangle"
        }
    }

    /// The sentence under the headline.
    ///
    /// Ours wherever we know what happened, the core's own text only where we
    /// do not. The raw string crossing up from Rust is written for whoever is
    /// reading a log — lowercase, and ending in things like "(os error 61)" —
    /// and putting that in front of someone who just wants their machine back
    /// is asking them to translate. Two cases keep it deliberately: the changed
    /// host key, whose message carries the two fingerprints being compared and
    /// must not be paraphrased, and the unclassified failure, where the core's
    /// account is the only account there is.
    /// Deliberately does not repeat the headline. The address is already up
    /// there in most of these, and "Not Authorized Yet" over "…doesn't have
    /// this device's key yet" said "yet" twice in two lines.
    private func detail(_ kind: Connection.Failure, _ message: String) -> String {
        switch kind {
        case .keyRejected:
            return "\(host.user)@\(host.address) hasn’t been given this device’s key."
        case .unreachable:
            return
                "Nothing answered on port \(host.port). The machine may be asleep, "
                + "or the address may be wrong."
        case .daemonMissing:
            return "SSH connected, but the Far Cooler daemon didn’t answer. Install it there."
        case .hostKeyChanged, .noIdentity, .keyNotTrusted, .stopped, .other:
            return message
        }
    }

    private func headline(_ kind: Connection.Failure) -> String {
        switch kind {
        case .keyRejected: return "Not Authorized Yet"
        case .hostKeyChanged: return "This Machine’s Key Changed"
        case .unreachable: return "Can’t Reach \(host.address)"
        case .daemonMissing: return "Far Cooler Isn’t Installed"
        case .noIdentity: return "This Device Has No Key"
        case .keyNotTrusted: return "Key Not Trusted"
        case .stopped: return "Stopped Waiting"
        case .other: return "Can’t Connect"
        }
    }

    @ViewBuilder
    private func retry(_ style: some PrimitiveButtonStyle) -> some View {
        Button("Try Again") { Task { await connect(host) } }
            .buttonStyle(style)
    }
}

/// The worktree list plus what it takes to act on it: quick task, new
/// workspace, pull-to-refresh. Shown two places — as `FleetView`'s own
/// fallback when a host has no terminal to land on, and inside the sheet
/// `TerminalView` opens to switch terminals — so a task started from either
/// one works the same way and neither loses a capability the other has.
struct WorkspaceListView: View {
    @ObservedObject var connection: Connection
    let onSelect: (Terminal) -> Void
    /// Non-nil only in the sheet: what "Done" calls. `FleetView`'s own use
    /// leaves this nil because a pushed screen already has a back button.
    var onDismiss: (() -> Void)?
    /// The machines to switch between, when this is the sheet.
    ///
    /// Switching hosts lives here because this is already where you go to
    /// switch what you are looking at. The app opens onto terminals now (see
    /// `RootView`), so there is no host list to go back to — and inventing a
    /// second switcher screen for the rarer of the two switches would be one
    /// more place to look.
    var hosts: HostStore?

    @State private var showNewWorkspace = false
    @State private var showQuickTask = false
    @State private var removeCandidate: Workspace?
    @State private var confirmingRemove = false
    @State private var needsTypedConfirmation: Workspace?

    var body: some View {
        FleetList(fleet: connection.fleet, connection: connection, onSelect: onSelect, onRemove: { ws in
            removeCandidate = ws
            confirmingRemove = true
        }) { action, terminal in
            Task { await connection.act(action, on: terminal) }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if let hosts {
                HostSwitcherBar(hosts: hosts, connection: connection, onSwitch: onDismiss)
            }
        }
        .refreshable { await connection.refresh() }
        // Let the theme's ground show through; a List paints an opaque
        // background of its own that would sit on top of it.
        .scrollContentBackground(.hidden)
        .toolbar {
            // Sparkles for "describe it" (QuickTaskView), plain plus for
            // "fill in the form" (NewWorkspaceView) — same two flows the
            // Mac keeps side by side, kept apart here by icon rather than
            // by picking a winner, since a phone's one-sentence flow is
            // new and unproven next to a form that already works.
            ToolbarItem(placement: .topBarTrailing) {
                Button { showQuickTask = true } label: { Image(systemName: "sparkles") }
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button { showNewWorkspace = true } label: { Image(systemName: "plus") }
            }
            if let onDismiss {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done", action: onDismiss)
                }
            }
        }
        .sheet(isPresented: $showNewWorkspace) {
            NewWorkspaceView(repositories: connection.repositories, connection: connection) { repository, name, branch in
                await connection.createWorkspace(repository: repository, name: name, branch: branch)
            }
        }
        .sheet(isPresented: $showQuickTask) {
            TaskComposerView(connection: connection)
        }
        .confirmationDialog(
            "Remove worktree for \(removeCandidate?.task ?? "")?",
            isPresented: $confirmingRemove,
            titleVisibility: .visible
        ) {
            Button("Remove", role: .destructive) {
                guard let ws = removeCandidate else { return }
                Task {
                    switch await connection.removeWorktree(ws, confirm: "") {
                    case .ok:
                        break
                    case .confirmationRequired, .failed:
                        // The typed-name sheet also handles and displays a
                        // `.failed` result — route every non-.ok outcome
                        // there so there is one place this is shown, not two.
                        needsTypedConfirmation = ws
                    }
                }
            }
            Button("Cancel", role: .cancel) {}
        }
        .sheet(item: $needsTypedConfirmation) { ws in
            RemoveWorktreeConfirmSheet(workspace: ws) { typed in
                await connection.removeWorktree(ws, confirm: typed)
            }
        }
    }
}

/// Which machine you are looking at, and every way of changing that.
///
/// A strip along the bottom rather than a section in a list: the list above it
/// is worktrees on ONE machine, and putting the machine inside it would read as
/// one more thing in the same collection. This says what the collection belongs
/// to.
///
/// Split out of `WorkspaceListView` because it turned out to be the app's only
/// escape hatch, and it was attached to the one screen you cannot reach when you
/// need an escape hatch — the connected one. `FleetView` now puts it under the
/// connecting, approval and failure screens too, which is what makes those
/// screens leaveable at all.
struct HostSwitcherBar: View {
    @ObservedObject var hosts: HostStore
    /// The connection whose state the chip shows, and which its tap retries.
    /// Also how the settings screen names the daemon it is talking to. Absent
    /// before a connection exists, which is most of the time this bar matters.
    @ObservedObject var connection: Connection
    /// Called after picking a different machine, for the caller that is a sheet
    /// and needs to close itself. Nil where the bar is part of the screen.
    var onSwitch: (() -> Void)?

    @State private var showAddHost = false
    /// The host being edited, rather than a bare flag: a flag plus a separate
    /// `hosts.selected` lookup can present a sheet with nothing in it if the
    /// selection changes between the tap and the presentation.
    @State private var editingHost: Host?
    @State private var showSettings = false

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "server.rack")
                .font(.caption)
                .foregroundStyle(.secondary)

            Menu {
                ForEach(hosts.hosts) { host in
                    Button {
                        hosts.selected = host
                        onSwitch?()
                    } label: {
                        if host.id == hosts.selected?.id {
                            Label(host.label, systemImage: "checkmark")
                        } else {
                            Text(host.label)
                        }
                    }
                }
                Divider()
                Button("Add a Machine…") { showAddHost = true }
                if let selected = hosts.selected {
                    // Editing and removing were unreachable from anywhere in the
                    // app: `HostStore.remove` existed and had no caller, so a
                    // machine typed in wrong was permanent, and permanent plus
                    // unreachable meant the app opened onto a screen it could
                    // never get past.
                    Button("Edit This Machine…") { editingHost = selected }
                }
                // Reachable from here because there is nowhere else left.
                //
                // Settings and the device's public key used to live on the root
                // screen, which was the host list. The app opens onto terminals
                // now, so that screen only appears when there are no hosts —
                // and everything that was on it would have become unreachable
                // the moment you added one.
                Button("This Device…") { showSettings = true }
            } label: {
                HStack(spacing: 4) {
                    Text(hosts.selected?.label ?? "No Machine")
                        .font(.callout.weight(.medium))
                    Image(systemName: "chevron.up.chevron.down").font(.caption2)
                }
            }

            Spacer(minLength: 0)

            LinkStatusChip(connection: connection)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.bar)
        .sheet(isPresented: $showAddHost) {
            HostEditorView { hosts.add($0) }
        }
        .sheet(item: $editingHost) { host in
            HostEditorView(
                existing: host,
                onSave: { hosts.update($0) },
                onRemove: { hosts.remove($0) })
        }
        .sheet(isPresented: $showSettings) {
            NavigationStack {
                SettingsView(connection: connection)
                    .toolbar {
                        ToolbarItem(placement: .topBarLeading) {
                            NavigationLink("Authorize") { AuthorizeView() }
                        }
                    }
            }
        }
    }
}

/// Whether this machine is answering, and a way to ask it again.
///
/// The Mac's sidebar dot, on a phone. It sits in the machine bar because that
/// strip is already what says which machine you are looking at, and because it
/// is under every phase including the ones you cannot otherwise escape — the
/// same property that made the bar the app's escape hatch in the first place.
///
/// Connected is a dot and no words. A permanent "Connected" on a phone screen
/// is noise, and the absence of amber says the same thing in no space at all.
///
/// The tap works from every state, green included. That is the "it's actually
/// cooked" case: the app believes it is fine and the person holding it can see
/// that it is not, and a button that refuses to try because the app disagrees
/// is a button that fails exactly when it is needed.
struct LinkStatusChip: View {
    @ObservedObject var connection: Connection

    var body: some View {
        Button {
            connection.reconnectNow()
        } label: {
            HStack(spacing: 5) {
                Circle()
                    .fill(color)
                    .frame(width: 7, height: 7)
                if let label {
                    Text(label)
                        .font(.caption2.weight(.medium))
                        .foregroundStyle(.secondary)
                }
            }
            // A 7-point dot is not a tap target. The padding is, and it stays
            // there when the label does not so the target does not move.
            .padding(.vertical, 6)
            .padding(.leading, 8)
            .padding(.trailing, label == nil ? 8 : 10)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label ?? "Connected")
        .accessibilityHint("Reconnects to this machine")
    }

    private var color: Color {
        switch connection.phase {
        case .connected: return .green
        case .connecting, .reconnecting: return .orange
        case .needsApproval, .failed: return .red
        }
    }

    /// Nothing to say when it is working.
    ///
    /// The attempt number is deliberately not shown. "Reconnecting (4)" prices
    /// a wait nobody asked for and reads as an error count; what someone wants
    /// to know here is whether to keep waiting or tap, and the word alone
    /// answers that.
    private var label: String? {
        switch connection.phase {
        case .connected: return nil
        case .connecting: return "Connecting"
        case .reconnecting: return "Reconnecting"
        case .needsApproval: return "Not Trusted"
        case .failed: return "Disconnected"
        }
    }
}

/// The rows themselves: every workspace on a host, its terminals, and the
/// swipe actions on each. Split out of `WorkspaceListView` because the two
/// places that view is shown agree on everything except the frame around the
/// list — a `NavigationStack` with a title and a "Done" button in the sheet,
/// nothing of the kind when it's `FleetView`'s own fallback content — and
/// `List` content itself carries no opinion about what encloses it.
struct FleetList: View {
    let fleet: Fleet
    @ObservedObject var connection: Connection
    let onSelect: (Terminal) -> Void
    var onRemove: (Workspace) -> Void = { _ in }
    let onAction: (Connection.Action, Terminal) -> Void

    @State private var hiddenExpanded = false

    private var shown: [Workspace] { fleet.workspaces.filter { !$0.isHidden } }
    private var hidden: [Workspace] { fleet.workspaces.filter(\.isHidden) }
    private var hiddenAttention: Int {
        hidden.flatMap(\.terminals).filter(\.agent.wantsAttention).count
    }

    var body: some View {
        List {
            if fleet.workspaces.isEmpty {
                Text("No workspaces on this machine.")
                    .foregroundStyle(.secondary)
            }

            ForEach(shown) { workspace in
                let numbering = workspace.ordinals()
                Section {
                    // Creation order, always. Sorting whatever needs you to the
                    // top read well until you watched it happen: an agent three
                    // rows down finishes, every row under it slides, and the tap
                    // you had already committed to lands on something else. On a
                    // phone that is worse, not better — the target is smaller
                    // and the finger is already moving. Attention is a mark on a
                    // row, and a mark you can find in a list that holds still
                    // beats one that comes to you by moving the list.
                    ForEach(workspace.terminals) { terminal in
                        Button { onSelect(terminal) } label: {
                            TerminalRow(terminal: terminal, ordinal: numbering[terminal.id]) { action in
                                onAction(action, terminal)
                            }
                        }
                        .buttonStyle(.plain)
                    }
                    if workspace.terminals.isEmpty {
                        Text("No terminals").font(.callout).foregroundStyle(.secondary)
                    }
                    Button {
                        Task { await connection.createTerminal(workspace: workspace) }
                    } label: {
                        Label("New terminal", systemImage: "plus")
                    }
                    .font(.callout)
                } header: {
                    HStack {
                        Text(workspace.task)
                        Spacer()
                        Text(workspace.branch)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                        Menu {
                            Button {
                                Task { await connection.createTerminal(workspace: workspace) }
                            } label: {
                                Label("New terminal", systemImage: "plus")
                            }
                            if workspace.isHidden {
                                Button {
                                    Task { await connection.unhideWorkspace(workspace) }
                                } label: {
                                    Label("Unhide", systemImage: "eye")
                                }
                            } else {
                                Button {
                                    Task { await connection.hideWorkspace(workspace) }
                                } label: {
                                    Label("Hide", systemImage: "eye.slash")
                                }
                            }
                            if !workspace.isMainCheckout {
                                Button(role: .destructive) {
                                    onRemove(workspace)
                                } label: {
                                    Label("Remove Worktree…", systemImage: "trash")
                                }
                            }
                        } label: {
                            Image(systemName: "ellipsis.circle")
                                .foregroundStyle(.secondary)
                        }
                    }
                }
            }

            if !hidden.isEmpty {
                Section {
                    if hiddenExpanded {
                        ForEach(hidden) { workspace in
                            let numbering = workspace.ordinals()
                            ForEach(workspace.terminals) { terminal in
                                Button { onSelect(terminal) } label: {
                                    TerminalRow(
                                        terminal: terminal, ordinal: numbering[terminal.id]
                                    ) { action in
                                        onAction(action, terminal)
                                    }
                                }
                                .buttonStyle(.plain)
                            }
                            HStack {
                                Text(workspace.task).font(.caption).foregroundStyle(.secondary)
                                Spacer()
                                Button("Unhide") {
                                    Task { await connection.unhideWorkspace(workspace) }
                                }
                                .font(.caption)
                            }
                        }
                    }
                } header: {
                    Button {
                        hiddenExpanded.toggle()
                    } label: {
                        HStack {
                            Image(systemName: hiddenExpanded ? "chevron.down" : "chevron.right")
                                .font(.caption2)
                            Text("Hidden")
                            Text("\(hidden.count)")
                                .foregroundStyle(.tertiary)
                            if hiddenAttention > 0 {
                                Circle().fill(Color.orange).frame(width: 6, height: 6)
                            }
                            Spacer()
                        }
                    }
                    .buttonStyle(.plain)
                }
            }

            Section {
                HStack {
                    Circle()
                        .fill(fleet.runtimeHealthy ? .green : .orange)
                        .frame(width: 8, height: 8)
                    Text(
                        fleet.runtimeHealthy
                            ? "\(fleet.livePanes) live"
                            : "tmux unavailable on this host"
                    )
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }
        }
    }
}

struct TerminalRow: View {
    let terminal: Terminal
    /// Which of several identically-labeled siblings this is, from
    /// `Workspace.ordinals()`, or nil when its label is unique in the
    /// workspace and numbering it would answer a question nobody asked.
    var ordinal: Int?
    let onAction: (Connection.Action) -> Void

    private var kind: StateKind { StateKind.parse(terminal.state) }

    var body: some View {
        HStack(spacing: 10) {
            Circle().fill(processColor(kind)).frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 4) {
                    Text(terminal.label).font(.body)
                    if let ordinal {
                        Text("\(ordinal)")
                            .font(.caption2.monospaced())
                            .foregroundStyle(.tertiary)
                    }
                }
                Text(terminal.state.lowercased())
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()

            // The reason to have opened the app. Only the two states worth
            // acting on get color, so a list of twenty still reads at a glance.
            if terminal.agent.isAgent && terminal.agent != .unknown {
                Label(terminal.agent.label, systemImage: terminal.agent.symbol)
                    .labelStyle(.iconOnly)
                    .font(.system(size: 13, weight: terminal.agent.wantsAttention ? .semibold : .regular))
                    .foregroundStyle(attentionColor(terminal.agent))
                    .accessibilityLabel(terminal.agent.label)
            }
        }
        .swipeActions(edge: .trailing) {
            if kind == .lost {
                Button("Dismiss") { onAction(.dismissLost) }.tint(.gray)
            }
            Button("Restart") { onAction(.restart) }.tint(.blue)
            if kind == .running || kind == .starting {
                Button("Stop", role: .destructive) { onAction(.stop) }
            }
        }
    }
}

/// The dot color for "is the process alive" — shared by the fleet list and
/// the terminal tab strip (`TerminalTabStrip`), so the same terminal cannot
/// read green in one screen and red in the other.
func processColor(_ kind: StateKind) -> Color {
    switch kind {
    case .running: return .green
    case .starting: return .yellow
    case .exited: return .secondary
    // The one state that means Far Cooler does not know what happened.
    case .lost, .error: return .red
    case .unknown: return .secondary
    }
}

/// The color behind an agent's activity glyph, shared with the tab strip for
/// the same reason as `processColor` above.
func attentionColor(_ agent: AgentActivity) -> Color {
    switch agent {
    case .blocked: return .orange
    case .done: return .green
    default: return .secondary
    }
}

/// The second phase: the worktree has uncommitted work, so removal needs its
/// name typed exactly. Also where any other refusal surfaces, since there is
/// no room for an error message inside a confirmationDialog.
struct RemoveWorktreeConfirmSheet: View {
    let workspace: Workspace
    let onRemove: (String) async -> Connection.RemoveWorktreeResult

    @Environment(\.dismiss) private var dismiss
    @State private var typed = ""
    @State private var working = false
    @State private var errorMessage: String?

    private var matches: Bool { typed == workspace.task }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text("This worktree has uncommitted work. Type its name to remove it anyway.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                TextField("Type \(workspace.task) to confirm", text: $typed)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                if let errorMessage {
                    Section {
                        Text(errorMessage).foregroundStyle(.red).font(.footnote)
                    }
                }
            }
            .navigationTitle("Remove worktree")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Remove", role: .destructive) {
                        working = true
                        Task {
                            switch await onRemove(typed) {
                            case .ok:
                                working = false
                                dismiss()
                            case .confirmationRequired:
                                working = false
                                errorMessage = "That name didn't match — try again."
                            case .failed(let message):
                                working = false
                                errorMessage = message
                            }
                        }
                    }
                    .disabled(!matches || working)
                }
            }
        }
    }
}

/// Registers a repository on a remote host. Always remote: this app has no
/// filesystem of its own worth pointing at, unlike macOS's version of this
/// sheet, which also offers a local file picker.
struct AddRepositorySheet: View {
    let connection: Connection
    /// Called with the new repository's id after a successful registration,
    /// so the caller can select it immediately.
    let onRegistered: (String) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var path = ""
    @State private var working = false
    @State private var errorMessage: String?

    private var canConfirm: Bool { !path.trimmingCharacters(in: .whitespaces).isEmpty && !working }

    var body: some View {
        NavigationStack {
            Form {
                TextField("Path on the host", text: $path)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Section {
                    Text("Far Cooler creates a worktree per task, so it needs an existing repository already on this host.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                if let errorMessage {
                    Section {
                        Text(errorMessage).foregroundStyle(.red).font(.footnote)
                    }
                }
            }
            .navigationTitle("Add repository")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add") {
                        working = true
                        errorMessage = nil
                        Task {
                            do {
                                try await connection.addRepositoryRoot(path: path)
                                let id = try await connection.registerRepository(path: path)
                                working = false
                                onRegistered(id)
                                dismiss()
                            } catch {
                                working = false
                                errorMessage = error.localizedDescription
                            }
                        }
                    }
                    .disabled(!canConfirm)
                }
            }
        }
    }
}

struct NewWorkspaceView: View {
    let repositories: [Repository]
    let connection: Connection
    let onCreate: (String, String, String) async -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var repository: String = ""
    @State private var name = ""
    @State private var branch = ""
    @State private var working = false
    @State private var showAddRepository = false

    private var trimmedName: String { name.trimmingCharacters(in: .whitespaces) }

    /// The folder this name lands in.
    ///
    /// Shown under the field because the name IS the worktree's directory now:
    /// nothing encourages naming a worktree carefully while the thing being
    /// named is invisible.
    private var folder: String { TaskSlug.sanitize(trimmedName) }

    /// Sixty is the machine's cap on a name.
    private var isTooLong: Bool { trimmedName.unicodeScalars.count > 60 }

    /// The branch this form suggests, from the name and the machine's prefix.
    ///
    /// This form had no suggestion at all and made you type a branch by hand,
    /// which meant the machine's branch prefix — the whole point of the setting
    /// — could not reach the one place on this screen that names a branch. Now
    /// it matches the Mac's sheet: type nothing and get the suggestion.
    private var suggestedBranch: String {
        trimmedName.isEmpty
            ? "" : TaskSlug.slug(from: trimmedName, prefix: connection.branchPrefix)
    }

    private var effectiveBranch: String {
        let typed = branch.trimmingCharacters(in: .whitespaces)
        return typed.isEmpty ? suggestedBranch : typed
    }

    /// Both name rules are checked here, not just left to the machine, because
    /// `createWorkspace` swallows its error: a refused name would close this
    /// sheet on a worktree that was never created and say nothing about why.
    private var isValid: Bool {
        !repository.isEmpty && !folder.isEmpty && !isTooLong && !effectiveBranch.isEmpty
    }

    var body: some View {
        NavigationStack {
            Form {
                Picker("Repository", selection: $repository) {
                    Text("Choose").tag("")
                    ForEach(repositories) { Text($0.displayName).tag($0.id) }
                }
                Button("Add a repository…") { showAddRepository = true }
                TextField("Name", text: $name)
                if !trimmedName.isEmpty { folderPreview }
                TextField(
                    "Branch", text: $branch,
                    prompt: Text(
                        suggestedBranch.isEmpty
                            ? connection.branchPrefix + "my-worktree" : suggestedBranch)
                )
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                Section {
                    Text(
                        "A workspace is one git worktree and one branch. Its name is the "
                        + "worktree's folder, so it can't be changed later.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
            }
            .navigationTitle("New workspace")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Create") {
                        working = true
                        Task {
                            await onCreate(repository, trimmedName, effectiveBranch)
                            working = false
                            dismiss()
                        }
                    }
                    .disabled(!isValid || working)
                }
            }
            .sheet(isPresented: $showAddRepository) {
                AddRepositorySheet(connection: connection) { newId in
                    repository = newId
                }
            }
        }
    }

    /// What the name becomes on disk, or why it cannot become anything.
    ///
    /// Both refusals are spelled out rather than left as a dimmed Create
    /// button, which says a name is wrong without saying which rule it broke.
    @ViewBuilder
    private var folderPreview: some View {
        if isTooLong {
            refusal("A name can be at most 60 characters.")
        } else if folder.isEmpty {
            refusal("A name needs a letter or a number in it.")
        } else {
            HStack(spacing: 6) {
                Image(systemName: "folder").foregroundStyle(.tertiary)
                Text(folder)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
    }

    private func refusal(_ message: String) -> some View {
        Label(message, systemImage: "exclamationmark.triangle")
            .font(.caption)
            .foregroundStyle(.orange)
    }
}
