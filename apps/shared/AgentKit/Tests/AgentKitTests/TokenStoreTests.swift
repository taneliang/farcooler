import Testing

@testable import AgentKit

/// Which keychain slot a session lives in (ov-501).
///
/// On the Mac every channel's app wrote one login-keychain item, so Far Cooler
/// Local's session became Far Cooler Canary's, and the canary relay refused it
/// with "HTTP 401 after refresh". These pin the partition and the one-time
/// cleanup without touching the real Keychain.
struct TokenStoreTests {
    /// Stable keeps the name it shipped with; every other channel has its own.
    @Test func eachChannelHasItsOwnSlot() {
        let names = ["stable", "preview", "canary", "local"].map { TokenStore.service(for: $0) }
        #expect(Set(names).count == 4)
        #expect(TokenStore.service(for: "stable") == TokenStore.sharedService)
        #expect(TokenStore.service(for: "canary") == "com.farcooler.account.canary")
    }

    /// The slot this process actually uses. The test bundle is unstamped, so
    /// on the Mac it is a local build, and a local build must not read the
    /// shared slot — this is the line that fails if the call site goes back to
    /// the one name.
    @Test func aMacBuildReadsItsChannelsSlot() {
        #if os(macOS)
            #expect(AppVersion.channel == "local")
            #expect(TokenStore.service == "com.farcooler.account.local")
        #else
            #expect(TokenStore.service == TokenStore.sharedService)
        #endif
    }

    /// Labels for a session the new slot never held are cleared; nothing else is.
    @Test func onlyLabelsWithNoSessionBehindThemAreForgotten() {
        let canary = TokenStore.service(for: "canary")
        #expect(Account.labelsOutliveTheirSession(service: canary, signedIn: true, holdsSession: false))
        #expect(!Account.labelsOutliveTheirSession(service: canary, signedIn: true, holdsSession: true))
        #expect(!Account.labelsOutliveTheirSession(service: canary, signedIn: false, holdsSession: false))
        // Stable's slot is the shared one, and its session is its own.
        #expect(
            !Account.labelsOutliveTheirSession(
                service: TokenStore.sharedService, signedIn: true, holdsSession: false))
    }
}
