import AgentKit
import Foundation

/// One terminal's agent session, as a view needs it.
///
/// Holds a `Transcript` and nothing else derived: activity, pane mode and the
/// agent's modes all arrive on the `Terminal` from the daemon (`Model.swift`),
/// because two clients deciding those for themselves is the disagreement this
/// whole design exists to prevent.
///
/// DEVIATION FROM THE PLAN, recorded honestly, the same way `DaemonClient`
/// records its own: the accepted architecture has this talk to
/// `farcooler-client`'s C ABI directly, the way `apps/ios/FarCooler/ClientCore.swift`
/// already does against `FarCoolerClient.xcframework`. The Mac target links no
/// such library today — `Package.swift` only vends `CFarCoolerVT`, the VT core
/// — and wiring one is outside the four files this task may touch. So this
/// follows the pattern the task was pointed at instead, `TerminalStream.swift`:
/// a dedicated object that shells the `farcooler` CLI directly, the same
/// seam (`cliPath` / `cliEnvironment` / `cliHostArguments`) `TerminalSurface`
/// already hands to `TerminalStream`. The CLI subcommands this calls
/// (`terminal agent-subscribe`, `agent-prompt`, `agent-answer`,
/// `agent-set-mode`, `agent-cancel`) do not exist yet — `crates/cli` was not in
/// scope for this task either — so this compiles and is ready to be exercised
/// the moment they land, but is inert against today's CLI. Swapping this file
/// for one built on `ClientCore` is the natural follow-up, exactly as
/// `DaemonClient`'s own doc comment describes for itself.
@MainActor
final class AgentStream: ObservableObject {
    @Published private(set) var transcript = Transcript()
    /// The run of the stream this transcript was built from.
    ///
    /// A shim renumbers its events from zero every time it restarts, and a
    /// pane-mode toggle restarts it. Rather than trying to reconcile two
    /// numberings — which failed in four different places — the daemon stamps
    /// the stream and a change means "you are holding a different
    /// conversation; take this one instead".
    private var epoch: UInt64 = 0
    /// Why this chat has stopped updating, in this app's words, or nil while
    /// it is. Drawn by `AgentComposer.activity`.
    ///
    /// Only the subscription writes it. It used to carry the refusals of
    /// `send`, `answer` and the rest too, and nothing drew it at all, so every
    /// one of them was lost (ov-136). A refusal now stays on the thing that
    /// was refused: `answering` for an answer, `failure` for everything else.
    @Published private(set) var connectionError: String?
    /// Polls that have failed in a row. One failed `agent-subscribe` among
    /// five a second is noise, and a line that flashed up and away for it
    /// would say something was wrong when nothing lasting was.
    private var failedPolls = 0
    /// How many failed polls in a row it takes to say so: about a second.
    static let pollsBeforeSaying = 4

    /// The answer to a permission ask, while it is out and after it failed.
    /// AgentKit's, as on iOS and Android: the card stays up until the runner
    /// takes the answer.
    @Published private(set) var answering = PermissionAnswering()
    /// The option last chosen for the ask whose answer failed, for Try Again.
    private var failedAnswer: (request: String, option: String)?

    /// Whether a prompt is out. A second send while one is out is refused, so
    /// Return pressed twice, or Try Again pressed during a send, sends once.
    @Published private(set) var sending = false

    /// The last send, setting or queue change that did not land.
    @Published private(set) var failure: AgentActionFailure?

    /// Runs the CLI in tests instead of a subprocess, the way
    /// `DaemonClient.commandRunnerForTesting` does. Throws `StreamError` as the
    /// subprocess would.
    var runnerForTesting: (@MainActor ([String]) async throws -> Data)?

    private let terminal: String
    private var binary: String?
    private var environment: [String: String] = [:]
    private var hostArguments: [String] = []
    private var pollTask: Task<Void, Never>?
    /// The one `--follow` reader, while it runs. See `AgentFollow`.
    private var follower: AgentFollow?
    /// The reader's restart after it ended, while one is waiting.
    private var restartTask: Task<Void, Never>?
    /// Set when the CLI refused `--follow`: one older than this app, from
    /// `FARCOOLER_BIN`. This stream polls from then on, as it always used to.
    private var followRefused = false
    /// Why this session's runner cannot be acted on, or nil if it can —
    /// the same check `ContentView.act(on:)` runs for every terminal-pane
    /// mutation, reached here too.
    ///
    /// Without this, `send`, `answer`, `cancel`, `setConfig` and the rest
    /// below routed correctly BY `hostArguments` — the CLI subprocess really
    /// does run against the right runner — but never asked FIRST whether
    /// that runner was already known to be gone. Messaging an agent on a
    /// runner already `.unreachable` burned a full `ConnectTimeout` finding
    /// that out the hard way, with no banner, for a refusal `FleetStore`
    /// already had the answer to.
    var refusal: () -> String? = { nil }

    init(terminal: String) {
        self.terminal = terminal
    }

    /// Begin reading. Safe to call again: a second call replaces the first
    /// reader rather than running two, the same rule `TerminalStream.start`
    /// follows for the same reason — a pane can be reconfigured without first
    /// being told to stop.
    ///
    /// One `agent-subscribe --follow` process for the life of the view, which
    /// prints only when there's news (ov-229). This was a fresh process every
    /// 200 ms. Polling remains for the tests' runner and for a CLI that
    /// predates the flag.
    func start(
        binary: String?, environment: [String: String], hostArguments: [String] = [],
        refusal: @escaping () -> String? = { nil }
    ) {
        stop()
        self.binary = binary
        self.environment = environment
        self.hostArguments = hostArguments
        self.refusal = refusal

        if runnerForTesting == nil, binary != nil, !followRefused {
            startFollowing()
        } else {
            startPolling()
        }
    }

    private func startPolling() {
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.pump()
                try? await Task.sleep(for: .milliseconds(200))
            }
        }
    }

    private func startFollowing() {
        guard let binary else { return }
        let reader = AgentFollow()
        follower = reader
        reader.start(
            binary: binary,
            arguments: hostArguments + [
                "terminal", "agent-subscribe", terminal,
                "--from-seq", "\(transcript.cursor)", "--epoch", "\(epoch)", "--json", "--follow",
            ],
            environment: environment,
            onLine: { [weak self] line in
                Task { @MainActor in
                    guard let self, self.follower === reader else { return }
                    if let batch = try? JSONDecoder().decode(Batch.self, from: line) {
                        self.take(batch)
                    }
                }
            },
            onEnd: { [weak self] printed, said, status in
                Task { @MainActor in
                    guard let self, self.follower === reader else { return }
                    self.followEnded(printed: printed, said: said, status: status)
                }
            })
    }

    /// The reader exited on its own: the link dropped, the runner went away,
    /// or the CLI doesn't know `--follow`. Counted like a failed poll, and
    /// tried again after a wait that grows, so a runner that's gone isn't
    /// asked five times a second.
    private func followEnded(printed: Bool, said: String, status: Int32) {
        follower = nil
        // Status 2 is clap's usage error: an unknown flag, whatever it said.
        if !printed, said.contains("--follow") || status == 2 {
            followRefused = true
            startPolling()
            return
        }
        noteFailure(StreamError.failed(said))
        let wait = min(0.25 * pow(2, Double(max(failedPolls - 1, 0))), 5)
        restartTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(wait))
            guard let self, !Task.isCancelled, self.follower == nil, self.pollTask == nil else { return }
            self.startFollowing()
        }
    }

    func stop() {
        pollTask?.cancel()
        pollTask = nil
        restartTask?.cancel()
        restartTask = nil
        follower?.stop()
        follower = nil
    }

    deinit {
        pollTask?.cancel()
        restartTask?.cancel()
        follower?.stop()
    }

    /// Ask for everything after what we already hold.
    ///
    /// The cursor comes from the transcript rather than from a counter kept
    /// here, so a reconnect cannot skip or repeat events after a gap — the
    /// same reason `Transcript.cursor` exists rather than a second count.
    func pump() async {
        do {
            take(try await agentSubscribe(fromSeq: transcript.cursor))
        } catch {
            noteFailure(error)
        }
    }

    /// Fold one batch into the transcript.
    private func take(_ batch: Batch) {
        // A different epoch means the stream restarted — the pane was
        // toggled, or the shim came back — and every number this holds
        // counts positions in a conversation that no longer exists. The
        // batch that comes back is the whole transcript, so it replaces
        // rather than appends. Four separate bugs came from trying to
        // reconcile the two numberings instead of admitting they are
        // different streams.
        if batch.epoch != epoch {
            epoch = batch.epoch
            transcript.resetForNewEpoch()
        } else if batch.events.isEmpty {
            // Cleared here too, not only after applying events. A steady
            // poll that returns nothing is the healthy case, and leaving a
            // previous failure's message up through it meant the banner
            // stayed on screen forever once anything had ever gone wrong.
            recovered()
            return
        }

        let decoded = batch.events.map { frame -> Sequenced in
            // A frame this client cannot read becomes a visible gap, never
            // a dropped event. `try?` here meant a decoder that fell behind
            // the daemon rendered a blank chat with no pickers and no sign
            // anything was wrong — which is exactly the silence the whole
            // Gap contract exists to forbid.
            let event = (try? AgentEvent.decode(from: frame.payloadJson)) ?? .gap(.unparsed)
            return Sequenced(seq: frame.seq, event: event)
        }
        transcript.apply(decoded)
        recovered()
    }

    /// A read worked. Written only when it changes anything: `@Published`
    /// fires on every assignment, and this ran five times a second with
    /// nothing to say, redrawing the chat each time (ov-229).
    private func recovered() {
        failedPolls = 0
        if connectionError != nil { connectionError = nil }
    }

    private func noteFailure(_ error: Error) {
        // A sentence, never the error. This was `String(describing:)`,
        // which would have drawn `failed("error: …")` had anything drawn it.
        failedPolls += 1
        if failedPolls >= Self.pollsBeforeSaying {
            let sentence = Self.pollTrouble(error)
            if connectionError != sentence { connectionError = sentence }
        }
    }

    /// What the composer says while this chat can't be read.
    static func pollTrouble(_ error: Error) -> String {
        if case StreamError.cliMissing = error {
            return "Far Cooler’s command-line tool isn’t installed, so this chat can’t update."
        }
        return "This chat isn’t updating. Trying again…"
    }

    // MARK: - Calls

    private struct EventFrame: Decodable {
        let seq: UInt64
        let payloadJson: String
    }

    private struct Batch: Decodable {
        let events: [EventFrame]
        let epoch: UInt64
    }

    /// New events for this terminal's agent session, from a cursor.
    ///
    /// Empty rather than an error for a terminal that has never run an agent —
    /// the daemon's own contract (`terminal.agent_subscribe`, tested in
    /// `crates/client/tests/against_a_real_daemon.rs`) — so a chat view can
    /// open before the first turn instead of showing a connection failure for
    /// a pane that is simply new.
    private func agentSubscribe(fromSeq: UInt64) async throws -> Batch {
        let data = try await runCLI([
            "terminal", "agent-subscribe", terminal,
            "--from-seq", "\(fromSeq)", "--epoch", "\(epoch)", "--json",
        ])
        return try JSONDecoder().decode(Batch.self, from: data)
    }

    /// Whether a call is refused before it is made, because this session's
    /// runner is already known to be gone.
    ///
    /// Every mutating call below asks this first, before any local,
    /// optimistic transcript edit as well as before the CLI call, so a message
    /// typed to a runner already known to be gone is refused at once instead
    /// of hanging for a `ConnectTimeout` or drawing a local echo of something
    /// that was never sent. The runner's own reason is not repeated: the
    /// composer already shows it above the field.
    private func refusedHere() -> Bool { refusal() != nil }

    /// Send a prompt. True once the runner has it; false, with `failure` saying
    /// why, when it didn't go or another send was still out.
    ///
    /// The composer keeps its text until this says true (ov-136). It used to
    /// clear first and send with `try?`, so a message that never left was gone
    /// from the field and drawn in the conversation as if it had been sent.
    @discardableResult
    func send(_ text: String, images: [ComposerImage] = []) async -> Bool {
        guard !sending else { return false }
        guard !refusedHere() else {
            failure = AgentActionFailure(.send, Self.sentence(.send, refusedWith: nil))
            return false
        }
        sending = true
        defer { sending = false }
        if failure?.action == .send { failure = nil }
        // Always drawn, never predicted.
        //
        // This used to echo only when the composer believed no turn was
        // running — a guess made from fleet state about a decision taken on
        // the agent channel. Guessing wrong meant the message reached the
        // model and was drawn by nobody. The daemon answers with a
        // `PromptQueue` if it held the message, and `Transcript` withdraws the
        // row then; until it does, what you typed is on screen. Drawn BEFORE
        // the call for that reason: a queue report that beat a later echo
        // would leave the words in the conversation and the queue both.
        // Written to a temp file and handed to the CLI by path.
        //
        // The CLI reads the bytes and puts them in the prompt as image blocks;
        // the path never leaves this machine. Passing base64 as an argument
        // instead would put a megabyte on a command line, which is the one
        // thing an argv is guaranteed to be bad at.
        var pictures: [String] = []
        var scratch: [URL] = []
        for image in images {
            let url = FileManager.default.temporaryDirectory
                .appendingPathComponent("farcooler-attach-\(UUID().uuidString)")
                .appendingPathExtension(image.mime == "image/png" ? "png" : "jpg")
            // A picture that can't be handed over stops the send. It used to
            // be skipped, and the message went without it, saying nothing.
            do {
                try image.data.write(to: url)
            } catch {
                for url in scratch { try? FileManager.default.removeItem(at: url) }
                failure = AgentActionFailure(.send, Self.pictureUnsent)
                return false
            }
            scratch.append(url)
            pictures.append(url.path)
        }
        let arguments = AgentAction.sendArguments(terminal: terminal, text: text, images: pictures)
        defer { for url in scratch { try? FileManager.default.removeItem(at: url) } }
        // Drawn after the pictures are ready, so a send stopped above draws
        // nothing, and before the call, for the queue report's sake (above).
        let echo = transcript.appendLocalUserMessage(text)
        do {
            _ = try await runCLI(arguments)
            return true
        } catch {
            guard Self.sendIsKnownUnsent(error) else {
                // No word from a runner: the link dropped or timed out, maybe
                // after the daemon took the prompt. It may be with the agent
                // already, which redraws nothing for a prompt that went
                // straight in, so the echo stays. Saying "wasn't sent" here
                // invited a second copy of the prompt; this says what is
                // known, and offers no Try Again. The words stay in the field
                // for a person who checks and wants to send them again.
                failure = AgentActionFailure(.send, Self.sendMayNotHaveLanded, canRetry: false)
                return false
            }
            // Refused, so certainly not sent. The words are still in the
            // composer, so the echo comes back out: left in, a Try Again that
            // worked would draw them twice.
            transcript.withdrawLocalUserMessage(rowID: echo)
            failure = AgentActionFailure(.send, Self.sentence(.send, for: error))
            return false
        }
    }

    /// What a send says when a picture couldn't be read or handed over.
    static let pictureUnsent = "Couldn’t attach the picture. Your message wasn’t sent."

    /// What a send says when the runner may have it: no refusal came back.
    static let sendMayNotHaveLanded =
        "Your message may not have reached the runner. Check the chat before sending it again."

    /// Whether a failed send certainly didn't reach the agent: the CLI wasn't
    /// there, it refused the size before sending, or a runner refused it by
    /// word. Anything else broke after it may have landed.
    static func sendIsKnownUnsent(_ error: Error) -> Bool {
        // A cancelled send may have been delivered before it was stopped.
        if error is CancellationError { return false }
        guard case let StreamError.failed(message) = error else { return true }
        let lower = message.lowercased()
        if lower.contains("too large") || lower.contains("payload") { return true }
        return word(of: error) != nil
    }

    /// Say a send failed before it reached this stream: the composer couldn't
    /// read a picture it was given.
    func refuseSend(_ sentence: String) {
        failure = AgentActionFailure(.send, sentence)
    }

    /// Rewrite a message that has not gone out yet.
    func editQueued(_ id: String, _ text: String) async {
        await perform(.editQueued(id: id, text: text))
    }

    /// Send a queued message into the turn already running.
    func steerQueued(_ id: String) async {
        await perform(.steerQueued(id: id))
    }

    /// Take back a message that has not gone out yet.
    func cancelQueued(_ id: String) async {
        await perform(.cancelQueued(id: id))
    }

    func setConfig(_ id: String, _ value: String) async {
        await perform(.config(id: id, value: value))
    }

    /// Answer the pending permission ask.
    ///
    /// The card comes down when the runner takes the answer, or refuses it as
    /// one nothing holds any more, never on the click. It used to come down
    /// on the click with the call's error dropped, so a refused ⌘↩ left the
    /// agent blocked on an ask this pane could no longer show: its
    /// `Permission` is behind the cursor. See AgentKit's `PermissionAnswering`,
    /// which iOS and Android follow too.
    func answer(_ requestID: String, _ optionID: String) async {
        guard answering.begin(requestID) else { return }
        failedAnswer = nil
        let outcome: PermissionAnswering.Outcome
        if refusedHere() {
            outcome = .failed("Couldn’t reach this runner. Your answer wasn’t sent.")
        } else {
            do {
                _ = try await runCLI(["terminal", "agent-answer", terminal, requestID, optionID, "--json"])
                outcome = .sent
            } catch {
                outcome = PermissionAnswering.outcome(refusedWith: Self.word(of: error))
            }
        }
        guard answering.finish(requestID, outcome) else {
            failedAnswer = (requestID, optionID)
            return
        }
        // Down at once, and only if it is still the one this answered. The
        // daemon records a `Resolved` for a shim ask once the shim has the
        // answer, which would clear it on a later poll; a hook ask may never
        // get one here. This is the fast path for both.
        if transcript.pendingPermission?.id == requestID {
            transcript.clearPendingPermission()
        }
    }

    /// Send the failed answer again, with the option chosen the first time.
    func retryAnswer() async {
        guard let failed = failedAnswer, answering.sending == nil else { return }
        await answer(failed.request, failed.option)
    }

    /// Run the failed action again. Only the call: a failed send's words are
    /// in the composer, which sends them itself.
    ///
    /// The failure is taken down before the call, so a second click while the
    /// first is out finds nothing to run. A failure puts it back.
    func retry() async {
        guard let failed = failure, failed.action != .send else { return }
        failure = nil
        await perform(failed.action)
    }

    /// Put the failure away without trying again.
    func dismissFailure() { failure = nil }

    /// Start this pane's agent again, in place (ov-174): `set-pane-mode
    /// agent` on a pane already in agent mode respawns its shim, and what was
    /// queued is sent once the new one is up. A refusal is said like any
    /// other action's.
    func restart() async {
        await perform(.restart)
    }

    /// The line a pane whose agent stopped or never started shows beside the
    /// composer, whatever the transcript holds; nil while nothing has said it
    /// failed.
    ///
    /// Read here rather than in the view, so a test can hold it: it was drawn
    /// only over an empty transcript, so an agent that died after its first
    /// reply left nothing on screen but a turn that stopped (ov-174).
    func stoppedLine(for terminal: Terminal) -> String? {
        terminal.chatFailure?.sentence(started: !transcript.rows.isEmpty)
    }

    /// Whether that line offers Restart (not with no adapter).
    func restartOffered(for terminal: Terminal) -> Bool {
        terminal.chatFailure?.offersRestart == true
    }

    /// One mutating call other than a send or an answer, said if it fails.
    private func perform(_ action: AgentAction) async {
        guard !refusedHere() else {
            failure = AgentActionFailure(action, Self.sentence(action, refusedWith: nil))
            return
        }
        if failure?.action == action { failure = nil }
        var previous: String?
        if case let .config(id, value) = action {
            // Shown before it is confirmed. The adapter applies the change
            // without announcing it, so waiting for an echo left the picker
            // snapping back to its old value — which reads as the control
            // doing nothing at all. Put back if the runner refuses it.
            previous = transcript.configOptions.first { $0.id == id }?.currentValue
            transcript.selectConfigOptionLocally(id: id, value: value)
        }
        do {
            _ = try await runCLI(action.arguments(terminal: terminal) + ["--json"])
        } catch {
            if case let .config(id, value) = action, let previous,
                transcript.configOptions.first(where: { $0.id == id })?.currentValue == value
            {
                transcript.selectConfigOptionLocally(id: id, value: previous)
            }
            failure = AgentActionFailure(action, Self.sentence(action, for: error))
        }
    }

    /// The runner's refusal word on a failed call: the `code:` line `--json`
    /// puts on the CLI's stderr. Nil for a failure that never reached it.
    static func word(of error: Error) -> String? {
        guard case let StreamError.failed(message) = error else { return nil }
        return TaskFailure.code(in: message)
    }

    /// What to say about a failed `action`, from how it failed.
    static func sentence(_ action: AgentAction, for error: Error) -> String {
        if action == .send, case let StreamError.failed(message) = error {
            // The size ceiling is refused by the CLI before anything reaches a
            // runner, so it has no word; read off the prose, as on iOS.
            let lower = message.lowercased()
            if lower.contains("too large") || lower.contains("payload") {
                return "That was too large to send. Try a smaller image."
            }
        }
        return sentence(action, refusedWith: word(of: error))
    }

    /// What to say about a failed `action`, from the runner's word. No word
    /// means it never reached a runner that could refuse it.
    static func sentence(_ action: AgentAction, refusedWith word: String?) -> String {
        guard let word, !word.isEmpty else { return "Couldn’t reach this runner. " + action.unsent }
        return RunnerRefusal.trouble(forWord: word, message: "", after: action.unsent).sentence
    }

    // MARK: - Subprocess
    //
    // Deliberately not `DaemonClient.run`: that method is private to its own
    // file, for the same reason `TerminalStream` does not call it either — a
    // stream-shaped object outlives any one view and manages its own process,
    // rather than borrowing a helper scoped to the fleet-refresh call sites
    // that already use it.

    enum StreamError: LocalizedError {
        case cliMissing
        case failed(String)
        case malformed

        var errorDescription: String? {
            switch self {
            case .cliMissing: "The farcooler CLI was not found."
            case let .failed(message): message
            case .malformed: "The daemon returned something unreadable."
            }
        }
    }

    private func runCLI(_ args: [String]) async throws -> Data {
        if let stub = runnerForTesting { return try await stub(args) }
        guard let binary else { throw StreamError.cliMissing }
        let ran = await ProcessRunner.run(
            binary, hostArguments + args, environment: environment)
        if let why = ran.launchFailure { throw StreamError.failed(why) }
        if ran.cancelled { throw CancellationError() }
        if !ran.succeeded {
            let message =
                String(data: ran.stderr, encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            throw StreamError.failed(message.isEmpty ? "command failed" : message)
        }
        return ran.stdout
    }
}

/// One thing a chat pane asks its agent's runner to do, other than answer a
/// permission ask (AgentKit's `PermissionAnswering` holds that one).
enum AgentAction: Equatable {
    case send
    case config(id: String, value: String)
    case editQueued(id: String, text: String)
    case steerQueued(id: String)
    case cancelQueued(id: String)
    /// Start the pane's agent again, in place.
    case restart

    /// The CLI call. For `.send` only its start: `AgentStream.send` adds the
    /// composer's words and pictures.
    func arguments(terminal: String) -> [String] {
        switch self {
        case .send: return ["terminal", "agent-prompt", terminal]
        case let .config(id, value): return ["terminal", "agent-set-config", terminal, id, value]
        // `--` before the words, so a message starting with a dash is a
        // message, not a flag (ov-214 review: `--help` "sent" and was lost).
        case let .editQueued(id, text): return ["terminal", "agent-edit-queued", terminal, id, "--", text]
        case let .steerQueued(id): return ["terminal", "agent-steer-queued", terminal, id]
        case let .cancelQueued(id): return ["terminal", "agent-cancel-queued", terminal, id]
        case .restart: return ["terminal", "set-pane-mode", terminal, "agent"]
        }
    }

    /// A message sent from the composer: its pictures by path, then `--json`,
    /// then `--` and the words, so words starting with a dash are words.
    static func sendArguments(terminal: String, text: String, images: [String] = []) -> [String] {
        AgentAction.send.arguments(terminal: terminal) + images.flatMap { ["--image", $0] } + ["--json", "--", text]
    }

    /// What didn't happen, as the end of a sentence that says why.
    var unsent: String {
        switch self {
        case .send: return "Your message wasn’t sent."
        case .config: return "The setting wasn’t changed."
        case .editQueued: return "Your edit to the queued message wasn’t saved."
        case .steerQueued: return "The queued message wasn’t sent."
        case .cancelQueued: return "The queued message wasn’t removed."
        case .restart: return "The agent wasn’t restarted."
        }
    }

    /// The queued message this is about, so its row can carry the failure.
    var queuedID: String? {
        switch self {
        case let .editQueued(id, _), let .steerQueued(id), let .cancelQueued(id): return id
        case .send, .config, .restart: return nil
        }
    }
}

/// An action that didn't land, and what to say about it.
struct AgentActionFailure: Equatable {
    let action: AgentAction
    let sentence: String
    /// Whether Try Again is offered. Not for a send that may have landed:
    /// trying again could give the agent the prompt twice.
    let canRetry: Bool

    init(_ action: AgentAction, _ sentence: String, canRetry: Bool = true) {
        self.action = action
        self.sentence = sentence
        self.canRetry = canRetry
    }
}
