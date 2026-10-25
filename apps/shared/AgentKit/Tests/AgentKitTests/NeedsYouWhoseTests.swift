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
