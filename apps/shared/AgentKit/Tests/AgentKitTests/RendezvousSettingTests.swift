import Foundation
import Testing

@testable import AgentKit

/// The setting that exists for one day: the day the rendezvous service every
/// tunneled connection meets at stops answering.
///
/// It is a recovery valve, so what it must never do is fail quietly. A value
/// this accepted and the tunnel then dropped would produce a device that looks
/// configured and reaches nothing — and a tunnel with no rendezvous does not
/// refuse, it times out, which is also what a revoked device, a dead relay and
/// a runner that never started look like. So the decision about what is usable
/// is made once, here, and the screen asks the same function before it enables
/// its button.
@MainActor
struct RendezvousSettingTests {
    /// Empty is the default, and empty must stay empty all the way down.
    ///
    /// The tempting bug is a helpful one: filling in the URL the app ships
    /// with, so the field is never blank. That reads as a convenience and is
    /// the opposite — it writes the exact address this setting exists to move
    /// off into the one place nobody would look for it, and it goes on working
    /// right up until the day it matters.
    @Test func nothingTypedMeansTheAppsOwnRendezvous() {
        #expect(Account.derpMapSetting("") == "")
        #expect(Account.derpMapSetting("   ") == "")
        #expect(Account.derpMapSetting("\n") == "")
    }

    /// A rendezvous somebody deliberately typed is kept exactly as typed.
    @Test func aRendezvousSomebodyTypedIsKept() {
        #expect(
            Account.derpMapSetting("https://derp.example/derpmap.json")
                == "https://derp.example/derpmap.json")
        // Surrounding whitespace is a paste, not a decision.
        #expect(
            Account.derpMapSetting("  https://derp.example/derpmap.json\n")
                == "https://derp.example/derpmap.json")
    }

    /// Cleartext is refused, and this is the one refusal here that is about
    /// security rather than tidiness.
    ///
    /// A map fetched over `http` is a map anybody on the path can rewrite, and
    /// rewriting it moves both ends of a tunnel onto a rendezvous of the
    /// rewriter's choosing. That is the single thing this setting must not
    /// make possible, so it is refused rather than upgraded: a value we had to
    /// repair is a value nobody deliberately chose.
    @Test func aCleartextRendezvousIsRefused() {
        #expect(Account.derpMapSetting("http://derp.example/derpmap.json") == "")
        #expect(Account.derpMapSetting("ftp://derp.example/derpmap.json") == "")
        #expect(Account.derpMapSetting("derp.example/derpmap.json") == "")
        #expect(Account.derpMapSetting("https:///derpmap.json") == "")
    }

    /// Whitespace inside the value is refused rather than stripped.
    ///
    /// The tunnel library refuses a URL carrying a space, because one of its
    /// backends hands the URL to a subprocess over a line protocol whose
    /// fields are separated by spaces — so a second field would arrive there
    /// as a command nobody typed. A setting that saved and then silently did
    /// nothing is worse than one that would not save.
    @Test func aRendezvousCarryingASecondFieldIsRefused() {
        #expect(Account.derpMapSetting("https://derp.example/map.json allow bbbb") == "")
        #expect(Account.derpMapSetting("https://derp.example/a b.json") == "")
    }
}
