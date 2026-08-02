import Foundation

/// One terminal's agent session, live, over the same host connection a
/// terminal's screen already polls through.
///
/// Holds a `Transcript` and nothing else derived: `agentMode`, the available
/// modes and commands all arrive on the transcript itself, from the events
/// the daemon sent — never recomputed here, for the same reason `Connection`
/// never computes a workspace's state. `Transcript`, `AgentEvent` and
/// `Sequenced` are not imported from anywhere: this target compiles
/// `apps/shared/AgentKit/Sources/AgentKit` directly as part of the app (see
/// `generate-project.py`'s `agentKitGroup`), because iOS has no SwiftPM
/// project to vend it as a real module the way `apps/macos/Package.swift`
/// does — so those types simply live in this same module already.
@MainActor
final class AgentStream: ObservableObject {
    @Published private(set) var transcript = Transcript()
    @Published private(set) var connectionError: String?

    private let terminal: String
    private let core: ClientCore
    private var pollTask: Task<Void, Never>?

    init(terminal: String, core: ClientCore) {
        self.terminal = terminal
        self.core = core
    }

    deinit { pollTask?.cancel() }

    func start() {
        pollTask?.cancel()
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.pump()
                // Not the Mac's 200ms: that poll is a local call into a
                // daemon on the same machine, and this one is an ssh round
                // trip. A chat transcript has no per-frame redraw to protect
                // the way a terminal's screen does, so a slower, still-brisk
                // cadence costs far less battery for a difference nobody
                // reading a conversation would notice.
                try? await Task.sleep(for: .milliseconds(700))
            }
        }
    }

    func stop() {
        pollTask?.cancel()
        pollTask = nil
    }

    private struct EventFrame: Decodable {
        let seq: UInt64
        let payloadJson: String
    }

    private struct Batch: Decodable {
        let events: [EventFrame]
        /// Which run of the stream these numbers belong to. See `epoch`.
        let epoch: UInt64
    }

    /// Ask for everything after what we already hold.
    ///
    /// The cursor comes from `transcript.cursor` rather than a counter kept
    /// here, so a reconnect — or this object simply being recreated when the
    /// tab strip switches to a different agent pane, see `AgentView` — cannot
    /// skip or repeat events after a gap.

    /// The run of the stream this transcript was built from.
    private var epoch: UInt64 = 0


    private func pump() async {
        do {
            let data = try await core.call(
                "terminal.agent_subscribe",
                [
                    "terminal": terminal,
                    "fromSeq": Int(clamping: transcript.cursor),
                    "epoch": Int(clamping: epoch),
                ])
            let batch = try JSONDecoder().decode(Batch.self, from: data)

            // A different epoch means the stream restarted — the pane was
            // toggled, or the shim came back — and every number this holds
            // counts positions in a conversation that no longer exists. The
            // batch that comes back is the whole transcript, so it replaces
            // rather than appends. See `AgentSupervisor::replay`; the Mac
            // client does exactly this, and without it a phone looking at a
            // toggled pane shows a conversation that is not the one being
            // served.
            if batch.epoch != epoch {
                epoch = batch.epoch
                transcript.resetForNewEpoch()
            } else if batch.events.isEmpty {
                connectionError = nil
                return
            }
            let decoded = batch.events.compactMap { frame -> Sequenced? in
                // A malformed single frame does not need to fail the whole
                // batch — `AgentEvent.decode` already turns an event this
                // client does not recognise into `.gap(.unparsed)` rather
                // than throwing; only truly unreadable JSON reaches here.
                guard let event = try? AgentEvent.decode(from: frame.payloadJson) else { return nil }
                return Sequenced(seq: frame.seq, event: event)
            }
            transcript.apply(decoded)
            connectionError = nil
        } catch {
            connectionError = String(describing: error)
        }
    }

    func send(_ text: String, whileWorking: Bool = false) async {
        // Echoed locally only when it is going out NOW. A message written
        // mid-turn waits in the queue and joins the transcript when it is
        // actually sent — see `AgentStream.send` on the Mac.
        if !whileWorking {
            transcript.appendLocalUserMessage(text)
        }
        _ = try? await core.call("terminal.agent_prompt", ["terminal": terminal, "text": text])
    }

    /// Rewrite a message that has not gone out yet.
    func editQueued(_ id: String, _ text: String) async {
        _ = try? await core.call(
            "terminal.agent_edit_queued",
            ["terminal": terminal, "queuedId": id, "text": text])
    }

    /// Take back a message that has not gone out yet.
    func cancelQueued(_ id: String) async {
        _ = try? await core.call(
            "terminal.agent_cancel_queued", ["terminal": terminal, "queuedId": id])
    }

    func setModel(_ model: String) async {
        _ = try? await core.call(
            "terminal.agent_set_model", ["terminal": terminal, "model": model])
    }

    func setConfig(_ id: String, _ value: String) async {
        _ = try? await core.call(
            "terminal.agent_set_config", ["terminal": terminal, "configId": id, "value": value])
    }

    func answer(_ requestID: String, _ optionID: String) async {
        _ = try? await core.call(
            "terminal.agent_answer",
            ["terminal": terminal, "requestId": requestID, "optionId": optionID])
    }

    func setMode(_ mode: String) async {
        _ = try? await core.call("terminal.agent_set_mode", ["terminal": terminal, "mode": mode])
    }

    func cancel() async {
        _ = try? await core.call("terminal.agent_cancel", ["terminal": terminal])
    }
}
