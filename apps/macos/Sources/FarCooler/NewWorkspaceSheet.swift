import AgentKit
import SwiftUI

/// New Workspace…: a name, an optional task prefix, and which repository,
/// on a runner with workspaces.
struct NewWorkspaceSheet: View {
    /// Repositories on runners that can make workspaces.
    let repositories: [(host: String, repository: Repository)]
    let name: String
    /// Make it; the refusal to show, or nil when it was made.
    let onCreate: (_ host: String, _ repository: String, _ name: String, _ prefix: String) async -> String?

    @Environment(\.dismiss) private var dismiss
    @State private var typed = ""
    @State private var prefix = ""
    @State private var chosen = 0
    @State private var working = false
    @State private var refusal: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("New Workspace").font(.title3.weight(.semibold))
            Text("A workspace has its own board, task prefix and orchestrator.")
                .font(.callout)
                .foregroundStyle(.secondary)
            Form {
                if repositories.count > 1 {
                    Picker("Repository", selection: $chosen) {
                        ForEach(repositories.indices, id: \.self) { i in
                            Text(label(repositories[i])).tag(i)
                        }
                    }
                }
                TextField("Name", text: $typed)
                TextField("Task prefix", text: $prefix, prompt: Text("Optional, like bil"))
            }
            if let refusal {
                Text(refusal).font(.callout).foregroundStyle(.orange)
            }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Create Workspace") { create() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(working || trimmed.isEmpty || repositories.isEmpty)
            }
        }
        .padding(20)
        .frame(width: 420)
        .onAppear { typed = name }
    }

    private var trimmed: String { typed.trimmingCharacters(in: .whitespacesAndNewlines) }

    private func label(_ entry: (host: String, repository: Repository)) -> String {
        entry.host.isEmpty ? entry.repository.displayName : "\(entry.repository.displayName) · \(entry.host)"
    }

    private func create() {
        guard repositories.indices.contains(chosen) else { return }
        let target = repositories[chosen]
        working = true
        Task {
            refusal = await onCreate(
                target.host, target.repository.id, trimmed, prefix.trimmingCharacters(in: .whitespaces))
            working = false
            if refusal == nil { dismiss() }
        }
    }
}
