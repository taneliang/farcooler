import Testing

@testable import AgentKit

@Test func theDraftNamesTheTaskAndEndsWhereTheyKeepTyping() {
    #expect(
        AskAboutTask.draft(key: "ov-241", title: "Phones: Ask the Orchestrator")
            == "About ov-241 (“Phones: Ask the Orchestrator”): ")
}

@Test func aTaskTitleCannotDoAnythingWhereItLands() {
    // A CSI that would close a bracketed paste, an OSC, ^C, bidi and a newline.
    let hostile = "fix\u{1B}[201~ it\u{1B}]0;x\u{07} now\u{03}\u{202E}!\nnext"
    let line = AskAboutTask.oneLine(hostile)
    #expect(line == "fix it now! next", "\(line)")
    #expect(!AskAboutTask.draft(key: "ov-1", title: hostile).contains("\u{1B}"))
}

@MainActor
@Test func aChatOrchestratorGetsTheDraftInItsComposerAndNothingIsPasted() async {
    var offered: [String] = []
    var pasted = 0
    var copied: [String] = []
    let delivery = await AskAboutTask.deliver(
        key: "ov-9", title: "Move it", isAgentPane: true,
        offer: { offered.append($0) }, paste: { _ in pasted += 1; return true },
        copy: { copied.append($0) })
    #expect(delivery == .composer)
    #expect(offered == ["About ov-9 (“Move it”): "])
    #expect(pasted == 0 && copied.isEmpty)
}

@MainActor
@Test func aTerminalOrchestratorIsPastedToAndOtherwiseTheReferenceIsCopied() async {
    var offered = 0
    var asked: [String] = []
    var copied: [String] = []
    let pasted = await AskAboutTask.deliver(
        key: "ov-9", title: "Move it", isAgentPane: false,
        offer: { _ in offered += 1 }, paste: { asked.append($0); return true },
        copy: { copied.append($0) })
    #expect(pasted == .pasted)
    #expect(asked == ["About ov-9 (“Move it”): "] && copied.isEmpty && offered == 0)

    // The runner refused: nothing is typed, and the reference is on the
    // clipboard without the trailing space.
    let refused = await AskAboutTask.deliver(
        key: "ov-9", title: "Move it", isAgentPane: false,
        offer: { _ in offered += 1 }, paste: { _ in false },
        copy: { copied.append($0) })
    #expect(refused == .copied)
    #expect(copied == ["About ov-9 (“Move it”):"])
    #expect(AskAboutTask.copiedNotice(key: "ov-9") == "Copied a reference to ov-9. Paste it into the orchestrator.")
}

@MainActor
@Test func aWaitingOfferIsTakenOnceAndJoinedNotReplaced() {
    let offers = ComposerOffers()
    offers.offer("first", to: "t1")
    offers.offer("second", to: "t1")
    offers.offer("", to: "t2")
    #expect(offers.waiting == ["t1": "first\n\nsecond"])
    #expect(offers.take(for: "t1") == "first\n\nsecond")
    #expect(offers.take(for: "t1") == nil)
    // And behind what is already being typed, never over it.
    #expect(ComposerOffers.joined(field: "half a thought ", offered: "About ov-1 (“x”): ")
        == "half a thought\n\nAbout ov-1 (“x”): ")
    #expect(ComposerOffers.joined(field: "", offered: "About ov-1 (“x”): ") == "About ov-1 (“x”): ")
}
