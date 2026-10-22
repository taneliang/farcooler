import AgentKit
import AppKit
import SwiftUI

/// What the Welcome page needs from the window (ov-519).
struct WelcomeSetup {
    let model: WelcomeModel
    /// This Mac's checks, from what its runner said.
    var step: MacStep
    var agents: HarnessAvailability
    /// Register the repository at a path on this Mac, selecting its Main on
    /// success: nil, or the words it failed with.
    var register: (String) async -> String?
    /// Ask this Mac's runner what's installed again.
    var onCheckAgain: () -> Void
    /// The Add Repository sheet, where a repository lives on another runner;
    /// nil when this Mac is the only one.
    var onAnotherRunner: (() -> Void)?
    /// Seconds between looks while a check is failing.
    var recheckEvery: TimeInterval = 5
}

/// The first thing a new Mac shows (ov-519): what Far Cooler is for, whether
/// this Mac is ready, and the repository to start with, found for you.
///
/// It replaces a spinner and then "No Repositories", which said nothing about
/// what happens next. The words are `FirstRunCopy.Welcome`'s, written in
/// ov-205 and never shown until now.
struct WelcomeView: View {
    let setup: WelcomeSetup
    @ObservedObject private var model: WelcomeModel
    @Environment(\.colorScheme) private var scheme
    @Environment(\.colorSchemeContrast) private var contrast

    init(setup: WelcomeSetup) {
        self.setup = setup
        model = setup.model
    }

    /// The widest the page is: a reading column, centered in the window.
    static let width: CGFloat = 560

    var body: some View {
        GeometryReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 28) {
                    header
                    thisMac
                    repository
                }
                .frame(maxWidth: Self.width, alignment: .leading)
                .identified("welcome-column")
                .padding(32)
                .frame(maxWidth: .infinity, minHeight: proxy.size.height)
            }
            .scrollBounceBehavior(.basedOnSize)
        }
        .task { await model.look() }
        // Something to fix: look again while the page is up. The runner keeps
        // a miss for a minute (`programs::find`), so Check Again right after
        // an install can read the old answer; this makes the page tick over
        // by itself once it has the new one, and stops when nothing is wrong.
        .task(id: MacChecks.fix(setup.step) != nil) {
            while MacChecks.fix(setup.step) != nil {
                try? await Task.sleep(for: .seconds(setup.recheckEvery))
                if Task.isCancelled { return }
                setup.onCheckAgain()
            }
        }
        .identified("welcome")
    }

    // MARK: - Header

    private var header: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(FirstRunCopy.Welcome.title).font(.title2.weight(.semibold))
            Text(FirstRunCopy.Welcome.body)
                .font(.callout)
                .foregroundStyle(.quietInk)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    // MARK: - This Mac

    private var thisMac: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(FirstRunCopy.Welcome.macTitle).font(.caption.weight(.semibold)).foregroundStyle(.quietInk)
            VStack(alignment: .leading, spacing: Spacing.inset) {
                ForEach(MacChecks.rows(setup.step, agents: setup.agents)) { check in
                    row(check)
                }
            }
            .padding(Spacing.inset)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Fill.inset(contrast), in: .card)
            .identified("welcome-checks")
        }
    }

    private func row(_ check: MacCheck) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
            mark(check.result)
                .frame(width: 18)
            VStack(alignment: .leading, spacing: 6) {
                Text(check.text)
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
                if let detail = check.detail {
                    Text(detail).font(.caption).foregroundStyle(.quietInk)
                }
                if check.result == .failing { fixes }
            }
            Spacer(minLength: 0)
        }
        .accessibilityElement(children: .contain)
        .identified("welcome-check-\(check.kind.rawValue)")
    }

    @ViewBuilder private func mark(_ result: MacCheck.Result) -> some View {
        switch result {
        case .passing:
            Image(systemName: "checkmark").foregroundStyle(.quietInk).accessibilityLabel("Passed")
        case .failing:
            // The one amber on the page: the thing to fix.
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(Tint.attention(scheme))
                .accessibilityLabel("Needs attention")
        case .pending:
            ProgressView().controlSize(.small)
        }
    }

    /// The fix for the failing check: the command to copy, and Check Again.
    @ViewBuilder private var fixes: some View {
        switch MacChecks.fix(setup.step) {
        case .installTmux:
            HStack(spacing: Spacing.group) {
                Text(FirstRunCopy.Welcome.tmuxCommand)
                    .font(.system(.callout, design: .monospaced))
                    .textSelection(.enabled)
                Button(FirstRunCopy.Welcome.copyCommand) {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(FirstRunCopy.Welcome.tmuxCommand, forType: .string)
                }
                .controlSize(.small)
                Button(FirstRunCopy.Welcome.checkAgain, action: setup.onCheckAgain).controlSize(.small)
            }
            .identified("welcome-fix-tmux")
        case .installAgent:
            Button(FirstRunCopy.Welcome.checkAgain, action: setup.onCheckAgain).controlSize(.small)
                .identified("welcome-fix-agent")
        case .restartService:
            Button(FirstRunCopy.Welcome.tryAgain, action: setup.onCheckAgain).controlSize(.small)
        case nil:
            EmptyView()
        }
    }

    // MARK: - Repository

    private var repository: some View {
        VStack(alignment: .leading, spacing: Spacing.inset) {
            VStack(alignment: .leading, spacing: 6) {
                Text(FirstRunCopy.Welcome.repositoryTitle).font(.caption.weight(.semibold))
                    .foregroundStyle(.quietInk)
                Text(FirstRunCopy.Welcome.repositoryBody)
                    .font(.callout)
                    .foregroundStyle(.quietInk)
                    .fixedSize(horizontal: false, vertical: true)
            }
            found
            if let failure = model.failure {
                VStack(alignment: .leading, spacing: Spacing.group) {
                    Callout(icon: "xmark.octagon.fill", tone: .error, text: failure.sentence)
                    if let transcript = failure.transcript, !transcript.isEmpty { DetailBox(text: transcript) }
                }
                .identified("welcome-failure")
            }
            actions
        }
    }

    @ViewBuilder private var found: some View {
        if !model.suggestions.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                Text(FirstRunCopy.Welcome.foundTitle).font(.callout.weight(.medium))
                VStack(spacing: 2) {
                    ForEach(model.suggestions) { suggestion in
                        suggestionRow(suggestion)
                    }
                }
                .padding(4)
                .background(Fill.inset(contrast), in: .card)
                .identified("welcome-found")
            }
        } else if model.scanned {
            Text(FirstRunCopy.Welcome.foundNone)
                .font(.callout)
                .foregroundStyle(.quietInk)
                .fixedSize(horizontal: false, vertical: true)
                .identified("welcome-found-none")
        }
    }

    private func suggestionRow(_ suggestion: RepositorySuggestion) -> some View {
        let selected = model.selected == suggestion.path
        return Button {
            model.selected = suggestion.path
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: Spacing.group) {
                Text(suggestion.name).font(.callout.weight(.medium)).lineLimit(1)
                Text(suggestion.displayPath(home: model.home))
                    .font(.caption).foregroundStyle(.quietInk)
                    .lineLimit(1).truncationMode(.head)
                Spacer(minLength: Spacing.group)
                Text(suggestion.when(now: Date())).font(.caption).foregroundStyle(.quietInk).lineLimit(1)
                    .fixedSize()
            }
            .padding(.horizontal, Spacing.group)
            .padding(.vertical, 7)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(selected ? Fill.selection(active: true, contrast: contrast) : .clear, in: .control)
            .contentShape(.control)
        }
        .buttonStyle(.plain)
        .disabled(model.adding != nil)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .identified("welcome-suggestion")
    }

    private var actions: some View {
        HStack(spacing: Spacing.group) {
            Button(FirstRunCopy.Welcome.chooseFolder) { chooseFolder() }
                .disabled(model.adding != nil)
                .identified("welcome-choose-folder")
            if let onAnotherRunner = setup.onAnotherRunner {
                Button(FirstRunCopy.Welcome.anotherRunner, action: onAnotherRunner)
                    .disabled(model.adding != nil)
                    .identified("welcome-another-runner")
            }
            Spacer(minLength: 0)
            if let name = model.adding.map({ ($0 as NSString).lastPathComponent }) {
                HStack(spacing: Spacing.group) {
                    ProgressView().controlSize(.small)
                    Text(FirstRunCopy.Welcome.adding(name)).foregroundStyle(.quietInk)
                        .lineLimit(1).truncationMode(.middle)
                }
                .identified("welcome-adding")
            } else if let chosen = model.chosen {
                Button {
                    Task { await model.use(chosen.path, register: setup.register) }
                } label: {
                    // A long name gives way, as the repository rows' do.
                    Text(FirstRunCopy.Welcome.add(chosen.name)).lineLimit(1).truncationMode(.middle)
                        .identified("welcome-use-label")
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.defaultAction)
                .identified("welcome-use")
            }
        }
    }

    private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        panel.message = "Choose a Git repository."
        guard panel.runModal() == .OK, let url = panel.url else { return }
        Task { await model.choose(url, register: setup.register) }
    }
}
