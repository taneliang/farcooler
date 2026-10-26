import AgentKit
import SwiftUI

/// Show Charter for a runner on another machine (ov-755): the charter's text
/// in a sheet. The charter is the team's rules, and every member reads it;
/// this Mac can't open the file, so the runner sends the text.
///
/// The owner, the workspace's lead and a Maintainer can also edit it here
/// (ov-784): Edit turns the page into a text editor, and Save sends the
/// whole charter back with the hash it was read at, so an edit made
/// meanwhile is never written over. Everyone else reads it.
struct CharterSheetRequest: Identifiable {
    let host: String
    let workspace: WorkspaceSummary
    /// Whether this Mac may edit it (`DaemonClient.editsCharter`).
    var editable = false
    var id: String { "\(host)|\(workspace.id)" }
}

struct CharterSheet: View {
    let workspace: WorkspaceSummary
    /// Asks the runner for the text.
    let load: () async -> CharterReading
    /// Sends an edit, made to the charter with the given hash: the charter as
    /// now written, or why not. Nil for a reader, who gets no Edit.
    var save: ((_ text: String, _ expected: String) async -> CharterReading)?
    /// Opens straight into an edit of this text: for a specimen.
    var editing: String?
    @State private var reading: CharterReading?
    /// The text being edited, while editing.
    @State private var draft: String?
    @State private var saving = false
    /// Why the last save didn't land.
    @State private var failure: String?
    @Environment(\.dismiss) private var dismiss

    /// Who may change a charter, said under the title so a reader knows whom
    /// to ask.
    static let whoEdits = "The runner’s owner, this workspace’s lead and Maintainers can edit its charter."

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text("\(workspace.name) Charter").font(.headline)
                    .lineLimit(2)
                Text(Self.whoEdits)
                    .font(.caption)
                    .foregroundStyle(.quietInk)
                    .fixedSize(horizontal: false, vertical: true)
            }
            content
            if let failure {
                Text(failure)
                    .font(.callout)
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("charter-failure")
            }
            buttons
        }
        .padding(20)
        .frame(minWidth: 320, idealWidth: 560, maxWidth: .infinity, minHeight: 240, idealHeight: 520, maxHeight: .infinity)
        // Escape leaves an edit, or closes the sheet, as Cancel and Done do.
        .onExitCommand {
            if draft != nil { cancelEdit() } else { dismiss() }
        }
        .task {
            reading = await load()
            if let editing { draft = editing }
        }
    }

    @ViewBuilder
    private var content: some View {
        if let draft {
            TextEditor(text: Binding(get: { draft }, set: { self.draft = $0 }))
                .font(.system(.callout, design: .monospaced))
                .scrollContentBackground(.hidden)
                .padding(6)
                .surface(.inset, in: .control)
                .disabled(saving)
                .accessibilityIdentifier("charter-editor")
        } else {
            switch reading {
            case nil:
                ProgressView().controlSize(.small).frame(maxWidth: .infinity, maxHeight: .infinity)
            case .text(let text, _)?:
                Self.page(text)
            case .missing?:
                Self.note(
                    save == nil
                        ? "No charter yet. The orchestrator writes one when the lead first talks to it."
                        : "No charter yet. The orchestrator writes one when the lead first talks to it, or you can write it now.")
            case .failed(let why)?:
                Self.note(why)
            }
        }
    }

    @ViewBuilder
    private var buttons: some View {
        HStack {
            Spacer()
            if draft != nil {
                Button("Cancel", action: cancelEdit)
                    .disabled(saving)
                Button(saving ? "Saving…" : "Save") { Task { await commit() } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(!Self.canSave(draft: draft, was: reading) || saving)
                    .accessibilityIdentifier("charter-save")
            } else {
                if save != nil, Self.canEdit(reading) {
                    Button("Edit") { beginEdit() }
                        .accessibilityIdentifier("charter-edit")
                }
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
            }
        }
    }

    /// Whether Edit is offered for what's showing: a charter, or none yet.
    /// Not while it's read, nor when it couldn't be, which has no hash to
    /// make an edit to.
    static func canEdit(_ reading: CharterReading?) -> Bool {
        switch reading {
        case .text?, .missing?: return true
        case .failed?, nil: return false
        }
    }

    /// Whether Save is offered: something written, and changed from what was
    /// read.
    static func canSave(draft: String?, was reading: CharterReading?) -> Bool {
        guard let draft, !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
        if case .text(let text, _)? = reading { return draft != text }
        return true
    }

    private func beginEdit() {
        failure = nil
        if case .text(let text, _)? = reading { draft = text } else { draft = "" }
    }

    private func cancelEdit() {
        draft = nil
        failure = nil
    }

    private func commit() async {
        guard let save, let draft, let reading else { return }
        saving = true
        defer { saving = false }
        let written = await save(draft, reading.sha256)
        if case .failed(let why) = written {
            failure = why
            return
        }
        self.reading = written
        self.draft = nil
        failure = nil
    }

    /// The charter, scrolling when it is long, and selectable.
    static func page(_ text: String) -> some View {
        ScrollView {
            Text(text)
                .font(.system(.callout, design: .monospaced))
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(10)
        }
        .surface(.inset, in: .control)
        .accessibilityIdentifier("charter-text")
    }

    static func note(_ sentence: String) -> some View {
        Text(sentence)
            .font(.callout)
            .foregroundStyle(.quietInk)
            .multilineTextAlignment(.leading)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
}
