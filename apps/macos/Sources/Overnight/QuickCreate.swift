import SwiftUI

/// Start a task by describing it.
///
/// The old flow was: open a sheet, pick a repository, type a name, type a
/// branch, press Create, wait, find the new worktree, make a terminal, choose
/// an agent, wait for it to boot, then finally type what you actually wanted.
/// Nine interactions before the first useful word, which is long enough to lose
/// the thought that started it.
///
/// Everything in that list except the last item is derivable. The description
/// IS the task name, the branch is a slug of it, the project is the one you
/// were last in, and the agent is a preference. So there is one field, and what
/// you type in it becomes the agent's first message.
///
/// It stays open after submitting, because the stated need is several tasks in
/// quick succession and a panel that closes on every one turns a burst into a
/// sequence of separate decisions.
struct QuickCreate: View {
    let projects: [Repository]
    @Binding var project: String
    /// Description, project id. Returns once the work is queued, not finished.
    let onSubmit: (String, String) -> Void
    let onClose: () -> Void

    @State private var text = ""
    @State private var justCreated: String?
    @FocusState private var focused: Bool

    private var chosen: Repository? {
        projects.first { $0.id == project } ?? projects.first
    }

    private var branch: String { Branch.slug(from: text) }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 10) {
                Image(systemName: "sparkle")
                    .font(.system(size: 13))
                    .foregroundStyle(.tertiary)

                TextField("What do you want done?", text: $text)
                    .textFieldStyle(.plain)
                    .font(.system(size: 15))
                    .focused($focused)
                    .onSubmit(submit)

                if projects.count > 1 {
                    Picker("", selection: $project) {
                        ForEach(projects) { Text($0.displayName).tag($0.id) }
                    }
                    .labelsHidden()
                    .fixedSize()
                    .controlSize(.small)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 11)

            Divider()

            // What pressing Enter will do, spelled out. A form that derives
            // things silently is one you have to run to find out about.
            HStack(spacing: 6) {
                if let created = justCreated {
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(.green)
                    Text("Started \(created)")
                    Spacer()
                    Text("Type another")
                        .foregroundStyle(.tertiary)
                } else if text.isEmpty {
                    Text("A worktree, a branch and an agent — from one sentence.")
                        .foregroundStyle(.tertiary)
                    Spacer()
                } else {
                    Image(systemName: "arrow.triangle.branch").foregroundStyle(.tertiary)
                    Text(branch).foregroundStyle(.secondary)
                    Spacer()
                    Text("↩ to start").foregroundStyle(.tertiary)
                }
            }
            .font(.system(size: 11))
            .padding(.horizontal, 14)
            .padding(.vertical, 7)
        }
        .background(.regularMaterial)
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .overlay(
            RoundedRectangle(cornerRadius: 10).strokeBorder(Color.primary.opacity(0.08))
        )
        .shadow(color: .black.opacity(0.18), radius: 20, y: 8)
        .frame(width: 520)
        .onAppear { focused = true }
        .onExitCommand(perform: onClose)
    }

    private func submit() {
        let description = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !description.isEmpty, let chosen else { return }
        onSubmit(description, chosen.id)

        // Cleared, not closed. The next task is usually seconds behind the
        // first, and closing would make each one a separate decision to start.
        justCreated = Branch.title(from: description)
        text = ""
        focused = true
    }
}

/// Turning a sentence into a branch name.
enum Branch {
    /// A git-safe slug.
    ///
    /// Conservative on purpose: git accepts far more than this, but a branch
    /// name is something people type, paste into a PR title and see in a CI
    /// log, and one carrying punctuation from a sentence is a small tax paid
    /// repeatedly.
    static func slug(from text: String) -> String {
        let lowered = text.lowercased()
        var out = ""
        var lastWasDash = true  // leading dashes are dropped

        for character in lowered {
            if character.isLetter || character.isNumber {
                out.append(character)
                lastWasDash = false
            } else if !lastWasDash {
                out.append("-")
                lastWasDash = true
            }
            // Long enough to stay readable, short enough for a terminal title.
            if out.count >= 48 { break }
        }
        while out.hasSuffix("-") { out.removeLast() }
        return out.isEmpty ? "task" : out
    }

    /// A short human title, for a sidebar row.
    static func title(from text: String) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count > 42 else { return trimmed }
        // Cut on a word boundary rather than mid-word.
        let cut = trimmed.prefix(42)
        if let space = cut.lastIndex(of: " ") {
            return String(cut[..<space]) + "…"
        }
        return String(cut) + "…"
    }
}
