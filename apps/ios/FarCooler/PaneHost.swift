import PhotosUI
import SwiftUI
import UIKit

/// Every pane you have opened in this workspace, all still there.
///
/// One `TerminalView` was reused for every pane before this: switching tabs
/// pointed it at a different terminal, which meant SwiftUI tore down whatever
/// the old pane had built. A chat lost its scroll position and its half-typed
/// message, a review lost which files were open and where you were reading, and
/// a terminal renegotiated its size with tmux on every visit — which is the
/// content jumping around on open.
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
/// Only panes actually VISITED are mounted. Mounting the whole fleet would pay
/// setup for panes nobody opens, and a workspace's handful of visited panes is
/// what "the tabs I am working in" actually means.
@MainActor
struct PaneHost: View {
    @ObservedObject var connection: Connection
    let hosts: HostStore?

    @State private var current: Terminal
    /// Visited panes, oldest first. An array rather than a set so the order is
    /// stable — SwiftUI identity in a `ForEach` is the whole mechanism here.
    @State private var visited: [Terminal]
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

    init(terminal: Terminal, connection: Connection, hosts: HostStore? = nil) {
        self.connection = connection
        self.hosts = hosts
        _current = State(initialValue: terminal)
        _visited = State(initialValue: [terminal])
    }

    /// The current terminal as the daemon describes it RIGHT NOW.
    ///
    /// `current` is a copy taken when the pane was opened, which is right for
    /// identity and wrong for pane mode — switching a pane to chat has to change
    /// what the toolbar offers.
    private var live: Terminal {
        guard let workspace = currentWorkspace?.id else { return current }
        return connection.terminal(current.id, in: workspace) ?? current
    }

    private var currentWorkspace: Workspace? {
        connection.fleet.workspaces.first { $0.terminals.contains { $0.id == current.id } }
    }

    private var currentName: String {
        current.displayName(ordinal: currentWorkspace?.ordinals()[current.id])
    }

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
                        TerminalView(
                            terminal: pane,
                            isVisible: pane.id == current.id,
                            connection: connection,
                            pastes: pastes)
                            // Hidden, not removed. `opacity` keeps the view in the
                            // hierarchy — which is what preserves its state — while
                            // `allowsHitTesting` stops a pane nobody can see from
                            // swallowing taps meant for the one on top of it.
                            .opacity(pane.id == current.id ? 1 : 0)
                            .allowsHitTesting(pane.id == current.id)
                            // Never recycled onto a different terminal.
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
                    workspaces: connection.fleet.workspaces, current: current, onSelect: select
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
        // is true regardless of which of its panes is focused.
        .navigationTitle(currentWorkspace?.task ?? currentName)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { toolbar }
        .photosPicker(isPresented: $showPhotoPicker, selection: $pickedImage, matching: .images)
        .onChange(of: pickedImage) { _, item in
            guard let item else { return }
            let target = live.id
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
        .sheet(isPresented: $showWorkspaceList) {
            NavigationStack {
                WorkspaceListView(
                    connection: connection,
                    onSelect: { terminal in
                        select(terminal)
                        showWorkspaceList = false
                    },
                    onDismiss: { showWorkspaceList = false },
                    hosts: hosts
                )
                .navigationTitle("Worktrees")
                .navigationBarTitleDisplayMode(.inline)
            }
        }
        // Which pane is on screen, so a banner about THIS one is suppressed
        // while banners about the others still arrive — see `Notifier`.
        .onAppear { markVisible() }
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
        .onChange(of: liveTerminalIDs) { _, ids in prune(to: ids) }
        // Keyboard room is owned by each pane below. The outer host remains
        // full-height so its navigation boundary and tab overlay never move.
        .ignoresSafeArea(.keyboard, edges: .bottom)
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

        ToolbarItem(placement: .principal) {
            VStack(spacing: 0) {
                Text(currentWorkspace?.task ?? currentName)
                    .font(.headline)
                    .lineLimit(1)
                if let branch = currentWorkspace?.branch {
                    Text(branch)
                        .font(.caption2.monospaced())
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
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
            if live.canSwitchPaneMode {
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
            if !live.isAgentPane && !live.isChangesPane {
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

            if live.isChangesPane, let workspace = currentWorkspace {
                ChangesToolbarMenu(store: connection.changesStores.store(for: workspace.id))
            }

            Button { showWorkspaceList = true } label: {
                Image(systemName: "square.stack")
            }
            .accessibilityLabel("Switch terminal")
        }
    }

    /// Every terminal the fleet still has, so a pane that has gone can be
    /// dropped. A sorted array rather than a set because `onChange` needs
    /// `Equatable` and wants a stable order.
    private var liveTerminalIDs: [String] {
        connection.fleet.workspaces.flatMap(\.terminals).map(\.id).sorted()
    }

    /// Show a pane, mounting it the first time it is asked for.
    private func select(_ terminal: Terminal) {
        guard terminal.id != current.id else { return }
        if !visited.contains(where: { $0.id == terminal.id }) {
            visited.append(terminal)
        }
        current = terminal
    }

    private func prune(to ids: [String]) {
        let alive = Set(ids)
        // Never prune to nothing: a poll that briefly returns an empty fleet —
        // a reconnect, a host mid-restart — would otherwise unmount every pane
        // and throw away exactly the state this type exists to keep.
        guard !alive.isEmpty else { return }
        visited.removeAll { !alive.contains($0.id) }
        if !alive.contains(current.id), let next = visited.first {
            current = next
        }
    }

    private func markVisible() {
        Notifier.shared.visibleTerminal = current.id
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
