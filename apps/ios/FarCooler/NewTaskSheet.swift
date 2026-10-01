import SwiftUI

// New Task… on a workspace's board (ov-66, the owner's ruling 4): a title,
// which it needs, and details, which become the task's intent. Filed through
// the client core's `task.create` as the user, as the Mac's board and
// Android's sheet file one. Every rule and sentence is AgentKit's
// (`PhoneNewTask`); this file draws.

/// The New Task sheet. Only a filed task closes it: a refusal keeps what was
/// typed and says why under it.
struct NewTaskSheet: View {
    @ObservedObject var connection: Connection
    let workspace: WorkspaceSummary

    @State private var title = ""
    @State private var details = ""
    @State private var sending = false
    /// Why the last try didn't land, in this app's words.
    @State private var failure: String?

    @Environment(\.dismiss) private var dismiss
    @FocusState private var titleFocused: Bool

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    // Up to three lines, growing, rather than one that scrolls
                    // sideways and loses the start of what was typed. A vertical
                    // field takes Return as a newline, so the newline is turned
                    // back into the submit it was asked to be.
                    TextField("Title", text: $title, axis: .vertical)
                        .lineLimit(1...3)
                        .focused($titleFocused)
                        .onChange(of: title) { _, now in
                            guard now.contains("\n") else { return }
                            title = now.replacingOccurrences(of: "\n", with: "")
                            send()
                        }
                        .submitLabel(.done)
                        .onSubmit(send)
                        .accessibilityIdentifier("new-task-title")
                    if PhoneNewTask.isTooLong(title) {
                        Text(PhoneNewTask.tooLong)
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .accessibilityIdentifier("new-task-too-long")
                    }
                } header: {
                    // Where the task is going: nothing else in the sheet said.
                    Text("On \(workspace.name)")
                        .accessibilityIdentifier("new-task-workspace")
                }
                Section {
                    TextField("Details (optional)", text: $details, axis: .vertical)
                        .lineLimit(3...8)
                        .accessibilityIdentifier("new-task-details")
                }
                if let failure {
                    Section {
                        Label(failure, systemImage: "exclamationmark.triangle.fill")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                            .accessibilityIdentifier("new-task-failure")
                    }
                }
            }
            .disabled(sending)
            .navigationTitle("New Task")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                        .disabled(sending)
                }
                ToolbarItem(placement: .confirmationAction) {
                    if sending {
                        ProgressView()
                    } else {
                        Button("Add Task", action: send)
                            .disabled(!PhoneNewTask.titleFits(title))
                            .accessibilityIdentifier("new-task-add")
                    }
                }
            }
        }
        .interactiveDismissDisabled(sending)
        .presentationDetents([.medium, .large])
        .onAppear { titleFocused = true }
    }

    private func send() {
        guard !sending, PhoneNewTask.titleFits(title) else { return }
        sending = true
        failure = nil
        Task {
            let refused = await connection.createTask(on: workspace, title: title, details: details)
            sending = false
            if let refused {
                failure = refused
            } else {
                dismiss()
            }
        }
    }
}
