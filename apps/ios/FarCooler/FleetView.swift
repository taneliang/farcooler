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
    /// Part of the route rather than state inside the screen because it is a
    /// property of the tap: every door into a workspace names the pane it meant,
    /// and they differ only in what they are able to name. The workspace list
    /// names the agent whose row was tapped. The inbox names one too — its rows
    /// are one per blocked agent and one for the diff, so it says `.agent` or
    /// `.changes` and never shrugs. See `NeedsYouView.section(for:)`.
    ///
    /// `.none` is "nobody at this door had an opinion", and no door emits it any
    /// more. It survives because three things that are not doors still produce
    /// it: `WorkspaceRoute.alive(_:in:)` returns it when a focus names a pane
    /// that has left the worktree, which is how a stale answer falls through to
    /// the next one; `FleetView.deferringToMemory(_:)` writes it into a route
    /// coming back out of `@SceneStorage` that has a remembered tab behind it,
    /// which is how a door that is now history stops outranking the place you
    /// actually ended up; and a path persisted by a build that predates the
    /// inbox's rows decodes straight back into it. All three are resolved at the
    /// moment the screen is built by `WorkspaceRoute.resolve()`.
    ///
    /// Which resolves it in an order, and the order is an order of authority:
    ///
    /// 1. **The route says.** Somebody pointed at a pane on the way in — an
    ///    inbox row, a workspace list row, a tapped card, the switcher sheet.
    ///    That is the most recent thing anyone asked for and it wins outright.
    /// 2. **The tab you were last on.** `Connection.lastFocus`, written only
    ///    when a person taps a chip. Read here rather than stored in the route
    ///    itself, because a path element whose value changes is a destination
    ///    SwiftUI may rebuild — see that property, and `WorkspaceView`.
    /// 3. **The rule**, `rule(for:inbox:)`: whatever needs you now.
    ///
    /// Two and three used to be one, and the difference between them is the
    /// difference between a place you chose and a place the app picked for you.
    /// A workspace you last read the diff of should open on that diff when you
    /// come back to it, at the gym ninety seconds later or in the morning —
    /// `docs/jobs-to-be-done.md` F4 is the owner saying review has to be
    /// resumable, and landing somewhere other than where you were is exactly
    /// what it is not to be. A workspace you have never chosen a tab in has
    /// nothing to be resumed to, and opens on whatever needs you at seven
    /// rather than on the agent that needed you at midnight.
    ///
    /// One beats two only while the door is still the most recent thing that
    /// happened, and a path coming back out of `@SceneStorage` is not that: the
    /// door it names was walked through before the app was killed, and any chip
    /// tapped on the far side of it is newer. So a restored route hands its
    /// focus to the memory instead — `FleetView.deferringToMemory(_:)` is where
    /// the two swap places. It matters because every door names a pane now, the
    /// inbox included: without the swap, a workspace entered on an agent and
    /// then read as a diff would come back on the agent, which is the F4 case
    /// this whole order exists to answer. Only where there is a memory to swap
    /// with, though. A workspace nobody ever chose a tab in has none, and there
    /// the door is the only surviving record of where that person was sitting,
    /// so a restored route keeps it and one still beats three.
    ///
    /// Both of the last two degrade the same way: a remembered or ruled agent
    /// that has left the fleet falls through to the next answer rather than to
    /// a placeholder.
    enum Focus: Hashable, Codable {
        /// One agent, by terminal id.
        case agent(String)
        /// The worktree's own diff.
        case changes
        /// No opinion — see the order above.
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
    /// Which tab a workspace opens on when nobody said and nobody ever chose.
    ///
    /// The last of the three answers in `Route.Focus`, and the only one that
    /// reads the world as it is right now rather than as somebody left it.
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
    /// Two callers reach it now, and there used to be three. The inbox row was
    /// the third: it pointed at a workspace and left the pane to this. It points
    /// at a pane itself now, so what is left is a focus — the route's or the
    /// remembered one's — naming a pane that has since gone, and a restored path
    /// with neither answer to give: no remembered tab, and a focus that a build
    /// older than the inbox's rows wrote down as `.none`. Still one function,
    /// because two copies of it would be two answers to "where does this
    /// workspace open".
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

    /// The tab each workspace was last left on, as JSON, so being interrupted
    /// does not cost you your place INSIDE the screen either.
    ///
    /// A second key rather than a field on `SavedPath`, and beside the path for
    /// the same reason `Connection.lastFocus` is beside it in memory: this
    /// changes when somebody taps a chip and the path changes when somebody
    /// navigates, which are different moments, and a path saved by a build
    /// without this still decodes.
    ///
    /// **Persisted deliberately, and it was a real question.** A memory of
    /// where you were is stalest exactly when the app was killed longest ago,
    /// and the argument for letting it die with the process is that a workspace
    /// you last touched at midnight should open on whatever needs you at seven.
    /// That argument is already made and already answered one level up: `path`
    /// itself is persisted, so seven o'clock puts you back in the midnight
    /// WORKSPACE regardless. Restoring the screen and then re-deriving the tab
    /// inside it is not caution, it is the app half-remembering — and F4's case
    /// is a phone killed in a pocket between sets at the gym, where the whole
    /// point is to come back to the diff you were reading. If the staleness
    /// worry is right it is right about the path, and that is a different
    /// change from this one.
    ///
    /// What that costs is bounded by `restoreFocus`, which admits an entry only
    /// if the fleet that just answered still has the workspace AND the pane it
    /// names — so nothing here can resurrect a tab for something that is gone.
    ///
    /// Runner-scoped like the path, and for the same reason: ids are
    /// per-runner, so a memory written on one names nothing on another.
    @SceneStorage("fleet.focus") private var savedFocus = ""

    /// Whether the saved place has had its one chance to come back. See
    /// `restorePlace` for why it gets exactly one.
    @State private var restoredPlace = false

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
            // The saved place's one chance, taken the moment there is a fleet
            // to check it against. See `restorePlace`.
            .onChange(of: hasFleet) { _, arrived in
                if arrived { restorePlace() }
            }
            // And the other direction: every push and every pop is written
            // down. Cheap — a couple of hundred bytes of JSON on a navigation,
            // not on a poll — because `path` only changes when someone
            // navigates.
            .onChange(of: path) { _, routes in savePath(routes) }
            // The same, for the half of "where you were" the path deliberately
            // does not carry. Also only on a human action: `Connection`
            // publishes this when a chip is tapped and at no other time — a
            // poll that changes the whole fleet leaves it alone.
            .onChange(of: connection.lastFocus) { _, focus in saveFocus(focus) }
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
            // Focused on the terminal that was tapped. The inbox's door points
            // at a pane too now, so the two no longer disagree about whether to
            // name one — only about what they can name. Here, any terminal in
            // the workspace, including an idle or exited one. There, a blocked
            // agent or the worktree's diff, which is all that door lists. See
            // `Route.Focus`.
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
            // exact loss `WorkspaceView` exists to prevent.
            //
            // That used to cost you the tab you were actually on: a restored
            // path re-ran the focus rule, so reading a diff, being interrupted
            // and coming back landed you on a blocked agent instead. It does
            // not any more, and not because the path changed — the memory lives
            // beside it, in `Connection.lastFocus`, where changing it moves no
            // navigation state at all. See `Route.Focus` for the order the two
            // are read in.
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
    /// Once, and only once. After this the path is whatever the person holding
    /// the phone has done with it, and a second pass on a later reconnect would
    /// be the app steering them somewhere they had already left.
    ///
    /// Two halves, because where you were is two facts: which screens were
    /// stacked up, and which tab was showing inside the last of them. They are
    /// stored apart for the reason `savedFocus` gives, and restored in an order
    /// that matters — the remembered tabs go in FIRST, because installing the
    /// path is what mounts a `WorkspaceRoute`, and that view reads the memory in
    /// the same turn it appears. Seeded afterwards, the workspace would already
    /// have opened on the rule's answer and latched it. `restorePath` then puts
    /// a question of its own to the same memory — see `deferringToMemory(_:)` —
    /// so the order is load-bearing twice over, and swapping these two lines
    /// costs you the tab you were on in two different ways.
    private func restorePlace() {
        guard !restoredPlace else { return }
        restoredPlace = true
        restoreFocus()
        restorePath()
    }

    /// The tabs, checked against the fleet that just arrived.
    ///
    /// Restored whether or not the path is — the memory is about workspaces
    /// rather than about a location, so it is worth having for a workspace
    /// opened by hand ten minutes from now as much as for the one coming back
    /// on the path.
    ///
    /// Entries are dropped, not repaired, and both halves are checked: the
    /// workspace has to still be in the fleet, and an `.agent` has to still name
    /// a pane in it. `WorkspaceRoute` would degrade a dead one to the rule
    /// anyway, so this is not what makes the app correct — it is what stops the
    /// stored value accumulating worktrees that were merged away months ago.
    private func restoreFocus() {
        guard let data = savedFocus.data(using: .utf8),
            let saved = try? JSONDecoder().decode(SavedFocus.self, from: data),
            saved.runner == host.id.uuidString
        else { return }

        let live = connection.fleet.workspaces
        let usable = saved.focus.filter { workspace, focus in
            guard let workspace = live.first(where: { $0.id == workspace }) else { return false }
            guard case .agent(let terminal) = focus else { return true }
            return workspace.terminals.contains { $0.id == terminal }
        }
        guard !usable.isEmpty else { return }
        connection.seedFocus(usable)
    }

    /// The screens, in the order they were pushed.
    ///
    /// Truncated at the first route that no longer resolves rather than
    /// filtered, because a path is a sequence of pushes: keeping depth 2 after
    /// dropping depth 1 would put a screen on top of a screen nobody navigated
    /// through. A path that resolves to nothing simply leaves you at the root,
    /// which is where a cold launch has always landed.
    ///
    /// Restored as it was written down except for the focus, which a route
    /// gives up where there is a fresher record of where you were — see
    /// `deferringToMemory(_:)`.
    private func restorePath() {
        guard path.isEmpty, let data = savedPath.data(using: .utf8),
            let saved = try? JSONDecoder().decode(SavedPath.self, from: data),
            saved.runner == host.id.uuidString
        else { return }

        let usable = Array(saved.routes.prefix(while: resolves)).map(deferringToMemory)
        guard !usable.isEmpty else { return }
        // No push animation for a screen you never left. Restoring where you
        // were should look like the app remembering, not like it navigating.
        var silent = Transaction()
        silent.disablesAnimations = true
        withTransaction(silent) { path = usable }
    }

    /// A route on its way back out of `@SceneStorage`, with its focus handed
    /// over to the tab that workspace was last left on.
    ///
    /// A route's focus is the door somebody came in by, and it wins outright
    /// because on the way IN it is the most recent thing anybody asked for: the
    /// inbox row that named the blocked agent, the workspace list row that named
    /// the terminal. None of that is true of a route being restored. The door
    /// was walked through before the app was killed, and a chip tapped on the
    /// far side of it is strictly newer — so what the door said is history and
    /// where the person actually ended up is what they want back. `.none` is how
    /// a route says it has no opinion, so writing it here is exactly how the
    /// door steps aside and `WorkspaceRoute.resolve()` reaches the memory.
    ///
    /// Only where there IS one, which is why this asks rather than blanking
    /// every restored route. `Connection.lastFocus` records chip taps and
    /// nothing else — deliberately, see `Connection.rememberFocus` — so a
    /// workspace opened straight onto an agent and never tabbed away from has no
    /// entry at all. There the door is not stale evidence of where somebody was
    /// sitting, it is the only evidence, and blanking it would drop them on the
    /// rule's answer instead: the same lost place this exists to prevent,
    /// arrived at through the other door.
    ///
    /// The entry is trusted rather than re-checked, because `restoreFocus` ran
    /// first in the same turn and against the same fleet, and admits an entry
    /// only if that fleet still has the workspace AND the pane it names. If it
    /// dies later anyway, `resolve` degrades it to the rule, which is a better
    /// answer than a door two interruptions ago.
    ///
    /// What this returns is also what gets written back down, since saving the
    /// path is on a change to it. That is the intent and not a leak: the route
    /// has no opinion any more, and saying so is what the next restore should
    /// read.
    private func deferringToMemory(_ route: Route) -> Route {
        guard case .workspace(let id, _) = route, connection.lastFocus[id] != nil else {
            return route
        }
        return .workspace(id: id, focus: .none)
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

    private func saveFocus(_ focus: [String: Route.Focus]) {
        let saved = SavedFocus(runner: host.id.uuidString, focus: focus)
        guard let data = try? JSONEncoder().encode(saved),
            let json = String(data: data, encoding: .utf8)
        else { return }
        savedFocus = json
    }

    /// The path and the runner it was walked on, which is the whole of what
    /// `savedPath` holds. See it for why the runner has to travel with it.
    private struct SavedPath: Codable {
        var runner: String
        var routes: [Route]
    }

    /// The remembered tabs and the runner they were chosen on. The runner
    /// travels with them for the reason it travels with the path: workspace and
    /// terminal ids mean nothing on a different one.
    private struct SavedFocus: Codable {
        var runner: String
        var focus: [String: Route.Focus]
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
    ///
    /// Built on `failure(_:)`'s skeleton, which is the shape this screen should
    /// always have had: a mark, a headline, a sentence, and the actions anchored
    /// at the bottom where a thumb is. What it was instead is the exact layout
    /// that function's own comment says it was rebuilt to stop being — a
    /// leading-aligned stack of pill buttons of two different widths floating in
    /// the middle of the view, a ragged staircase giving the eye no line to
    /// follow, with the bottom third of a tall screen empty underneath it. Two
    /// screens one connection apart disagreeing about that shape is bad enough;
    /// that this is the FIRST screen a newly added runner produces made it the
    /// worst place in the app to leave the older one standing.
    ///
    /// The fingerprint goes in a `DetailBox`, which is where every other piece
    /// of host output in this app goes — `failure`'s own undiagnosed message
    /// twenty lines down, the adapter editor's, the task composer's. It was a
    /// hand-rolled radius-10 rectangle over `secondarySystemBackground`: one
    /// more invention of a container the app already has, and one that made the
    /// runner's words look like the app's own prose. `DetailBox` keeps the
    /// selection, so a fingerprint is still something you can copy and compare.
    private func approval(_ fingerprint: String) -> some View {
        VStack(spacing: 0) {
            Spacer()

            // Neither amber nor red. Nothing has gone wrong and no agent is
            // waiting on anyone — a runner this device has not met is a
            // question, which is what the mark and the headline both say.
            Image(systemName: "questionmark.circle")
                .font(.system(size: 42, weight: .thin))
                .foregroundStyle(.tertiary)
                .padding(.bottom, 22)

            Text("Unrecognized Runner")
                .font(.title2.weight(.semibold))
                .multilineTextAlignment(.center)
                .padding(.bottom, 8)

            Text("\(host.address) presented a key this device hasn’t seen before.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 320)

            DetailBox(text: fingerprint)
                .frame(maxWidth: 320)
                .padding(.top, 14)

            // The one thing that makes the fingerprint above worth showing:
            // where to get the other copy of it. Selectable, because it is a
            // command somebody has to run somewhere else.
            Text("Check it on the host: ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .textSelection(.enabled)
                .frame(maxWidth: 320)
                .padding(.top, 14)

            Spacer()

            VStack(spacing: 18) {
                Button {
                    store.trust(host, fingerprint: fingerprint)
                    var trusted = host
                    trusted.fingerprint = fingerprint
                    Task { await connect(trusted) }
                } label: {
                    Text("Trust This Runner").frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)

                // The other answer. Saying no used to have nowhere to go — this
                // screen had one button on it — which made "I am not sure about
                // this fingerprint" and "yes, trust it" the same tap for anyone
                // who just wanted out. It leaves the host untrusted and lands on
                // the failure screen, which is where the switcher and the editor
                // are.
                //
                // Plain text rather than a second bordered pill, for the reason
                // `failure` gives about its own alternatives: two pills give two
                // things the same weight when only one of them is the answer.
                Button("Not Now") {
                    connection.declineHostKey(host)
                }
            }
            .padding(.horizontal, 40)
            .padding(.bottom, 40)
        }
        .padding(.horizontal)
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

            // Only where the app has no diagnosis of its own — the same
            // scoping the Mac's `ChangesPane` uses, and for the same reason: a
            // transcript under a sentence that already names the cause and the
            // fix is noise.
            //
            // Nothing is discarded. For a runner nobody can reach, this text is
            // the only diagnosis that exists, and somebody debugging one needs
            // it. It just goes where output goes rather than where prose does,
            // so the app stops appearing to have said it.
            if kind == .other, !message.isEmpty {
                DetailBox(text: message)
                    .frame(maxWidth: 320)
                    .padding(.top, 14)
            }

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
        // Sentences somebody wrote, each naming both what happened and what to
        // do about it — three of them in `Connection`, `hostKeyChanged` in
        // `crates/client/src/ssh.rs`. They are the core's words only in the
        // sense that the core is where they are stored.
        case .hostKeyChanged, .noIdentity, .keyNotTrusted, .stopped:
            return message
        // The undiagnosed arm, and the only one where `message` is whatever
        // came back rather than something written to be read. Those words go
        // into a `DetailBox` in `failure(_:)` instead of standing here as the
        // app's own account of the runner.
        //
        // No cause named, deliberately: from this side the cause is unknowable,
        // and a guess sends somebody to loosen an sshd setting that was never
        // the problem. See `Enrollment.note(about:outcome:)`. Nor any retry
        // promised — whether one is under way is `retryOrGiveUp`'s business,
        // and the button below is the only offer this screen makes.
        case .other:
            return "The attempt to reach it didn’t finish."
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
/// STARTING pane, and only that. `WorkspaceView` reports the chips a person
/// taps to `Connection.lastFocus`, which this view reads on the way in and
/// never again — a memory read once at mount cannot rebuild anything, which is
/// the whole reason it is a dictionary on the connection and not a value in the
/// path.
///
/// A focus naming an agent that has gone falls back — to the remembered tab,
/// then to the rule — rather than to a placeholder, which is what makes a path
/// restored hours later land somewhere useful. See `Route.Focus`.
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
        // Three answers in order of authority — what the door asked for, then
        // the tab this workspace was last left on, then the rule. Each one is
        // filtered through `alive` first, so a `.none` here always means "this
        // answer named a pane that is gone" or "this answer had no opinion",
        // and both fall through to the next for the same reason. See
        // `Route.Focus`, which sets the order out in full.
        var wanted = alive(focus, in: workspace)
        if case .none = wanted, let remembered = connection.lastFocus[workspace.id] {
            wanted = alive(remembered, in: workspace)
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

    /// A focus, or `.none` when what it names has left this worktree.
    ///
    /// The one degradation rule, and it is applied to both of the answers that
    /// can be stale rather than only to the route's. An agent stopped overnight
    /// is exactly as gone whether the path named it or you last read it, and
    /// answering "the pane you want is not here" differently in the two cases is
    /// how a screen ends up opening onto nothing. The rule needs no such filter,
    /// because it picks out of the fleet as it is right now.
    ///
    /// `.changes` always survives: nothing on the runner has to exist for the
    /// worktree's own diff. See `Pane`.
    private func alive(_ focus: Route.Focus, in workspace: Workspace) -> Route.Focus {
        guard case .agent(let id) = focus else { return focus }
        return workspace.terminals.contains { $0.id == id } ? focus : .none
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
        // No `.scrollContentBackground(.hidden)` any more. It let the terminal
        // palette show behind these rows, and an inset-grouped row draws
        // `secondarySystemGroupedBackground` — `#1C1C1E` dark, `#FFFFFF` light,
        // fixed colors rather than theme ones. On five of the seven dark themes
        // that made the card DARKER than its own ground, which is dark mode's
        // elevation rule upside down, and on the three light themes with a
        // `#FFFFFF` background it made card and ground identical, so the rows
        // had no card at all. `NeedsYouView` carries the numbers and the full
        // argument; the short version is that the theme is the terminal's
        // palette and a list of workspaces is chrome.
        .toolbar {
            // A sparkle for "describe it" (`TaskComposerView`), plain plus for
            // "fill in the form" (`NewWorkspaceView`) — same two flows the
            // Mac keeps side by side, kept apart here by icon rather than
            // by picking a winner, since a phone's one-sentence flow is
            // new and unproven next to a form that already works.
            //
            // `sparkle` rather than `sparkles`: the Mac's `QuickCreate` and its
            // command palette both mark this flow with the singular, and one
            // concept gets one glyph. There is no argument either way beyond
            // that, which is itself the argument for taking the Mac's.
            //
            // Named out loud, because an `Image` alone in a `Button` is read
            // as its SF Symbol: "sparkle" and "plus" were the whole of what
            // VoiceOver had to tell the app's two ways of starting work apart,
            // and neither is a word this product uses. Each is named for the
            // sheet it opens — Quick Task's own title, and the workspace form's
            // title-cased as a button label is — so the button and the screen
            // behind it are one name, and `FleetList`'s empty state points at
            // these two by exactly these names.
            ToolbarItem(placement: .topBarTrailing) {
                Button { showQuickTask = true } label: { Image(systemName: "sparkle") }
                    .accessibilityLabel("Quick Task")
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button { showNewWorkspace = true } label: { Image(systemName: "plus") }
                    .accessibilityLabel("New Workspace")
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
                // The only way to change runners in the app, and it was about
                // 21 points tall. The words keep their size; the band around
                // them is the guideline's 44, and `contentShape` makes that band
                // live rather than merely occupied — padding on a menu label is
                // layout only otherwise. Same move the tab strip's chips made.
                .frame(minHeight: PaneMetrics.target)
                .contentShape(.rect)
            }

            Spacer(minLength: 0)

            LinkStatusChip(connection: connection)
        }
        .padding(.horizontal, 16)
        // The 10 points of vertical padding that used to be here are gone, and
        // the height moved into the two controls instead. Both are 44 now, so
        // the bar is 44 rather than the 41 it was — three points for two targets
        // that clear the floor, on the one strip that is under every phase of
        // this screen including the ones you cannot otherwise leave.
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
/// is noise, and a dot the eye passes over says the same thing in no space at
/// all. It used to say "the absence of amber", and amber is no longer this
/// chip's to spend: orange means an agent is waiting on you, everywhere in this
/// app and on the widget, the Live Activity and the complication.
///
/// The comment was right and the code had drifted: it said "the absence of a
/// colored dot" and then drew a GREEN one, which is the color this app gives a
/// finished agent. So the chip is down to two colors — neutral for a link with
/// nothing wrong with it, whether it is up or on its way up, and red for one
/// that has stopped. Which of the two neutral cases you are in is carried by
/// the word beside the dot, in a channel that costs no color at all.
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
            //
            // The comment was right and the arithmetic was not: 7 points of dot
            // plus 6 above and 6 below is 19, and with the runner bar's own
            // padding the whole thing came to about 25. The horizontal padding
            // still holds the label off the edges; the height is the
            // guideline's, and `contentShape` makes all of it live.
            .padding(.vertical, 6)
            .padding(.leading, 8)
            .padding(.trailing, label == nil ? 8 : 10)
            .frame(minHeight: PaneMetrics.target)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label ?? "Connected")
        .accessibilityHint("Reconnects to this runner")
    }

    private var color: Color {
        switch connection.phase {
        // Not green, per the note above. Not nothing either, which is what the
        // Mac's `HostDot` draws for a connected runner: that dot is not a
        // button in that state and this one is. This IS the escape hatch — the
        // paragraph below is the argument that the tap has to work when the app
        // believes the link is fine and the person holding it can see that it
        // is not — and an invisible button fails exactly then.
        case .connected: return .secondary
        // In progress, not attention, and no longer yellow: a pane that is
        // `starting` gave up its yellow for the same reason, which
        // `processColor` states in full.
        case .connecting, .reconnecting: return .secondary
        // Red is for a fault, and `daemonMissing` is not one. SSH worked; Far
        // Cooler simply is not over there yet, which is one `host install`
        // away. The Mac has said this all along — `notInstalled` is
        // `.secondary` in both `HostDot` and `troubleColor` — and this app's
        // own full-screen failure agrees, drawing every kind but a changed host
        // key in `.tertiary`. This chip was the one surface still calling it a
        // failure.
        case .failed(let message):
            return Connection.Failure(message: message) == .daemonMissing ? .secondary : .red
        // Left red deliberately. A fingerprint nobody has answered is not a
        // fault, but until somebody does this device cannot talk to that runner
        // at all, and the row is the one place that says so.
        case .needsApproval: return .red
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

    var body: some View {
        List {
            if fleet.workspaces.isEmpty {
                // What this screen is FOR, rather than only what it lacks.
                //
                // The door that leads here promises it — "This is where you
                // start one", in `NeedsYouView.workspacesFooter` — and arriving
                // at "No workspaces on this runner." was that promise going
                // unanswered by the screen that has to keep it. This is the one
                // place on the phone where work is started, and a person who
                // took the sentence at its word landed on a flat denial.
                //
                // `ContentUnavailableView` rather than a hand-rolled headline:
                // it is the platform's own empty state, at the size and rhythm
                // iOS gives every other one, and the Mac's workspace
                // placeholder and the watch's already use it.
                //
                // **No `actions:` block, deliberately.** The two buttons that
                // start work are in this screen's own toolbar, a thumb's reach
                // above this text, and the comment on them records a decision
                // NOT to pick a winner between the two flows. One button here
                // would pick it; both would be four controls for two actions on
                // one screen, which is the competing pair to avoid. The
                // sentence names them instead, in the words their accessibility
                // labels use — so the button a reader is sent to is the button
                // they can find, by ear or by eye.
                //
                // Still a row inside the `List`, so pull-to-refresh survives: a
                // runner that has nothing yet is exactly the fleet worth
                // pulling on.
                ContentUnavailableView(
                    "No Workspaces",
                    systemImage: "arrow.triangle.branch",
                    description: Text(
                        "This is where you start one, with Quick Task or New Workspace.")
                )
                .listRowBackground(Color.clear)
                .listRowSeparator(.hidden)
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
                        row(terminal, ordinal: numbering[terminal.id])
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
                            // First in and last squeezed. Seven things share
                            // this line and the task is the only one that says
                            // WHICH worktree this is; without a priority it was
                            // simply the first `Text` the layout reached for
                            // when the branch and the counts wanted room.
                            .layoutPriority(1)
                        // Something under here wants you, said once at the top
                        // rather than left to be inferred from a row further
                        // down that may be scrolled off.
                        //
                        // In the color of the row you get by expanding it, not
                        // in orange whatever is inside — see `mostUrgent(in:)`.
                        // A header that says orange over a row that says red is
                        // a header pointing somewhere else.
                        if let leader = mostUrgent(in: workspace.terminals) {
                            Circle()
                                .fill(attentionColor(leader))
                                .frame(width: 6, height: 6)
                                .accessibilityLabel(leader.activityLabel)
                        }
                        // The worktree this workspace names is not on disk any
                        // more. Said on the header because every row under it
                        // is about a pane in a directory that is gone, and
                        // "Removed" beats twenty terminals failing separately.
                        if workspace.worktreeMissing {
                            Text("worktree gone")
                                .font(.caption2)
                                // Red, not amber. A worktree that is no longer
                                // on disk is a failure, and amber in this app
                                // means an agent is waiting on you — which the
                                // dot two views to the left is already saying,
                                // in the same color, about something else.
                                .foregroundStyle(.red)
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
                            // Read aloud, the parts are "plus 82" and "minus
                            // 13" — two numbers with nothing attaching them to
                            // a diff, which is what this said until now. The
                            // clause is the phone's whole share of what the
                            // Mac's sidebar tooltip explains: this number is
                            // more than the branch has committed, and nothing
                            // on the row said so. No hover here, so the label
                            // is the only place it can be said.
                            .accessibilityElement(children: .ignore)
                            .accessibilityLabel(
                                "\(counts.insertions) added, \(counts.deletions) removed, "
                                    + "including work that isn’t committed yet")
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
                        // Hide, Unhide, Remove Worktree and the stack all live
                        // behind this glyph and nowhere else on the phone, and
                        // it inherited a grouped header's `.footnote` — a
                        // roughly 22-point target for the only destructive
                        // action on the screen. The glyph is `.title3` and its
                        // band is 44, which costs this header about 24 points of
                        // height per workspace. Worth it: a header is read once
                        // on the way past and this is the one control in it.
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
                                .font(.title3)
                                .foregroundStyle(.secondary)
                                .frame(
                                    minWidth: PaneMetrics.target,
                                    minHeight: PaneMetrics.target,
                                    alignment: .trailing)
                                .contentShape(.rect)
                        }
                    }
                    // Against the grouped list's uppercasing, which is right for
                    // the word "SETTINGS" and wrong for both halves of this: the
                    // task is a sentence somebody wrote, and a branch is an
                    // identifier whose case is not ours to change —
                    // `FEAT/ADD-AUTH` is not a branch that exists. `NeedsYouView`
                    // states this rule for the same two strings one tap away,
                    // and this header was the copy that never got it.
                    .textCase(nil)
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
                                row(terminal, ordinal: numbering[terminal.id])
                            }
                            HStack {
                                Text(workspace.task).font(.caption).foregroundStyle(.secondary)
                                Spacer()
                                Button("Unhide") {
                                    Task { await connection.unhideWorkspace(workspace) }
                                }
                                .font(.caption)
                                .frame(minHeight: PaneMetrics.target)
                                .contentShape(.rect)
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
                            // Same rule as a workspace header: the color of
                            // what you would find, not orange regardless.
                            if let leader = mostUrgent(in: hidden.flatMap(\.terminals)) {
                                Circle()
                                    .fill(attentionColor(leader))
                                    .frame(width: 6, height: 6)
                                    .accessibilityLabel(leader.activityLabel)
                            }
                            Spacer()
                        }
                        // The only route to a hidden workspace, in a header that
                        // gave it about 20 points of height.
                        .frame(minHeight: PaneMetrics.target)
                        .contentShape(.rect)
                    }
                    .buttonStyle(.plain)
                }
            }

            Section {
                HStack {
                    Circle()
                        // Red rather than amber for "tmux unavailable". No
                        // agent is waiting on anyone; the runtime that every
                        // pane on this runner lives inside is not answering,
                        // which is a failure. The audit did not list this one
                        // and it is the same bug as the six it did.
                        //
                        // And neutral rather than green for the healthy case,
                        // which was the OTHER half of the same bug: green is
                        // what an agent that has finished its work wears, and
                        // this spent it on "nothing is wrong" at the foot of the
                        // list where those agents are. The count beside it says
                        // the fleet is alive; the colour is now reserved for the
                        // sentence that is news.
                        .fill(fleet.runtimeHealthy ? Color.secondary : Color.red)
                        .frame(width: 8, height: 8)
                    Text(
                        fleet.runtimeHealthy
                            ? "\(fleet.livePanes) live"
                            // The same three words the Mac and Android show. This one
                            // said "on this host" — the wrong noun for a runner, and
                            // redundant on a screen that speaks for one already.
                            : "tmux unavailable"
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

    /// One terminal, as a row you tap to open it.
    ///
    /// Shared by the visible workspaces and the hidden ones, which drew the same
    /// row twice with the same identifier and the same swipe actions.
    private func row(_ terminal: Terminal, ordinal: Int?) -> some View {
        Button { onSelect(terminal) } label: {
            HStack(spacing: PaneMetrics.step) {
                TerminalRow(terminal: terminal, ordinal: ordinal)
                    // A list row's label otherwise sizes to its text. Give
                    // selection the whole visible row, including the blank
                    // trailing space a thumb naturally lands in.
                    .frame(maxWidth: .infinity, alignment: .leading)

                // The same row is a `NavigationLink` on Needs You, where the
                // system draws this, and was a bare `Button` here — so one row
                // said "this opens something" and the identical row one tap away
                // said nothing at all. Drawn by hand because the tap is not a
                // push: it goes up to `FleetView.show(_:)`, which may replace
                // the route or dismiss the sheet this list is sitting in.
                //
                // Hidden from VoiceOver, which is what the system does with its
                // own: the row already says where it goes.
                Image(systemName: "chevron.forward")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("fleet-terminal-\(terminal.id)")
        .swipeActions(edge: .trailing) { terminalActions(for: terminal) }
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

/// The column every row in a workspace section is built on.
///
/// A `TerminalRow` draws an 8-point state dot and then its words 10 points
/// after it, so its text starts 18 points in. Nothing else on either screen knew
/// that number: `NeedsYouView`'s overflow line started at 0 and its changes row
/// at roughly 24, which is three text edges inside one section — three margins
/// for the eye to find on a screen whose whole job is to be read at a glance.
/// Named here, beside the row that sets it, so the other two can ask instead of
/// guessing.
///
/// Not on `PaneMetrics`' scale, and deliberately so: that scale is about how far
/// apart two things sit, and this is the width of one specific column. The 10 is
/// what shipped and what the rows are drawn to; moving it to the scale's 8 would
/// re-cut every row in the app to buy one fewer number.
enum RowGutter {
    /// The state dot's diameter, and the width of the column it sits in.
    static let dot: CGFloat = 8
    /// The gap between that column and what it labels.
    static let gap: CGFloat = 10
    /// Where a row's text starts.
    static let text: CGFloat = dot + gap
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
        HStack(alignment: .firstTextBaseline, spacing: RowGutter.gap) {
            ProcessDot(kind: kind)
                // The dot sits on the first line's baseline rather than in the
                // middle of a row that is now up to four lines tall.
                //
                // The comment was right and the code only approximated it. It
                // was a hardcoded `.padding(.top, 6)`, measured against `.body`
                // at the default Dynamic Type size — and the first line's
                // baseline moves with the type size while 6 points does not, so
                // the dot drifted up the line as the text grew and down it as
                // the text shrank, at every setting but one. Aligned to the
                // baseline itself now: the dot rests ON it, the way a period
                // does, and the number is the one SwiftUI computes rather than
                // one this file remembers.
                .alignmentGuide(.firstTextBaseline) { $0[.bottom] }

            // A full step between the two groups, a tight one inside each.
            //
            // Every band was `spacing: 3` — one distance for everything, which
            // is a stack with no grouping in it. What the pane IS and where its
            // agent got to are one thought in two lines; what the agent SAID is
            // a different thought, and the gap is now the thing that says so.
            VStack(alignment: .leading, spacing: PaneMetrics.step) {
                VStack(alignment: .leading, spacing: PaneMetrics.tight) {
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
                        //
                        // Six silhouettes, where the Mac's `StatusGlyph` forbids
                        // them outright — "not a moon for idle and a gearwheel
                        // for working" — and that is deliberate. The Mac's dot
                        // is one pointer-rest from explaining itself: every
                        // `StatusGlyph` carries a `.help` with the state's name
                        // in it. A phone has no hover, so a mark that encodes
                        // its meaning in hue alone is unrecoverable — and the
                        // pair a reader most needs separated, blocked and done,
                        // is orange-filled against green-filled, which is
                        // exactly the pair that collapses for the commonest kind
                        // of color blindness. The shape is what survives that.
                        //
                        // The two halves of the Mac's complaint that ARE about
                        // this list were taken: the mark is one size (Android
                        // stepped 16→20 on attention and moved its own column),
                        // and it is filled for the states that want a person and
                        // outlined for the ones that do not, which is
                        // hollow-vs-filled in SF Symbols' own vocabulary.
                        //
                        // The weight below still steps, and that is safe HERE
                        // and nowhere else: this glyph is anchored against the
                        // `Spacer` above it, so a semibold mark grows leftward
                        // into slack and nothing after it moves. `TabChip` keeps
                        // one weight for the opposite reason — a chip that grows
                        // slides every chip after it sideways.
                        //
                        // `.footnote`, which is 13 points at the default size —
                        // the same 13 this was hardcoded to as
                        // `.system(size: 13)`, and now the same 13 that GROWS
                        // when the words beside it do. A fixed-size glyph next
                        // to scaling text is a mark that shrinks relative to its
                        // own row at every accessibility size, and this one is on
                        // every row in the app.
                        if terminal.agent.isAgent && terminal.agent != .unknown {
                            Label(terminal.activityLabel, systemImage: terminal.activitySymbol)
                                .labelStyle(.iconOnly)
                                .font(
                                    .footnote.weight(
                                        terminal.agent.wantsAttention ? .semibold : .regular))
                                .foregroundStyle(attentionColor(terminal))
                                .accessibilityLabel(terminal.activityLabel)
                        }
                    }

                    // Band 2: where the agent IS — the question it is blocked
                    // on, its position in its own task list, or what it is
                    // doing. One line, composed on the host so a Mac, a phone
                    // and a watch cannot disagree about which of those three to
                    // show.
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
                }

                // Bands 3 and 4: what the agent said it did, and what it
                // spawned and has not finished with.
                //
                // `.secondary`, not `.tertiary`. `NeedsYouView` calls these
                // lines "the part of the row that answers 'what did it do',
                // which the owner is explicit is most of what reviewing an
                // agent's work is" — and they were drawn in the faintest tier
                // the platform offers, at 11 points, in the same tier as the
                // ordinal beside the title, which nobody reads. The ordinal
                // stays tertiary; it is a disambiguator and it earns that tier.
                //
                // Wrapped rather than left loose, and guarded on being non-empty
                // — an empty group would otherwise still take the step of
                // spacing above it and leave a gap under every one-agent row.
                if !terminal.recentSteps.isEmpty || !terminal.runningSubagents.isEmpty {
                    VStack(alignment: .leading, spacing: PaneMetrics.tight) {
                        // Already redacted and cut to a row's width by the
                        // daemon, so this renders them and decides nothing about
                        // them.
                        ForEach(Array(terminal.recentSteps.enumerated()), id: \.offset) { _, step in
                            Text(step)
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.tail)
                        }

                        ForEach(terminal.runningSubagents, id: \.self) { name in
                            Text("\u{2442} \(name)")
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.tail)
                        }
                    }
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
            // of what is known about it. Exited is worth saying; running is the
            // ordinary case and `ProcessDot` now draws nothing for it, so a
            // live shell is a row with a name on it and nothing else — which is
            // the point of the silence, not a fact withheld.
            return kind == .running ? nil : terminal.state.lowercased()
        }
        guard let elapsed = terminal.displayDuration(at: now) else { return terminal.activityLabel }
        return "\(terminal.activityLabel) \(elapsed)"
    }
}

/// The dot for "is the process alive" — shared by the fleet list and the
/// terminal tab strip (`TerminalTabStrip`), so the same terminal cannot read
/// one way in one screen and another way in the other.
///
/// **Silence is the default, and green is spent once.** This was `.green` for
/// `running`, which put a green dot on every row of a list where green already
/// meant something else: `attentionColor` gives it to `done`, so an idle
/// `zsh` and an agent that had just finished its work wore the same mark, and
/// on a busy worktree they wore it three rows apart. A running process is the
/// ordinary case and now says nothing; green survives on exactly one state,
/// the same one the Mac spends it on. See `StatusGlyph` over there, whose
/// `.idle, .running, .exited` branch has drawn `Color.clear` since `8bce995`.
/// `ProcessDot` reserves the column either way, so names line up down the list
/// rather than stepping in and out as panes start and stop.
///
/// **Not yellow for `starting`.** A pane is starting for well under a second,
/// and a color nobody has time to read is a color spent for nothing. The Mac
/// calls `starting` in-progress and paints it `.secondary`; so does this now.
///
/// **`exited` keeps its dot**, where the Mac draws nothing for it. The Mac can
/// afford silence because its fused `Status` puts "Exited" on the row in
/// words; `TerminalRow.statusText` spells the state out only for panes that
/// are NOT agents, so an agent pane whose process is gone would otherwise lose
/// its only mark.
func processColor(_ kind: StateKind) -> Color {
    switch kind {
    case .running: return .clear
    case .starting, .exited, .unknown: return .secondary
    // The one state that means Far Cooler does not know what happened.
    case .lost, .error: return .red
    }
}

/// That dot, drawn — one shape, one size, and a hollow one where something is
/// missing.
///
/// **Shape before color.** Hollow says "something is not there" before the hue
/// does, which is the half of this vocabulary that survives a colorblind
/// reader, a grayscale screenshot and a phone in bright sun. The phones had
/// dropped it entirely and drew every state as a filled disc, leaving hue as
/// the only channel; `StatusGlyph.mark` on the Mac has carried it all along,
/// with the same three states hollow — a pane that died where the app was not
/// looking, one that failed to start, and one the runner would not report on.
///
/// **One size**, `RowGutter.dot`. The tab strip drew 6 and this list drew 8 for
/// the same mark; that is the drift the Mac had to be pulled back from, where
/// eleven call sites passed five diameters of a glyph whose whole argument is
/// that it is always the same mark. Eight is the one that is load-bearing:
/// `RowGutter.text` is measured off it, and `NeedsYou`'s changes row puts its
/// symbol in a frame of exactly this width so the two sets of words share an
/// axis.
struct ProcessDot: View {
    let kind: StateKind

    var body: some View {
        Group {
            if hollow {
                // 1.5, which is `StatusGlyph`'s line width for the same ring at
                // the same diameter — a third of the dot, so the hole reads as
                // a hole rather than as a slightly soft disc.
                Circle().strokeBorder(processColor(kind), lineWidth: 1.5)
            } else {
                Circle().fill(processColor(kind))
            }
        }
        .frame(width: RowGutter.dot, height: RowGutter.dot)
    }

    private var hollow: Bool {
        switch kind {
        case .lost, .error, .unknown: return true
        case .running, .starting, .exited: return false
        }
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

/// The one terminal a roll-up should wear the color of, out of everything
/// waiting inside it.
///
/// A collapsed workspace header cannot show six dots, so it shows the one that
/// decides the color. It was showing solid orange for all of them, which is the
/// bug the Mac fixed in `90c8dd6` and the phone inherited: a worktree whose only
/// pending item is a FAILED agent said orange in the header and red one tap
/// down, so the same pane had two colors decided by whether the section was
/// expanded. Orange also means one specific thing across this app, the widget,
/// the Live Activity and the complication — an agent waiting on an answer — and
/// spending it on a worktree where nothing is waiting weakens it everywhere.
///
/// The ranking is `Status.mostUrgent(in:)`'s, for the reason stated there:
/// blocked first, because an agent stalled mid-turn is the state this
/// application exists for and the only one where the work resumes the moment you
/// look; a dead turn next, because something stopped; `done` last, because
/// nothing is waiting on anything — it is finished and merely unread.
///
/// Nil when nothing inside wants attention, which is also when nothing should be
/// drawn.
func mostUrgent(in terminals: some Sequence<Terminal>) -> Terminal? {
    terminals
        .filter(\.agent.wantsAttention)
        .min { attentionRank($0) < attentionRank($1) }
}

private func attentionRank(_ terminal: Terminal) -> Int {
    if terminal.agent == .blocked { return 0 }
    if terminal.turnDidFail { return 1 }
    return 2
}

/// What a sheet says when the thing it asked for did not happen: the app's own
/// sentence, and — when the app has no account of its own — the daemon's words
/// underneath it.
///
/// Two fields rather than one string, because the two are read differently and
/// must never be concatenated. `sentence` is Far Cooler talking; `transcript`
/// is what came back from the runner, and a runner's words set as body text
/// under a heading this app wrote is the app appearing to have said them.
struct SheetFailure {
    let sentence: String
    var transcript: String?
}

/// One failure, drawn the way this codebase already draws them: a written
/// sentence, then a `DetailBox` holding the transcript.
///
/// One view rather than a copy in each sheet, so two sheets reporting the same
/// kind of failure cannot come to render it differently — the principle
/// `f9f37eb` and `776d3e0` both turned on. `DetailBox` itself is AgentKit's and
/// the Mac's; see `DaemonUpdateCard`, `RunnersSettings` and `ChangesPane`.
private struct SheetFailureSection: View {
    let failure: SheetFailure

    var body: some View {
        Section {
            Text(failure.sentence)
                .foregroundStyle(.red)
                .font(.footnote)
            if let transcript = failure.transcript, !transcript.isEmpty {
                DetailBox(text: transcript)
            }
        }
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
    @State private var failure: SheetFailure?

    private var matches: Bool { typed == workspace.task }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text("This workspace has uncommitted changes. Enter its name to remove it.")
                        .font(.footnote)
                        .foregroundStyle(.secondary)
                }
                TextField("Type \(workspace.task) to confirm", text: $typed)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                if let failure {
                    SheetFailureSection(failure: failure)
                }
            }
            .navigationTitle("Remove Worktree")
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
                            // The app's own diagnosis, and it is a complete
                            // one: the name typed is not the name on file.
                            // Nothing came back from the runner to show, and
                            // nothing needs to.
                            case .confirmationRequired:
                                working = false
                                failure = SheetFailure(
                                    sentence: "That name didn’t match — try again.")
                            // `message` is whatever the call came back with,
                            // and this side has no idea why. It used to be set
                            // into the very same red line the sentence above
                            // uses, which made a runner's words read as Far
                            // Cooler's. Kept — it is the only account of what
                            // happened — and put in the box instead.
                            case .failed(let message):
                                working = false
                                failure = SheetFailure(
                                    sentence: "Removing this worktree didn’t finish.",
                                    transcript: message)
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
    @State private var failure: SheetFailure?

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
                if let failure {
                    SheetFailureSection(failure: failure)
                }
            }
            .navigationTitle("Add Repository")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add") {
                        working = true
                        failure = nil
                        Task {
                            do {
                                try await connection.addRepositoryRoot(path: path)
                                let id = try await connection.registerRepository(path: path)
                                working = false
                                onRegistered(id)
                                dismiss()
                            } catch {
                                // Either of the two calls, and this side cannot
                                // tell which — nor what the runner made of the
                                // path. So one sentence about the step, and the
                                // runner's answer below it rather than in place
                                // of it.
                                working = false
                                failure = SheetFailure(
                                    sentence: "Adding this repository didn’t finish.",
                                    transcript: error.localizedDescription)
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
            .navigationTitle("New Workspace")
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

    /// Red rather than amber. A name the runner will refuse is a failure, and
    /// amber in this app means one thing — an agent waiting on you — which is
    /// not something a text field can be.
    private func refusal(_ message: String) -> some View {
        Label(message, systemImage: "exclamationmark.triangle")
            .font(.caption)
            .foregroundStyle(.red)
    }
}
