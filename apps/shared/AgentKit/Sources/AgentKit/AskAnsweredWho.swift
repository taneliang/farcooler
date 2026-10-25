import Foundation

// Who answered an ask, said on its row when the runner has a team (ov-743).
//
// The runner names the person who answered and, when the pane was someone
// else's, whose it was. This words it for the reader: "You answered",
// "Bob answered", "Alice answered for Bob", "Alice answered for you".

extension AgentRow.Ask {
    /// How a row says `person` to `viewer`, and whether those are the row's
    /// own words ("you", "the owner"), which a sentence may capitalize. A
    /// member's name is used exactly as the runner sent it, never re-cased
    /// ("van der Berg" stays so); a member with no name is said by handle.
    private func said(_ person: Person, viewer: String) -> (words: String, own: Bool) {
        if person.handle == viewer { return ("you", true) }
        if !person.name.isEmpty { return (person.name, false) }
        return person.handle.isEmpty ? ("the owner", true) : (person.handle, false)
    }

    /// The row's title for an ask someone answered on a runner with members:
    /// "You answered", "Bob answered", "Alice answered for Bob", "Alice
    /// answered for you". Nil on a solo runner (the runner names no one), and
    /// while this device isn't told who it is, so a row never says "Bob
    /// answered" to Bob. An `answeredFor` that is the answerer adds nothing
    /// and is dropped.
    public func answeredSentence(viewer: String?) -> String? {
        guard let viewer, let person = answeredPerson else { return nil }
        let who = said(person, viewer: viewer)
        var sentence = (who.own ? who.words.prefix(1).uppercased() + who.words.dropFirst() : who.words) + " answered"
        if let other = answeredFor, other.handle != person.handle { sentence += " for \(said(other, viewer: viewer).words)" }
        return sentence
    }
}
