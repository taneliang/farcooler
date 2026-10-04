import Foundation
import Testing

@testable import AgentKit

// A cut-off patch says it is cut off on iOS too (ov-149).

private func decode(_ json: String) throws -> ChangesFileDiff {
    try JSONDecoder().decode(ChangesFileDiff.self, from: Data(json.utf8))
}

struct ChangesFileDiffTests {
    /// The bytes `file_diff_json` emits for a patch the daemon cut off.
    @Test func aTruncatedFixtureCarriesTheNotice() throws {
        let diff = try decode(
            """
            {"path":"Cargo.lock","unsupported":null,"truncated":true,
             "firstParentOfMerge":false,
             "hunks":[{"lines":[{"kind":"added","oldNumber":null,"newNumber":1,"text":"x"}]}]}
            """)
        #expect(diff.truncated)
        #expect(diff.notices == ["This patch was cut short. It\u{2019}s too big to send whole."])
        #expect(diff.lines().count == 1)
    }

    @Test func aMergeSaysItIsShownAgainstItsFirstParent() throws {
        let diff = try decode(
            """
            {"path":"a","unsupported":null,"truncated":true,"firstParentOfMerge":true,"hunks":[]}
            """)
        #expect(
            diff.notices == [
                ChangesFileDiff.truncatedNotice, "This is a merge, shown against its first parent only.",
            ])
    }

    @Test func aWholePatchAndAnOlderDaemonSayNothing() throws {
        let whole = try decode(
            """
            {"path":"a","unsupported":null,"truncated":false,"firstParentOfMerge":false,"hunks":[]}
            """)
        #expect(whole.notices.isEmpty)
        let older = try decode(#"{"hunks":[]}"#)
        #expect(older.notices.isEmpty)
    }

    @Test func theBudgetHoldsBackPastSixHundredAndSaysHowMany() {
        let long = Array(0..<700)
        #expect(PatchBudget.visible(long, whole: false).count == 600)
        #expect(PatchBudget.visible(long, whole: true).count == 700)
        #expect(PatchBudget.moreLabel(total: 700, whole: false) == "Show 100 More Lines")
        #expect(PatchBudget.moreLabel(total: 601, whole: false) == "Show 1 More Line")
        #expect(PatchBudget.moreLabel(total: 700, whole: true) == nil)
        #expect(PatchBudget.moreLabel(total: 600, whole: false) == nil)
    }
}
