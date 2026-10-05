import SwiftUI

// The app's navigation, once any runner has answered: one `NavigationStack`
// whose root is Needs You (spec §6.1).
//
// Needs You is the front door. A workspace is pushed over it, a task over the
// workspace, and a worktree over the task, so Back walks up the way you came
// down. The shell is still what a worktree looks like once you're in one
// (`WorktreeScreen`): the pane pager and the bottom bar, scoped to that one
// worktree. It covers the stack rather than being pushed onto it
// (`PhoneNavigator.worktree`). Where a launch
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
    @Published var path: [PhoneRoute] = [] { didSet { keep() } }
    /// The worktree open over them, if one is.
    ///
    /// Covering the stack rather than pushed onto it: every pane in the
    /// shell carries a `NavigationStack` of its own for its bar, and a stack
    /// pushed inside another one is drawn as neither — the whole stack showed
    /// its root instead. Back on the pane's bar takes the cover away, onto
    /// the screen it was opened from, which is what Back does anywhere.
    @Published var worktree: PhoneWorktree? { didSet { keep() } }
    /// Whether somebody moved: opened a screen, or an item or a link did.
    /// A launch decides nothing after that (`PhoneLaunch.decide`).
    var moved = false
    /// Whether the launch has been decided. Once, and never again.
    var decided = false
    /// When the launch began, for `PhoneLaunch.decideWithin`.
    let began = Date()

    /// The stack as `PhoneLaunch.stackKey` keeps it: the pushed screens,
    /// then the worktree over them.
    ///
    /// A worktree is kept to resume, not on the pane it was opened on: the
    /// pane last chosen in it is `ShellFleetMap.resume`'s to remember, and
    /// it may have moved since.
    var stack: [PhoneRoute] {
        let cover = worktree ?? pending?.cover
        return path + (cover.map { [.worktree(runner: $0.runner, worktree: $0.worktree, landing: .resume)] } ?? [])
    }

    /// A worktree waiting to cover the stack until the screens under it
    /// have landed (see `place`).
    private var pending: PendingCover?

    /// Keep where the phone is, for the next launch to reopen (ruling 1).
    ///
    /// Not before the launch has decided or somebody has moved: until then
    /// the stack is the empty one every launch starts on, and keeping it
    /// would throw away the one the last run left.
    func keep(in defaults: UserDefaults = .standard) {
        guard decided || moved else { return }
        defaults.set(PhoneLaunch.encode(stack), forKey: PhoneLaunch.stackKey)
    }

    /// Open one screen over the one showing.
    func open(_ route: PhoneRoute) {
        moved = true
        pending = nil
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
        place(stack)
    }

    /// Reopen `stack`, as a launch does: no move of anybody's.
    func reopen(_ stack: [PhoneRoute]) { place(stack) }

    /// Stand `stack` up whole: its screens pushed, then its worktree over
    /// them, in that order and never in one pass (ov-337).
    ///
    /// A cover presented in the same pass as the pushes under it races them:
    /// whenever the presentation began first, which a loaded simulator made
    /// it do one run in two, the screens were pushed while it was in flight,
    /// and the workspace pushed then lost its title the moment it came back
    /// to the top. Its bar stayed unnamed for good, the bare
    /// `NavigationStackHosting` CI read after Back from a notification's task
    /// and from a relaunched one. Pushed first, the title holds. So the
    /// worktree waits for the top screen under it to appear (`appeared`),
    /// and covers at once when the stack is already that.
    private func place(_ stack: [PhoneRoute]) {
        guard let last = stack.last, let cover = PhoneWorktree(last) else {
            pending = nil
            path = stack
            worktree = nil
            return
        }
        let under = Array(stack.dropLast())
        if under == path {
            pending = nil
            worktree = cover
            return
        }
        let waiting = PendingCover(cover: cover, under: under)
        pending = waiting
        worktree = nil
        push(under, for: waiting)
        // The backstop, for a top screen that never says it appeared: the
        // place still opens, late rather than never. It trades correctness
        // for liveness: a top screen slower than this to appear gets its
        // cover while its push may still be landing, which is the race
        // above, back for that one case.
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(2))
            self?.land(waiting)
        }
    }

    /// Push the screens a held worktree waits on: at once, or under
    /// `-phone-stack-lags` 0.3 s late, as a loaded simulator's stack took
    /// them on CI. Only the push lags; everything around it is what ships,
    /// so a cover put up in the same pass as the pushes gets them while
    /// it's still going up.
    private func push(_ under: [PhoneRoute], for waiting: PendingCover) {
        #if DEBUG
        if PhoneHarness.stackLags {
            Task { [weak self] in
                try? await Task.sleep(for: .milliseconds(300), tolerance: .zero)
                guard self?.pending?.id == waiting.id else { return }
                self?.path = under
            }
            return
        }
        #endif
        path = under
    }

    /// The screen on top of the stack appeared: `route`, or Needs You for
    /// nil. A worktree waiting on it covers it now.
    func appeared(_ route: PhoneRoute?) {
        guard let waiting = pending, route == waiting.under.last else { return }
        land(waiting)
    }

    private func land(_ waiting: PendingCover) {
        // Only over the stack it was placed on: somebody who went Back
        // before it landed has moved past it.
        guard pending?.id == waiting.id else { return }
        pending = nil
        guard path == waiting.under else {
            // Kept again without it, so a relaunch doesn't reopen a
            // worktree nobody saw: `stack` counted it while it was held.
            keep()
            return
        }
        worktree = waiting.cover
    }
}

/// A worktree held back from covering the stack until the screens it was
/// placed over are on it (`PhoneNavigator.place`).
private struct PendingCover {
    let id = UUID()
    let cover: PhoneWorktree
    let under: [PhoneRoute]
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
    /// What a tapped notification or card is about, held by `FleetView` until
    /// this takes it up. See `follow`.
    @Binding var pendingDestination: Destination?

    @StateObject private var navigator = PhoneNavigator()
    /// What waits for its runner to open: the tap just taken up, or the place
    /// the last run was in. One at a time, and a tap outranks a relaunch.
    @State private var waiting: PhoneWaiting?

    var body: some View {
        NavigationStack(path: $navigator.path) {
            NeedsYouScreen(fleet: fleet, hosts: hosts, open: navigator.go)
                .onAppear { navigator.appeared(nil) }
                .navigationDestination(for: PhoneRoute.self) { route in
                    destination(route)
                        .onAppear { navigator.appeared(route) }
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
            #if DEBUG
            .overlay(alignment: .topLeading) { PhoneProbeView() }
            #endif
        }
        .environment(\.phoneNavigator, navigator)
        #if DEBUG
        .overlay(alignment: .topLeading) { PhoneProbeView().offset(y: 2) }
        #endif
        .onAppear {
            takeArrival()
            decideLaunch()
        }
        .task {
            // Asked again until it's decided, and at the limit, when a
            // runner that never answers can no longer hold it open. A
            // saved stack's task waits on its board, which no change here
            // is told about.
            while !navigator.decided, !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(500))
                decideLaunch()
            }
        }
        .task(id: waiting?.id) { await follow() }
        .onChange(of: fleet.needsYouReadings) { _, _ in decideLaunch() }
        .onChange(of: pendingDestination) { _, _ in takeArrival() }
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
        case .history(let place, let status):
            if let connection = fleet.connection(for: place), let status = TaskStatus(rawValue: status) {
                BoardHistoryScreen(connection: connection, place: place, status: status)
            } else {
                RunnerGone()
            }
        case .plan(let place, let page):
            if let connection = fleet.connection(for: place) {
                PlanPageScreen(connection: connection, place: place, page: page)
            } else {
                RunnerGone()
            }
        case .worktree:
            // Never pushed: a worktree covers the stack. See
            // `PhoneNavigator.worktree`.
            EmptyView()
        }
    }

    /// Where the app opens (ruling 4, ov-182). Nothing is pushed over a stack
    /// somebody has already moved.
    ///
    /// A place the last run kept reopens where it was, whatever is waiting
    /// on Needs You (ruling 1): held for its runner, then opened, or opened
    /// at the nearest level still there when it's gone (`follow`). With none
    /// kept, Needs You decides once every runner has said what it holds, as
    /// `PhoneLaunch.decide` says.
    private func decideLaunch() {
        guard !navigator.decided, waiting == nil else { return }
        let saved = PhoneLaunch.decode(UserDefaults.standard.data(forKey: PhoneLaunch.stackKey))
        if let kept = Destination(phoneStack: saved) {
            waiting = PhoneWaiting(destination: kept, arrival: .restore, stack: saved)
            return
        }
        switch PhoneLaunch.decide(
            fleet.needsYouReadings, elapsed: Date().timeIntervalSince(navigator.began),
            moved: navigator.moved, linking: false,
            itemCount: fleet.needsYou.count, last: lastWorkspace,
            exists: { fleet.connection(for: $0)?.workspace($0.workspace) != nil })
        {
        case .wait:
            return
        case .stay:
            // The kept stack stays kept: a launch that gave up on a runner
            // that wasn't answering reopens it next time, and one whose
            // screen is gone is written over by the first move.
            navigator.decided = true
        case .open(let stack):
            navigator.decided = true
            navigator.reopen(stack)
        }
    }

    /// The workspace last opened, which a restore falls back to.
    private var lastWorkspace: PhoneWorkspace? {
        UserDefaults.standard.string(forKey: PhoneLaunch.lastWorkspaceKey)
            .flatMap(PhoneWorkspace.init(stored:))
    }

    /// A tapped notification or card (ruling 3, ov-183), taken from
    /// `FleetView`, which holds it from the moment it arrives, a cold launch
    /// included. It outranks a relaunch still waiting.
    private func takeArrival() {
        guard let destination = pendingDestination else { return }
        pendingDestination = nil
        waiting = PhoneWaiting(destination: destination, arrival: .notification)
    }

    /// What the phone holds now, for the resolver: the runners being talked
    /// to, and the paired ones nothing is connecting, which a tap is told to
    /// connect when it names one.
    private func destinationSources() -> [PhoneDestination.Source] {
        let connected = fleet.runners.map { runner in
            let connection = runner.connection
            return PhoneDestination.Source(
                host: runner.host.id.uuidString, runnerId: connection.lastDaemon?.runnerId,
                ready: connection.phase == .connected && connection.hasFleet && connection.lastDaemon != nil,
                idle: false, fleet: connection.fleet, boardList: connection.boardList,
                boards: connection.boards)
        }
        // A paired runner nothing is connecting is seated with the id it said
        // when it last connected, so a push naming it can connect it (ov-231).
        return PhoneDestination.sources(
            connected: connected, paired: hosts.hosts.map { $0.id.uuidString }, known: RunnerIds().all,
            everyRunner: FleetSettings.allRunnersAtOnce, selected: hosts.selected?.id.uuidString)
    }

    /// Open what waits, asked again as the runners come up, until it opens
    /// or the resolver drops it (`DestinationResolver`): a tap waits a minute
    /// for a runner and its task or pane, a relaunch ten seconds, and neither
    /// opens late, over something somebody has gone on to read.
    private func follow() async {
        while let current = waiting, !Task.isCancelled {
            let sources = destinationSources()
            let deadline =
                current.arrival == .restore
                ? DestinationResolver.Deadline.restore : DestinationResolver.Deadline.notificationPhone
            switch DestinationResolver.resolve(
                current.destination, arrival: current.arrival,
                in: PhoneDestination.world(sources, last: lastWorkspace),
                elapsed: Date().timeIntervalSince(current.began), deadline: deadline,
                interrupted: navigator.moved)
            {
            case .wait:
                break
            case .connect(let host):
                // Paired, and nothing is dialing it: dial it, and ask again.
                hosts.selected = hosts.hosts.first { $0.id.uuidString == host }
            case .open(let open, let fellBack):
                let link = PhoneDestination.link(
                    for: open, fleet: sources.first { $0.host == open.runner.host }?.fleet)
                if let segment = link.segment, case .workspace(let place)? = link.stack.last {
                    segment.remember(for: place)
                }
                waiting = nil
                navigator.decided = true
                if current.arrival == .restore {
                    // What was kept, whole, while its deepest screen is still
                    // there: the task a worktree was opened from stays under
                    // it. Gone, the nearest level that is.
                    navigator.reopen(fellBack ? link.stack : current.stack ?? link.stack)
                } else {
                    navigator.go(link.stack)
                }
                return
            case .stay:
                // A tap whose subject isn't to be found, or a relaunch somebody
                // moved past: the app stays where it is, and the kept stack
                // stays kept.
                waiting = nil
                navigator.decided = true
                return
            }
            try? await Task.sleep(for: .milliseconds(500))
        }
    }
}

/// A destination waiting for its runner, and how it came: a tapped
/// notification, or the last run's place on a relaunch.
struct PhoneWaiting: Identifiable, Equatable {
    let id = UUID()
    let destination: Destination
    let arrival: DestinationResolver.Arrival
    /// The stack a relaunch kept, whole, for `follow` to reopen when its
    /// deepest screen is still there.
    let stack: [PhoneRoute]?
    let began = Date()

    init(destination: Destination, arrival: DestinationResolver.Arrival, stack: [PhoneRoute]? = nil) {
        self.destination = destination
        self.arrival = arrival
        self.stack = stack
    }
}

#if DEBUG
/// What a UI test can't see from the screen: which orchestrator panes are
/// live (their session open, rather than mounted and stopped), which pane
/// `Notifier` holds as being read, and how many screens the stack kept.
@MainActor
final class PhoneProbe {
    static let shared = PhoneProbe()
    /// Every orchestrator pane that has been mounted, by terminal.
    var orchestrators: Set<String> = []
}

/// `phone-probe`: `live=<ids> watch=<id> kept=<n>`, sampled, since none of
/// it is observable. On the stack and on a worktree's cover, so it's read
/// wherever the test is.
private struct PhoneProbeView: View {
    var body: some View {
        TimelineView(.periodic(from: .now, by: 0.25)) { _ in
            let live = PhoneProbe.shared.orchestrators.filter(TerminalSession.running.contains)
                .sorted()
            let kept = PhoneLaunch.decode(
                UserDefaults.standard.data(forKey: PhoneLaunch.stackKey)).count
            Rectangle()
                .fill(Color.white.opacity(0.001))  // style-exempt: DEBUG probe: a near-invisible hit target the UI tests read, not a fill
                .frame(width: 1, height: 1)  // style-exempt: DEBUG probe: a near-invisible hit target the UI tests read, not a fill
                .accessibilityElement()
                .accessibilityIdentifier("phone-probe")
                .accessibilityValue(
                    "live=\(live.joined(separator: ",")) "
                        + "watch=\(Notifier.shared.visibleTerminal ?? "") kept=\(kept)")
        }
    }
}
#endif

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
/// fleet, and a lift reaches only the column.
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
