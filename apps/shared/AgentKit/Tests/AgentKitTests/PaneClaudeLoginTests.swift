import XCTest

@testable import AgentKit

/// A pane's info says whose Claude login its agent runs on (ov-597).
final class PaneClaudeLoginTests: XCTestCase {
    private let alex = PaneClaudeLogin(handle: "alex", name: "Alex Rivera", owner: false)
    private let owner = PaneClaudeLogin(handle: "", name: "", owner: true)

    func testAMembersLoginIsNamedAfterThem() {
        XCTAssertEqual(alex.sentence(viewer: ""), "Uses Alex Rivera’s Claude login")
        XCTAssertEqual(alex.sentence(viewer: "bo"), "Uses Alex Rivera’s Claude login")
        XCTAssertEqual(alex.sentence(viewer: nil), "Uses Alex Rivera’s Claude login")
    }

    func testTheViewersOwnLoginIsYours() {
        XCTAssertEqual(alex.sentence(viewer: "alex"), "Uses your Claude login")
        XCTAssertEqual(owner.sentence(viewer: ""), "Uses your Claude login")
    }

    func testTheOwnersLoginIsTheOwnersToEveryoneElse() {
        XCTAssertEqual(owner.sentence(viewer: "alex"), "Uses the owner’s Claude login")
        XCTAssertEqual(owner.sentence(viewer: nil), "Uses the owner’s Claude login")
    }

    func testAReleasedDeviceIsNobodysSoNothingIsYours() {
        XCTAssertEqual(alex.sentence(viewer: RunnerPeople.released), "Uses Alex Rivera’s Claude login")
        XCTAssertEqual(owner.sentence(viewer: RunnerPeople.released), "Uses the owner’s Claude login")
    }

    func testAMemberWithNoNameIsNamedByTheirHandle() {
        let bare = PaneClaudeLogin(handle: "alex", name: " ", owner: false)
        XCTAssertEqual(bare.sentence(viewer: nil), "Uses alex’s Claude login")
    }

    func testItDecodesTheBridgesShapeAndAbsentIsNil() throws {
        struct Row: Decodable { var claudeLogin: PaneClaudeLogin? }
        let json = #"{"claudeLogin":{"handle":"alex","name":"Alex Rivera","owner":false}}"#
        XCTAssertEqual(try JSONDecoder().decode(Row.self, from: Data(json.utf8)).claudeLogin, alex)
        XCTAssertNil(try JSONDecoder().decode(Row.self, from: Data(#"{"claudeLogin":null}"#.utf8)).claudeLogin)
        XCTAssertNil(try JSONDecoder().decode(Row.self, from: Data("{}".utf8)).claudeLogin)
    }
}
