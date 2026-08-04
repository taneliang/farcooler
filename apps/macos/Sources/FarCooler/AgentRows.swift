import AgentKit
import SwiftUI

struct AgentRowView: View {
    let row: TranscriptRow
    /// Whether this is the newest row, which is what makes a thought "live".
    ///
    /// A thought is still being written exactly while nothing has followed it.
    /// That needs no extra state in the model: the transcript already knows
    /// the order, and asking "is anything after me" is the same question.
    var isLast: Bool = false
    /// The request this row is blocked on, if it is the one being asked about.
    ///
    /// A permission names the tool call it gates, so it can be shown ON that
    /// call rather than in a panel elsewhere. What you are approving and what
    /// it will run are then the same object, and there is nothing to match up
    /// by eye.
    var pending: PendingPermission?
    var onAnswer: ((String) -> Void)?

    var body: some View {
        switch row.kind {
        // The parent pointer is not read here: nesting already placed this row
        // inside the block it belongs to, so drawing it a second time as text
        // would say the same thing twice.
        case let .message(role, text, _):
            MessageRow(role: role, text: text, isLive: isLast)
        case let .tool(tool):
            ToolRowView(tool: tool, isLive: isLast, pending: pending, onAnswer: onAnswer)
        case let .subagent(block):
            // The request is handed down rather than stopping here, because the
            // call it gates is one of the block's children — see
            // `SubagentBlockView.permission(gating:)`.
            SubagentBlockView(block: block, pending: pending, onAnswer: onAnswer)
        case let .gap(reason):
            GapRow(reason: reason)
        }
    }
}

/// The answers to a permission request.
///
/// Shared by the inline case and the standalone card so there is one set of
/// buttons with one set of shortcuts, rather than two that drift.
struct ApprovalControls: View {
    let options: [PermissionOption]
    let onChoose: (String) -> Void

    var body: some View {
        // Ordered by what the question actually is.
        //
        // ACP hands back a flat list and rendering it flat gave every option
        // the same weight — so the straight yes, the straight no, and a policy
        // change that will answer every FUTURE question too all looked like
        // peers. They are not: two of them answer this request and the rest
        // change the rules.
        HStack(spacing: 8) {
            if let allowOption {
                Button { onChoose(allowOption.id) } label: {
                    label(allowOption)
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.return, modifiers: .command)
            }
            if let rejectOption {
                Button { onChoose(rejectOption.id) } label: {
                    label(rejectOption)
                }
                .keyboardShortcut(.delete, modifiers: .command)
            }

            ForEach(secondaryOptions) { option in
                Button(option.name) { onChoose(option.id) }
                    // `.plain` and grey, NOT `.link`.
                    //
                    // A link style paints the label accent-blue, which is the
                    // same mistake the composer's selector row made: blue reads
                    // as somewhere to go, not as a thing that changes what the
                    // agent may do from here on. It also put a second blue next
                    // to the prominent Allow, so the two competed.
                    .buttonStyle(.plain)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 0)
        }
    }

    /// The name, plus the shortcut on the button it belongs to — a keystroke
    /// nobody can see is a keystroke nobody uses.
    private func label(_ option: PermissionOption) -> some View {
        HStack(spacing: 5) {
            Text(option.name)
            if let hint = hint(for: option) {
                Text(hint).foregroundStyle(.secondary)
            }
        }
    }

    /// Everything that is not the straight yes or no — the "always" variants,
    /// and anything an adapter offers that this client has never heard of.
    /// Kept rather than dropped: swallowing one would make an answer
    /// unreachable.
    private var secondaryOptions: [PermissionOption] {
        options.filter { $0.id != allowOption?.id && $0.id != rejectOption?.id }
    }

    /// ACP names these `allow_once`, `allow_always`, `reject_once`,
    /// `reject_always`. The first of each kind is what the shortcut answers —
    /// the once-only one, since it is the conservative reading of a keystroke
    /// pressed without looking.
    private var allowOption: PermissionOption? {
        options.first { $0.kind.hasPrefix("allow_once") }
            ?? options.first { $0.kind.hasPrefix("allow") }
            ?? options.first
    }

    private var rejectOption: PermissionOption? {
        options.first { $0.kind.hasPrefix("reject") }
    }

    private func hint(for option: PermissionOption) -> String? {
        if option.id == allowOption?.id { return "⌘↩" }
        if option.id == rejectOption?.id { return "⌘⌫" }
        return nil
    }
}

/// One message. Three shapes for three roles, because they answer three
/// different questions: what did I say, what did it say, and what did it
/// think before saying that.
private struct MessageRow: View {
    let role: Role
    let text: String
    var isLive: Bool = false

    var body: some View {
        switch role {
        case .user:
            // Right-aligned with a fill, the one place in the transcript that
            // is not the agent talking — it has to read as a different voice
            // at a glance, not just on close reading.
            HStack {
                Spacer(minLength: 32)
                Text(text)
                    .font(.body)
                    .textSelection(.enabled)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    // `quaternary` rather than a hand-mixed opacity: it is the
                    // fill AppKit uses for exactly this — a grouped surface
                    // that must stay legible in both appearances without
                    // anyone picking two numbers and hoping.
                    .background(.quaternary, in: RoundedRectangle(cornerRadius: 14))
            }

        case .agent:
            // Plain body text, full width. This is the common case and the
            // one that should cost the eye nothing extra to read.
            HStack {
                MarkdownText(text: text)
                Spacer(minLength: 32)
            }

        case .thought:
            // Open while it is being written, closed once it is done.
            //
            // Watching an agent think is the interesting part, and a
            // permanently collapsed row hides the only thing happening during
            // a long turn. But a finished thought is scratch work, and a
            // transcript of them is unreadable — so it folds itself away the
            // moment anything follows it.
            ThoughtRow(text: text, isLive: isLive)
        }
    }
}

/// The agent's reasoning: streaming while it happens, folded away after.
private struct ThoughtRow: View {
    let text: String
    let isLive: Bool

    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Button {
                withAnimation(Motion.snap) { expanded.toggle() }
            } label: {
                HStack(spacing: 5) {
                    Image(systemName: "chevron.right")
                        .font(.caption2)
                        .rotationEffect(.degrees(showing ? 90 : 0))
                    Text(isLive ? "Thinking…" : "Thought")
                        .font(.caption.weight(.medium))
                }
                .foregroundStyle(.secondary)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if showing {
                // While it is being written, only the last few lines — enough
                // to see it moving, which is the whole point. A thinking agent
                // that shows one collapsed word looks stuck, and one that
                // shows everything pushes the conversation off screen.
                MarkdownText(text: isLive && !expanded ? Self.tail(of: text) : text, secondary: true)
                    .padding(.leading, 2)
            }
        }
        // Driven by the model, not by a timer: a thought stops being live the
        // instant something follows it, and the fold should follow that.
        .animation(Motion.snap, value: isLive)
        .animation(Motion.snap, value: text)
    }

    /// Open while live unless the reader has closed it; closed after unless
    /// the reader has opened it.
    private var showing: Bool { expanded || isLive }

    /// The last few lines, for a thought still being written.
    private static func tail(of text: String, lines: Int = 5) -> String {
        let all = text.split(separator: "\n", omittingEmptySubsequences: false)
        guard all.count > lines else { return text }
        return all.suffix(lines).joined(separator: "\n")
    }
}

/// One tool call, mutated in place by every update — never a new row — so a
/// call that reports progress four times still occupies the one line it
/// earned.
private struct ToolRowView: View {
    let tool: ToolRow
    /// Whether this is the newest row — nothing has followed it yet.
    var isLive: Bool = false
    var pending: PendingPermission?
    var onAnswer: ((String) -> Void)?

    private var expandable: Bool { tool.content != nil || tool.diff != nil }

    @State private var expanded = false

    var body: some View {
        // One container for the summary and what it opens.
        //
        // These used to be two siblings, with the chevron OUTSIDE the summary's
        // fill — so an expanded detail sat left of the box it came from and
        // read as a separate thing that had appeared underneath. A disclosure
        // and what it discloses are one object; drawing them as one says so.
        VStack(alignment: .leading, spacing: 0) {
            if expandable {
                Button {
                    withAnimation(Motion.snap) { expanded.toggle() }
                } label: {
                    label.contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            } else {
                label
            }

            if showingDetail {
                Divider()
                VStack(alignment: .leading, spacing: 8) {
                    if let content = tool.content, !content.isEmpty {
                        // No fill of its own: it is already inside one.
                        DetailBox(text: content, chrome: false)
                    }
                    if let diff = tool.diff {
                        DiffView(diff: diff)
                    }
                }
                .padding(9)
                .frame(maxWidth: .infinity, alignment: .leading)
            }

            // The question, on the thing being asked about.
            if let pending, let onAnswer {
                Divider()
                ApprovalControls(options: pending.options, onChoose: onAnswer)
                    .padding(9)
            }
        }
        .background(.quinary, in: RoundedRectangle(cornerRadius: 7))
        .overlay {
            // Outlined only while it is waiting on you. A tool call that needs
            // nothing should not shout, and one that does should be findable
            // without reading the transcript.
            if pending != nil {
                RoundedRectangle(cornerRadius: 7).strokeBorder(Color.orange.opacity(0.45))
            }
        }
        .animation(Motion.snap, value: expanded)
        .animation(Motion.snap, value: pending != nil)
        // Driven by the model rather than a timer, exactly as the thought row
        // is: the fold follows the turn moving on.
        .animation(Motion.snap, value: isLive)
        .animation(Motion.snap, value: running)
    }

    /// Open on its own while it is waiting to be approved, and while it is
    /// the thing currently happening.
    ///
    /// Being asked to allow a command without being shown the command is not a
    /// decision, it is a guess. A reader who then folds it away has said they
    /// have seen enough, so that still wins.
    ///
    /// The live case is the same rule `ThoughtRow` already follows: a command
    /// that is running with nothing after it IS the turn, and watching its
    /// output is the only way to see what is happening. Once the agent moves
    /// on it folds itself away, because a transcript of every command's full
    /// output is unreadable.
    private var showingDetail: Bool { expanded || pending != nil || (isLive && running) }

    /// Still going, as the agent last reported it.
    private var running: Bool { tool.status == .pending || tool.status == .inProgress }

    private var label: some View {
        HStack(spacing: 7) {
            // Inside the box, not beside it — the chevron belongs to the row it
            // opens, and outside the fill it aligned the detail to the wrong
            // edge.
            if expandable {
                Image(systemName: "chevron.right")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .rotationEffect(.degrees(showingDetail ? 90 : 0))
            }
            // The same dot the sidebar uses for a terminal's status, mapped
            // onto a tool call's own four states — one vocabulary for
            // "something is happening" everywhere it appears, rather than a
            // second one invented for this row.
            StatusGlyph(status: status, size: 7)
            Text(tool.title)
                .font(.callout.weight(.medium))
                // One line, always. An adapter puts the whole absolute path in
                // the title, which wrapped to six lines in a narrow pane and
                // turned a one-line status row into the largest thing on
                // screen.
                .lineLimit(1)
                .truncationMode(.middle)
            if let location = tool.locations.first {
                Text(location)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 4)
        }
        // Inset on a faint fill — applied to the whole container in `body`, so
        // a tool call reads as machinery rather than as something the agent
        // said. Without it the transcript is one undifferentiated column of
        // text and the eye cannot find the prose.
        .padding(.horizontal, 9)
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var status: Status {
        switch tool.status {
        case .pending: return .starting
        case .inProgress: return .working
        case .completed: return .done
        case .failed: return .failed
        }
    }
}

/// A subagent's dispatch and everything it did.
///
/// Open while it works, closed once it reports — the same rule `ThoughtRow`
/// uses, and for the same reason: the interesting moment is while it happens,
/// and a finished one is noise until you ask. The difference is that a reader
/// who touches it wins permanently, because a block that shut itself while
/// someone was reading it is worse than one that stayed open.
private struct SubagentBlockView: View {
    let block: SubagentBlock
    /// The request the transcript is blocked on, if it names one of this
    /// block's children.
    var pending: PendingPermission?
    var onAnswer: ((String) -> Void)?

    /// `nil` means nobody has said, so the automatic rule applies.
    @State private var toggled: Bool?
    @State private var showingAll = false

    /// How many children a running block shows. Enough to see what it is
    /// doing; few enough that six at once still fit on a screen.
    private static let visibleChildren = 3

    /// Derived on the model, not here: interruption leaves the tool's status
    /// alone, so asking it directly keeps a cut-off block spinning forever.
    private var running: Bool { block.isRunning }

    /// Open while it runs, unless the reader has said otherwise — and always
    /// open while it is waiting on an answer, since the buttons live inside.
    private var showing: Bool { pending != nil || (toggled ?? running) }

    /// The LAST few, not the first: a running block's newest step is the one
    /// worth watching, and a block that pinned its opening three rows would
    /// show the same three for the whole of a long run.
    private var shown: [TranscriptRow] {
        // A pending request suspends the cap. The gating call is usually the
        // newest child and so within the window anyway, but "usually" is not
        // good enough for an approval: a button nobody can reach wedges the
        // agent with no way to see why, which is the failure
        // `unattachedPermission` exists to prevent and this must not
        // re-introduce one level down. Bounded height is a convenience; a
        // deadlock is not.
        guard pending == nil else { return block.children }
        guard !showingAll, block.children.count > Self.visibleChildren else { return block.children }
        return Array(block.children.suffix(Self.visibleChildren))
    }

    private var hidden: Int { block.children.count - shown.count }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(Motion.snap) { toggled = !showing }
            } label: {
                header.contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if showing && !block.children.isEmpty {
                Divider()
                VStack(alignment: .leading, spacing: 10) {
                    // Above the rows it hides, because that is where they are:
                    // these are the OLDEST children, and an affordance for them
                    // placed below the newest ones would point the wrong way.
                    if hidden > 0 {
                        Button("… \(hidden) more") { withAnimation(Motion.snap) { showingAll = true } }
                            .buttonStyle(.plain)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    ForEach(shown) { child in
                        AgentRowView(
                            row: child,
                            // While the block runs, its newest child is what is
                            // happening — the same thing `isLast` means at the
                            // top level, asked one level down. Without it a
                            // running subagent shows three shut rows and none of
                            // the output that is the reason to watch it.
                            isLast: running && child.id == block.children.last?.id,
                            pending: permission(gating: child),
                            onAnswer: onAnswer)
                    }
                }
                .padding(9)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .background(.quinary, in: RoundedRectangle(cornerRadius: 7))
        .animation(Motion.snap, value: showing)
        // Driven by the model rather than by the toggle alone: children arrive
        // while the block is open, and an unanimated insert makes the
        // transcript below it jump.
        .animation(Motion.snap, value: block.children.count)
    }

    private var header: some View {
        HStack(spacing: 7) {
            Image(systemName: "chevron.right")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .rotationEffect(.degrees(showing ? 90 : 0))
            StatusGlyph(status: status, size: 7)
            Text(block.tool.title)
                .font(.callout.weight(.medium))
                // One line, for the reason `ToolRowView` gives: an adapter puts
                // whole paths in a title, and a wrapped one turns a status row
                // into the largest thing on screen.
                .lineLimit(1)
                .truncationMode(.middle)
            Text(block.subtitle)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer(minLength: 4)
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// What a collapsed block still answers without being opened.

    /// The same four-state vocabulary a tool call uses, with one override: an
    /// interrupted block is NOT a completed one. The transcript marks it
    /// interrupted precisely because the turn ended before it reported, and
    /// wearing the green dot of a subagent that did report is the single
    /// dishonesty this feature is not allowed to commit.
    private var status: Status {
        if block.interrupted { return .failed }
        switch block.tool.status {
        case .pending: return .starting
        case .inProgress: return .working
        case .completed: return .done
        case .failed: return .failed
        }
    }

    /// The pending request, if it is this child's call that it gates.
    ///
    /// The same test `AgentSurface` runs over the top level, run over the
    /// children — so a permission raised inside a subagent lands on the call it
    /// is about rather than on the block as a whole.
    private func permission(gating child: TranscriptRow) -> PendingPermission? {
        guard let pending, case let .tool(tool) = child.kind, tool.id == pending.toolCall
        else { return nil }
        return pending
    }
}

/// A break in the transcript, named rather than hidden.
///
/// This is the one row in the whole feature that is not allowed to be quiet.
/// `StatusGlyph`'s silence-by-default rule is for a terminal that has nothing
/// to say; a gap is the opposite of nothing — it is the transcript admitting
/// history is missing — and rendering it as a thin rule between two messages
/// would let a reader miss the one fact this design exists to never hide.
private struct GapRow: View {
    let reason: GapReason

    // `.loadEmpty` is news, not a failure — a fresh chat pane has nothing to
    // restore because nothing happened yet, and dressing that in the same
    // orange "something broke" language as a real gap would tell the user the
    // opposite of the truth. It still gets a row, because "nothing was lost"
    // is exactly the kind of thing this row exists to say plainly rather than
    // leave the user to infer from silence — see the type doc above.
    private var isInformational: Bool {
        if case .loadEmpty = reason { return true }
        return false
    }

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: isInformational ? "info.circle" : "scissors")
                .font(.system(size: 11))
            Text(sentence)
                .font(.system(size: 11.5, weight: .medium))
            Spacer(minLength: 0)
        }
        // `Color.secondary`, not `.secondary` — that shorthand resolves to
        // `HierarchicalShapeStyle`, a different type from `Color.orange`, and
        // a ternary needs both branches to agree.
        .foregroundStyle(isInformational ? Color.secondary : Color.orange)
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            isInformational ? Color.secondary.opacity(0.08) : Color.orange.opacity(0.10),
            in: RoundedRectangle(cornerRadius: 6))
    }

    private var sentence: String {
        switch reason {
        case .ringTrimmed:
            return "Some earlier history was trimmed and is not shown here."
        case .loadUnsupported:
            return "This session could not be loaded from where it left off."
        case .loadEmpty:
            return "This session has no recorded turns yet — there is nothing to restore."
        case .loadFailed(let detail):
            return "This session could not be loaded from where it left off: \(detail)"
        case .unparsed:
            return "Something happened here that this version cannot show."
        }
    }
}


/// A turn in progress, said where the work is appearing.
///
/// A spinner in a status line is a control that reports on the conversation
/// from outside it. Shimmering text at the end of the transcript IS the
/// conversation, one line ahead of itself — which is what every chat interface
/// worth copying does now, and it reads as the agent about to speak rather than
/// as the app being busy.
struct WorkingRow: View {
    /// One pass of the highlight, in seconds.
    private static let period: TimeInterval = 1.1

    /// When this row appeared, so the sweep starts at the start of the word.
    ///
    /// Phase taken straight from the wall clock put the highlight wherever the
    /// current second happened to land — so the shimmer began mid-word, which
    /// reads as a glitch rather than as a sweep.
    @State private var start: Date?

    var body: some View {
        // Driven by `TimelineView`, not by an animated `@State`.
        //
        // The first version started a `repeatForever` animation in `onAppear`,
        // and this row lives at the end of a `LazyVStack` that is rebuilt on
        // every streamed event — so the animation was restarted from zero many
        // times a second and never visibly moved. A timeline owns its own clock
        // and does not care how often the view is recreated.
        TimelineView(.animation) { context in
            Text("Working…")
                .font(.callout)
                .foregroundStyle(
                    LinearGradient(
                        stops: stops(at: phase(now: context.date)),
                        startPoint: .leading,
                        endPoint: .trailing))
        }
        .onAppear { if start == nil { start = Date() } }
    }

    private func phase(now: Date) -> Double {
        guard let start else { return 0 }
        return now.timeIntervalSince(start)
            .truncatingRemainder(dividingBy: Self.period) / Self.period
    }

    /// The highlight enters from before the first letter and leaves past the
    /// last, rather than appearing at one edge and vanishing at the other.
    private func stops(at phase: Double) -> [Gradient.Stop] {
        let center = -0.35 + phase * 1.7
        let width = 0.3
        return [
            .init(color: .secondary, location: min(max(center - width, 0), 1)),
            .init(color: .primary, location: min(max(center, 0), 1)),
            .init(color: .secondary, location: min(max(center + width, 0), 1)),
        ]
    }
}

/// A message written but not yet sent.
///
/// Drawn like the user's own messages, because it is one — right-aligned, same
/// bubble — but dashed and dimmed to say it has not gone anywhere yet. That
/// distinction is the whole point: before this, a message typed mid-turn was
/// drawn identically to one the agent had received, and there was no way to
/// tell which had actually happened.
struct QueuedRow: View {
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
            Spacer(minLength: 32)

            VStack(alignment: .trailing, spacing: 4) {
                if editing {
                    TextField("", text: $draft, axis: .vertical)
                        .textFieldStyle(.plain)
                        .font(.body)
                        .frame(minWidth: 160)
                        .onSubmit(commit)
                } else if queued.text.isEmpty && queued.imageCount > 0 {
                    // An image with no words is still a message. Without this
                    // the bubble was empty and read as a dropped attachment.
                    Label(
                        queued.imageCount == 1 ? "1 image" : "\(queued.imageCount) images",
                        systemImage: "photo")
                        .font(.body)
                } else {
                    Text(queued.text)
                        .font(.body)
                        .textSelection(.enabled)
                }

                HStack(spacing: 8) {
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
                    Button("Remove", action: onCancel)
                        .buttonStyle(.plain)
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            // Opaque for the same reason the plan panel is: it floats over the
            // transcript, and a half-transparent bubble with conversation
            // showing through it is not a bubble.
            .background {
                RoundedRectangle(cornerRadius: 14)
                    .fill(.regularMaterial)
                    .overlay {
                        RoundedRectangle(cornerRadius: 14)
                            .strokeBorder(
                                .tertiary,
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
