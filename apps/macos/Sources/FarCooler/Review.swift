import AgentKit
import SwiftUI

/// What a branch changed, and what you have said about it.
///
/// Decoded from `farcooler review … --json`, which is the same path every other
/// read in this app takes. The daemon owns all of it: the anchors, the buffer,
/// and whether a comment still points anywhere. Nothing here computes state — it
/// draws what the host derived, the same contract the terminal rows keep.

struct ChangeSet: Decodable, Equatable {
    var branch: String
    var baseRef: String
    var baseCommit: String
    var headCommit: String
    var insertions: Int
    var deletions: Int
    var commits: [ReviewCommit]
    var files: [ChangedFile]
    var workingTree: WorkingTree?

    enum CodingKeys: String, CodingKey {
        case branch
        case baseRef = "base_ref"
        case baseCommit = "base_commit"
        case headCommit = "head_commit"
        case insertions, deletions, commits, files
        case workingTree = "working_tree"
    }

    static let empty = ChangeSet(
        branch: "", baseRef: "", baseCommit: "", headCommit: "",
        insertions: 0, deletions: 0, commits: [], files: [], workingTree: nil)

    var isDirty: Bool {
        guard let w = workingTree else { return false }
        return !w.staged.isEmpty || !w.unstaged.isEmpty || !w.untracked.isEmpty
            || !w.conflicted.isEmpty
    }
}

struct ReviewCommit: Decodable, Equatable, Identifiable {
    var sha: String
    var subject: String
    var author: String
    var timestamp: Int

    var id: String { sha }
    var short: String { String(sha.prefix(8)) }
}

struct ChangedFile: Decodable, Equatable, Identifiable {
    var path: String
    var insertions: Int
    var deletions: Int
    var binary: Bool

    var id: String { path }
}

struct WorkingTree: Decodable, Equatable {
    var staged: [String]
    var unstaged: [String]
    var untracked: [String]
    var conflicted: [String]
}

/// One thing you said, and where the daemon says it still points.
struct ReviewEntry: Decodable, Equatable, Identifiable {
    var id: String
    var body: String
    var status: String
    var disposition: String
    var anchor: Anchor
    /// Empty when the anchor resolved exactly. Anything else is the daemon
    /// saying how much certainty it lost, and it is shown rather than hidden.
    var anchorState: String
    var line: Int?
    var answer: String?
    var answerCorrelation: String?
    var resourceVersion: Int

    enum CodingKeys: String, CodingKey {
        case id, body, status, disposition, anchor, line, answer
        case anchorState = "anchor_state"
        case answerCorrelation = "answer_correlation"
        case resourceVersion = "resource_version"
    }

    struct Anchor: Decodable, Equatable {
        var kind: String?
        var path: String?
    }

    /// Where this comment is about, in the words a person would use.
    var place: String {
        switch anchor.kind {
        case "file", "lines", "hunk":
            guard let p = anchor.path else { return "a file" }
            if let line { return "\(p):\(line)" }
            return p
        case "commit": return "a commit"
        case "branch": return "the branch"
        default: return "No specific location"
        }
    }

    var isQuestion: Bool { disposition == "ask" }
    var needsReread: Bool { anchorState.contains("needs re-read") }
    var isSent: Bool { status == "sent" }
    var isUnknown: Bool { status == "UNKNOWN" }
}

/// Everything the review surface needs for one workspace.
@MainActor
final class ReviewStore: ObservableObject {
    @Published var changeSet: ChangeSet = .empty
    @Published var entries: [ReviewEntry] = []
    @Published var diff: [DiffComputation.Line] = []
    @Published var selectedFile: String?
    @Published var loading = false
    @Published var error: String?

    private let client: DaemonClient
    private let workspace: Workspace

    init(client: DaemonClient, workspace: Workspace) {
        self.client = client
        self.workspace = workspace
    }

    func load(fresh: Bool = false) async {
        loading = true
        defer { loading = false }

        var args = ["review", "status", workspace.short, "--json"]
        if fresh { args.append("--fresh") }
        if let data = await client.reviewJSON(args) {
            changeSet = (try? JSONDecoder().decode(ChangeSet.self, from: data)) ?? .empty
        }
        if let data = await client.reviewJSON(["review", "list", workspace.short, "--json"]) {
            entries = (try? JSONDecoder().decode([ReviewEntry].self, from: data)) ?? []
        }
    }

    func closeFile() {
        selectedFile = nil
        diff = []
    }

    func openFile(_ path: String) async {
        selectedFile = path
        diff = await client.reviewDiff(workspace: workspace.short, path: path)
    }

    func capture(_ body: String, ask: Bool) async {
        // The anchor follows what you were looking at, and "nothing" is a real
        // answer rather than a failure — over half of real review comments are
        // not about a particular place.
        await client.reviewNote(
            workspace: workspace.short, body: body, file: selectedFile, ask: ask)
        await load()
    }

    func drop(_ entry: ReviewEntry) async {
        await client.reviewDrop(workspace: workspace.short, entry: entry.id)
        await load()
    }

    func send(_ entries: [ReviewEntry], to terminal: Terminal, ask: Bool) async {
        await client.reviewSend(
            workspace: workspace.short, terminal: terminal.short,
            entries: entries.map(\.id), ask: ask)
        await load()
    }

    func markSeen() async {
        await client.reviewSeen(workspace: workspace.short)
        await load()
    }
}

/// The review surface: what changed on the left, what you said on the right.
///
/// A Far Cooler-owned split beside the pane canvas, NOT a pane in it. `PaneGroup`
/// is a projection of tmux's own window tree keyed by terminal id, and there is
/// no pane identity for a view that is not a process — inventing one would put a
/// second layout tree next to tmux's, which is exactly what this codebase
/// deleted once already.
struct ReviewPane: View {
    /// Owned by the view, not handed in.
    ///
    /// An earlier version kept these in a dictionary on `ContentView` and wrote
    /// to it from a `Task`, which meant every re-render built a fresh store while
    /// the previous one was still loading — so each load was thrown away by the
    /// next render and the pane sat there permanently empty. `@StateObject` plus
    /// an `.id(workspace)` on the call site gives one store per worktree with a
    /// lifetime SwiftUI actually manages.
    @StateObject private var review: ReviewStore
    let workspace: Workspace
    let terminals: [Terminal]

    init(client: DaemonClient, workspace: Workspace, terminals: [Terminal]) {
        _review = StateObject(wrappedValue: ReviewStore(client: client, workspace: workspace))
        self.workspace = workspace
        self.terminals = terminals
    }

    @State private var draft = ""
    @State private var asking = false
    @State private var sendTarget: Terminal?
    @State private var showingFiles = true

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    if showingFiles {
                        changed
                    }
                    if review.selectedFile != nil {
                        diffView
                    }
                    buffer
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            Divider()
            composer
        }
        .background(.background)
        .task { await review.load() }
    }

    private var header: some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 1) {
                Text(review.changeSet.branch.isEmpty ? workspace.branch : review.changeSet.branch)
                    .font(.system(size: 12, weight: .medium))
                    .lineLimit(1)
                HStack(spacing: 6) {
                    Text("vs \(review.changeSet.baseRef)")
                    if review.changeSet.insertions > 0 {
                        Text("+\(review.changeSet.insertions)").foregroundStyle(.green)
                    }
                    if review.changeSet.deletions > 0 {
                        Text("−\(review.changeSet.deletions)").foregroundStyle(.red)
                    }
                    if review.changeSet.isDirty {
                        Circle().fill(.orange).frame(width: 5, height: 5)
                    }
                }
                .font(.system(size: 10.5))
                .foregroundStyle(.secondary)
            }
            Spacer(minLength: 4)
            // Icons, not words. This panel's width is not ours to choose — the
            // window's size can be owned by something else entirely — so the
            // header has to survive being narrow, and two text buttons pushed
            // the branch name straight off the edge.
            Button {
                Task { await review.load(fresh: true) }
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help("Read the worktree again")

            Button {
                Task { await review.markSeen() }
            } label: {
                Image(systemName: "checkmark.circle")
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .help("Mark Reviewed")
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    private var changed: some View {
        VStack(alignment: .leading, spacing: 6) {
            sectionTitle("Changed files", count: review.changeSet.files.count)
            ForEach(review.changeSet.files) { f in
                Button {
                    Task { await review.openFile(f.path) }
                } label: {
                    HStack(spacing: 6) {
                        Text(f.path)
                            .font(.system(size: 11, design: .monospaced))
                            .lineLimit(1)
                            .truncationMode(.head)
                            .layoutPriority(-1)
                        Spacer(minLength: 6)
                        if f.binary {
                            Text("binary").foregroundStyle(.secondary)
                        } else {
                            Text("+\(f.insertions)").foregroundStyle(.green)
                            Text("−\(f.deletions)").foregroundStyle(.red)
                        }
                    }
                    .font(.system(size: 10.5, design: .monospaced))
                    .padding(.vertical, 2)
                    .padding(.horizontal, 6)
                    .background(
                        review.selectedFile == f.path
                            ? Color.accentColor.opacity(0.15) : .clear,
                        in: RoundedRectangle(cornerRadius: 4))
                }
                .buttonStyle(.plain)
            }

            if !review.changeSet.commits.isEmpty {
                sectionTitle("Commits", count: review.changeSet.commits.count)
                    .padding(.top, 4)
                ForEach(review.changeSet.commits) { c in
                    HStack(spacing: 8) {
                        Text(c.short)
                            .font(.system(size: 10.5, design: .monospaced))
                            .foregroundStyle(.secondary)
                        Text(c.subject)
                            .font(.system(size: 11))
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .layoutPriority(-1)
                    }
                }
            }
        }
    }

    private var diffView: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(review.selectedFile ?? "")
                    .font(.system(size: 11, weight: .medium, design: .monospaced))
                    .lineLimit(1)
                Spacer()
                Button("Close") { review.closeFile() }
                    .buttonStyle(.plain)
                    .font(.system(size: 10.5))
                    .foregroundStyle(.secondary)
            }
            // No syntax highlighting, deliberately and still: what earns the
            // pixels is which lines changed, and colouring keywords on top of an
            // add/remove background fights the one signal that matters.
            ScrollView(.horizontal, showsIndicators: false) {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(review.diff) { line in
                        diffRow(line)
                    }
                }
            }
        }
    }

    private func diffRow(_ line: DiffComputation.Line) -> some View {
        HStack(spacing: 0) {
            Text(line.oldNumber.map(String.init) ?? "")
                .frame(width: 30, alignment: .trailing)
                .foregroundStyle(.tertiary)
            Text(line.newNumber.map(String.init) ?? "")
                .frame(width: 30, alignment: .trailing)
                .foregroundStyle(.tertiary)
            Text(marker(for: line.kind)).frame(width: 12)
            Text(line.text.isEmpty ? " " : line.text)
                .fixedSize(horizontal: true, vertical: false)
            Spacer(minLength: 0)
        }
        .font(.system(size: 10.5, design: .monospaced))
        .background(background(for: line.kind))
    }

    private func marker(for kind: DiffComputation.Kind) -> String {
        switch kind {
        case .added: return "+"
        case .removed: return "−"
        case .context: return " "
        }
    }

    private func background(for kind: DiffComputation.Kind) -> Color {
        switch kind {
        case .added: return .green.opacity(0.13)
        case .removed: return .red.opacity(0.13)
        case .context: return .clear
        }
    }

    private var buffer: some View {
        VStack(alignment: .leading, spacing: 8) {
            sectionTitle("Comments", count: review.entries.count)
            if review.entries.isEmpty {
                Text("Nothing yet. Anything you type below goes here.")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
            ForEach(review.entries) { e in
                VStack(alignment: .leading, spacing: 3) {
                    HStack(spacing: 6) {
                        Text(e.isQuestion ? "Ask" : "Fix")
                            .font(.system(size: 9, weight: .semibold))
                            .padding(.horizontal, 5)
                            .padding(.vertical, 1)
                            .background(
                                (e.isQuestion ? Color.purple : Color.blue).opacity(0.18),
                                in: Capsule())
                        Text(e.place)
                            .font(.system(size: 10.5, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .layoutPriority(-1)
                        if e.needsReread {
                            Text("re-read")
                                .font(.system(size: 9))
                                .foregroundStyle(.orange)
                        } else if !e.anchorState.isEmpty {
                            Text(e.anchorState)
                                .font(.system(size: 9))
                                .foregroundStyle(.secondary)
                        }
                        Spacer(minLength: 2)
                        // Icons, not words. This column can be narrow, and a
                        // status word here pushed the comment itself off the
                        // right edge — the one thing on the card that has to be
                        // readable.
                        if e.isUnknown {
                            Image(systemName: "questionmark.circle.fill")
                                .font(.system(size: 9))
                                .foregroundStyle(.orange)
                                .help("Far Cooler can't tell whether the agent got this")
                        } else if e.isSent {
                            Image(systemName: "paperplane.fill")
                                .font(.system(size: 8))
                                .foregroundStyle(.secondary)
                                .help("Sent to an agent")
                        }
                        Button {
                            Task { await review.drop(e) }
                        } label: {
                            Image(systemName: "xmark").font(.system(size: 8))
                        }
                        .buttonStyle(.plain)
                        .foregroundStyle(.tertiary)
                    }
                    Text(e.body).font(.system(size: 11.5)).fixedSize(horizontal: false, vertical: true)
                    if let a = e.answer {
                        VStack(alignment: .leading, spacing: 2) {
                            if e.answerCorrelation == "uncorrelated" {
                                Text("Couldn't match this answer to this question")
                                    .font(.system(size: 9))
                                    .foregroundStyle(.orange)
                            }
                            Text(a)
                                .font(.system(size: 11))
                                .foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .padding(.leading, 8)
                        .overlay(alignment: .leading) {
                            Rectangle().fill(.quaternary).frame(width: 2)
                        }
                    }
                }
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 6))
            }
        }
    }

    private var composer: some View {
        VStack(alignment: .leading, spacing: 6) {
            TextField(
                review.selectedFile.map { "Comment on \($0)" } ?? "Comment on this workspace",
                text: $draft, axis: .vertical
            )
            .textFieldStyle(.plain)
            .font(.system(size: 11.5))
            .lineLimit(1...4)
            .onSubmit(file)

            HStack(spacing: 6) {
                Toggle("Ask", isOn: $asking)
                    .toggleStyle(.button)
                    .controlSize(.small)
                    .font(.system(size: 10.5))
                    .help("File this as a question instead of an instruction")
                Spacer(minLength: 0)
                Button("Add", action: file)
                    .controlSize(.small)
                    .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .keyboardShortcut(.return, modifiers: .command)
            }
            Divider()
            HStack(spacing: 6) {
                let open = review.entries.filter { !$0.isSent }
                Text("^[\(open.count) comment](inflect: true) to send")
                    .font(.system(size: 10.5))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .layoutPriority(-1)
                Spacer(minLength: 4)
                // The terminal is YOURS to pick, including one already working.
                // There is no such thing as an ask terminal: the one that just
                // answered a question is very often the one you then tell to fix
                // the thing.
                Menu {
                    ForEach(terminals) { t in
                        Button(t.title) {
                            Task { await review.send(open, to: t, ask: false) }
                        }
                    }
                } label: {
                    Text("Send")
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .controlSize(.small)
                .help("Send these comments to a terminal you pick")
                .disabled(open.isEmpty || terminals.isEmpty)

                Menu {
                    ForEach(terminals) { t in
                        Button(t.title) {
                            Task { await review.send(open.filter(\.isQuestion), to: t, ask: true) }
                        }
                    }
                } label: {
                    Text("Ask")
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .controlSize(.small)
                .help("Send the questions to a terminal you pick")
                .disabled(!open.contains(where: \.isQuestion) || terminals.isEmpty)
            }
        }
        .padding(10)
    }

    private func file() {
        let body = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !body.isEmpty else { return }
        let ask = asking
        draft = ""
        asking = false
        Task { await review.capture(body, ask: ask) }
    }

    private func sectionTitle(_ text: String, count: Int) -> some View {
        HStack(spacing: 5) {
            Text(text.uppercased())
                .font(.system(size: 9.5, weight: .semibold))
                .foregroundStyle(.secondary)
            Text("\(count)")
                .font(.system(size: 9.5))
                .foregroundStyle(.tertiary)
        }
    }
}
