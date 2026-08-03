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
    @Published private(set) var connectionError: String?

    private let terminal: String
    private var binary: String?
    private var environment: [String: String] = [:]
    private var hostArguments: [String] = []
    private var pollTask: Task<Void, Never>?

    init(terminal: String) {
        self.terminal = terminal
    }

    /// Begin polling. Safe to call again: a second call replaces the first
    /// poll loop rather than running two, the same rule `TerminalStream.start`
    /// follows for the same reason — a pane can be reconfigured without first
    /// being told to stop.
    func start(binary: String?, environment: [String: String], hostArguments: [String] = []) {
        stop()
        self.binary = binary
        self.environment = environment
        self.hostArguments = hostArguments

        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.pump()
                try? await Task.sleep(for: .milliseconds(200))
            }
        }
    }

    func stop() {
        pollTask?.cancel()
        pollTask = nil
    }

    deinit { pollTask?.cancel() }

    /// Ask for everything after what we already hold.
    ///
    /// The cursor comes from the transcript rather than from a counter kept
    /// here, so a reconnect cannot skip or repeat events after a gap — the
    /// same reason `Transcript.cursor` exists rather than a second count.
    private func pump() async {
        do {
            let batch = try await agentSubscribe(fromSeq: transcript.cursor)

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
                connectionError = nil
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
            connectionError = nil
        } catch {
            connectionError = String(describing: error)
        }
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

    func send(_ text: String, images: [ComposerImage] = [], whileWorking: Bool) async {
        // Echoed locally only when it is going out NOW.
        //
        // A message written mid-turn does not become part of the conversation
        // when you press return — the agent is busy, and it waits. Drawing it
        // in the transcript anyway claimed something that had not happened; it
        // shows up in the queue instead, and joins the transcript at the moment
        // it is actually sent.
        if !whileWorking {
            transcript.appendLocalUserMessage(text)
        }
        // Written to a temp file and handed to the CLI by path.
        //
        // The CLI reads the bytes and puts them in the prompt as image blocks;
        // the path never leaves this machine. Passing base64 as an argument
        // instead would put a megabyte on a command line, which is the one
        // thing an argv is guaranteed to be bad at.
        var arguments = ["terminal", "agent-prompt", terminal, text]
        var scratch: [URL] = []
        for image in images {
            let url = FileManager.default.temporaryDirectory
                .appendingPathComponent("farcooler-attach-\(UUID().uuidString)")
                .appendingPathExtension(image.mime == "image/png" ? "png" : "jpg")
            guard (try? image.data.write(to: url)) != nil else { continue }
            scratch.append(url)
            arguments += ["--image", url.path]
        }
        _ = try? await runCLI(arguments)
        for url in scratch { try? FileManager.default.removeItem(at: url) }
    }

    /// Rewrite a message that has not gone out yet.
    func editQueued(_ id: String, _ text: String) async {
        _ = try? await runCLI(["terminal", "agent-edit-queued", terminal, id, text])
    }

    /// Send a queued message into the turn already running.
    func steerQueued(_ id: String) async {
        _ = try? await runCLI(["terminal", "agent-steer-queued", terminal, id])
    }

    /// Take back a message that has not gone out yet.
    func cancelQueued(_ id: String) async {
        _ = try? await runCLI(["terminal", "agent-cancel-queued", terminal, id])
    }

    func setModel(_ model: String) async {
        _ = try? await runCLI(["terminal", "agent-set-model", terminal, model])
    }

    func setConfig(_ id: String, _ value: String) async {
        // Shown before it is confirmed. The adapter applies the change without
        // announcing it, so waiting for an echo left the picker snapping back
        // to its old value — which reads as the control doing nothing at all.
        transcript.selectConfigOptionLocally(id: id, value: value)
        _ = try? await runCLI(["terminal", "agent-set-config", terminal, id, value])
    }

    func answer(_ requestID: String, _ optionID: String) async {
        // Taken down on click, not on an echo. The agent resumes without
        // acknowledging the request it was blocked on, so a card that waited
        // for confirmation sat there after the work it gated had happened.
        transcript.clearPendingPermission()
        _ = try? await runCLI(["terminal", "agent-answer", terminal, requestID, optionID])
    }

    func setMode(_ mode: String) async {
        _ = try? await runCLI(["terminal", "agent-set-mode", terminal, mode])
    }

    func cancel() async {
        _ = try? await runCLI(["terminal", "agent-cancel", terminal])
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
        guard let binary else { throw StreamError.cliMissing }
        let env = environment
        let command = hostArguments + args

        return try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let process = Process()
                process.executableURL = URL(fileURLWithPath: binary)
                process.arguments = command
                process.environment = env

                let out = Pipe()
                let err = Pipe()
                process.standardOutput = out
                process.standardError = err

                do {
                    try process.run()
                } catch {
                    continuation.resume(throwing: StreamError.failed(error.localizedDescription))
                    return
                }

                let stdout = out.fileHandleForReading.readDataToEndOfFile()
                let stderr = err.fileHandleForReading.readDataToEndOfFile()
                process.waitUntilExit()

                if process.terminationStatus != 0 {
                    let message =
                        String(data: stderr, encoding: .utf8)?
                        .trimmingCharacters(in: .whitespacesAndNewlines) ?? "command failed"
                    continuation.resume(throwing: StreamError.failed(message))
                    return
                }

                continuation.resume(returning: stdout)
            }
        }
    }
}
