import SwiftUI

/// The fleet, and what stands in front of it until somebody answers.
///
/// What is left of what this was, and it is a good deal less than it was
/// twice over. It used to be the app's navigation — a `NavigationStack`, a
/// `[Route]` path, a persisted copy of that path, and the rules for restoring,
/// truncating and deferring to it — and all of that went when the shell became
/// the app's navigation. Then it used to OWN A CONNECTION, and stand in front
/// of it in four full-screen phases, one per `Connection.Phase`.
///
/// **That is the half the multi-runner port took.** A screen can only be about
/// one runner: with several connections a single failing one would blank the
/// whole app, and a newly added runner needing authorization would show its
/// screen to nobody whenever any other runner answered — which is the ordinary
/// case, and the one flow onboarding cannot afford to lose. `FleetStore` owns
/// every connection, and the phases are `RunnerStatusRow`s.
///
/// So what remains is two branches and the plumbing between them: the shell
/// once anything has a fleet, a list of rows until then, and the deep link that
/// arrives before either exists.
///
/// What did not change: every state shown here is DERIVED by the daemon at the
/// moment of asking. The phone never computes a terminal's state, because a
/// client that re-derives can disagree with the daemon and with the Mac about
/// the same terminal.
@MainActor
struct FleetView: View {
    let store: RunnerStore
    /// Every runner this app is talking to.
    ///
    /// It used to own a `Connection` of its own — one `@StateObject`, started
    /// by this view's `.task` and torn down with it. That is what made the
    /// runner a mode: the connection's lifetime was a screen's lifetime, so
    /// reaching another runner meant destroying the screen. The store owns
    /// every connection now, this view owns none, and there is no `host`
    /// argument any more because there is no one runner this screen is about.
    @ObservedObject var fleet: FleetStore

    /// The terminal a tapped Live Activity card asked for, held until a fleet
    /// arrives that has it.
    ///
    /// A card tapped at COLD LAUNCH delivers its URL before the first
    /// connection has produced a fleet, so looking the id up as it arrives
    /// finds nothing and the tap opens the app onto whatever it would have
    /// opened onto anyway. That is indistinguishable from a card that ignored
    /// the tap, which is the failure this whole task exists to remove — so the
    /// id is remembered instead, and the shell picks it up the moment there is
    /// a fleet to look in. See `dropUnknownTerminal`.
    @State private var pendingTerminal: String?

    /// Whether to offer a way off the spinner yet. See `waitedLongEnough`.
    @State private var stalled = false

    @Environment(\.scenePhase) private var scenePhase

    /// The runner being corrected, from the row that asked.
    @State private var editing: Runner?

    /// Whether this device's own key is on screen.
    ///
    /// A flag here rather than a `NavigationLink` in the row, because a row has
    /// no idea what it is inside: `RunnerStatusRow` is drawn both here, in a
    /// stack, and over the shell's overview, which deliberately has none. Where
    /// "Authorize This Device" goes is the placing screen's decision.
    @State private var authorizing = false

    var body: some View {
        // No `NavigationStack` around the connected app, and that is the whole
        // shape of this change.
        //
        // This view used to declare one with an explicit `[Route]` path, and
        // the argument for it was sound while there were screens to push: the
        // path had to sit beside the `Connection`, because every route was an
        // id that means something only against THIS runner's fleet. There are
        // no routes now. The shell is one screen — a workspace is a position in
        // it, a tab is a position in it, and moving between them is a gesture
        // rather than a push — so a stack around it would be a stack of one
        // with a navigation bar this design puts at the BOTTOM of the display
        // as a piece of glass.
        //
        // What still needs a stack is the screen before any fleet exists: it
        // is titled, and a runner row's "Authorize This Device" pushes into it.
        // That branch declares its own, which is also what keeps the shell out
        // of one — see `phases`.
        phases
            // `item:` and not a flag: with several runners the sheet has to
            // carry WHICH one a row asked to correct, and a flag plus a
            // separate lookup can present an editor for a runner the list has
            // moved on from.
            .sheet(item: $editing) { runner in
                HostEditorView(
                    existing: runner,
                    onSave: { store.update($0) },
                    onRemove: { store.remove($0) })
            }
            // The app coming back is the moment a backoff timer cannot predict.
            // `.background` is passed on too, so a phone in a pocket stops
            // polling — which is both a battery question and one plausible way
            // a session died in the first place.
            //
            // Every runner, not one: one scene phase fans out to N connections.
            // See `FleetStore.setActive`.
            .onChange(of: scenePhase) { _, phase in
                fleet.setActive(phase == .active)
            }
            // A workspace leaving the fleet no longer needs anything from this
            // view.
            //
            // The rule that stood here truncated the navigation path at the
            // first route naming a worktree the runner had stopped reporting,
            // because the screen underneath was a pane host pointed at a
            // workspace that no longer existed. The shell has no path to
            // truncate and cannot be pointed at a workspace that is not in the
            // fleet: `ShellPosition` is an INDEX, resolved against whatever
            // `ShellFleetMap.of` just built, and `ShellPaneTrack` prunes the
            // retained panes of terminals that have gone. A workspace removed
            // while you are in it is the fleet renumbering under a position,
            // which is the case that shape was chosen for.
            //
            // A card tapped at cold launch, arriving as `…://terminal/<id>`.
            //
            // Here rather than on the root view, because this is the screen
            // that stands over the fleet the id has to be looked up in — every
            // runner's, now, which is what makes a card about the machine in
            // the other room land rather than quietly resolve to nothing.
            // Routing it from the root would mean a second way to choose a
            // terminal, threaded down through views that know nothing about
            // one.
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
                // Not resolved here. `ShellScreen` derives the tab this id is
                // from the fleet it already holds and honors it on the next
                // body pass, which is the same code path a cold launch takes
                // once its first fleet lands. See `dropUnknownTerminal`.
                dropUnknownTerminal()
            }
            // The runner has answered and does not have it, so it is never
            // coming. Watched on the fleet's own generation rather than on
            // `hasFleet` alone, because the pane a card names can also be
            // stopped between the tap and the answer.
            // Any runner's poll is a fresh answer to "does anybody have this
            // pane". `entries` is republished on every one of them.
            .onChange(of: fleet.entries.count) { _, _ in dropUnknownTerminal() }
            .onChange(of: fleet.hasFleet) { _, _ in dropUnknownTerminal() }
    }

    /// Two branches, and the one place a `NavigationStack` is still declared.
    ///
    /// **It was four full-screen phases, one per `Connection.Phase`, and that
    /// shape is wrong in both directions once the app holds several
    /// connections.** A single failing runner would blank the whole app,
    /// because the failure phase owned the screen; and a newly added runner
    /// needing authorization would show its screen to nobody whenever any other
    /// runner answered — which is the ordinary case, and the one flow
    /// onboarding cannot afford to lose. `RunnerStatusRow` is the answer, and
    /// it is Android's: a row that appears next to the runner it concerns is
    /// the shape that survives N runners.
    ///
    /// So the branch is no longer a phase. It is whether ANY runner has said
    /// what it has: with a fleet there is a shell, and the runners in trouble
    /// are rows over the overview's grid. Without one there is nothing to draw
    /// a shell from — see `ShellScreen.seed` — and this screen is the rows on
    /// their own.
    ///
    /// The stack is around the second branch only. It is what gives it a title
    /// and what a row's "Authorize This Device" pushes into; the shell gets
    /// none, deliberately, because its own navigation is a gesture and the only
    /// navigation bar in it belongs to the overview, which declares a stack of
    /// its own.
    @ViewBuilder
    private var phases: some View {
        if fleet.hasFleet {
            connected
        } else {
            NavigationStack { escapable { waitingForAnyone } }
        }
    }

    /// Every runner, and what each of them is doing, while none of them has
    /// answered.
    ///
    /// The first-run-after-launch screen, and the failure screen, and the
    /// approval screen, all at once — because with several runners they are the
    /// same screen. Each row says only what its own runner is doing and offers
    /// only that runner's next move; a runner that is merely connecting says
    /// so, and one that has answered is not here at all because this branch is
    /// not drawn once anything has a fleet.
    ///
    /// A `ScrollView` and not a `List`: these rows are prose and controls, and
    /// a grouped list would put each one in a card with a chevron's worth of
    /// inset, which reads as a row you can tap into. There is nothing to tap
    /// into — the moves are the buttons.
    private var waitingForAnyone: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                // Not a per-runner spinner. Every one of these rows says
                // "Connecting…" on its own; a second spinner over the top would
                // be the app being anxious about a wait it is already
                // describing.
                ForEach(fleet.runners, id: \.host.id) { runner in
                    RunnerStatusRow(
                        connection: runner.connection,
                        host: runner.host,
                        // Named on the row, because nothing above it names
                        // them: this is a list of runners.
                        showsLabel: true,
                        onRetry: { fleet.retry(runner.host.id) },
                        onReconnectNow: { runner.connection.reconnectNow() },
                        onTrust: { store.trust(runner.host, fingerprint: $0) },
                        onReviewKey: { store.forgetKey(runner.host) },
                        onEdit: { editing = runner.host },
                        onAuthorize: { authorizing = true })
                    Divider().padding(.leading, 16)
                }

                // A device with runners, none of them connected yet, and a
                // stalled one somewhere in the list. Held back for a few
                // seconds rather than shown at once: a healthy connection
                // resolves well inside that, and a "Stop Waiting" flashing up
                // on every launch would read as though something were wrong
                // every time. After that it is the honest offer, because an
                // address that routes nowhere takes over a minute to fail on
                // its own — see `Connection.giveUp(on:)`.
                if stalled {
                    Button("Stop Waiting") {
                        for runner in fleet.runners {
                            runner.connection.giveUp(on: runner.host)
                        }
                    }
                    .buttonStyle(.bordered)
                    .padding(16)
                    .transition(.opacity)
                }
            }
        }
        .navigationDestination(isPresented: $authorizing) { AuthorizeView(runners: store) }
        .task { await waitedLongEnough() }
    }

    /// Hand the shell the terminal a card asked for, if this runner has it.
    ///
    /// The two-phase dance survives the shell unchanged, because what made it
    /// necessary has not changed: a card tapped at COLD LAUNCH delivers its URL
    /// before the first connection has produced a fleet, so an id looked up as
    /// it arrives finds nothing and the tap opens the app onto whatever it
    /// would have opened onto anyway — indistinguishable from a card that
    /// ignored the tap.
    ///
    /// What changed is where the second phase lives. It used to be
    /// `openRequested()`, run again on `hasFleet`, resolving the id against the
    /// fleet and then choosing between a push, a route replacement and a
    /// retarget — three answers, because there were three shapes a screen could
    /// be in. There is nothing to push any more, and every pane on this runner
    /// is one position in one shell: `ShellScreen` reads `pendingTerminal` as a
    /// DERIVED value, the tab id that terminal is once the fleet has one, so
    /// "run it again when a fleet arrives" is simply that derivation
    /// re-evaluating. The shell honors it on appearance as well as on change,
    /// which covers the cold-launch case where the fleet and the shell arrive
    /// in the same turn and there is no change to observe. See
    /// `ShellScreen.requestedTab` and `ShellRootView.honorRequest`.
    ///
    /// The id is held HERE rather than inside the shell because this is the
    /// view that receives the URL: `.onOpenURL` has to be attached above a
    /// screen that only exists once a fleet does, or a card tapped at a cold
    /// launch would be delivered to nothing.
    ///
    /// Dropped, and not kept waiting, once the runner has answered without it:
    /// the pane is gone, or the card was about another runner entirely — the
    /// URL carries an id and no host, so there is nothing here to switch to.
    /// Held open, a pane created much later would be jumped to long after
    /// anybody tapped anything.
    private func dropUnknownTerminal() {
        // Across every runner, and only once every one of them has answered.
        // A card carries a terminal id and no host, so "no runner has it" is
        // the only form the answer can take — and a runner still connecting has
        // not answered, so dropping the link on the strength of the two that
        // have would throw away a card about a pane on the third.
        guard let id = pendingTerminal, !fleet.runners.isEmpty,
            fleet.runners.allSatisfy({ $0.connection.hasFleet })
        else { return }
        let all = fleet.entries.flatMap(\.workspace.terminals)
        guard !all.contains(where: { $0.id == id }) else { return }
        pendingTerminal = nil
    }

    /// Every screen shown BEFORE a connection exists, wrapped in the ways out of
    /// it.
    ///
    /// This is the bug those screens all had. `FleetView` is the app's only
    /// screen — the app opens onto a runner rather than a list of them — so it
    /// has no back button, and the host switcher used to live inside the
    /// workspace list, which only existed once a connection had succeeded. Any
    /// phase short of `.connected` was therefore a room with no doors: "Could
    /// not connect" offered "Try again" and nothing else, and if trying again
    /// could not work — the wrong address, a runner that never
    /// authorized this phone — there was no way to reach another runner, add
    /// one, fix this one, or even see this device's key. Force-quitting was the
    /// only exit.
    ///
    /// So the switcher came out of the connected screen and went under these
    /// three instead, in the same place with the same behavior. The bar is what
    /// makes each of them a screen you can leave. It is under these three and
    /// nothing else now: the connected screen is the shell, which is full bleed
    /// and puts the same menu in its overview's toolbar. See `RunnerMenu`.
    private func escapable<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        content()
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .safeAreaInset(edge: .bottom, spacing: 0) {
                HostSwitcherBar(hosts: store, connection: nil)
            }
            .navigationTitle("Runners")
            .navigationBarTitleDisplayMode(.inline)
    }

    private func waitedLongEnough() async {
        stalled = false
        try? await Task.sleep(for: .seconds(4))
        guard !Task.isCancelled else { return }
        withAnimation { stalled = true }
    }

    /// The app, once this runner has answered: the navigation shell.
    ///
    /// One branch, and it is the last thing between a connection and the shell.
    /// `hasFleet` is what it turns on because the shell opens ON a pane — see
    /// `ShellScreen.seed` — and there is no pane to open on until the runner
    /// has said what it has. It is set by the fleet read itself rather than by
    /// the phase, which flips to `.connected` a whole SSH round trip earlier.
    ///
    /// This used to be the inbox — a list of what on this runner was waiting on
    /// a person — with the shell behind a debug flag beside it. Both are gone:
    /// the shell's overview IS the fleet screen — searchable,
    /// precedence-sorted, needs-you first — and a second screen answering the
    /// same question with rows instead of cards was a second thing to keep
    /// true.
    private var connected: some View {
        ShellScreen(fleet: fleet, hosts: store, pendingTerminal: $pendingTerminal)
    }

    // MARK: - What used to stand in front of a runner

    // Four full-screen phases lived here: `connecting`, `approval`, `failure`
    // and `primaryAction`, one branch per `Connection.Phase`, each owning the
    // whole display.
    //
    // They are `RunnerStatusRow` now, and the move is not a refactor. A screen
    // can only be about one runner: a single failing runner would have blanked
    // the whole app, and a newly added runner needing authorization would have
    // shown its screen to nobody whenever any other runner answered -- which is
    // the ordinary case, and the one flow onboarding cannot afford to lose.
    //
    // Every distinct next move survived the move, including the two Android's
    // row does not have. Which move a failure gets is `RunnerTrouble.nextMove`'s
    // decision, in AgentKit, so the row and anything else that ever draws one
    // cannot come to disagree.

}

/// Which runner you are looking at, and every way of changing that.
///
/// A strip along the bottom rather than a section in a list: the list above it
/// is worktrees on ONE runner, and putting the runner inside it would read as
/// one more thing in the same collection. This says what the collection belongs
/// to.
///
/// Split out of the workspace list because it turned out to be the app's only
/// escape hatch, and it was attached to the one screen you cannot reach when
/// you need an escape hatch — the connected one. It is under the connecting,
/// approval and failure screens now and under nothing else: the connected app
/// is the shell, which is full bleed and has no room for a strip, and the same
/// menu is a toolbar item on the shell's overview instead. See
/// `RunnerMenu`, which is the half both of them share.
struct HostSwitcherBar: View {
    @ObservedObject var hosts: RunnerStore
    /// The connection whose state the chip shows, and which its tap retries.
    /// Also how the settings screen names the daemon it is talking to.
    ///
    /// Optional now that the store owns connections: a runner that has just
    /// been added has no connection for the frame before the store brings one
    /// up, and this bar is under exactly the screens that show before one
    /// exists.
    var connection: Connection?

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "server.rack")
                .font(.caption)
                .foregroundStyle(.secondary)

            RunnerMenu(hosts: hosts, connection: connection)

            Spacer(minLength: 0)

            if let connection { LinkStatusChip(connection: connection) }
        }
        .padding(.horizontal, 16)
        // The 10 points of vertical padding that used to be here are gone, and
        // the height moved into the two controls instead. Both are 44 now, so
        // the bar is 44 rather than the 41 it was — three points for two targets
        // that clear the floor, on the one strip that is under every phase of
        // this screen including the ones you cannot otherwise leave.
        .background(.bar)
    }
}

/// Which runner, and every way of changing that — the menu, without a bar
/// around it.
///
/// Split out of `HostSwitcherBar` when the connected app stopped having a strip
/// to put one on. The shell is full bleed: the only chrome it has is a piece of
/// glass at the bottom that IS the workspace, and a second bar under it would
/// be a second thing competing for the same edge. So the connected app carries
/// this menu as a toolbar item on the overview — the screen that is the fleet,
/// and therefore the screen that should say whose fleet it is — while the three
/// pre-connected phases go on carrying the bar, because a screen with no fleet
/// has no overview to put anything on.
///
/// One type rather than two copies, and that is the whole reason it exists.
/// This is the app's only way to reach another runner, to correct the one it is
/// on, and to reach this device's own key; two menus that had drifted apart
/// would mean the door out of a dead connection and the door out of a live one
/// offering different things.
struct RunnerMenu: View {
    @ObservedObject var hosts: RunnerStore
    /// The runner this menu is standing over, for the settings sheet it opens.
    ///
    /// Optional, and nil is an ordinary answer rather than a gap: the shell
    /// draws this over a MERGED fleet, so before it has come to rest there is
    /// no one runner the menu is about. `SettingsView` has taken an optional
    /// connection since the onboarding screen learned to open it, and for the
    /// same reason — a screen with no runner behind it still has settings.
    var connection: Connection?
    /// Called after picking a different runner, for a caller that has something
    /// to close. Nil everywhere it is part of the screen.
    var onSwitch: (() -> Void)?

    @State private var showAdd = false
    /// The host being edited, rather than a bare flag: a flag plus a separate
    /// `hosts.selected` lookup can present a sheet with nothing in it if the
    /// selection changes between the tap and the presentation.
    @State private var editingRunner: Runner?
    @State private var showSettings = false

    var body: some View {
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
                    .lineLimit(1)
                    .truncationMode(.middle)
                Image(systemName: "chevron.up.chevron.down").font(.caption2)
            }
            // The only way to change runners in the app, and it was about
            // 21 points tall. The words keep their size; the band around
            // them is the guideline's 44, and `contentShape` makes that band
            // live rather than merely occupied — padding on a menu label is
            // layout only otherwise.
            .frame(minHeight: PaneMetrics.target)
            .contentShape(.rect)
        }
        .accessibilityIdentifier("runner-menu")
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

    init(sentence: String, transcript: String? = nil) {
        self.sentence = sentence
        self.transcript = transcript
    }

    /// The same two fields, decided by the shared table.
    ///
    /// `RunnerRefusal` already speaks in exactly this pair — our sentence, the
    /// runner's words, never spliced — and it lives in AgentKit because that is
    /// the only Swift CI actually runs. This is the seam, not a second opinion.
    init(_ trouble: ReviewTrouble) {
        self.init(sentence: trouble.sentence, transcript: trouble.transcript)
    }
}

/// One failure, drawn the way this codebase already draws them: a written
/// sentence, then a `DetailBox` holding the transcript.
///
/// One view rather than a copy in each sheet, so two sheets reporting the same
/// kind of failure cannot come to render it differently — the principle
/// `f9f37eb` and `776d3e0` both turned on. `DetailBox` itself is AgentKit's and
/// the Mac's; see `DaemonUpdateCard`, `RunnersSettings` and `ChangesPane`.
/// Internal rather than private to this file, because `RunnerSettings`'s own
/// typed-name sheet reports the same kind of failure — which is precisely the
/// case the paragraph above says one view exists to prevent.
struct SheetFailureSection: View {
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
///
/// **Recovered verbatim from `f319376`.** It went out of the tree with
/// `FleetList`, whose per-row swipe actions were the only caller, and the
/// ceremony is the reason it came back rather than being rewritten: a typed
/// name is the one thing standing between a thumb and a directory with
/// uncommitted work in it, and a rewrite is a chance to make it one tap
/// lighter by accident. Its caller now is `ShellPaneChromeModifier` — see that type
/// for why the door is on the pane's bar rather than on an overview card.
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
                            case .failed(let message, let word):
                                working = false
                                failure = SheetFailure(
                                    RunnerRefusal.trouble(
                                        forWord: word,
                                        message: message,
                                        otherwise: "Removing this worktree didn’t finish."))
                            }
                        }
                    }
                    .disabled(!matches || working)
                }
            }
        }
    }
}

/// Where a removal has got to, held by whoever OPENS one.
///
/// Two steps and one value, rather than a bool and two optionals. The flow has
/// a second phase — a worktree with uncommitted work needs its name typed —
/// and the version this replaced tracked that with three pieces of state in
/// each caller, which is three chances for a confirmation sheet to be left up
/// over a screen that has moved on. One optional is dismissed by writing `nil`
/// to it, from anywhere, and there is nothing else to forget.
///
/// Owned by the CALLER and not by the modifier below, for the reason
/// `ShellScreen` gives about its own two sheets: the surface a menu item was
/// tapped on can be unmounted before the answer comes back — the overview is
/// mounted from the first point of a lift and gone again when nothing is
/// touching it — so the presenter has to be something that outlives it.
/// **The connection is part of the request**, and that is the multi-runner
/// port's mark on this ceremony. A removal is a call to ONE daemon about a
/// worktree only that daemon has, and the screen that starts it can be looking
/// at a merged fleet: the overview's grid holds cards from every connected
/// runner. Resolving the connection where the flow runs rather than where the
/// menu was tapped would run `workspace.remove_worktree` against whichever
/// runner the shell happened to be resting on, with an id that means something
/// different over there.
enum RemoveWorktreeRequest {
    /// "Remove worktree for X?", with a Remove and a Cancel.
    case confirming(Workspace, on: Connection)
    /// The typed-name ceremony, which is also where a refusal is reported.
    case typing(Workspace, on: Connection)

    var workspace: Workspace {
        switch self {
        case .confirming(let workspace, _), .typing(let workspace, _): return workspace
        }
    }

    var connection: Connection {
        switch self {
        case .confirming(_, let connection), .typing(_, let connection): return connection
        }
    }
}

/// The whole of removing a worktree from this app: ask, try, and fall through
/// to the typed name when the runner says the work is not finished with.
///
/// **One copy, because it is a ceremony and ceremonies drift.** There are two
/// doors into this now — the pane's own bar (`ShellPaneChromeModifier`) and
/// the overview card's context menu — and the Mac has two as well. What must
/// not vary between them is how much confirmation a destructive action gets,
/// so the sequence lives here and the doors only decide when to open it.
///
/// The sequence mirrors macOS's: a plain confirmation first, and the typed
/// name only when the daemon asks for one. `workspace.remove_worktree` is what
/// decides that — a clean worktree goes on an empty `confirm`, a dirty one
/// answers `confirmationRequired` — so the phone never asks for a typed name
/// the runner would not have asked for, and never skips one it would.
///
/// Every non-`.ok` answer routes to the same sheet, refusals included. That is
/// deliberate and it is `ShellPaneChromeModifier`'s note kept: a
/// `confirmationDialog` has nowhere to put a sentence, and two places for one
/// failure to appear is one of them nobody maintains.
struct RemoveWorktreeFlow: ViewModifier {
    @Binding var request: RemoveWorktreeRequest?

    /// True only while the first dialog is the step we are on, so advancing to
    /// the sheet takes the dialog down without ending the flow.
    private var confirming: Bool {
        if case .confirming = request { return true }
        return false
    }

    private var typing: Workspace? {
        if case .typing(let workspace, _) = request { return workspace }
        return nil
    }

    func body(content: Content) -> some View {
        content
            .confirmationDialog(
                "Remove worktree for \(request?.workspace.task ?? "")?",
                isPresented: Binding(
                    get: { confirming },
                    // A dismissal that is not this flow moving on — the
                    // Cancel, or a tap outside — ends it. Guarded on the step,
                    // because SwiftUI also reports `false` at the moment the
                    // sheet below takes over.
                    set: { shown in if !shown, confirming { request = nil } }),
                titleVisibility: .visible,
                // `presenting:` rather than reading the binding back inside
                // the action, because a dialog hands its buttons the value it
                // was BUILT with. Read at tap time instead, the request can
                // already have been cleared by the same tap's dismissal, and
                // the Remove button would quietly do nothing.
                presenting: request?.workspace
            ) { workspace in
                Button("Remove", role: .destructive) {
                    // Read off the request rather than off the modifier, so the
                    // call goes to the runner the worktree is on. See
                    // `RemoveWorktreeRequest`.
                    guard let connection = request?.connection else { return }
                    Task {
                        switch await connection.removeWorktree(workspace, confirm: "") {
                        case .ok:
                            request = nil
                        case .confirmationRequired, .failed:
                            request = .typing(workspace, on: connection)
                        }
                    }
                }
                Button("Cancel", role: .cancel) { request = nil }
            }
            .sheet(
                item: Binding(
                    get: { typing },
                    set: { workspace in if workspace == nil { request = nil } })
            ) { workspace in
                // The connection is captured from the request that BUILT this
                // sheet rather than read at tap time, for the same reason the
                // dialog's `presenting:` exists one modifier up: the answer
                // arrives after the request has been cleared.
                let connection = request?.connection
                RemoveWorktreeConfirmSheet(workspace: workspace) { typed in
                    // A sheet with no connection behind it cannot happen — the
                    // request that opened it carried one — and reports the
                    // runner having gone rather than claiming a removal that
                    // never left the phone.
                    guard let connection else {
                        return .failed("This runner is no longer connected.", word: nil)
                    }
                    return await connection.removeWorktree(workspace, confirm: typed)
                }
            }
    }
}

extension View {
    /// Ask about, and carry out, the removal `request` names.
    func removeWorktreeFlow(_ request: Binding<RemoveWorktreeRequest?>) -> some View {
        modifier(RemoveWorktreeFlow(request: request))
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
                                // tell which — but the runner names WHY, and on
                                // this screen the reasons want opposite moves:
                                // a system folder is never addable and wants a
                                // narrower one, a folder overlapping a root you
                                // already have wants a different folder, and a
                                // duplicate wants nothing at all. A refusal we
                                // cannot read still falls back to the sentence
                                // this sheet has always shown, with the
                                // runner's answer below it rather than in place
                                // of it.
                                working = false
                                failure = SheetFailure(
                                    ClientCore.trouble(
                                        error,
                                        otherwise: "Adding this repository didn’t finish."))
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
