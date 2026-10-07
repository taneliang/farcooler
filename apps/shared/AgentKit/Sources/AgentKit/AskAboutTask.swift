import Foundation

/// Ask the Orchestrator, on a phone's task screen (ov-241).
///
/// The Mac's `AskOrchestrator` (ov-184), for the same job: the orchestrator
/// owns the task list, so a person who wants a task moved, reworded, started or
/// dropped says so to the orchestrator. This puts the start of that message in
/// its composer, naming the task, and goes there; the person finishes the
/// sentence and sends it.
///
/// It never starts an orchestrator. With none running the control stays on the
/// screen, off, and says what to do, so a person looking for it learns why it
/// can't be used.
///
/// Here rather than in the screens, because `apps/ios` has no unit tests CI
/// runs and these are the rules with a person's words and a terminal in them.
/// Android says the same sentences from `model/AskAboutTask.kt`.
public enum AskAboutTask {
    /// The control's title.
    public static let title = "Ask the Orchestrator"

    /// Why the control is off, and what turns it on.
    public static let unavailable = "Start an orchestrator to ask about this task"

    /// The words left in the composer: a reference, not a copy, because the
    /// orchestrator reads the task itself (`farcooler task show`). Ends where
    /// the person goes on typing.
    public static func draft(key: String, title: String) -> String {
        "About \(oneLine(key)) (“\(oneLine(title))”): "
    }

    /// `raw` as one line that does nothing when it reaches a composer or a
    /// terminal, the way the daemon's `one_line` makes one (here by removing
    /// rather than spelling out): escape sequences (a CSI such as `ESC[201~`,
    /// which would close a bracketed paste, an OSC, or ESC and the character
    /// after it), every other control character (^C, ^D, ^U, DEL, C1), line
    /// breaks and tabs as spaces, and invisible format characters (bidi
    /// overrides, zero-width marks). Runs of space become one. The one place a
    /// task's words are made safe, for the composer, the daemon and the
    /// clipboard alike.
    public static func oneLine(_ raw: String) -> String {
        var out = String.UnicodeScalarView()
        var scalars = raw.unicodeScalars[...]
        while let c = scalars.popFirst() {
            switch c.value {
            case 0x1B:
                guard let kind = scalars.popFirst() else { break }
                if kind == "[" {
                    // CSI: parameters, then one final byte, 0x40 to 0x7E.
                    while let next = scalars.popFirst(), !(0x40...0x7E).contains(next.value) {}
                } else if kind == "]" {
                    // OSC: up to BEL, or ESC and a backslash.
                    while let next = scalars.popFirst() {
                        if next.value == 0x07 { break }
                        if next.value == 0x1B { _ = scalars.popFirst(); break }
                    }
                }
            case 0x09, 0x0A, 0x0D, 0x0B, 0x0C, 0x85, 0x2028, 0x2029:
                out.append(" ")
            case 0x00...0x1F, 0x7F...0x9F, 0xAD, 0x200B...0x200F, 0x202A...0x202E,
                0x2060...0x2064, 0x2066...0x2069, 0xFEFF:
                break
            default:
                out.append(c)
            }
        }
        return String(out).split(separator: " ").joined(separator: " ")
    }

    /// Where the draft went.
    public enum Delivery: Equatable, Sendable {
        /// A chat orchestrator's composer, waiting for the person.
        case composer
        /// A terminal orchestrator's input line, pasted by the runner with no
        /// Enter.
        case pasted
        /// Onto the clipboard: the runner couldn't prove the pane safe, or is
        /// too old to hold a draft behind a dialog.
        case copied
        /// Held by the runner behind a dialog, pasted once it closes (ov-385).
        case held(DraftHold)
        /// The runner never answered in time, so it may have pasted after all.
        /// Nothing is copied and nothing is claimed: copying as well would leave
        /// the reference in the box and on the clipboard under a notice that says
        /// the paste didn't happen.
        case maybePasted
    }

    /// What the runner said to a paste into a terminal orchestrator.
    public enum DraftResult: Equatable, Sendable {
        case pasted
        /// A dialog was up: the runner holds it and pastes it once it closes.
        case held(DraftHold)
        /// It refused, or the call never left this phone: nothing was typed.
        case declined
        /// No answer in time, or the link dropped mid-call: it may have been typed.
        case unknown

        /// From a failed call's answer line, as `WriteOutcome.failed` reads one.
        public static func failed(word: String?, disconnected: Bool, notSent: Bool) -> DraftResult {
            if case .neverSent = WriteOutcome.failed(word: word, disconnected: disconnected, notSent: notSent) {
                return .declined
            }
            return .unknown
        }
    }

    /// What the screen says when the reference was copied instead of pasted.
    public static func copiedNotice(key: String) -> String {
        "Copied a reference to \(oneLine(key)). Paste it into the orchestrator."
    }

    /// Hand the draft to the orchestrator, whichever kind of pane it is.
    ///
    /// A chat pane takes it in its composer. A terminal pane (a shell running
    /// claude, adopted as the orchestrator) is asked of the runner, which pastes
    /// it with no Enter only past the gate that types an answer: a proven, idle
    /// agent with an empty box and a known paste mode. A dialog in the way, on
    /// a runner that can, holds it until the dialog closes (ov-385). Anything
    /// else, or any failure, copies it instead. Nothing here ever presses Enter, and the
    /// phone never writes to the pane itself.
    @MainActor
    public static func deliver(
        key: String, title: String, isAgentPane: Bool,
        offer: (String) -> Void,
        paste: (String) async -> DraftResult,
        copy: (String) -> Void
    ) async -> Delivery {
        await deliver(
            text: draft(key: key, title: title), isAgentPane: isAgentPane, offer: offer, paste: paste, copy: copy)
    }

    /// `deliver` for any draft: the rulings' Discuss leaves its own words the
    /// same way (ov-333).
    @MainActor
    public static func deliver(
        text: String, isAgentPane: Bool,
        offer: (String) -> Void,
        paste: (String) async -> DraftResult,
        copy: (String) -> Void
    ) async -> Delivery {
        if isAgentPane {
            offer(text)
            return .composer
        }
        switch await paste(text) {
        case .pasted: return .pasted
        case .held(let hold): return .held(hold)
        case .unknown: return .maybePasted
        case .declined:
            copy(text.trimmingCharacters(in: .whitespaces))
            return .copied
        }
    }
}

/// Text one screen wants to put in a pane's composer, held until the composer
/// takes it.
///
/// The Mac's `ComposerHandoff`, for a phone: the task screen and the pane are
/// not siblings, and the pane may not be composed yet when the text is offered.
/// So it WAITS here, by terminal id, until a composer asks. Held per runner (on
/// its `Connection`), because a terminal id is minted by a daemon and two
/// runners can hand out the same one.
@MainActor
public final class ComposerOffers: ObservableObject {
    /// What is waiting, by terminal id, so a composer can watch for its own.
    @Published public private(set) var waiting: [String: String] = [:]

    public init() {}

    /// Leave text for a pane. Two offered before either is taken are joined
    /// rather than replaced: nothing a person wrote is overwritten by something
    /// else they wrote.
    public func offer(_ text: String, to terminal: String) {
        guard !text.isEmpty else { return }
        if let already = waiting[terminal], !already.isEmpty {
            waiting[terminal] = already + "\n\n" + text
        } else {
            waiting[terminal] = text
        }
    }

    /// Take what is waiting, once.
    public func take(for terminal: String) -> String? {
        waiting.removeValue(forKey: terminal)
    }

    /// What a field holds after `offered` arrives: appended behind whatever is
    /// being typed, never over it, so a half-written message survives.
    public static func joined(field: String, offered: String) -> String {
        field.isEmpty ? offered : field.trimmingCharacters(in: .whitespacesAndNewlines) + "\n\n" + offered
    }
}
