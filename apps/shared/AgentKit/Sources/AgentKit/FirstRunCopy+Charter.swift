import Foundation

extension FirstRunCopy {
    /// The Mac's Reading and Charter screens (ov-519 phase 2b), and Suggest
    /// Themes on an empty Plan.
    ///
    /// Row titles are sentence case, as Settings' are; buttons and segment
    /// labels title case. `CharterCopyTests` reads every string back.
    public enum Charter {
        // MARK: Reading

        /// "Reading harbor"
        public static func readingTitle(_ repository: String) -> String { "Reading \(repository)" }
        public static func readingBody(_ repository: String) -> String {
            "Far Cooler is looking at how \(repository) is built and how changes land, "
                + "so it can suggest how agents should work here."
        }
        /// What the survey reads, one row each, while it does.
        public static let readingRows = [
            "History and how changes land",
            "Tests and CI",
            "Pull requests and reviews on GitHub",
            "Notes for agents, such as CLAUDE.md",
            "Agents installed on this Mac",
        ]
        public static func readFailed(_ repository: String) -> String { "Far Cooler couldn’t read \(repository)." }
        /// Under the failure: the orchestrator can still start, and asks instead.
        public static let readFailedNext =
            "You can still start the orchestrator. It asks how you like work done before it begins."
        public static let tryAgain = "Try Again"
        public static let startWithoutCharter = "Start Orchestrator Anyway"

        // MARK: Charter

        public static let eyebrow = "Your Orchestrator"
        /// "How should agents work in harbor?"
        public static func title(_ repository: String) -> String { "How should agents work in \(repository)?" }
        public static func lede(_ repository: String) -> String {
            "The orchestrator is the agent you talk to. It plans the work, starts other agents on it, "
                + "and checks what they did. These are its rules, filled in from what Far Cooler found in "
                + "\(repository). Change anything now, or later with Show Charter in the orchestrator menu."
        }

        public static let from = "From"
        public static let byDefault = "Default"
        public static let changed = "Changed"
        /// After "Changed": what Far Cooler had suggested, as a phrase
        /// (`suggestedLanding`, `suggestedLanes`, `suggestedInitiative`).
        public static func changedFrom(_ suggested: String) -> String { "Far Cooler suggested \(suggested)." }
        public static func suggestedLanding(_ pullRequests: Bool) -> String {
            pullRequests ? "pull requests" : "pushing directly"
        }
        public static func suggestedLanes(_ count: Int?) -> String {
            count.map { "\($0) at once" } ?? "no limit"
        }
        public static func suggestedInitiative(_ on: Bool) -> String { on ? "having it on" : "having it off" }
        /// Under Who reviews and Agents may once Changes Land is changed: their
        /// words were the survey's for the other way.
        public static let followsLanding = "Updated to match Changes land when you start."

        public static let landingTitle = "Changes land"
        public static let landingDetail = "How finished work reaches your main branch."
        public static let landingDefault = "Nothing in the repository asks for pull requests."
        public static let pullRequests = "Pull Requests"
        public static let direct = "Push Directly"
        public static let doneTitle = "Done means"
        public static let reviewTitle = "Who reviews"
        public static let autonomyTitle = "Agents may"
        public static let lanesTitle = "Agents at once"
        public static let lanesDetail = "Each agent works in its own folder and branch."
        public static let lanesDefault = "The first run starts one at a time, until the first one’s work lands."
        public static let noLimit = "No Limit"
        public static let initiativeTitle = "Suggest next steps"
        public static let initiativeDetail =
            "At each check-in, the orchestrator adds a next step or two to each theme. "
            + "It builds the ones that are easy to undo, and suggests the rest."
        public static let alsoTitle = "Also follows"

        public static let runWith = "Run it with"
        public static let notNow = "Not Now"
        public static let talkItThrough = "Talk It Through…"
        public static let start = "Start Orchestrator"
        public static let footnote =
            "Start Orchestrator saves this as the charter and starts the orchestrator on it. "
            + "Talk It Through starts it on a draft to go through with you first."
        public static let starting = "Starting…"
        public static func writeFailed(_ repository: String) -> String {
            "Far Cooler couldn’t save \(repository)’s charter."
        }

        // MARK: Resume

        public static let resumeTitle = "You started talking this through"
        public static let resumeBody =
            "Your draft charter is saved. Pick up where you left off with the orchestrator, "
            + "or make your choices below to start over. Starting over replaces the draft."
        public static let resumeButton = "Continue Talking It Through"

        // MARK: Suggest Themes

        public static let suggestTitle = "Not sure where to start?"
        public static let suggestBody =
            "The orchestrator can read recent commits, pull requests and issues, "
            + "and suggest a few themes to work toward. You choose which to keep."
        public static let suggestButton = "Suggest Themes"
        public static let suggestSent = "Asked the orchestrator for themes. Its suggestions appear in the conversation."
        /// Suggest Themes couldn't be typed in: busy past a minute, asking
        /// something, or gone. Nothing went on the clipboard.
        public static let suggestUndelivered =
            "The orchestrator was busy, so Far Cooler didn’t ask it for themes. Try Suggest Themes again when it’s done."
        /// The runner refused for text already in the orchestrator's box (ov-557).
        public static let suggestNotEmpty =
            "The orchestrator’s box wasn’t empty, so Far Cooler didn’t ask it for themes. "
            + "Send or clear what’s there, then try Suggest Themes again."
        /// It was typed and not confirmed, or was left in the box: not tried
        /// again, so the prompt isn't sent twice.
        public static let suggestCheckPane =
            "Far Cooler typed the request for themes, but the orchestrator didn’t confirm it took it. "
            + "Check its terminal, and ask there if it isn’t on its way."
        public static let suggestAskedFirst =
            "The orchestrator is asking you something, so Far Cooler didn’t ask it for themes. "
            + "Answer it, then try Suggest Themes again."
        public static let suggestGone = "The orchestrator closed before Far Cooler could ask it for themes."

        /// Every string with no repository in it, and each that has one with
        /// a sample, for the copy tests.
        public static var all: [String] {
            let sample = "harbor"
            return readingRows + [
                readingTitle(sample), readingBody(sample), readFailed(sample), readFailedNext, tryAgain,
                startWithoutCharter, eyebrow, title(sample), lede(sample), from, byDefault, changed,
                changedFrom(suggestedLanding(true)), changedFrom(suggestedLanding(false)),
                changedFrom(suggestedLanes(2)), changedFrom(suggestedLanes(nil)), changedFrom(suggestedInitiative(true)),
                followsLanding, landingTitle, landingDetail, landingDefault, pullRequests, direct,
                doneTitle, reviewTitle, autonomyTitle, lanesTitle, lanesDetail, lanesDefault, noLimit,
                initiativeTitle, initiativeDetail, alsoTitle, runWith, notNow,
                talkItThrough, start, footnote, starting, writeFailed(sample), resumeTitle, resumeBody,
                resumeButton, suggestTitle, suggestBody, suggestButton, suggestSent, suggestUndelivered,
                suggestNotEmpty, suggestCheckPane, suggestAskedFirst, suggestGone,
            ]
        }

        /// The buttons and segment labels, which are title case.
        public static var buttons: [String] {
            [tryAgain, startWithoutCharter, pullRequests, direct, noLimit, notNow, talkItThrough, start, resumeButton,
             suggestButton]
        }
    }
}
