import SwiftUI

/// Ask the Orchestrator on a task's screen (ov-241): the one thing a task
/// offers for changing it, as the Mac has it.
///
/// It leaves "About ov-241 (“Title”): " in the orchestrator's composer, or has
/// the runner paste it into a terminal orchestrator, and goes there; the
/// person finishes the sentence. It never starts an orchestrator. With none
/// running it stays here, off, and says what to do. The rules are
/// `AskAboutTask`'s, in AgentKit, where they are tested.
struct AskOrchestratorSection: View {
    @ObservedObject var connection: Connection
    let place: PhoneWorkspace
    let row: TaskRow

    @Environment(\.phoneNavigator) private var navigator
    /// What the last ask left on the clipboard, when the runner couldn't paste.
    @State private var notice: String?
    @State private var asking = false

    /// The workspace's orchestrator, if it has one that isn't dead.
    private var seat: Terminal? {
        guard let summary = connection.workspace(place.workspace), !summary.isImplicit,
            WorkspaceScreen.orchestratorIsUp(in: connection, summary: summary)
        else { return nil }
        return OrchestratorSegment.terminal(in: connection, summary: summary)
    }

    var body: some View {
        let seat = seat
        Section {
            Button {
                if let seat { ask(seat) }
            } label: {
                Label(AskAboutTask.title, systemImage: "text.bubble")
                    // Grey as a whole when off: the icon stayed blue beside a
                    // grey title.
                    .foregroundStyle(seat == nil ? Color.secondary : Color.accentColor)
            }
            .disabled(seat == nil || asking)
            .accessibilityIdentifier("ask-orchestrator")
        } footer: {
            if seat == nil {
                Text(AskAboutTask.unavailable)
                    .accessibilityIdentifier("ask-orchestrator-unavailable")
            } else if let notice {
                Text(notice)
                    .accessibilityIdentifier("ask-orchestrator-notice")
            }
        }
    }

    private func ask(_ seat: Terminal) {
        asking = true
        notice = nil
        Task { @MainActor in
            let delivery = await AskAboutTask.deliver(
                key: row.key, title: row.title, isAgentPane: seat.isAgentPane,
                offer: { connection.composerOffers.offer($0, to: seat.id) },
                paste: { await connection.draftPrompt(terminal: seat.id, text: $0) },
                copy: { UIPasteboard.general.string = $0 })
            asking = false
            // A paste that may have landed is not copied as well, and needs no notice.
            if delivery == .copied {
                // Stays here: the reference is on the clipboard and the notice
                // says so, where a person pasting it can read it.
                notice = AskAboutTask.copiedNotice(key: row.key)
                return
            }
            guard let home = connection.fleet.worktrees.first(where: {
                $0.terminals.contains { $0.id == seat.id }
            }) else { return }
            navigator?.open(
                .worktree(runner: place.runner, worktree: home.id, landing: .terminal(seat.id)))
        }
    }
}

extension Connection {
    /// Ask the runner to paste `text` into a terminal orchestrator's box,
    /// pressing no Enter. `.declined` means nothing was typed; `.unknown` that
    /// no answer came in time, so it may have been; `.held` that a dialog was
    /// up and the runner pastes it once the dialog closes (ov-385), which the
    /// pane's `HeldDraftBar` then says.
    func draftPrompt(terminal: String, text: String) async -> AskAboutTask.DraftResult {
        do {
            let answer = try await rpc("terminal.draft_prompt", ["terminal": terminal, "text": text])
            let result = HeldDraft.result(of: answer)
            // The pane says it waits: read the hold now rather than at the
            // next poll.
            if case .held = result { await refresh() }
            return result
        } catch {
            var lost = false
            var notSent = false
            if let core = error as? ClientCore.CoreError, case let .disconnected(_, never) = core {
                lost = true
                notSent = never
            }
            return .failed(
                word: ClientCore.refusalWord(of: error), disconnected: lost, notSent: notSent)
        }
    }
}
