import SwiftUI

/// Start a task by describing it.
///
/// The old flow was: open a sheet, pick a repository, type a name, type a
/// branch, press Create, wait, find the new worktree, make a terminal, choose
/// an agent, wait for it to boot, then finally type what you actually wanted.
/// Nine interactions before the first useful word, which is long enough to lose
/// the thought that started it.
///
/// Everything in that list except the last item is derivable. A few words of
/// the description name the worktree (`TaskName`, asked of the on-device model
/// where there is one), the branch is a slug of that name, the project is the
/// one you were last in, and the agent is a preference. So there is one field,
/// and what you type in it becomes the agent's first message.
///
/// It closes on ⏎, and ⌥⏎ starts the task and keeps it open. It used to stay
/// open after every submit, for bursts of several tasks in quick succession
/// (242ec102); in use, the one-task case is the common one, and a panel left
/// on top of the task you just started is in the way of looking at it. The
/// burst is still one key away.
///
/// **Closing waits for the task to exist.** The draft is kept, and the panel
/// stays, until the agent's terminal has been made; a start that fails leaves
/// both, with a sentence saying why. Before, the panel cleared the draft the
/// moment ⏎ was pressed, and a start that then failed took the task with it
/// and said nothing.
struct QuickCreate: View {
    /// Every runner's repositories, tagged the same way `FleetStore.repositories`
    /// tags them. Carried together rather than flattened to a bare `[Repository]`
    /// so the picker below can name the runner, not just the project — see
    /// `NewWorkspaceSheet`, which tags the same way for the same reason.
    let projects: [(host: String, repository: Repository)]
    @Binding var project: String
    /// Start the task. Returns once its agent's terminal exists — `nil` — or
    /// with the sentence to show when it could not be started. `name` is the
    /// one the footer showed. The panel closes itself on success unless ⌥⏎
    /// asked it not to, through `onClose` — the same way Esc closes it.
    ///
    /// `host` comes from `chosen` below, the same picker selection that
    /// resolved `project` — not re-derived by the caller from `project`
    /// alone. `NewWorkspaceSheet.Choice` carries host and repository
    /// together for exactly this reason: a repository chosen without its
    /// host, handed to whatever runner happens to be "current" downstream,
    /// is how a task starts on the wrong one with no error at all.
    let onSubmit: (TaskRequest) async -> TaskSubmission.Outcome
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
    /// Who names the task. The on-device model where this Mac has one, and
    /// the heuristic otherwise; a test hands in its own.
    var namer: TaskNamer = .onDevice
    /// Whether a start is in flight and how the last one failed. Owned by
    /// the caller so it outlives the panel being closed and reopened.
    @ObservedObject var submission: TaskSubmission

    /// The draft survives closing the panel.
    ///
    /// A long prompt is often written in two sittings — you start one, go and
    /// look at something, come back. Losing it on Esc taught people not to
    /// close the panel, which is worse than the panel being open.
    @AppStorage("tasks.draft") private var text = ""
    @AppStorage("tasks.agent") private var agent = "claude"
    @AppStorage("tasks.model") private var model = ""

    @State private var justCreated: String?
    /// The namer's answer, and the description it answered for. Asked while
    /// you type (see `body`'s `.task`), so ⏎ never waits for a model: the
    /// name it sends is whichever the footer shows at that moment.
    @State private var named: (description: String, name: String)?

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

    private var description: String {
        text.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// The branch follows the name, so the directory and the branch agree.
    private var branch: String {
        Branch.slug(from: name, prefix: branchPrefix(chosen?.host ?? ""))
    }

    /// The name this task will carry: the namer's answer for exactly this
    /// description when it has one, the heuristic until then. It is a
    /// directory rather than a stored string, so what this becomes is what
    /// the sidebar row says for as long as the worktree exists.
    private var name: String {
        if let leftover { return leftover.name }
        if let named, named.description == description { return named.name }
        return TaskName.heuristic(description)
    }

    /// A worktree the last start made in the chosen project without starting
    /// its agent. Starting again goes on in it, under its name, rather than
    /// making another beside it.
    private var leftover: TaskSubmission.Left? {
        guard let left = submission.left, let chosen, left.host == chosen.host,
            left.project == chosen.repository.id
        else { return nil }
        return left
    }

    /// The worktree about to be created, which is the only thing naming this
    /// task. Nobody typed it, so it is shown rather than left to be found.
    private var worktreePath: String? {
        guard let chosen, WorktreeName.isValid(name) else { return nil }
        return WorktreeName.path(repository: chosen.repository.displayName, name: name)
    }

    /// A description of nothing but punctuation is not a task. Any letter or
    /// digit, in any script, is — a description with no word a directory can
    /// hold is still named (`task`). `TaskName` keeps the name well under the
    /// daemon's sixty-scalar ceiling, but the ceiling is checked where the
    /// name is decided rather than assumed from a cut made somewhere else.
    private var canSubmit: Bool {
        chosen != nil && hasWords && TaskPrompt.problem(description) == nil
            && WorktreeName.isValid(name)
    }

    private var hasWords: Bool { description.contains { $0.isLetter || $0.isNumber } }

    /// Why ⏎ would do nothing, when there is text and it would. Said rather
    /// than left to be discovered.
    private var reason: String? {
        guard !description.isEmpty else { return nil }
        if !hasWords { return "Add a word to say what you want done." }
        if let problem = TaskPrompt.problem(description) { return problem }
        if chosen == nil { return "Pick a project for the new workspace." }
        return nil
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
                    onSubmit: { keepOpen in Task { await submit(keepOpen: keepOpen) } },
                    onCancel: onClose
                )
                .frame(height: composerHeight)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 11)

            if let failure = submission.failure {
                Label(failure, systemImage: "exclamationmark.triangle.fill")
                    .font(.system(size: 11))
                    .foregroundStyle(.red)
                    .padding(.horizontal, 14)
                    .padding(.bottom, 8)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Divider()
            footer
        }
        .background(.regularMaterial)
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Color.primary.opacity(0.08)))
        .shadow(color: .black.opacity(0.18), radius: 20, y: 8)
        .frame(width: 560)
        .onAppear {
            submission.opened()
            OnDeviceNamer.prewarm()
        }
        // A failure is about the text it was for; editing it clears it.
        // Emptying it is starting on something else, so a worktree the last
        // start left behind is no longer this draft's to go on in.
        .onChange(of: text) { _, _ in
            submission.failure = nil
            if description.isEmpty { submission.forgetLeft() }
        }
        // Named while typing, a moment after the typing stops: `.task(id:)`
        // cancels the previous one on every keystroke, so only a pause asks
        // the model anything.
        .task(id: description) {
            let asked = description
            guard !asked.isEmpty else { return }
            try? await Task.sleep(for: .milliseconds(250))
            guard !Task.isCancelled else { return }
            let answer = await namer.name(for: asked)
            guard !Task.isCancelled else { return }
            named = (asked, answer)
        }
    }

    /// Grows with the prompt, up to a point.
    private var composerHeight: CGFloat {
        let lines = text.reduce(1) { $1 == "\n" ? $0 + 1 : $0 }
        let wrapped = max(lines, (text.count / 62) + 1)
        return min(max(CGFloat(wrapped) * 19 + 6, 25), 220)
    }

    private var footer: some View {
        HStack(spacing: 8) {
            if submission.starting {
                ProgressView().controlSize(.mini)
                Text("Starting…").foregroundStyle(.secondary)
            } else if let reason {
                Text(reason).foregroundStyle(.secondary).lineLimit(1)
            } else if let created = justCreated, text.isEmpty {
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

            HStack(spacing: 12) {
                if canSubmit {
                    Text("↩ Start")
                    Text("⌥↩ Start and Keep Open")
                } else {
                    Text("⇧↩ New Line")
                }
            }
            .foregroundStyle(.tertiary)
        }
        .font(.system(size: 11))
        .padding(.horizontal, 14)
        .padding(.vertical, 7)
    }

    /// Start the task; once it exists, clear the draft and close unless
    /// `keepOpen`. A failure keeps both and shows why. Not private so a test
    /// can press ⏎ without a window.
    func submit(keepOpen: Bool) async {
        // The same gate the footer shows, not a weaker one.
        guard canSubmit, let chosen else { return }
        let request = TaskRequest(
            description: description, name: name, host: chosen.host,
            project: chosen.repository.id, preset: Agents.preset(agent: agent, model: model),
            workspace: leftover?.workspace)
        // Which opening of the panel this is, so a start that finishes after
        // the panel was closed and opened again cannot close the new one.
        let opening = submission.opening
        guard let startedAs = await submission.run({ await onSubmit(request) }) else { return }
        // The name it was started under, which may carry a `-2` the panel
        // could not know about.
        justCreated = WorktreeName.display(startedAs)
        // Only if it is still the text that was started: the panel may have
        // been closed, reopened and written in while this was in flight.
        if description == request.description { text = "" }
        if !keepOpen && submission.opening == opening { onClose() }
    }
}

/// One task to start, as the panel decided it.
struct TaskRequest: Equatable {
    var description: String
    var name: String
    var host: String
    var project: String
    var preset: String
    /// A worktree an earlier start made and could not start the agent in,
    /// to start it in now instead of making another. `name` is its name.
    var workspace: String? = nil
}

/// A task start in flight, and how the last one ended.
@MainActor
final class TaskSubmission: ObservableObject {
    @Published private(set) var starting = false
    @Published var failure: String?
    /// Counts the panel's openings (`opened()`). A start remembers the one
    /// it was made from and closes only that.
    private(set) var opening = 0
    /// The worktree the last start made without starting its agent, until a
    /// start succeeds or the draft is emptied. The panel starts the next
    /// attempt in it (`TaskRequest.workspace`) rather than making another.
    @Published private(set) var left: Left?

    /// A worktree made for a task whose agent did not start, and where.
    struct Left: Equatable {
        var host: String
        var project: String
        var workspace: String
        var name: String
    }

    /// How a start ended, as the panel needs it.
    enum Outcome: Equatable {
        /// Started, under this name — the panel's, or it with a suffix.
        case started(name: String)
        /// Not started, with the sentence to show, and the worktree it made
        /// on the way when it made one.
        case failed(String, left: Left? = nil)
    }

    func opened() { opening += 1 }

    func forgetLeft() { left = nil }

    /// Run `start` unless one is already running. The name it started under,
    /// or nil; a failure is kept in `failure` for the panel to show.
    func run(_ start: () async -> Outcome) async -> String? {
        guard !starting else { return nil }
        starting = true
        failure = nil
        let outcome = await start()
        starting = false
        switch outcome {
        case .started(let name):
            left = nil
            return name
        case .failed(let sentence, let madeNow):
            failure = sentence
            // Kept through a failure that made nothing — a runner out of
            // reach says nothing about the worktree still being there.
            if let madeNow { left = madeNow }
            return nil
        }
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
        return prefix + (out.isEmpty ? "workspace" : out)
    }
}
