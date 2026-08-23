import Foundation

// Which changed files a tool wrote, and where they go in a review.
//
// Written for the phone in `apps/ios/FarCooler/Changes.swift` and moved here
// whole when the Mac's diff pane needed the same answer. The reasoning in the
// doc comments below is the phone's and is kept as it was argued; what changed
// in the move is recorded where it changed.
//
// Pure Foundation, and deliberately a rule over PATHS rather than over either
// app's `ChangedFile`. The two apps declare that type separately and the
// declarations do not agree — the phone carries a `directory` the Mac does not,
// the Mac carries a custom `init(from:)` the phone does not — so a shared type
// here would be a third declaration for the other two to drift from. A path is
// what both of them do have, spelled the same way, straight off the same wire.

/// Whether a tool wrote a file rather than a person or an agent, and the
/// reading order that follows from it.
public enum GeneratedFile {
    /// Whether a tool wrote this file rather than a person or an agent.
    ///
    /// It exists because of what it does to the two numbers at the top of the
    /// screen. A branch that touched eleven source files and regenerated
    /// `Cargo.lock` reads as four thousand lines changed, and the reader — who
    /// has ninety seconds — has no way to tell that from a branch that really
    /// did rewrite four thousand lines. Counted apart, the same branch reads as
    /// `+300 −120`, and a quieter line beside it says a lockfile moved too.
    ///
    /// **This rule belongs on the host, beside `crates/core/src/feed.rs`.** It
    /// is a fact about a repository — what its build regenerates, what its
    /// `.gitattributes` marks `linguist-generated`, what its own conventions
    /// call vendored — and the host is the only place that can read any of
    /// that. Deciding it in a client means a client that is wrong about a
    /// repository it has never seen. It is here only because putting it there
    /// is a protocol field plus a daemon rule plus both clients, which is a
    /// larger change than the screens that need it; when that lands, this
    /// becomes a fallback for older runners and nothing else. Hoisting it into
    /// this module is what stops the interim from being TWO lists: both clients
    /// are already wrong in exactly the same way, so the day the host answers,
    /// there is one caller-side rule to demote rather than two to reconcile.
    ///
    /// Deliberately conservative in the meantime. Everything below is a name a
    /// tool writes and nobody edits by hand, so a false positive costs a
    /// lockfile sorted to the end of a list the reader can still scroll to; a
    /// rule like "anything under `vendor/`" would start demoting files people
    /// wrote. The phone's version of this sentence said the cost was "a fold
    /// the reader can open with one tap" — true there and only there. The Mac
    /// folds nothing: it orders, and counts apart.
    ///
    /// Three things the host version would get right and this cannot, all of
    /// them pinned by `GeneratedFilesTests` so the blind spots are visible
    /// rather than assumed away:
    ///
    /// - **`.gitattributes`.** A repository that marks its own generated files
    ///   is telling us the answer, and nothing here reads it.
    /// - **Directories.** Only the last path component is examined, so
    ///   `src/generated/api.ts` is a file this says a person wrote.
    /// - **Case.** Matching is exact, because git paths are exact bytes and a
    ///   case-folded compare would be this rule guessing about a filesystem it
    ///   is not running on. A repository holding `cargo.lock` gets nothing.
    public static func isGenerated(_ path: String) -> Bool {
        let name = (path as NSString).lastPathComponent
        if generatedNames.contains(name) { return true }
        // Suffixes rather than whole names, for the families whose stem varies:
        // `pnpm-lock.yaml` is matched above, `schema.generated.ts` here.
        return generatedSuffixes.contains { name.hasSuffix($0) }
    }

    /// The files in reading order: what somebody wrote, then what a tool wrote.
    ///
    /// Generated last rather than in the daemon's order, because the reason
    /// they are marked at all is that they are not what the review is about —
    /// and Next dropping somebody into the middle of a regenerated lockfile,
    /// eleven files before the end, is the tap that ends a review.
    ///
    /// A stable partition, which is the part worth stating: within each of the
    /// two groups the daemon's own order survives untouched, and a comparison
    /// with no generated file in it comes back exactly as it went in. So this
    /// is inert on every branch that has not regenerated anything, which is
    /// most of them.
    ///
    /// Generic over the caller's row type and taking a path accessor, for the
    /// reason the file header gives: there is no shared `ChangedFile` to be
    /// generic over. One pass rather than two `filter`s so `isGenerated` runs
    /// once per file — both clients call this on every redraw of a list that
    /// can hold hundreds of rows.
    public static func reviewOrder<File>(_ files: [File], path: (File) -> String) -> [File] {
        var written: [File] = []
        var generated: [File] = []
        written.reserveCapacity(files.count)
        for file in files {
            if isGenerated(path(file)) {
                generated.append(file)
            } else {
                written.append(file)
            }
        }
        return written + generated
    }

    /// Whole names, matched against the last path component at any depth: a
    /// workspace holding four crates has four `Cargo.lock`s and they are all
    /// lockfiles.
    private static let generatedNames: Set<String> = [
        "Cargo.lock", "package-lock.json", "pnpm-lock.yaml", "yarn.lock", "bun.lockb",
        "Gemfile.lock", "Podfile.lock", "poetry.lock", "uv.lock", "composer.lock",
        "go.sum", "Package.resolved", "flake.lock", "gradle.lockfile", "mix.lock",
        // An .xcodeproj is generated state in this repository specifically —
        // `apps/ios/generate-project.py` writes it — and it is the file that
        // most often makes an iOS branch look twice its size.
        "project.pbxproj",
    ]

    private static let generatedSuffixes: [String] = [
        ".generated.swift", ".generated.ts", ".generated.go", ".g.dart", ".pb.go",
        ".pb.rs", "_pb2.py",
    ]
}
