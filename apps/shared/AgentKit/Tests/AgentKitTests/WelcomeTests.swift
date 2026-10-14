import Foundation
import Testing

@testable import AgentKit

// The Mac's Welcome page (ov-519): its checks, its words, and the repositories
// it finds on this Mac.

// MARK: - The checks

@Test func aProblemIsTheFirstThingToDo() {
    let both = HarnessAvailability(agentsFound: ["claude", "codex"])
    let rows = MacChecks.rows(.ready([.claude, .codex]), agents: both)
    #expect(rows.map(\.kind) == [.service, .tmux, .agents])
    #expect(rows.allSatisfy { $0.result == .passing })
    #expect(rows[2].text == "Ready to run Claude Code and Codex.")
    // The agent that isn't there is said once, quietly, under the ones that are.
    #expect(rows[2].detail == "Cursor isn’t installed on this Mac.")

    let noTmux = MacChecks.rows(.noTmux, agents: both)
    #expect(noTmux.map(\.result) == [.passing, .failing])
    #expect(noTmux[1].text == FirstRunCopy.Welcome.macNoTmux)
    #expect(MacChecks.fix(.noTmux) == .installTmux)

    let noAgent = MacChecks.rows(.noAgent, agents: HarnessAvailability(agentsFound: []))
    #expect(noAgent.map(\.result) == [.passing, .passing, .failing])
    #expect(MacChecks.fix(.noAgent) == .installAgent)

    #expect(MacChecks.rows(.serviceDown, agents: both).map(\.result) == [.failing])
    #expect(MacChecks.fix(.serviceDown) == .restartService)
    #expect(MacChecks.rows(.checking, agents: both).map(\.result) == [.pending])
    #expect(MacChecks.fix(.ready([.claude])) == nil && MacChecks.fix(.checking) == nil)
}

@Test func aRunnerThatDidntSayWhichAgentsItHasNamesNoMissingOne() {
    let unknown = HarnessAvailability(agentsFound: nil)
    let rows = MacChecks.rows(.ready([]), agents: unknown)
    #expect(rows[2].text == "Ready to run agents.")
    #expect(rows[2].detail == nil)
}

@Test func theWelcomeCallsThemAgents() {
    let welcome = FirstRunCopy.Welcome.body
    #expect(welcome.hasPrefix("Agents work on your projects on this Mac"))
    #expect(!welcome.contains("Coding agents"))
    #expect(FirstRunCopy.Welcome.macReady([]) == "Ready to run agents.")
}

// MARK: - Found on this Mac

/// A disk as a dictionary: a folder's subfolders, which are repositories, the
/// date of each one's newest commit and when it was last touched. It counts
/// what the walk asks of it, so the bounds are measured, not assumed.
private final class FakeDisk: SuggestionFileSystem, @unchecked Sendable {
    var folders: [String: [String]] = [:]
    var repositories: Set<String> = []
    var commits: [String: Date] = [:]
    var touches: [String: Date] = [:]
    var links: [String: String] = [:]
    var symlinks: Set<String> = []
    private let lock = NSLock()
    private(set) var folderReads = 0
    private(set) var repositoryChecks = 0
    private(set) var dated: [String] = []

    func subfolders(of path: String) -> [String] { lock.withLock { folderReads += 1 }; return folders[path] ?? [] }
    func isRepository(_ path: String) -> Bool { lock.withLock { repositoryChecks += 1 }; return repositories.contains(path) }
    func isLink(_ path: String) -> Bool { symlinks.contains(path) }
    func touched(_ path: String) -> Date? { touches[path] ?? commits[path] }
    func resolved(_ path: String) -> String { links[path] ?? path }
    func lastCommit(of path: String) async -> Date? { lock.withLock { dated.append(path) }; return commits[path] }
}

private let home = "/Users/ada"
private func day(_ n: Int) -> Date { Date(timeIntervalSince1970: 1_700_000_000 + Double(n) * 86_400) }

@Test func repositoriesComeNewestCommitFirstAndStopAtFive() async {
    let disk = FakeDisk()
    let names = ["a", "b", "c", "d", "e", "f", "g"]
    disk.folders["\(home)/Dev"] = names
    for (i, name) in names.enumerated() {
        disk.repositories.insert("\(home)/Dev/\(name)")
        disk.commits["\(home)/Dev/\(name)"] = day(i)
    }
    let found = await RepositorySuggestions.scan(home: home, fileSystem: disk)
    #expect(found.map(\.name) == ["g", "f", "e", "d", "c"])
}

@Test func theWalkReadsTwoFoldersDeepAndNoDeeper() async {
    let disk = FakeDisk()
    disk.folders["\(home)/Code"] = ["work", "one"]
    disk.folders["\(home)/Code/work"] = ["two", "deep"]
    disk.folders["\(home)/Code/work/deep"] = ["three"]
    disk.repositories = ["\(home)/Code/one", "\(home)/Code/work/two", "\(home)/Code/work/deep/three"]
    let found = await RepositorySuggestions.scan(home: home, fileSystem: disk)
    #expect(Set(found.map(\.name)) == ["one", "two"])
}

@Test func onlyTheUsualFoldersAreRead() async {
    let disk = FakeDisk()
    for root in ["Dev", "Code", "Projects", "src", "Developer"] {
        disk.folders["\(home)/\(root)"] = ["p-\(root)"]
        disk.repositories.insert("\(home)/\(root)/p-\(root)")
    }
    // Documents would ask for permission, and Downloads isn't where projects live.
    disk.folders["\(home)/Documents"] = ["notes"]
    disk.folders["\(home)/Downloads"] = ["clone"]
    disk.repositories.formUnion(["\(home)/Documents/notes", "\(home)/Downloads/clone"])
    let found = await RepositorySuggestions.scan(home: home, fileSystem: disk)
    #expect(Set(found.map(\.name)) == ["p-Dev", "p-Code", "p-Projects", "p-src", "p-Developer"])
}

@Test func aFolderThatIsNotARepositoryAndTheNoiseAroundOneAreSkipped() async {
    let disk = FakeDisk()
    disk.folders["\(home)/Dev"] = ["plain", "repo", ".hidden", "node_modules"]
    disk.repositories = ["\(home)/Dev/repo", "\(home)/Dev/.hidden", "\(home)/Dev/node_modules"]
    // Inside a repository is its own: a nested one isn't a second project.
    disk.folders["\(home)/Dev/repo"] = ["nested"]
    disk.repositories.insert("\(home)/Dev/repo/nested")
    let found = await RepositorySuggestions.scan(home: home, fileSystem: disk)
    #expect(found.map(\.name) == ["repo"])
}

@Test func oneRepositoryReachedByTwoLinksIsListedOnce() async {
    let disk = FakeDisk()
    disk.folders["\(home)/Dev"] = ["harbor"]
    disk.folders["\(home)/Code"] = ["harbor"]
    disk.repositories = ["\(home)/Dev/harbor", "\(home)/Code/harbor"]
    disk.links["\(home)/Code/harbor"] = "\(home)/Dev/harbor"
    #expect(await RepositorySuggestions.scan(home: home, fileSystem: disk).count == 1)
}

@Test func aRepositoryWithNoCommitComesLastAndEqualDatesKeepAStableOrder() {
    let list = [
        RepositorySuggestion(path: "/x/new", lastCommit: nil),
        RepositorySuggestion(path: "/x/b", lastCommit: day(3)),
        RepositorySuggestion(path: "/x/a", lastCommit: day(3)),
        RepositorySuggestion(path: "/x/old", lastCommit: day(1)),
    ]
    #expect(RepositorySuggestions.ranked(list).map(\.name) == ["a", "b", "old", "new"])
}

@Test func theWalkStopsAtItsCap() async {
    let disk = FakeDisk()
    let many = (0..<(RepositorySuggestions.visitCap + 500)).map { "p\($0)" }
    disk.folders["\(home)/src"] = many
    disk.repositories = Set(many.map { "\(home)/src/\($0)" })
    // The newest repository is the last the walk would read, in the order it
    // reads them (by name): past the cap it is never reached, and with the cap
    // gone it would be found and win.
    let late = many.sorted().last!
    for name in many { disk.commits["\(home)/src/\(name)"] = day(0) }
    disk.commits["\(home)/src/\(late)"] = day(900)
    let found = await RepositorySuggestions.scan(home: home, fileSystem: disk)
    #expect(!found.contains { $0.name == late }, "read past the cap: \(found.map(\.name))")
    // The walk's own work is what is bounded: a folder read per folder, one
    // check each, and nothing beyond the cap.
    #expect(disk.repositoryChecks <= RepositorySuggestions.visitCap, "\(disk.repositoryChecks) checks")
}

@Test func onlyTheMostRecentlyTouchedAreDated() async {
    let disk = FakeDisk()
    let names = (0..<300).map { "r\($0)" }
    disk.folders["\(home)/Dev"] = names
    for (i, name) in names.enumerated() {
        let path = "\(home)/Dev/\(name)"
        disk.repositories.insert(path)
        disk.touches[path] = day(i)
        // The commit dates run the other way: undated repositories can't
        // outrank the dated finalists, so only the finalists' answers count.
        disk.commits[path] = day(1000 - i)
    }
    let found = await RepositorySuggestions.scan(home: home, fileSystem: disk)
    // However many are on disk, a bounded number of `git log`s run, and they
    // are the ones touched most recently.
    #expect(disk.dated.count == RepositorySuggestions.dated, "\(disk.dated.count) dated")
    let newest = Set((290..<300).map { "\(home)/Dev/r\($0)" })
    #expect(Set(disk.dated) == newest)
    #expect(found.count == RepositorySuggestions.limit && found.allSatisfy { newest.contains($0.path) })
    // Among the finalists the commit date decides: r290 has the newest commit.
    #expect(found.first?.name == "r290")
}

@Test func aLinkBelowARootIsNeverFollowed() async {
    let disk = FakeDisk()
    // `~/Dev/notes` is a link to a folder holding a repository, and `~/Dev/shortcut`
    // is a link to a repository: neither is read, so no privacy prompt and no stall.
    disk.folders["\(home)/Dev"] = ["notes", "shortcut", "real"]
    disk.folders["\(home)/Dev/notes"] = ["inside"]
    disk.repositories = ["\(home)/Dev/notes/inside", "\(home)/Dev/shortcut", "\(home)/Dev/real"]
    disk.symlinks = ["\(home)/Dev/notes", "\(home)/Dev/shortcut"]
    let found = await RepositorySuggestions.scan(home: home, fileSystem: disk)
    #expect(found.map(\.name) == ["real"])
    #expect(disk.folderReads == RepositorySuggestions.roots.count, "read only the roots: \(disk.folderReads)")
}

@Test func aSuggestionIsNamedByItsFolderAndDatedInEnglish() {
    let suggestion = RepositorySuggestion(path: "\(home)/Dev/harbor", lastCommit: day(0))
    #expect(suggestion.name == "harbor")
    #expect(suggestion.displayPath(home: home) == "~/Dev/harbor")
    #expect(suggestion.displayPath(home: "/Users/bob") == "\(home)/Dev/harbor")
    #expect(suggestion.when(now: day(2)) == "2 days ago")
    #expect(suggestion.when(now: day(0).addingTimeInterval(20)) == "Just now")
    #expect(RepositorySuggestion(path: "/x/y", lastCommit: nil).when(now: day(0)) == "No commits yet")
}
