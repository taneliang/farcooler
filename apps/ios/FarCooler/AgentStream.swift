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

    /// Why this pane is not showing a conversation: a sentence this app wrote,
    /// and — only where it has no account of its own — the core's own words.
    ///
    /// Two fields rather than one string, because one string is how the core's
    /// words came to be rendered as the app's. Every assignment below but one
    /// is a sentence somebody wrote; the last is `localizedDescription`, and it
    /// went into the same `Text` under the same headline as the others, so
    /// there was no way to read it as anything but Far Cooler talking. Kept,
    /// because for a runner that cannot be reached it is the only diagnosis
    /// there is — it goes in a `DetailBox` now, where output goes.
    struct Trouble: Equatable {
        let sentence: String
        var transcript: String?
    }

    @Published private(set) var connectionError: Trouble?

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

    /// The shape `terminal.agent_subscribe` answers with.
    ///
    /// Not private, and not because anything here needed loosening:
    /// `WatchLinkHost` makes the same call to answer the watch's one question
    /// about a blocked agent, and a second declaration of these four fields
    /// would be a second spelling of `payloadJson` waiting to happen. The
    /// daemon's own CLI points at this type by name — see `TerminalCmd::
    /// AgentSubscribe` in `crates/cli/src/main.rs` — so it is already the
    /// documented Swift shape of that reply.
    struct EventFrame: Decodable {
        let seq: UInt64
        let payloadJson: String
    }

    struct Batch: Decodable {
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
            // Epoch 0 with nothing in it means the daemon has no agent session
            // for this terminal at all — `AgentSupervisor::replay` returns
            // exactly that for a terminal it has never seen. A client starting
            // at epoch 0 matches it, finds no events, and returns, which looked
            // from the outside like a conversation nobody had started yet. It
            // is not: it is a pane that has no shim behind it, and the two want
            // different words.
            if batch.epoch == 0 && batch.events.isEmpty && transcript.rows.isEmpty {
                connectionError = Trouble(
                    sentence:
                        "No agent session on pane \(terminal.prefix(8)) yet. "
                        + "It may still be starting.")
                return
            }

            if batch.epoch != epoch {
                epoch = batch.epoch
                transcript.resetForNewEpoch()
            } else if batch.events.isEmpty {
                connectionError = nil
                return
            }
            let decoded = batch.events.map { frame -> Sequenced in
                // A malformed single frame does not fail the whole batch, and
                // it does not vanish either.
                //
                // `AgentEvent.decode` already turns an event this client does
                // not RECOGNIZE into `.gap(.unparsed)`, so what reaches the
                // fallback here is genuinely unreadable JSON. That used to be
                // dropped by a `compactMap`, which left a hole in the
                // transcript with nothing to show it: the agent did something,
                // this build could not read it, and the screen simply omitted
                // it. A gap marker is the format's own word for exactly that,
                // and saying "something here I can't show" is the only honest
                // rendering of a payload we failed to parse.
                let event = (try? AgentEvent.decode(from: frame.payloadJson)) ?? .gap(.unparsed)
                return Sequenced(seq: frame.seq, event: event)
            }
            transcript.apply(decoded)
            recordForGlances()
            connectionError = nil
        } catch {
            // `String(describing:)` on a Swift error prints the CASE, not the
            // message: what reached the chat pane was the literal text
            // `disconnected("not connected")` — a Rust-side word for the FFI's
            // empty session slot, wrapped in Swift enum syntax, shown to
            // somebody who wanted to read a conversation. `localizedDescription`
            // is the half of it that was meant to be read.
            //
            // And a dropped link gets a sentence of its own, because it is the
            // one that happens: any ssh hiccup anywhere in the app empties that
            // slot, so every poll afterwards lands here until something
            // reconnects. `refresh` on `Connection` is what does the
            // reconnecting; this pane's job is to say so and then be quiet
            // about it, which is what clearing on the next good batch does.
            if let core = error as? ClientCore.CoreError, case .disconnected = core {
                connectionError = Trouble(
                    sentence: "The connection to this runner dropped. Reconnecting…")
            } else {
                // The one arm with nothing written about it, so the sentence
                // says only what is known — that the read did not finish — and
                // the words themselves go in the box below it. Naming a cause
                // here would be inventing one: from this side of an ssh link
                // the cause is unknowable, and a guess sends somebody to change
                // a setting that was never the problem. See `Enrollment.note`.
                connectionError = Trouble(
                    sentence: "The request that reads it didn’t finish.",
                    transcript: error.localizedDescription)
            }
        }
    }

    func send(_ text: String, images: [(mime: String, data: Data)] = []) async {
        // Always drawn, never predicted — see the Mac's `AgentStream.send`.
        transcript.appendLocalUserMessage(text)
        // Base64 through the FFI, which decodes it into the protocol's bytes.
        // The picture travels WITH the prompt; there is no path, because a path
        // from a phone means nothing on the host.
        var args: [String: Any] = ["terminal": terminal, "text": text]
        if !images.isEmpty {
            args["images"] = images.map {
                ["mime": $0.mime, "base64": $0.data.base64EncodedString()]
            }
        }
        do {
            _ = try await core.call("terminal.agent_prompt", args)
            sendFailure = nil
        } catch {
            // Said out loud, and retryable.
            //
            // This was `try?`. The message had already been echoed into the
            // transcript by the line above, so a send that failed looked
            // exactly like a send that worked — your words on screen and an
            // agent that never answered. That is the whole of "the message
            // appears in the chat but nothing happens", and it is worse with a
            // picture attached, where a multi-megabyte payload is the most
            // likely thing to fail.
            sendFailure = SendFailure(message: Self.message(for: error)) { [weak self] in
                await self?.send(text, images: images)
            }
        }
    }

    /// A message that did not go, and the way to try it again.
    ///
    /// Deliberately NOT folded into `connectionError`: that one is cleared by
    /// every successful poll, which for a failed send would mean the warning
    /// disappearing a second or two after it appeared, while the undelivered
    /// message stayed on screen looking sent.
    struct SendFailure: Identifiable {
        let id = UUID()
        let message: String
        let retry: () async -> Void
    }

    @Published var sendFailure: SendFailure?

    /// The core's answer, as something worth putting on a phone screen.
    private static func message(for error: Error) -> String {
        let text = error.localizedDescription.lowercased()
        if text.contains("too large") || text.contains("payload") {
            return "That was too large to send. Try a smaller image."
        }
        if text.contains("not found") {
            return "That agent isn't running anymore."
        }
        return "Couldn't reach this runner. Your message wasn't sent."
    }

    /// Rewrite a message that has not gone out yet.
    func editQueued(_ id: String, _ text: String) async {
        _ = try? await core.call(
            "terminal.agent_edit_queued",
            ["terminal": terminal, "queuedId": id, "text": text])
    }

    /// Send a queued message into the turn already running.
    func steerQueued(_ id: String) async {
        _ = try? await core.call(
            "terminal.agent_steer_queued", ["terminal": terminal, "queuedId": id])
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
        // Shown before it is confirmed, exactly as on the Mac.
        //
        // The adapter applies `session/set_config_option` and says nothing back,
        // so a picker that waited for an echo snapped to its old value and read
        // as a control that does nothing. That was found and fixed on the Mac
        // and never ported; the phone has had the bug ever since.
        transcript.selectConfigOptionLocally(id: id, value: value)
        _ = try? await core.call(
            "terminal.agent_set_config", ["terminal": terminal, "configId": id, "value": value])
    }

    func answer(_ requestID: String, _ optionID: String) async {
        // Taken down on tap, not on an echo. The agent resumes without
        // acknowledging the request it was blocked on, so a card that waited
        // for confirmation sat there after the work it gated had happened —
        // the same fix the Mac already carries.
        transcript.clearPendingPermission()
        // And down on every OTHER surface with it. A lock screen card offering
        // an answer to a permission this pane just answered is the duplicate
        // this whole seam is careful about, and this is the moment the phone
        // knows it is gone.
        recordForGlances()
        _ = try? await core.call(
            "terminal.agent_answer",
            ["terminal": terminal, "requestId": requestID, "optionId": optionID])
    }

    /// Write what this agent is waiting on into the App Group, for the surfaces
    /// that cannot ask.
    ///
    /// A widget extension has no connection and no way to reach a runner, so
    /// the option names it would put on a button exist for it only if something
    /// with a connection writes them down. This pane is one of the two places
    /// that ever holds them — `WatchLinkHost.pendingPermission` is the other —
    /// and it holds them already, folded, on every poll.
    ///
    /// **Only on a change.** `pump` runs every 700ms, and a file written twenty
    /// times a minute per open pane would be a shared container rewritten for
    /// nothing: a permission arrives once and stands until it is answered. The
    /// id is enough to tell them apart — an agent does not reissue one — and
    /// comparing ids rather than whole permissions keeps this a string
    /// comparison on a poll path.
    ///
    /// `nil` is written as deliberately as a permission is. A pane that has
    /// just watched a permission go is the best-informed thing on the phone
    /// about that fact, and a card left offering answers to a question that is
    /// over is exactly what `GlancePermissions` exists to prevent.
    private func recordForGlances() {
        let pending = transcript.pendingPermission
        guard pending?.id != recordedRequest else { return }
        recordedRequest = pending?.id
        GlancePermissionStore.update {
            $0.recording(
                pending.map { permission in
                    GlancePermission(
                        terminal: terminal,
                        request: permission.id,
                        options: permission.options.map {
                            GlancePermissionOption(id: $0.id, name: $0.name, kind: $0.kind)
                        },
                        observedAt: Date())
                },
                for: terminal)
        }
    }

    /// The id this pane last filed, so an unchanged permission is not refiled.
    private var recordedRequest: String?

    func setMode(_ mode: String) async {
        _ = try? await core.call("terminal.agent_set_mode", ["terminal": terminal, "mode": mode])
    }

    func cancel() async {
        _ = try? await core.call("terminal.agent_cancel", ["terminal": terminal])
    }
}
