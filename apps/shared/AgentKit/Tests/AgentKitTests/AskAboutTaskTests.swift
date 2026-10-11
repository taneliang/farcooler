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
@Test func aTerminalOrchestratorIsPastedToAndOtherwiseTheReferenceIsCopied() async {
    var asked: [String] = []
    var copied: [String] = []
    let pasted = await AskAboutTask.deliver(
        key: "ov-9", title: "Move it",
        paste: { asked.append($0); return .pasted },
        copy: { copied.append($0) })
    #expect(pasted == .pasted)
    #expect(asked == ["About ov-9 (“Move it”): "] && copied.isEmpty)

    // The runner refused: nothing is typed, and the reference is on the
    // clipboard without the trailing space.
    let refused = await AskAboutTask.deliver(
        key: "ov-9", title: "Move it",
        paste: { _ in .declined },
        copy: { copied.append($0) })
    #expect(refused == .copied)
    #expect(copied == ["About ov-9 (“Move it”):"])
    #expect(AskAboutTask.copiedNotice(key: "ov-9") == "Copied a reference to ov-9. Paste it into the orchestrator.")
}

@MainActor
@Test func aPasteThatMayHaveLandedIsNotAlsoCopied() async {
    var copied: [String] = []
    let delivery = await AskAboutTask.deliver(
        key: "ov-9", title: "Move it", paste: { _ in .unknown }, copy: { copied.append($0) })
    #expect(delivery == .maybePasted)
    #expect(copied.isEmpty, "copied and pasted both")
    #expect(AskAboutTask.DraftResult.failed(word: RunnerRefusal.timedOutWord, disconnected: false, notSent: false) == .unknown)
    #expect(AskAboutTask.DraftResult.failed(word: nil, disconnected: true, notSent: false) == .unknown)
    #expect(AskAboutTask.DraftResult.failed(word: nil, disconnected: true, notSent: true) == .declined)
    #expect(AskAboutTask.DraftResult.failed(word: "not-found", disconnected: false, notSent: false) == .declined)
}
