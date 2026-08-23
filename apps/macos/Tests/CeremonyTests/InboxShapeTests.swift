import Foundation
import Testing

@testable import Far_Cooler

/// The two envelopes `changes inbox --json` has arrived in.
///
/// This app shells out to the runner's `farcooler`, so the runner can be older
/// than the app — and the inbox is the one `changes` answer whose shape changed
/// rather than merely gaining keys. Before the CLI and the FFI became one
/// builder it printed a bare array whose `workspace_id` was the eight-character
/// short; it prints `{"items": …, "elsewhere": n}` with the full UUID now, the
/// same object the phones read.
///
/// Both must decode, and both must key the sidebar's lookup the same way. The
/// failure this pins is silent in both directions: `refreshChangesInbox` returns
/// on a decode failure, so a wrong envelope is a sidebar with every diff count
/// gone and nothing said, and a wrong key is the same thing with the rows still
/// arriving.
struct InboxShapeTests {
    /// What a current runner sends.
    private static let object = """
        {"items":[
          {"workspace_id":"018f7c1e-0000-7000-8000-0123456789ab",
           "short":"456789ab","task_name":"ship the thing","branch":"feature",
           "changed_since_reviewed":true,"insertions":12,"deletions":3}],
         "elsewhere":2}
        """

    /// What a runner from before the unification sends: no envelope, no
    /// `short`, and the short id sitting in `workspace_id`.
    private static let array = """
        [{"workspace_id":"456789ab","task_name":"ship the thing","branch":"feature",
          "changed_since_reviewed":true,"insertions":12,"deletions":3}]
        """

    @Test func theCurrentShapeDecodesWithItsKeysIntact() throws {
        let rows = try #require(InboxReply.rows(from: Data(Self.object.utf8)))
        #expect(rows.count == 1)
        let r = try #require(rows.first)
        #expect(r.workspaceId == "018f7c1e-0000-7000-8000-0123456789ab")
        #expect(r.short == "456789ab")
        #expect(r.changedSinceReviewed)
        #expect(r.insertions == 12)
        #expect(r.deletions == 3)
        #expect(r.hasDiff)
    }

    @Test func anOlderRunnersBareArrayStillDecodes() throws {
        let rows = try #require(InboxReply.rows(from: Data(Self.array.utf8)))
        #expect(rows.count == 1)
        #expect(rows.first?.workspaceId == "456789ab")
        #expect(rows.first?.short == nil, "the key did not exist yet")
    }

    /// The lookup key is the same string either way round, which is the whole
    /// point of carrying both halves: the sidebar asks by `workspace.short`.
    @Test func bothShapesKeyTheSidebarLookupIdentically() throws {
        let new = try #require(InboxReply.rows(from: Data(Self.object.utf8))).first
        let old = try #require(InboxReply.rows(from: Data(Self.array.utf8))).first
        #expect(new?.short ?? new?.workspaceId == "456789ab")
        #expect(old?.short ?? old?.workspaceId == "456789ab")
    }

    /// An empty inbox is a decode that succeeds with nothing in it, not a
    /// decode that fails — otherwise a clean fleet is indistinguishable from a
    /// runner that cannot answer, and the sidebar would keep the last numbers
    /// it saw.
    @Test func anEmptyInboxIsNotAFailure() throws {
        let rows = try #require(InboxReply.rows(from: Data(#"{"items":[],"elsewhere":0}"#.utf8)))
        #expect(rows.isEmpty)
        #expect(InboxReply.rows(from: Data("[]".utf8))?.isEmpty == true)
    }

    /// Not JSON at all is nil, so the caller keeps what it had rather than
    /// blanking the sidebar on one bad read.
    @Test func somethingThatIsNotAnInboxIsNil() {
        #expect(InboxReply.rows(from: Data("not json".utf8)) == nil)
    }
}
