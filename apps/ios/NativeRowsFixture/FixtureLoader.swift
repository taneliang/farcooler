import CryptoKit
import Foundation

/// Test-host-only loader. Production row decoding remains intentionally tolerant.
enum FixtureFailure: String, Error {
    case identity, missing, unreadable, oversized, digest, malformed, schema, rows, semantics, grouping
}

enum FixtureLoader {
    static let hostID = "com.farcooler.fixture.native-rows"
    static let digest = "0f963393dc148ad45290ce4b90116d80ca76ef9294e72882cd3e287d5c4419de"
    static let limit = 65_536
    static let ids = ["turn:fixture-turn", "tool:exec-28657ce1-a066-4a00-a17b-ca479ae15b42", "tool:exec-fccc8116-d738-4d7f-ac0f-f655c8a13542"]

    struct Validated {
        let page: AgentRowPage
        let tools: [AgentRow]
    }

    /// This entry point always checks real identity and digest before decode.
    static func load(bundle: Bundle = .main) throws -> Validated {
        guard bundle.bundleIdentifier == hostID else { throw FixtureFailure.identity }
        let url = bundle.url(forResource: "agent-rows-ov893-producer", withExtension: "json")
        return try verify(identity: bundle.bundleIdentifier, data: readResource(url))
    }

    static func readResource(_ url: URL?) throws -> Data {
        guard let url else { throw FixtureFailure.missing }
        guard let input = InputStream(url: url) else { throw FixtureFailure.unreadable }
        return try read(input)
    }

    static func read(_ input: InputStream) throws -> Data {
        input.open(); defer { input.close() }
        guard input.streamStatus != .error else { throw FixtureFailure.unreadable }
        var data = Data(); var buffer = [UInt8](repeating: 0, count: 4096)
        while data.count <= limit {
            let count = input.read(&buffer, maxLength: min(buffer.count, limit + 1 - data.count))
            if count < 0 { throw FixtureFailure.unreadable }
            if count == 0 {
                guard input.streamStatus != .error else { throw FixtureFailure.unreadable }
                break
            }
            data.append(contentsOf: buffer.prefix(count))
        }
        guard data.count <= limit else { throw FixtureFailure.oversized }
        return data
    }

    static func verify(identity: String?, data: Data) throws -> Validated {
        guard identity == hostID else { throw FixtureFailure.identity }
        guard data.count <= limit else { throw FixtureFailure.oversized }
        let actual = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
        guard actual == digest else { throw FixtureFailure.digest }
        return try validate(data)
    }

    /// Pure seam for adversarial tests. It never bypasses load's digest gate.
    static func validate(_ data: Data) throws -> Validated {
        guard data.count <= limit else { throw FixtureFailure.oversized }
        func integer(_ value: Any?) -> Int? {
            guard let n = value as? NSNumber, String(cString: n.objCType) != "c" else { return nil }
            let value = n.doubleValue
            guard value.isFinite else { return nil }
            return Int(exactly: value)
        }
        func boolean(_ value: Any?) -> Bool? {
            guard let n = value as? NSNumber, String(cString: n.objCType) == "c" else { return nil }
            return n.boolValue
        }
        guard let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { throw FixtureFailure.malformed }
        guard Set(object.keys) == Set(["epoch", "rev", "moreBefore", "rows"]),
              integer(object["epoch"]) == 1, integer(object["rev"]) == 9,
              boolean(object["moreBefore"]) == false,
              let rawRows = object["rows"] as? [[String: Any]], rawRows.count == 3 else { throw FixtureFailure.schema }
        let rowKeys: Set<String> = ["id", "kind", "ord", "provisional", "rev", "turn"]
        guard rawRows.compactMap({ $0["id"] as? String }) == ids,
              rawRows.enumerated().allSatisfy({ Set($0.element.keys) == rowKeys && integer($0.element["ord"]) == $0.offset && boolean($0.element["provisional"]) == false }),
              rawRows[0]["turn"] is NSNull, rawRows[1]["turn"] as? String == ids[0], rawRows[2]["turn"] as? String == ids[0],
              integer(rawRows[0]["rev"]) == 2, integer(rawRows[1]["rev"]) == 5, integer(rawRows[2]["rev"]) == 9,
              let turn = rawRows[0]["kind"] as? [String: Any], Set(turn.keys) == ["Turn"],
              let genericKind = rawRows[1]["kind"] as? [String: Any], Set(genericKind.keys) == ["Tool"],
              let generic = genericKind["Tool"] as? [String: Any], generic["operation"] == nil,
              let noteKind = rawRows[2]["kind"] as? [String: Any], Set(noteKind.keys) == ["Tool"],
              let note = noteKind["Tool"] as? [String: Any], let op = note["operation"] as? [String: Any] else { throw FixtureFailure.rows }
        guard let turnPayload = turn["Turn"] as? [String: Any],
              Set(turnPayload.keys) == ["activity", "background_running", "duration_ms", "ended_ms", "origin", "outcome", "prompt", "started_ms"],
              turnPayload["activity"] as? String == "Busy", integer(turnPayload["background_running"]) == 0,
              turnPayload["origin"] as? String == "Other", turnPayload["prompt"] as? String == "",
              ["duration_ms", "ended_ms", "outcome", "started_ms"].allSatisfy({ turnPayload[$0] is NSNull }) else { throw FixtureFailure.semantics }
        let expectedGenericInput = "command: [\"/bin/zsh\",\"-lc\",\"'/Applications/Far Cooler Canary.app/Contents/Resources/farcooler' task set ov-893 --status in_progress; '/Applications/Far Cooler Canary.app/Contents/Resources/farcooler' task note ov-893 --kind decision --body 'Research native operation rows using actual tool-call/projector data and owner screenshots, after ov-894. Keep exact raw expansion and conservative fallback for shell compounds/dynamic text; semantic recognition never supplies caller authority or success. No product edits/builds.'\"]"
        let expectedNoteInput = "command: [\"/bin/zsh\",\"-lc\",\"'/Applications/Far Cooler Canary.app/Contents/Resources/farcooler' task note ov-893 --kind decision --body 'Recommend optional semantic metadata on existing stable Tool rows: conservative literal-command recognition for immediate presentation, later corroborated by authenticated CLI/daemon receipts. Never derive authority from actor flags or success from command text/output wording. Keep compounds/dynamic commands generic unless per-operation execution receipts prove their identity; preserve raw input/output and authorized links.'\"]"
        let expectedGenericSummary = "'/Applications/Far Cooler Canary.app/Contents/Resources/farcooler' task set ov-8…"
        let expectedNoteSummary = "'/Applications/Far Cooler Canary.app/Contents/Resources/farcooler' task note ov-…"
        let toolKeys: Set<String> = ["diff", "ended_ms", "file_path", "input", "name", "result", "started_ms", "status", "summary"]
        let opKeys: Set<String> = ["version", "provenance", "kind", "target", "preview", "preview_clipped", "lifecycle", "input_clipped", "output_available", "output_clipped"]
        guard Set(generic.keys) == toolKeys, Set(note.keys) == toolKeys.union(["operation"]), Set(op.keys) == opKeys,
              generic["name"] as? String == "Bash", note["name"] as? String == "Bash",
              generic["result"] as? String == "ov-893  in_progress  Agent view: Far Cooler operations get native rows\nov-893  c143ae3d  decision",
              let genericInput = generic["input"] as? String, genericInput == expectedGenericInput,
              genericInput.unicodeScalars.count <= 2000, genericInput.split(separator: "\n", omittingEmptySubsequences: false).count <= 40,
              generic["summary"] as? String == expectedGenericSummary, note["summary"] as? String == expectedNoteSummary,
              (generic["diff"] as? [Any])?.isEmpty == true, (note["diff"] as? [Any])?.isEmpty == true,
              generic["file_path"] is NSNull, note["file_path"] is NSNull,
              boolean(op["input_clipped"]) == false, boolean(op["output_available"]) == true, boolean(op["output_clipped"]) == false,
              integer(op["version"]) == 1, op["provenance"] as? String == "literal_intent",
              op["kind"] as? String == "task_note_decision", op["target"] as? String == "ov-893",
              op["lifecycle"] as? String == "completed", boolean(op["preview_clipped"]) == true,
              let preview = op["preview"] as? String, !preview.isEmpty, preview.unicodeScalars.count <= 280,
              let raw = note["input"] as? String, raw == expectedNoteInput, raw.hasPrefix("command: "),
              let commandData = String(raw.dropFirst("command: ".count)).data(using: .utf8),
              let command = (try? JSONSerialization.jsonObject(with: commandData)) as? [String], command.count == 3,
              command[0] == "/bin/zsh", command[1] == "-lc",
              command[2].hasPrefix("'/Applications/Far Cooler Canary.app/Contents/Resources/farcooler' task note ov-893 --kind decision --body '"),
              raw.unicodeScalars.count <= 2000, raw.split(separator: "\n", omittingEmptySubsequences: false).count <= 40,
              preview.split(separator: "\n", omittingEmptySubsequences: false).count <= 4,
              let body = command[2].components(separatedBy: "--body '").last, body.hasSuffix("'"),
              String(body.dropLast()).unicodeScalars.count > 280,
              preview == String(String(body.dropLast()).unicodeScalars.prefix(280)), raw.contains(preview),
              note["result"] as? String == "ov-893  264ca0f7  decision",
              note["status"] as? String == "Done", generic["status"] as? String == "Done",
              integer(note["started_ms"]) == 1793574957570, integer(note["ended_ms"]) == 1793574959743,
              integer(generic["started_ms"]) == 1793574924733, integer(generic["ended_ms"]) == 1793574926490 else { throw FixtureFailure.semantics }
        let page: AgentRowPage
        do { page = try AgentRowPage.decode(data) } catch { throw FixtureFailure.malformed }
        guard page.epoch == 1, page.rev == 9, !page.moreBefore,
              page.rows.map(\.id) == ids, page.rows.count == 3,
              page.rows.map(\.ord) == [0, 1, 2], page.rows.map(\.rev) == [2, 5, 9],
              page.rows.allSatisfy({ !$0.provisional }), page.rows.map(\.turn) == [nil, ids[0], ids[0]],
              case .turn(let decodedTurn) = page.rows[0].kind,
              decodedTurn.prompt == "", decodedTurn.origin == "Other", decodedTurn.activity == "Busy",
              decodedTurn.backgroundRunning == 0, decodedTurn.startedMs == nil, decodedTurn.endedMs == nil,
              decodedTurn.durationMs == nil, decodedTurn.outcome == nil, decodedTurn.suggestion == nil,
              decodedTurn.images.isEmpty, decodedTurn.tokens == nil, decodedTurn.harness == nil,
              case .tool(let first) = page.rows[1].kind, first.operation == nil,
              first.name == "Bash", first.summary == expectedGenericSummary, first.status == .done,
              first.startedMs == 1793574924733, first.endedMs == 1793574926490,
              first.diff.isEmpty, first.filePath == nil, first.input == expectedGenericInput,
              first.result == (generic["result"] as? String),
              case .tool(let second) = page.rows[2].kind,
              second.name == "Bash", second.summary == expectedNoteSummary, second.status == .done,
              second.startedMs == 1793574957570, second.endedMs == 1793574959743,
              second.diff.isEmpty, second.filePath == nil,
              let decodedOp = second.operation, decodedOp.version == 1, decodedOp.provenance == "literal_intent",
              decodedOp.kind == "task_note_decision", decodedOp.target == "ov-893", decodedOp.preview == preview,
              decodedOp.previewClipped, decodedOp.lifecycle == .completed, !decodedOp.inputClipped,
              decodedOp.outputAvailable, !decodedOp.outputClipped,
              second.input == raw, second.result == "ov-893  264ca0f7  decision" else { throw FixtureFailure.semantics }
        let items = AgentConversation.items(page.rows.map(\.id)) { id in
            guard let row = page.rows.first(where: { $0.id == id }) else { return .other }
            return AgentConversation.groupRole(row.kind)
        }
        guard items == [.row(ids[0]), .tools(id: "tools:\(ids[1])", rows: [ids[1], ids[2]])] else { throw FixtureFailure.grouping }
        return Validated(page: page, tools: Array(page.rows.dropFirst()))
    }
}

/// A failure can never carry a page or a readiness marker.
enum FixtureState {
    case loading
    case failed(FixtureFailure)
    case ready(FixtureLoader.Validated)

    static func resolve(_ load: () throws -> FixtureLoader.Validated) -> FixtureState {
        do { return .ready(try load()) }
        catch let failure as FixtureFailure { return .failed(failure) }
        catch { return .failed(.unreadable) }
    }
}
