import Foundation
import Testing

@testable import AgentKit

/// Decode real bytes the daemon emitted, captured from a live session that
/// dispatched three subagents.
///
/// `AgentStream.pump` decodes with `try?` and drops what fails, so a field
/// this side spells differently from the Rust side is not a crash and not a
/// warning — it is a chat that silently renders less than happened. Synthetic
/// JSON in the other tests cannot catch that, because the same author wrote
/// both sides of it.
@Test func everyEventTheDaemonActuallySentDecodes() throws {
    let url = Bundle.module.url(forResource: "live_events", withExtension: "jsonl")
    let path = url?.path ?? "Tests/AgentKitTests/live_events.jsonl"
    let raw = (try? String(contentsOfFile: path, encoding: .utf8)) ?? ""
    let lines = raw.split(separator: "\n").filter { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
    #expect(!lines.isEmpty, "no captured events at \(path)")

    var undecodable: [String] = []
    var gaps = 0
    var parented = 0
    var dispatches = 0
    var summaries = 0

    for line in lines {
        guard let event = try? AgentEvent.decode(from: String(line)) else {
            undecodable.append(String(line.prefix(120)))
            continue
        }
        switch event {
        case .gap: gaps += 1
        case let .message(_, _, parent): if parent != nil { parented += 1 }
        case let .toolCall(_, _, _, _, _, parent, subagent):
            if parent != nil { parented += 1 }
            if subagent { dispatches += 1 }
        case let .toolUpdate(_, _, _, _, _, _, parent, summary):
            if parent != nil { parented += 1 }
            if summary != nil { summaries += 1 }
        default: break
        }
    }

    #expect(undecodable.isEmpty, "the daemon sent events this client cannot read: \(undecodable)")
    #expect(gaps == 0, "real traffic decoded into \(gaps) gaps")
    print("decoded \(lines.count): parented=\(parented) dispatches=\(dispatches) summaries=\(summaries)")
}
