import Foundation

// The rows under "This Mac" on the Welcome page (ov-519): the background
// service, tmux and which agents are installed, each said as passing or as the
// one thing to fix. Derived from `MacStep`, so the page and its tests read the
// same rule, and the words are `FirstRunCopy.Welcome`'s.

/// One check on the Welcome page.
public struct MacCheck: Equatable, Sendable, Identifiable {
    public enum Kind: String, Sendable { case service, tmux, agents }
    public enum Result: Equatable, Sendable { case passing, failing, pending }

    public let kind: Kind
    public let result: Result
    public let text: String
    /// A quieter second line: which agents aren't installed.
    public let detail: String?

    public var id: String { kind.rawValue }
}

/// The fix a failing check offers.
public enum MacFix: Equatable, Sendable {
    /// Copy `FirstRunCopy.Welcome.tmuxCommand`, then check again.
    case installTmux
    /// Install an agent's command, then check again.
    case installAgent
    case restartService
}

public enum MacChecks {
    /// The checks for `step`. Everything before the failing one passes and
    /// everything after it waits, so a problem reads as the first thing to do.
    /// Checking shows one row, since nothing is known yet.
    public static func rows(_ step: MacStep, agents: HarnessAvailability) -> [MacCheck] {
        typealias W = FirstRunCopy.Welcome
        func row(_ kind: MacCheck.Kind, _ result: MacCheck.Result, _ text: String, _ detail: String? = nil) -> MacCheck {
            MacCheck(kind: kind, result: result, text: text, detail: detail)
        }
        switch step {
        case .checking:
            return [row(.service, .pending, W.macChecking)]
        case .serviceDown:
            return [row(.service, .failing, W.macServiceDown)]
        case .noTmux:
            return [row(.service, .passing, W.macServiceRunning), row(.tmux, .failing, W.macNoTmux)]
        case .noAgent:
            return [
                row(.service, .passing, W.macServiceRunning), row(.tmux, .passing, W.macTmuxReady),
                row(.agents, .failing, W.macNoAgent),
            ]
        case .ready(let ready):
            let missing = agents.missing
            return [
                row(.service, .passing, W.macServiceRunning), row(.tmux, .passing, W.macTmuxReady),
                row(
                    .agents, .passing, W.macReady(ready),
                    missing.isEmpty || ready.isEmpty
                        ? nil : FirstRunCopy.Conversation.someMissing(missing, on: .thisMac)),
            ]
        }
    }

    /// The fix for `step`'s failing check, if it has one.
    public static func fix(_ step: MacStep) -> MacFix? {
        switch step {
        case .noTmux: return .installTmux
        case .noAgent: return .installAgent
        case .serviceDown: return .restartService
        case .checking, .ready: return nil
        }
    }
}
