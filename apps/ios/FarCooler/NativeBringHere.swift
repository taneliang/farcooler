import Foundation

/// Bring Here (ov-369, R-28): the draft a person left in claude's own box,
/// moved into the conversation's composer, so a send has one draft and not
/// two. The runner's `terminal.bring_draft` reads the box with nothing typed;
/// the composer takes the text; then the runner clears the box, only while it
/// still holds exactly that (`BringHere`).
protocol DraftSink: Sendable {
    /// The box's draft, and whether it was cleared: read, or with
    /// `expected`, cleared of exactly that.
    func bringDraft(terminal: String, expected: String?) async throws -> (text: String, cleared: Bool)
}

/// The runner's `terminal.bring_draft`, over this phone's client core.
struct CoreDraftSink: DraftSink {
    let core: ClientCore

    func bringDraft(terminal: String, expected: String?) async throws -> (text: String, cleared: Bool) {
        var args: [String: Any] = ["terminal": terminal]
        if let expected { args["expected"] = expected }
        let data = try await core.call("terminal.bring_draft", args)
        let object = (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
        return (object["text"] as? String ?? "", object["cleared"] as? Bool ?? false)
    }
}

extension NativePaneModel {
    /// Whether the draft line offers Bring Here: claude, on a runner that
    /// serves it.
    var offersBringHere: Bool { drafts != nil && preset.hasPrefix("claude") }

    /// Move the box's draft into the composer, ahead of what it holds, and
    /// clear the box. The draft line's Bring Here.
    func bringHere() async {
        guard let drafts, !bringing, offersBringHere else { return }
        bringing = true
        defer { bringing = false }
        issue = nil
        let terminal = terminal
        issue = await BringHere.run(
            agent: agent,
            read: { await BringHere.result({ try await drafts.bringDraft(terminal: terminal, expected: nil).text }, failure: Self.failure) },
            place: { text in self.draft = BringHere.merged(box: text, native: self.draft) },
            clear: { text in
                await BringHere.result({ try await drafts.bringDraft(terminal: terminal, expected: text).cleared }, failure: Self.failure)
            })
    }
}
