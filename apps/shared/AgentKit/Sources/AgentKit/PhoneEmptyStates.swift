import Foundation

/// What a phone's empty state says under its title, as a short lede and a few
/// icon rows rather than a paragraph (ov-245).
///
/// The Mac's `EmptyStateCopy` (ov-205), for the same reason: the owner read a
/// five-line paragraph under "No Workspace Selected" as "too many words", and
/// nobody scans a paragraph. So an empty state says what the thing is for in one
/// short line, then two or three rows, each an SF Symbol and a few words, then
/// its button. Android says the same rows in sentence case with Material icons
/// (`model/PhoneEmptyStates.kt`).
public struct PhoneEmptyCopy: Equatable, Sendable {
    /// One row: a symbol and a few words, sentence case with no period, since a
    /// row reads as an item in a list.
    public struct Row: Equatable, Hashable, Sendable {
        public let symbol: String
        public let text: String

        public init(symbol: String, text: String) {
            self.symbol = symbol
            self.text = text
        }
    }

    /// The thing's purpose, in one short sentence.
    public let lede: String
    public let rows: [Row]

    public init(lede: String, rows: [Row]) {
        self.lede = lede
        self.rows = rows
    }
}

/// Every phone empty state that explains something, by name. Where each is
/// drawn is in its screen's file; the words and their shape are here, where
/// `PhoneEmptyStatesTests` reads them, because `apps/ios` has no unit tests CI
/// runs and a paragraph is exactly what slips back in unnoticed.
public enum PhoneEmptyStates {
    /// No Orchestrator, in a workspace. The Mac's words.
    public static let noOrchestrator = PhoneEmptyCopy(
        lede: "An orchestrator turns your requests into tasks and starts agents on them.",
        rows: [
            .init(symbol: "bubble.left", text: "Tell it what you want built"),
            .init(symbol: "hand.raised", text: "Anything it can’t decide comes to you"),
        ])

    /// Under an empty Needs You, while no orchestrator runs anywhere.
    public static let noAgentsWorking = PhoneEmptyCopy(
        lede: "No agents are working yet.",
        rows: [
            .init(symbol: "person.crop.circle.badge.plus", text: "Start an orchestrator and give it work"),
        ])

    /// A runner that lists no repository. The Mac's words.
    public static let noRepositories = PhoneEmptyCopy(
        lede: "Add the repository you want agents to work in.",
        rows: [
            .init(symbol: "arrow.triangle.branch", text: "Each agent gets its own folder and branch"),
            .init(symbol: "arrow.triangle.merge", text: "Your files don’t change until you merge"),
        ])

    /// A workspace's Worktrees with none. The Mac's words.
    public static let noWorktrees = PhoneEmptyCopy(
        lede: "A worktree gives an agent its own folder and branch.",
        rows: [
            .init(symbol: "arrow.triangle.merge", text: "Your files don’t change until you merge"),
        ])

    /// An empty board its orchestrator leads, running. The Mac's words.
    public static let boardWithOrchestrator = PhoneEmptyCopy(
        lede: "Tasks show up here as the orchestrator makes them.",
        rows: [
            .init(symbol: "bubble.left", text: "Tell it what you want built"),
            .init(symbol: "eye", text: "Finished work waits here for your review"),
        ])

    /// The same with none running: start it first.
    public static let boardNoOrchestrator = PhoneEmptyCopy(
        lede: "Tasks show up here once an orchestrator is running.",
        rows: [
            .init(symbol: "person.crop.circle.badge.plus", text: "Start the orchestrator, then give it work"),
        ])

    /// A board on a runner too old for workspaces: no orchestrator leads it, and
    /// the one line is short enough to need no rows.
    public static let boardImplicit = PhoneEmptyCopy(
        lede: "Tasks show up here as they’re added.", rows: [])

    static var all: [PhoneEmptyCopy] {
        [
            noOrchestrator, noAgentsWorking, noRepositories, noWorktrees, boardWithOrchestrator,
            boardNoOrchestrator, boardImplicit,
        ]
    }

    /// Every word above, for the voice check beside `FirstRunCopy.all`.
    static var allStrings: [String] {
        all.flatMap { [$0.lede] + $0.rows.map(\.text) }
    }
}
