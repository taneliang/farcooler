import Foundation
import Testing

@testable import Far_Cooler

/// Where a lockfile goes, and what it stops counting toward.
///
/// The rule itself is `GeneratedFile` and is tested in AgentKit, where both
/// clients can reach it. What is pinned here is the Mac's end of it: that
/// `reviewOrder` is the reordered list while `files` stays the daemon's, since
/// the pane draws and walks the first and the header counts the second, and
/// that a comparison with nothing generated in it comes back untouched — which
/// is what keeps this off every branch that has not regenerated anything.
@MainActor
struct GeneratedOrderTests {
    private static let workspaceJSON = """
        {"id":"w1","short":"w1","task":"t","branch":"feature",
        "worktree":"/tmp/w1","state":"ready","terminals":[]}
        """

    private func store(_ changeSet: String, scope: DiffScope = .branch) throws -> ChangesStore {
        let ws = try JSONDecoder().decode(Workspace.self, from: Data(Self.workspaceJSON.utf8))
        let s = ChangesStore(client: DaemonClient(target: ""), workspace: ws)
        s.changeSet = try JSONDecoder().decode(ChangeSet.self, from: Data(changeSet.utf8))
        s.scope = scope
        return s
    }

    private func file(_ path: String, _ insertions: Int, _ deletions: Int) -> String {
        """
        {"path":"\(path)","status":"modified","old_path":null,
         "insertions":\(insertions),"deletions":\(deletions),"binary":false}
        """
    }

    private func branch(_ files: [String], insertions: Int, deletions: Int) -> String {
        """
        {"branch":"feature","base_ref":"main","base_commit":"abc","head_commit":"def",
         "insertions":\(insertions),"deletions":\(deletions),"commits":[],
         "files":[\(files.joined(separator: ","))]}
        """
    }

    /// The shape the split exists for: eleven lines of work and a lockfile that
    /// moved four thousand.
    private func lockfileBranch() throws -> ChangesStore {
        try store(
            branch(
                [
                    file("Cargo.lock", 2000, 2012),
                    file("crates/daemon/src/a.rs", 8, 3),
                    file("crates/daemon/src/b.rs", 3, 0),
                ],
                insertions: 2011, deletions: 2015))
    }

    @Test("The daemon's order survives on `files` and only `reviewOrder` moves")
    func onlyReviewOrderMoves() throws {
        let s = try lockfileBranch()
        #expect(
            s.files.map(\.path) == [
                "Cargo.lock", "crates/daemon/src/a.rs", "crates/daemon/src/b.rs",
            ])
        #expect(
            s.reviewOrder.map(\.path) == [
                "crates/daemon/src/a.rs", "crates/daemon/src/b.rs", "Cargo.lock",
            ])
    }

    /// The one the movement arithmetic depends on. `moveFile` walks
    /// `reviewOrder` by index, so the last file in this list is the last place
    /// Next can land — and that has to be the lockfile rather than a source
    /// file eleven rows before the end of the daemon's order.
    @Test("Next reaches the lockfile last, not in the middle")
    func lockfileIsLast() throws {
        let s = try lockfileBranch()
        #expect(s.reviewOrder.last?.path == "Cargo.lock")
        #expect(s.reviewOrder.count == s.files.count, "nothing is hidden, only moved")
    }

    @Test("Nothing generated means nothing reordered")
    func inertWithoutGeneratedFiles() throws {
        let s = try store(
            branch(
                [file("crates/daemon/src/a.rs", 8, 3), file("README.md", 1, 1)],
                insertions: 9, deletions: 4))
        #expect(s.reviewOrder.map(\.path) == s.files.map(\.path))
        #expect(s.generatedFiles.isEmpty)
        #expect(s.handWrittenFiles.count == 2)
    }

    /// What the pane header draws instead of the daemon's whole-comparison
    /// number. `changeSet.insertions` is still 2,011 and is still right about
    /// the comparison; it is simply not the number the reader needs first.
    @Test("The subtotals hold the lockfile apart without losing it")
    func subtotals() throws {
        let s = try lockfileBranch()
        #expect(s.writtenInsertions == 11)
        #expect(s.writtenDeletions == 3)
        #expect(s.generatedInsertions == 2000)
        #expect(s.generatedDeletions == 2012)
        // Neither the daemon's count nor the split is wrong — they answer
        // different questions, and both are still on screen.
        #expect(s.changeSet.insertions == 2011)
    }

    /// Uncommitted reorders too. A regenerated lockfile is exactly as
    /// unhelpful to walk into before it is committed as after, and inside each
    /// group the working tree's own order — staged, unstaged, untracked —
    /// survives.
    @Test("Uncommitted is reordered on the same rule")
    func localScope() throws {
        let s = try store(
            """
            {"branch":"feature","base_ref":"main","base_commit":"abc","head_commit":"def",
             "insertions":0,"deletions":0,"commits":[],"files":[],
             "working_tree":{
               "staged":["Cargo.lock"],"unstaged":["src/main.rs"],
               "untracked":["src/new.rs"],"conflicted":[],
               "changes":[
                 {"path":"Cargo.lock","status":"modified","old_path":null,
                  "insertions":40,"deletions":40,"binary":false},
                 {"path":"src/main.rs","status":"modified","old_path":null,
                  "insertions":2,"deletions":1,"binary":false}]}}
            """,
            scope: .local)
        #expect(s.files.map(\.path) == ["Cargo.lock", "src/main.rs", "src/new.rs"])
        #expect(s.reviewOrder.map(\.path) == ["src/main.rs", "src/new.rs", "Cargo.lock"])
    }
}
