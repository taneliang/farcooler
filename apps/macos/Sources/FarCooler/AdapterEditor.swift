import SwiftUI

/// One agent's adapter, with a button that proves the launch half works.
///
/// The fields are grouped **Launch** and **Detection**, and that split is the
/// most important thing in this sheet. Test starts the adapter and completes an
/// ACP handshake, so it proves Launch. It cannot prove Detection: those strings
/// are matched against output only that agent produces, and getting one wrong
/// does not fail loudly — it stops the agent being recognized, which surfaces
/// later as "chat mode broke" from somewhere else entirely.
///
/// So the sheet says which half it can vouch for rather than showing one green
/// checkmark over seven fields.
struct AdapterEditor: View {
    @State private var draft: AdapterInfo
    /// Text the arrays are edited as, because a `[String]` has no editor and one
    /// line per entry is how these read in the config file anyway.
    @State private var argsText: String
    @State private var envText: String
    @State private var commandsText: String
    @State private var identityText: String
    @State private var blockedText: String
    @State private var workingText: String

    @State private var testing = false
    @State private var outcome: AdapterTestOutcome?

    private let store: RunnerSettingsStore
    /// Whether this is being created rather than edited.
    ///
    /// The name is the table's key in the file and what a pane's process is
    /// matched against, so changing it on something that exists would write a
    /// SECOND adapter and leave the first one in place. Editable while it is new;
    /// fixed afterwards.
    private let isNew: Bool
    private let onSave: (AdapterInfo) -> Void
    @Environment(\.dismiss) private var dismiss

    init(
        adapter: AdapterInfo, store: RunnerSettingsStore, isNew: Bool,
        onSave: @escaping (AdapterInfo) -> Void
    ) {
        _draft = State(initialValue: adapter)
        _argsText = State(initialValue: adapter.args.joined(separator: "\n"))
        _envText = State(
            initialValue: adapter.env.sorted { $0.key < $1.key }
                .map { "\($0.key)=\($0.value)" }
                .joined(separator: "\n"))
        _commandsText = State(initialValue: adapter.commands.joined(separator: "\n"))
        _identityText = State(initialValue: adapter.identity.joined(separator: "\n"))
        _blockedText = State(initialValue: adapter.blocked.joined(separator: "\n"))
        _workingText = State(initialValue: adapter.working.joined(separator: "\n"))
        self.store = store
        self.isNew = isNew
        self.onSave = onSave
    }

    var body: some View {
        VStack(spacing: 0) {
            Form {
                if draft.origin == .builtIn {
                    Section {
                        Label(
                            "Saving this writes an override on that runner. "
                            + "You can revert to the shipped one at any time.",
                            systemImage: "info.circle")
                            .font(.caption)
                    }
                }

                Section {
                    TextField("Name", text: $draft.preset)
                        .autocorrectionDisabled()
                        .disabled(!isNew)
                } header: {
                    Text("Agent")
                } footer: {
                    if !isNew {
                        Text(
                            "The name identifies this agent in the config file and is matched "
                            + "against the process in a pane, so it is fixed once it exists.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }

                launchSection
                detectionSection
            }
            .formStyle(.grouped)
            Divider()
            footer
        }
        .frame(width: 600, height: 680)
    }

    private var launchSection: some View {
        Section {
            // Only for the agents that have one. Offering a choice that cannot
            // work would be worse than not offering it — see `nativeIsAvailable`.
            if draft.nativeIsAvailable {
                Picker("Protocol", selection: $draft.backend) {
                    Text("ACP Adapter").tag(AdapterInfo.Backend.acp)
                    Text("Native").tag(AdapterInfo.Backend.native)
                }
            }
            TextField("Program", text: $draft.program, prompt: Text("npx"))
                .autocorrectionDisabled()
            lines("Arguments", $argsText, prompt: "-y\n@agentclientprotocol/my-agent-acp")
            lines("Environment", $envText, prompt: "MY_AGENT_API_KEY=…")
        } header: {
            Text("Launch")
        } footer: {
            VStack(alignment: .leading, spacing: 4) {
                if draft.nativeIsAvailable && draft.backend == .native {
                    // Said here rather than discovered later: under Native the
                    // arguments mean something different, and the protocol
                    // flags are not yours to set.
                    Text(
                        "Native speaks this agent's own protocol, with no adapter and no npx. "
                            + "Arguments are added after the flags the protocol needs."
                    )
                    // This used to read "Chat mode still runs the ACP adapter",
                    // which stopped being true once `start_backend` began
                    // dispatching on this field (`crates/cli/src/agent_host.rs`).
                    // A footer describing the opposite of what the toggle does
                    // is worse than no footer.
                    Text("Chat mode uses this after the daemon restarts.")
                        .foregroundStyle(.secondary)
                } else {
                    Text("One per line. Environment entries are KEY=value.")
                }
                Text("This is the half Test can prove.")
                    .foregroundStyle(.secondary)
            }
            .font(.caption)
        }
    }

    private var detectionSection: some View {
        Section {
            lines("Process names", $commandsText, prompt: "my-agent")
            lines("Identity", $identityText, prompt: "My Agent v")
            lines("Waiting for you", $blockedText, prompt: "Do you want to")
            lines("Working", $workingText, prompt: "esc to interrupt")
        } header: {
            Text("Detection")
        } footer: {
            VStack(alignment: .leading, spacing: 4) {
                Text(
                    "How Far Cooler recognizes this agent on a screen. Process names are matched "
                    + "as prefixes, because tmux truncates them; the rest are matched against the "
                    + "text the agent draws.")
                Text(
                    "Test cannot check these. A wrong value here does not fail — the agent simply "
                    + "stops being recognized, and notifications for it stop arriving.")
                    .foregroundStyle(.orange)
            }
            .font(.caption)
        }
    }

    private var footer: some View {
        HStack(spacing: 12) {
            Button(testing ? "Testing…" : "Test") {
                Task {
                    testing = true
                    outcome = await store.test(adapter: assembled)
                    testing = false
                }
            }
            .disabled(testing || assembled.program.trimmingCharacters(in: .whitespaces).isEmpty)

            if testing {
                ProgressView().controlSize(.small)
            } else if let outcome {
                switch outcome {
                case .worked(let reported):
                    Label("Starts and speaks ACP — \(reported)", systemImage: "checkmark.circle.fill")
                        .font(.caption)
                        .foregroundStyle(.green)
                        .lineLimit(2)
                case .failed(let why):
                    Label(why, systemImage: "xmark.circle.fill")
                        .font(.caption)
                        .foregroundStyle(.red)
                        .lineLimit(2)
                }
            }

            Spacer()
            Button("Cancel") { dismiss() }
            Button("Save") {
                onSave(assembled)
                dismiss()
            }
            .keyboardShortcut(.defaultAction)
            .disabled(!canSave)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
    }

    /// A blank program is refused here as well as by the daemon, because the
    /// daemon's refusal arrives as a banner and this one arrives as a disabled
    /// button next to the empty field.
    private var canSave: Bool {
        !draft.preset.trimmingCharacters(in: .whitespaces).isEmpty
            && !assembled.program.trimmingCharacters(in: .whitespaces).isEmpty
    }

    /// The draft plus whatever is in the text areas right now.
    private var assembled: AdapterInfo {
        var out = draft
        out.preset = draft.preset.trimmingCharacters(in: .whitespaces)
        out.program = draft.program.trimmingCharacters(in: .whitespaces)
        out.args = Self.split(argsText)
        out.env = Dictionary(
            uniqueKeysWithValues: Self.split(envText).compactMap { line -> (String, String)? in
                // Split on the FIRST `=` only: a value can contain them, and a
                // token or a URL routinely does.
                guard let at = line.firstIndex(of: "=") else { return nil }
                let key = String(line[line.startIndex..<at]).trimmingCharacters(in: .whitespaces)
                guard !key.isEmpty else { return nil }
                return (key, String(line[line.index(after: at)...]))
            })
        out.commands = Self.split(commandsText)
        out.identity = Self.split(identityText)
        out.blocked = Self.split(blockedText)
        out.working = Self.split(workingText)
        return out
    }

    /// One entry per line, blank lines dropped.
    ///
    /// Not trimmed beyond the newline: a detection string can legitimately begin
    /// or end with a space — several built-ins do, because that is what the agent
    /// actually draws — and trimming would break exactly the ones that matter.
    static func split(_ text: String) -> [String] {
        text.split(separator: "\n", omittingEmptySubsequences: false)
            .map(String.init)
            .filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
    }

    /// A labelled multi-line field.
    private func lines(_ label: String, _ text: Binding<String>, prompt: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label).font(.callout)
            TextEditor(text: text)
                .font(.system(size: 11, design: .monospaced))
                .frame(height: 54)
                .overlay(
                    RoundedRectangle(cornerRadius: 5)
                        .strokeBorder(Color.primary.opacity(0.12)))
                .overlay(alignment: .topLeading) {
                    if text.wrappedValue.isEmpty {
                        Text(prompt)
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundStyle(.tertiary)
                            .padding(.leading, 5)
                            .padding(.top, 3)
                            .allowsHitTesting(false)
                    }
                }
        }
    }
}
