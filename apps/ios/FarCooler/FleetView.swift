import SwiftUI

/// One runner, and what stands in front of it until it answers.
///
/// What is left of what this was. It used to be the app's navigation — a
/// `NavigationStack`, a `[Route]` path, a persisted copy of that path, and the
/// rules for restoring, truncating and deferring to it. All of that is gone:
/// the navigation shell is the app's navigation now, and a shell position is a
/// pair of indices into the fleet this connection is already publishing, so
/// there is nothing to push, nothing to persist and nothing to resolve.
///
/// What remains is the part that was never navigation: OWNING the connection,
/// and standing in front of it until it answers. The four phases, the ways out
/// of the three that have no fleet behind them, and the deep link that arrives
/// before any of it exists.
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
    /// id is remembered instead, and the shell picks it up the moment there is
    /// a fleet to look in. See `dropUnknownTerminal`.
    @State private var pendingTerminal: String?

    /// Whether to offer a way off the spinner yet. See `waitedLongEnough`.
    @State private var stalled = false

    @Environment(\.scenePhase) private var scenePhase

    /// Open when correcting this runner's details, from any phase that has a
    /// reason to doubt them.
    @State private var editing = false

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
        // What still needs a stack is everything before a fleet exists: the
        // failure screen pushes `AuthorizeView`, and all four pre-connection
        // screens are titled. Each of those branches declares its own, which is
        // also what keeps the shell out of one — see `phases`.
        phases
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
            // that owns the connection whose fleet the id has to be looked up
            // in. Routing it from the root would mean a second way to choose a
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
            .onChange(of: connection.pollGeneration) { _, _ in dropUnknownTerminal() }
    }

    /// One branch per connection phase, and the one place a `NavigationStack`
    /// is still declared.
    ///
    /// Split out of `body` rather than written inline, for the compiler's sake:
    /// a `switch` over an associated-value enum inside a long modifier chain is
    /// the shape Swift's type checker gives up on, and it did.
    ///
    /// **The three pre-connected screens are unchanged**, `escapable(_:)` and
    /// all. That wrapper puts `HostSwitcherBar` under each of them, and the bar
    /// is the app's only escape hatch before a runner answers: without it,
    /// "Could not connect" is a room with no doors, because the switcher used
    /// to live inside the connected screen and the connected screen is the one
    /// you cannot reach. The stack around them is what gives them a title and
    /// what `failure`'s "Authorize This Device" pushes into.
    ///
    /// The shell gets no stack, deliberately, and gets one nowhere else either
    /// — see `body`. Its own navigation is a gesture, and the only navigation
    /// bar in it belongs to the overview, which declares a stack of its own.
    @ViewBuilder
    private var phases: some View {
        switch connection.phase {
        case .connecting:
            NavigationStack { escapable { connecting } }

        case .needsApproval(let fingerprint):
            NavigationStack { escapable { approval(fingerprint) } }

        case .failed(let message):
            NavigationStack { escapable { failure(message) } }

        // Reconnecting renders exactly what connected renders. The fleet on
        // screen is the last one this runner sent, and it is a better answer
        // than a spinner while the link comes back — see
        // `Connection.Phase.reconnecting`. The status chip in the overview's
        // toolbar is where the difference shows.
        case .connected, .reconnecting:
            if hasFleet {
                connected
            } else {
                NavigationStack { waitingForFleet }
            }
        }
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
        guard let id = pendingTerminal, connection.phase == .connected, hasFleet else { return }
        let all = connection.fleet.workspaces.flatMap(\.terminals)
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
        ShellScreen(connection: connection, hosts: store, pendingTerminal: $pendingTerminal)
    }

    /// The wait for the runner's first answer.
    ///
    /// Not an empty shell. Before the first fleet there is no position to open
    /// on, and a shell with no workspace is a bar naming something that does
    /// not exist. The runner's own name is the title because there is nothing
    /// else yet to say what is being waited on.
    private var waitingForFleet: some View {
        ProgressView()
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .navigationTitle(host.label)
            .navigationBarTitleDisplayMode(.inline)
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
        // A card tapped at cold launch delivers its URL before there is a
        // fleet to look the id up in. Nothing has to be re-run for it now:
        // `ShellScreen.requestedTab` is derived from this connection's own
        // fleet, so a fleet arriving IS the second look. What is left is
        // deciding that an id this runner does not have is never coming — see
        // `dropUnknownTerminal`.
        dropUnknownTerminal()
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
            // The only push left in the app, and it is a leaf with nothing
            // under it: reachable from the failure screen alone, which is a
            // phase with no fleet, so the shell that has replaced every other
            // destination is not on screen to be pushed over. That is why
            // `phases` gives these three branches a `NavigationStack` of their
            // own and gives the shell none.
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
    /// Also how the settings screen names the daemon it is talking to. Absent
    /// before a connection exists, which is most of the time this bar matters.
    @ObservedObject var connection: Connection

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "server.rack")
                .font(.caption)
                .foregroundStyle(.secondary)

            RunnerMenu(hosts: hosts, connection: connection)

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
    @ObservedObject var connection: Connection
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
