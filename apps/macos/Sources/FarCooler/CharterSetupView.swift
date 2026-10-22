import AgentKit
import SwiftUI

/// The first run's Reading and Charter screens (ov-519 phase 2b), in place of
/// a workspace whose orchestrator hasn't started and has no charter yet.
///
/// Reading says what the survey is looking at while it runs. The Charter
/// screen is one decision: every answer filled in, each saying where it came
/// from, and Start Orchestrator to begin. Every string the survey sent is
/// drawn verbatim, never as Markdown: a repository wrote it (2a review, R2).
struct CharterSetupView: View {
    @ObservedObject var setup: CharterSetup
    @Environment(\.colorSchemeContrast) private var contrast

    /// The reading column, as Welcome's.
    static let readingWidth: CGFloat = 560
    /// The charter's column: wide enough for a label beside its answer.
    static let charterWidth: CGFloat = 760
    /// The labels' column in each row.
    static let labelWidth: CGFloat = 168

    var body: some View {
        GeometryReader { proxy in
            ScrollView {
                Group {
                    switch setup.phase {
                    case .reading, .failed: reading
                    case .ready, .starting: charter
                    }
                }
                .padding(32)
                .frame(maxWidth: .infinity, minHeight: proxy.size.height)
            }
            .scrollBounceBehavior(.basedOnSize)
            // The decision never needs a scroll: the actions stay at the
            // window's foot while the answers scroll under them (review S5).
            .safeAreaInset(edge: .bottom, spacing: 0) {
                if showsCharter { bar }
            }
        }
        .task {
            if setup.form == nil, setup.phase == .reading { await setup.read() }
        }
        .identified("charter-setup")
    }

    // MARK: - Reading

    private var reading: some View {
        VStack(alignment: .leading, spacing: 20) {
            VStack(alignment: .leading, spacing: 8) {
                Text(verbatim: FirstRunCopy.Charter.readingTitle(setup.repository))
                    .font(.title2.weight(.semibold))
                    .lineLimit(2).truncationMode(.middle)
                Text(verbatim: FirstRunCopy.Charter.readingBody(setup.repository))
                    .font(.callout).foregroundStyle(.quietInk)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if case .failed(let why) = setup.phase {
                VStack(alignment: .leading, spacing: Spacing.inset) {
                    Callout(icon: "xmark.octagon.fill", tone: .error, text: why)
                    Text(FirstRunCopy.Charter.readFailedNext)
                        .font(.callout).foregroundStyle(.quietInk)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: Spacing.group) {
                        Button(FirstRunCopy.Charter.notNow) { setup.notNow() }
                            .identified("charter-not-now")
                        Spacer(minLength: 0)
                        Button(FirstRunCopy.Charter.startWithoutCharter) { setup.startWithoutCharter() }
                            .disabled(setup.installed.isEmpty)
                            .identified("charter-start-anyway")
                        Button(FirstRunCopy.Charter.tryAgain) { Task { await setup.read() } }
                            .buttonStyle(.borderedProminent)
                            .keyboardShortcut(.defaultAction)
                            .identified("charter-try-again")
                    }
                }
                .identified("charter-read-failed")
            } else {
                VStack(alignment: .leading, spacing: Spacing.inset) {
                    ForEach(FirstRunCopy.Charter.readingRows, id: \.self) { row in
                        HStack(spacing: Spacing.group) {
                            ProgressView().controlSize(.small).frame(width: 18)
                            Text(row).font(.callout)
                            Spacer(minLength: 0)
                        }
                        .identified("charter-reading-row")
                    }
                }
                .padding(Spacing.inset)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Fill.inset(contrast), in: .card)
            }
        }
        .frame(maxWidth: Self.readingWidth, alignment: .leading)
        .identified("charter-reading")
    }

    // MARK: - Charter

    private var charter: some View {
        VStack(alignment: .leading, spacing: 20) {
            VStack(alignment: .leading, spacing: 8) {
                Text(FirstRunCopy.Charter.eyebrow)
                    .font(.caption.weight(.semibold)).foregroundStyle(.quietInk)
                Text(verbatim: FirstRunCopy.Charter.title(setup.repository))
                    .font(.title2.weight(.semibold))
                    .lineLimit(3).truncationMode(.middle)
                    .fixedSize(horizontal: false, vertical: true)
                Text(verbatim: FirstRunCopy.Charter.lede(setup.repository))
                    .font(.callout).foregroundStyle(.quietInk)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if setup.draftWaiting { resume }
            if let rows = setup.form?.rows {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(rows.enumerated()), id: \.element.id) { index, row in
                        // A rule between rows, as a table's cells; none under the last.
                        if index < rows.count - 1 {
                            rowView(row).separator(.grid, edge: .bottom)
                        } else {
                            rowView(row)
                        }
                    }
                }
                .background(Fill.inset(contrast), in: .card)
                .clipShape(.card)
                .identified("charter-rows")
            }
        }
        .frame(maxWidth: Self.charterWidth, alignment: .leading)
        .identified("charter-screen")
    }

    private var showsCharter: Bool {
        switch setup.phase {
        case .ready, .starting: return true
        case .reading, .failed: return false
        }
    }

    /// The pinned foot: a write that failed, the actions, and what each does.
    private var bar: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            if let failure = setup.failure {
                Callout(icon: "xmark.octagon.fill", tone: .error, text: failure)
                    .identified("charter-write-failed")
            }
            actions
            Text(FirstRunCopy.Charter.footnote)
                .font(.caption).foregroundStyle(.quietInk)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: Self.charterWidth, alignment: .leading)
        .padding(.horizontal, 32)
        .padding(.vertical, Spacing.inset)
        .frame(maxWidth: .infinity)
        // The page's own paper, opaque, so nothing scrolled under shows through.
        .background(Surface.contentFill)
        // A rule where the answers scroll under the bar.
        .separator(.grid, edge: .top)
        .identified("charter-bar")
    }

    private var resume: some View {
        VStack(alignment: .leading, spacing: Spacing.group) {
            Text(FirstRunCopy.Charter.resumeTitle).font(.callout.weight(.semibold))
            Text(FirstRunCopy.Charter.resumeBody)
                .font(.callout).foregroundStyle(.quietInk)
                .fixedSize(horizontal: false, vertical: true)
            // With a draft waiting, carrying on is the default: Start
            // Orchestrator would replace it (review S7).
            Button(FirstRunCopy.Charter.resumeButton) { setup.continueTalking() }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.defaultAction)
                .disabled(setup.installed.isEmpty || isStarting)
                .identified("charter-resume-button")
        }
        .padding(Spacing.inset)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Fill.inset(contrast), in: .card)
        .identified("charter-resume")
    }

    private func rowView(_ row: CharterRow) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.section) {
            Text(row.title)
                .font(.callout.weight(.medium))
                .frame(width: Self.labelWidth, alignment: .leading)
            VStack(alignment: .leading, spacing: 6) {
                control(row)
                ForEach(Array(row.text.enumerated()), id: \.offset) { _, line in
                    // The survey's own words: verbatim, a repository wrote them.
                    Text(verbatim: line)
                        .font(.callout)
                        .foregroundStyle(row.control == .none && row.note == nil ? Color.primary : Ink.quiet)
                        .fixedSize(horizontal: false, vertical: true)
                        .textSelection(.enabled)
                }
                if let note = row.note {
                    Text(note).font(.caption).foregroundStyle(.quietInk)
                        .fixedSize(horizontal: false, vertical: true)
                        .identified("charter-note-\(row.id)")
                }
                provenance(row.provenance)
                    .identified("charter-provenance-\(row.id)")
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, Spacing.section)
        .padding(.vertical, Spacing.inset)
        .accessibilityElement(children: .contain)
        .identified("charter-row-\(row.id)")
    }

    @ViewBuilder private func control(_ row: CharterRow) -> some View {
        switch row.control {
        case .landing:
            Picker(row.title, selection: choice(\.landing, or: .direct)) {
                ForEach(CharterLanding.allCases, id: \.self) { Text($0.title).tag($0) }
            }
            .pickerStyle(.segmented).labelsHidden().fixedSize()
            .disabled(isStarting)
        case .lanes:
            Picker(row.title, selection: choice(\.lanes, or: .count(CharterForm.defaultLanes))) {
                ForEach(setup.form?.laneOptions ?? [], id: \.self) { Text($0.title).tag($0) }
            }
            .pickerStyle(.segmented).labelsHidden().fixedSize()
            .disabled(isStarting)
        case .initiative:
            Toggle(row.title, isOn: choice(\.initiative, or: true))
                .toggleStyle(.switch).labelsHidden().controlSize(.small)
                .disabled(isStarting)
        case .none:
            EmptyView()
        }
    }

    /// "FROM  main requires pull requests", in the caption under an answer,
    /// the detail hanging beside the tag when it wraps.
    private func provenance(_ p: CharterRow.Provenance) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(p.tag.uppercased()).font(.caption2.weight(.semibold))
                .foregroundStyle(tagStyle(p))
            if let detail = p.detail {
                // The survey's words: verbatim.
                Text(verbatim: detail).font(.caption).foregroundStyle(.quietInk)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(p.detail.map { "\(p.tag): \($0)" } ?? p.tag)
    }

    /// Changed stands out; From and Default read as captions, at a contrast
    /// that holds at caption size, and as primary under Increase Contrast
    /// (review S6).
    private func tagStyle(_ p: CharterRow.Provenance) -> Color {
        p.isChanged || contrast == .increased ? Color.primary : Ink.quiet
    }

    private func choice<Value>(_ path: WritableKeyPath<CharterChoices, Value>, or fallback: Value) -> Binding<Value> {
        Binding(
            get: { setup.form?.choices[keyPath: path] ?? fallback },
            set: { setup.form?.choices[keyPath: path] = $0 })
    }

    private var isStarting: Bool {
        if case .starting = setup.phase { return true }
        return false
    }

    // MARK: - Actions

    private var actions: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: Spacing.group) {
                runWith
                Spacer(minLength: Spacing.section)
                buttons
            }
            VStack(alignment: .leading, spacing: Spacing.inset) {
                runWith
                HStack(spacing: Spacing.group) {
                    Spacer(minLength: 0)
                    buttons
                }
            }
        }
    }

    @ViewBuilder private var runWith: some View {
        if setup.installed.count > 1 {
            HStack(spacing: Spacing.group) {
                Text(FirstRunCopy.Charter.runWith).font(.callout).foregroundStyle(.quietInk)
                Picker(
                    FirstRunCopy.Charter.runWith,
                    selection: Binding(get: { setup.harness ?? setup.installed[0] }, set: { setup.harness = $0 })
                ) {
                    ForEach(setup.installed) { harness in
                        Text(AgentHarness(rawValue: harness.rawValue)?.title ?? harness.title).tag(harness)
                    }
                }
                .pickerStyle(.segmented).labelsHidden().fixedSize()
                .disabled(isStarting)
            }
            .identified("charter-run-with")
        } else if let only = setup.installed.first {
            Text(verbatim: "\(FirstRunCopy.Charter.runWith) \(AgentHarness(rawValue: only.rawValue)?.title ?? only.title)")
                .font(.callout).foregroundStyle(.quietInk)
                .identified("charter-run-with")
        }
    }

    private var buttons: some View {
        HStack(spacing: Spacing.group) {
            Button(FirstRunCopy.Charter.notNow) { setup.notNow() }
                .disabled(isStarting)
                .identified("charter-not-now")
            Button { Task { await setup.start(talk: true) } } label: {
                StartingLabel(title: FirstRunCopy.Charter.talkItThrough, starting: setup.phase == .starting(talk: true))
            }
            .disabled(!setup.canStart)
            .identified("charter-talk")
            Button { Task { await setup.start(talk: false) } } label: {
                StartingLabel(title: FirstRunCopy.Charter.start, starting: setup.phase == .starting(talk: false))
            }
            .modifier(DefaultAction(on: !setup.draftWaiting))
            .disabled(!setup.canStart)
            .identified("charter-start")
        }
        .fixedSize()
    }
}

extension CharterRow.Provenance {
    var isChanged: Bool {
        if case .changed = self { return true }
        return false
    }
}

/// The screen's one default button, prominent and on Return, or a plain one.
private struct DefaultAction: ViewModifier {
    let on: Bool

    @ViewBuilder func body(content: Content) -> some View {
        if on {
            content.buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
        } else {
            content
        }
    }
}

/// A button's title, or "Starting…" with a spinner in the same place while it
/// does what it says. Both are laid out, one hidden, so the button is as wide
/// either way and the row around it never moves (review R2).
private struct StartingLabel: View {
    let title: String
    let starting: Bool

    var body: some View {
        ZStack {
            Text(title).opacity(starting ? 0 : 1)
            HStack(spacing: Spacing.tight) {
                ProgressView().controlSize(.mini)
                Text(FirstRunCopy.Charter.starting)
            }
            .opacity(starting ? 1 : 0)
            .identified(starting ? "charter-starting" : "charter-starting-slot")
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(starting ? FirstRunCopy.Charter.starting : title)
    }
}
