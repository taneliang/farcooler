import Foundation
import Testing

@testable import AgentKit

/// One pull request that wants a review, seen by two runners signed in as one
/// GitHub login, is one thing to do (ov-589). Every total the apps show is the
/// length of a merged list, never a sum of each runner's own count, so the
/// duplicate has to be gone from the list the total is taken from.
@Suite struct NeedsYouRunnerCountTests {
    private func request(_ id: String, rank: UInt32) -> NeedsYouItem {
        NeedsYouItem(id: id, kind: .prReview, rank: rank, since: nil, question: "PR")
    }

    private func ask(_ id: String) -> NeedsYouItem {
        NeedsYouItem(id: id, kind: .ask, rank: 5, since: nil, question: "?")
    }

    /// Mutation: `merge` without its `seenRequests` filter. Red: 3, not 2.
    @Test func theAppsTotalCountsARequestOnTwoRunnersOnce() {
        let shown = PhoneInbox.shown(
            lists: [
                "studio": [request("pr-review:aaaa", rank: 500), ask("ask:1")],
                "build-box": [request("pr-review:aaaa", rank: 400)],
            ],
            unread: [:])
        #expect(shown.count == 2)
        #expect(shown.filter { $0.kind == .prReview }.count == 1)
    }

    /// The widget, complication and watch count `needingYou`, from the lists
    /// the publication holds. Mutation: as above. Red: 3, not 2.
    @Test func theGlancesCountARequestOnTwoRunnersOnce() {
        let now = Date(timeIntervalSince1970: 1_790_000_000)
        let empty = FleetSnapshot(agents: [], capturedAt: now, complete: true)
        var publication = FleetPublication()
        publication.record(runner: "a", snapshot: empty)
        publication.record(runner: "b", snapshot: empty)
        publication.record(needsYou: [
            "a": [request("pr-review:aaaa", rank: 500), ask("ask:1")],
            "b": [request("pr-review:aaaa", rank: 400)],
        ])
        #expect(publication.merged(at: now).needingYou == 2)
    }
}
