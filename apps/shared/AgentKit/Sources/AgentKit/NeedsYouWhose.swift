import Foundation

// Who each Needs You item is for, said on every row, and the one deliberate
// step before answering someone else's (ov-737).
//
// Anyone may answer any item, so the list says whose it is, and an answer to
// another person's item asks "This is for Bob. Answer for them?" first. The
// runner decides whose an item is (`NeedsYouConcern`); this only words it.

extension NeedsYou {
    /// What a row says for an item nobody owns.
    public static let anyoneLabel = "Anyone"
    /// What a row says for the reader's own.
    public static let yoursLabel = "You"

    /// The runners whose items say whose they are: a runner with a member
    /// puts a concern on each item that has an owner, and a runner without
    /// one puts none on any. So a runner that sent no concern at all is a
    /// solo runner (or one too old to say), and its rows stay as they were
    /// rather than all reading "Anyone".
    public static func teamRunners(in items: [NeedsYouItem]) -> Set<String> {
        Set(items.filter { $0.concern != nil }.map(\.runner))
    }
}

extension NeedsYouItem {
    /// Who this item is for, as its row says it: "You" for the reader's own,
    /// a member's name or "Owner" for someone else's, "Anyone" for nobody's.
    /// Nil on a runner that has no team (`teamRunners`), where there is
    /// nothing to tell apart. Also nil for the reader's own when the list is
    /// split (`listIsSplit`): its "Yours" header already says so.
    public func whose(teamRunners: Set<String>, listIsSplit: Bool = false) -> String? {
        if let concern {
            if concern.yours { return listIsSplit ? nil : NeedsYou.yoursLabel }
            return concern.name.isEmpty ? "Owner" : concern.name
        }
        return teamRunners.contains(runner) ? NeedsYou.anyoneLabel : nil
    }

    /// The question asked before an answer goes to someone else's item, or
    /// nil when the answer needs no extra step: the reader's own and nobody's.
    public var answerForOthersPrompt: String? {
        othersWho.map { "This is for \($0). Answer for them?" }
    }

    /// Whose a sentence says this item is, when it is someone else's: a
    /// member's name, or "the owner". `othersName` is the row's label
    /// ("Owner"); this is the same person as a sentence needs them (ov-739).
    public var othersWho: String? {
        guard let concern, !concern.yours else { return nil }
        return concern.name.isEmpty ? "the owner" : concern.name
    }

    /// What a surface that can't confirm says instead of answering.
    public var openToAnswerSentence: String? {
        othersWho.map { "This is for \($0). Open Far Cooler to answer it." }
    }
}

extension NeedsYou {
    /// The ask on `terminal` when it is someone else's, or nil when it is the
    /// reader's, nobody's, or not listed.
    ///
    /// For a surface that holds an agent and not an item, as the Live
    /// Activity and the watch's agent screen do: it finds the item the runner
    /// filed for that agent (`ask` or `blocked`, the kinds an agent's answer
    /// buttons settle). Nil when the list is unknown, so a surface on a build
    /// or a runner that says nothing keeps its buttons as it always had them.
    public static func othersItem(askingOn terminal: String, in items: [NeedsYouItem]?) -> NeedsYouItem? {
        items?.first { ($0.kind == .ask || $0.kind == .blocked) && $0.terminal?.id == terminal && $0.isOthers }
    }

    /// Whose that ask is, as a sentence says it: "Bob" or "the owner".
    public static func othersWho(askingOn terminal: String, in items: [NeedsYouItem]?) -> String? {
        othersItem(askingOn: terminal, in: items)?.othersWho
    }
}
