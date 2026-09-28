import Foundation

/// The question a task in Needs Decision is waiting on, with the answers it
/// offered: what the card draws as its Answer buttons (spec §2.5).
///
/// Read out of `task show --json` rather than out of `TaskDetailModel`,
/// which drops a note's `extra`; the options live there, as
/// `{"options": [...]}`, written by `farcooler task ask --option`.
public struct TaskQuestion: Equatable, Sendable, Identifiable {
    /// The question note's id.
    public var id: String
    /// What was asked, verbatim.
    public var body: String
    /// The answers the asker offered, in their order. Empty for a question
    /// that offered none, which the card answers with Answer… instead.
    public var options: [String]

    public init(id: String, body: String, options: [String]) {
        self.id = id
        self.body = body
        self.options = options
    }

    /// How many options are drawn as buttons. More than three in a row is a
    /// row nobody reads, so the rest go in a menu.
    public static let buttonLimit = 3

    /// The options drawn as buttons.
    public var buttons: [String] { Array(options.prefix(Self.buttonLimit)) }

    /// The options past the buttons, for a menu beside them.
    public var overflow: [String] { Array(options.dropFirst(Self.buttonLimit)) }

    /// The question still waiting in a task's record, or nil when there is
    /// none: no question, or the latest one answered since.
    ///
    /// The latest question is the open one, whatever came before it. An
    /// answer written after it closes it; the card then offers no buttons,
    /// and the record below says what the answer was.
    public static func open(in data: Data) -> TaskQuestion? {
        guard let wire = try? JSONDecoder().decode(Wire.self, from: data) else { return nil }
        // By time, and by the order the runner listed them where two share a
        // millisecond.
        let notes = wire.notes.enumerated().sorted {
            ($0.element.at, $0.offset) < ($1.element.at, $1.offset)
        }.map(\.element)
        guard let last = notes.lastIndex(where: { $0.kind == "question" }) else { return nil }
        if notes[(last + 1)...].contains(where: { $0.kind == "answer" }) { return nil }
        let question = notes[last]
        return TaskQuestion(
            id: question.id, body: question.body, options: question.extra?.options ?? [])
    }

    private struct Wire: Decodable {
        var notes: [Note]

        enum CodingKeys: String, CodingKey { case notes }

        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            notes = try c.decodeIfPresent([Note].self, forKey: .notes) ?? []
        }
    }

    private struct Note: Decodable {
        var id: String
        var kind: String
        var at: Int64
        var body: String
        var extra: Extra?

        enum CodingKeys: String, CodingKey { case id, kind, at, body, extra }

        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            id = try c.decode(String.self, forKey: .id)
            kind = try c.decode(String.self, forKey: .kind)
            at = try c.decodeIfPresent(Int64.self, forKey: .at) ?? 0
            body = try c.decodeIfPresent(String.self, forKey: .body) ?? ""
            // An `extra` this build can't read is no options, not no note.
            extra = try? c.decodeIfPresent(Extra.self, forKey: .extra)
        }
    }

    private struct Extra: Decodable {
        var options: [String]?
    }
}
