package com.farcooler.model

// Which changed files a tool wrote, and where they go in a review.
//
// A THIRD copy of `apps/shared/AgentKit/Sources/AgentKit/GeneratedFiles.swift`,
// which is itself the phone's original rule hoisted out of
// `apps/ios/FarCooler/Changes.swift` in `92058f4` so the Mac's diff pane and
// iOS's fold could not disagree about what a lockfile is. Android cannot import
// AgentKit — there is no way to link Swift into an APK — so the list is written
// out again here, and the doc comments below are that module's, kept as they
// were argued rather than re-reasoned.
//
// **What stops the two lists drifting is a test that reads the Swift.**
// `GeneratedFilesTest` parses the two literals out of `GeneratedFiles.swift` in
// this same checkout and asserts they are set-equal to the two below. A name
// added on either side and not the other fails on the next `testInstrumentedUnitTest`,
// which is the only mechanism available: a comment asking the next person to
// remember is what produced the drift `92058f4` was written to close. Every
// case in `GeneratedFilesTests.swift` is ported beside it, so the three pinned
// limitations are pinned here too.
//
// Pure Kotlin, and deliberately a rule over PATHS rather than over
// [ChangedFile], for AgentKit's own reason: each app declares its own row type
// and they do not agree, while a path is what all three have, spelled the same
// way, straight off the same wire.

/**
 * Whether a tool wrote a file rather than a person or an agent, and the reading
 * order that follows from it.
 */
object GeneratedFile {
    /**
     * Whether a tool wrote this file rather than a person or an agent.
     *
     * It exists because of what it does to the two numbers at the top of the
     * screen. A branch that touched eleven source files and regenerated
     * `Cargo.lock` reads as four thousand lines changed, and the reader — who
     * has ninety seconds — has no way to tell that from a branch that really
     * did rewrite four thousand lines. Counted apart, the same branch reads as
     * `+300 −120`, and a quieter line beside it says a lockfile moved too.
     *
     * **This rule belongs on the host, beside `crates/core/src/feed.rs`.** It
     * is a fact about a repository — what its build regenerates, what its
     * `.gitattributes` marks `linguist-generated`, what its own conventions
     * call vendored — and the host is the only place that can read any of
     * that. Deciding it in a client means a client that is wrong about a
     * repository it has never seen. It is here only because putting it there
     * is a protocol field plus a daemon rule plus every client, which is a
     * larger change than the screens that need it; when that lands, all three
     * copies become a fallback for older runners and nothing else. That
     * argument is settled — `92058f4` records it — and this file does not
     * reopen it.
     *
     * Deliberately conservative in the meantime. Everything below is a name a
     * tool writes and nobody edits by hand, so a false positive costs a
     * lockfile sorted to the end of a list the reader can still scroll to; a
     * rule like "anything under `vendor/`" would start demoting files people
     * wrote. The phone's original version of this sentence said the cost was
     * "a fold the reader can open with one tap" — true of that screen and of
     * no other, which is why AgentKit records it as the one sentence that did
     * not survive the hoist. It is true again here: the Changes tab folds.
     *
     * Three things the host version would get right and this cannot, all of
     * them pinned by `GeneratedFilesTest` so the blind spots are visible
     * rather than assumed away:
     *
     * - **`.gitattributes`.** A repository that marks its own generated files
     *   is telling us the answer, and nothing here reads it.
     * - **Directories.** Only the last path component is examined, so
     *   `src/generated/api.ts` is a file this says a person wrote.
     * - **Case.** Matching is exact, because git paths are exact bytes and a
     *   case-folded compare would be this rule guessing about a filesystem it
     *   is not running on. A repository holding `cargo.lock` gets nothing.
     */
    fun isGenerated(path: String): Boolean {
        val name = path.substringAfterLast('/')
        if (name in GENERATED_NAMES) return true
        // Suffixes rather than whole names, for the families whose stem varies:
        // `pnpm-lock.yaml` is matched above, `schema.generated.ts` here.
        return GENERATED_SUFFIXES.any { name.endsWith(it) }
    }

    /**
     * The files in reading order: what somebody wrote, then what a tool wrote.
     *
     * Generated last rather than in the daemon's order, because the reason they
     * are marked at all is that they are not what the review is about — and
     * Next dropping somebody into the middle of a regenerated lockfile, eleven
     * files before the end, is the tap that ends a review.
     *
     * A stable partition, which is the part worth stating: within each of the
     * two groups the daemon's own order survives untouched, and a comparison
     * with no generated file in it comes back exactly as it went in. So this is
     * inert on every branch that has not regenerated anything, which is most of
     * them.
     *
     * Generic over the caller's row type and taking a path accessor, for the
     * reason the file header gives: there is no shared changed-file type to be
     * generic over. One pass rather than two filters so [isGenerated] runs once
     * per file — this is called on every recomposition of a list that can hold
     * hundreds of rows.
     */
    fun <T> reviewOrder(files: List<T>, path: (T) -> String): List<T> {
        val written = ArrayList<T>(files.size)
        val generated = ArrayList<T>()
        for (file in files) {
            if (isGenerated(path(file))) generated.add(file) else written.add(file)
        }
        return written + generated
    }

    /**
     * Whole names, matched against the last path component at any depth: a
     * workspace holding four crates has four `Cargo.lock`s and they are all
     * lockfiles.
     *
     * Internal rather than private so `GeneratedFilesTest` can compare it with
     * the Swift literal it parses. Nothing else reads it.
     */
    internal val GENERATED_NAMES: Set<String> = setOf(
        "Cargo.lock", "package-lock.json", "pnpm-lock.yaml", "yarn.lock", "bun.lockb",
        "Gemfile.lock", "Podfile.lock", "poetry.lock", "uv.lock", "composer.lock",
        "go.sum", "Package.resolved", "flake.lock", "gradle.lockfile", "mix.lock",
        // An .xcodeproj is generated state in this repository specifically —
        // `apps/ios/generate-project.py` writes it — and it is the file that
        // most often makes an iOS branch look twice its size.
        "project.pbxproj",
    )

    internal val GENERATED_SUFFIXES: List<String> = listOf(
        ".generated.swift", ".generated.ts", ".generated.go", ".g.dart", ".pb.go",
        ".pb.rs", "_pb2.py",
    )
}
