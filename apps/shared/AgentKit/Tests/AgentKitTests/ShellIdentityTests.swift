import Foundation
import Testing

@testable import AgentKit

/// What the shell's ids have to say, now that it holds several runners' fleets.
///
/// Every one of these fails against the composition the shell used before the
/// port — `"\(worktree)/\(pane.id)"`, with no runner in it — which is the
/// whole reason they are here.
///
/// **The worktree ids below are written short for readability, and that is a
/// fixture choice rather than the wire's shape.** The app decodes the daemon's
/// full UUIDv7; the eight-character form is `Worktree.short`, which nothing
/// uses as an identity. See `ShellIdentity`, which corrects the premise this
/// port was scoped on. What these pin is not that a collision happens — it is
/// that a tab id names the runner, so resolving one back to a connection is a
/// lookup rather than a search across every runner for a matching id.
struct ShellIdentityTests {
    /// One worktree id, two runners, two different worktrees.
    ///
    /// The mutation this exists for is deleting the runner from
    /// `ShellIdentity.worktree`: it leaves every single-runner screen working
    /// and leaves the merged fleet with two cards under one identity.
    @Test func twoRunnersSharingAWorktreeIDAreStillTwoWorktrees() {
        let one = ShellIdentity.worktree(runner: "RUNNER-A", worktree: "3f9a1c07")
        let two = ShellIdentity.worktree(runner: "RUNNER-B", worktree: "3f9a1c07")
        #expect(one != two)
    }

    /// The same, one level down, where it would cost a mounted pane rather than
    /// a card: `ShellPaneTrack` retains by tab id.
    @Test func twoRunnersSharingAWorktreeIDAreStillTwoTabs() {
        let one = ShellIdentity.tab(runner: "RUNNER-A", worktree: "3f9a1c07", pane: "changes")
        let two = ShellIdentity.tab(runner: "RUNNER-B", worktree: "3f9a1c07", pane: "changes")
        #expect(one != two)
    }

    /// The rule that predates the port and must survive it: the Changes pane
    /// has ONE pane id for the whole app, so the worktree has to be in the tab
    /// id or every worktree's diff is the same tab.
    @Test func twoWorktreesOnOneRunnerHaveDifferentChangesTabs() {
        let one = ShellIdentity.tab(runner: "RUNNER-A", worktree: "3f9a1c07", pane: "changes")
        let two = ShellIdentity.tab(runner: "RUNNER-A", worktree: "b21e4d55", pane: "changes")
        #expect(one != two)
    }

    /// Two panes of one worktree are two tabs. The case that was never broken,
    /// pinned so a composition that dropped the PANE instead would be caught by
    /// the same file.
    @Test func twoPanesOfOneWorktreeAreTwoTabs() {
        let diff = ShellIdentity.tab(runner: "RUNNER-A", worktree: "3f9a1c07", pane: "changes")
        let agent = ShellIdentity.tab(runner: "RUNNER-A", worktree: "3f9a1c07", pane: "t-77")
        #expect(diff != agent)
    }

    /// A tab id is its worktree's id plus the pane, so a screen holding one
    /// and a screen holding the other are talking about the same worktree.
    ///
    /// Not a decoder — nothing parses these apart, and `ShellIdentity`'s header
    /// says why. What this pins is that the two functions compose the same
    /// prefix, which is what lets `ShellFleetMap` key one side table by
    /// worktree and another by tab without a second spelling of "the same
    /// runner".
    @Test func aTabIDIsBuiltOnItsWorktreeID() {
        let worktree = ShellIdentity.worktree(runner: "RUNNER-A", worktree: "3f9a1c07")
        let tab = ShellIdentity.tab(runner: "RUNNER-A", worktree: "3f9a1c07", pane: "t-77")
        #expect(tab.hasPrefix(worktree))
    }

    /// The cache next door already spells a remembered tab
    /// `"\(runner)/\(worktree)/\(index)"` — `RunnerDirectory.group()` — and
    /// has since before any of this. A live tab and a cached one that named the
    /// same worktree differently would be two answers to "is this the same
    /// thing", which is the drift the composition is centralized to prevent.
    @Test func theLiveSpellingMatchesTheCacheTheGridAlreadyWrites() {
        #expect(
            ShellIdentity.tab(runner: "RUNNER-A", worktree: "3f9a1c07", pane: "0")
                == "RUNNER-A/3f9a1c07/0")
    }

    // MARK: - What VoiceOver says (ov-55 4A.4)

    /// **No VoiceOver label or hint on the phone says pane, tab or session.**
    /// A terminal is an agent or a terminal, and the pane, the tab and the
    /// session are this app's plumbing (spec §1). A source scrape over
    /// `apps/ios/FarCooler`, of every `accessibilityLabel` and
    /// `accessibilityHint` given a string: the iOS target has no unit tests
    /// to ask the rendered tree, and this is where the words are written.
    ///
    /// One exemption, by name: the key row's Tab key is called "Tab", which
    /// is what the key is, not a tab of the app.
    @Test("No VoiceOver label says pane, tab or session")
    func noVoiceOverLabelSaysPaneTabOrSession() throws {
        // The scrape itself fails on a planted string, so an empty result
        // below means the words aren't there, not that nothing was read.
        let planted = #"x.accessibilityLabel("Close \(name) Pane").accessibilityHint("Switches tab")"#
        #expect(Self.offenders(in: planted).count == 2)
        #expect(Self.offenders(in: #".accessibilityLabel("Tab")"#).isEmpty)

        let root = try #require(Self.repositoryRoot(), "cannot find the repository")
        let app = root.appendingPathComponent("apps/ios/FarCooler")
        let files = try #require(
            FileManager.default.enumerator(at: app, includingPropertiesForKeys: nil)?
                .compactMap { $0 as? URL }.filter { $0.pathExtension == "swift" })
        var scanned = 0
        var found: [String] = []
        for file in files {
            guard let source = try? String(contentsOf: file, encoding: .utf8) else { continue }
            scanned += 1
            found += Self.offenders(in: source).map { "\(file.lastPathComponent): \($0)" }
        }
        #expect(scanned > 40, "the scrape read \(scanned) files")
        #expect(found.isEmpty, "\(found)")
    }

    /// The labels and hints in `source` whose words include pane, tab or
    /// session, except the Tab key's own name.
    static func offenders(in source: String) -> [String] {
        let call = try! NSRegularExpression(
            pattern: #"accessibility(?:Label|Hint)\(\s*(?:Text\(\s*)?"((?:[^"\\]|\\.)*)""#)
        let word = try! NSRegularExpression(
            pattern: #"\b(?:panes?|tabs?|sessions?)\b"#, options: .caseInsensitive)
        let range = NSRange(source.startIndex..., in: source)
        return call.matches(in: source, range: range).compactMap { match in
            guard let text = Range(match.range(at: 1), in: source).map({ String(source[$0]) }),
                text != "Tab"
            else { return nil }
            let words = NSRange(text.startIndex..., in: text)
            return word.firstMatch(in: text, range: words) == nil ? nil : text
        }
    }

    private static func repositoryRoot() -> URL? {
        var directory = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
        while directory.path != "/" {
            if FileManager.default.fileExists(
                atPath: directory.appendingPathComponent("apps/ios/generate-project.py").path)
            {
                return directory
            }
            directory = directory.deletingLastPathComponent()
        }
        return nil
    }
}
