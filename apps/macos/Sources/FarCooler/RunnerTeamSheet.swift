import AgentKit
import SwiftUI

/// Which runner a team sheet is about, and which of its two jobs it is doing.
///
/// A type for the same reason `RunnerChoice` is one: `""` is a real target
/// (this Mac), so the sheet cannot be an optional string.
struct RunnerTeamRequest: Identifiable, Equatable {
    enum Purpose: Equatable {
        /// The runner isn't known to notify anything yet.
        case notify
        /// The runner already notifies; this puts it in a team.
        case addToTeam

        /// A runner that said it's paired is added to a team; any other, which
        /// includes one that hasn't been asked yet, is paired.
        init(paired: Bool?) { self = paired == true ? .addToTeam : .notify }

        /// The runner menu's item for this job.
        var menuTitle: String {
            switch self {
            case .notify: return "Notify Me from This Runner…"
            case .addToTeam: return "Add to Team…"
            }
        }
    }

    let target: String
    let purpose: Purpose

    /// What the runner is called on screen.
    var name: String { target.isEmpty ? "This Mac" : target }
    var id: String { "\(target)#\(purpose)" }
}

/// Choose the team a runner joins (ov-528).
///
/// Opens from a runner's menu in Settings > Runners. The personal team is
/// already chosen (ruling R-67): the common case is one click on the button.
/// The list is for the person who belongs to other teams, and it is a list of
/// rows rather than a pop-up menu so that a long team name wraps instead of
/// being cut, and so every team is visible at once.
///
/// The sequence behind the button is `RunnerRegistration`'s. This view only
/// says what is happening and, when it fails, what didn't happen and why.
struct RunnerTeamSheet: View {
    let request: RunnerTeamRequest
    /// Called with the sentence to leave under the runner's row, once it's in.
    let onDone: (String) -> Void
    /// Called when the runner's Far Cooler is too old to prove it's yours, to
    /// open the reinstall question for it. Nil for this Mac, which has none.
    let onInstallAgain: ((String) -> Void)?

    @ObservedObject private var runners: Runners
    @Environment(\.dismiss) private var dismiss

    @State private var loaded: Loaded
    @State private var selection: String?
    @State private var working = false
    @State private var failure: (message: String, detail: String?, tooOld: Bool)?
    /// The question asked before a runner of yours is moved out of another of
    /// your teams. Nothing has changed while this is showing.
    @State private var moveQuestion: String?
    /// The key the runner showed, which the move holds it to.
    @State private var moveKey: String?

    enum Loaded: Equatable {
        case loading
        case teams([DirectoryTeam])
        case failed(String)
    }

    init(
        request: RunnerTeamRequest, runners: Runners = .shared, onDone: @escaping (String) -> Void,
        onInstallAgain: ((String) -> Void)? = nil, loaded: Loaded = .loading,
        failure: (message: String, detail: String?, tooOld: Bool)? = nil, moveQuestion: String? = nil,
        moveKey: String? = nil
    ) {
        self.request = request
        _runners = ObservedObject(wrappedValue: runners)
        self.onDone = onDone
        self.onInstallAgain = onInstallAgain
        // `loaded` and `failure` are for captures and tests, which draw the
        // sheet in a state without waiting for a relay to put it there.
        _loaded = State(initialValue: loaded)
        _failure = State(initialValue: failure)
        _moveQuestion = State(initialValue: moveQuestion)
        _moveKey = State(initialValue: moveKey)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            header
            teamList
            if let moveQuestion {
                Text(moveQuestion)
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("runner-team-move-question")
            }
            if let failure {
                VStack(alignment: .leading, spacing: 6) {
                    Label(failure.message, systemImage: "exclamationmark.triangle.fill")
                        .font(.caption)
                        .foregroundStyle(.red)
                        .fixedSize(horizontal: false, vertical: true)
                    if let detail = failure.detail, !detail.isEmpty {
                        DetailBox(text: detail)
                    }
                    if failure.tooOld, let onInstallAgain, !request.target.isEmpty {
                        Button("Install Again…") {
                            onInstallAgain(request.target)
                            dismiss()
                        }
                        .accessibilityIdentifier("runner-team-install-again")
                    }
                }
            }
            buttons
        }
        .padding(20)
        .frame(minWidth: 320, idealWidth: 420, maxWidth: 520)
        .task { await load() }
    }

    // MARK: - Pieces

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(request.purpose == .notify ? "Notify Me from This Runner" : "Add to Team")
                .font(.headline)
                .accessibilityIdentifier("runner-team-title")
            Label {
                Text(request.name)
                    .lineLimit(2)
                    .truncationMode(.middle)
            } icon: {
                Image(systemName: request.target.isEmpty ? "laptopcomputer" : "server.rack")
            }
            .font(.subheadline)
            .foregroundStyle(.secondary)
            Text(
                request.purpose == .notify
                    ? "The runner joins the team you choose, then lets you know on your devices when an agent needs you."
                    : "Choose the team this runner belongs to. If it’s in another of your teams, it moves, and you stay its owner."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    @ViewBuilder
    private var teamList: some View {
        switch loaded {
        case .loading:
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                Text("Getting your teams…").foregroundStyle(.secondary)
            }
            .accessibilityIdentifier("runner-team-loading")
        case .failed(let message):
            Label(message, systemImage: "exclamationmark.triangle.fill")
                .font(.caption)
                .foregroundStyle(.red)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("runner-team-load-failed")
        case .teams(let teams):
            // Scrolls past about five rows, so a person in many teams still has
            // the buttons on screen.
            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(teams) { team in
                        row(team)
                    }
                }
            }
            .frame(maxHeight: 236)
            .fixedSize(horizontal: false, vertical: true)
            .scrollIndicatorsFlash(onAppear: true)
            .accessibilityIdentifier("runner-team-list")
            // Overlay scroll bars hide, so say it when rows are out of sight.
            if teams.count > 4 {
                Text("Scroll for more teams")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func row(_ team: DirectoryTeam) -> some View {
        let chosen = selection == team.id
        return Button {
            selection = team.id
            failure = nil
            moveQuestion = nil
            moveKey = nil
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Image(systemName: chosen ? "largecircle.fill.circle" : "circle")
                    .foregroundStyle(chosen ? AnyShapeStyle(.tint) : AnyShapeStyle(.secondary))
                VStack(alignment: .leading, spacing: 2) {
                    Text(team.name)
                        .foregroundStyle(.primary)
                        .lineLimit(3)
                        .fixedSize(horizontal: false, vertical: true)
                    Text(Self.caption(for: team))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(chosen ? Fill.inset() : Color.clear, in: RoundedRectangle(cornerRadius: Radius.small))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(working)
        .accessibilityAddTraits(chosen ? [.isSelected] : [])
        .accessibilityIdentifier("runner-team-\(team.id)")
    }

    private var buttons: some View {
        HStack {
            if working { ProgressView().controlSize(.small) }
            Spacer()
            Button("Cancel") {
                // From the move question, back to the list: nothing was moved.
                if moveQuestion != nil {
                    moveQuestion = nil
                    moveKey = nil
                } else {
                    dismiss()
                }
            }
                .keyboardShortcut(.cancelAction)
                .disabled(working)
            Button(confirmTitle) {
                Task { await submit() }
            }
            .keyboardShortcut(.defaultAction)
            .disabled(working || chosenTeam == nil)
            .accessibilityIdentifier("runner-team-confirm")
        }
    }

    // MARK: - Words

    /// The default button names what the title says it does.
    private var confirmTitle: String {
        if moveQuestion != nil { return "Move Runner" }
        return request.purpose == .notify ? "Notify Me" : "Add to Team"
    }

    /// The line under a team's name.
    static func caption(for team: DirectoryTeam) -> String {
        if team.isPersonal { return "Just you" }
        return team.role == "admin" ? "You’re an admin" : "You’re a member"
    }

    // MARK: - Doing

    private var chosenTeam: DirectoryTeam? {
        guard case .teams(let teams) = loaded else { return nil }
        return teams.first { $0.id == selection }
    }

    private func load() async {
        // Already set by a caller that has the teams (a capture, a test).
        if case .teams(let teams) = loaded {
            selection = selection ?? Self.defaultSelection(in: teams)
            return
        }
        switch await runners.teamsForJoining() {
        case .failure(let why):
            loaded = .failed(why.message)
        case .success(let teams):
            guard !teams.isEmpty else {
                loaded = .failed("You don’t belong to a team that can hold runners, so there’s nowhere to add this one.")
                return
            }
            loaded = .teams(teams)
            selection = Self.defaultSelection(in: teams)
        }
    }

    /// The personal team (ruling R-67), or the first when there isn't one.
    static func defaultSelection(in teams: [DirectoryTeam]) -> String? {
        (teams.first(where: \.isPersonal) ?? teams.first)?.id
    }

    private func submit() async {
        guard let team = chosenTeam else { return }
        working = true
        failure = nil
        defer { working = false }
        let key = moveKey
        moveQuestion = nil
        moveKey = nil
        switch await runners.join(request.target, team: team, moveKey: key) {
        case .registered(_, let message):
            onDone(message)
            dismiss()
        case .needsMove(let question, let key):
            moveQuestion = question
            moveKey = key
        case .failed(let message, let detail, let reason):
            failure = (message, detail, reason == .runnerTooOld)
        }
    }
}
