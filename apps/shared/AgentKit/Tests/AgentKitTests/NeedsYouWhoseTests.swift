import Foundation
import Testing

@testable import AgentKit

// Who each Needs You item is for, and the step before answering for someone
// else (ov-737).

private func item(_ id: String, runner: String = "studio", _ concern: NeedsYouConcern?) -> NeedsYouItem {
    var item = NeedsYouItem(id: id, kind: .decision, rank: 1, since: nil, question: "q", concern: concern)
    item.runner = runner
    return item
}

private let mine = NeedsYouConcern(handle: "alice", name: "Alice Chen", yours: true)
private let bob = NeedsYouConcern(handle: "bob", name: "Bartholomew Okafor-Lindqvist", yours: false)
private let owner = NeedsYouConcern(handle: "", name: "", yours: false)

@Test("A row says You, a name, Owner or Anyone on a runner with a team")
func aRowSaysWhoseItIs() {
    let items = [item("a", mine), item("b", bob), item("c", owner), item("d", nil)]
    let team = NeedsYou.teamRunners(in: items)
    #expect(items.map { $0.whose(teamRunners: team) } == ["You", "Bartholomew Okafor-Lindqvist", "Owner", "Anyone"])
}

@Test("Under a Yours header a row doesn't repeat You")
func aSplitListDoesntRepeatYou() {
    let items = [item("a", mine), item("b", bob), item("c", nil)]
    let team = NeedsYou.teamRunners(in: items)
    let split = NeedsYou.grouped(items).isSplit
    #expect(split)
    #expect(items.map { $0.whose(teamRunners: team, listIsSplit: split) } == [nil, "Bartholomew Okafor-Lindqvist", "Anyone"])
    let flat = [item("a", mine), item("c", nil)]
    #expect(flat.map { $0.whose(teamRunners: team, listIsSplit: NeedsYou.grouped(flat).isSplit) } == ["You", "Anyone"])
}

@Test("A runner with no team says nothing, even beside one that has")
func aSoloRunnerSaysNothing() {
    let items = [item("a", bob), item("b", nil), item("c", runner: "laptop", nil)]
    let team = NeedsYou.teamRunners(in: items)
    #expect(team == ["studio"])
    #expect(items.map { $0.whose(teamRunners: team) } == ["Bartholomew Okafor-Lindqvist", "Anyone", nil])
}

@Test("Only someone else's item asks before it is answered, naming them")
func onlyAnotherPersonsItemAsks() {
    #expect(item("a", mine).answerForOthersPrompt == nil)
    #expect(item("b", nil).answerForOthersPrompt == nil, "nobody's answers with no extra step")
    #expect(item("c", bob).answerForOthersPrompt == "This is for Bartholomew Okafor-Lindqvist. Answer for them?")
    #expect(item("d", owner).answerForOthersPrompt == "This is for the owner. Answer for them?")
}

/// The watch's alert names the first word, so a long name doesn't push
/// Answer below the fold at 40mm (ov-745).
///
/// Mutation: `shortAnswerForOthersPrompt` returning the full prompt. Red.
@Test("The watch's prompt uses a first name, and leaves the rest alone")
func theWatchPromptUsesAFirstName() {
    func who(_ name: String) -> NeedsYouItem {
        item("w", NeedsYouConcern(handle: "w", name: name, yours: false))
    }
    #expect(item("c", bob).shortAnswerForOthersPrompt == "This is for Bartholomew. Answer for them?")
    #expect(who("Bob").shortAnswerForOthersPrompt == "This is for Bob. Answer for them?")
    #expect(who("Mary-Jane Smith").shortAnswerForOthersPrompt == "This is for Mary-Jane. Answer for them?")
    #expect(who("Okafor-Lindqvist").shortAnswerForOthersPrompt == "This is for Okafor-Lindqvist. Answer for them?")
    #expect(item("d", owner).shortAnswerForOthersPrompt == "This is for the owner. Answer for them?")
    #expect(item("a", mine).shortAnswerForOthersPrompt == nil)
    #expect(item("b", nil).shortAnswerForOthersPrompt == nil)
    // The row and header still say the whole name.
    #expect(item("c", bob).answerForOthersPrompt == "This is for Bartholomew Okafor-Lindqvist. Answer for them?")
}

@Test("A note answered for someone says so in its byline, and only then")
func aNoteAnsweredForSomeoneSaysSo() throws {
    let json = """
        {"notes": [
          {"id": "n1", "kind": "answer", "actor": "member:alice", "at": 1, "body": "pdfkit",
           "extra": {"for": {"handle": "bob", "name": "Bob Okafor"}}},
          {"id": "n2", "kind": "answer", "actor": "user", "at": 2, "body": "pdfkit",
           "extra": {"for": {"handle": "", "name": ""}}},
          {"id": "n3", "kind": "answer", "actor": "member:alice", "at": 3, "body": "pdfkit", "extra": {}},
          {"id": "n4", "kind": "answer", "actor": "member:alice", "at": 4, "body": "pdfkit"}
        ]}
        """
    let notes = try TaskDetailModel.decode(Data(json.utf8)).notes
    let people = RunnerPeople(
        members: [.init(handle: "alice", name: "Alice Chen")], claudeAccount: "", you: "")
    #expect(notes.map { $0.byline(on: people) } == ["Alice Chen for Bob Okafor", "You for You", "Alice Chen", "Alice Chen"], "the owner's card, read by the owner")
    let bobsPhone = RunnerPeople(
        members: [.init(handle: "alice", name: "Alice Chen"), .init(handle: "bob", name: "Bob Okafor")], claudeAccount: "", you: "bob")
    #expect(notes.map { $0.byline(on: bobsPhone) } == ["Alice Chen for You", "Owner for Owner", "Alice Chen", "Alice Chen"], "Bob's own card says You")
    #expect(notes[0].byline(on: nil) == "alice for Bob Okafor", "who the reader is isn't known, so no You")
}
