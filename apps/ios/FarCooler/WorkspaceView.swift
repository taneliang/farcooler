import PhotosUI
import SwiftUI
import UIKit

/// One tab in a workspace.
///
/// Two kinds, and the second one has no object behind it on the runner. Every
/// `changes.*` RPC takes a `workspace_id` and nothing else — see
/// `Session::change_set` and `Session::file_diff` in
/// `crates/client/src/session.rs`, which pass `None` where a terminal-scoped
/// call passes an id — so the diff is a fact about the worktree that this app
/// can ask for whether or not anybody ever opened a `changes` pane in it. That
/// is what lets Changes be a tab at no cost on the daemon side.
///
/// A `changes` pane the host DOES have is filtered out of the tab strip and
/// folded into this one by `Pane.init(_:)`. Both resolve to the same
/// `ChangesStore`, keyed by workspace on `Connection`, so what has been read,
/// what is folded, where you were and the notes you have written are one review
/// rather than one per door.
enum Pane: Identifiable, Hashable {
    case terminal(Terminal)
    case changes

    /// The pane a terminal belongs on. A `changes` pane is the Changes tab, not
    /// a tab of its own — the alternative is two chips showing one diff.
    init(_ terminal: Terminal) {
        self = terminal.isChangesPane ? .changes : .terminal(terminal)
    }

    /// Namespaced, because a terminal id and the word "changes" are different
    /// kinds of thing and a collision between them would silently give two
    /// panes one SwiftUI identity — which is resolved by drawing one of them.
    var id: String {
        switch self {
        case .terminal(let terminal): return "terminal:\(terminal.id)"
        case .changes: return "changes"
        }
    }

    /// This tab as something worth writing down.
    ///
    /// A `Pane` holds a whole `Terminal` — a snapshot of what the daemon said
    /// when the tab was opened — and a snapshot is the wrong thing to remember
    /// or persist, for the reason `Route` gives about its own cases. A
    /// `Route.Focus` is an id, which either still names a pane on this runner
    /// or does not, and the second answer is one `WorkspaceRoute` can act on.
    ///
    /// Never `.none`: that value means "nobody said", and a tab somebody tapped
    /// is the opposite of that.
    var focus: Route.Focus {
        switch self {
        case .terminal(let terminal): return .agent(terminal.id)
        case .changes: return .changes
        }
    }

    /// The terminal this tab is, where it is one. Nil for Changes, which is
    /// exactly the question `Notifier.visibleTerminal` is asking.
    var terminal: Terminal? {
        if case .terminal(let terminal) = self { return terminal }
        return nil
    }
}

/// One worktree: the agents working in it, its diff, and every tab you have
/// opened still mounted behind the one on screen.
///
/// This was `PaneHost`, scoped to a terminal and given a tab strip of the whole
/// fleet. The owner's account of reviewing an agent's work is what re-scoped it:
/// reading a diff is only part of the job, and the larger part is seeing what
/// the agent said it did, deciding whether that was right, and replying to it.
/// Those two live in one worktree and you move between them constantly, so they
/// are tabs of one screen rather than two destinations with a Back between them.
/// The reply channel is the agent's own composer, one chip away.
///
/// One `TerminalView` was reused for every pane before any of this: switching
/// tabs pointed it at a different terminal, which meant SwiftUI tore down
/// whatever the old pane had built. A chat lost its scroll position and its
/// half-typed message, a review lost which files were open and where you were
/// reading, and a terminal renegotiated its size with tmux on every visit —
/// which is the content jumping around on open.
///
/// So panes are mounted and left mounted, and switching shows a different one.
/// State survives because nothing is destroyed: there is no scroll position to
/// restore, no fold state to remember, and no code that could get either wrong.
/// A whole class of bug stops being representable.
///
/// What does NOT survive is anything that costs the host. Each pane's ssh
/// stream, agent poll and tmux size assertion follow `isVisible` rather than
/// mounting — without that, keeping panes alive would hold several streams open
/// and have several of them arguing with tmux about one pane's geometry, making
/// the resize churn worse rather than better.
///
/// Only panes actually VISITED are mounted. Mounting every tab would pay setup
/// for panes nobody opens — including fetching a diff nobody asked to see — and
/// a workspace's handful of visited panes is what "the tabs I am working in"
/// actually means.
@MainActor
struct WorkspaceView: View {
    /// The worktree this screen is, by id.
    ///
    /// An id and not a `Workspace`, so everything drawn from it is read live.
    /// The panes are the part that must not be re-read — see `visited` — and
    /// they carry their own values.
    let workspaceID: String

    @ObservedObject var connection: Connection
    let hosts: RunnerStore?

    /// A pane something OUTSIDE this screen has asked to show — a tapped Live
    /// Activity card, or the switcher sheet naming a pane in this workspace,
    /// both routed by `FleetView.show(_:)`.
    ///
    /// `initial` below is only a starting value, so once this view is mounted it
    /// cannot be pointed anywhere by its arguments; a deep link that arrived
    /// while a pane was already open would otherwise be silently ignored.
    /// Cleared here the moment it is honored, so asking twice for the same pane
    /// works — the second ask is a change again.
    @Binding var requested: Terminal?

    /// How to open a pane this workspace does not hold. The switcher sheet
    /// lists the whole runner, and a terminal in another worktree is a different
    /// screen — see `FleetView.show(_:)`, which replaces this route rather than
    /// stacking a second host on it.
    let onOpen: (Terminal) -> Void

    @State private var current: Pane
    /// Visited panes, oldest first. An array rather than a set so the order is
    /// stable — SwiftUI identity in a `ForEach` is the whole mechanism here.
    @State private var visited: [Pane]
    @State private var showWorkspaceList = false
    /// Images on their way into a terminal. Owned here rather than per pane, so
    /// a transfer keeps running — and keeps reporting — when you switch away
    /// from the pane that started it.
    @StateObject private var pastes = ImagePasteQueue()
    @State private var pickedImage: PhotosPickerItem?
    @State private var showPhotoPicker = false
    /// The navigation bar's lower edge in window coordinates. iOS 26 changes
    /// the hosting view's top proposal while an input accessory is active; the
    /// bar itself does not move, so it is the stable boundary for the tab strip.
    @State private var navigationBarBottom: CGFloat?
    /// Height of the floating strip and its breathing room, used as scroll
    /// content inset without shortening the full-bleed scroll surface.
    @State private var tabStripHeight: CGFloat = 0

    @Environment(\.scenePhase) private var scenePhase

    init(
        workspace: String,
        initial: Pane,
        connection: Connection,
        hosts: RunnerStore? = nil,
        requested: Binding<Terminal?> = .constant(nil),
        onOpen: @escaping (Terminal) -> Void = { _ in }
    ) {
        self.workspaceID = workspace
        self.connection = connection
        self.hosts = hosts
        self.onOpen = onOpen
        _requested = requested
        _current = State(initialValue: initial)
        _visited = State(initialValue: [initial])
    }

    /// The current terminal as the daemon describes it RIGHT NOW, or nil on the
    /// Changes tab, which is not a terminal at all.
    ///
    /// `current` holds a copy taken when the pane was opened, which is right for
    /// identity and wrong for pane mode — switching a pane to chat has to change
    /// what the toolbar offers.
    private var live: Terminal? {
        guard let terminal = current.terminal else { return nil }
        return connection.terminal(terminal.id, in: workspaceID) ?? terminal
    }

    private var currentWorkspace: Workspace? {
        connection.fleet.workspaces.first { $0.id == workspaceID }
    }

    /// What to call this workspace before the fleet has one to call it.
    ///
    /// Only ever shown in the gap between this screen appearing and the next
    /// poll answering, which `WorkspaceRoute` has usually already closed.
    private var workspaceName: String { currentWorkspace?.task ?? "Workspace" }

    var body: some View {
        GeometryReader { geometry in
            let contentTop = geometry.frame(in: .global).minY
            let fallbackBarBottom = contentTop + geometry.safeAreaInsets.top
            let navigationClearance = max(
                0, (navigationBarBottom ?? fallbackBarBottom) - contentTop)

            ZStack(alignment: .top) {
                // Pane content remains full-bleed. It scrolls behind the
                // navigation bar and the floating glass strip, preserving the
                // depth and edge treatment of the original screen.
                ZStack {
                    ForEach(visited) { pane in
                        content(of: pane)
                            // Hidden, not removed. `opacity` keeps the view in the
                            // hierarchy — which is what preserves its state — while
                            // `allowsHitTesting` stops a pane nobody can see from
                            // swallowing taps meant for the one on top of it.
                            .opacity(pane.id == current.id ? 1 : 0)
                            .allowsHitTesting(pane.id == current.id)
                            // Never recycled onto a different pane.
                            .id(pane.id)
                    }
                }
                // The scroll surface extends beneath the navigation chrome,
                // but its resting content begins below the bar and tab strip.
                // Unlike padding, a safe-area inset is consumed by ScrollView
                // as content inset, so rows can still travel behind the glass
                // when the user scrolls.
                .safeAreaInset(edge: .top, spacing: 0) {
                    Color.clear.frame(height: navigationClearance + tabStripHeight)
                }

                // An overlay, not layout chrome: pin only the control while
                // leaving the scroll surface underneath at full height.
                TerminalTabStrip(
                    workspace: currentWorkspace,
                    changes: connection.inbox[workspaceID],
                    current: current,
                    onSelect: choose
                )
                .padding(.top, 6)
                .padding(.bottom, 8)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height in
                    guard abs(tabStripHeight - height) > 0.5 else { return }
                    tabStripHeight = height
                }
                .padding(.top, navigationClearance)
            }
            .overlay {
                NavigationBarBoundaryReader(bottom: $navigationBarBottom)
                    .frame(width: 0, height: 0)
                    .accessibilityHidden(true)
            }
        }
        // Underlap the navigation chrome consistently in both keyboard states.
        // The tab overlay above uses the bar's real frame to stay below it.
        .ignoresSafeArea(.container, edges: .top)
        // Painted past every edge, including under the home indicator and
        // behind the docked composer.
        //
        // A background that stops at the safe area leaves the band below it
        // showing the window's own black, which reads as a bar across the
        // bottom of the screen rather than as the terminal's ground continuing.
        .background(TerminalPalette.background.ignoresSafeArea())
        // Over the panes, above the key row, and gone the moment the path is
        // typed. Nothing about a transfer is ever written into the pane itself.
        .overlay(alignment: .bottom) { ImagePasteChips(queue: pastes) }
        // A terminal is dark regardless of the phone's own appearance — the host
        // doesn't know or care whether this device is in Light Mode.
        .preferredColorScheme(Themes.shared.current.colorScheme)
        // The task and its branch: the place you are is the workspace, and that
        // is true regardless of which of its tabs is focused.
        //
        // Both lines are the system's now. This screen hand-rolled the pair in a
        // `.principal` toolbar item — a `VStack(spacing: 0)` holding a
        // `.headline` over a `.caption2` — ten lines from `NeedsYouView`, which
        // asks the navigation bar for exactly this shape and gets the platform's
        // metrics, its truncation and its behavior under the bar for free. A
        // title built by hand also stops being a title as far as the system is
        // concerned: it does not participate in the bar's own layout, and
        // VoiceOver reads it as two loose labels rather than as the name of the
        // screen.
        .navigationTitle(workspaceName)
        // Empty rather than absent in the gap before the fleet has answered,
        // which is the same gap `workspaceName` covers with "Workspace". There
        // is no way to ask for no subtitle at all, and the branch arrives within
        // a poll.
        .navigationSubtitle(currentWorkspace?.branch ?? "")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { toolbar }
        .photosPicker(isPresented: $showPhotoPicker, selection: $pickedImage, matching: .images)
        .onChange(of: pickedImage) { _, item in
            guard let item else { return }
            // No terminal to type a path into — only reachable if the tab moved
            // while the picker was up. Cleared rather than left set, or picking
            // the same photo again would not read as a change and nothing would
            // happen twice.
            guard let target = live?.id else {
                pickedImage = nil
                return
            }
            let core = connection.core
            Task {
                // Loaded as data rather than as an `Image`: the picker hands
                // back the original file, and re-encoding a screenshot through
                // SwiftUI would smear the small text that is usually the whole
                // reason someone is sending one.
                guard let data = try? await item.loadTransferable(type: Data.self),
                    let image = UIImage(data: data)
                else {
                    pickedImage = nil
                    return
                }
                pastes.send(image, terminal: target, core: core)
                pickedImage = nil
            }
        }
        // The fast path across worktrees, and it keeps that job.
        //
        // Every selection goes back up to `FleetView.show(_:)` rather than being
        // handled here, including a pane in THIS workspace: that function
        // already knows the three answers, and one of them — a terminal in
        // another worktree, which this strip no longer holds a chip for — is one
        // this screen cannot give. A pane in this workspace comes straight back
        // down through `requested` and switches tabs without rebuilding
        // anything.
        .sheet(isPresented: $showWorkspaceList) {
            NavigationStack {
                WorkspaceListView(
                    connection: connection,
                    onSelect: { terminal in
                        // Dismissed BEFORE the selection is acted on. A pane in
                        // another worktree replaces this route, which takes this
                        // view and the sheet it owns with it — and a sheet whose
                        // presenter has gone is a sheet with nobody left to
                        // close it.
                        showWorkspaceList = false
                        onOpen(terminal)
                    },
                    onDismiss: { showWorkspaceList = false },
                    hosts: hosts
                )
                .navigationTitle("Workspaces")
                .navigationBarTitleDisplayMode(.inline)
            }
        }
        // Which pane is on screen, so a banner about THIS one is suppressed
        // while banners about the others still arrive — see `Notifier`.
        .onAppear {
            markVisible()
            // Also on appear, not only on change. A request set before this
            // view mounted never changes value afterwards, so it would sit
            // there unclaimed and block the next tap on the same card.
            honorRequest()
        }
        .onChange(of: requested?.id) { _, _ in honorRequest() }
        .onChange(of: current.id) { _, _ in markVisible() }
        .onDisappear { Notifier.shared.visibleTerminal = nil }
        .onChange(of: scenePhase) { _, phase in
            guard phase == .active else { return }
            // Coming back to the app is reading whatever it comes back to.
            markVisible()
        }
        // A pane that no longer exists cannot be the one on screen.
        //
        // Removing a terminal used to leave the screen pointed at it; now it
        // also has to come out of `visited`, or its `TerminalView` would stay
        // mounted forever holding a session for a pane the host has forgotten.
        // The Changes tab is never pruned: nothing on the runner has to exist
        // for it, so nothing can stop existing.
        .onChange(of: liveTerminalIDs) { _, ids in prune(to: ids) }
        // Keyboard room is owned by each pane below. The outer host remains
        // full-height so its navigation boundary and tab overlay never move.
        .ignoresSafeArea(.keyboard, edges: .bottom)
    }

    /// What one tab draws.
    ///
    /// The two branches are the two things a workspace is: a pane on the runner,
    /// and the worktree's own diff. The second needs nothing on the runner to
    /// exist — see `Pane`.
    @ViewBuilder
    private func content(of pane: Pane) -> some View {
        switch pane {
        case .terminal(let terminal):
            TerminalView(
                terminal: terminal,
                isVisible: pane.id == current.id,
                connection: connection,
                pastes: pastes)

        case .changes:
            // The store comes from `Connection`, keyed by workspace, so this is
            // the SAME review a `changes` pane in this worktree would show and
            // the same one the Mac is looking at: the scroll position, the
            // folds, the diffs already read and the notes written are one per
            // worktree, not one per way in. See `ChangesStores`.
            //
            // No `isVisible` argument, and it needs none. What that flag buys on
            // a terminal is a stream, a poll and a tmux size assertion that must
            // stop when the pane is merely hidden; `ChangesView` holds none of
            // those. It loads once, on mount, and mounting only happens when
            // somebody selects this tab.
            //
            // `agents` as plain values rather than the `Connection` they come
            // from, for the reason `ChangesView.agents` states: this is the one
            // screen in the app whose body is a forty-card lazy stack somebody
            // is mid-scroll through.
            ChangesView(
                store: connection.changesStores.store(for: workspaceID),
                workspaceName: workspaceName,
                agents: currentWorkspace?.reviewAgentTargets() ?? [])
        }
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        // Only when there is something wrong, and then unmissably. A terminal
        // that has quietly stopped updating is exactly where a frozen link is
        // noticed, and this is the screen people actually sit on.
        if connection.phase != .connected {
            ToolbarItem(placement: .topBarLeading) {
                LinkStatusChip(connection: connection)
            }
        }

        // All trailing controls live in ONE ordered group. `ChangesView` used
        // to contribute Review options from its own toolbar tree; SwiftUI then
        // merged parent and child items in a different order on each pane.
        // Keeping ownership here makes the final button invariant: the
        // worktree switcher is declared last and is always rightmost.
        ToolbarItemGroup(placement: .topBarTrailing) {
            // Terminal or chat, on the pane that can be either. Shown only
            // where the daemon says switching is supported.
            if let live, live.canSwitchPaneMode {
                Button {
                    Task {
                        await connection.setPaneMode(
                            live, to: live.isAgentPane ? "terminal" : "agent")
                    }
                } label: {
                    Image(
                        systemName: live.isAgentPane
                            ? "terminal" : "bubble.left.and.text.bubble.right")
                }
                .accessibilityLabel(live.isAgentPane ? "Show the terminal" : "Show the chat")
            }

            // Only on a pane that is actually a terminal: this sends an image
            // by typing its path into a tty.
            if let live, !live.isAgentPane, !live.isChangesPane {
                Menu {
                    // A plain Button, and the picker hangs off the screen: a
                    // `PhotosPicker` placed directly in a menu renders as a row
                    // and can never present, because menu content is not a live
                    // view hierarchy to present FROM.
                    Button {
                        showPhotoPicker = true
                    } label: {
                        Label("Choose Photo", systemImage: "photo")
                    }
                    if UIPasteboard.general.hasImages {
                        Button {
                            if let image = UIPasteboard.general.image {
                                pastes.send(image, terminal: live.id, core: connection.core)
                            }
                        } label: {
                            Label("Paste Image", systemImage: "doc.on.clipboard")
                        }
                    }
                } label: {
                    Image(systemName: "photo.badge.plus")
                }
                .accessibilityLabel("Send an image")
            }

            // The review's own controls — the base it is compared against, and
            // the notes waiting to be sent. Keyed on the TAB rather than on a
            // pane's mode, because the tab is now the only way this screen shows
            // a diff. A host-side `changes` pane arrives here as the same tab;
            // see `Pane.init(_:)`.
            if case .changes = current {
                ChangesToolbarMenu(store: connection.changesStores.store(for: workspaceID))
            }

            Button { showWorkspaceList = true } label: {
                // `rectangle.stack`, which is the mark the Mac puts on the set
                // of workspaces. Nothing distinguishes it from `square.stack`
                // beyond which app drew it first, and that is the whole reason
                // to stop having two.
                Image(systemName: "rectangle.stack")
            }
            .accessibilityLabel("Switch workspace")
        }
    }

    /// Every terminal the fleet still has, so a pane that has gone can be
    /// dropped. A sorted array rather than a set because `onChange` needs
    /// `Equatable` and wants a stable order.
    private var liveTerminalIDs: [String] {
        connection.fleet.workspaces.flatMap(\.terminals).map(\.id).sorted()
    }

    /// Show the pane a deep link or the switcher sheet asked for, through the
    /// same `select` the tab strip uses — retargeting rather than rebuilding, so
    /// the panes already mounted keep everything they were holding.
    ///
    /// Ignored, and cleared, for a terminal in another worktree. `FleetView`
    /// only ever sends one that is in this one, and honoring anything else would
    /// mean drawing a pane with no chip and a title naming the wrong worktree.
    private func honorRequest() {
        guard let terminal = requested else { return }
        requested = nil
        guard currentWorkspace?.terminals.contains(where: { $0.id == terminal.id }) == true
        else { return }
        select(Pane(terminal))
    }

    /// Show a tab BECAUSE somebody tapped its chip, and remember that they did.
    ///
    /// The only writer of `Connection.lastFocus`, and every other route into
    /// `select` is deliberately not one:
    ///
    /// - `honorRequest()`, which is a tapped Live Activity card or the switcher
    ///   sheet. That tap chose a notification, not a place to work: writing it
    ///   down would let a 3am ping about one agent move where you land when you
    ///   come back to the diff you were reading, which is the loss this whole
    ///   memory exists to prevent.
    /// - The initial pane, which never comes through here at all — it is
    ///   `init`'s argument, and it is usually the focus RULE's answer. Storing
    ///   the rule's answer as a choice would make the memory self-fulfilling:
    ///   every workspace would remember wherever it happened to open, and the
    ///   rule would never get to run again.
    /// - `prune`, where the pane you were reading stopped existing. Being moved
    ///   because the runner took something away is not a preference, and
    ///   recording it would outlive the accident that caused it.
    ///
    /// Written even when the chip tapped is already current. Nothing moves, but
    /// the person did say where they want to be, and a rule-chosen tab they
    /// confirm is one they should come back to.
    private func choose(_ pane: Pane) {
        connection.rememberFocus(pane.focus, in: workspaceID)
        select(pane)
    }

    /// Show a tab, mounting it the first time it is asked for.
    private func select(_ pane: Pane) {
        guard pane.id != current.id else { return }
        if !visited.contains(where: { $0.id == pane.id }) {
            visited.append(pane)
        }
        current = pane
    }

    private func prune(to ids: [String]) {
        let alive = Set(ids)
        // Never prune to nothing: a poll that briefly returns an empty fleet —
        // a reconnect, a host mid-restart — would otherwise unmount every pane
        // and throw away exactly the state this type exists to keep.
        guard !alive.isEmpty else { return }
        visited.removeAll { pane in
            guard let terminal = pane.terminal else { return false }
            return !alive.contains(terminal.id)
        }
        if !visited.contains(where: { $0.id == current.id }) {
            // Changes is the floor. A worktree whose last agent was stopped
            // while you were reading it still has a diff, which is usually why
            // you were there — the alternative is a screen with nothing on it.
            select(visited.first ?? .changes)
        }
    }

    /// Which pane the runner should believe is being read.
    ///
    /// Nil on the Changes tab, and that is the honest answer rather than a gap:
    /// no pane is on screen, so no pane's notification should be suppressed and
    /// no agent's finished turn should be marked seen. `Connection.markVisibleSeen`
    /// reads exactly this and reports an empty watch list for it.
    private func markVisible() {
        Notifier.shared.visibleTerminal = current.terminal?.id
        Task { await connection.markVisibleSeen() }
    }
}

/// Reports the actual UIKit navigation-bar boundary instead of inferring it
/// from SwiftUI's keyboard-dependent safe-area proposal.
private struct NavigationBarBoundaryReader: UIViewRepresentable {
    @Binding var bottom: CGFloat?

    func makeUIView(context: Context) -> NavigationBarBoundaryProbe {
        NavigationBarBoundaryProbe()
    }

    func updateUIView(_ view: NavigationBarBoundaryProbe, context: Context) {
        view.onChange = { measured in
            guard bottom != measured else { return }
            bottom = measured
        }
        view.report()
    }
}

private final class NavigationBarBoundaryProbe: UIView {
    var onChange: ((CGFloat) -> Void)?

    override func didMoveToWindow() {
        super.didMoveToWindow()
        report()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        report()
    }

    func report() {
        guard let window else { return }
        var responder: UIResponder? = self
        while let current = responder {
            if let controller = current as? UIViewController,
                let bar = controller.navigationController?.navigationBar
            {
                let measured = bar.convert(bar.bounds, to: window).maxY
                DispatchQueue.main.async { [weak self] in self?.onChange?(measured) }
                return
            }
            responder = current.next
        }
    }
}
