import AgentKit
import SwiftUI

/// A compact task row's second line (ov-104, after ov-92's quiet metadata):
/// "<lead> · <agent> · <progress>", e.g. "claude working · 3 of 5",
/// "Waiting on ov-90 · 1 of 4" or "Done 6d ago". The status itself is the
/// group's header, so the line leads with what the header doesn't say: what
/// the task waits on, how long it has sat, the ask, or when it landed. Color
/// only where the row needs the person: a decision waiting, an agent asking,
/// or a task blocked on another.
enum TaskRowMeta {
    enum Tone: Equatable {
        /// Secondary, the row's ordinary metadata.
        case quiet
        /// Amber: the row needs the person.
        case attention
        /// Red: its agent's turn, or its pane, failed (`Status.tone`).
        case failed
    }

    /// Who's on the task, in a word: "claude working", "codex needs you",
    /// "2 agents working", "Subagent", "No Agent".
    struct Agent: Equatable {
        var word: String
        /// The most urgent status among its agents, whose ink the line takes.
        var status: Status?
        var needsYou: Bool { status == .blocked }
        /// Subagents, whose sentence the lead usually says already.
        var isSubagent = false
    }

    struct Line: Equatable {
        /// What it says first: why it's blocked, its staleness, the ask, or
        /// its time.
        var lead: String?
        var agent: Agent?
        /// Its acceptance progress, "3 of 5", or nil.
        var progress: String?
        var tone: Tone

        var isEmpty: Bool { lead == nil && agent == nil && progress == nil }
        /// Read as one line, for VoiceOver and the tests.
        var text: String {
            [lead, agent?.word, progress.map { "\($0) met" }].compactMap { $0 }.joined(separator: " · ")
        }
    }

    /// Whether the row needs the person: the only rows with any color.
    static func needsAttention(_ row: TaskRow) -> Bool {
        row.status == .needsDecision || !row.blockedBy.isEmpty
    }

    /// The progress shown: "3 of 5" while a line is open, or on an active
    /// row with every line met; nothing on a Done or Canceled row with all
    /// of them met, which is what's expected, nor with no lines at all.
    static func progress(_ row: TaskRow) -> String? {
        guard let progress = row.acceptanceProgress, progress.total > 0 else { return nil }
        if progress.isComplete, row.status.isFinished { return nil }
        return "\(progress.met) of \(progress.total)"
    }

    /// What a Needs Decision row asks, short enough for the line.
    static let ask = "Answer to unblock"

    /// The lead is the first of: what blocks it; when it starts or who's on it
    /// (`startLine`, AgentKit's sentence, ov-212 and ov-213); how long it has
    /// sat; the ask; its time. The start line explains a stop, so it comes
    /// before "No movement": a card waiting its turn to build isn't stuck.
    ///
    /// A subagent's word is dropped when the lead already says "subagent":
    /// "Claude subagent working, 12 min · Subagent" says it twice.
    static func line(_ row: TaskRow, agent: Agent? = nil, startLine: String? = nil, at now: Date) -> Line {
        let lead =
            row.blockedSummary ?? startLine ?? stale(row, at: now)
            ?? (row.status == .needsDecision ? ask : nil)
            ?? (row.status.isFinished || agent == nil ? row.timeNote(at: now) : nil)
        let repeated = agent?.isSubagent == true && startLine?.localizedCaseInsensitiveContains("subagent") == true
        return Line(
            lead: lead, agent: repeated ? nil : agent, progress: progress(row),
            tone: needsAttention(row) || agent?.needsYou == true
                ? .attention : agent?.status?.tone == .failed ? .failed : .quiet)
    }

    /// "No movement for 3d", or nil for a row still moving.
    static func stale(_ row: TaskRow, at now: Date) -> String? {
        guard row.staleness(at: now) == .stale else { return nil }
        let days = max(1, Int(row.stoppedFor(at: now) / 86_400))
        return "No movement for \(days)d"
    }

    /// The agents on a task, in a word, from their panes: the harness and
    /// what the most urgent is doing. "No Agent" for a task in progress with
    /// nobody on it; nil with nothing to say.
    static func agent(live: [BoardPane], presence: TaskAgentPresence) -> Agent? {
        if case .noAgent = presence { return Agent(word: presence.title ?? "No Agent") }
        // Subagents have no pane of their own, and with none of the task's own
        // on it the word is theirs. With a pane too, the pane's word stands.
        if case .subagents = presence, live.isEmpty {
            return Agent(word: presence.title ?? "Subagent", isSubagent: true)
        }
        guard !live.isEmpty else { return nil }
        let statuses = live.map(\.terminal.status)
        let status = Status.mostUrgent(in: statuses) ?? statuses[0]
        let who = live.count == 1 ? Terminal.name(of: live[0].terminal.preset) : "\(live.count) agents"
        return Agent(word: [who, word(status)].compactMap { $0 }.joined(separator: " "), status: status)
    }

    /// What an agent's state reads as beside its name: the status table's
    /// word (`Status.word`), so "done" isn't "idle" here alone (ov-137).
    static func word(_ status: Status) -> String? { status.word }
}

/// The second line, drawn: one size, one quiet color unless the row needs
/// the person, and the checklist glyph monochrome beside the progress. The
/// agent's word goes first when the row is too narrow for all of it: the
/// lead and the progress are what the line is for.
struct TaskRowMetaView: View {
    let row: TaskRow
    var agent: TaskRowMeta.Agent?
    /// Whether the runner can be believed about its agents right now
    /// (`BoardAgents.runnerRecordsTasks`): false drops the subagent half of the
    /// start line and leaves the board's own wait.
    var speaksOfAgents = true

    @Environment(\.colorScheme) private var scheme

    /// The status table's inks (`GlanceState.Tone.color`): amber, not the
    /// accent, which is for controls (ov-137).
    static func color(_ tone: TaskRowMeta.Tone, scheme: ColorScheme) -> Color {
        switch tone {
        case .quiet: GlanceState.Tone.quiet.color(scheme)
        case .attention: GlanceState.Tone.needsYou.color(scheme)
        case .failed: GlanceState.Tone.failed.color(scheme)
        }
    }

    var body: some View {
        BoardTick { now in
            let line = TaskRowMeta.line(
                row, agent: agent, startLine: row.startLine(at: now, speaksOfAgents: speaksOfAgents), at: now)
            if !line.isEmpty {
                ViewThatFits(in: .horizontal) {
                    words(line).fixedSize()
                    words(Self.without(line))
                }
                .font(.system(size: WorkspaceStyle.PaneText.minimum))
                .foregroundStyle(Self.color(line.tone, scheme: scheme))
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(line.text)
            }
        }
    }

    /// The line without the agent's word, or, with no word to drop, as it is
    /// with its lead truncating.
    static func without(_ line: TaskRowMeta.Line) -> TaskRowMeta.Line {
        var shorter = line
        if shorter.lead != nil { shorter.agent = nil }
        return shorter
    }

    private func words(_ line: TaskRowMeta.Line) -> some View {
        HStack(spacing: 4) {
            if let lead = line.lead { Text(lead).lineLimit(1) }
            if let agent = line.agent {
                if line.lead != nil { Text("·") }
                Text(agent.word).lineLimit(1)
            }
            if let progress = line.progress {
                if line.lead != nil || line.agent != nil { Text("·") }
                HStack(spacing: 4) {
                    Image(systemName: "checklist")
                    Text(progress).monospacedDigit()
                }
                .fixedSize()
            }
        }
    }
}
