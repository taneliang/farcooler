import Foundation

/// Whom a person talks to in a workspace on a shared runner (ov-526).
///
/// Only a workspace's lead, and the runner's owner, type into its
/// orchestrator; the runner refuses anyone else `not_lead`. A member who
/// isn't the lead talks to their rep instead, which relays to the
/// orchestrator. This is where the apps ask which it is, so the Mac and the
/// iPhone never offer a composer the runner would refuse, and so the words
/// for the rep are said once.
extension RunnerPeople {
    /// Who a device talks to in one workspace.
    public enum Talk: Equatable, Sendable {
        /// The orchestrator: the owner's device, or the lead's.
        case orchestrator
        /// A member's own rep. `existing` is the rep they already have in
        /// this workspace, running or stopped, and nil before the first.
        case rep(handle: String, name: String, existing: Rep?)
        /// Nobody: a device that was a member's and is nobody's now, which
        /// the runner lets into neither.
        case neither
    }

    /// Whom this device talks to in `workspace`.
    ///
    /// A runner that doesn't name the lead (`lead` nil) is read the safe way
    /// round for a member: they get their rep, never an orchestrator composer
    /// that may be refused. The owner's device always gets the orchestrator.
    public func talk(in workspace: WorkspaceSummary) -> Talk {
        if you == Self.released { return .neither }
        guard isMember else { return .orchestrator }
        if let lead = workspace.lead, lead == you { return .orchestrator }
        let name = members.first { $0.handle == you }?.name ?? you
        let existing = members.first { $0.handle == you }?.reps.first { $0.workspace == workspace.id }
        return .rep(handle: you, name: name, existing: existing)
    }

    /// What the rep's place is called where the orchestrator's would be:
    /// the segment, the navigator's row and the title bar's mark.
    public static let repTitle = "Your Rep"

    /// While the runner opens the rep.
    public static let repStarting = "Starting Your Rep…"

    /// The first screen of a rep, which is also what shows while it starts:
    /// what a rep is for, then whose Claude account its work runs on. Two
    /// sentences, from the same words Settings uses (`repPurpose`,
    /// `accountLine`).
    public var repIntro: [String] { [Self.repPurpose, accountLine] }

    /// A stopped rep: the title and what Resume does.
    public static let repStoppedTitle = "Your Rep Is Stopped"
    public static let repStoppedBody = "Your rep isn’t running. Resume picks up where it left off."

    /// The buttons.
    public static let startRep = "Start Your Rep"
    public static let resumeRep = "Resume Your Rep"
    public static let tryAgain = "Try Again"

    /// A device the runner has removed from the team.
    public static let neitherTitle = "No One to Talk To"
    public static let neitherBody =
        "This device was removed from the team, so it can’t talk to the orchestrator or a rep. Ask the runner’s owner to add it again."

    /// What the screen says when the runner won't start the rep, in place of
    /// the runner's own words.
    public static let repNotStarted =
        "The runner didn’t start your rep. Check that this device is still on the team, then try again."

    /// When who's on the team can't be read.
    public static let unknownTitle = "Can’t Tell Who’s on the Team"
    public static let unknownBody = "The runner didn’t say who’s on the team."
}

/// Whom a device talks to in a workspace, counting what the app hasn't read
/// yet: `RunnerPeople.Talk` plus the two states before and beside the answer.
public enum TalkSeat: Equatable, Sendable {
    /// The orchestrator's composer, which the runner will take.
    case orchestrator
    /// Not known yet: `member.list` hasn't answered. Nothing to type into.
    case reading
    /// `member.list` didn't answer, so there's no telling.
    case unknown
    /// A member's own rep, and the one they have here, if any.
    case rep(handle: String, name: String, existing: RunnerPeople.Rep?)
    /// Nobody: a device the runner has removed from the team.
    case neither

    /// The seat in `workspace`.
    ///
    /// A runner without people (`hasPeople` false), and a runner's implicit
    /// workspace, have only the orchestrator, and nobody there is refused it.
    /// The owner never waits on the people: `isLocalOwner` (a Mac that runs
    /// the runner) is the orchestrator's at once, and `knownYou`, who this
    /// device was the last time the runner said ("" for the owner), keeps an
    /// owner's device on it while the list is read again or can't be. A
    /// device that has never been told stays shut out until it is: `reading`
    /// until the list arrives, `unknown` if it failed.
    public static func resolve(
        workspace: WorkspaceSummary, people: RunnerPeople?, readFailed: Bool, hasPeople: Bool,
        knownYou: String? = nil, isLocalOwner: Bool = false
    ) -> TalkSeat {
        guard hasPeople, !workspace.isImplicit, !isLocalOwner else { return .orchestrator }
        guard let people else {
            if knownYou == "" { return .orchestrator }
            return readFailed ? .unknown : .reading
        }
        switch people.talk(in: workspace) {
        case .orchestrator: return .orchestrator
        case .neither: return .neither
        case let .rep(handle, name, existing): return .rep(handle: handle, name: name, existing: existing)
        }
    }

    /// What each orchestrator surface does for this seat: the one place the
    /// windows ask, so a surface can't be drawn for a device the runner
    /// would refuse it (`not_lead`).
    public struct Surfaces: Equatable, Sendable {
        /// The conversation column draws the orchestrator, or the rep's
        /// column (its pane, or its first screen, or a notice).
        public var columnIsOrchestrator: Bool
        /// The title bar's orchestrator menu: restart, replace, step down.
        public var orchestratorMenu: Bool
        /// Ask the Orchestrator, on a task.
        public var asksOrchestrator: Bool
        /// Reverse and Discuss on a ruling.
        public var discussesRulings: Bool
        /// Use as Orchestrator, on a pane.
        public var adoptsOrchestrator: Bool
        /// The word for the place in the navigator, crumb and palette.
        public var title: String
    }

    /// The surfaces for this seat. Only the orchestrator's seat has any.
    public var surfaces: Surfaces {
        let on = self == .orchestrator
        return Surfaces(
            columnIsOrchestrator: on, orchestratorMenu: on, asksOrchestrator: on, discussesRulings: on,
            adoptsOrchestrator: on, title: title)
    }

    /// What the place the orchestrator takes is called: "Your Rep" for a
    /// member's rep, else "Orchestrator".
    public var title: String {
        if case .rep = self { return RunnerPeople.repTitle }
        return "Orchestrator"
    }
}
