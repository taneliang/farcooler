import Foundation
import Testing

@testable import Far_Cooler

/// A Tailcat runner a person moved to its address is still held to the
/// granter's pin in the conversation view (ov-485 fix 1, M4): the settings
/// `ssh -G` prints for the entry `RouteSwitch` writes resolve to the pin, not
/// to `known_hosts`, which for that entry is `/dev/null`.
struct RemoteReachAddressRouteTests {
    /// `ssh -G`'s keywords for an address-route entry, lowercased as it prints them.
    private let addressRoute: [String: String] = [
        "hostname": "192.168.1.180", "port": "22", "user": "you",
        "hostkeyalias": "farcooler-fc-e4f8501ee01bcae9",
        "userknownhostsfile": "/dev/null", "globalknownhostsfile": "/dev/null",
    ]

    @Test func theAddressRouteIsHeldToTheGrantersPin() {
        guard case .success(let destination) = RemoteReach.destination(addressRoute, cli: "/x/farcooler") else {
            Issue.record("the address route didn't resolve")
            return
        }
        #expect(destination.pipe == nil, "dialed through a pipe on the address route")
        #expect(
            RemoteReach.pin(addressRoute, pipe: destination.pipe)
                == .granted(id: "fc-e4f8501ee01bcae9", pipe: nil))
    }

    /// A runner the owner added by hand keeps `known_hosts`, and an alias that
    /// isn't a plain id is never taken as one.
    @Test func anyOtherRunnerUsesKnownHosts() {
        var plain = addressRoute
        plain["hostkeyalias"] = nil
        #expect(RemoteReach.pin(plain, pipe: nil) == .knownHosts)
        var hostile = addressRoute
        hostile["hostkeyalias"] = "farcooler-$(id)"
        #expect(RemoteReach.pin(hostile, pipe: nil) == .knownHosts)
    }
}
