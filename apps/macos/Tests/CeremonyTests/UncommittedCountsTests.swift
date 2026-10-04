import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// What the Uncommitted header adds up.
///
/// The defect these pin: the counts used to be computed from `fileDiffs`, which
/// fills a file at a time as rows scroll into view, so the header started near
/// zero on a worktree long enough to scroll and climbed as the reader went down
/// it — and emptied again on every scope switch and every Refresh. The numbers
/// now come from the working tree the daemon already sent, so they are whole
/// before a single diff has been read. Every store here has an EMPTY
/// `fileDiffs`, which is the point.
@MainActor
struct UncommittedCountsTests {
    private static let worktreeJSON = """
        {"id":"w1","short":"w1","task":"t","branch":"feature",
        "worktree":"/tmp/w1","state":"ready","terminals":[]}
        """

    private func store(_ changeSet: String) throws -> ChangesStore {
        let ws = try JSONDecoder().decode(Worktree.self, from: Data(Self.worktreeJSON.utf8))
        let s = ChangesStore(client: DaemonClient(target: ""), worktree: ws)
        s.changeSet = try JSONDecoder().decode(ChangeSet.self, from: Data(changeSet.utf8))
        s.scope = .local
        return s
    }

    /// The whole change set as `farcooler changes status --json` prints it,
    /// with one file in each group git keeps apart.
    private static let dirty = """
        {"branch":"feature","base_ref":"main","base_commit":"abc","head_commit":"def",
         "insertions":2,"deletions":0,"commits":[],"files":[],
         "working_tree":{
           "staged":["staged.txt"],"unstaged":["README.md"],
           "untracked":["new.txt"],"conflicted":[],
           "changes":[
             {"path":"staged.txt","status":"added","old_path":null,
              "insertions":2,"deletions":0,"binary":false},
             {"path":"README.md","status":"modified","old_path":null,
              "insertions":1,"deletions":3,"binary":false},
             {"path":"new.txt","status":"untracked","old_path":null,
              "insertions":3,"deletions":0,"binary":false}]}}
        """

    @Test func everyDirtyFileCarriesItsCountsBeforeAnyDiffIsRead() throws {
        let s = try store(Self.dirty)
        #expect(s.fileDiffs.isEmpty, "nothing has been scrolled into view")
        let byPath = Dictionary(uniqueKeysWithValues: s.files.map { ($0.path, $0) })
        #expect(byPath["staged.txt"]?.insertions == 2)
        #expect(byPath["README.md"]?.deletions == 3)
        #expect(byPath["new.txt"]?.insertions == 3, "a file git has never seen still has lines")
    }

    /// An untracked file is read like any other, and its row keeps a small
    /// "Untracked" label. The daemon diffs it against nothing, so there is
    /// content under the label rather than a placeholder in its place.
    @Test func anUntrackedFileIsReadAndKeepsItsLabel() async throws {
        let s = try store(Self.dirty)
        var asked: [String] = []
        s.diffSource = { path in
            asked.append(path)
            return .success(FileDiff(lines: [
                DiffComputation.Line(id: 0, kind: .added, oldNumber: nil, newNumber: 1, text: "alpha"),
                DiffComputation.Line(id: 1, kind: .added, oldNumber: nil, newNumber: 2, text: "beta"),
            ]))
        }
        #expect(s.untrackedLabel("new.txt") == "Untracked")
        #expect(s.untrackedLabel("README.md") == nil)
        await s.ensure("new.txt")
        #expect(asked == ["new.txt"], "the diff was asked for, not skipped")
        let lines = try #require(s.fileDiffs["new.txt"]?.lines)
        #expect(lines.map(\.text) == ["alpha", "beta"])
        #expect(lines.allSatisfy { $0.kind == .added })
        #expect(s.fileDiffs["new.txt"]?.unsupported == nil)
    }

    /// A new file with nothing in it says so, rather than "No textual changes".
    @Test func anEmptyUntrackedFileSaysEmptyFile() throws {
        let s = try store(Self.dirty)
        #expect(s.emptyNote("new.txt") == "Empty file")
        #expect(s.emptyNote("README.md") == "No textual changes")
    }

    /// What the pane's header sums. `+6 -3` on a worktree nobody has scrolled.
    @Test func theHeaderTotalIsWholeAtRest() throws {
        let s = try store(Self.dirty)
        #expect(s.files.reduce(0) { $0 + $1.insertions } == 6)
        #expect(s.files.reduce(0) { $0 + $1.deletions } == 3)
    }

    /// A file that is staged and then modified again is in both groups, once
    /// per diff of it, and it is ONE row — so its counts are summed rather than
    /// found, or the row would report only the half that happened to be first.
    @Test func aFileInBothGroupsIsOneRowWithBothHalves() throws {
        let s = try store(
            """
            {"branch":"feature","base_ref":"main","base_commit":"abc","head_commit":"def",
             "insertions":0,"deletions":0,"commits":[],"files":[],
             "working_tree":{
               "staged":["README.md"],"unstaged":["README.md"],
               "untracked":[],"conflicted":[],
               "changes":[
                 {"path":"README.md","status":"modified","old_path":null,
                  "insertions":1,"deletions":0,"binary":false},
                 {"path":"README.md","status":"modified","old_path":null,
                  "insertions":4,"deletions":2,"binary":false}]}}
            """)
        #expect(s.files.count == 1, "one file, however many groups name it")
        #expect(s.files.first?.insertions == 5)
        #expect(s.files.first?.deletions == 2)
    }

    /// A runner can be older than this app: the daemon wrote zeroes into these
    /// fields for as long as they existed and only fills them now. A change set
    /// without them must still decode — a number less is not the same as a pane
    /// that shows nothing.
    @Test func aRunnerThatSendsNoCountsStillDecodes() throws {
        let s = try store(
            """
            {"branch":"feature","base_ref":"main","base_commit":"abc","head_commit":"def",
             "insertions":0,"deletions":0,"commits":[],"files":[],
             "working_tree":{
               "staged":[],"unstaged":["README.md"],"untracked":[],"conflicted":[],
               "changes":[{"path":"README.md","status":"modified","old_path":null}]}}
            """)
        #expect(s.files.count == 1)
        #expect(s.files.first?.insertions == 0)
        #expect(s.files.first?.status == .modified)
    }

    /// A binary file has no counts worth showing and the row badges it, which
    /// it can only do if the flag survives the merge of the two groups.
    @Test func aBinaryFileSaysSo() throws {
        let s = try store(
            """
            {"branch":"feature","base_ref":"main","base_commit":"abc","head_commit":"def",
             "insertions":0,"deletions":0,"commits":[],"files":[],
             "working_tree":{
               "staged":[],"unstaged":["art/icon.png"],"untracked":[],"conflicted":[],
               "changes":[{"path":"art/icon.png","status":"modified","old_path":null,
                "insertions":0,"deletions":0,"binary":true}]}}
            """)
        #expect(s.files.first?.binary == true)
    }

    /// The Branch scope is untouched by any of this: its header is the change
    /// set's own `insertions`, which is committed work and a different question.
    @Test func theBranchScopeStillReadsItsOwnTotal() throws {
        let s = try store(Self.dirty)
        s.scope = .branch
        #expect(s.changeSet.insertions == 2)
        #expect(s.files.isEmpty, "the branch's file list is what the daemon sent for it")
    }
}
