import Foundation

// Where a worktree's review was left, kept per runner and worktree and put
// back the next time its changes open (ov-233): the Mac returns where you
// were, so it applies the position quietly where the phones offer it on a
// card.
//
// The record is the phones' `ReviewPosition`, field for field, so the two can
// become one type without migrating a byte. Only its key differs: one Mac
// talks to several runners, whose worktree ids can collide.

/// What was open: the comparison, and the file at the top of the screen.
struct ReviewPlace: Codable, Equatable {
    /// `branch`, `local`, or a full sha, as `DiffScope`'s wire says it.
    var scope: String
    /// The file at the top of the screen, or the one jumped to.
    var file: String?
    /// Written by the phones, when no file was open; unused here.
    var topFile: String?
    var savedAt: Double

    /// The scope as it's kept: the commit's sha for a commit.
    static func scope(_ scope: DiffScope, commit: String?) -> String {
        switch scope {
        case .branch: "branch"
        case .local: "local"
        case .commit: commit ?? "branch"
        }
    }
}

enum ReviewMemory {
    static func key(host: String, worktree: String) -> String { "changes.position.\(host)/\(worktree)" }

    static func read(host: String, worktree: String, in defaults: UserDefaults = .standard) -> ReviewPlace? {
        defaults.data(forKey: key(host: host, worktree: worktree)).flatMap { try? JSONDecoder().decode(ReviewPlace.self, from: $0) }
    }

    static func write(_ place: ReviewPlace, host: String, worktree: String, in defaults: UserDefaults = .standard) {
        guard let data = try? JSONEncoder().encode(place) else { return }
        defaults.set(data, forKey: key(host: host, worktree: worktree))
    }

    /// Where a kept place lands in what the runner lists now.
    struct Landing: Equatable {
        var scope: DiffScope
        /// The commit, for the commit scope.
        var commit: String?
        /// The file to bring to the top, for the store to check against what
        /// that scope lists.
        var file: String?
    }

    /// A commit the branch no longer lists (an amend or a rebase) is the
    /// whole branch again, quietly; the file is the store's to confirm.
    static func landing(_ place: ReviewPlace, in set: ChangeSet) -> Landing {
        let file = place.file
        switch place.scope {
        case "branch", "":
            return Landing(scope: .branch, commit: nil, file: file)
        case "local", "staged", "unstaged":
            return Landing(scope: .local, commit: nil, file: file)
        default:
            guard set.commits.contains(where: { $0.sha == place.scope }) else {
                return Landing(scope: .branch, commit: nil, file: nil)
            }
            return Landing(scope: .commit, commit: place.scope, file: file)
        }
    }
}

extension ChangesStore {
    /// Write down where the review is, when it's changed and it's the
    /// reader's own (`positionApplied`).
    func remember() {
        guard positionApplied else { return }
        let place = ReviewPlace(
            scope: ReviewPlace.scope(scope, commit: selectedCommit), file: selectedFile, topFile: nil, savedAt: 0)
        guard place != lastKept else { return }
        lastKept = place
        var stamped = place
        stamped.savedAt = Date().timeIntervalSince1970
        ReviewMemory.write(stamped, host: worktree.host ?? "", worktree: worktree.id, in: defaults)
    }

    /// The first time the change set is read: put the kept comparison and file
    /// back, quietly. A commit that's gone is the whole branch again; a file
    /// that's gone leaves the top of it. Nothing is said either way.
    func applyKeptPosition() async {
        positionApplied = true
        guard let kept = ReviewMemory.read(host: worktree.host ?? "", worktree: worktree.id, in: defaults),
            selectedFile == nil, scope == .branch
        else { return }
        let landing = ReviewMemory.landing(kept, in: changeSet)
        if let sha = landing.commit {
            await select(commit: sha)
        } else if landing.scope != scope {
            scope = landing.scope
        }
        if let file = landing.file, files.contains(where: { $0.path == file }) {
            selectedFile = file
            restoreTarget = file
        }
        lastKept = nil
        remember()
    }
}
