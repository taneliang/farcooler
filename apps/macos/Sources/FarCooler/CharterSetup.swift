import AgentKit
import Foundation

/// Whether a workspace's orchestrator starts on the first run's Reading and
/// Charter screens (ov-519 phase 2b), from what this Mac can see.
enum CharterNeed: Equatable {
    /// It has a charter, or this Mac can't tell: Start Orchestrator starts.
    case none
    /// No charter yet: Start Orchestrator reads the repository first.
    case charter
    /// No charter, and a draft the orchestrator was going through with the
    /// owner: the screens offer to carry on with it.
    case draft

    /// The draft's name beside the charter (`charter_draft::DRAFT_FILE`).
    static let draftFile = "charter-draft.md"

    /// Only on this Mac's own runner, which says where the charter is and
    /// whose disk this app can look at (`CharterAccess`), and only one that
    /// can survey. A charter there, or the workspace a runner keeps for a
    /// layout from before workspaces, needs nothing.
    static func of(
        _ workspace: WorkspaceSummary, host: String, surveys: Bool,
        exists: (String) -> Bool = { FileManager.default.fileExists(atPath: $0) }
    ) -> CharterNeed {
        guard host.isEmpty, surveys, !workspace.isImplicit, let path = workspace.charter, !exists(path) else {
            return .none
        }
        let draft = (path as NSString).deletingLastPathComponent + "/" + draftFile
        return exists(draft) ? .draft : .charter
    }
}

/// How an orchestrator from the Charter screens starts, which says what it
/// does first (`FirstRunCopy.Conversation`'s starting lines).
enum CharterStart: Equatable {
    /// On the charter just written: it goes straight to work.
    case charter
    /// On the draft: it goes through it with the owner.
    case draft
    /// With no charter (a survey that failed): it asks.
    case interview
    /// On a charter that was already there when it was written (`wrote`
    /// false): not the one on screen.
    case existing

    var startingLine: String {
        switch self {
        case .charter: return FirstRunCopy.Conversation.startingWithCharter
        case .draft: return FirstRunCopy.Conversation.startingOnDraft
        case .interview: return FirstRunCopy.Conversation.startingBody
        case .existing: return FirstRunCopy.Conversation.startingOnExisting
        }
    }
}

/// The Reading and Charter screens for one workspace (ov-519 phase 2b).
///
/// Reading runs the runner's survey, about a second, then the Charter screen
/// shows each answer with where it came from. Start Orchestrator writes the
/// charter from it and starts the orchestrator, which goes straight to work;
/// Talk It Through writes a draft instead and starts the orchestrator on it,
/// to go through with the owner in the conversation column.
///
/// Nothing here outlives the window: the state that matters is the charter or
/// draft on the runner's disk, which `CharterNeed` reads again on relaunch.
@MainActor
final class CharterSetup: ObservableObject {
    enum Phase: Equatable {
        case reading
        case ready
        /// The survey didn't come back: why, in this app's words.
        case failed(String)
        /// Writing the charter (or the draft, `talk`) before starting.
        case starting(talk: Bool)
    }

    /// What the screens do on the runner and in the window. Closures, so a
    /// test drives them without either.
    struct Actions {
        var survey: () async -> (data: Data?, message: String?)
        var write: ([String]) async -> (data: Data?, message: String?)
        /// Start the orchestrator with this harness, as the window's own
        /// Start does, saying how: on the charter, the draft, or neither.
        var start: (OrchestratorHarness, CharterStart) -> Void
        /// Put the screens away.
        var close: () -> Void
    }

    let host: String
    let workspace: WorkspaceSummary
    /// The repository's name, as the switcher shows it.
    let repository: String
    /// The agents this runner can start, in the order offered.
    let installed: [OrchestratorHarness]
    /// A draft is waiting from an earlier Talk It Through.
    let draftWaiting: Bool

    @Published private(set) var phase: Phase = .reading
    @Published var form: CharterForm?
    @Published var harness: OrchestratorHarness?
    /// Why the charter wasn't written, on the Charter screen.
    @Published private(set) var failure: String?

    private let actions: Actions
    /// The agent somebody chose from a Start Orchestrator menu, first choice
    /// in Run It With.
    private let preferred: OrchestratorHarness?

    var id: String { "\(host)|\(workspace.id)" }

    init(
        host: String, workspace: WorkspaceSummary, repository: String, installed: [OrchestratorHarness],
        draftWaiting: Bool, preferred: OrchestratorHarness?, actions: Actions
    ) {
        self.host = host
        self.workspace = workspace
        self.repository = repository
        self.installed = installed
        self.draftWaiting = draftWaiting
        self.preferred = preferred
        self.actions = actions
        harness = preferred.flatMap { installed.contains($0) ? $0 : nil }
    }

    /// Run the survey, and show the Charter screen when it's back.
    func read() async {
        phase = .reading
        failure = nil
        let (data, message) = await actions.survey()
        // Left while reading: the view's task was cancelled, and the runner
        // was never asked to finish. Still reading, so coming back reads
        // again rather than showing a failure that didn't happen (review S2).
        if Task.isCancelled || message == DaemonClient.cancelledMessage { return }
        guard let data, let survey = CharterSurvey(json: data) else {
            phase = .failed(DaemonClient.surveyRefusal(message, repository: repository))
            return
        }
        form = CharterForm(survey: survey)
        if harness == nil { harness = Self.harness(for: survey, installed: installed) }
        phase = .ready
    }

    /// The agent Run It With starts on: the survey's first installed one,
    /// Claude Code then Codex, else whichever this runner has.
    static func harness(for survey: CharterSurvey, installed: [OrchestratorHarness]) -> OrchestratorHarness? {
        let program = survey.answer("harness")?.value ?? ""
        let named = AgentHarness.allCases.first { $0.program == program }
            .flatMap { OrchestratorHarness(rawValue: $0.rawValue) }
        if let named, installed.contains(named) { return named }
        return installed.first
    }

    /// Whether Start Orchestrator and Talk It Through can be pressed.
    var canStart: Bool { phase == .ready && form != nil && harness != nil }

    /// Start Orchestrator (`talk` false) or Talk It Through (`talk` true):
    /// write the charter or its draft, then start. A charter that already
    /// exists is never overwritten, and the orchestrator starts on it.
    func start(talk: Bool) async {
        guard canStart, let form, let harness else { return }
        phase = .starting(talk: talk)
        failure = nil
        let (data, message) = await actions.write(form.writeArguments(workspace: workspace.id, draft: talk))
        guard let written = data.flatMap(CharterWritten.init(json:)) else {
            failure = DaemonClient.charterRefusal(message, repository: repository)
            phase = .ready
            return
        }
        // A charter that appeared meanwhile is never overwritten: the
        // orchestrator starts on that one, not on what was on screen (RN3).
        actions.start(harness, !written.wrote ? .existing : talk ? .draft : .charter)
        actions.close()
    }

    /// Continue Talking It Through: start the orchestrator on the draft that
    /// is there, untouched. Writing another would replace what was agreed.
    func continueTalking() {
        guard let harness = harness ?? installed.first else { return }
        actions.start(harness, .draft)
        actions.close()
    }

    /// After a survey that failed: start as before the survey existed, and
    /// the orchestrator asks.
    func startWithoutCharter() {
        guard let harness = harness ?? installed.first else { return }
        actions.start(harness, .interview)
        actions.close()
    }

    func notNow() { actions.close() }
}
