import SwiftUI

/// Where the phone is, as a value.
///
/// Every screen this stack can push is a case here, and `FleetView` renders
/// `[Route]` as its navigation path. That is what turns "what does Back mean"
/// into a question about the path rather than about which view happened to
/// declare a destination: appending never truncates, so a screen reached two
/// ways sits at two different depths and Back is the screen you came from in
/// both. The shape this replaced, and the bug it had, are on `FleetView.path`.
///
/// Ids and not values, because the path is `Codable` and persisted — see
/// `FleetView.savedPath`. A `Terminal` is a snapshot of what the daemon said a
/// moment ago; persisting one would restore a screen built from a description
/// of the world that has since moved on. An id either still names something on
/// this runner or it does not, and the second answer is one this screen can act
/// on.
///
/// `.review` and `.terminal` were two routes here and are one now. They only
/// ever differed in what the screen opened ONTO — a diff or a transcript — and
/// the owner's account of reviewing an agent's work is that those are two views
/// of one thing you move between constantly: read what it said it did, judge
/// it, look at the change, reply. Two destinations made that a Back and a
/// second navigation. One destination with a FOCUS makes it a chip.
enum Route: Hashable, Codable {
    /// Every workspace on this runner, and the toolbar that starts new work.
    case workspaces
    /// One worktree — its agents and its diff — with one of them showing.
    case workspace(id: String, focus: Focus)

    /// Which tab a workspace opens on.
    ///
    /// Part of the route rather than state inside the screen because it is the
    /// only thing the two doors into a workspace disagree about: the inbox
    /// knows a workspace needs you and not which pane of it, the workspace list
    /// knows exactly which agent was tapped.
    ///
    /// `.none` is the honest value for the first of those, and it is not a
    /// missing focus — it is "apply the rule", resolved by
    /// `Route.Focus.rule(for:inbox:)` at the moment the screen is built. That
    /// matters most for a RESTORED path: a workspace saved at midnight and
    /// reopened at seven should open on whatever needs you at seven, not on the
    /// agent that needed you then.
    enum Focus: Hashable, Codable {
        /// One agent, by terminal id.
        case agent(String)
        /// The worktree's own diff.
        case changes
        /// No opinion — see `rule(for:inbox:)`.
        case none
    }

    /// The workspace this route is about, or nil for the ones that are not
    /// about one. Asked by the two places that care: a deep link arriving while
    /// a workspace is open, and a workspace that left the fleet underneath one.
    var workspaceID: String? {
        if case .workspace(let id, _) = self { return id }
        return nil
    }
}

extension Route.Focus {
    /// Which tab a workspace opens on when nobody said.
    ///
    /// Blocked agent, then unread diff, then whatever the fleet list would put
    /// at the top. In that order because it is the order of "what did you open
    /// this for": an agent that stopped to ask is waiting on you, a diff that
    /// moved is waiting to be read, and a workspace where neither is true is
    /// one you went looking for rather than one that called.
    ///
    /// The blocked agent is chosen by `sortRank` — `farcooler_core::feed::rank`,
    /// computed on the runner — with the terminal id as a tiebreak, because
    /// ranks genuinely collide and `min(by:)` over a collision has to land on
    /// the same agent every time or the screen opens somewhere different on
    /// each poll.
    ///
    /// One function, called from the inbox row, from a restored path, and from
    /// a focus naming an agent that has since gone. Three copies of this would
    /// be three answers to "where does this workspace open".
    static func rule(for workspace: Workspace, inbox: InboxRow?) -> Route.Focus {
        // A `changes` pane the host happens to have open is not an agent and is
        // not a candidate: the diff it shows is the Changes tab, which is
        // already the second branch below.
        let panes = workspace.terminals.filter { !$0.isChangesPane }

        if let blocked = panes.filter({ $0.agent == .blocked })
            .min(by: { ($0.sortRank, $0.id) < ($1.sortRank, $1.id) })
        {
            return .agent(blocked.id)
        }
        // Both halves, and they are not the same fact. `hasDiff` is true of
        // every worktree with work on it and stays true after you have read it;
        // `changedSinceReviewed` is the daemon's watermark, and it is what makes
        // this "there is something new here" rather than "there is a branch
        // here". `NeedsYouView.items` gates its second tier on the same pair.
        if let inbox, inbox.changedSinceReviewed, inbox.hasDiff { return .changes }
        if let top = panes.min(by: { ($0.sortRank, $0.id) < ($1.sortRank, $1.id) }) {
            return .agent(top.id)
        }
        // A workspace with no panes at all still has a diff to read, and that
        // is the whole reason Changes needs no pane to exist behind it.
        return .changes
    }
}

/// One runner, and everything the phone shows of it.
///
/// This used to be the screen between a host and a terminal, and then mostly a
/// hand-off: it picked a terminal on connect — `fleet.landingTerminal` — and
/// the worktree list existed only for a runner that had none. It no longer
/// lands anywhere. A connected phone opens onto `NeedsYouView`, which is the
/// answer to the question the app is actually opened with, and the fleet list
/// is one tap below it. See that view's own comment, and
/// `docs/jobs-to-be-done.md` for why the front door changed.
///
/// What did not change: every state shown here is DERIVED by the daemon at the
/// moment of asking. The phone never computes a terminal's state, because a
/// client that re-derives can disagree with the daemon and with the Mac about
/// the same terminal.
@MainActor
struct FleetView: View {
    let host: Runner
    let store: RunnerStore

    @StateObject private var connection = Connection()

    /// Where this screen is, as an explicit list of pushes.
    ///
    /// This was `@State private var landing: Terminal?`, drawn by a
    /// `.navigationDestination(item:)` declared on the ROOT of the stack, and
    /// that shape is what put Back on the wrong screen. SwiftUI resolves a
    /// destination at the depth where it is declared, so assigning `landing`
    /// from the workspace list did not COVER that list — it truncated the
    /// implicit path to depth 0 and pushed the pane as the single element, and
    /// backing out of it landed on Needs You.
    ///
    /// The comment that stood here defended that single root destination as the
    /// thing giving Back one meaning, and the bug it guarded against was real:
    /// an earlier build had the pane reachable both as the stack root and as a
    /// push, so whether a terminal had a back button at all depended on whether
    /// the fleet happened to be empty when this screen connected, and backing
    /// out of a pushed terminal landed on a list that usually had a terminal to
    /// go straight back into. One screen at two depths is one screen with two
    /// answers to "what does Back mean here".
    ///
    /// An explicit path buys a stronger invariant than that fix did, and buys
    /// it without making either answer wrong. Every navigation is an APPEND or
    /// a system pop; appending never truncates; so a pane is at depth 1 when it
    /// was opened from the inbox and at depth 2 when it was opened from the
    /// workspace list, and Back is the screen you came from by construction
    /// rather than by special case. One destination TYPE at any depth, in place
    /// of one destination at one depth.
    @State private var path: [Route] = []

    /// The path this scene was last on, as JSON, so being interrupted does not
    /// cost you your place.
    ///
    /// `docs/jobs-to-be-done.md` F4 is the owner saying the phone has to
    /// survive being put down every ninety seconds. `landing` restored nothing
    /// at all, and could not: a decoded `Terminal` is a value with no identity
    /// anything could write down. A path of ids has one.
    ///
    /// The runner is stored beside the routes because `@SceneStorage` is keyed
    /// by the scene rather than by this view, so one value outlives the
    /// `.id(host)` rebuild that switching runners performs — and terminal ids
    /// are per-runner, so a path saved on one names nothing on another. A
    /// string rather than `Data` only so it is legible when this misbehaves.
    @SceneStorage("fleet.path") private var savedPath = ""

    /// Whether the saved path has had its one chance to come back. See
    /// `restorePath` for why it gets exactly one.
    @State private var restoredPath = false

    /// Whether the runner has told us what it has, at least once.
    ///
    /// On the connection rather than in a `@State` here, and that is not
    /// bookkeeping — it is the difference between an honest screen and a
    /// flickering lie. `phase == .connected` is set BEFORE the first `fleet`
    /// call is awaited, in both `start` and `reconnect`, so a flag flipped on
    /// the phase would draw "Nothing needs you" over `Fleet.empty` for a whole
    /// SSH round trip. And a flag set after `connect(_:)` returns would never
    /// be set at all for the connection that failed, gave up, and then came
    /// back through `reconnectNow` — which is the ordinary way out of the
    /// failure screen. `Connection.hasFleet` is set by the read itself, which
    /// is the only moment that actually answers the question.
    private var hasFleet: Bool { connection.hasFleet }

    /// The terminal a tapped Live Activity card asked for, held until a fleet
    /// arrives that has it.
    ///
    /// A card tapped at COLD LAUNCH delivers its URL before the first
    /// connection has produced a fleet, so looking the id up as it arrives
    /// finds nothing and the tap opens the app onto whatever it would have
    /// opened onto anyway. That is indistinguishable from a card that ignored
    /// the tap, which is the failure this whole task exists to remove — so the
    /// id is remembered instead and `openRequested` runs again once there is a
    /// fleet to look in.
    @State private var pendingTerminal: String?

    /// The terminal to show, handed to `WorkspaceView` and cleared the moment
    /// it takes it.
    ///
    /// Only ever a pane in the workspace already on screen — `show(_:)` sends
    /// anything else through the path instead — because this is the retarget
    /// channel, and retargeting is the thing that keeps mounted panes alive.
    ///
    /// One-shot rather than a record of which terminal the card chose: the tab
    /// strip moves on afterwards without telling this screen, so a second card
    /// for the SAME terminal has to read as a new request rather than as the
    /// value this already holds — which would change nothing and open nothing.
    @State private var requested: Terminal?

    /// Whether to offer a way off the spinner yet. See `waitedLongEnough`.
    @State private var stalled = false

    @Environment(\.scenePhase) private var scenePhase

    /// Open when correcting this runner's details, from any phase that has a
    /// reason to doubt them.
    @State private var editing = false

    var body: some View {
        // The stack lives here rather than in `RootView`, and that is not
        // tidying. `path` and `Connection` have to be the same view's state:
        // every route is an id that means something only against THIS runner's
        // fleet, and resolving one, validating a restored one, and popping a
        // pane when the fleet empties are all reads of `connection`. A stack a
        // level up would have put the path above the only data that can say
        // what it names.
        //
        // `RootView`'s stated reason for owning it is not lost, because it was
        // never a reason for owning it — it was a reason for the stack to
        // EXIST. `FleetView` was previously pushed from the host list and
        // inherited that screen's stack; opening straight onto it left no
        // navigation bar, so no title, no terminal/chat switch, and nothing for
        // a destination to push into. Declaring the stack here satisfies that
        // in full. The `.id(host)` that rebuilds everything below on a runner
        // change stays one level up, and now rebuilds this stack and its path
        // along with the connection — which is right, since a path naming
        // terminals on one runner names nothing on another.
        NavigationStack(path: $path) {
            phases
            // Every screen this stack can push, declared once.
            //
            // On the stack's ROOT content, which is where a
            // `navigationDestination(for:)` belongs: it is inherited by every
            // depth below, so a route appended from Needs You and the same
            // route appended from the workspace list resolve to the same view
            // at different depths. That is precisely what the
            // `navigationDestination(item:)` this replaced could not do — see
            // `path`.
            //
            // Attached outside the phase switch rather than inside
            // `connected`, so the destinations exist in every phase. A Live
            // Activity card tapped while this screen is still connecting sets
            // `pendingTerminal`, and `openRequested` runs again the moment a
            // fleet arrives; a destination declared inside the connected branch
            // would not be there to receive it.
            //
            // Changing pane while one is open does NOT come through here — the
            // tab strip retargets `WorkspaceView` in place, and a deep link or
            // the switcher sheet naming a pane in the SAME workspace goes to
            // `requested` for the same reason. Appending a second `.workspace`
            // would mount a second `WorkspaceView` on top of the first and leave
            // every pane underneath it holding a stream nobody can see. A pane
            // in a DIFFERENT workspace replaces the last route rather than
            // appending — see `show(_:)`.
            .navigationDestination(for: Route.self) { destination($0) }
            .sheet(isPresented: $editing) {
                HostEditorView(
                    existing: host,
                    onSave: { store.update($0) },
                    onRemove: { store.remove($0) })
            }
            .task { await connect(host) }
            // The app coming back is the moment a backoff timer cannot predict.
            //
            // Here rather than in `RootView`, because this is where the
            // connection is: the same reason the host switcher moved down out
            // of the connected screen. `.background` is passed on too, so a
            // phone in a pocket stops polling — which is both a battery
            // question and one plausible way the session died in the first
            // place.
            .onChange(of: scenePhase) { _, phase in
                connection.setActive(phase == .active)
            }
            // A workspace that leaves the fleet has nothing left to show, so it
            // comes off the path.
            //
            // This used to watch the fleet's TERMINAL count and pop when it hit
            // zero, because the screen underneath was a pane host and a pane
            // host with no panes is a dead end with the switcher sheet as its
            // only exit. `WorkspaceView` is not that: its Changes tab needs no
            // pane to exist, so a worktree whose last agent was stopped is still
            // a screen worth standing on — you are usually there to read what
            // the agent left behind.
            //
            // What IS a dead end is the worktree itself being removed, and that
            // is what this watches now. Deliberately not the focused pane
            // disappearing: `WorkspaceView` moves off the pane it opened with
            // whenever the tab strip is used, so "the terminal we arrived at is
            // gone" is routine and correct and must not yank anyone anywhere.
            //
            // Guarded on the fleet being non-empty, because a reconnect can
            // briefly answer with nothing and every workspace would look
            // removed — the same guard, for the same reason, as
            // `WorkspaceView.prune`.
            //
            // Truncated at the FIRST workspace rather than emptied outright, so
            // someone who reached it through the workspace list lands back on
            // that list — which still has its toolbar, and is the one screen
            // that can start the work that would refill the fleet.
            .onChange(of: workspaceIDs) { _, ids in
                guard !ids.isEmpty else { return }
                let live = Set(ids)
                guard
                    let gone = path.firstIndex(where: { route in
                        guard let id = route.workspaceID else { return false }
                        return !live.contains(id)
                    })
                else { return }
                path.removeSubrange(gone...)
            }
            // The saved path's one chance, taken the moment there is a fleet to
            // check it against. See `restorePath`.
            .onChange(of: hasFleet) { _, arrived in
                if arrived { restorePath() }
            }
            // And the other direction: every push and every pop is written
            // down. Cheap — a couple of hundred bytes of JSON on a navigation,
            // not on a poll — because `path` only changes when someone
            // navigates.
            .onChange(of: path) { _, routes in savePath(routes) }
            // A tapped Live Activity card, arriving as `…://terminal/<id>`.
            //
            // Here rather than on the root view, because this is the screen
            // that owns `path` and the connection whose fleet the id has to be
            // looked up in. Routing it from the root would mean a second way to
            // choose a terminal, threaded down through views that know nothing
            // about one.
            //
            // The scheme is deliberately not checked: iOS only delivers URLs
            // whose scheme this app registered, and each channel registers only
            // its own, so a canary build cannot be handed a stable link in the
            // first place.
            .onOpenURL { url in
                guard url.host() == "terminal" else { return }
                let terminal = url.lastPathComponent
                guard !terminal.isEmpty else { return }
                pendingTerminal = terminal
                openRequested()
            }
        }
    }

    /// The stack's root, one branch per connection phase.
    ///
    /// Split out of `body` rather than written inline, for the compiler's sake:
    /// a `switch` over an associated-value enum inside a long modifier chain is
    /// the shape Swift's type checker gives up on, and it did.
    @ViewBuilder
    private var phases: some View {
        switch connection.phase {
        case .connecting:
            escapable { connecting }

        case .needsApproval(let fingerprint):
            escapable { approval(fingerprint) }

        case .failed(let message):
            escapable { failure(message) }

        // Reconnecting renders exactly what connected renders. The fleet on
        // screen is the last one this runner sent, and it is a better answer
        // than a spinner while the link comes back — see
        // `Connection.Phase.reconnecting`. The status chip in the bar is where
        // the difference shows.
        case .connected, .reconnecting:
            connected
        }
    }

    /// What each route draws.
    ///
    /// The workspace goes through a small wrapper rather than being built
    /// straight out of the fleet here, and that indirection is load-bearing —
    /// see `WorkspaceRoute`.
    @ViewBuilder
    private func destination(_ route: Route) -> some View {
        switch route {
        case .workspaces:
            // Titled for what it lists, and for the word the owner uses. It was
            // "Worktrees" under a row labeled "Working", which was two names
            // for one screen and neither of them what the screen is. See
            // `NeedsYouView.workspacesRow`.
            //
            // `onSelect` appends, so a workspace opened from here sits ON this
            // list at depth 2 and Back returns to it. That is the whole of the
            // Back bug: the same callback used to assign `landing`, and
            // assigning `landing` replaced this screen with the pane.
            //
            // Focused on the terminal that was tapped, and that is the only
            // thing this door and the inbox's door disagree about. Here you
            // pointed at an agent; there you pointed at a workspace and the
            // rule decides. See `Route.Focus`.
            WorkspaceListView(
                connection: connection,
                onSelect: { show($0) },
                hosts: store
            )
            .navigationTitle("Workspaces")
            .navigationBarTitleDisplayMode(.inline)

        case .workspace(let id, let focus):
            WorkspaceRoute(
                workspace: id, focus: focus, connection: connection, hosts: store,
                requested: $requested, onOpen: show)
        }
    }

    /// Show a terminal, wherever it is.
    ///
    /// The one place that decides between the three things a request to open a
    /// pane can mean, so a deep link, the workspace list and the switcher sheet
    /// inside a workspace cannot answer it differently.
    ///
    /// - Already in the workspace on screen: hand it to `requested`, which
    ///   `WorkspaceView` honors by switching tabs. Nothing is pushed and nothing
    ///   is rebuilt, so every mounted pane keeps its scroll position, its
    ///   half-typed message and its open ssh stream.
    /// - A DIFFERENT workspace while one is open: replace the route rather than
    ///   append. Appending would stack a second `WorkspaceView` on the first and
    ///   leave every pane underneath it holding a stream nobody can see;
    ///   replacing swaps the host at the same depth, so Back still means what it
    ///   meant a moment ago.
    /// - Nothing open: append, which is the ordinary push.
    private func show(_ terminal: Terminal) {
        guard
            let workspace = connection.fleet.workspaces.first(where: { candidate in
                candidate.terminals.contains { $0.id == terminal.id }
            })
        else { return }

        if path.last?.workspaceID == workspace.id {
            requested = terminal
        } else if path.last?.workspaceID != nil {
            path[path.count - 1] = .workspace(id: workspace.id, focus: .agent(terminal.id))
        } else {
            path.append(.workspace(id: workspace.id, focus: .agent(terminal.id)))
        }
    }

    /// Open the terminal a card asked for, if this runner's fleet has it yet.
    ///
    /// Runs when the URL arrives and again when a connection has produced a
    /// fleet, because at a cold launch those are two different moments and the
    /// first one has nothing to search.
    private func openRequested() {
        guard let id = pendingTerminal else { return }
        let all = connection.fleet.workspaces.flatMap(\.terminals)
        if let terminal = all.first(where: { $0.id == id }) {
            pendingTerminal = nil
            // Push, replace or retarget — `show(_:)` owns that decision, and
            // owning it in one place is why a card behaves the same whether it
            // was tapped at the inbox, inside the workspace it names, or inside
            // a different one.
            //
            // The path goes on naming the pane the workspace was OPENED with
            // and is not rewritten as the tab strip moves. A path element that
            // changes value is a destination SwiftUI is free to rebuild, and
            // rebuilding this one discards every mounted pane — which is the
            // exact loss `WorkspaceView` exists to prevent. The cost used to be
            // that a restored path reopened the pane you arrived at; it is
            // smaller now, because a workspace restored from the inbox carries
            // `.none` and re-runs the focus rule against the fleet as it is
            // when you come back. See `Route.Focus`.
            show(terminal)
        } else if connection.phase == .connected {
            // The runner answered and does not have it: the pane is gone, or
            // the card was about another runner entirely — the URL carries an
            // id and no host, so there is nothing here to switch to. Dropped
            // rather than kept waiting, or a pane created much later would be
            // jumped to long after anyone tapped anything.
            pendingTerminal = nil
        }
    }

    /// Put the phone back where it was, once there is a fleet to check that
    /// against.
    ///
    /// Deliberately not applied at launch, when `@SceneStorage` hands the value
    /// back. At that moment there is no fleet, so a `.terminal` route names
    /// something this app cannot resolve, and installing it would push a screen
    /// with nothing in it over the connecting, approval and failure screens —
    /// the three that most need to be visible. Held instead until the runner
    /// has answered, which is the first moment the question "does this still
    /// exist" has an answer.
    ///
    /// Truncated at the first route that no longer resolves rather than
    /// filtered, because a path is a sequence of pushes: keeping depth 2 after
    /// dropping depth 1 would put a screen on top of a screen nobody navigated
    /// through. A path that resolves to nothing simply leaves you at the root,
    /// which is where a cold launch has always landed.
    ///
    /// Once, and only once. After this the path is whatever the person holding
    /// the phone has done with it, and a second pass on a later reconnect would
    /// be the app steering them somewhere they had already left.
    private func restorePath() {
        guard !restoredPath else { return }
        restoredPath = true
        guard path.isEmpty, let data = savedPath.data(using: .utf8),
            let saved = try? JSONDecoder().decode(SavedPath.self, from: data),
            saved.runner == host.id.uuidString
        else { return }

        let usable = Array(saved.routes.prefix(while: resolves))
        guard !usable.isEmpty else { return }
        // No push animation for a screen you never left. Restoring where you
        // were should look like the app remembering, not like it navigating.
        var silent = Transaction()
        silent.disablesAnimations = true
        withTransaction(silent) { path = usable }
    }

    /// Whether this runner still has the thing a route names.
    ///
    /// The workspace, and deliberately not its focus. A worktree that is still
    /// here is a screen worth restoring even if the agent you were reading has
    /// since been stopped — the diff is still there, the other agents are still
    /// there, and `WorkspaceRoute` falls back to the focus rule for exactly this
    /// case. Refusing the whole route over a missing pane would drop you at the
    /// inbox instead, which is further from where you were.
    private func resolves(_ route: Route) -> Bool {
        switch route {
        case .workspaces:
            // Always. The screen lists whatever there is, including nothing.
            return true
        case .workspace(let id, _):
            return connection.fleet.workspaces.contains { $0.id == id }
        }
    }

    private func savePath(_ routes: [Route]) {
        let saved = SavedPath(runner: host.id.uuidString, routes: routes)
        guard let data = try? JSONEncoder().encode(saved),
            let json = String(data: data, encoding: .utf8)
        else { return }
        savedPath = json
    }

    /// The path and the runner it was walked on, which is the whole of what
    /// `savedPath` holds. See it for why the runner has to travel with it.
    private struct SavedPath: Codable {
        var runner: String
        var routes: [Route]
    }

    /// Every screen shown BEFORE a connection exists, wrapped in the ways out of
    /// it.
    ///
    /// This is the bug those screens all had. `FleetView` is the root of the
    /// app's only navigation stack — the app opens onto a runner rather than a
    /// list of them — so it has no back button, and the host switcher lives
    /// inside `WorkspaceListView`, which only exists once a connection has
    /// succeeded. Any phase short of `.connected` was therefore a room with no
    /// doors: "Could not connect" offered "Try again" and nothing else, and if
    /// trying again could not work — the wrong address, a runner that never
    /// authorized this phone — there was no way to reach another runner, add
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

    /// The root of the stack, in every phase that has a fleet behind it.
    ///
    /// Always the inbox, never a terminal. Everything else this stack shows is
    /// appended to `path` on top of it — see the `navigationDestination` in
    /// `body` — which is what makes Back the screen you came from.
    ///
    /// Nothing is handed down to open a terminal any more. The rows push a
    /// `Route` value themselves, which is an append; the callback that used to
    /// come back up here existed only so that one assignment to `landing` could
    /// be the app's single terminal destination.
    @ViewBuilder
    private var connected: some View {
        if hasFleet {
            NeedsYouView(connection: connection, runner: host, store: store)
        } else {
            // Not the inbox with nothing in it. Until the runner has answered,
            // "Nothing needs you" would be a claim made from `Fleet.empty` —
            // see `hasFleet`. The runner's own name is the title here because
            // there is nothing else yet to say what is being waited on.
            ProgressView()
                .navigationTitle(host.label)
                .navigationBarTitleDisplayMode(.inline)
        }
    }

    /// Every workspace this runner still has. Watched rather than the fleet
    /// itself, because this screen only cares about one of it going away.
    /// Sorted because `onChange` wants `Equatable` and a stable order.
    private var workspaceIDs: [String] {
        connection.fleet.workspaces.map(\.id).sorted()
    }

    /// Connect, then let the inbox draw whatever fleet that connection
    /// produced.
    ///
    /// Shared by the initial `.task` and every retry below — the approval
    /// screen's "Trust This Runner" and the failure screen's "Try Again" each
    /// start a fresh connection of their own, and each one has to end with the
    /// deep link getting its second look.
    ///
    /// What is gone from here is `landing = connection.fleet.landingTerminal`.
    /// That line chose an agent for you at every connect, and it chose one on
    /// every reconnection ceremony too — so a phone that lost its tunnel in
    /// transit could come back on a different pane than the one you were
    /// reading.
    private func connect(_ target: Runner) async {
        await connection.start(host: target)
        // A card tapped at cold launch delivers its URL before there is a fleet
        // to look the id up in, so `openRequested` found nothing and gave up.
        // This is its second chance, now that there is somewhere to look.
        openRequested()
    }

    // MARK: - Phases

    /// First contact. The fingerprint is shown and refused until a human says
    /// yes, because silently trusting an unknown key is what makes an
    /// interception invisible.
    private func approval(_ fingerprint: String) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            Label("Unrecognized Runner", systemImage: "questionmark.circle")
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
                "Check it on the host: ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub"
            )
            .font(.footnote)
            .foregroundStyle(.secondary)

            Button("Trust This Runner") {
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
                    Button("Edit This Runner…") { editing = true }
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
            //
            // The one link in this stack that is a view rather than a `Route`,
            // and the exception is deliberate. It is a leaf with nothing under
            // it, reachable only from the failure screen — a phase with no
            // fleet, so nothing can be appended to the path underneath it:
            // `openRequested` appends only for a terminal the runner has
            // answered with, and the only calls to `connect` that could produce
            // one are the buttons on the screen this covers. Nothing here is
            // worth persisting either, which is the other half of what a route
            // is for.
            NavigationLink {
                AuthorizeView(runners: store)
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
    /// and putting that in front of someone who just wants their runner back
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
                "Nothing answered on port \(host.port). The runner may be asleep, "
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
        case .hostKeyChanged: return "This Host’s Key Changed"
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

/// One workspace, opened on one tab, resolved exactly once.
///
/// The wrapper exists for a single reason, and removing it puts back the worst
/// bug this screen can have. `WorkspaceView` needs a starting `Pane`, the path
/// holds an id and a focus, and the obvious thing — resolving that focus every
/// time this destination is evaluated — makes the host's very existence
/// conditional on the lookup going on succeeding. It routinely stops
/// succeeding: the pane you arrived at can be killed, dismissed or removed
/// while you are reading a different tab in the same workspace, which is an
/// ordinary state `WorkspaceView` already handles by pruning it out of
/// `visited`. If the lookup drove the view structure, that same moment would
/// swap the whole host for a placeholder and throw away every mounted pane —
/// the exact loss `WorkspaceView` exists to prevent, and the same argument
/// `FleetView`'s removed-workspace rule makes from the other side.
///
/// So the starting pane is latched the first time it resolves and never
/// unlatched. After that this view's structure is fixed and the host is left
/// alone; which tab is on screen inside it is `WorkspaceView`'s business, not
/// the path's. `FleetView` still pops the route when the WORKSPACE leaves the
/// fleet, which is the one case where there is genuinely nothing left to show.
///
/// A focus naming an agent that has gone falls back to the rule rather than to
/// a placeholder, which is what makes a path restored hours later land
/// somewhere useful. See `Route.Focus`.
@MainActor
private struct WorkspaceRoute: View {
    let workspace: String
    let focus: Route.Focus
    @ObservedObject var connection: Connection
    let hosts: RunnerStore
    @Binding var requested: Terminal?
    /// How to open a pane this workspace does not contain — the switcher sheet
    /// can name one anywhere on the runner. See `FleetView.show(_:)`.
    let onOpen: (Terminal) -> Void

    @State private var opened: Pane?

    private var live: Workspace? {
        connection.fleet.workspaces.first { $0.id == workspace }
    }

    var body: some View {
        Group {
            if let opened {
                WorkspaceView(
                    workspace: workspace, initial: opened, connection: connection,
                    hosts: hosts, requested: $requested, onOpen: onOpen)
            } else {
                ProgressView()
            }
        }
        .onAppear { resolve() }
        .onChange(of: liveIDs) { _, _ in resolve() }
    }

    /// This workspace and its panes, as something `onChange` can compare.
    ///
    /// Scoped to the workspace rather than the fleet, and the workspace's own id
    /// is in it deliberately: a worktree with no terminals at all still has a
    /// diff to open, and a key made only of terminal ids would never change when
    /// such a workspace arrived — leaving this on the spinner for good. Sorted
    /// because `onChange` wants a stable order.
    private var liveIDs: [String] {
        guard let live else { return [] }
        return [live.id] + live.terminals.map(\.id).sorted()
    }

    private func resolve() {
        guard opened == nil, let workspace = live else { return }
        // The focus the route carries, and the rule for everything else —
        // `.none` because the inbox had no opinion, and a named agent that has
        // since left the fleet. Both end up in the same place on purpose.
        var wanted = focus
        if case .agent(let id) = focus,
            !workspace.terminals.contains(where: { $0.id == id })
        {
            wanted = .none
        }
        if case .none = wanted {
            wanted = Route.Focus.rule(for: workspace, inbox: connection.inbox[workspace.id])
        }

        switch wanted {
        case .changes, .none:
            opened = .changes
        case .agent(let id):
            guard let terminal = workspace.terminals.first(where: { $0.id == id }) else {
                opened = .changes
                return
            }
            opened = Pane(terminal)
        }
    }
}

/// The workspace list plus what it takes to act on it: quick task, new
/// workspace, pull-to-refresh. Shown two places — pushed from the inbox's
/// Workspaces row, and inside the sheet `WorkspaceView` opens to switch
/// terminals —
/// so a task started from either one works the same way and neither loses a
/// capability the other has.
///
/// It was `FleetView`'s fallback for a runner with no terminal to land on,
/// which is why the two toolbar buttons live here: they were on the only screen
/// a runner with nothing running could show. That makes this the one place to
/// start work, and it is now a tap below the front door rather than at it —
/// which is why the row that leads here counts WORKSPACES and not working
/// agents. See `NeedsYouView.workspacesRow`.
struct WorkspaceListView: View {
    @ObservedObject var connection: Connection
    let onSelect: (Terminal) -> Void
    /// Non-nil only in the sheet: what "Done" calls. `FleetView`'s own use
    /// leaves this nil because a pushed screen already has a back button.
    var onDismiss: (() -> Void)?
    /// The runners to switch between, when this is the sheet.
    ///
    /// Switching hosts lives here because this is already where you go to
    /// switch what you are looking at. The app opens onto terminals now (see
    /// `RootView`), so there is no host list to go back to — and inventing a
    /// second switcher screen for the rarer of the two switches would be one
    /// more place to look.
    var hosts: RunnerStore?

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
            NewWorkspaceView(repositories: connection.repositories, connection: connection) { repository, name, branch, adopt in
                await connection.createWorkspace(
                    repository: repository, name: name, branch: branch, adopt: adopt)
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

/// Which runner you are looking at, and every way of changing that.
///
/// A strip along the bottom rather than a section in a list: the list above it
/// is worktrees on ONE runner, and putting the runner inside it would read as
/// one more thing in the same collection. This says what the collection belongs
/// to.
///
/// Split out of `WorkspaceListView` because it turned out to be the app's only
/// escape hatch, and it was attached to the one screen you cannot reach when you
/// need an escape hatch — the connected one. `FleetView` now puts it under the
/// connecting, approval and failure screens too, which is what makes those
/// screens leaveable at all.
struct HostSwitcherBar: View {
    @ObservedObject var hosts: RunnerStore
    /// The connection whose state the chip shows, and which its tap retries.
    /// Also how the settings screen names the daemon it is talking to. Absent
    /// before a connection exists, which is most of the time this bar matters.
    @ObservedObject var connection: Connection
    /// Called after picking a different runner, for the caller that is a sheet
    /// and needs to close itself. Nil where the bar is part of the screen.
    var onSwitch: (() -> Void)?

    @State private var showAdd = false
    /// The host being edited, rather than a bare flag: a flag plus a separate
    /// `hosts.selected` lookup can present a sheet with nothing in it if the
    /// selection changes between the tap and the presentation.
    @State private var editingRunner: Runner?
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
                // One entry, not one per kind of adding. This said "Add a
                // Runner…" and went straight to the address form, which is the
                // long road — the ceremony that would have picked up a runner's
                // address, user, port and host key without anybody typing was
                // reachable only from a screen this device stopped showing the
                // moment it had its first runner.
                Button("Add…") { showAdd = true }
                if let selected = hosts.selected {
                    // Editing and removing were unreachable from anywhere in the
                    // app: `RunnerStore.remove` existed and had no caller, so a
                    // runner typed in wrong was permanent, and permanent plus
                    // unreachable meant the app opened onto a screen it could
                    // never get past.
                    Button("Edit This Runner…") { editingRunner = selected }
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
                    Text(hosts.selected?.label ?? "No Runner")
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
        .sheet(isPresented: $showAdd) {
            AddView(runners: hosts)
        }
        .sheet(item: $editingRunner) { host in
            HostEditorView(
                existing: host,
                onSave: { hosts.update($0) },
                onRemove: { hosts.remove($0) })
        }
        .sheet(isPresented: $showSettings) {
            NavigationStack {
                // No "Authorize" in the toolbar any more. It was the fifth way
                // into add-shaped territory and the least explicable — a verb
                // with no object, in a bar, next to a title — and what it
                // actually offered was this device's public key. That is a row
                // in the Devices section now, beside the rest of it.
                SettingsView(connection: connection, runners: hosts)
            }
        }
    }
}

/// Whether this runner is answering, and a way to ask it again.
///
/// The Mac's sidebar dot, on a phone. It sits in the runner bar because that
/// strip is already what says which runner you are looking at, and because it
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
        .accessibilityHint("Reconnects to this runner")
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
    /// Which workspace's stack is being looked at. One value rather than a flag
    /// plus two strings: a sheet that can be presented with nothing to show is
    /// a sheet that will eventually be presented with nothing to show.
    @State private var stackTarget: StackTarget?

    /// A repository and a branch — everything `stack.get` needs.
    struct StackTarget: Identifiable {
        let repository: String
        let branch: String
        var id: String { "\(repository)#\(branch)" }
    }

    private var shown: [Workspace] { fleet.workspaces.filter { !$0.isHidden } }
    private var hidden: [Workspace] { fleet.workspaces.filter(\.isHidden) }
    private var hiddenAttention: Int {
        hidden.flatMap(\.terminals).filter(\.agent.wantsAttention).count
    }

    var body: some View {
        List {
            if fleet.workspaces.isEmpty {
                Text("No workspaces on this runner.")
                    .foregroundStyle(.secondary)
            }

            ForEach(shown) { workspace in
                let numbering = workspace.ordinals()
                Section {
                    // The host's order: blocked first, then done, then working,
                    // oldest first inside each — `farcooler_core::feed::rank`,
                    // computed once so this list and a widget showing one agent
                    // cannot disagree about which one matters.
                    //
                    // It replaced creation order, and the argument creation
                    // order won on is still true: a row that moves under a
                    // finger already travelling toward it is a tap that lands on
                    // something else. What changed is that the same order now
                    // has to hold on surfaces with room for ONE agent, where
                    // "find the marked row" is not an option — and two orders,
                    // one per surface, is the disagreement the ladder exists to
                    // prevent. The rank only moves when the agent's own state
                    // does, so the list still holds still while you read it.
                    ForEach(workspace.terminals.sorted { $0.sortRank < $1.sortRank }) { terminal in
                        Button { onSelect(terminal) } label: {
                            TerminalRow(terminal: terminal, ordinal: numbering[terminal.id])
                                // A list row's label otherwise sizes to its
                                // text. Give selection the whole visible row,
                                // including the blank trailing space a thumb
                                // naturally lands in.
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("fleet-terminal-\(terminal.id)")
                        .swipeActions(edge: .trailing) { terminalActions(for: terminal) }
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
                    HStack(spacing: 6) {
                        Text(workspace.task)
                        // Something under here wants you, said once at the top
                        // rather than left to be inferred from a row further
                        // down that may be scrolled off.
                        if workspace.terminals.contains(where: { $0.agent.wantsAttention }) {
                            Circle()
                                .fill(Color.orange)
                                .frame(width: 6, height: 6)
                                .accessibilityLabel("Needs you")
                        }
                        // The worktree this workspace names is not on disk any
                        // more. Said on the header because every row under it
                        // is about a pane in a directory that is gone, and
                        // "Removed" beats twenty terminals failing separately.
                        if workspace.worktreeMissing {
                            Text("worktree gone")
                                .font(.caption2)
                                .foregroundStyle(.orange)
                        }
                        Spacer()
                        // What this worktree has changed, which is the fact the
                        // Mac's sidebar puts here and the one that says whether
                        // a workspace is worth opening. Monospaced so the
                        // columns line up down the list, and absent entirely
                        // when there is nothing — a row of `+0 -0` on every
                        // clean worktree is noise in the shape of information.
                        if let counts = connection.inbox[workspace.id], counts.hasDiff {
                            HStack(spacing: 4) {
                                Text("+\(counts.insertions)").foregroundStyle(.green)
                                Text("-\(counts.deletions)").foregroundStyle(.red)
                            }
                            .font(.caption2.monospaced())
                        }
                        // The main checkout has a branch like everything else,
                        // but WHICH branch is not the useful fact about it —
                        // that it is the repository itself, and so cannot be
                        // removed, is. The Mac says the same thing here.
                        Text(workspace.isMainCheckout ? "Primary checkout" : workspace.branch)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Menu {
                            Button {
                                Task { await connection.createTerminal(workspace: workspace) }
                            } label: {
                                Label("New terminal", systemImage: "plus")
                            }
                            // Where this branch sits, and what GitHub says.
                            // Only offered when the daemon told us which
                            // repository the worktree belongs to — an older one
                            // did not, and a menu item that cannot work is
                            // worse than one that is not there.
                            if let repository = workspace.repository, !workspace.branch.isEmpty {
                                Button {
                                    stackTarget = StackTarget(
                                        repository: repository, branch: workspace.branch)
                                } label: {
                                    Label("Stack & Pull Request", systemImage: "square.stack.3d.up")
                                }
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
                            // The same host order as the shown workspaces
                            // above. A hidden workspace sorted differently would
                            // be a second answer to "which agent matters most"
                            // living one disclosure triangle away.
                            ForEach(workspace.terminals.sorted { $0.sortRank < $1.sortRank }) { terminal in
                                Button { onSelect(terminal) } label: {
                                    TerminalRow(
                                        terminal: terminal, ordinal: numbering[terminal.id])
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                        .contentShape(Rectangle())
                                }
                                .buttonStyle(.plain)
                                .accessibilityIdentifier("fleet-terminal-\(terminal.id)")
                                .swipeActions(edge: .trailing) { terminalActions(for: terminal) }
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
        .sheet(item: $stackTarget) { target in
            StackView(
                repository: target.repository,
                branch: target.branch,
                connection: connection)
        }
    }

    @ViewBuilder
    private func terminalActions(for terminal: Terminal) -> some View {
        let kind = StateKind.parse(terminal.state)
        if kind == .lost {
            Button("Dismiss") { onAction(.dismissLost, terminal) }.tint(.gray)
        }
        // Nothing destructive for a terminal whose runner did not answer.
        // Restart kills the pane and starts a new epoch: the right answer for a
        // process that is gone, and the wrong one for a process that is fine
        // behind a tmux server that was busy for a moment — which is exactly
        // what `unknown` means. The daemon already refuses `dismiss` in this
        // state; this keeps the phone from offering the more damaging of the
        // two at all.
        if kind != .unknown {
            Button("Restart") { onAction(.restart, terminal) }.tint(.blue)
        }
        if kind == .running || kind == .starting {
            Button("Stop", role: .destructive) { onAction(.stop, terminal) }
        }
    }
}

struct TerminalRow: View {
    let terminal: Terminal
    /// Which of several identically-labeled siblings this is, from
    /// `Workspace.ordinals()`, or nil when its label is unique in the
    /// workspace and numbering it would answer a question nobody asked.
    var ordinal: Int?

    private var kind: StateKind { StateKind.parse(terminal.state) }

    var body: some View {
        // Once a second, and only for the rows that have a clock to run. A
        // `TimelineView` is how the elapsed string ticks at all — see
        // `Terminal.displayDuration(at:)` for why the time has to arrive as an
        // argument rather than be read inside the row.
        //
        // Wrapped around the whole row rather than around the one `Text`,
        // because the schedule is what decides how often SwiftUI re-evaluates
        // this subtree — and a row with no clock must not be re-evaluated
        // every second to display nothing new. Both branches are `.periodic`
        // because a ternary has to produce one type, and an hour is "never" at
        // the scale of a list somebody is looking at; a row whose agent starts
        // working gets its new schedule from the state change itself rather
        // than by waiting out the hour.
        TimelineView(.periodic(from: .now, by: hasClock ? 1 : 3600)) { tick in
            content(at: tick.date)
        }
    }

    /// Whether anything in this row changes with the passing of time.
    private var hasClock: Bool {
        terminal.agent == .working || terminal.agent == .blocked
    }

    private func content(at now: Date) -> some View {
        HStack(alignment: .top, spacing: 10) {
            Circle()
                .fill(processColor(kind))
                .frame(width: 8, height: 8)
                // The dot sits on the first line's baseline rather than in the
                // middle of a row that is now up to four lines tall.
                .padding(.top, 6)

            VStack(alignment: .leading, spacing: 3) {
                // Band 1: what it is, and how long it has been that.
                HStack(spacing: 4) {
                    Text(terminal.label).font(.body)
                    if let ordinal {
                        Text("\(ordinal)")
                            .font(.caption2.monospaced())
                            .foregroundStyle(.tertiary)
                    }
                    if let status = statusText(at: now) {
                        Text(status)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    Spacer(minLength: 0)
                    // The reason to have opened the app. Only the two states
                    // worth acting on get color, so a list of twenty still
                    // reads at a glance.
                    if terminal.agent.isAgent && terminal.agent != .unknown {
                        Label(terminal.activityLabel, systemImage: terminal.activitySymbol)
                            .labelStyle(.iconOnly)
                            .font(.system(size: 13, weight: terminal.agent.wantsAttention ? .semibold : .regular))
                            .foregroundStyle(attentionColor(terminal))
                            .accessibilityLabel(terminal.activityLabel)
                    }
                }

                // Band 2: where the agent IS — the question it is blocked on,
                // its position in its own task list, or what it is doing. One
                // line, composed on the host so a Mac, a phone and a watch
                // cannot disagree about which of those three to show.
                //
                // This is what replaced `terminal.state.lowercased()`, which
                // spent the most valuable line of the row restating the dot
                // immediately to its left.
                if !terminal.signalLine.isEmpty {
                    Text(terminal.signalLine)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }

                // Band 3: the agent's own last words, oldest first. Already
                // redacted and cut to a row's width by the daemon, so this
                // renders them and decides nothing about them.
                ForEach(Array(terminal.recentSteps.enumerated()), id: \.offset) { _, step in
                    Text(step)
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }

                // Band 4: what it spawned and has not finished with.
                ForEach(terminal.runningSubagents, id: \.self) { name in
                    Text("\u{2442} \(name)")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
        }
        .contentShape(Rectangle())
    }

    /// "Working 12m", "Needs you 2m", or just the state for a pane that is not
    /// an agent.
    ///
    /// A duration only where one exists: `displayDuration` returns nil under
    /// five seconds, so a row does not flicker "1s" on its way to saying
    /// something useful.
    private func statusText(at now: Date) -> String? {
        guard terminal.agent.isAgent, terminal.agent != .unknown else {
            // A plain shell still says whether it is alive, which is the whole
            // of what is known about it. Exited is worth saying; running is
            // what the green dot already said.
            return kind == .running ? nil : terminal.state.lowercased()
        }
        guard let elapsed = terminal.displayDuration(at: now) else { return terminal.activityLabel }
        return "\(terminal.activityLabel) \(elapsed)"
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

/// The same color, for a terminal whose finished turn may have DIED.
///
/// Green and red are the whole difference between "it's done" and "it stopped
/// working", and `AgentActivity` alone cannot tell them apart — the daemon
/// sends both as `done` and says which in `turnFailed`. Every surface that has
/// a terminal in hand asks this one instead, so the fleet list and the tab
/// strip cannot disagree about the same pane.
func attentionColor(_ terminal: Terminal) -> Color {
    terminal.turnDidFail ? .red : attentionColor(terminal.agent)
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
                    Text("This worktree has uncommitted changes. Enter its name to remove it.")
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
                    Text("Choose an existing repository on this runner for the new worktree.")
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
    let onCreate: (String, String, String, Bool) async -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var repository: String = ""
    @State private var name = ""
    @State private var branch = ""
    @State private var working = false
    @State private var showAddRepository = false
    @State private var showBranchPicker = false
    /// Set when a branch was picked from the list rather than typed.
    ///
    /// Adoption is a different operation, not a flag on the same one: the
    /// daemon takes the branch over and names the worktree after it, so the
    /// name field stops mattering and the form says so rather than collecting
    /// something it will throw away.
    @State private var adopting: Branch?

    private var trimmedName: String { name.trimmingCharacters(in: .whitespaces) }

    /// The folder this name lands in.
    ///
    /// Shown under the field because the name IS the worktree's directory now:
    /// nothing encourages naming a worktree carefully while the thing being
    /// named is invisible.
    private var folder: String { TaskSlug.sanitize(trimmedName) }

    /// Sixty is the runner's cap on a name.
    private var isTooLong: Bool { trimmedName.unicodeScalars.count > 60 }

    /// The branch this form suggests, from the name and the runner's prefix.
    ///
    /// This form had no suggestion at all and made you type a branch by hand,
    /// which meant the runner's branch prefix — the whole point of the setting
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

    /// Both name rules are checked here, not just left to the runner, because
    /// `createWorkspace` swallows its error: a refused name would close this
    /// sheet on a worktree that was never created and say nothing about why.
    private var isValid: Bool {
        // Adoption has nothing to validate but the repository: the branch was
        // picked from a list the runner produced, and the name comes from it.
        if adopting != nil { return !repository.isEmpty }
        return !repository.isEmpty && !folder.isEmpty && !isTooLong && !effectiveBranch.isEmpty
    }

    var body: some View {
        NavigationStack {
            Form {
                Picker("Repository", selection: $repository) {
                    Text("Choose").tag("")
                    ForEach(repositories) { Text($0.displayName).tag($0.id) }
                }
                Button("Add a repository…") { showAddRepository = true }

                if let adopting {
                    // Adoption collapses the form: there is nothing to name and
                    // nothing to branch from.
                    Section {
                        LabeledContent("Resuming") {
                            Text(adopting.name)
                                .font(.footnote.monospaced())
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                        Button("Start a new branch instead") { self.adopting = nil }
                    } footer: {
                        Text(
                            "Far Cooler takes this branch over in a new worktree named after "
                            + "it. Nothing on the branch changes.")
                    }
                } else {
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

                    // The other way work arrives.
                    //
                    // Before this the only option was a new branch, so picking
                    // up something pushed from another machine — or produced by
                    // a cloud agent — meant typing its name exactly and hoping.
                    if !repository.isEmpty {
                        Button {
                            showBranchPicker = true
                        } label: {
                            Label("Resume an existing branch…", systemImage: "arrow.uturn.down")
                        }
                    }

                    Section {
                        Text(
                            "A workspace contains one Git worktree and branch. Its name is also "
                            + "the folder name and can’t be changed later.")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
            }
            .navigationTitle("New workspace")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(adopting == nil ? "Create" : "Resume") {
                        working = true
                        Task {
                            if let adopting {
                                await onCreate(repository, adopting.name, adopting.name, true)
                            } else {
                                await onCreate(repository, trimmedName, effectiveBranch, false)
                            }
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
            .sheet(isPresented: $showBranchPicker) {
                BranchPicker(repository: repository, connection: connection) { branch in
                    adopting = branch
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
