import AgentKit
import SwiftUI

/// The first run's Reading and Charter screens in the window (ov-519 phase
/// 2b): every Start Orchestrator for a workspace with no charter opens them
/// first, as does adding a repository from Welcome; a draft left from Talk It
/// Through brings them back on relaunch, offering to carry on with it.
extension ContentView {
    /// The key both dictionaries file a workspace's screens under.
    func charterKey(_ workspace: WorkspaceSummary, host: String) -> String { "\(host)|\(workspace.id)" }

    func charterNeed(_ workspace: WorkspaceSummary, host: String) -> CharterNeed {
        CharterNeed.of(workspace, host: host, surveys: store.clients[host]?.surveysRepositories == true)
    }

    /// Open the screens for `workspace` in place of starting its
    /// orchestrator, when it needs a charter: true when they opened (or were
    /// open), false to start as before. `preferred` is the agent somebody
    /// chose from a Start Orchestrator menu.
    @discardableResult
    func openCharterSetup(_ workspace: WorkspaceSummary, host: String, preferred: OrchestratorHarness?) -> Bool {
        let need = charterNeed(workspace, host: host)
        // No repository named, no survey to run (`--repo` takes a repository).
        guard need != .none, let client = store.clients[host], let repository = workspace.repository else {
            return false
        }
        let key = charterKey(workspace, host: host)
        charterDismissed.remove(key)
        if charterSetups[key] != nil { return true }
        let name = client.repositories.first { $0.id == repository }?.displayName ?? workspace.name
        charterSetups[key] = CharterSetup(
            host: host, workspace: workspace, repository: name,
            installed: OrchestratorChoices(client.availability).installed,
            draftWaiting: need == .draft, preferred: preferred,
            actions: CharterSetup.Actions(
                survey: { await client.surveyRepository(repository) },
                write: { await client.writeCharter($0) },
                start: { harness, how in
                    startOrchestratorNow(
                        workspace, host: host, harness: harness, replace: false, line: how.startingLine)
                    // Talk It Through happens in the conversation column, if
                    // that's still the workspace on screen: the write can take
                    // seconds, long enough to have gone elsewhere (review S3).
                    if how == .draft, case .workspace(host, workspace.id, _)? = selection {
                        selectOrchestrator(keyboard: .conversation)
                    }
                },
                close: {
                    charterSetups[key] = nil
                    charterDismissed.insert(key)
                }))
        #if DEBUG
            if let setup = charterSetups[key] { captureState(setup) }
        #endif
        return true
    }

    /// The screens to draw in place of the workspace `selection` shows, if
    /// any: none once its orchestrator is seated or starting, whichever way
    /// it was started.
    func charterSetup(for selection: Selection?) -> CharterSetup? {
        guard case .workspace(let host, let id, _)? = selection,
            let setup = charterSetups["\(host)|\(id)"]
        else { return nil }
        let seated = WorkspaceScreen.orchestrator(of: setup.workspace, host: host, in: store.fleet) != nil
            || startingOrchestrators.isStarting(setup.workspace, host: host)
        return seated ? nil : setup
    }

    /// On showing a workspace: a draft from an earlier Talk It Through,
    /// with no orchestrator to go through it, brings the screens back, once
    /// per launch unless they're opened again.
    func offerCharterResume(_ scene: WorkspaceScene) {
        guard let workspace = scene.summary, case .workspace? = selection else { return }
        let host = scene.host
        #if DEBUG
            // A capture of the screens in the real window, which otherwise
            // open only on a click (`FARCOOLER_CAPTURE_CHARTER`, debug builds).
            if ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_CHARTER"] != nil {
                openCharterSetup(workspace, host: host, preferred: nil)
                return
            }
        #endif
        guard charterNeed(workspace, host: host) == .draft,
            !charterDismissed.contains(charterKey(workspace, host: host)),
            WorkspaceScreen.orchestrator(of: workspace, host: host, in: store.fleet) == nil,
            !startingOrchestrators.isStarting(workspace, host: host)
        else { return }
        openCharterSetup(workspace, host: host, preferred: nil)
    }

    /// After Welcome added a repository and went to its Main: its screens.
    func openCharterSetupAfterRegistering(_ selection: Selection) {
        guard case .workspace(let host, let id, _) = selection,
            let workspace = store.fleet.runnerWorkspaces[host]?.first(where: { $0.id == id })
        else { return }
        openCharterSetup(workspace, host: host, preferred: nil)
    }

    #if DEBUG
        /// A state a capture of the real window needs that only a click would
        /// reach (`FARCOOLER_CAPTURE_CHARTER`, debug builds): `changed` moves
        /// two toggles off the survey's suggestion, `start` presses Start
        /// Orchestrator, which a stand-in CLI then holds or refuses.
        private func captureState(_ setup: CharterSetup) {
            let wanted = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_CHARTER"] ?? ""
            guard wanted == "changed" || wanted == "start" else { return }
            Task {
                await setup.read()
                if wanted == "changed" {
                    setup.form?.choices.landing = setup.form?.suggested.landing == .direct ? .pullRequests : .direct
                    setup.form?.choices.lanes = .noLimit
                } else {
                    await setup.start(talk: false)
                }
            }
        }
    #endif

    // MARK: - Suggest Themes

    /// Suggest Themes for `workspace`'s empty Plan: the runner's prompt,
    /// typed into its orchestrator as the owner's own message once it's
    /// between turns. Nil where it can't be: no live orchestrator, a member
    /// who talks to their rep, or a runner from before the prompt (empty).
    func suggestThemes(host: String, workspace: WorkspaceSummary, canAct: Bool) -> SuggestThemes? {
        guard canAct, let client = store.clients[host],
            let prompt = SuggestThemes.sendable(client.suggestThemesPrompt),
            surfaces(host: host, workspace: workspace).asksOrchestrator,
            let seat = WorkspaceScreen.orchestrator(of: workspace, host: host, in: store.fleet),
            StateKind.parse(seat.terminal.state) == .running
        else { return nil }
        let key = charterKey(workspace, host: host)
        return SuggestThemes(
            asked: themesAsked.contains(key),
            send: {
                themesAsked.insert(key)
                // Typed with Return once the orchestrator is between turns, and
                // only into an empty box (`onlyIfEmpty`, ov-557): a message the
                // owner left half-written there is never sent along with it.
                client.typeWhenIdle(
                    worktree: seat.worktree.id, terminal: seat.terminal.id, text: prompt,
                    name: seat.worktree.task, undelivered: nil,
                    instead: { cause in
                        // Not asked after all: offered again, and said why,
                        // with nothing on the clipboard (review S4, RN1).
                        if SuggestThemes.offersAgain(after: cause) { themesAsked.remove(key) }
                        errorBanner = SuggestThemes.undelivered(cause)
                    },
                    onlyIfEmpty: true)
            })
    }
}

/// Suggest Themes on an empty Plan (ov-519): a prompt the runner's skill
/// answers, sent to the orchestrator through its chat. There's no theme UI.
struct SuggestThemes {
    /// Pressed already in this window: it says so rather than offering it twice.
    var asked: Bool
    var send: () -> Void

    /// Why the themes weren't asked for, by what stopped the typing.
    static func undelivered(_ cause: TaskFailure.Undelivered) -> String {
        switch cause {
        case .askedFirst: return FirstRunCopy.Charter.suggestAskedFirst
        case .gone: return FirstRunCopy.Charter.suggestGone
        case .neverReady, .notTyped: return FirstRunCopy.Charter.suggestUndelivered
        case .notEmpty: return FirstRunCopy.Charter.suggestNotEmpty
        case .mayHaveTyped: return FirstRunCopy.Charter.suggestCheckPane
        }
    }

    /// Whether the button may be pressed again after `cause`. Not when the
    /// prompt may have been typed (it wasn't confirmed, or was left in the
    /// box): asking twice would send it twice.
    static func offersAgain(after cause: TaskFailure.Undelivered) -> Bool {
        cause != .mayHaveTyped
    }

    /// The prompt as it's typed: one line, no control characters, or nil when
    /// there's nothing to send (an older runner sends none). The runner wrote
    /// it for its own orchestrator, so it gains nothing it didn't have; this
    /// only keeps a stray newline from sending half of it early.
    static func sendable(_ prompt: String) -> String? {
        let line = String(prompt.unicodeScalars.map { CharacterSet.controlCharacters.contains($0) ? " " : Character($0) })
            .trimmingCharacters(in: .whitespaces)
        return line.isEmpty ? nil : line
    }
}
