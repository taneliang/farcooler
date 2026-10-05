import Foundation
import Testing

@testable import AgentKit

/// When a worktree says large files weren't downloaded, and who may try again
/// (ov-199). Android's `LfsNoticeTest` holds the same decisions.
struct LfsNoticeTests {
    @Test("A worktree with no pointer files says nothing")
    func nothing() {
        #expect(LfsNotice.make(pointers: nil, mayAct: true) == nil)
        #expect(LfsNotice.make(pointers: 0, mayAct: true) == nil)
    }

    @Test("A worktree with pointer files says so; a read grant sees the sentence without the button")
    func says() throws {
        #expect(LfsNotice.make(pointers: 2, mayAct: true) == LfsNotice(pointers: 2, canRetry: true))
        let read = try #require(LfsNotice.make(pointers: 2, mayAct: false))
        #expect(!read.canRetry)
    }

    @Test("The words are the card's")
    func words() {
        #expect(LfsNotice.title == "Some large files weren’t downloaded.")
        #expect(LfsNotice.detail.contains("“git lfs pull” in this worktree"), "a pull in the main checkout misses another branch's objects")
        #expect(!LfsNotice.retrying.contains("Download"), "the runner never fetches; it copies from its own store")
        #expect(LfsNotice.retry == "Try Again")
    }

    @Test("A refusal says the shared sentence, and anything else says the runner couldn’t be asked")
    func failures() {
        #expect(
            LfsNotice.failure(word: "capability-unsupported")
                == "Couldn’t ask the runner to try again. This runner’s Far Cooler is too old for this. Update it there, then try again.")
        #expect(LfsNotice.failure(word: nil) == LfsNotice.unreachable)
        #expect(LfsNotice.failure(word: "no-such-word") == LfsNotice.unreachable)
    }

    @Test("A worktree row decodes its pointer count under lfs_pointers")
    func decodes() throws {
        let row = #"{"id":"w1","short":"w1","task":"t","branch":"b","worktree":"/x","state":"ready","terminals":[],"lfs_pointers":3}"#
        let worktree = try JSONDecoder().decode(Worktree.self, from: Data(row.utf8))
        #expect(worktree.lfsPointers == 3)
        let old = try JSONDecoder().decode(Worktree.self, from: Data(row.replacingOccurrences(of: #","lfs_pointers":3"#, with: "").utf8))
        #expect(old.lfsPointers == nil, "a runner that predates the count says nothing")
    }
}
