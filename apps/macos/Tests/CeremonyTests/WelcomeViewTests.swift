import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The Mac's Welcome page (ov-519): what a Mac with no repository shows in
/// place of a spinner and "No Repositories".
@MainActor
@Suite(.serialized)
struct WelcomeViewTests {
    private static func suggestion(_ name: String, daysAgo: Int) -> RepositorySuggestion {
        RepositorySuggestion(path: "/Users/ada/Dev/\(name)", lastCommit: Date().addingTimeInterval(-Double(daysAgo) * 86_400))
    }

    private static func setup(
        _ step: MacStep, agents: [String]? = ["claude", "codex"], found: [RepositorySuggestion] = [],
        register: @escaping (String) async -> String? = { _ in nil }, runners: Bool = false,
        onCheckAgain: @escaping () -> Void = {}, recheckEvery: TimeInterval = 5
    ) -> WelcomeSetup {
        WelcomeSetup(
            model: WelcomeModel(home: "/Users/ada", scan: { found }), step: step,
            agents: HarnessAvailability(agentsFound: agents), register: register, onCheckAgain: onCheckAgain,
            onAnotherRunner: runners ? {} : nil, recheckEvery: recheckEvery)
    }

    /// The page, with its model's look finished.
    private static func drawn(
        _ setup: WelcomeSetup, width: CGFloat = 900, appearance: NSAppearance.Name = .aqua
    ) async -> EmptyStateProbe.Drawn {
        await setup.model.look()
        return await EmptyStateProbe.draw(WelcomeView(setup: setup), width: width, appearance: appearance)
    }

    @Test("A ready Mac shows its three checks and the repositories found, with the newest chosen")
    func ready() async {
        let found = [Self.suggestion("harbor", daysAgo: 1), Self.suggestion("billing", daysAgo: 9)]
        let drawn = await Self.drawn(Self.setup(.ready([.claude, .codex]), found: found))
        #expect(drawn.contains("welcome"))
        #expect(
            ["welcome-check-service", "welcome-check-tmux", "welcome-check-agents"].allSatisfy(drawn.contains),
            "\(drawn.ids)")
        #expect(drawn.count("welcome-suggestion") == 2, "\(drawn.ids)")
        #expect(drawn.contains("welcome-choose-folder") && drawn.contains("welcome-use"), "\(drawn.ids)")
        // Nothing is wrong, so no fix is offered.
        #expect(!drawn.contains("welcome-fix-tmux") && !drawn.contains("welcome-fix-agent"))
    }

    @Test("A missing tmux offers its command and stops at that check")
    func noTmux() async {
        let drawn = await Self.drawn(Self.setup(.noTmux))
        #expect(drawn.contains("welcome-fix-tmux"), "\(drawn.ids)")
        #expect(drawn.contains("welcome-check-tmux") && !drawn.contains("welcome-check-agents"), "\(drawn.ids)")
    }

    @Test("No installed agent offers Check Again at the agents check")
    func noAgent() async {
        let drawn = await Self.drawn(Self.setup(.noAgent, agents: []))
        #expect(drawn.contains("welcome-fix-agent") && drawn.contains("welcome-check-agents"), "\(drawn.ids)")
    }

    @Test("With nothing found it says so and keeps Choose Folder")
    func nothingFound() async {
        let drawn = await Self.drawn(Self.setup(.ready([.claude])))
        #expect(drawn.contains("welcome-found-none") && !drawn.contains("welcome-found"), "\(drawn.ids)")
        #expect(drawn.contains("welcome-choose-folder") && !drawn.contains("welcome-use"), "\(drawn.ids)")
    }

    @Test("Add to Another Runner appears only with another runner to add to")
    func anotherRunner() async {
        let none = await Self.drawn(Self.setup(.ready([.claude])))
        let some = await Self.drawn(Self.setup(.ready([.claude]), runners: true))
        #expect(some.contains("welcome-another-runner") && !none.contains("welcome-another-runner"), "\(some.ids)")
    }

    @Test("The window's placeholder draws Welcome for a Mac with no repository")
    func placeholderDrawsWelcome() async {
        let setup = Self.setup(.ready([.claude]), found: [Self.suggestion("harbor", daysAgo: 2)])
        await setup.model.look()
        let placeholder = FleetPlaceholder(phase: .noRepositories, welcome: setup, onNewWorktree: {}, onTryAgain: {})
        let drawn = await EmptyStateProbe.draw(placeholder)
        #expect(drawn.contains("welcome") && drawn.contains("welcome-suggestion"), "\(drawn.ids)")
    }

    @Test("The page fits and centers at 900 and 1400 points, in light and dark")
    func fitsAtEveryWidth() async {
        let found = (1...5).map { Self.suggestion("a-long-repository-name-\($0)", daysAgo: $0) }
        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            for width in [900.0, 1400.0] {
                let drawn = await Self.drawn(Self.setup(.ready([.claude, .codex]), found: found), width: width, appearance: appearance)
                let page = drawn.frame("welcome-column")
                #expect(page != nil)
                guard let page else { continue }
                // The column is a reading width, centered: it never stretches with the window.
                #expect(page.width <= WelcomeView.width + 2, "\(page) at \(width)")
                #expect(abs(page.midX - width / 2) < 2, "\(page) at \(width)")
                // The primary button sits at the column's trailing edge, as a
                // default button does, not floating inside a wider frame.
                // Measured on the label, whose own padding inside the button is
                // under 24 pt, so a button floating in a wider frame is far off.
                if let label = drawn.frame("welcome-use-label") {
                    #expect(page.maxX - label.maxX < 24, "\(label) in \(page)")
                }
                for id in ["welcome-checks", "welcome-found", "welcome-use"] {
                    #expect(drawn.frame(id).map { $0.minX >= page.minX - 1 && $0.maxX <= page.maxX + 1 } == true, "\(id)")
                }
            }
        }
    }

    @Test("A long repository name keeps the button on one line, inside the column")
    func longName() async {
        let long = String(repeating: "a-very-long-repository-name-", count: 8)
        let found = [Self.suggestion(long, daysAgo: 1)]
        for width in [900.0, 1400.0] {
            let drawn = await Self.drawn(Self.setup(.ready([.claude]), found: found), width: width)
            let page = drawn.frame("welcome-column")
            let button = drawn.frame("welcome-use")
            #expect(page != nil && button != nil, "\(drawn.ids)")
            guard let page, let button else { continue }
            #expect(button.height < 40, "wrapped: \(button)")
            #expect(button.maxX <= page.maxX + 1 && button.minX >= page.minX - 1, "\(button) in \(page)")
        }
    }

    @Test("A failing check looks again by itself, and a passing page doesn't")
    func looksAgainWhileFailing() async throws {
        let failing = LookCounter()
        let setup = Self.setup(.noTmux, onCheckAgain: { failing.count += 1 }, recheckEvery: 0.05)
        _ = await EmptyStateProbe.draw(WelcomeView(setup: setup))
        #expect(failing.count >= 2, "looked \(failing.count) times")
        let passing = LookCounter()
        let ready = Self.setup(.ready([.claude]), onCheckAgain: { passing.count += 1 }, recheckEvery: 0.05)
        _ = await EmptyStateProbe.draw(WelcomeView(setup: ready))
        #expect(passing.count == 0)
    }

    final class LookCounter { var count = 0 }

    // MARK: - The model

    @Test("Looking chooses the newest repository, once")
    func looksOnce() async {
        var scans = 0
        let found = [Self.suggestion("harbor", daysAgo: 1), Self.suggestion("billing", daysAgo: 9)]
        let model = WelcomeModel(home: "/Users/ada", scan: { scans += 1; return found })
        await model.look()
        await model.look()
        #expect(scans == 1)
        #expect(model.selected == "/Users/ada/Dev/harbor" && model.chosen?.name == "harbor")
    }

    @Test("A repository that wouldn't register says so, with the runner's words under it")
    func failedAdd() async {
        let model = WelcomeModel(home: "/Users/ada", scan: { [] })
        await model.use("/Users/ada/Dev/harbor") { _ in "not allowed" }
        #expect(model.failure?.sentence == FirstRunCopy.Welcome.couldNotAdd)
        #expect(model.failure?.transcript == "not allowed")
        #expect(model.adding == nil)
        await model.use("/Users/ada/Dev/harbor") { _ in nil }
        #expect(model.failure == nil)
    }

    @Test("A folder chosen by hand registers only if it's a repository")
    func chosenFolder() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ov519-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: root) }
        let plain = root.appendingPathComponent("plain"), repo = root.appendingPathComponent("repo")
        try FileManager.default.createDirectory(at: plain, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: repo.appendingPathComponent(".git"), withIntermediateDirectories: true)
        var registered: [String] = []
        let model = WelcomeModel(home: root.path, scan: { [] })
        await model.choose(plain) { registered.append($0); return nil }
        #expect(registered.isEmpty)
        #expect(model.failure?.sentence == FirstRunCopy.Welcome.notARepository)
        await model.choose(repo) { registered.append($0); return nil }
        #expect(registered == [repo.path] && model.failure == nil)
    }

    // MARK: - The real disk

    @Test("The real scan finds repositories on disk, newest commit first, and skips a linked worktree")
    func realDisk() async throws {
        let git = LocalSuggestionFileSystem.git ?? "/usr/bin/git"
        try #require(FileManager.default.isExecutableFile(atPath: git), "no git to make a repository with")
        let home = FileManager.default.temporaryDirectory.appendingPathComponent("ov519-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: home) }
        func make(_ path: String, at date: String) throws {
            let dir = home.appendingPathComponent(path)
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            for args in [["init", "-q"], ["commit", "-q", "--allow-empty", "-m", "first"]] {
                let process = Process()
                process.executableURL = URL(fileURLWithPath: git)
                process.arguments = ["-C", dir.path, "-c", "user.name=T", "-c", "user.email=t@example.com"] + args
                process.environment = ["GIT_COMMITTER_DATE": date, "GIT_AUTHOR_DATE": date, "PATH": "/usr/bin:/bin"]
                try process.run()
                process.waitUntilExit()
            }
        }
        try make("Dev/old", at: "2024-01-01T12:00:00Z")
        try make("Dev/work/new", at: "2024-06-01T12:00:00Z")
        // A linked worktree's `.git` is a file: it is somebody's checkout, not a project.
        let linked = home.appendingPathComponent("Dev/linked")
        try FileManager.default.createDirectory(at: linked, withIntermediateDirectories: true)
        try "gitdir: /elsewhere".write(to: linked.appendingPathComponent(".git"), atomically: true, encoding: .utf8)
        try FileManager.default.createDirectory(at: home.appendingPathComponent("Dev/plain"), withIntermediateDirectories: true)

        let found = await RepositorySuggestions.scan(home: home.path, fileSystem: LocalSuggestionFileSystem())
        #expect(found.map(\.name) == ["new", "old"], "\(found.map(\.path))")
        #expect(found.first?.lastCommit == ISO8601DateFormatter().date(from: "2024-06-01T12:00:00Z"))
    }

    @Test("Without git, the repository's own log of HEAD dates it")
    func reflog() throws {
        let repo = FileManager.default.temporaryDirectory.appendingPathComponent("ov519-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: repo) }
        let logs = repo.appendingPathComponent(".git/logs")
        try FileManager.default.createDirectory(at: logs, withIntermediateDirectories: true)
        let line = "\(String(repeating: "0", count: 40)) \(String(repeating: "a", count: 40)) Ada <a@x.org> 1700000000 -0700\tcommit (initial): first\n"
        try (line + line.replacingOccurrences(of: "1700000000", with: "1700086400")).write(
            to: logs.appendingPathComponent("HEAD"), atomically: true, encoding: .utf8)
        #expect(LocalSuggestionFileSystem.reflogDate(repo.path) == Date(timeIntervalSince1970: 1_700_086_400))
        #expect(LocalSuggestionFileSystem.reflogDate("/nonexistent/ov519") == nil)
    }

    @Test("A repository registered from the page opens its own Main, and only when one is new")
    func opensTheNewMain() {
        func repository(_ id: String) -> (host: String, repository: Repository) {
            ("", Repository(id: id, short: "r", displayName: id, remote: "", repositoryRootId: "root"))
        }
        let main = WorkspaceSummary(
            id: "main-new", name: "Main", taskPrefix: "new", isMain: true, ordinal: 0, repository: "new", orchestrator: nil)
        var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [], branchPrefix: nil)
        fleet.runnerWorkspaces[""] = [main]
        let both = [repository("old"), repository("new")]
        #expect(FleetPlaceholder.mainOfNewRepository(known: ["old"], in: both, fleet: fleet)?.workspace.id == "main-new")
        // Nothing new, or several new: never a guess.
        #expect(FleetPlaceholder.mainOfNewRepository(known: ["old", "new"], in: both, fleet: fleet) == nil)
        #expect(FleetPlaceholder.mainOfNewRepository(known: [], in: both, fleet: fleet) == nil)
        // The window goes there: the workspace, with nothing opened in it, so
        // its conversation column's invitation is what's on screen.
        #expect(
            ContentView.selectionAfterRegistering(known: ["old"], in: both, fleet: fleet, current: nil)
                == .workspace(host: "", workspace: "main-new", focus: nil))
        #expect(ContentView.selectionAfterRegistering(known: ["old", "new"], in: both, fleet: fleet, current: nil) == nil)
        // Somebody already in a place keeps it, as a returning user adding a
        // second repository from the sheet does.
        #expect(
            ContentView.selectionAfterRegistering(
                known: ["old"], in: both, fleet: fleet, current: .workspace(host: "", workspace: "main-old", focus: nil))
                == nil)
        #expect(ContentView.selectionAfterRegistering(known: ["old"], in: both, fleet: fleet, current: .needsYou) == nil)
    }
}
