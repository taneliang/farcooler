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
    /// `accessibilityHint`: the iOS target has no unit tests to ask the
    /// rendered tree, and this is where the words are written.
    ///
    /// Given a string, that string. Given anything else — a ternary, a
    /// `??`, a property or a function (`.accessibilityHint(opens)`, ov-66) —
    /// every string in the argument, and every string in the body of each
    /// `var` or `func` it names in the same file. What a label reads out of
    /// the runner's data (a worktree's name) is the runner's words, not
    /// this app's, and no scrape can read it.
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
        // A variable hint, and a choice between two labels.
        let variable = #"""
            row.accessibilityHint(opens)
            private var opens: String {
                switch kind {
                case .agent: "Opens the agent’s pane"
                default: "Opens the task"
                }
            }
            x.accessibilityLabel(more ? "Next tab" : "Next")
            y.accessibilityLabel(spoken(count))
            func spoken(_ n: Int) -> String { n == 1 ? "1 session" : "\(n) things" }
            z.accessibilityLabel(worktree.name)
            """#
        #expect(
            Self.offenders(in: variable) == ["Opens the agent’s pane", "Next tab", "1 session"])

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
    /// session, except the Tab key's own name: the strings in each call's
    /// argument, and in the bodies of the properties and functions it names.
    static func offenders(in source: String) -> [String] {
        let word = try! NSRegularExpression(
            pattern: #"\b(?:panes?|tabs?|sessions?)\b"#, options: .caseInsensitive)
        func says(_ text: String) -> Bool {
            text != "Tab" && word.firstMatch(in: text, range: NSRange(text.startIndex..., in: text)) != nil
        }
        var found: [String] = []
        for argument in arguments(in: source) {
            found += literals(in: argument).filter(says)
            for name in names(in: argument) {
                for body in bodies(of: name, in: source) {
                    found += literals(in: body).filter(says)
                }
            }
        }
        return found
    }

    /// The text between the parentheses of every `accessibilityLabel(` and
    /// `accessibilityHint(` call, strings skipped when counting parentheses.
    private static func arguments(in source: String) -> [String] {
        let call = try! NSRegularExpression(pattern: #"accessibility(?:Label|Hint)\("#)
        let text = Array(source.unicodeScalars)
        return call.matches(in: source, range: NSRange(source.startIndex..., in: source)).compactMap {
            match in
            guard let range = Range(match.range, in: source) else { return nil }
            let open = source.unicodeScalars.distance(
                from: source.unicodeScalars.startIndex, to: range.upperBound.samePosition(in: source.unicodeScalars)!)
            return balanced(text, from: open, open: "(", close: ")")
        }
    }

    /// The text from `start` to the bracket that closes the one before it.
    private static func balanced(
        _ text: [Unicode.Scalar], from start: Int, open: Unicode.Scalar, close: Unicode.Scalar
    ) -> String {
        var depth = 1
        var inString = false
        var index = start
        while index < text.count {
            let c = text[index]
            if inString {
                if c == "\\" { index += 1 } else if c == "\"" { inString = false }
            } else if c == "\"" {
                inString = true
            } else if c == open {
                depth += 1
            } else if c == close {
                depth -= 1
                if depth == 0 { break }
            }
            index += 1
        }
        var out = String.UnicodeScalarView()
        out.append(contentsOf: text[start..<min(index, text.count)])
        return String(out)
    }

    /// The string literals in `code`.
    private static func literals(in code: String) -> [String] {
        let literal = try! NSRegularExpression(pattern: #""((?:[^"\\]|\\.)*)""#)
        return literal.matches(in: code, range: NSRange(code.startIndex..., in: code)).compactMap {
            Range($0.range(at: 1), in: code).map { String(code[$0]) }
        }
    }

    /// The lower-case names in `code` outside its strings: what a property
    /// or a function it calls could be called.
    private static func names(in code: String) -> Set<String> {
        let literal = try! NSRegularExpression(pattern: #""(?:[^"\\]|\\.)*""#)
        let bare = literal.stringByReplacingMatches(
            in: code, range: NSRange(code.startIndex..., in: code), withTemplate: " ")
        let name = try! NSRegularExpression(pattern: #"\b[a-z][A-Za-z0-9_]*\b"#)
        return Set(
            name.matches(in: bare, range: NSRange(bare.startIndex..., in: bare)).compactMap {
                Range($0.range, in: bare).map { String(bare[$0]) }
            })
    }

    /// The bodies of every `var name` and `func name` in `source` that has
    /// one.
    private static func bodies(of name: String, in source: String) -> [String] {
        let declaration = try! NSRegularExpression(
            pattern: #"\b(?:var|func)\s+"# + NSRegularExpression.escapedPattern(for: name)
                + #"\b[^{\n]*\{"#)
        let text = Array(source.unicodeScalars)
        return declaration.matches(in: source, range: NSRange(source.startIndex..., in: source))
            .compactMap { match in
                guard let range = Range(match.range, in: source) else { return nil }
                let open = source.unicodeScalars.distance(
                    from: source.unicodeScalars.startIndex,
                    to: range.upperBound.samePosition(in: source.unicodeScalars)!)
                return balanced(text, from: open, open: "{", close: "}")
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
