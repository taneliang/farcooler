import Foundation

// Ported from apps/macos/Sources/FarCooler/QuickCreate.swift (`Branch`) and
// Composer.swift (`Agents`). Pure logic, no UI and no host calls, so it can be
// copied verbatim rather than re-derived: the whole point is that a phone and
// a Mac describing the same task land on the same branch name and offer the
// same agents, and the only way to guarantee that is to make the rules
// identical rather than merely similar.

/// Turning a sentence into a branch name and a worktree name.
///
/// Same shape as the Mac's `Branch`: one sentence is both the worktree's name
/// and, slugged, its branch. Keeping the two derivations next to each other (as
/// the Mac does) is what keeps them from drifting apart the next time either one
/// is tuned.
enum TaskSlug {
    /// A git-safe slug, behind whatever the machine says branches start with.
    ///
    /// Conservative on purpose: git accepts far more than this, but a branch
    /// name is something people type, paste into a PR title and see in a CI
    /// log, and one carrying punctuation from a sentence is a small tax paid
    /// repeatedly.
    ///
    /// The prefix is applied HERE rather than by the daemon, because the
    /// composer shows you the branch it is about to create — a prefix added on
    /// the far side would make that preview a lie. The daemon still validates
    /// the finished name.
    ///
    /// The 48-character budget is spent on the slug, not on the result: a long
    /// prefix must not eat the part that says what the task was.
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
        return prefix + (out.isEmpty ? "task" : out)
    }

    /// The worktree's name, which is the directory it is created in.
    ///
    /// The slug again, minus the branch prefix, rather than the trimmed
    /// sentence this used to hand over. A name is a path component now: the
    /// machine caps it at 60 characters and refuses one with no letters or
    /// numbers left after sanitizing, and a sentence can be either — "Ship
    /// it!!!" is the second. Slugging is the one derivation that can be
    /// neither, and it costs nothing, because the directory is read back as
    /// prose in the fleet list anyway.
    ///
    /// Through `sanitize` and not merely `slug`, because the two disagree about
    /// what a letter is: Swift says yes to `é` and to 写, and the machine — which
    /// keeps ASCII and dashes everything else — says no to both. A description
    /// written in Chinese would otherwise slug to something this thinks is a
    /// name and the daemon refuses outright, and `createWorkspace` swallows that
    /// refusal, so Quick Task would close having created nothing and said
    /// nothing.
    static func name(from text: String) -> String {
        let name = sanitize(slug(from: text))
        return name.isEmpty ? "task" : name
    }

    /// A worktree's name read back the way the fleet list reads it.
    ///
    /// Same rule as the machine's, so copy that says "open X" names the row
    /// someone is about to go looking for rather than its directory.
    static func displayName(of name: String) -> String {
        name.replacingOccurrences(of: "-", with: " ")
            .replacingOccurrences(of: "_", with: " ")
    }

    /// The directory a typed name lands in.
    ///
    /// Duplicated from the machine for the same reason the branch prefix is
    /// applied here rather than there: the form shows the folder it is about
    /// to create, and a preview computed on the far side would be a preview
    /// that can lie. The machine still has the last word, and refuses a name
    /// this leaves empty.
    static func sanitize(_ text: String) -> String {
        var out = ""
        var lastWasDash = true  // leading dashes are dropped
        for character in text {
            if character == "_" || (character.isASCII && (character.isLetter || character.isNumber))
            {
                out.append(character)
                lastWasDash = false
            } else if !lastWasDash {
                out.append("-")
                lastWasDash = true
            }
        }
        while out.hasSuffix("-") { out.removeLast() }
        return out
    }
}

/// The agents Far Cooler knows how to launch, and the models worth offering.
///
/// A short curated list plus "Default", not an exhaustive one — see
/// Composer.swift on the Mac for why. Named `QuickAgents` rather than `Agents`
/// only so it cannot be mistaken for a type shared with the terminal work
/// landing in this app at the same time; the list itself is copied verbatim
/// from the Mac so the two pickers never disagree about what "codex" means.
enum QuickAgents {
    struct Agent: Identifiable, Hashable {
        let id: String
        let name: String
        let models: [String]
    }

    static let all: [Agent] = [
        Agent(id: "claude", name: "Claude Code", models: ["opus", "sonnet", "haiku"]),
        Agent(id: "codex", name: "Codex", models: ["gpt-5.6-sol", "gpt-5.6-sol-high"]),
        Agent(id: "cursor", name: "Cursor", models: ["auto", "sonnet-4.5", "gpt-5"]),
    ]

    static func agent(_ id: String) -> Agent {
        all.first { $0.id == id } ?? all[0]
    }

    /// The preset string the host expects: `agent` or `agent:model`.
    static func preset(agent: String, model: String) -> String {
        model.isEmpty ? agent : "\(agent):\(model)"
    }
}
