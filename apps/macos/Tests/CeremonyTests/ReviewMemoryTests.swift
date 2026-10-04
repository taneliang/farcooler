import Foundation
import Testing

@testable import Far_Cooler

/// A worktree's review comes back where it was on the Mac (ov-233): the
/// comparison and the file at the top, applied quietly, and written down only
/// once the reader's own position is the store's.
@MainActor
struct ReviewMemoryTests {
    private static func file(_ path: String) -> ChangedFile {
        ChangedFile(path: path, status: .modified, oldPath: nil, insertions: 1, deletions: 0, binary: false)
    }

    private static func commit(_ sha: String) -> ChangeCommit {
        ChangeCommit(sha: sha, subject: "Commit \(sha)", body: nil, author: "Ada", timestamp: 0)
    }

    private static let set = ChangeSet(
        branch: "feature", baseRef: "main", baseSource: nil, baseCommit: "", headCommit: "", insertions: 0, deletions: 0,
        commits: [commit("aaaa1111"), commit("bbbb2222")], files: [file("a.swift"), file("b.swift")], workingTree: nil)

    private static func defaults(_ name: String = #function) -> UserDefaults {
        let suite = "farcooler.test.review.\(name)"
        let defaults = UserDefaults(suiteName: suite)!
        defaults.removePersistentDomain(forName: suite)
        return defaults
    }

    private static let worktree = Worktree(
        id: "lane", short: "lane", task: "lane", branch: "feat/lane", repository: "overnight", host: "studio",
        path: "/tmp/lane", state: "active", terminals: [])

    private static func store(_ defaults: UserDefaults) -> ChangesStore {
        let store = ChangesStore(client: DaemonClient(target: "studio"), worktree: worktree, defaults: defaults)
        store.changeSet = set
        return store
    }

    private static func keep(_ scope: String, file: String?, in defaults: UserDefaults) {
        ReviewMemory.write(
            ReviewPlace(scope: scope, file: file, topFile: nil, savedAt: 1), host: "studio", worktree: "lane", in: defaults)
    }

    @Test("A position is kept per runner and worktree, and reads back")
    func keys() {
        let defaults = Self.defaults()
        Self.keep("branch", file: "a.swift", in: defaults)
        #expect(ReviewMemory.read(host: "studio", worktree: "lane", in: defaults)?.file == "a.swift")
        #expect(ReviewMemory.read(host: "", worktree: "lane", in: defaults) == nil, "another runner's worktree of the same id")
        #expect(ReviewMemory.key(host: "studio", worktree: "lane") == "changes.position.studio/lane")
    }

    @Test("A kept file is brought to the top, silently, and the comparison with it")
    func appliesTheFile() async {
        let defaults = Self.defaults()
        Self.keep("local", file: "b.swift", in: defaults)
        let store = Self.store(defaults)
        await store.applyKeptPosition()
        #expect(store.scope == .local)
        #expect(store.selectedFile == nil, "local lists only what's dirty, and this worktree has none: the file isn't there")
        Self.keep("branch", file: "b.swift", in: defaults)
        let branch = Self.store(defaults)
        await branch.applyKeptPosition()
        #expect(branch.scope == .branch)
        #expect(branch.selectedFile == "b.swift")
        #expect(branch.restoreTarget == "b.swift")
    }

    @Test("A file that's gone leaves the top of the branch, and a commit that's gone is the branch again")
    func goneIsQuiet() async {
        let defaults = Self.defaults()
        Self.keep("branch", file: "deleted.swift", in: defaults)
        let store = Self.store(defaults)
        await store.applyKeptPosition()
        #expect(store.scope == .branch && store.selectedFile == nil && store.restoreTarget == nil)
        #expect(
            ReviewMemory.landing(ReviewPlace(scope: "cccc3333", file: "a.swift", topFile: nil, savedAt: 1), in: Self.set)
                == ReviewMemory.Landing(scope: .branch, commit: nil, file: nil))
        #expect(
            ReviewMemory.landing(ReviewPlace(scope: "bbbb2222", file: "a.swift", topFile: nil, savedAt: 1), in: Self.set)
                == ReviewMemory.Landing(scope: .commit, commit: "bbbb2222", file: "a.swift"))
    }

    @Test("Nothing is written until the kept position has been put back, then each change is")
    func writesAfterwards() async {
        let defaults = Self.defaults()
        Self.keep("branch", file: "a.swift", in: defaults)
        let store = Self.store(defaults)
        // The reader hasn't been given back their place: the pane scrolling
        // to the first file mustn't overwrite it.
        store.selectedFile = "b.swift"
        #expect(ReviewMemory.read(host: "studio", worktree: "lane", in: defaults)?.file == "a.swift")
        store.selectedFile = nil
        await store.applyKeptPosition()
        #expect(store.selectedFile == "a.swift")
        store.selectedFile = "b.swift"
        #expect(ReviewMemory.read(host: "studio", worktree: "lane", in: defaults)?.file == "b.swift")
    }

    @Test("The record is the phones' own, byte for field")
    func samePlaceAsThePhones() throws {
        let place = ReviewPlace(scope: "branch", file: "a.swift", topFile: "b.swift", savedAt: 2)
        let object = try #require(
            JSONSerialization.jsonObject(with: try JSONEncoder().encode(place)) as? [String: Any])
        #expect(Set(object.keys) == ["scope", "file", "topFile", "savedAt"])
    }
}
