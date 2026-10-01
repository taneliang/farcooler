import AgentKit
import SwiftUI

/// A task prefix for a new workspace, when none was typed: the runner
/// requires one (`workspace create --prefix`).
enum WorkspacePrefix {
    /// The name's letters, lowercased, three of them where there are, and
    /// more, then a digit, until it's not one `taken` already: "Billing" is
    /// `bil`, and beside a `bil`, `bill`. A name with no letters is `ws`.
    /// Always a letter followed by at most seven letters or digits, which is
    /// what the runner takes.
    static func derive(name: String, taken: Set<String>) -> String {
        let letters = String(name.lowercased().unicodeScalars.filter { $0.isASCII && CharacterSet.lowercaseLetters.contains($0) }.map(Character.init))
        let base = letters.isEmpty ? "ws" : letters
        let used = Set(taken.map { $0.lowercased() })
        for length in 3...8 where length <= max(base.count, 3) {
            let candidate = String(base.prefix(length))
            if !used.contains(candidate) { return candidate }
        }
        let stem = String(base.prefix(6))
        for n in 2...99 {
            let candidate = "\(stem)\(n)"
            if candidate.count <= 8, !used.contains(candidate) { return candidate }
        }
        return String(stem.prefix(4)) + String(Int.random(in: 1000...9999))
    }
}

/// New Workspace…: a name, a task prefix (derived from the name when left
/// empty), and which repository, on a runner with workspaces.
struct NewWorkspaceSheet: View {
    /// Repositories on runners that can make workspaces.
    let repositories: [(host: String, repository: Repository)]
    let name: String
    /// The prefixes a runner's workspaces use already, by runner.
    var takenPrefixes: (_ host: String) -> Set<String> = { _ in [] }
    /// Make it; the refusal to show, or nil when it was made.
    let onCreate: (_ host: String, _ repository: String, _ name: String, _ prefix: String) async -> String?

    @Environment(\.dismiss) private var dismiss
    @State private var typed = ""
    @State private var prefix = ""
    @State private var chosen = 0
    @State private var working = false
    @State private var failure: SheetFailure?

    var body: some View {
        SheetFrame(
            title: "New Workspace",
            subtitle: "A workspace has its own board, task prefix, and orchestrator.",
            confirmTitle: "Create Workspace",
            canConfirm: !working && !trimmed.isEmpty && !repositories.isEmpty,
            working: working,
            failure: failure,
            onCancel: { dismiss() },
            onConfirm: { await create() }
        ) {
            Form {
                if repositories.count > 1 {
                    Picker("Repository", selection: $chosen) {
                        ForEach(repositories.indices, id: \.self) { i in
                            Text(label(repositories[i])).tag(i)
                        }
                    }
                }
                TextField("Name", text: $typed)
                TextField("Task prefix", text: $prefix, prompt: Text(derived))
                if prefix.trimmingCharacters(in: .whitespaces).isEmpty {
                    Text("Left empty, tasks start with “\(derived)-”, from the name.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            .formStyle(.grouped)
            .scrollDisabled(true)
        }
        .onAppear { typed = name }
    }

    private var trimmed: String { typed.trimmingCharacters(in: .whitespacesAndNewlines) }

    /// The prefix used when none is typed.
    private var derived: String {
        let host = repositories.indices.contains(chosen) ? repositories[chosen].host : ""
        return WorkspacePrefix.derive(name: trimmed, taken: takenPrefixes(host))
    }

    private func label(_ entry: (host: String, repository: Repository)) -> String {
        RepositoryChoice.label(entry, multipleHosts: Set(repositories.map(\.host)).count > 1)
    }

    private func create() async {
        guard repositories.indices.contains(chosen) else { return }
        let target = repositories[chosen]
        working = true
        failure = nil
        let typedPrefix = prefix.trimmingCharacters(in: .whitespaces)
        let refusal = await onCreate(
            target.host, target.repository.id, trimmed, typedPrefix.isEmpty ? derived : typedPrefix)
        working = false
        if let refusal {
            // The runner's words go in the transcript, under a sentence of
            // this app's own, rather than on screen as if Far Cooler wrote them.
            failure = SheetFailure(
                sentence: "Couldn’t create the workspace.", transcript: refusal)
        } else {
            dismiss()
        }
    }
}
