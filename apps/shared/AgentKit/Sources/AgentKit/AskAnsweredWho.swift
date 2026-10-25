import Foundation

// Who answered an ask, said on its row when the runner has a team (ov-743).
//
// The runner names the person who answered and, when the pane was someone
// else's, whose it was. This words it for the reader: "You answered",
// "Bob answered", "Alice answered for Bob", "Alice answered for you".

extension AgentRow.Ask {
    /// How a row says `person` to `viewer`: "you", else the member's name (or
    /// handle, for a member with no name), else "the owner".
    private func said(_ person: Person, viewer: String) -> String {
        if person.handle == viewer { return "you" }
        if !person.name.isEmpty { return person.name }
        return person.handle.isEmpty ? "the owner" : person.handle
    }

    /// The row's title for an ask someone answered on a runner with members:
    /// "You answered", "Bob answered", "Alice answered for Bob", "Alice
    /// answered for you". Nil on a solo runner (the runner names no one), and
    /// while this device isn't told who it is, so a row never says "Bob
    /// answered" to Bob.
    public func answeredSentence(viewer: String?) -> String? {
        guard let viewer, let person = answeredPerson else { return nil }
        let who = said(person, viewer: viewer)
        var sentence = "\(who.prefix(1).uppercased())\(who.dropFirst()) answered"
        if let other = answeredFor { sentence += " for \(said(other, viewer: viewer))" }
        return sentence
    }
}
