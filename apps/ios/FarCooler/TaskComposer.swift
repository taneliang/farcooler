import SwiftUI

/// Start a task by describing it — the phone's version of the Mac's ⌘N.
///
/// The Mac's `QuickCreate` exists because the old flow made you supply four
/// things — a name, a branch, an agent, a first message — when only the last
/// one is actually a decision; the rest are derivable from it. That argument
/// applies at least as strongly here: a phone's keyboard makes typing a name
/// AND a branch AND a message a worse tax than it is on a Mac, not a smaller
/// one. So this is the same shape: one sentence in, everything else derived
/// or remembered from last time.
///
/// Presented as a sheet rather than as a persistent panel, unlike the Mac,
/// because iOS has no equivalent of a floating command panel — a sheet is
/// the platform's version of "a thing you summon, use, and put away."
@MainActor
struct TaskComposerView: View {
    @ObservedObject var connection: Connection

    @Environment(\.dismiss) private var dismiss

    /// The draft survives dismissing the sheet.
    ///
    /// A long prompt is often written in two sittings — start it, get
    /// interrupted, come back — and losing it on a swipe-to-dismiss (which,
    /// on a phone, happens by accident far more often than Esc does on a Mac)
    /// would teach people not to close the sheet, which is worse.
    @AppStorage("quicktask.draft") private var text = ""
    @AppStorage("quicktask.project") private var project = ""
    @AppStorage("quicktask.agent") private var agentID = "claude"
    @AppStorage("quicktask.model") private var model = ""

    @State private var phase: Phase = .idle

    private enum Phase: Equatable {
        case idle
        case creatingWorkspace
        case startingAgent
        case sending
        case done(String)
        case failed(String)
    }

    private var isWorking: Bool {
        switch phase {
        case .creatingWorkspace, .startingAgent, .sending: return true
        case .idle, .done, .failed: return false
        }
    }

    private var chosenRepository: Repository? {
        connection.repositories.first { $0.id == project } ?? connection.repositories.first
    }

    private var canSubmit: Bool {
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && chosenRepository != nil && !isWorking
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    composer
                    if !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        HStack(spacing: 6) {
                            Image(systemName: "arrow.triangle.branch")
                                .foregroundStyle(.tertiary)
                            Text(TaskSlug.slug(from: text, prefix: connection.branchPrefix))
                                .font(.caption.monospaced())
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                    }
                }

                // Only shown with a choice to make. A picker with one option
                // is not a picker, it is a label that looks tappable — the
                // same reasoning as the Mac's `if projects.count > 1`.
                if connection.repositories.count > 1 {
                    Section("Project") {
                        Picker("Project", selection: $project) {
                            ForEach(connection.repositories) { repo in
                                Text(repo.displayName).tag(repo.id)
                            }
                        }
                        .disabled(isWorking)
                    }
                }

                Section("Agent") {
                    Picker("Agent", selection: $agentID) {
                        ForEach(QuickAgents.all) { Text($0.name).tag($0.id) }
                    }
                    .disabled(isWorking)
                    .onChange(of: agentID) { _, _ in model = "" }

                    Picker("Model", selection: $model) {
                        Text("Default").tag("")
                        ForEach(QuickAgents.agent(agentID).models, id: \.self) { Text($0).tag($0) }
                    }
                    .disabled(isWorking)
                }

                Section {
                    status
                    Button(action: submit) {
                        HStack {
                            Spacer()
                            if isWorking {
                                ProgressView().padding(.trailing, 6)
                            }
                            Text(isWorking ? "Starting…" : "Start")
                                .fontWeight(.semibold)
                            Spacer()
                        }
                    }
                    .disabled(!canSubmit)
                }
            }
            .navigationTitle("Quick Task")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
            .onAppear(perform: ensureProject)
            .onChange(of: connection.repositories) { _, _ in ensureProject() }
        }
    }

    /// A plain multi-line field. No return-to-submit here, unlike the Mac's
    /// `Composer` — iOS's return key is how you get a second line on this
    /// keyboard, every Notes-like app on the platform agrees, and overriding
    /// it would fight muscle memory for the sake of copying a Mac shortcut
    /// that does not exist on this device anyway. `Start` is the send action.
    private var composer: some View {
        ZStack(alignment: .topLeading) {
            if text.isEmpty {
                Text("What do you want done?")
                    .foregroundStyle(.tertiary)
                    .padding(.top, 8)
                    .padding(.leading, 5)
                    .allowsHitTesting(false)
            }
            TextEditor(text: $text)
                .frame(minHeight: 90, maxHeight: 180)
                .disabled(isWorking)
        }
    }

    @ViewBuilder
    private var status: some View {
        switch phase {
        case .idle:
            EmptyView()
        case .creatingWorkspace:
            ProgressRow("Creating worktree…")
        case .startingAgent:
            ProgressRow("Starting \(QuickAgents.agent(agentID).name)…")
        case .sending:
            ProgressRow("Sending the task…")
        case .done(let title):
            Label("Started \(title)", systemImage: "checkmark.circle.fill")
                .foregroundStyle(.green)
        case .failed(let message):
            Label(message, systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(.orange)
                .font(.callout)
        }
    }

    /// Default to the last-used project, falling back to whichever one the
    /// host offers first if that one is gone (removed, or this is the first
    /// run and nothing is remembered yet).
    private func ensureProject() {
        guard !connection.repositories.contains(where: { $0.id == project }) else { return }
        project = connection.repositories.first?.id ?? ""
    }

    /// Create the worktree, start the agent, wait for it, hand it the
    /// sentence. Each step's failure is reported at that step rather than
    /// folded into one generic error, because "could not create a worktree"
    /// and "the worktree exists but the agent never started" call for
    /// different next actions from whoever is reading this on a phone.
    private func submit() {
        let description = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !description.isEmpty, let repository = chosenRepository else { return }

        let branch = TaskSlug.slug(from: description, prefix: connection.branchPrefix)
        let title = TaskSlug.title(from: description)
        let agentName = QuickAgents.agent(agentID).name
        let preset = QuickAgents.preset(agent: agentID, model: model)

        Task {
            phase = .creatingWorkspace
            let workspaceID: String
            do {
                workspaceID = try await connection.createWorkspace(
                    repository: repository.id, task: title, branch: branch, base: "")
            } catch {
                phase = .failed("Could not create the worktree: \(error.localizedDescription)")
                return
            }
            await connection.refresh()

            phase = .startingAgent
            let terminalID: String
            do {
                terminalID = try await connection.createTerminal(
                    workspace: workspaceID, title: agentID, preset: preset)
            } catch {
                phase = .failed(
                    "Created the worktree, but could not start \(agentName): "
                    + error.localizedDescription)
                return
            }

            // Up to a minute, polling the same `activity` the fleet list
            // shows: a cold agent on a slow machine is not a failure, and
            // there is no fixed delay that is both short enough to feel quick
            // and long enough to never be wrong. This is the phone's version
            // of `DaemonClient.startTask`'s wait loop.
            var ready = false
            pollLoop: for _ in 0..<60 {
                try? await Task.sleep(for: .seconds(1))
                await connection.refresh()
                guard let current = connection.terminal(terminalID, in: workspaceID) else {
                    continue
                }
                switch current.agent {
                case .idle:
                    ready = true
                    break pollLoop
                // It asked something before we got a word in — a trust
                // prompt, or a resume dialog. Stop rather than typing a task
                // description into a question it hasn't finished asking.
                case .blocked:
                    phase = .failed(
                        "\(agentName) is waiting on a question. Open \(title) to answer it.")
                    return
                default:
                    continue
                }
            }
            guard ready else {
                // The Mac's `startTask` returns quietly here and never sends
                // the sentence if the agent is neither idle nor blocked after
                // a minute. Doing the same on a phone — closing having done
                // half the job with no explanation — is exactly what this
                // screen exists to avoid, so it says so instead.
                phase = .failed(
                    "\(agentName) was not ready within a minute. Nothing was sent — "
                    + "open \(title) to check on it.")
                return
            }

            phase = .sending
            do {
                try await connection.writeRaw(terminal: terminalID, hex: hexEncode(description))
                // The return as its own write, deliberately: an agent's
                // composer treats a newline that arrives in the same packet
                // as pasted text, not as submit. `DaemonClient.send` on the
                // Mac makes the same two calls for the same reason.
                try await connection.writeRaw(terminal: terminalID, hex: "0d")
            } catch {
                phase = .failed(
                    "Started \(agentName), but could not send the task: "
                    + error.localizedDescription)
                return
            }

            phase = .done(title)
            text = ""
        }
    }

    private func hexEncode(_ text: String) -> String {
        text.utf8.map { String(format: "%02x", $0) }.joined()
    }
}

private struct ProgressRow: View {
    let label: String
    init(_ label: String) { self.label = label }

    var body: some View {
        HStack(spacing: 8) {
            ProgressView().controlSize(.small)
            Text(label).foregroundStyle(.secondary)
        }
    }
}
