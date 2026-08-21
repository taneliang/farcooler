import Foundation

/// What a glance surface needs in order to offer an agent's own answers, and
/// what came of the last answer somebody tapped.
///
/// **Why this file exists at all.** A permission's id and its option names live
/// in exactly one place: the agent's event stream, reachable only through
/// `terminal.agent_subscribe` on a live SSH session. `FleetSnapshot` carries a
/// headline and no request id — `WatchLinkHost.pendingPermission` says so, and
/// it replays the stream for precisely that reason — and the push carries no
/// more: `crates/daemon/src/push.rs`'s `Notification` is `title`, `subtitle`,
/// `status`, `terminal`, `machine` and a timestamp, composed in
/// `crates/daemon/src/watch.rs` from a tmux SCREEN. What that sampler calls
/// `blocked_question` is a line scraped off the screen with no id and no
/// options attached to it. So a widget extension, which has no connection to
/// any runner, cannot know an agent's answers unless something that DID have a
/// connection writes them down.
///
/// This is that writing-down, and the App Group is where it goes. That choice
/// is not a preference between two equal sources — the push side has nothing
/// to send. Two facts settle it:
///
///   - The daemon derives `blocked` by reading a pane's screen, and most panes
///     it reports are a bare `claude` or `codex` under tmux with no ACP session
///     behind them. For those the structured permission does not exist ANYWHERE
///     in the system, not even on the runner, and `terminal.agent_answer` has
///     nothing to answer. A push field would be empty in the common case and a
///     surface reading it could not tell "no options" from "no permission".
///   - Where the structured permission does exist — a chat-hosted pane, whose
///     events `AgentSupervisor` retains — it is already reachable by the one
///     call the phone makes to answer, so putting a copy on the push would be a
///     second sender for a fact the answering path has to re-read anyway.
///
/// **The cost of that choice, stated plainly.** Only the app can write here,
/// and only about an agent it has actually looked at. A permission that arose
/// while the app was not running is not in this file, so the card shows its
/// existing tap target and no buttons — which is the honest rendering of "this
/// phone has never seen what that agent offered", and better than a pair of
/// buttons whose words nobody on this device can vouch for.
///
/// **Nothing here is a claim that a permission is still open.** A record is
/// what was true when it was written. Every consumer treats it as a candidate
/// and nothing more: the card offers buttons only while the PUSH — which is as
/// fresh as the last thing that happened — says the leader is blocked, and the
/// delivery path re-reads the agent's stream and refuses to send when the id it
/// was handed is no longer the one pending. See
/// `WatchLinkHost.answerFromGlance`, which is where that refusal lives.
///
/// Written by the app on the main actor and read by the widget extension. One
/// writer, several readers, and `SnapshotStore`'s atomic replace underneath, so
/// a reader sees the previous file or the next one and never half of one.
public struct GlancePermissions: Codable, Sendable, Equatable {
    /// What each agent was last seen waiting on.
    public var pending: [GlancePermission]

    /// What this phone last sent about each agent, and how it went.
    public var answers: [GlanceAnswer]

    public init(pending: [GlancePermission] = [], answers: [GlanceAnswer] = []) {
        self.pending = pending
        self.answers = answers
    }

    public static let empty = GlancePermissions()

    /// How many agents this file remembers, evicting the least recently seen.
    ///
    /// The same bound and the same reasoning as `WatchLinkHost.prune`: the
    /// number of agents somebody actually answers from a glance is small, the
    /// file is read on a render path, and an unbounded list would grow for the
    /// life of an install with entries nothing will ever look at again.
    public static let limit = 8

    public func permission(for terminal: String) -> GlancePermission? {
        pending.first { $0.terminal == terminal }
    }

    public func answer(for terminal: String) -> GlanceAnswer? {
        answers.first { $0.terminal == terminal }
    }

    /// Record what an agent is waiting on, or record that it is waiting on
    /// nothing.
    ///
    /// `nil` REMOVES rather than leaves the previous record standing, and that
    /// is the whole point of passing it: a caller that has just read an agent's
    /// stream and found no permission has established something, and leaving a
    /// superseded record in place would let a card go on offering an answer the
    /// agent has stopped asking for.
    ///
    /// The answer for that terminal is dropped alongside any CHANGE of
    /// permission, because a `GlanceAnswer` is about one request id and says
    /// nothing about the next one. It is kept when the same permission is
    /// merely re-observed, so a failure stays on screen while the card that
    /// reports it is still about the request that failed.
    public func recording(
        _ observed: GlancePermission?, for terminal: String
    ) -> GlancePermissions {
        var next = self
        let previous = permission(for: terminal)
        next.pending.removeAll { $0.terminal == terminal }
        if let observed {
            next.pending.insert(observed, at: 0)
            next.pending = Array(next.pending.prefix(Self.limit))
        }
        if previous?.request != observed?.request {
            next.answers.removeAll { $0.terminal == terminal }
        }
        return next
    }

    /// Forget what an agent was waiting on, keeping whatever this phone last
    /// said about it.
    ///
    /// Split from `recording` because the two have opposite obligations. This
    /// runs after a successful send, where the answer is the only thing left
    /// worth showing; `recording` runs after an observation, where a changed
    /// permission makes the old answer misleading.
    public func clearingPermission(for terminal: String) -> GlancePermissions {
        var next = self
        next.pending.removeAll { $0.terminal == terminal }
        return next
    }

    /// Take ownership of one answer, or refuse.
    ///
    /// Nil means **this permission already has an answer from this phone**, and
    /// a nil return is the only thing standing between a second tap and a
    /// second write to a live agent. ACP's `session/prompt` is sent with
    /// `request_no_wait` and nothing acknowledges a permission answer, so a
    /// duplicate is not detectable downstream: `terminal.agent_answer` in
    /// `crates/daemon/src/rpc.rs` posts a `DaemonMessage::Answer` and returns
    /// the terminal's row without waiting, and `AgentEvent::Resolved` — the one
    /// event that could retire a permission — is declared in
    /// `crates/agent-core/src/event.rs` and emitted by nothing. Nobody
    /// downstream will catch this for us, so it is caught here.
    ///
    /// A claim that ended in `nothingSent` is NOT an answer and does not
    /// refuse: see `GlanceAnswer.Outcome`, where the whole point of that case
    /// is that trying again costs nothing. `settling` releases those.
    public func claiming(
        terminal: String, request: String, option: String, optionName: String, at: Date
    ) -> GlancePermissions? {
        if let standing = answer(for: terminal), standing.request == request,
            standing.refusesAnotherTap
        {
            return nil
        }
        var next = self
        next.answers.removeAll { $0.terminal == terminal }
        next.answers.insert(
            GlanceAnswer(
                terminal: terminal, request: request, option: option, optionName: optionName,
                outcome: .inFlight, message: "", at: at),
            at: 0)
        next.answers = Array(next.answers.prefix(Self.limit))
        return next
    }

    /// Say how the claimed answer ended.
    ///
    /// The record is KEPT whatever the outcome, because the sentence has to
    /// reach somebody: "nothing was sent, so it's safe to try again" is the
    /// most useful thing this surface can say and it would be thrown away by a
    /// delete. What the outcome decides is whether the claim still refuses the
    /// next tap — `GlanceAnswer.refusesAnotherTap`, which is false for
    /// `nothingSent` alone, so that one case hands the buttons back while every
    /// other keeps them off.
    public func settling(
        terminal: String, request: String, outcome: GlanceAnswer.Outcome, message: String, at: Date
    ) -> GlancePermissions {
        var next = self
        guard let index = next.answers.firstIndex(where: {
            $0.terminal == terminal && $0.request == request
        }) else { return self }
        next.answers[index].outcome = outcome
        next.answers[index].message = message
        next.answers[index].at = at
        return next
    }
}

/// One answer an agent offered, as the agent worded it.
///
/// The third copy of `PermissionOption`'s three fields in this codebase, and
/// the reason is the reason `WatchPermissionOption` is the second: a copy
/// exists wherever these fields cross a boundary between two binaries that
/// cannot share a type. `WatchLink.swift` states the rule for the phone-to-
/// watch seam; this is the app-to-widget-extension seam, and the extension
/// compiles neither `AgentEvent.swift` — a 479-line file whose dependencies it
/// has no use for — nor `WatchLink.swift`, which would claim it talks to a
/// watch it has never heard of. The conversion from `PermissionOption` is a
/// field-for-field copy so it has nothing to decide.
public struct GlancePermissionOption: Codable, Sendable, Equatable, Identifiable {
    public let id: String
    /// What the button says. Never abbreviated, never truncated — see
    /// `GlancePermission.fit`.
    public let name: String
    /// The agent's own category: `allow_once`, `reject_once`, and whatever else
    /// it invents. Carried rather than interpreted, with one exception stated
    /// where it happens.
    public let kind: String

    public init(id: String, name: String, kind: String) {
        self.id = id
        self.name = name
        self.kind = kind
    }
}

/// What one agent was last seen waiting on.
public struct GlancePermission: Codable, Sendable, Equatable {
    public let terminal: String
    /// The permission's own id, which an answer sends straight back.
    public let request: String
    /// The answers the agent offered, in the order it offered them.
    public let options: [GlancePermissionOption]
    /// When this phone read it off the agent's stream.
    ///
    /// Not a freshness gate on its own, and deliberately not used as one: a
    /// permission left up over lunch is still live, and a surface that hid the
    /// buttons after ten minutes would hide them in exactly the case somebody
    /// most wants them. What this is for is telling a record apart from a much
    /// older one when a card has been sitting on a lock screen for a long time.
    public let observedAt: Date

    public init(
        terminal: String, request: String, options: [GlancePermissionOption], observedAt: Date
    ) {
        self.terminal = terminal
        self.request = request
        self.options = options
        self.observedAt = observedAt
    }
}

/// What this phone last sent about one agent, and how it went.
public struct GlanceAnswer: Codable, Sendable, Equatable {
    /// The three things a surface has to be able to tell apart, plus the state
    /// a send is in while it runs.
    ///
    /// These are `WatchLinkClient`'s distinctions, kept rather than reinvented,
    /// because the failure they exist to prevent is the same one on any surface
    /// that can only report and cannot take back: a **reject** that landed,
    /// reported as unsent, followed by a tap on the option that allows.
    public enum Outcome: String, Codable, Sendable {
        /// Claimed and running. The buttons are off for the duration, which is
        /// what `PermissionView` does for exactly this reason.
        case inFlight
        /// The phone handed it to the runner. Not "the agent acted on it" —
        /// see `WatchReply.sent`, which is careful about the same distinction.
        case sent
        /// We never heard back, so it MAY have gone through. The buttons stay
        /// off and the sentence says to check before trying again.
        case unsure
        /// It failed before anything was handed over, so trying again costs
        /// nothing. The buttons come back.
        case nothingSent
    }

    public let terminal: String
    public let request: String
    public let option: String
    /// The option's own name, so the card can say which of several buttons
    /// landed. "Answered" alone does not say that, and on a lock screen the tap
    /// and the confirmation may be minutes apart.
    public let optionName: String
    public var outcome: Outcome
    /// The sentence to put on the card. Never a raw error from underneath.
    public var message: String
    public var at: Date

    public init(
        terminal: String, request: String, option: String, optionName: String,
        outcome: Outcome, message: String, at: Date
    ) {
        self.terminal = terminal
        self.request = request
        self.option = option
        self.optionName = optionName
        self.outcome = outcome
        self.message = message
        self.at = at
    }

    /// Whether this record may still be shown, and whether it may still refuse
    /// a tap.
    ///
    /// A bound is needed because nothing removes these on a schedule: a card
    /// that sat on a lock screen overnight would otherwise report an answer
    /// from yesterday about a permission long gone. Fifteen minutes is longer
    /// than any of the budgets in `WatchLinkHost` by a wide margin — the
    /// slowest is a ten-second replay — so it cannot expire a send that is
    /// still running, and it is far shorter than the hour-long stale date the
    /// relay gives a card.
    public static let freshFor: TimeInterval = 15 * 60

    public func isFresh(at now: Date) -> Bool {
        // Absolute, so a clock that moved backwards expires a record rather
        // than making it immortal.
        abs(now.timeIntervalSince(at)) < Self.freshFor
    }

    /// Whether the buttons stay off. Everything except the case that
    /// established nothing was written.
    public var refusesAnotherTap: Bool { outcome != .nothingSent }
}

/// Which options a surface can show without shortening any of them.
public struct GlanceOptionFit: Sendable, Equatable {
    /// What to draw, in the agent's own order.
    public let shown: [GlancePermissionOption]
    /// How many of the agent's answers are not on screen, so the surface can
    /// say so rather than let somebody choose from a list they believe is
    /// complete.
    public let hidden: Int

    public static let none = GlanceOptionFit(shown: [], hidden: 0)

    public init(shown: [GlancePermissionOption], hidden: Int) {
        self.shown = shown
        self.hidden = hidden
    }
}

extension GlancePermission {
    /// Which of the agent's answers fit, given how many lines of buttons there
    /// is room for.
    ///
    /// **Nothing here shortens a name.** `Allow Bash(cargo test -p
    /// farcooler-core)` is a real fixture in this repo, and `Allow Bash(cargo
    /// test…` is a button whose own label hides what it allows. So the list is
    /// what gives way, never the words: an option that does not fit is left off
    /// and counted in `hidden`, and the surface says how many it left off.
    ///
    /// **A yes shown without the no is refused outright.** `PermissionView`
    /// found this on a 46mm simulator — the emphasized option filled the screen
    /// and the reject fell below the fold — and named it: "a permission screen
    /// whose yes is visible and whose no is not is a screen arguing for yes,
    /// which is the one thing an approval must never do." A lock screen card is
    /// smaller than that watch screen and does not scroll, so the rule has to
    /// be arithmetic here rather than layout. Hence step 3 below, and hence the
    /// empty result: no buttons and a route into the app beats a card that only
    /// knows how to say yes.
    ///
    /// The three steps, in order:
    ///
    ///   1. If every option fits, show every option. This is the good case and
    ///      it needs no rule at all — three short answers in the agent's order
    ///      is exactly what the agent asked.
    ///   2. Otherwise fall back to the pair `ApprovalControls` already draws on
    ///      the phone: the plain yes and the plain no, chosen by the same
    ///      predicates, kept in the agent's relative order because a subsequence
    ///      preserves it. This is not a surface inventing an Allow/Deny pair —
    ///      every word on both buttons is the agent's, and the rest of its
    ///      answers are counted rather than dropped silently.
    ///   3. If that pair does not fit either, or if this agent's vocabulary
    ///      offers no recognizable no, show nothing. An agent whose `kind`
    ///      values this build has never met falls here, and that is correct:
    ///      `PermissionView.plainYes` returns nil for the same input and draws
    ///      every button alike, which is a choice a card with three lines does
    ///      not have.
    ///
    /// `columns` is characters per line and `lines` is lines of button. Both
    /// are estimates made by the caller and both are deliberately conservative,
    /// because the two directions cost differently: underestimating sends
    /// somebody into the app who could have answered from the card, while
    /// overestimating clips a button off the bottom of a card that does not
    /// scroll — and the button at the bottom is the reject.
    public func fit(lines: Int, columns: Int) -> GlanceOptionFit {
        guard lines > 0, columns > 0, !options.isEmpty else { return .none }

        let cost = { (option: GlancePermissionOption) -> Int in
            max(1, Int((Double(option.name.count) / Double(columns)).rounded(.up)))
        }
        let total = { (list: [GlancePermissionOption]) -> Int in list.reduce(0) { $0 + cost($1) } }

        if total(options) <= lines { return GlanceOptionFit(shown: options, hidden: 0) }

        guard let yes = options.firstIndex(where: Self.allows),
            let no = options.firstIndex(where: Self.rejects)
        else { return GlanceOptionFit(shown: [], hidden: options.count) }

        // A subsequence, so whichever the agent listed first is still first.
        let pair = [min(yes, no), max(yes, no)].map { options[$0] }
        guard total(pair) <= lines else { return GlanceOptionFit(shown: [], hidden: options.count) }
        return GlanceOptionFit(shown: pair, hidden: options.count - pair.count)
    }

    /// Which option gets the filled button, and nothing beyond that.
    ///
    /// The same derivation `ApprovalControls` runs on the phone and
    /// `PermissionView.plainYes` runs on the watch: the `allow_once` if the
    /// agent offered one, otherwise the first thing that allows at all. Copied
    /// a third time rather than reinvented so a card, a wrist and a phone
    /// looking at one permission agree about which answer is the plain yes.
    ///
    /// Nil for an agent whose vocabulary this build has never met, which draws
    /// every button alike.
    public var plainYes: GlancePermissionOption? {
        let allowing = options.filter(Self.allows)
        return allowing.first { $0.kind.lowercased().contains("once") } ?? allowing.first
    }

    /// The one place `kind` is interpreted rather than carried.
    ///
    /// Read for LAYOUT and never for wording: it decides which two of the
    /// agent's answers survive a card with three lines on it, and which one is
    /// drawn filled. The words on every button are still `name`, always.
    static func allows(_ option: GlancePermissionOption) -> Bool {
        let kind = option.kind.lowercased()
        return kind.contains("allow") || kind.contains("accept")
    }

    static func rejects(_ option: GlancePermissionOption) -> Bool {
        let kind = option.kind.lowercased()
        return kind.contains("reject") || kind.contains("deny")
    }
}

/// Where the glance permissions live, and the only code that reads or writes
/// them.
///
/// A second file beside `fleet.json` rather than a field on `FleetSnapshot`,
/// and the separation is not tidiness. The snapshot is written by two
/// processes — the app on every poll and `FarCoolerNotify` on every alerting
/// push — and the notification extension has no connection to any runner, so it
/// could only ever write this half as empty. A merge that blanked an agent's
/// options every time a push arrived would take the buttons off the card at the
/// exact moment the card appeared.
///
/// Everything that can fail returns the empty value rather than throwing, for
/// `SnapshotStore`'s reason: the readers are a widget's render path and an app
/// intent, and neither has anywhere useful to put an error. An unreadable file
/// says the same thing as an absent one — nothing is known — and a card with no
/// buttons is the correct rendering of that.
public enum GlancePermissionStore {
    private static let fileName = "permissions.json"

    /// The App Group container, resolved exactly as the snapshot's is.
    ///
    /// Through `SnapshotStore` rather than beside it: there are four channels
    /// and each has its own group, the key is stamped into every target's
    /// Info.plist by `generate-project.py`, and a second reader of that key is
    /// a second place for the four-way list to drift.
    private static func container() -> URL? {
        guard let group = SnapshotStore.groupIdentifier else { return nil }
        return SnapshotStore.container(forGroup: group)
    }

    public static func read() -> GlancePermissions {
        guard let directory = container() else { return .empty }
        return read(fromContainer: directory)
    }

    public static func read(fromContainer container: URL) -> GlancePermissions {
        guard let data = try? Data(contentsOf: container.appendingPathComponent(fileName))
        else { return .empty }
        return (try? decoder.decode(GlancePermissions.self, from: data)) ?? .empty
    }

    public static func write(_ permissions: GlancePermissions) {
        guard let directory = container() else { return }
        try? write(permissions, toContainer: directory)
    }

    public static func write(_ permissions: GlancePermissions, toContainer container: URL) throws {
        let data = try encoder.encode(permissions)
        try data.write(to: container.appendingPathComponent(fileName), options: .atomic)
    }

    /// Read, change, write.
    ///
    /// Safe because there is exactly ONE writer — the app, on the main actor —
    /// and the widget extension only ever reads. A second writing process would
    /// need a coordinated read-modify-write; the notification extension is
    /// deliberately not one, per the note on this type.
    public static func update(_ change: (GlancePermissions) -> GlancePermissions?) {
        guard let next = change(read()) else { return }
        write(next)
    }

    /// Seconds since 1970 on both sides, pinned rather than left to the
    /// default, for `SnapshotStore`'s reason: the app and the extension must
    /// not be built with different strategies and write a file neither can
    /// read.
    private static let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .secondsSince1970
        return encoder
    }()

    private static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .secondsSince1970
        return decoder
    }()
}
