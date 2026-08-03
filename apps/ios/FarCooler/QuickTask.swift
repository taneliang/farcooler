import Foundation

// Ported from apps/macos/Sources/FarCooler/QuickCreate.swift (`Branch`) and
// Composer.swift (`Agents`). Pure logic, no UI and no host calls, so it can be
// copied verbatim rather than re-derived: the whole point is that a phone and
// a Mac describing the same task land on the same branch name and offer the
// same agents, and the only way to guarantee that is to make the rules
// identical rather than merely similar.

/// Turning a sentence into a branch name and a short title.
///
/// Same shape as the Mac's `Branch`: one sentence is both the task's name and,
/// slugged, its branch. Keeping the two derivations next to each other (as the
/// Mac does) is what keeps them from drifting apart the next time either one
/// is tuned.
enum TaskSlug {
    /// A git-safe slug.
    ///
    /// Conservative on purpose: git accepts far more than this, but a branch
    /// name is something people type, paste into a PR title and see in a CI
    /// log, and one carrying punctuation from a sentence is a small tax paid
    /// repeatedly.
    static func slug(from text: String) -> String {
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
        return out.isEmpty ? "task" : out
    }

    /// A short human title, for the workspace's task name.
    static func title(from text: String) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count > 42 else { return trimmed }
        // Cut on a word boundary rather than mid-word.
        let cut = trimmed.prefix(42)
        if let space = cut.lastIndex(of: " ") {
            return String(cut[..<space]) + "…"
        }
        return String(cut) + "…"
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
