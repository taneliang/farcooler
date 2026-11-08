import Testing
@testable import AgentKit

/// What a phone says it is at pairing (ov-989).
struct PhoneModelTests {
    @Test func aKnownIdentifierIsNamedAndAnUnknownOneIsSaidAsItIs() {
        #expect(PhoneModel.name(forIdentifier: "iPhone16,1") == "iPhone 15 Pro")
        #expect(PhoneModel.name(forIdentifier: "iPhone99,9") == "iPhone99,9")
        #expect(!PhoneModel.current().isEmpty)
    }
}
