import Testing
@testable import AgentKit

@Test func aSlashAtTheStartOpensTheCommandPicker() {
    let token = activeToken(in: "/mod", cursor: 4)
    guard case let .slash(prefix, _) = token else {
        Issue.record("expected a slash token, got \(token)")
        return
    }
    #expect(prefix == "mod")
}

@Test func aSlashMidSentenceIsJustASlash() {
    // Otherwise typing a file path or "and/or" pops a command menu over the
    // text the user is writing.
    let token = activeToken(in: "read src/main.rs", cursor: 16)
    guard case .none = token else {
        Issue.record("a slash inside a word must not open the picker")
        return
    }
}

@Test func anAtSignOpensTheFilePickerAnywhere() {
    // Unlike a slash command, a file mention is legitimate mid-sentence.
    let token = activeToken(in: "look at @src/ma", cursor: 15)
    guard case let .mention(prefix, _) = token else {
        Issue.record("expected a mention token, got \(token)")
        return
    }
    #expect(prefix == "src/ma")
}

@Test func anEmailAddressDoesNotOpenTheFilePicker() {
    // An @ preceded by a word character is part of that word.
    let token = activeToken(in: "mail me@example.com", cursor: 19)
    guard case .none = token else {
        Issue.record("an @ inside a word must not open the picker")
        return
    }
}

@Test func aSpaceClosesTheToken() {
    let token = activeToken(in: "@src/main.rs and then", cursor: 21)
    guard case .none = token else {
        Issue.record("the token ends at whitespace")
        return
    }
}
