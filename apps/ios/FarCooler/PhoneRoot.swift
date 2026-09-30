import SwiftUI

// The app's navigation, once any runner has answered: one `NavigationStack`
// whose root is Needs You (spec §6.1).
//
// Needs You is the front door. A workspace is pushed over it, a task over the
// workspace, and a worktree over the task, so Back walks up the way you came
// down. The shell is still what a worktree looks like once you're in one
// (`WorktreeScreen`): the pane pager and the bottom bar, scoped to that one
// worktree, with the overview it used to lift into retired. It covers the
// stack rather than being pushed onto it (`PhoneNavigator.worktree`). Where a launch
// opens and where a link lands are `PhoneLaunch`'s rules, in AgentKit.

extension EnvironmentValues {
    /// Take the screen this is inside off the stack. Set by `WorktreeScreen`,
    /// whose own navigation bar is hidden, for the pane's bar to offer Back.
    @Entry var phoneBack: (() -> Void)? = nil
    /// The phone's stack, for a screen in it to push onto. Nil outside it,
    /// in a harness that mounts the shell alone.
    @Entry var phoneNavigator: PhoneNavigator? = nil
}

/// The phone's stack of screens over Needs You.
///
/// An object rather than a `@State` array and a closure in the environment,
/// and that is not style: a closure is a new value on every pass of the root,
/// so every screen reading it was re-evaluated whenever the root was, and the
/// stack writing its path back on each of those passes made the root pass
/// again, sixty times a second. A segmented control re-rendered under a
/// finger lost the tap. A reference in the environment never changes.
@MainActor
final class PhoneNavigator: ObservableObject {
    /// The screens pushed over Needs You: workspaces and tasks.
    @Published var path: [PhoneRoute] = []
    /// The worktree open over them, if one is.
    ///
    /// Covering the stack rather than pushed onto it: every pane in the
    /// shell carries a `NavigationStack` of its own for its bar, and a stack
    /// pushed inside another one is drawn as neither — the whole stack showed
    /// its root instead. Back on the pane's bar takes the cover away, onto
    /// the screen it was opened from, which is what Back does anywhere.
    @Published var worktree: PhoneWorktree?
    /// Whether somebody moved: opened a screen, or an item or a link did.
    /// A launch decides nothing after that (`PhoneLaunch.decide`).
    var moved = false
    /// Whether the launch has been decided. Once, and never again.
    var decided = false
    /// When the launch began, for `PhoneLaunch.decideWithin`.
    let began = Date()

    /// Open one screen over the one showing.
    func open(_ route: PhoneRoute) {
        moved = true
        if let cover = PhoneWorktree(route) {
            worktree = cover
            return
        }
        worktree = nil
        // Already the screen under the cover, as a task is when its agent's
        // pane chip opens it: going back to it, not stacking a second copy.
        if path.last != route { path.append(route) }
    }

    /// Replace everything, from Needs You: an item opened, or a link followed.
    func go(_ stack: [PhoneRoute]) {
        moved = true
        if let last = stack.last, let cover = PhoneWorktree(last) {
            path = Array(stack.dropLast())
            worktree = cover
        } else {
            path = stack
            worktree = nil
        }
    }
}

/// A worktree route, as the cover over the stack presents it.
struct PhoneWorktree: Identifiable, Equatable {
    let runner: String
    let worktree: String
    let landing: WorktreeLanding

    init?(_ route: PhoneRoute) {
        guard case .worktree(let runner, let worktree, let landing) = route else { return nil }
        self.runner = runner
        self.worktree = worktree
        self.landing = landing
    }

    var id: String { "\(runner)/\(worktree)/\(landing)" }
}

/// The stack, its root, and where it opens.
struct PhoneRoot: View {
    @ObservedObject var fleet: FleetStore
    @ObservedObject var hosts: RunnerStore
    /// A tapped notification or card's terminal, held by `FleetView` until a
    /// runner's fleet has it. See `followLink`.
    @Binding var pendingTerminal: String?

    @StateObject private var navigator = PhoneNavigator()

    var body: some View {
        NavigationStack(path: $navigator.path) {
            NeedsYouScreen(fleet: fleet, hosts: hosts, open: navigator.go)
                .navigationDestination(for: PhoneRoute.self) { route in
                    destination(route)
                }
        }
        .fullScreenCover(item: $navigator.worktree) { cover in
            Group {
                if let id = UUID(uuidString: cover.runner) {
                    WorktreeScreen(
                        fleet: fleet, hosts: hosts,
                        scope: ShellScope(runner: id, worktree: cover.worktree, landing: cover.landing))
                } else {
                    RunnerGone()
                }
            }
            .environment(\.phoneNavigator, navigator)
        }
        .environment(\.phoneNavigator, navigator)
        .onAppear {
            followLink()
            decideLaunch()
        }
        .task {
            // Once more at the limit, when a runner that never answers can
            // no longer hold the decision open.
            try? await Task.sleep(for: .seconds(PhoneLaunch.decideWithin))
            decideLaunch()
        }
        .onChange(of: fleet.needsYouReadings) { _, _ in decideLaunch() }
        .onChange(of: pendingTerminal) { _, _ in followLink() }
        .onChange(of: fleet.entries.count) { _, _ in followLink() }
    }

    @ViewBuilder
    private func destination(_ route: PhoneRoute) -> some View {
        switch route {
        case .workspace(let place):
            if let connection = fleet.connection(for: place) {
                WorkspaceScreen(fleet: fleet, hosts: hosts, connection: connection, place: place)
            } else {
                RunnerGone()
            }
        case .task(let place, let task):
            if let connection = fleet.connection(for: place) {
                TaskScreen(connection: connection, place: place, task: task)
            } else {
                RunnerGone()
            }
        case .worktree:
            // Never pushed: a worktree covers the stack. See
            // `PhoneNavigator.worktree`.
            EmptyView()
        }
    }

    /// Where the app opens, once every runner has said what needs you
    /// (ruling 4). Nothing is pushed over a stack somebody has already moved.
    private func decideLaunch() {
        guard !navigator.decided else { return }
        let last = UserDefaults.standard.string(forKey: PhoneLaunch.lastWorkspaceKey)
            .flatMap(PhoneWorkspace.init(stored:))
        switch PhoneLaunch.decide(
            fleet.needsYouReadings, elapsed: Date().timeIntervalSince(navigator.began),
            moved: navigator.moved, linking: pendingTerminal != nil,
            itemCount: fleet.needsYou.count, last: last,
            exists: { fleet.connection(for: $0)?.workspace($0.workspace) != nil })
        {
        case .wait:
            return
        case .stay:
            navigator.decided = true
        case .open(let stack):
            navigator.decided = true
            navigator.path = stack
        }
    }

    /// A tapped notification, once a runner's fleet has its terminal: its
    /// workspace, its task, then the pane (`Fleet.phoneLink`).
    private func followLink() {
        guard let id = pendingTerminal else { return }
        for runner in fleet.runners {
            guard
                let link = runner.connection.fleet.phoneLink(
                    toTerminal: id, runner: runner.host.id.uuidString)
            else { continue }
            if let segment = link.segment, case .workspace(let place)? = link.stack.last {
                segment.remember(for: place)
            }
            pendingTerminal = nil
            navigator.go(link.stack)
            return
        }
    }
}

/// A screen whose runner this app has stopped talking to.
private struct RunnerGone: View {
    var body: some View {
        ContentUnavailableView {
            Label("Runner Not Connected", systemImage: "server.rack")
        } description: {
            Text("Far Cooler isn’t talking to this runner right now.")
        }
    }
}

extension FleetStore {
    /// The connection a workspace's runner is on, or nil when it's not being
    /// talked to.
    func connection(for place: PhoneWorkspace) -> Connection? {
        UUID(uuidString: place.runner).flatMap { connection(for: $0) }
    }

    /// The runner a workspace is on, as a person named it.
    func runner(_ id: String) -> Runner? {
        runners.first { $0.host.id.uuidString == id }?.host
    }
}

extension Connection {
    /// Every workspace this runner has, as the board list names them: each
    /// repository's, Main first, or its one implicit workspace on a runner
    /// without workspaces.
    func workspace(_ id: String) -> WorkspaceSummary? {
        boardList.first { $0.id == id }
    }
}

/// One worktree, pushed: the shell scoped to it (spec §6.1).
///
/// Today's pane pager and bottom bar, over this worktree's panes alone:
/// swiping the bar sideways moves between its terminals, not across the
/// fleet, and the lift that reached the overview reaches only the column.
/// It covers the stack rather than being pushed onto it, since every pane
/// has a navigation stack of its own for its bar; the pane's bar carries
/// Back (`phoneBack`), which takes the cover away.
struct WorktreeScreen: View {
    @ObservedObject var fleet: FleetStore
    @ObservedObject var hosts: RunnerStore
    let scope: ShellScope

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        ShellScreen(fleet: fleet, hosts: hosts, pendingTerminal: .constant(nil), scope: scope)
            .environment(\.phoneBack, { dismiss() })
            // VoiceOver's two-finger scrub, which a pushed screen gets for
            // free and a cover doesn't.
            .accessibilityAction(.escape) { dismiss() }
    }
}
