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
/// cut short names the worktree, the branch is a slug of it, the project is the
/// one you were last in, and the agent is a preference. So there is one field,
/// and what you type in it becomes the agent's first message.
///
/// It stays open after submitting, because the stated need is several tasks in
/// quick succession and a panel that closes on every one turns a burst into a
/// sequence of separate decisions.
struct QuickCreate: View {
    /// Every runner's repositories, tagged the same way `FleetStore.repositories`
    /// tags them. Carried together rather than flattened to a bare `[Repository]`
    /// so the picker below can name the runner, not just the project — see
    /// `NewWorkspaceSheet`, which tags the same way for the same reason.
    let projects: [(host: String, repository: Repository)]
    @Binding var project: String
    /// Description, host, project, preset. Returns once queued, not
    /// finished.
    ///
    /// `host` comes from `chosen` below, the same picker selection that
    /// resolved `project` — not re-derived by the caller from `project`
    /// alone. `NewWorkspaceSheet.Choice` carries host and repository
    /// together for exactly this reason: a repository chosen without its
    /// host, handed to whatever runner happens to be "current" downstream,
    /// is how a task starts on the wrong one with no error at all.
    let onSubmit: (String, String, String, String) -> Void
    let onResume: () -> Void
    let onClose: () -> Void
    /// What the CHOSEN runner says branch names start with.
    ///
    /// Per runner rather than one setting for the app: the branch is created on
    /// the runner holding the project, and that runner's convention is the one
    /// that matters. Looked up by the caller from `chosen`'s host for the same
    /// reason `onSubmit` carries the host — a repository resolved without it is
    /// how work starts on the wrong runner with no error at all.
    var branchPrefix: (String) -> String = { _ in "" }

    /// The draft survives closing the panel.
    ///
    /// A long prompt is often written in two sittings — you start one, go and
    /// look at something, come back. Losing it on Esc taught people not to
    /// close the panel, which is worse than the panel being open.
    @AppStorage("tasks.draft") private var text = ""
    @AppStorage("tasks.agent") private var agent = "claude"
    @AppStorage("tasks.model") private var model = ""

    @State private var justCreated: String?

    /// The project `project` names, or nil if it names nothing any runner
    /// currently has.
    ///
    /// No fallback to `projects.first`. `project` is `tasks.lastProject`,
    /// persisted across launches — if the repository it names was removed
    /// (or the whole runner it lived on was), the picker below renders with
    /// no row selected, and this being nil is what keeps `⏎` from starting a
    /// task on whatever happened to be first in the list instead: a fallback
    /// here would be a runner picked with nothing on screen saying so, which
    /// is the exact failure this project exists to remove.
    private var chosen: (host: String, repository: Repository)? {
        projects.first { $0.repository.id == project }
    }

    /// Whether more than one runner has a repository on offer — the picker
    /// names the runner alongside the repository only when that distinction
    /// is real, same rule `NewWorkspaceSheet` follows.
    private var multipleHosts: Bool {
        Set(projects.map(\.host)).count > 1
    }

    private func label(for entry: (host: String, repository: Repository)) -> String {
        guard multipleHosts else { return entry.repository.displayName }
        let host = entry.host.isEmpty ? "This Mac" : entry.host
        return "\(entry.repository.displayName) — \(host)"
    }

    private var branch: String {
        Branch.slug(from: text, prefix: branchPrefix(chosen?.host ?? ""))
    }

    /// The name this task will carry, derived the same way `startTask` derives
    /// the one it sends — a description cut to a title. It is a directory now
    /// rather than a stored string, so what this becomes is what the sidebar
    /// row says for as long as the worktree exists.
    private var name: String {
        Branch.title(from: text)
    }

    /// The worktree about to be created, which is the only thing naming this
    /// task. Nobody typed it, so it is shown rather than left to be found.
    private var worktreePath: String? {
        guard let chosen, WorktreeName.isValid(name) else { return nil }
        return WorktreeName.path(repository: chosen.repository.displayName, name: name)
    }

    /// A description of nothing but punctuation is a description the daemon
    /// will refuse: it leaves no directory behind once slugged. `Branch.title`
    /// keeps this well under the sixty-scalar ceiling, but the ceiling is
    /// checked where the name is decided rather than assumed from a cut made
    /// somewhere else.
    private var canSubmit: Bool {
        chosen != nil && WorktreeName.isValid(name)
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
                // Both halves of what is about to be made, because neither was
                // typed: the directory is what this task will be called from
                // here on, the branch is where its commits go. The path yields
                // its width first — its tail is the name, and that is the end
                // worth keeping.
                if let worktreePath {
                    Image(systemName: "folder").foregroundStyle(.tertiary)
                    Text(worktreePath)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.head)
                        .layoutPriority(-1)
                }
                Image(systemName: "arrow.triangle.branch").foregroundStyle(.tertiary)
                Text(branch).foregroundStyle(.secondary).lineLimit(1)
            }

            Spacer(minLength: 10)

            // Shown whenever there is more than one project to confuse, or
            // whenever the persisted selection matches none of them — the
            // latter is what makes a stale `tasks.lastProject` a visible
            // "pick one" instead of a silent wrong runner (see `chosen`).
            if projects.count > 1 || chosen == nil {
                Picker("", selection: $project) {
                    ForEach(projects, id: \.repository.id) { entry in
                        Text(label(for: entry)).tag(entry.repository.id)
                    }
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
        // The same gate the footer shows, not a weaker one. ⏎ is the only way
        // in here, and a name the daemon refuses would clear the draft on its
        // way to a failure nothing on screen reports.
        guard canSubmit, let chosen else { return }
        onSubmit(
            description, chosen.host, chosen.repository.id,
            Agents.preset(agent: agent, model: model))

        justCreated = WorktreeName.display(name)
        // Cleared only on success, which is also what clears the draft.
        text = ""
    }
}

/// Turning a name into a worktree directory.
///
/// Beside `Branch` because they are the two halves of the same question, asked
/// of the same typed words. The rule below is the daemon's, mirrored: a
/// workspace has no stored name any more, so the directory a worktree is
/// created in IS its name, and a client offering to create one has to show
/// which directory that will be. A preview computed by a rule merely close to
/// the daemon's is a preview of a path that never gets created — the same trap
/// `Branch.slug` avoids by applying the prefix here rather than on the far side.
enum WorktreeName {
    /// What a directory may contain, spelled out rather than asked of
    /// `isLetter`, which answers yes for `é` where the daemon answers dash.
    private static let safe = Set(
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_".unicodeScalars)

    /// One path component: anything outside `[A-Za-z0-9_-]` becomes a dash,
    /// runs of dashes collapse, and the ends are trimmed.
    ///
    /// Case survives, unlike a branch slug. The directory is read back as the
    /// workspace's name, so `Rate Limiting` has to come out spelled the way it
    /// went in.
    static func slug(_ s: String) -> String {
        s.unicodeScalars
            .map { safe.contains($0) ? Character($0) : "-" }
            .reduce(into: "") { acc, c in
                if c == "-" && acc.hasSuffix("-") { return }
                acc.append(c)
            }
            .trimmingCharacters(in: CharacterSet(charactersIn: "-"))
    }

    /// Where a worktree lands, from `worktrees` down. The daemon owns
    /// everything above that and never tells a client what it is, but the tail
    /// is the part being named.
    static func path(repository: String, name: String) -> String {
        "worktrees/\(slug(repository))/\(slug(name))"
    }

    /// Whether the daemon will take this as a name: at most sixty scalars, and
    /// something left over once slugged. Checked here so a client cannot spend
    /// a round trip being told what it already knew — a name of pure
    /// punctuation is short enough and still has no directory to be.
    static func isValid(_ s: String) -> Bool {
        s.unicodeScalars.count <= 60 && !slug(s).isEmpty
    }

    /// What a typed name will read as once it has been through the filesystem
    /// and back. Every client shows the directory, not what was typed, so a
    /// client saying "started X" about a name it has not round-tripped is
    /// naming something the sidebar does not call by that name.
    static func display(_ name: String) -> String {
        slug(name)
            .replacingOccurrences(of: "-", with: " ")
            .replacingOccurrences(of: "_", with: " ")
    }
}

/// Turning a sentence into a branch name.
enum Branch {
    /// A git-safe slug, behind whatever the runner says branches start with.
    ///
    /// Conservative on purpose: git accepts far more than this, but a branch
    /// name is something people type, paste into a PR title and see in a CI
    /// log, and one carrying punctuation from a sentence is a small tax paid
    /// repeatedly.
    ///
    /// The prefix is applied HERE, on the client, rather than by the daemon —
    /// because the composer shows you the branch it is about to create, and a
    /// prefix added on the far side would make that preview a lie. The daemon
    /// still validates the finished name, which is the check that protects git.
    ///
    /// The 48-character budget is spent on the slug, not on the result: a long
    /// prefix must not be able to eat the part that says what the task was.
    static func slug(from text: String, prefix: String = "") -> String {
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
        return prefix + (out.isEmpty ? "task" : out)
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
