import AgentKit
import SwiftUI

/// Whether the daemon answering for a runner is the program this app was built
/// to drive.
///
/// Far Cooler is two programs — the app in your hand and the `farcoolerd` doing
/// the work — and only the app updates itself. A Mac that updates on Tuesday
/// goes on talking to whatever daemon each runner happened to be running on
/// Monday, and nothing anywhere said so: `farcooler status` prints MISMATCH and
/// nothing in the app was obliged to read it. The failure that produced this
/// type was a daemon fourteen commits behind the app driving it, and a feature
/// that had just shipped doing nothing at all — no error, no refusal, no dot.
/// Two components built from different source speak the protocol perfectly and
/// still behave like two different programs.
///
/// **This is never acted on automatically, and that is the whole design.**
/// Updating a runner restarts its daemon, and a daemon restart is not free:
///
/// - Terminals survive it. They are tmux panes, and a pane's replay is rebuilt
///   from tmux's own scrollback at attach — `crates/daemon/src/runtime.rs`
///   captures `capture_scrollback`, `capture_screen` and the cursor and writes
///   them in order, so the bytes come back from tmux rather than from anything
///   the daemon was holding.
/// - Agent conversations do not. `crates/daemon/src/agent_supervisor.rs` keeps
///   `recent` — the transcript — in a `HashMap` in memory, bounded by
///   `TRANSCRIPT_LIMIT`, and its doc is explicit that the daemon "owns this
///   transcript outright — it is not a cache in front of the shim's ring, and
///   nothing asks the shim for anything older." A daemon that goes away takes
///   every agent chat history on that runner with it.
/// - Anything typed and not yet sent dies with the shim as well, which is what
///   `ToggleRefusal::TurnInFlight` in the same file already refuses a pane-mode
///   toggle over: "the shim holds unsent prompts in memory
///   (`RunningSession::queue`) and dies with the pane."
///
/// So an app that quietly brought every runner up to date would be an app that
/// silently deleted your agents' memory to fix a version number. What is here
/// instead is a control that says the price before it is paid.
///
/// One thing this does NOT cover, said plainly rather than left for someone to
/// discover: `LocalDaemon.ensure()` already replaces THIS Mac's daemon at
/// launch, unasked, and has since before any of the above was written down.
/// Nothing here changes that — see `LocalDaemon`, where the same cost is now
/// recorded and where the reason it happens at launch is argued out. What this
/// type adds for the local runner is the case that call cannot fix on its own:
/// a replacement that did not take, leaving an app driving a daemon it was not
/// built with, which until now was visible nowhere.
enum DaemonSkew: Equatable {
    /// Not a version question right now: the runner is unreachable, has no
    /// Far Cooler on it, or has not answered its first read yet.
    ///
    /// Deliberately one case rather than three. A runner that cannot be
    /// reached is not a runner with a stale daemon — nobody knows what it is
    /// running — and one that has never had Far Cooler installed has no daemon
    /// to be behind. Both already have their own dot and their own sentence in
    /// `HostState`; offering to update either would be the app inventing a
    /// diagnosis it does not have.
    case unavailable

    /// The runner answers, but which build is answering could not be read.
    ///
    /// Real and reachable: `farcooler status` asks the daemon for host facts,
    /// and a daemon old enough to predate that method answers `NOT_FOUND` —
    /// so the runner that is most likely to be stale is exactly the one that
    /// can fail to say so. Kept apart from `.behind` because "probably old"
    /// is not "old", and from `.current` because silence is not agreement.
    ///
    /// Draws nothing anywhere (see `offersUpdate`). The app has no version to
    /// show and no update it can honestly offer, and a dot meaning "we could
    /// not tell" is one nobody can act on. What a person can still do is open
    /// Settings ▸ Runners, where `runner probe` reports the version of the
    /// `farcoolerd` on that runner's DISK — a different question from which
    /// build is running, and the nearest thing to an answer when the running
    /// one will not give its own.
    case unknown

    /// Built from the same source as this app.
    case current

    /// The daemon answered, and it is not this app's build. `daemon` is its
    /// version spelled the way the app spells its own — see
    /// `DaemonBuild.readable`, which turns `0.1.0+a1b2c3` into
    /// `0.1.0 (a1b2c3)` so two versions side by side do not read as two
    /// different idioms.
    case behind(daemon: String)

    /// So old that the handshake itself was refused.
    ///
    /// `crates/cli/src/remote.rs`'s `explain` produces "update the older side"
    /// for `ClientError::VersionMismatch`, and `DaemonClient` already gives
    /// that failure the five-minute retry cadence rather than the exponential
    /// one, precisely because no amount of retrying updates anything. It is
    /// the one flavor of `.unreachable` that IS a version, and the one where
    /// retrying is the only thing that cannot help — so it gets the update
    /// affordance even though the runner is not usable.
    case tooOldToTalk

    /// Whether the app has grounds to offer an update.
    ///
    /// The two cases where the app knows the daemon is wrong. `.unknown` is
    /// excluded on purpose: a dot that means "we could not tell" is a dot
    /// nobody can act on, and a column of those is how people learn to stop
    /// reading dots.
    var offersUpdate: Bool {
        switch self {
        case .behind, .tooOldToTalk: return true
        case .unavailable, .unknown, .current: return false
        }
    }

    /// What the daemon said it was, when it said anything.
    var daemonVersion: String? {
        if case .behind(let version) = self { return version }
        return nil
    }
}

/// One runner the app is offering to update, and how to do it.
///
/// A value carrying its own action, the way `SidebarMenuItem` does, so the
/// views below need no store, no client and no knowledge of whether this
/// runner is updated over ssh or out of the app bundle. `ContentView` is the
/// one place that knows which client a host belongs to, and it is the one
/// place that builds these.
struct DaemonUpdateTarget: Identifiable {
    /// The ssh target. Empty is this Mac — a real value here, not "nothing",
    /// the same convention `workspace.host` and `FleetStore` use.
    let host: String
    let skew: DaemonSkew
    let update: () async -> DaemonUpdateOutcome

    var id: String { host }
    var name: String { host.isEmpty ? "This Mac" : host }
}

/// How an update ended.
///
/// The failure carries the installer's own transcript rather than a sentence
/// written here. The moment it matters is an install that stopped halfway, and
/// the reason exists in exactly one place — `runner install` verifies a SHA-256
/// on the far side, refuses a host with no tmux, and names which of those it
/// hit. See `RunnersSettings`, which shows the same text in the same
/// `DetailBox` for the same reason.
enum DaemonUpdateOutcome: Equatable {
    /// The command reported no problem. Deliberately not "the runner is now
    /// current": the app's own confirmation is the next read of which build is
    /// answering, which lands moments later when the client reconnects — and
    /// if it disagrees, the dot comes back rather than the app insisting.
    case updated
    case failed(String)
}

/// What a person is told before Far Cooler is restarted on a runner, in one
/// place.
///
/// One copy, two surfaces — the sidebar's update card and the Reinstall
/// confirmation in Settings ▸ Runners — because a cost worded twice is a cost
/// somebody will eventually word wrongly, and the wrong wording here is one
/// that lets an agent's memory go without saying so.
///
/// Two short lines, where there were two paragraphs. The long form was true
/// and is not gone: tmux replaying a pane out of its own scrollback,
/// `agent_supervisor.rs` holding `recent` in a `HashMap` in memory. That is
/// WHY these two lines can be trusted, and it lives in `DaemonSkew`'s doc
/// above, where every claim made here is checked against the code that makes
/// it. It does not belong on screen: somebody deciding whether to spend an
/// afternoon of agent memory needs the price, not the receipt.
///
/// Neither line names the ACTION — updating, reinstalling, restarting — only
/// what happens to what is running, so both are true under a button that says
/// "Update Runner" and under one that says "Reinstall".
enum DaemonRestartCost {
    /// Said first now, because it is the half that costs something and the
    /// only fact anywhere on the card that changes the answer. It used to run
    /// second, under a paragraph of reassurance, in bold — which was the app
    /// noticing it had put the deciding sentence in the wrong place and trying
    /// to fix that with weight instead of with order.
    static let agents =
        "Agent conversations there are lost, along with anything typed and "
        + "not sent."

    /// Said second, and kept, because the question people actually arrive with
    /// is whether they are about to lose their terminals.
    static let terminals = "Terminals survive."

    /// Both lines in one paragraph, for a `confirmationDialog`, which takes a
    /// message rather than a layout. The caller names the runner in the
    /// sentence before this one, which is what "there" points at.
    static var sentence: String { "\(agents) \(terminals)" }
}

// MARK: - The dot

/// A runner whose daemon is not this app's build, said in the sidebar's own
/// vocabulary.
///
/// Shape rather than a new color. `StatusGlyph` sets the rule this follows —
/// "four colors in an application is already generous", and a hollow ring means
/// something is missing where a filled dot means something is happening — and
/// `HostDot` has already spent red on a runner that has given up and amber on
/// one that is reconnecting. A fifth hue for "answers everything, wrong build"
/// would be a color nobody could learn; an amber RING beside amber DOTS is a
/// difference you can see without being told, and it is honest about what it
/// means: the runner is fine, what it is running is not.
///
/// `.tooOldToTalk` is the exception and keeps a filled red dot, because that
/// runner really is unreachable — the color says how bad it is, and the card
/// behind it says what to do. That is also the one place this replaces an
/// existing control rather than filling an empty column: `HostDot` draws a red
/// retry dot there today, and retrying a protocol mismatch is the one thing
/// that provably cannot work.
struct DaemonSkewDot: View {
    let target: DaemonUpdateTarget

    @State private var showingCard = false

    var body: some View {
        Button { showingCard = true } label: {
            // Same 5pt as every other dot in a project header. A control that
            // is a hair larger than its neighbors reads as a mistake rather
            // than as emphasis.
            Group {
                if target.skew == .tooOldToTalk {
                    Circle().fill(Color.red)
                } else {
                    Circle().strokeBorder(Color.orange, lineWidth: 1.5)
                }
            }
            .frame(width: 6, height: 6)
        }
        .buttonStyle(.plain)
        .help(help)
        .popover(isPresented: $showingCard, arrowEdge: .bottom) {
            DaemonUpdateCard(targets: [target]) { showingCard = false }
        }
    }

    /// Ends in what a click does, like `HostDot`'s own help does — and says
    /// "what updating costs" rather than "update", because a tooltip that
    /// promises an action the click does not take is a tooltip that teaches
    /// people to be afraid of the control.
    private var help: String {
        switch target.skew {
        case .tooOldToTalk:
            return "This runner is too old for the app to reach — "
                + "click to see what updating costs"
        default:
            return "This runner is behind the app — "
                + "click to see what updating costs"
        }
    }
}

/// The same offer, in the sidebar's status bar, with words on it.
///
/// The dot above is enough for a runner you are already looking at, and it is
/// not enough on its own: a project header only exists where a project does, so
/// a Mac whose own daemon is stale and whose fleet is empty — a fresh update,
/// nothing checked out yet — would have nowhere to draw one. This row is always
/// there.
///
/// Unlike the trouble dots beside it, this is NOT gated on having more than one
/// runner. Naming a runner on a fleet of one is noise, which is why `showHosts`
/// exists; being the only place an action lives is not, and a fleet of one Mac
/// whose bundled daemon did not come along with the app update is exactly the
/// case this whole file was written for.
///
/// The label ends in an ellipsis because it opens something rather than doing
/// something, which is the one piece of Apple's button grammar that says "this
/// will ask first" before you click it.
struct DaemonUpdateBar: View {
    let targets: [DaemonUpdateTarget]

    @State private var showingCard = false

    var body: some View {
        Button { showingCard = true } label: {
            HStack(spacing: 5) {
                // Always the ring, never `DaemonSkewDot`'s red: this one dot
                // can stand for several runners at once, so it says "there is
                // version news down here" and leaves how bad it is per runner
                // to the card, which names each of them. A summary that took
                // the worst runner's color would make one unreachable runner
                // repaint a row that is mostly about runners doing fine.
                Circle()
                    .strokeBorder(Color.orange, lineWidth: 1.5)
                    .frame(width: 6, height: 6)
                Text(label)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .buttonStyle(.plain)
        .help(
            targets.count == 1
                ? "\(targets[0].name) isn’t running this app’s build of Far Cooler"
                : "\(targets.count) runners aren’t running this app’s build of Far Cooler"
        )
        .popover(isPresented: $showingCard, arrowEdge: .top) {
            DaemonUpdateCard(targets: targets) { showingCard = false }
        }
    }

    /// "Runner", not "Daemon": the thing a person added, named and can point
    /// at is a runner, and the `farcoolerd` on it is an implementation detail
    /// of that. See `DaemonUpdateCard.title(for:)`, which sets the rule and
    /// names the one place on these surfaces where the older word is still the
    /// honest one.
    private var label: String {
        targets.count == 1 ? "Update Runner…" : "Update \(targets.count) Runners…"
    }
}

// MARK: - The card

/// What updating costs, and the button that spends it.
///
/// Everything about this view exists so that nothing is replaced by accident:
///
/// - The cost is above the button, not after it and not in a tooltip. It is
///   the reason this is a card and not a menu item. It is also two short lines
///   rather than the three paragraphs it was: what is lost, then what isn't,
///   with the expensive one in primary text. `DaemonRestartCost` holds both,
///   and `DaemonSkew`'s doc holds the mechanism underneath them, which is
///   deliberately not on screen — nobody deciding this needs to be told that a
///   pane's replay is rebuilt out of tmux's scrollback.
/// - The versions are behind a disclosure, closed. They were the second thing
///   on the card and they were a comparison nobody could make: an app and a
///   daemon a few commits apart both say `0.1.0`, and the eye that goes
///   looking for a difference finds two identical numbers and a git SHA. The
///   number is still exactly what somebody pastes into a bug report, so it is
///   kept, spelled the same way, one click away.
/// - Nothing here has `.keyboardShortcut(.defaultAction)`. Escape dismisses;
///   Return does nothing at all. A popover that discards every agent
///   conversation on a runner when a stray Return arrives is not a popover
///   this app is allowed to ship.
/// - The failure keeps the card open with the installer's own words in it. An
///   install that stopped halfway is precisely when its transcript matters, and
///   a card that closed on failure would be the app deciding you did not need
///   to know why.
struct DaemonUpdateCard: View {
    let targets: [DaemonUpdateTarget]
    let onDone: () -> Void

    /// Per host, because a card built from the status bar can hold several
    /// runners and each of them succeeds or fails on its own.
    @State private var busy: Set<String> = []
    @State private var failures: [String: String] = [:]
    @State private var updated: Set<String> = []

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            ForEach(Array(targets.enumerated()), id: \.element.id) { index, target in
                if index > 0 { Divider() }
                runner(target)
            }
        }
        .padding(16)
        // Wide enough that the cost lands in two lines rather than a column of
        // five-word ones, and narrow enough to sit under a 6pt dot in a 320pt
        // sidebar without covering the list it came from.
        .frame(width: 340)
    }

    @ViewBuilder
    private func runner(_ target: DaemonUpdateTarget) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title(for: target)).font(.headline)
                Text(target.name)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            VStack(alignment: .leading, spacing: 8) {
                Text(lead(for: target))
                // The one line someone has to read, so it is the one line that
                // is not secondary gray — and it now comes BEFORE the
                // reassurance rather than after it. Emphasis was doing a job
                // that order should have been doing.
                Text(DaemonRestartCost.agents).foregroundStyle(.primary)
                Text(DaemonRestartCost.terminals)
            }
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            // Under the sentence it is evidence for, not between the title and
            // the sentence. Title, what happens, what it costs, then the small
            // print for anyone who wants it — and nothing between the two
            // lines that decide this and the buttons that act on them.
            versions(for: target)

            if let words = failures[target.host] {
                Text("The update didn’t finish.")
                    .font(.callout.weight(.medium))
                    .foregroundStyle(.orange)
                DetailBox(text: words)
            }

            actions(for: target)
        }
    }

    /// Sentence case, like every alert title on this platform. Title case is
    /// for the buttons.
    ///
    /// "Runner", not "daemon", and the rule the whole file now follows: the
    /// noun a person can point at is a runner — one `farcoolerd` under one
    /// Unix user, the same thing Settings ▸ Runners lists and the same thing
    /// this card names on its second line — while "daemon" is the name of a
    /// process they never see and did not install by that name. The word is
    /// not banned, it is spent where it is the only true one: the `Daemon`
    /// version row inside the disclosure, where what is being named really is
    /// the build that answered rather than the runner it answered for. Every
    /// other surface here says runner, and the engineering comments go on
    /// saying daemon, because they are talking about the process.
    private func title(for target: DaemonUpdateTarget) -> String {
        switch target.skew {
        case .tooOldToTalk: return "This runner is too old to reach"
        default: return "This runner is behind the app"
        }
    }

    /// Both stamps, in one idiom, behind a disclosure that starts closed.
    ///
    /// The block was full height under the title, and it invited a comparison
    /// that could not be made: both rows read `0.1.0`, because the app and the
    /// daemon are the same release built at different commits, and all a
    /// reader could do with it was notice that the parentheses differed. What
    /// the numbers are genuinely for — pasting into a bug report, and being
    /// able to check that "behind" is a fact rather than a claim — survives a
    /// click perfectly well.
    ///
    /// `AppVersion.display` says `0.2.0 (beta 3)` and a daemon says
    /// `0.1.0+a1b2c3`; `DaemonBuild.readable` already exists to close exactly
    /// that gap, and `DaemonSkew.behind` carries the result of it.
    ///
    /// Nothing at all for `.tooOldToTalk`, honestly: a daemon that refused the
    /// handshake never told us what it was, and a disclosure holding one row —
    /// this app's own version, which About already shows — is a control that
    /// opens onto nothing.
    @ViewBuilder
    private func versions(for target: DaemonUpdateTarget) -> some View {
        if let daemon = target.skew.daemonVersion {
            DisclosureGroup {
                VStack(alignment: .leading, spacing: 3) {
                    version("App", AppVersion.display)
                    // "Daemon", and this is the one row that keeps the word.
                    // A runner does not have a version; the `farcoolerd`
                    // answering for it does, and this row is the one thing on
                    // the card somebody copies into a bug report — where being
                    // exact about which of the two programs is meant is the
                    // whole point of writing it down.
                    version("Daemon", daemon)
                }
                .padding(.top, 4)
            } label: {
                // Sentence case: this is a label on a container, not a button.
                Text("Version details")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func version(_ label: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(label)
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(width: 52, alignment: .leading)
            Text(value)
                .font(.caption.monospaced())
                .textSelection(.enabled)
        }
    }

    /// One line, and it is the ACTION — not how the action works.
    ///
    /// What it replaced spelled out the mechanism twice over: a copy onto the
    /// far side over ssh, or a swap out of the app bundle. That difference is
    /// real and it is worth knowing when an install fails halfway — which is
    /// why `runner install` names it, and why the failure below shows the
    /// installer's own transcript. It is not worth knowing while deciding,
    /// because it changes nothing about the price, so remote and local now say
    /// the same shape of sentence and differ only in what they call the place.
    ///
    /// `.tooOldToTalk` gets its own line, because a runner the app cannot
    /// speak to at all is a different situation from one that works and is
    /// behind, and the card would be describing the wrong problem if it opened
    /// with the price of a fix to a problem it had not named. It also says
    /// retrying is pointless, because the dot this card hangs from is the one
    /// that used to offer a retry.
    private func lead(for target: DaemonUpdateTarget) -> String {
        if target.skew == .tooOldToTalk {
            return "Far Cooler can’t reach \(target.name) until it’s updated. "
                + "Retrying won’t help."
        }
        // "this Mac" mid-sentence, where the line above it says "This Mac":
        // the same runner, spelled for the grammar it is sitting in.
        return "Updating restarts Far Cooler on "
            + (target.host.isEmpty ? "this Mac" : target.name) + "."
    }

    @ViewBuilder
    private func actions(for target: DaemonUpdateTarget) -> some View {
        HStack {
            Spacer()
            if busy.contains(target.host) {
                ProgressView().controlSize(.small)
                Text("Updating…")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            } else if updated.contains(target.host) {
                // Said rather than left to the dot disappearing. The dot going
                // quiet is the confirmation, and it happens a beat later — when
                // the client reconnects and reads the new build — so without
                // this the moment right after a click looks like nothing
                // happened.
                Label("Updated", systemImage: "checkmark.circle.fill")
                    .font(.callout)
                    .foregroundStyle(.green)
            } else {
                Button("Not Now") { onDone() }
                    .keyboardShortcut(.cancelAction)
                Button(failures[target.host] == nil ? "Update Runner" : "Try Again") {
                    Task { await run(target) }
                }
                .buttonStyle(.borderedProminent)
            }
        }
    }

    private func run(_ target: DaemonUpdateTarget) async {
        busy.insert(target.host)
        failures[target.host] = nil
        defer { busy.remove(target.host) }

        switch await target.update() {
        case .updated:
            updated.insert(target.host)
            // Closed at once when this card was about one runner, because the
            // control this popover hangs from is about to stop existing: the
            // client reconnects to the daemon it just replaced, the runner
            // stops being stale, and the dot goes away — taking the popover
            // with it. A checkmark shown into that gap would be a confirmation
            // the view cannot promise to still be on screen to give. The dot
            // disappearing IS the confirmation.
            //
            // With several runners in one card the anchor survives, because
            // the others are still stale — so the row that succeeded says so
            // and stays put beside the ones that have not been done yet.
            if targets.count == 1 { onDone() }
        case .failed(let words):
            failures[target.host] = words
        }
    }
}
