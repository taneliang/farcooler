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
    /// Description, project, preset. Returns once queued, not finished.
    let onSubmit: (String, String, String) -> Void
    let onResume: () -> Void
    let onClose: () -> Void

    /// The draft survives closing the panel.
    ///
    /// A long prompt is often written in two sittings — you start one, go and
    /// look at something, come back. Losing it on Esc taught people not to
    /// close the panel, which is worse than the panel being open.
    @AppStorage("tasks.draft") private var text = ""
    @AppStorage("tasks.agent") private var agent = "claude"
    @AppStorage("tasks.model") private var model = ""

    @State private var justCreated: String?

    private var chosen: Repository? {
        projects.first { $0.id == project } ?? projects.first
    }

    private var branch: String { Branch.slug(from: text) }
    private var canSubmit: Bool {
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && chosen != nil
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .top, spacing: 10) {
                Image(systemName: "sparkle")
                    .font(.system(size: 13))
                    .foregroundStyle(.tertiary)
                    .padding(.top, 4)

                Composer(
                    text: $text,
                    placeholder: "What do you want done?",
                    onSubmit: submit,
                    onCancel: onClose
                )
                .frame(height: composerHeight)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 11)

            Divider()
            footer
        }
        .background(.regularMaterial)
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Color.primary.opacity(0.08)))
        .shadow(color: .black.opacity(0.18), radius: 20, y: 8)
        .frame(width: 560)
    }

    /// Grows with the prompt, up to a point.
    private var composerHeight: CGFloat {
        let lines = text.reduce(1) { $1 == "\n" ? $0 + 1 : $0 }
        let wrapped = max(lines, (text.count / 62) + 1)
        return min(max(CGFloat(wrapped) * 19 + 6, 25), 220)
    }

    private var footer: some View {
        HStack(spacing: 8) {
            if let created = justCreated, text.isEmpty {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                Text("Started \(created)").foregroundStyle(.secondary)
            } else if text.isEmpty {
                Button(action: onResume) {
                    Label("Resume a branch", systemImage: "arrow.uturn.backward")
                        .font(.system(size: 11))
                }
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
            } else {
                Image(systemName: "arrow.triangle.branch").foregroundStyle(.tertiary)
                Text(branch).foregroundStyle(.secondary).lineLimit(1)
            }

            Spacer(minLength: 10)

            if projects.count > 1 {
                Picker("", selection: $project) {
                    ForEach(projects) { Text($0.displayName).tag($0.id) }
                }
                .labelsHidden().fixedSize().controlSize(.small)
            }

            Picker("", selection: $agent) {
                ForEach(Agents.all) { Text($0.name).tag($0.id) }
            }
            .labelsHidden().fixedSize().controlSize(.small)
            .onChange(of: agent) { _, _ in model = "" }

            Picker("", selection: $model) {
                Text("Default model").tag("")
                ForEach(Agents.agent(agent).models, id: \.self) { Text($0).tag($0) }
            }
            .labelsHidden().fixedSize().controlSize(.small)

            Text(canSubmit ? "↩ start" : "⇧↩ newline")
                .foregroundStyle(.tertiary)
        }
        .font(.system(size: 11))
        .padding(.horizontal, 14)
        .padding(.vertical, 7)
    }

    private func submit() {
        let description = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !description.isEmpty, let chosen else { return }
        onSubmit(description, chosen.id, Agents.preset(agent: agent, model: model))

        justCreated = Branch.title(from: description)
        // Cleared only on success, which is also what clears the draft.
        text = ""
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
