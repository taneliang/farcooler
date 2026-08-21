import SwiftUI

/// One agent's adapter, with a button that proves the launch half works.
///
/// Grouped Launch and Detection, and that split is the most important thing
/// here: Test starts the adapter and completes an ACP handshake, so it proves
/// Launch. It cannot prove Detection — those strings are matched against output
/// only that agent produces, and a wrong one does not fail, it stops the agent
/// being recognized, which shows up later as "chat mode broke" somewhere else.
struct AdapterEditorView: View {
    @State private var draft: AdapterInfo
    @State private var argsText: String
    @State private var envText: String
    @State private var commandsText: String
    @State private var identityText: String
    @State private var blockedText: String
    @State private var workingText: String
    @State private var testing = false
    @State private var outcome: AdapterTestOutcome?

    /// Whether this is being created rather than edited.
    ///
    /// The name keys the table in the file and is matched against a pane's
    /// process, so changing it on something that exists would write a SECOND
    /// adapter and leave the first in place.
    private let isNew: Bool
    private let test: (AdapterInfo) async -> AdapterTestOutcome
    private let onSave: (AdapterInfo) -> Void
    @Environment(\.dismiss) private var dismiss

    init(
        adapter: AdapterInfo,
        isNew: Bool,
        test: @escaping (AdapterInfo) async -> AdapterTestOutcome,
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
        self.isNew = isNew
        self.test = test
        self.onSave = onSave
    }

    var body: some View {
        Form {
            if draft.origin == .builtIn {
                Section {
                    Label(
                        "Saving this writes an override on that runner. You can revert to the "
                        + "shipped one at any time.",
                        systemImage: "info.circle")
                        .font(.callout)
                }
            }

            Section {
                TextField("Name", text: $draft.preset)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .disabled(!isNew)
            } header: {
                Text("Agent")
            } footer: {
                if !isNew {
                    Text("The agent name is used in its configuration and can’t be changed later.")
                }
            }

            Section {
                TextField("Program", text: $draft.program, prompt: Text("npx"))
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                lines("Arguments", $argsText)
                lines("Environment", $envText)
            } header: {
                Text("Launch")
            } footer: {
                Text("Enter one argument or KEY=value environment variable per line. Test checks these settings.")
            }

            Section {
                lines("Process names", $commandsText)
                lines("Identity", $identityText)
                lines("Waiting for you", $blockedText)
                lines("Working", $workingText)
            } header: {
                Text("Detection")
            } footer: {
                Text(
                    "Far Cooler uses these values to recognize an agent and determine its status. "
                    + "Test doesn’t check them."
                )
            }

            Section {
                Button {
                    Task {
                        testing = true
                        outcome = await test(assembled)
                        testing = false
                    }
                } label: {
                    HStack {
                        Text(testing ? "Testing…" : "Test")
                        if testing {
                            Spacer()
                            ProgressView()
                        }
                    }
                }
                .disabled(testing || assembled.program.trimmingCharacters(in: .whitespaces).isEmpty)

                if let outcome {
                    switch outcome {
                    case .worked(let reported):
                        Label("Starts and speaks ACP — \(reported)", systemImage: "checkmark.circle.fill")
                            .foregroundStyle(.green)
                            .font(.callout)
                    case .failed(let why):
                        Label(why, systemImage: "xmark.circle.fill")
                            .foregroundStyle(.red)
                            .font(.callout)
                    }
                }
            }
        }
        .navigationTitle(isNew ? "New Agent" : draft.preset)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Save") {
                    onSave(assembled)
                    dismiss()
                }
                .disabled(!canSave)
            }
        }
    }

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
            Self.split(envText).compactMap { line -> (String, String)? in
                // The FIRST `=` only: a value can contain them, and a token or
                // a URL routinely does.
                guard let at = line.firstIndex(of: "=") else { return nil }
                let key = String(line[line.startIndex..<at]).trimmingCharacters(in: .whitespaces)
                guard !key.isEmpty else { return nil }
                return (key, String(line[line.index(after: at)...]))
            },
            uniquingKeysWith: { _, last in last })
        out.commands = Self.split(commandsText)
        out.identity = Self.split(identityText)
        out.blocked = Self.split(blockedText)
        out.working = Self.split(workingText)
        return out
    }

    /// One entry per line, blank lines dropped.
    ///
    /// Deliberately not trimmed further: a detection string can legitimately
    /// begin or end with a space — several built-ins do, because that is what
    /// the agent actually draws — and trimming would break the ones that matter.
    static func split(_ text: String) -> [String] {
        text.split(separator: "\n", omittingEmptySubsequences: false)
            .map(String.init)
            .filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
    }

    private func lines(_ label: String, _ text: Binding<String>) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label).font(.footnote).foregroundStyle(.secondary)
            TextEditor(text: text)
                .font(.system(size: 13, design: .monospaced))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .frame(minHeight: 60)
        }
    }
}
