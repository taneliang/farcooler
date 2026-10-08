import Foundation

/// Bring Here (ov-369, R-28): one draft across a conversation's composer and
/// claude's own box in the terminal.
///
/// The composer's draft is the device's own until Send; the box's is the
/// terminal's. A send needs the box empty, so a box holding text refuses it
/// (`draft`), and the composer offers Bring Here, which moves the box's text
/// into the composer, or Show Terminal.
///
/// Bring Here is two calls to the runner's `terminal.bring_draft`, so the
/// text is never in neither place: a read, which types nothing, then the
/// composer takes the text, then a clear, which empties the box only while
/// it still holds exactly what was read.
///
/// The text must end in exactly one place unless the runner can't say which:
/// - A clear that was refused, or answered that it cleared nothing, left the
///   box as it was (or emptied by someone else), so the composer gives the
///   text back (`withdraw`): left in, a refused clear would have the person
///   clear a box that holds more than the composer does, and a send would
///   send the same words twice.
/// - Only a clear that went `partly`, or that never answered, leaves the text
///   in both, and the composer says so.
///
/// Here so `swift test` reads the rules back, for the Mac and the phone
/// alike.
public enum BringHere {
    /// Whether a pane's composer offers Bring Here: claude, on a runner that
    /// serves `bring_draft`. Elsewhere it offers Show Terminal alone.
    public static func offered(preset: String, build: DaemonBuild?) -> Bool {
        preset.hasPrefix("claude") && build?.can(.bringDraft) == true
    }

    /// The composer's text once the box's is brought: the box's first, since
    /// it was there first, then on the next line what the composer held.
    public static func merged(box: String, native: String) -> String {
        let box = trimmedEnd(box)
        let native = native.trimmingCharacters(in: .newlines)
        if box.isEmpty { return native }
        if native.trimmingCharacters(in: .whitespaces).isEmpty { return box }
        return box + "\n" + native
    }

    /// `text` without its trailing whitespace and line breaks: a first
    /// line's indent is the draft's.
    private static func trimmedEnd(_ text: String) -> String {
        var end = text.endIndex
        while end > text.startIndex, text[text.index(before: end)].isWhitespace { end = text.index(before: end) }
        return String(text[..<end])
    }

    /// The composer's text once the box's, which `merged` put first, is taken
    /// back out. A composer that no longer starts with it is left alone.
    public static func withdrawn(box: String, from composer: String) -> String {
        let box = trimmedEnd(box)
        guard !box.isEmpty, composer.hasPrefix(box) else { return composer }
        var rest = composer.dropFirst(box.count)
        if rest.hasPrefix("\n") { rest = rest.dropFirst() }
        return String(rest)
    }

    /// Run Bring Here: `read` the box; hand its text to `place`, which puts
    /// it in the composer (`merged` with what the composer holds then); then
    /// `clear` the box of exactly that text; or, where the box wasn't cleared
    /// and still holds it (or someone else emptied it), `withdraw` it from the
    /// composer again. Each call answers what the runner said, or how it
    /// failed. Answers what the composer's line says after, nil for nothing.
    /// Runs on the caller's actor (`isolation`), so the closures need not be
    /// `Sendable`: a pane's model calls it from the main actor.
    public static func run(
        isolation: isolated (any Actor)? = #isolation,
        agent: String = "Claude",
        read: () async -> Result<String, AgentConversation.SendFailure>,
        place: (String) async -> Void,
        withdraw: (String) async -> Void,
        clear: (String) async -> Result<Bool, AgentConversation.SendFailure>
    ) async -> AgentConversation.SendIssue? {
        let text: String
        switch await read() {
        case .failure(let failure): return issue(for: failure, agent: agent)
        case .success(let read): text = read
        }
        // Emptied in the terminal since the send was refused: nothing to
        // bring, and the next Send finds the box free.
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return nil }
        // In the composer before the box is touched: whatever the clear
        // does, the text isn't lost.
        await place(text)
        switch await clear(text) {
        case .success(true): return nil
        case .success(false):
            // The box held nothing when the clear came: sent from the
            // terminal, or taken by another device. The composer's copy
            // would send it twice.
            await withdraw(text)
            return .said("The terminal’s box was emptied before Far Cooler could move its draft. If it was sent, it’s in the conversation.")
        case .failure(let failure):
            guard !leavesTextInBoth(failure) else {
                return .draftLeftInTerminal(leftWords(for: failure, agent: agent))
            }
            await withdraw(text)
            return stayedIssue(for: failure, agent: agent)
        }
    }

    /// `result`'s value, or its failure as `run` takes it, by `failure`.
    public static func result<T>(
        isolation: isolated (any Actor)? = #isolation,
        _ call: () async throws -> T, failure: (Error) -> AgentConversation.SendFailure
    ) async -> Result<T, AgentConversation.SendFailure> {
        do { return .success(try await call()) } catch { return .failure(failure(error)) }
    }

    /// What the line says when the read was refused, and nothing moved.
    public static func issue(for failure: AgentConversation.SendFailure, agent: String = "Claude") -> AgentConversation.SendIssue {
        switch failure {
        case .refused(_, let word?) where word == "scope-denied":
            return .said("This device can’t change what’s in this runner’s terminals.")
        case .refused(let what, _):
            switch what {
            case "pasted":
                return .said("The terminal’s box holds a pasted block or an image Far Cooler can’t read. Use the terminal.")
            case "too_tall": return .said("The terminal’s draft is too long to bring here whole. Use the terminal.")
            case "cursor":
                return .said("The cursor in the terminal’s box isn’t at the end, so its draft wasn’t moved. Use the terminal.")
            case "typing": return .said("Someone is typing in the terminal. Try again in a moment.")
            case "sending": return .said("Far Cooler is still sending to the terminal. Try again in a moment.")
            case "prompt", "dialog": return .handoff
            case "unsupported": return .said("\(agent)’s draft can’t be brought here. Use the terminal.")
            case "not_running", "not_an_agent": return .said("\(agent) isn’t running in this pane.")
            default: return .said("Far Cooler can’t read the terminal’s box. Use the terminal.")
            }
        case .timedOut, .lost(notSent: false):
            return .said("The runner didn’t answer in time. Check the terminal.")
        case .lost(notSent: true):
            return .said("The runner isn’t connected. Use the terminal.")
        }
    }

    /// Whether a failed clear may have taken some of the box's text, or may
    /// yet: the composer keeps its copy. Any other failure left the box whole.
    public static func leavesTextInBoth(_ failure: AgentConversation.SendFailure) -> Bool {
        switch failure {
        case .refused(let what, _): return what == "partly"
        case .timedOut, .lost(notSent: false): return true
        case .lost(notSent: true): return false
        }
    }

    /// What the line says when the clear failed and the draft stayed in the
    /// box alone.
    public static func stayedIssue(for failure: AgentConversation.SendFailure, agent: String = "Claude") -> AgentConversation.SendIssue {
        switch failure {
        case .refused(let what, _) where what == "typing" || what == "sending":
            return .said("Someone is typing in the terminal, so its draft stayed there. Try again in a moment.")
        case .refused(let what, _) where what == "changed" || what == "too_tall":
            return .draftInTerminal
        default: return issue(for: failure, agent: agent)
        }
    }

    /// What the line says when the box's text is here but the clear didn't
    /// go, or may not have: the text is in both places.
    public static func leftWords(for failure: AgentConversation.SendFailure, agent: String = "Claude") -> String {
        switch failure {
        case .refused(let what, _) where what == "partly":
            return "The draft is here, and part of it is still in the terminal’s box. Clear it there before sending."
        case .timedOut, .lost(notSent: false):
            return "The draft is here. The runner didn’t answer in time, so check the terminal’s box before sending."
        default:
            return "The draft is here, but it’s still in the terminal’s box too. Clear it there before sending."
        }
    }
}

/// A failed call, as `BringHere.run`'s closures answer it.
extension AgentConversation.SendFailure: Error {}
