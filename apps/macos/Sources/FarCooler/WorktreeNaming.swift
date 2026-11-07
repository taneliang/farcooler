import Foundation

/// Turning a name into a worktree directory.
///
/// Beside `Branch` because they are the two halves of the same question, asked
/// of the same typed words. The rule below is the daemon's, mirrored: a
/// worktree has no stored name any more, so the directory it is created in
/// IS its name, and a client offering to create one has to show
/// which directory that will be. A preview computed by a rule merely close to
/// the daemon's is a preview of a path that never gets created — the same trap
/// `Branch.slug` avoids by applying the prefix here rather than on the far side.
enum WorktreeName {
    /// What a directory may contain, spelled out rather than asked of
    /// `isLetter`, which answers yes for `é` where the daemon answers dash.
    private static let safe = Set(
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_".unicodeScalars)

    /// One path component: anything outside `[A-Za-z0-9_-]` becomes a dash,
    /// runs of dashes collapse, and the ends are trimmed.
    ///
    /// Case survives, unlike a branch slug. The directory is read back as the
    /// worktree's name, so `Rate Limiting` has to come out spelled the way it
    /// went in.
    static func slug(_ s: String) -> String {
        s.unicodeScalars
            .map { safe.contains($0) ? Character($0) : "-" }
            .reduce(into: "") { acc, c in
                if c == "-" && acc.hasSuffix("-") { return }
                acc.append(c)
            }
            .trimmingCharacters(in: CharacterSet(charactersIn: "-"))
    }

    /// Where a worktree lands, from `worktrees` down. The daemon owns
    /// everything above that and never tells a client what it is, but the tail
    /// is the part being named.
    static func path(repository: String, name: String) -> String {
        "worktrees/\(slug(repository))/\(slug(name))"
    }

    /// Whether the daemon will take this as a name: at most sixty scalars, and
    /// something left over once slugged. Checked here so a client cannot spend
    /// a round trip being told what it already knew — a name of pure
    /// punctuation is short enough and still has no directory to be.
    static func isValid(_ s: String) -> Bool {
        s.unicodeScalars.count <= 60 && !slug(s).isEmpty
    }

    /// What a typed name will read as once it has been through the filesystem
    /// and back. Every client shows the directory, not what was typed, so a
    /// client saying "started X" about a name it has not round-tripped is
    /// naming something the sidebar does not call by that name.
    static func display(_ name: String) -> String {
        slug(name)
            .replacingOccurrences(of: "-", with: " ")
            .replacingOccurrences(of: "_", with: " ")
    }
}

/// Turning a sentence into a branch name.
enum Branch {
    /// A git-safe slug, behind whatever the runner says branches start with.
    ///
    /// Conservative on purpose: git accepts far more than this, but a branch
    /// name is something people type, paste into a PR title and see in a CI
    /// log, and one carrying punctuation from a sentence is a small tax paid
    /// repeatedly.
    ///
    /// The prefix is applied HERE, on the client, rather than by the daemon —
    /// because the composer shows you the branch it is about to create, and a
    /// prefix added on the far side would make that preview a lie. The daemon
    /// still validates the finished name, which is the check that protects git.
    ///
    /// The 48-character budget is spent on the slug, not on the result: a long
    /// prefix must not be able to eat the part that says what the task was.
    static func slug(from text: String, prefix: String = "") -> String {
        let lowered = text.lowercased()
        var out = ""
        var lastWasDash = true  // leading dashes are dropped

        for character in lowered {
            if character.isLetter || character.isNumber {
                out.append(character)
                lastWasDash = false
            } else if !lastWasDash {
                out.append("-")
                lastWasDash = true
            }
            // Long enough to stay readable, short enough for a terminal title.
            if out.count >= 48 { break }
        }
        while out.hasSuffix("-") { out.removeLast() }
        return prefix + (out.isEmpty ? "worktree" : out)
    }
}
