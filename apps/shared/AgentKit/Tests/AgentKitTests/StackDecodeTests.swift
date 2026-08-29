import Foundation
import Testing

@testable import AgentKit

/// `stack_json`, transcribed key for key — and the row built off it.
///
/// The same discipline `FleetDecodeTests` follows, and for the same reason:
/// `Connection.stack` decodes this with a plain `JSONDecoder`, which ignores a
/// key nobody declared and leaves an OPTIONAL whose key never arrived as nil.
/// Neither is a throw, a warning or a log line. The payload below is copied
/// from the `json!` block in `stack_json` (`crates/client/src/session.rs`),
/// every key it emits, with the types that function writes.
///
/// Android has had this test since the same call reached that phone
/// (`StackTest.kt`); this is the iOS side of it, and it exists at all because
/// `CoreModel.swift` lives in AgentKit — the app target's only test bundle is a
/// UI-testing one, which cannot reach these types.
struct StackDecodeTests {
    static let stackJSON = """
    {
      "cycleDetected": false,
      "prKnown": true,
      "repoUrl": "https://github.com/o/overnight",
      "links": [
        {
          "branch": "feat/review-notes",
          "parentBranch": "feat/review",
          "parentGuessed": false,
          "ahead": 3,
          "behind": 1,
          "pr": {
            "number": 412,
            "url": "https://github.com/o/overnight/pull/412",
            "state": "open",
            "checks": "failing",
            "review": "changes_requested",
            "headOid": "9f21c0d4e5f6",
            "mergedAt": null,
            "fetchedAt": 1785925800000,
            "stale": true
          }
        },
        {
          "branch": "feat/review",
          "parentBranch": "main",
          "parentGuessed": true,
          "ahead": 0,
          "behind": 0,
          "pr": null
        }
      ]
    }
    """

    @Test func aStackDecodesEveryKeyStackJsonEmits() throws {
        let reply = try JSONDecoder().decode(
            StackResponse.self, from: Data(Self.stackJSON.utf8))

        #expect(reply.cycleDetected == false)
        #expect(reply.links.count == 2)
        // `gh` answered, and said what this repository is: the two facts that
        // decide whether the app may offer to create a pull request, and where
        // that offer would go.
        #expect(reply.prAnswered)
        #expect(reply.repoUrl == "https://github.com/o/overnight")

        let top = reply.links[0]
        #expect(top.branch == "feat/review-notes")
        #expect(top.parentBranch == "feat/review")
        #expect(top.parentGuessed == false)
        #expect(top.ahead == 3)
        #expect(top.behind == 1)

        let pr = try #require(top.pr)
        #expect(pr.number == 412)
        #expect(pr.url == "https://github.com/o/overnight/pull/412")
        #expect(pr.state == "open")
        #expect(pr.checks == "failing")
        #expect(pr.review == "changes_requested")
        #expect(pr.headOid == "9f21c0d4e5f6")
        // Nil and not zero: a pull request that has not landed has no date, and
        // zero would date it to 1970.
        #expect(pr.mergedAt == nil)
        #expect(pr.fetchedAt == 1_785_925_800_000)
        // Only displayable at all since the daemon started computing it from
        // `fetchedAt`; it was hardwired false at both construction sites before.
        #expect(pr.stale)

        // Absent rather than an empty record: a branch with no pull request and
        // a pull request nobody could read are different answers, and the next
        // test is about which one this is.
        #expect(reply.links[1].pr == nil)
    }

    /// The field that decides whether this app may offer an action, read the
    /// only safe way.
    ///
    /// A runner too old to send `prKnown` cannot have told us `gh` answered.
    /// The decode must survive it — an older runner losing the whole stack
    /// screen over one key would be a worse bug than the one this gates — and
    /// it must come out meaning "we could not ask", which is the reading that
    /// keeps Create Pull Request off a branch that already has one.
    @Test func anOlderRunnerSaysNothingAboutGhRatherThanFailingToDecode() throws {
        let json = """
        {"cycleDetected":false,"links":[{"branch":"main","parentBranch":"",\
        "parentGuessed":false,"ahead":0,"behind":0,"pr":null}]}
        """
        let reply = try JSONDecoder().decode(StackResponse.self, from: Data(json.utf8))
        #expect(reply.prKnown == nil)
        #expect(reply.prAnswered == false)
        #expect(reply.repoUrl == nil)
    }
}

/// What the branch header says about a pull request, and how loudly.
///
/// Pure and out here rather than assembled in a SwiftUI body, for the reason
/// Android's `Stack.kt` gives at the same place: a sentence built inside a view
/// is a sentence nothing can check.
struct PullRequestRowTests {
    private func pr(
        state: String = "open",
        checks: String = "unknown",
        review: String = "unknown"
    ) -> PullRequest {
        PullRequest(
            number: 335, url: "https://github.com/o/overnight/pull/335",
            state: state, checks: checks, review: review, stale: false,
            headOid: nil, mergedAt: nil, fetchedAt: nil)
    }

    /// The sentence is the two facts that decide what happens next, in the
    /// order they are asked about.
    @Test func theRowSaysWhereItGotToAndWhetherItBuilds() {
        #expect(
            pr(checks: "passing", review: "approved").headerSentence
                == "Approved · Checks passed")
        #expect(
            pr(checks: "failing", review: "changes_requested").headerSentence
                == "Changes requested · Checks failed")
        #expect(
            pr(state: "draft", checks: "pending", review: "review_required").headerSentence
                == "Draft · Checks running")
        // A repository with no CI at all is most of them. "Checks unknown" on
        // every row would be a word spent to say nothing.
        #expect(pr(review: "approved").headerSentence == "Approved")
        // And no wire word reaches the screen raw: an underscore mid-sentence
        // is a leaked enum name, which the Apple copy conventions rule out.
        #expect(!pr(review: "changes_requested").headerSentence.contains("_"))
        // A state this build predates says nothing rather than printing itself.
        #expect(pr(state: "something_new", checks: "passing").headerSentence == "Checks passed")
    }

    /// The whole rule the row is built on: quiet when healthy.
    ///
    /// An open, approved, green pull request must draw no color and no glyph
    /// at all. That is what makes the failing one glanceable at arm's length,
    /// and it is the position `71934f8` took when it turned the last permanent
    /// green dot neutral.
    @Test func aHealthyPullRequestIsGreyAndSilent() {
        let healthy = pr(checks: "passing", review: "approved")
        #expect(healthy.emphasis == .quiet)
        #expect(healthy.headerSymbol == nil)
    }

    @Test func onlyTroubleIsLoud() {
        #expect(pr(checks: "failing", review: "approved").emphasis == .alarm)
        #expect(pr(checks: "passing", review: "changes_requested").emphasis == .alarm)
        #expect(pr(checks: "pending", review: "approved").emphasis == .pending)
        // Exactly one glyph, and only on the thing this row exists to make
        // glanceable. A column of icons is a column nobody reads.
        #expect(pr(checks: "failing").headerSymbol == "xmark.circle.fill")
        #expect(pr(checks: "pending").headerSymbol == nil)
    }

    /// A landed or abandoned pull request is history, and history is quiet.
    ///
    /// Its checks and its review describe a decision already taken, so a red on
    /// either would be shouting about a question nobody is being asked.
    @Test func aMergedPullRequestIsNotableRatherThanActionable() {
        let merged = pr(state: "merged", checks: "failing", review: "changes_requested")
        #expect(merged.emphasis == .quiet)
        #expect(merged.headerSymbol == "arrow.triangle.merge")
        #expect(merged.headerSentence.hasPrefix("Merged"))

        let closed = pr(state: "closed", checks: "failing")
        #expect(closed.emphasis == .quiet)
        #expect(closed.headerSymbol == nil)
    }
}

/// The compare URL, which is the whole of the Create Pull Request button.
struct PullRequestLinkTests {
    private let repo = "https://github.com/o/overnight"

    @Test func theCompareUrlOpensTheFormAlreadyUnfolded() {
        let url = PullRequestLink.compare(
            repoURL: repo, baseRef: "origin/main", head: "feat/handle-retries")
        #expect(
            url?.absoluteString
                == "https://github.com/o/overnight/compare/main...feat/handle-retries?expand=1")
    }

    /// `baseRef` is a git ref, and only the REMOTE comes off it.
    ///
    /// `resolve_base` hands back a recorded ref, a pull request's base, or a
    /// default branch, and only some of those are remote-qualified — so both
    /// shapes have to survive. The last case is the one that separates this
    /// from `default_branch`'s `rsplit('/')`: that idiom is peeling
    /// `origin/HEAD`, where the last component is the whole answer, and here it
    /// would leave `2.0`, which names no branch at all.
    @Test func onlyTheRemoteComesOffTheBaseRef() {
        #expect(PullRequestLink.branchName("origin/main") == "main")
        #expect(PullRequestLink.branchName("main") == "main")
        #expect(PullRequestLink.branchName("upstream/main") == "main")
        #expect(PullRequestLink.branchName("refs/remotes/origin/main") == "main")
        #expect(PullRequestLink.branchName("refs/heads/release/2.0") == "release/2.0")
        #expect(PullRequestLink.branchName("origin/release/2.0") == "release/2.0")
        // A remote this client cannot name is left whole rather than guessed
        // at: a wrong strip invents a branch, and a missed one still resolves.
        #expect(PullRequestLink.branchName("release/2.0") == "release/2.0")
    }

    /// No link to nowhere.
    ///
    /// `repoUrl` is null exactly when `gh` could not say what this repository
    /// is, and a button that opens a broken page is worse than no button.
    @Test func nothingIsOfferedWhenAHalfIsMissing() {
        #expect(
            PullRequestLink.compare(repoURL: nil, baseRef: "origin/main", head: "feat/x") == nil)
        #expect(PullRequestLink.compare(repoURL: "", baseRef: "main", head: "feat/x") == nil)
        #expect(PullRequestLink.compare(repoURL: repo, baseRef: "", head: "feat/x") == nil)
        #expect(PullRequestLink.compare(repoURL: repo, baseRef: "main", head: "") == nil)
        // The repository's own checkout sits ON the base branch. `main...main`
        // is a compare page with nothing on it and a button GitHub refuses.
        #expect(PullRequestLink.compare(repoURL: repo, baseRef: "origin/main", head: "main") == nil)
    }
}
