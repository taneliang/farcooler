import PhotosUI
import SwiftUI
import UIKit

/// One agent session, full screen — the surface `TerminalView` swaps in for
/// `TerminalSurface` when `terminal.isAgentPane`.
///
/// Unlike the Mac, there is no tmux rectangle to draw into: this pane is
/// always the whole screen, and the tab strip that lets you leave it lives
/// below it exactly the way it lives below a terminal — `TerminalView` owns
/// that, not this view. Nothing here is new permanent chrome: mode and
/// attachments live in the composer row, matching the constraint that ruled
/// out a second header for this surface the way `TerminalPane` has none.
@MainActor
struct AgentView: View {
    let terminalID: String
    let workspaceID: String?
    @ObservedObject var connection: Connection

    @StateObject private var stream: AgentStream

    init(terminalID: String, workspaceID: String?, connection: Connection) {
        self.terminalID = terminalID
        self.workspaceID = workspaceID
        self.connection = connection
        _stream = StateObject(wrappedValue: AgentStream(terminal: terminalID, core: connection.core))
    }

    private var transcript: Transcript { stream.transcript }

    /// Whether a turn is running, as the daemon sees it.
    ///
    /// From the fleet rather than inferred here: the daemon derives activity
    /// for every surface that shows it, and a second opinion computed on the
    /// phone is exactly the disagreement this design exists to prevent.
    private var isWorking: Bool {
        guard let workspaceID else { return false }
        return connection.terminal(terminalID, in: workspaceID)?.agent == .working
    }

    var body: some View {
        VStack(spacing: 0) {
            if !transcript.plan.isEmpty {
                PlanPanel(entries: transcript.plan)
                    .padding(.horizontal, 12)
                    .padding(.top, 8)
            }

            transcriptBody
                // The composer sits in the transcript's bottom safe area, which
                // is the framework's own answer to "a control resting on
                // scrolling content": the conversation runs the full height and
                // scrolls behind it, and the scroll view insets itself so the
                // last line stays reachable. The Mac reached the same place by
                // the same route, after building it by hand first and freezing
                // the app.
                .safeAreaInset(edge: .bottom, spacing: 0) { composerStack }
        }
        .onAppear { stream.start() }
        .onDisappear { stream.stop() }
    }

    /// Everything that sits over the bottom of the conversation.
    @ViewBuilder
    private var composerStack: some View {
        VStack(spacing: 6) {
            if let pending = transcript.pendingPermission {
                ApprovalCard(pending: pending) { optionID in
                    Task { await stream.answer(pending.id, optionID) }
                }
                .padding(.horizontal, 12)
            }

            AgentComposer(
                availableModes: transcript.availableModes,
                agentMode: transcript.agentMode,
                availableCommands: transcript.availableCommands,
                workspaceID: workspaceID,
                core: connection.core,
                onSend: { text in
                    // Whether this goes out now or waits is the shim's call,
                    // but the echo depends on the answer — see
                    // `AgentStream.send`.
                    let working = isWorking
                    Task { await stream.send(text, whileWorking: working) }
                },
                onSetMode: { mode in Task { await stream.setMode(mode) } }
            )
        }
    }

    @ViewBuilder
    private var transcriptBody: some View {
        if transcript.rows.isEmpty {
            // The error goes ABOVE the empty state, not inside the transcript.
            //
            // The banner used to live in the scroll view, which only exists
            // once there are rows — so a session that never loaded at all
            // showed "Say something to begin" with the reason it was empty
            // hidden behind the very condition that made it empty. The one
            // moment the message is worth reading is the one moment it could
            // not be seen.
            VStack(spacing: 12) {
                if let error = stream.connectionError {
                    Text(error)
                        .font(.caption)
                        .foregroundStyle(.orange)
                        .multilineTextAlignment(.center)
                        .padding(.horizontal, 24)
                }
                emptyState
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 12) {
                        // A stale error banner rather than a blanked screen: a
                        // failed poll is not a disconnection, the same rule
                        // `Connection.refresh()` follows — the last known
                        // transcript stays up while this device tries again.
                        if let error = stream.connectionError {
                            Text(error)
                                .font(.caption)
                                .foregroundStyle(.orange)
                        }
                        ForEach(transcript.rows) { row in
                            AgentRowView(row: row)
                        }

                        // Written, not yet sent. See `QueuedPrompt`: a message
                        // typed while the agent is working waits for the turn
                        // to end, and drawing it like a sent one claimed
                        // something that had not happened.
                        ForEach(transcript.queue) { queued in
                            QueuedRow(
                                queued: queued,
                                onEdit: { text in
                                    Task { await stream.editQueued(queued.id, text) }
                                },
                                onCancel: { Task { await stream.cancelQueued(queued.id) } },
                                onSteer: { Task { await stream.steerQueued(queued.id) } })
                        }
                        // An invisible anchor rather than scrolling to the
                        // last row's own id: the last row mutates in place
                        // while a tool streams progress (see `Transcript`),
                        // so its id does not change and `scrollTo` would have
                        // nothing new to react to.
                        Color.clear.frame(height: 1).id("bottom")
                    }
                    .padding(12)
                }
                // Keyed on the CURSOR, not the row count. A streamed reply
                // coalesces into the row already on screen, so the count does
                // not change while the text grows off the bottom.
                .onChange(of: transcript.cursor) { _, _ in
                    withAnimation(.easeOut(duration: 0.2)) {
                        proxy.scrollTo("bottom", anchor: .bottom)
                    }
                }
                .onAppear { proxy.scrollTo("bottom", anchor: .bottom) }
            }
        }
    }

    private var emptyState: some View {
        VStack(spacing: 10) {
            Spacer()
            if let error = stream.connectionError {
                Image(systemName: "exclamationmark.triangle")
                    .font(.largeTitle)
                    .foregroundStyle(.orange)
                Text("Could not load this session")
                    .font(.headline)
                Text(error)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 32)
            } else {
                Text("Say something to begin.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            Spacer()
        }
        .frame(maxWidth: .infinity)
    }
}

// MARK: - Rows

/// One row of a rendered agent transcript.
///
/// A thin switch, deliberately: `Transcript` already decided what happened —
/// coalesced message chunks, mutated a tool call in place rather than
/// appending, kept a gap as its own row — this only decides how each of the
/// three shapes it can hand back gets drawn.
private struct AgentRowView: View {
    let row: TranscriptRow

    var body: some View {
        switch row.kind {
        case let .message(role, text):
            MessageRow(role: role, text: text)
        case let .tool(tool):
            ToolRowView(tool: tool)
        case let .gap(reason):
            GapRow(reason: reason)
        }
    }
}

/// One message. Three shapes for three roles, because they answer three
/// different questions: what did I say, what did it say, and what did it
/// think before saying that.
private struct MessageRow: View {
    let role: Role
    let text: String

    var body: some View {
        switch role {
        case .user:
            // Right-aligned with a fill — the one voice in the transcript
            // that is not the agent talking, and it has to read as a
            // different speaker at a glance, not on close reading.
            HStack {
                Spacer(minLength: 40)
                Text(text)
                    .font(.callout)
                    .textSelection(.enabled)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(Color.primary.opacity(0.07), in: RoundedRectangle(cornerRadius: 14))
            }

        case .agent:
            // Full width, through the SHARED renderer — the same one the Mac
            // uses. Plain `Text` here meant a table arrived as a wall of pipes
            // and a heading as a line starting with a hash: the same
            // conversation, unreadable on the phone.
            HStack {
                MarkdownText(text: text)
                Spacer(minLength: 40)
            }

        case .thought:
            // Collapsed by default: a thought is the agent's scratch work,
            // useful once something has gone wrong and noise every other
            // time. A `DisclosureGroup` with no binding starts closed, which
            // is the state most sessions never need to leave.
            DisclosureGroup {
                MarkdownText(text: text, secondary: true)
                    .padding(.top, 2)
            } label: {
                Text("Thought")
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
            }
        }
    }
}

/// One tool call, mutated in place by every update — never a new row — so a
/// call that reports progress four times still occupies the one line it
/// earned.
private struct ToolRowView: View {
    let tool: ToolRow

    private var expandable: Bool { tool.content != nil || tool.diff != nil }

    var body: some View {
        if expandable {
            DisclosureGroup {
                VStack(alignment: .leading, spacing: 8) {
                    if let content = tool.content, !content.isEmpty {
                        // Bounded, and for the same reason it is bounded on the
                        // Mac: `Text` measures its whole string on every layout
                        // pass, and a tool that returns thousands of lines
                        // inside an animated disclosure froze the app there.
                        DetailBox(text: content, chrome: false)
                    }
                    if let diff = tool.diff {
                        DiffView(diff: diff)
                    }
                }
                .padding(.top, 4)
            } label: {
                label
            }
        } else {
            label
        }
    }

    private var label: some View {
        HStack(spacing: 7) {
            // Not `StatusGlyph` — that type lives in the Mac target. Same
            // vocabulary, one dot at one weight for "something is happening",
            // just drawn locally: green finished, red missing, secondary
            // for everything in between. Orange is reserved for "needs you",
            // which a tool call never is.
            Circle()
                .fill(toolStatusColour(tool.status))
                .frame(width: 7, height: 7)
            Text(tool.title)
                .font(.subheadline.weight(.medium))
            if let location = tool.locations.first {
                Text(location)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 4)
        }
    }
}

private func toolStatusColour(_ status: ToolStatus) -> Color {
    switch status {
    case .pending: return .secondary.opacity(0.35)
    case .inProgress: return .secondary
    case .completed: return .green
    case .failed: return .red
    }
}

/// A break in the transcript, named rather than hidden.
///
/// The one row here that is not allowed to be quiet. A gap is the opposite of
/// nothing — it is the transcript admitting history is missing — and drawing
/// it as a thin rule between two messages would let a reader miss the one
/// fact this whole design exists to never hide.
private struct GapRow: View {
    let reason: GapReason

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "scissors")
                .font(.caption)
            Text(sentence)
                .font(.footnote.weight(.medium))
            Spacer(minLength: 0)
        }
        .foregroundStyle(.orange)
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 8))
    }

    private var sentence: String {
        switch reason {
        case .ringTrimmed:
            return "Some earlier history was trimmed and is not shown here."
        case .loadUnsupported:
            return "This session could not be loaded from where it left off."
        case .unparsed:
            return "Something happened here that this version cannot show."
        }
    }
}

// MARK: - Plan

/// The agent's task list, as the agent maintains it.
///
/// The same design as the Mac's, for the same reason the reducer is shared: a
/// list of bullets is the same information and none of the use — what a reader
/// wants is how far through it is and what is happening right now.
private struct PlanPanel: View {
    let entries: [PlanEntry]

    @State private var expanded = true

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button {
                withAnimation(.spring(response: 0.22, dampingFraction: 0.82)) {
                    expanded.toggle()
                }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "chevron.right")
                        .font(.caption2)
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                    Text("Tasks").font(.caption.weight(.semibold))
                    Text("\(entries.doneCount) of \(entries.count)")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    if !expanded, let active = entries.active {
                        Text("· \(active.content)")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    Spacer(minLength: 0)
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if expanded {
                ForEach(Array(entries.enumerated()), id: \.offset) { _, entry in
                    HStack(alignment: .top, spacing: 7) {
                        Image(systemName: PlanStatus(entry.status).symbol)
                            .font(.caption)
                            .foregroundStyle(PlanStatus(entry.status).tint)
                            .frame(width: 14)
                        Text(entry.content)
                            .font(.footnote)
                            .strikethrough(PlanStatus(entry.status).isDone)
                            .foregroundStyle(PlanStatus(entry.status).isDone ? .secondary : .primary)
                            .fixedSize(horizontal: false, vertical: true)
                        Spacer(minLength: 0)
                    }
                }
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10))
    }




}

/// A message written but not yet sent. See the Mac's `QueuedRow`.
private struct QueuedRow: View {
    let queued: QueuedPrompt
    let onEdit: (String) -> Void
    let onCancel: () -> Void
    /// Send this one into the turn already running.
    ///
    /// The queue's whole point is that a message you can still see and still
    /// edit beats one already gone — so waiting is the default. But a message
    /// written mid-turn is very often a correction, and a correction is worth
    /// nothing once the wrong thing has been done. This is the escape hatch:
    /// you looked at what you wrote and decided it should interrupt.
    let onSteer: () -> Void

    @State private var editing = false
    @State private var draft = ""

    var body: some View {
        HStack(alignment: .top, spacing: 6) {
            Spacer(minLength: 40)
            VStack(alignment: .trailing, spacing: 4) {
                if editing {
                    TextField("", text: $draft, axis: .vertical)
                        .textFieldStyle(.plain)
                        .font(.callout)
                        .frame(minWidth: 140)
                        .onSubmit(commit)
                } else {
                    Text(queued.text).font(.callout)
                }

                HStack(spacing: 10) {
                    Text("Queued")
                    Button("Send now", action: onSteer)
                        .buttonStyle(.plain)
                    Button(editing ? "Save" : "Edit") {
                        if editing {
                            commit()
                        } else {
                            draft = queued.text
                            editing = true
                        }
                    }
                    .buttonStyle(.plain)
                    Button("Remove", action: onCancel).buttonStyle(.plain)
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background {
                RoundedRectangle(cornerRadius: 14)
                    .fill(Color.primary.opacity(0.04))
                    .overlay {
                        RoundedRectangle(cornerRadius: 14)
                            .strokeBorder(
                                Color.secondary.opacity(0.4),
                                style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
                    }
            }
        }
    }

    private func commit() {
        let trimmed = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        editing = false
        guard !trimmed.isEmpty, trimmed != queued.text else { return }
        onEdit(trimmed)
    }
}

// MARK: - Approval

/// A permission request, blocking the turn until answered — the one place
/// this surface asks something of you rather than reporting something to
/// you. Buttons are full-width and tall on purpose: this is the card a thumb
/// has to hit correctly the first time, on a phone, possibly one-handed.
private struct ApprovalCard: View {
    let pending: PendingPermission
    let onChoose: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label("Needs your approval", systemImage: "hand.raised.fill")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.orange)

            VStack(spacing: 8) {
                ForEach(pending.options) { option in
                    Button {
                        onChoose(option.id)
                    } label: {
                        Text(option.name)
                            .font(.body.weight(.medium))
                            .frame(maxWidth: .infinity, minHeight: 46)
                    }
                    .buttonStyle(.bordered)
                    .tint(tint(for: option))
                }
            }
        }
        .padding(12)
        .background(Color.orange.opacity(0.08), in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Color.orange.opacity(0.35)))
    }

    /// A reject/deny option reads as destructive; everything else is a plain
    /// affirmative action. `kind` is a string the daemon defines (`allow_once`,
    /// `reject_once`, …) — matched loosely rather than against a fixed set, so
    /// a kind this client has never seen still lands on the safe, non-red
    /// default instead of a compile-time list going stale.
    private func tint(for option: PermissionOption) -> Color {
        let kind = option.kind.lowercased()
        return (kind.contains("reject") || kind.contains("deny")) ? .red : .accentColor
    }
}

// MARK: - Diff

/// A unified diff, computed client-side from the two full texts a tool call
/// carries.
///
/// No syntax highlighting — cut in the spec. What earns the pixels here is
/// which lines changed, not what language they are in. This duplicates the
/// Mac's `DiffView` rather than sharing it: `AgentKit` holds the decode and
/// the reduce, which both platforms must agree on bit-for-bit, but layout is
/// each platform's own job by design (see the plan's architecture note), and
/// a diff view is layout, not derivation.
struct DiffView: View {
    let diff: Diff

    /// Beyond this many lines, the diff opens collapsed. A four-line edit is
    /// worth seeing on arrival; a four-hundred-line rewrite is not something
    /// to scroll past to reach the message after it — doubly so on a screen
    /// this narrow.
    private static let collapseThreshold = 20

    @State private var expanded = false

    private var lines: [DiffComputation.Line] {
        DiffComputation.compute(old: diff.oldText ?? "", new: diff.newText)
    }

    var body: some View {
        let rows = lines
        VStack(alignment: .leading, spacing: 6) {
            header(for: rows)

            if rows.count > Self.collapseThreshold && !expanded {
                Button {
                    expanded = true
                } label: {
                    Text("Show \(rows.count) lines")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            } else {
                diffBody(rows)
            }
        }
    }

    private func header(for rows: [DiffComputation.Line]) -> some View {
        let added = rows.filter { $0.kind == .added }.count
        let removed = rows.filter { $0.kind == .removed }.count
        return HStack(spacing: 6) {
            Text(diff.path)
                .font(.caption.weight(.medium).monospaced())
                .lineLimit(1)
            Spacer(minLength: 8)
            if added > 0 {
                Text("+\(added)").foregroundStyle(.green)
            }
            if removed > 0 {
                Text("-\(removed)").foregroundStyle(.red)
            }
        }
        .font(.caption2.monospaced())
        .foregroundStyle(.secondary)
    }

    private func diffBody(_ rows: [DiffComputation.Line]) -> some View {
        // A narrower gutter than the Mac's: a phone has far fewer points to
        // spend on line numbers before the code column itself is squeezed
        // unreadable, and diffs here scroll horizontally besides (see the
        // wrapping `ScrollView` below).
        let gutterWidth: CGFloat = 26

        return ScrollView(.horizontal, showsIndicators: false) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(rows) { line in
                    HStack(spacing: 0) {
                        Text(line.oldNumber.map(String.init) ?? "")
                            .frame(width: gutterWidth, alignment: .trailing)
                        Text(line.newNumber.map(String.init) ?? "")
                            .frame(width: gutterWidth, alignment: .trailing)
                        Text(line.kind.marker)
                            .frame(width: 12, alignment: .center)
                        Text(line.text.isEmpty ? " " : line.text)
                            .fixedSize(horizontal: true, vertical: false)
                    }
                    .font(.caption2.monospaced())
                    .foregroundStyle(line.kind == .context ? .secondary : .primary)
                    .padding(.horizontal, 4)
                    .padding(.vertical, 1)
                    .background(line.kind.background)
                }
            }
            .padding(.vertical, 4)
        }
        .textSelection(.enabled)
        .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 6))
    }
}

/// The diffing itself, kept apart from the view so it has no SwiftUI in it —
/// identical algorithm to the Mac's `DiffComputation`, kept as two copies for
/// the same reason the view is: it is layout-adjacent, not the shared
/// contract `AgentKit` exists to hold.
enum DiffComputation {
    enum Kind { case context, added, removed }

    struct Line: Identifiable {
        let id: Int
        let kind: Kind
        let oldNumber: Int?
        let newNumber: Int?
        let text: String
    }

    /// Line-based diff of two whole file texts.
    ///
    /// Trims the common prefix and suffix first (O(n)), then runs a classic
    /// LCS only over whatever is left in the middle — a tool call is usually
    /// one small change inside two mostly-identical texts, so this throws
    /// away nearly all of the work before the expensive part ever runs. Past
    /// a size where the LCS table would be slow, the leftover middle is shown
    /// as a flat replace instead — still a correct diff, just not the minimal
    /// one.
    static func compute(old: String, new: String) -> [Line] {
        let oldLines = old.isEmpty ? [] : old.components(separatedBy: "\n")
        let newLines = new.isEmpty ? [] : new.components(separatedBy: "\n")

        var prefix = 0
        while prefix < oldLines.count, prefix < newLines.count,
            oldLines[prefix] == newLines[prefix]
        {
            prefix += 1
        }

        var suffix = 0
        while suffix < oldLines.count - prefix, suffix < newLines.count - prefix,
            oldLines[oldLines.count - 1 - suffix] == newLines[newLines.count - 1 - suffix]
        {
            suffix += 1
        }

        let oldMiddle = Array(oldLines[prefix..<(oldLines.count - suffix)])
        let newMiddle = Array(newLines[prefix..<(newLines.count - suffix)])

        var result: [Line] = []
        var id = 0
        var oldNum = 1
        var newNum = 1

        func push(_ kind: Kind, old: Int?, new: Int?, text: String) {
            result.append(Line(id: id, kind: kind, oldNumber: old, newNumber: new, text: text))
            id += 1
        }

        for line in oldLines[0..<prefix] {
            push(.context, old: oldNum, new: newNum, text: line)
            oldNum += 1
            newNum += 1
        }

        if oldMiddle.count * newMiddle.count <= 160_000 {
            for op in lcsDiff(oldMiddle, newMiddle) {
                switch op {
                case let .equal(line):
                    push(.context, old: oldNum, new: newNum, text: line)
                    oldNum += 1
                    newNum += 1
                case let .removed(line):
                    push(.removed, old: oldNum, new: nil, text: line)
                    oldNum += 1
                case let .added(line):
                    push(.added, old: nil, new: newNum, text: line)
                    newNum += 1
                }
            }
        } else {
            for line in oldMiddle {
                push(.removed, old: oldNum, new: nil, text: line)
                oldNum += 1
            }
            for line in newMiddle {
                push(.added, old: nil, new: newNum, text: line)
                newNum += 1
            }
        }

        let suffixStart = oldLines.count - suffix
        for offset in 0..<suffix {
            push(.context, old: oldNum, new: newNum, text: oldLines[suffixStart + offset])
            oldNum += 1
            newNum += 1
        }

        return result
    }

    private enum Op {
        case equal(String)
        case removed(String)
        case added(String)
    }

    private static func lcsDiff(_ a: [String], _ b: [String]) -> [Op] {
        guard !a.isEmpty else { return b.map { .added($0) } }
        guard !b.isEmpty else { return a.map { .removed($0) } }

        var table = [[Int]](repeating: [Int](repeating: 0, count: b.count + 1), count: a.count + 1)
        for i in stride(from: a.count - 1, through: 0, by: -1) {
            for j in stride(from: b.count - 1, through: 0, by: -1) {
                table[i][j] =
                    a[i] == b[j]
                    ? table[i + 1][j + 1] + 1
                    : max(table[i + 1][j], table[i][j + 1])
            }
        }

        var ops: [Op] = []
        var i = 0, j = 0
        while i < a.count && j < b.count {
            if a[i] == b[j] {
                ops.append(.equal(a[i]))
                i += 1
                j += 1
            } else if table[i + 1][j] >= table[i][j + 1] {
                ops.append(.removed(a[i]))
                i += 1
            } else {
                ops.append(.added(b[j]))
                j += 1
            }
        }
        while i < a.count { ops.append(.removed(a[i])); i += 1 }
        while j < b.count { ops.append(.added(b[j])); j += 1 }
        return ops
    }
}

extension DiffComputation.Kind {
    fileprivate var marker: String {
        switch self {
        case .context: return ""
        case .added: return "+"
        case .removed: return "-"
        }
    }

    fileprivate var background: Color {
        switch self {
        case .context: return .clear
        case .added: return .green.opacity(0.15)
        case .removed: return .red.opacity(0.15)
        }
    }
}

// MARK: - Composer

/// The prompt field, plus everything that hangs off it: slash commands, file
/// mentions, image attachments, and the agent-mode switcher. One row, the
/// same rule the constraint file states for the Mac: no second header, no
/// footer — whatever this pane needs to say lives here or nowhere.
private struct AgentComposer: View {
    /// The modes the agent offers, with their human names.
    ///
    /// `[AgentChoice]`, not `[String]`. This was `[String]` and had been
    /// failing to compile since modes gained names on the Mac — the phone's
    /// picker was listing wire identifiers like `acceptEdits` before that, and
    /// nothing at all after.
    let availableModes: [AgentChoice]
    let agentMode: String?
    let availableCommands: [String]
    let workspaceID: String?
    let core: ClientCore
    let onSend: (String) -> Void
    let onSetMode: (String) -> Void

    @State private var text = ""
    @State private var cursor = 0
    @State private var mentionResults: [String] = []
    @State private var mentionSearch: Task<Void, Never>?
    @State private var attachments: [ComposerAttachment] = []
    @State private var photoPickerItem: PhotosPickerItem?

    private var token: ComposerToken { activeToken(in: text, cursor: cursor) }

    private var canSend: Bool {
        !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        // A card that floats over the conversation, not a bar welded beneath it.
        //
        // This used to be a full-width strip on `.bar`, drawn to read as the
        // keyboard's top edge. That was the right instinct for iOS 18 and is
        // the wrong shape now: the platform's own messaging surfaces float a
        // rounded, glass field above scrolling content, and a squared-off slab
        // spanning edge to edge reads as a control from two releases ago.
        //
        // The transcript scrolls behind it — see `transcriptBody`'s
        // `safeAreaInset` — which is what the glass is for. A material with
        // nothing passing under it is just a grey rectangle.
        VStack(alignment: .leading, spacing: 0) {
            suggestions

            if !attachments.isEmpty {
                attachmentStrip
            }

            HStack(alignment: .bottom, spacing: 10) {
                PhotosPicker(selection: $photoPickerItem, matching: .images) {
                    Image(systemName: "photo.badge.plus")
                        .font(.system(size: 17))
                        .foregroundStyle(.secondary)
                }
                .onChange(of: photoPickerItem) { _, item in loadPickedPhoto(item) }

                modeMenu

                fieldWithPlaceholder

                Button(action: send) {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.system(size: 28))
                        .symbolRenderingMode(.hierarchical)
                }
                .disabled(!canSend)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 9)
        }
        .modifier(GlassField())
        .padding(.horizontal, 10)
        .padding(.bottom, 6)
        .onChange(of: text) { _, _ in scheduleMentionSearch() }
        .onChange(of: cursor) { _, _ in scheduleMentionSearch() }
    }

    // MARK: Field

    private var fieldWithPlaceholder: some View {
        ZStack(alignment: .leading) {
            if text.isEmpty {
                Text("Message").foregroundStyle(.tertiary).padding(.leading, 4)
            }
            ComposerTextView(text: $text, cursor: $cursor)
                .frame(minHeight: 34, maxHeight: 110)
        }
    }

    // MARK: Mode switcher

    @ViewBuilder
    private var modeMenu: some View {
        // A picker with one option is not a picker — the same rule
        // `TaskComposerView` follows for a single-repository fleet. A session
        // that never offered a second mode has nothing to switch to.
        if availableModes.count > 1 {
            Menu {
                ForEach(availableModes) { mode in
                    Button {
                        onSetMode(mode.id)
                    } label: {
                        if mode.id == agentMode {
                            Label(mode.name, systemImage: "checkmark")
                        } else {
                            Text(mode.name)
                        }
                    }
                }
            } label: {
                Image(systemName: "slider.horizontal.3")
                    .font(.system(size: 17))
                    .foregroundStyle(.secondary)
            }
        }
    }

    // MARK: Slash / mention suggestions

    @ViewBuilder
    private var suggestions: some View {
        switch token {
        case .none:
            EmptyView()
        case let .slash(prefix, range):
            let matches = availableCommands.filter {
                prefix.isEmpty || $0.lowercased().hasPrefix(prefix.lowercased())
            }
            if !matches.isEmpty {
                SuggestionList(items: matches, icon: "chevron.right.2") { command in
                    apply(range: range, replacement: "/\(command) ")
                }
            }
        case let .mention(_, range):
            if !mentionResults.isEmpty {
                SuggestionList(items: mentionResults, icon: "doc") { path in
                    apply(range: range, replacement: "@\(path) ")
                }
            }
        }
    }

    /// Replace the active token with a chosen completion, and put the caret
    /// right after what was inserted.
    ///
    /// Recomputed from character counts rather than reusing `range`'s indices
    /// after the mutation: a `String.Index` captured before `replaceSubrange`
    /// is not guaranteed valid after it, and the failure mode for guessing
    /// wrong is silent — a crash or a caret in the wrong place — not a
    /// compiler error.
    private func apply(range: Range<String.Index>, replacement: String) {
        let prefixCount = text.distance(from: text.startIndex, to: range.lowerBound)
        text.replaceSubrange(range, with: replacement)
        cursor = prefixCount + replacement.count
        mentionResults = []
    }

    private func scheduleMentionSearch() {
        mentionSearch?.cancel()
        guard case let .mention(prefix, _) = token, let workspaceID else {
            mentionResults = []
            return
        }
        mentionSearch = Task {
            // Debounced rather than fired on every keystroke: each search is
            // an ssh round trip, and a search whose result arrives after the
            // next keystroke already superseded it is nothing but cost.
            try? await Task.sleep(for: .milliseconds(200))
            guard !Task.isCancelled else { return }
            guard
                let data = try? await core.call(
                    "worktree.file_search",
                    ["workspace": workspaceID, "query": prefix, "limit": 20]),
                let decoded = try? JSONDecoder().decode(FileSearchResult.self, from: data)
            else { return }
            guard !Task.isCancelled else { return }
            mentionResults = decoded.paths
        }
    }

    private struct FileSearchResult: Decodable { let paths: [String] }

    // MARK: Attachments
    //
    // TODO(agent-images): `terminal.agent_prompt` (crates/client/src/ffi.rs)
    // only carries `text` today — there is no image content block on the
    // wire yet. Wiring the picker itself is straightforward and done below;
    // wiring delivery needs a protocol change this task's scope does not
    // cover, so an attached image is shown and can be removed, but sending
    // silently leaves it behind rather than pretending it went anywhere.
    // Paste is the same story: reachable, not delivered.

    private var attachmentStrip: some View {
        VStack(alignment: .leading, spacing: 4) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(attachments) { attachment in
                        ZStack(alignment: .topTrailing) {
                            Image(uiImage: attachment.image)
                                .resizable()
                                .scaledToFill()
                                .frame(width: 56, height: 56)
                                .clipShape(RoundedRectangle(cornerRadius: 8))
                            Button {
                                attachments.removeAll { $0.id == attachment.id }
                            } label: {
                                Image(systemName: "xmark.circle.fill")
                                    .font(.system(size: 15))
                                    .foregroundStyle(.white, .black.opacity(0.6))
                            }
                            .offset(x: 5, y: -5)
                        }
                    }
                }
                .padding(.horizontal, 10)
                .padding(.top, 6)
            }
            // Honest rather than silent: an image that visibly cannot be
            // sent is a far smaller surprise than one that is typed,
            // attached, sent, and simply never arrives. See the TODO above.
            Text("Images aren't sent to the agent yet")
                .font(.caption2)
                .foregroundStyle(.orange)
                .padding(.horizontal, 10)
        }
    }

    private func loadPickedPhoto(_ item: PhotosPickerItem?) {
        guard let item else { return }
        Task {
            if let data = try? await item.loadTransferable(type: Data.self),
                let image = UIImage(data: data)
            {
                attachments.append(ComposerAttachment(image: image))
            }
            photoPickerItem = nil
        }
    }

    private func send() {
        let message = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !message.isEmpty else { return }
        onSend(message)
        text = ""
        cursor = 0
        attachments = []
        mentionResults = []
    }
}

private struct ComposerAttachment: Identifiable {
    let id = UUID()
    let image: UIImage
}

/// The floating list a slash command or an `@` mention pops open, tap to
/// accept.
private struct SuggestionList: View {
    let items: [String]
    let icon: String
    let onChoose: (String) -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(items.prefix(8), id: \.self) { item in
                    Button {
                        onChoose(item)
                    } label: {
                        Label(item, systemImage: icon)
                            .font(.footnote.monospaced())
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 8)
                    }
                    .buttonStyle(.plain)
                    Divider().padding(.leading, 12)
                }
            }
        }
        .frame(maxHeight: 180)
        .background(.thickMaterial)
    }
}

/// A plain multi-line text field that also reports where the caret is.
///
/// `activeToken(in:cursor:)` needs a caret position to decide whether the `/`
/// or `@` under it should open a picker, and no SwiftUI `TextField` or
/// `TextEditor` on this deployment target hands one back for a plain `String`
/// binding. `UITextViewDelegate.textViewDidChangeSelection` does — which is
/// why this is a representable rather than a built-in control, the same
/// reasoning that put a `UIViewRepresentable` under `TerminalView`'s keyboard
/// input, just for a different reason: that one needed raw keystrokes with no
/// text field at all, and this one needs an ordinary text field that also
/// exposes its selection.
private struct ComposerTextView: UIViewRepresentable {
    @Binding var text: String
    @Binding var cursor: Int

    func makeUIView(context: Context) -> UITextView {
        let view = UITextView()
        view.font = .preferredFont(forTextStyle: .body)
        view.backgroundColor = .clear
        view.isScrollEnabled = true
        view.textContainerInset = UIEdgeInsets(top: 6, left: 0, bottom: 6, right: 0)
        view.textContainer.lineFragmentPadding = 0
        view.delegate = context.coordinator
        view.text = text
        return view
    }

    /// Only pushes `text` into the view when it changed from OUTSIDE the
    /// view's own typing — a slash-command completion, say, or `send()`
    /// clearing the field. When the user is simply typing, `text` already
    /// equals `uiView.text` because `textViewDidChange` just set it, and
    /// overwriting `.text` here on every SwiftUI render pass would reset
    /// `UITextView`'s own undo stack and marked (IME composition) text out
    /// from under whatever is mid-composition.
    func updateUIView(_ uiView: UITextView, context: Context) {
        guard uiView.text != text else { return }
        uiView.text = text
        let location = ComposerTextView.utf16Offset(forCharacterOffset: cursor, in: text)
        uiView.selectedRange = NSRange(location: location, length: 0)
    }

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    final class Coordinator: NSObject, UITextViewDelegate {
        var parent: ComposerTextView
        init(_ parent: ComposerTextView) { self.parent = parent }

        func textViewDidChange(_ textView: UITextView) {
            parent.text = textView.text
        }

        func textViewDidChangeSelection(_ textView: UITextView) {
            parent.cursor = ComposerTextView.characterOffset(
                forUTF16Offset: textView.selectedRange.location, in: textView.text)
        }
    }

    /// `UITextView.selectedRange` is UTF-16 code units; `activeToken` counts
    /// `Character`s. The two agree for plain ASCII commands and paths — the
    /// only content these pickers ever match against — and diverge only
    /// inside a multi-scalar grapheme cluster (an emoji, say), where landing
    /// mid-cluster falls back to the nearest end rather than crashing.
    fileprivate static func characterOffset(forUTF16Offset utf16Offset: Int, in text: String) -> Int {
        guard
            let utf16Index = text.utf16.index(
                text.utf16.startIndex, offsetBy: utf16Offset, limitedBy: text.utf16.endIndex),
            let index = String.Index(utf16Index, within: text)
        else { return text.count }
        return text.distance(from: text.startIndex, to: index)
    }

    fileprivate static func utf16Offset(forCharacterOffset characterOffset: Int, in text: String) -> Int {
        guard let index = text.index(text.startIndex, offsetBy: characterOffset, limitedBy: text.endIndex)
        else { return (text as NSString).length }
        return text.utf16.distance(from: text.utf16.startIndex, to: index.samePosition(in: text.utf16) ?? text.utf16.endIndex)
    }
}


/// Apple's Liquid Glass where the system has it, a material where it does not.
///
/// The Mac's composer carries the same modifier for the same reason (see
/// `GlassCard` there): a control resting ON scrolling content has to read as a
/// layer above it, and on iOS 26 that is what glass means. `.ultraThinMaterial`
/// is the closest thing on 17 and 18, which this app still supports, so the
/// shape and the spacing stay identical and only the surface differs.
private struct GlassField: ViewModifier {
    private let radius: CGFloat = 24

    func body(content: Content) -> some View {
        if #available(iOS 26.0, *) {
            content.glassEffect(.regular, in: .rect(cornerRadius: radius))
        } else {
            content
                .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: radius))
                .overlay {
                    RoundedRectangle(cornerRadius: radius)
                        .strokeBorder(Color.primary.opacity(0.08))
                }
        }
    }
}
