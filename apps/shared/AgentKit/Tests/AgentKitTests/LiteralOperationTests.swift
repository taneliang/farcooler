import Foundation
import Testing
@testable import AgentKit

@Suite struct LiteralOperationTests {
    func metadata() -> [String: Any] {
        ["version": 1, "provenance": "literal_intent", "kind": "task_note_decision", "target": "ov-893",
         "preview": "Exact  spacing; λ", "preview_clipped": false, "lifecycle": "unconfirmed",
         "input_clipped": false, "output_available": false, "output_clipped": false]
    }
    func tool(_ operation: Any?) throws -> AgentRow.Tool {
        var payload: [String: Any] = ["name": "Bash", "summary": "original generic summary", "status": "Done", "diff": [], "input": "original input excerpt"]
        if let operation { payload["operation"] = operation }
        let page: [String: Any] = ["epoch": 1, "rev": 1, "moreBefore": false, "rows": [["id": "tool:call", "ord": 0, "rev": 1, "provisional": false, "kind": ["Tool": payload]]]]
        let rows = try AgentRowPage.decode(JSONSerialization.data(withJSONObject: page)).rows
        guard case .tool(let tool) = try #require(rows.first).kind else { throw NSError(domain: "missing tool", code: 1) }
        return tool
    }
    @Test func wirePreservesIntentExactTextAndUncertainPresentation() throws {
        let value = try tool(metadata())
        #expect(value.name == "Bash" && value.summary == "original generic summary")
        #expect(value.input == "original input excerpt")
        #expect(value.operation?.preview == "Exact  spacing; λ")
        #expect(value.presentationName == "Far Cooler")
        #expect(value.presentationAccessibility.contains("recognized command"))
        #expect(value.presentationAccessibility.contains("Outcome unconfirmed"))
        #expect(value.presentationStatus == .ended("Outcome unconfirmed"))
        #expect(value.opens)
    }
    @Test func malformedOptionalWireMetadataNeverLosesTheGenericRow() throws {
        var variants: [Any] = ["forged", [1, 2], ["version": 7]]
        for (key, bad) in [("version", 2 as Any), ("provenance", "authenticated_receipt" as Any), ("lifecycle", "delivered" as Any), ("preview", String(repeating: "x", count: 281) as Any), ("target", "ov-893\n" as Any), ("preview_clipped", "false" as Any)] {
            var p = metadata(); p[key] = bad; variants.append(p)
        }
        var huge = metadata(); huge["unknown"] = String(repeating: "x", count: 1_024); variants.append(huge)
        for variant in variants {
            let value = try tool(variant)
            #expect(value.operation == nil)
            #expect(value.name == "Bash" && value.input == "original input excerpt")
        }
        #expect(try tool(nil).operation == nil)
    }
    @Test func cacheCodableDropsOnlyMalformedOrUnknownOptionalMetadata() throws {
        let original = try tool(metadata())
        let encoded = try JSONEncoder().encode(original)
        #expect(try JSONDecoder().decode(AgentRow.Tool.self, from: encoded) == original)
        var object = try #require(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
        let badValues: [Any] = ["malformed", ["version": 99], ["provenance": "authenticated_receipt"]]
        for bad in badValues {
            object["operation"] = bad
            let data = try JSONSerialization.data(withJSONObject: object)
            let value = try JSONDecoder().decode(AgentRow.Tool.self, from: data)
            #expect(value.operation == nil && value.input == original.input && value.name == original.name)
        }
        object.removeValue(forKey: "operation")
        #expect(try JSONDecoder().decode(AgentRow.Tool.self, from: JSONSerialization.data(withJSONObject: object)).operation == nil)
    }
    @Test func collapsedMixedGroupsKeepProvenanceAndCannotShowSuccessForUncertainty() throws {
        let value = try tool(metadata())
        let generic = try tool(nil)
        #expect(AgentConversation.recognizedGroupLabel([generic, value]) == "Recognized commands · Outcome unconfirmed")
        #expect(AgentConversation.groupStatus([generic, value]) == .ended("Outcome unconfirmed"))
        #expect(!AgentConversation.groupPurpose([value]).contains("Exact  spacing"))
        #expect(value.presentationSummary == "Add decision · ov-893")
        #expect(AgentConversation.recognizedGroupLabel([generic]) == nil)
    }
    @Test func encodedLimitAndDirectionControlsStayBoundedAndExact() throws {
        var p = metadata(); p["preview"] = "λ\u{202E}Exact";
        let value = try tool(p)
        #expect(value.operation?.containsControls == true)
        #expect(value.operation?.preview == "λ\u{202E}Exact")
        p["preview"] = String(repeating: "🐱", count: 280)
        #expect(try tool(p).operation == nil, "encoded metadata exceeds1024 even below scalar count ceiling")
    }
    @Test func genericAccessibilityKeepsStatus() throws {
        for (status, expected) in [(AgentRow.Status.running, "Running"), (.done, "Done"), (.failed, "Failed"), (.ended("other"), "Ended")] {
            var value = try tool(nil)
            value.status = status
            #expect(value.presentationAccessibility.contains(expected))
        }
    }

}
