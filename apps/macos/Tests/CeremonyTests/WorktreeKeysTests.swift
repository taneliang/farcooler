import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Every key the CLI renamed from `workspace` to `worktree`, read back by the
/// type that reads it.
///
/// Each fixture is the object `crates/cli/src/main.rs` builds with `json!`,
/// cut down to the keys the Swift type needs. A key this app still spelled the
/// old way would not fail loudly anywhere: the event stream decodes each line
/// with `try?` and skips what it cannot read, and a fleet that fails to decode
/// is a runner that looks unreachable. So the keys are pinned here instead.
struct WorktreeKeysTests {
    /// `worktree list --json`: the envelope is `worktrees`, and each row's
    /// `worktree` is still its directory.
    @Test func theFleetIsReadFromWorktrees() throws {
        let json = """
            {"runtime_healthy":true,"live_panes":0,"branch_prefix":"",
             "worktrees":[{"id":"w-1","short":"w1","task":"fix-it","branch":"fix-it",
                           "worktree":"/tmp/worktrees/repo/fix-it","state":"active",
                           "terminals":[]}]}
            """
        let fleet = try JSONDecoder().decode(Fleet.self, from: Data(json.utf8))
        #expect(fleet.worktrees.map(\.id) == ["w-1"])
        #expect(fleet.worktrees.first?.path == "/tmp/worktrees/repo/fix-it")
    }

    /// `events`, a terminal line: `terminal_event_json`.
    @Test func aTerminalEventNamesItsWorktree() throws {
        let json = """
            {"kind":"terminal","id":"t-1","short":"t1","worktree":"w-1","title":"claude",
             "preset":"claude","state":"running"}
            """
        let event = try JSONDecoder().decode(TerminalEvent.self, from: Data(json.utf8))
        #expect(event.worktree == "w-1")
    }

    /// `events`, a terminal line carrying a held draft (ov-385), in the shape
    /// `draft_hold_json` writes it; and the event applied to the row moves it.
    @Test func aTerminalEventCarriesItsHeldDraft() throws {
        let json = """
            {"kind":"terminal","id":"t-1","short":"t1","worktree":"w-1","title":"claude",
             "preset":"claude","state":"running",
             "draftHold":{"id":"0198f2c0-0000-7000-8000-00000000f385","state":"sent","heldMs":5,"expiresMs":1800005,"endedMs":9}}
            """
        let event = try JSONDecoder().decode(TerminalEvent.self, from: Data(json.utf8))
        #expect(event.draftHold == DraftHold(id: "0198f2c0-0000-7000-8000-00000000f385", state: .sent, expiresMs: 1_800_005))
        let none = try JSONDecoder().decode(TerminalEvent.self, from: Data("""
            {"kind":"terminal","id":"t-1","short":"t1","worktree":"w-1","title":"claude",
             "preset":"claude","state":"running","draftHold":null}
            """.utf8))
        #expect(none.draftHold == nil)
    }

    /// `events`, a layout line.
    @Test func aLayoutEventNamesItsWorktree() throws {
        let json = """
            {"kind":"layout","worktree":"w-1","groups":[]}
            """
        let event = try JSONDecoder().decode(LayoutEvent.self, from: Data(json.utf8))
        #expect(event.worktree == "w-1")
    }

    /// `layout show --json`, and every `layout …` reply: `layout_json`.
    @Test func aLayoutReplyNamesItsWorktree() throws {
        let json = """
            {"worktree":"w-1","groups":[]}
            """
        let list = try JSONDecoder().decode(PaneGroupList.self, from: Data(json.utf8))
        #expect(list.worktree == "w-1")
    }

    /// `changes inbox --json`: each item's id is `worktree_id`.
    @Test func anInboxRowNamesItsWorktree() throws {
        let json = """
            {"items":[{"worktree_id":"018f7c1e-0000-7000-8000-0123456789ab","short":"456789ab",
                       "changed_since_reviewed":false,"insertions":1,"deletions":0}],
             "elsewhere":0}
            """
        let rows = try #require(InboxReply.rows(from: Data(json.utf8)))
        #expect(rows.map(\.worktreeId) == ["018f7c1e-0000-7000-8000-0123456789ab"])
    }
}
