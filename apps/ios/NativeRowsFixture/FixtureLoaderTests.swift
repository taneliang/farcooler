import XCTest
import Foundation
@testable import NativeRowsFixtureHost

final class FixtureLoaderTests: XCTestCase {
    private func fixture() throws -> Data {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "agent-rows-ov893-producer", withExtension: "json"))
        return try Data(contentsOf: url)
    }
    private func error(_ expected: FixtureFailure, _ body: () throws -> Void) {
        XCTAssertThrowsError(try body()) { XCTAssertEqual($0 as? FixtureFailure, expected) }
    }
    func testPinnedIdentityDigestAndCap() throws {
        let data = try fixture()
        let value = try FixtureLoader.verify(identity: FixtureLoader.hostID, data: data)
        XCTAssertEqual(value.tools.map(\.id), Array(FixtureLoader.ids.dropFirst()))
        error(.identity) { _ = try FixtureLoader.verify(identity: "com.farcooler.ios", data: data) }
        error(.missing) { _ = try FixtureLoader.readResource(nil) }
        error(.unreadable) { _ = try FixtureLoader.readResource(URL(fileURLWithPath: "/nonexistent-fixture-ov901")) }
        error(.digest) { _ = try FixtureLoader.verify(identity: FixtureLoader.hostID, data: Data()) }
        var changed = data; changed[0] ^= 1
        error(.digest) { _ = try FixtureLoader.verify(identity: FixtureLoader.hostID, data: changed) }
        XCTAssertEqual(try FixtureLoader.read(InputStream(data: Data(repeating: 1, count: FixtureLoader.limit))).count, FixtureLoader.limit)
        error(.oversized) { _ = try FixtureLoader.read(InputStream(data: Data(repeating: 1, count: FixtureLoader.limit + 1))) }
    }
    func testFailureNeverCarriesReadyRows() {
        for failure in [FixtureFailure.identity, .missing, .unreadable, .oversized, .digest, .malformed, .schema, .rows, .semantics, .grouping] {
            guard case .failed(let actual) = FixtureState.resolve({ throw failure }) else { return XCTFail("failure produced ready") }
            XCTAssertEqual(actual, failure)
        }
    }
    func testStrictValidatorRejectsMutationsWithoutWeakeningDigest() throws {
        let data = try fixture()
        error(.malformed) { _ = try FixtureLoader.validate(Data("{".utf8)) }
        error(.malformed) { _ = try FixtureLoader.validate(Data(data.prefix(30))) }
        error(.oversized) { _ = try FixtureLoader.validate(Data(repeating: 1, count: FixtureLoader.limit + 1)) }
        func mutate(_ edit: (inout [String: Any]) -> Void) throws -> Data {
            var object = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any]); edit(&object)
            return try JSONSerialization.data(withJSONObject: object)
        }
        // Valid control for the pure seam; verify remains the only loader path.
        XCTAssertEqual(try FixtureLoader.validate(data).tools.map(\.id), Array(FixtureLoader.ids.dropFirst()))
        func rejected(_ expected: FixtureFailure, _ edit: (inout [String: Any]) -> Void) throws {
            let altered = try mutate(edit)
            error(expected) { _ = try FixtureLoader.validate(altered) }
            error(.digest) { _ = try FixtureLoader.verify(identity: FixtureLoader.hostID, data: altered) }
        }
        for (key, bad) in [("epoch", 1.5 as Any), ("rev", 9.5 as Any), ("epoch", "1" as Any), ("rev", 1e100 as Any)] {
            try rejected(.schema) { $0[key] = bad }
        }
        for (key, bad) in [("ord", 2.5 as Any), ("rev", 9.5 as Any), ("ord", true as Any)] {
            try rejected(.rows) { object in
                var rows = object["rows"] as! [[String: Any]]; rows[2][key] = bad; object["rows"] = rows
            }
        }
        for (key, bad) in [("version", 1.5 as Any), ("version", true as Any), ("preview", "R" as Any)] {
            try rejected(.semantics) { object in
                var rows = object["rows"] as! [[String: Any]]; var kind = rows[2]["kind"] as! [String: Any]
                var tool = kind["Tool"] as! [String: Any]; var op = tool["operation"] as! [String: Any]
                op[key] = bad; tool["operation"] = op; kind["Tool"] = tool; rows[2]["kind"] = kind; object["rows"] = rows
            }
        }
        for (index, key, bad) in [(1, "input", "other; command" as Any), (1, "summary", "wrong" as Any),
                                  (2, "summary", "wrong" as Any), (1, "diff", ["wrong"] as Any),
                                  (2, "file_path", "other" as Any), (2, "started_ms", 1793574957570.5 as Any),
                                  (1, "ended_ms", "1793574926490" as Any)] {
            try rejected(.semantics) { object in
                var rows = object["rows"] as! [[String: Any]]; var kind = rows[index]["kind"] as! [String: Any]
                var tool = kind["Tool"] as! [String: Any]; tool[key] = bad; kind["Tool"] = tool; rows[index]["kind"] = kind; object["rows"] = rows
            }
        }
        try rejected(.semantics) { object in
            var rows = object["rows"] as! [[String: Any]]; var kind = rows[0]["kind"] as! [String: Any]
            var turn = kind["Turn"] as! [String: Any]; turn["prompt"] = "unexpected"; kind["Turn"] = turn
            rows[0]["kind"] = kind; object["rows"] = rows
        }
        error(.schema) { _ = try FixtureLoader.validate(mutate { $0["rev"] = 10 }) }
        error(.schema) { _ = try FixtureLoader.validate(mutate { $0["epoch"] = true }) }
        error(.schema) { _ = try FixtureLoader.validate(mutate { $0["extra"] = "unexpected" }) }
        error(.schema) { _ = try FixtureLoader.validate(mutate { $0.removeValue(forKey: "epoch") }) }
        error(.schema) { _ = try FixtureLoader.validate(mutate { $0["rows"] = [] }) }
        error(.rows) { _ = try FixtureLoader.validate(mutate { object in
            var rows = object["rows"] as! [[String: Any]]; rows[0]["kind"] = ["Unknown": [:]]; object["rows"] = rows
        }) }
        error(.rows) { _ = try FixtureLoader.validate(mutate { object in
            var rows = object["rows"] as! [[String: Any]]; rows[2]["id"] = rows[1]["id"]; object["rows"] = rows
        }) }
        error(.rows) { _ = try FixtureLoader.validate(mutate { object in
            var rows = object["rows"] as! [[String: Any]]; rows[2]["turn"] = "other"; object["rows"] = rows
        }) }
        for (key, bad) in [("version", 2 as Any), ("provenance", "receipt" as Any), ("preview", "not original" as Any), ("kind", "task_show" as Any), ("lifecycle", "running" as Any)] {
            let altered = try mutate { object in
                var rows = object["rows"] as! [[String: Any]]; var kind = rows[2]["kind"] as! [String: Any]
                var tool = kind["Tool"] as! [String: Any]; var operation = tool["operation"] as! [String: Any]
                operation[key] = bad; tool["operation"] = operation; kind["Tool"] = tool; rows[2]["kind"] = kind; object["rows"] = rows
            }
            error(.semantics) { _ = try FixtureLoader.validate(altered) }
            error(.digest) { _ = try FixtureLoader.verify(identity: FixtureLoader.hostID, data: altered) }
        }
        for field in ["input", "result", "status"] {
            let altered = try mutate { object in
                var rows = object["rows"] as! [[String: Any]]; var kind = rows[2]["kind"] as! [String: Any]
                var tool = kind["Tool"] as! [String: Any]; tool[field] = "wrong"; kind["Tool"] = tool; rows[2]["kind"] = kind; object["rows"] = rows
            }
            error(.semantics) { _ = try FixtureLoader.validate(altered) }
            error(.digest) { _ = try FixtureLoader.verify(identity: FixtureLoader.hostID, data: altered) }
        }
    }
}
