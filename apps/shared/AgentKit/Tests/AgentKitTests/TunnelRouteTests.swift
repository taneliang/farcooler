import Foundation
import Testing

@testable import AgentKit

/// The owner's route rule (ov-485): the tunnel first, the address only when a
/// person asks for it, and the question asked only when it can help.
struct TunnelRouteTests {
    /// A failed tunnel with an address beside it asks, in the owner's words.
    @Test func aFailedTunnelWithAnAddressAsks() throws {
        let offer = try #require(
            TunnelRoute.offer(onTheTunnel: true, failedBeforeTheRunner: true, alternative: "192.168.1.180"))
        #expect(offer.title == "Connecting through Tailcat failed.")
        #expect(offer.question == "Connect directly to 192.168.1.180 instead?")
        #expect(offer.sentence == "Connecting through Tailcat failed. Connect directly to 192.168.1.180 instead?")
    }

    /// Never asked about a runner already on its address, about one with no
    /// address to offer, or after a failure the address would meet too.
    @Test func nothingIsOfferedWhereItCannotHelp() {
        #expect(TunnelRoute.offer(onTheTunnel: false, failedBeforeTheRunner: true, alternative: "a") == nil)
        #expect(TunnelRoute.offer(onTheTunnel: true, failedBeforeTheRunner: true, alternative: nil) == nil)
        #expect(TunnelRoute.offer(onTheTunnel: true, failedBeforeTheRunner: true, alternative: "") == nil)
        #expect(TunnelRoute.offer(onTheTunnel: true, failedBeforeTheRunner: false, alternative: "a") == nil)
    }

    /// Which failures are the runner never answering. The tunnel's own words,
    /// a dial nobody answered, no tunnel key and giving up all are; a refusal
    /// from sshd or the daemon is not, because the address meets the same one.
    @Test func onlyAFailureBeforeTheRunnerOffersTheAddress() {
        let before: [RunnerTrouble] = [
            .tunnelFailed(.noAnswer), .tunnelFailed(.rendezvous), .tunnelFailed(.unspecified),
            .tunnelFailed(.notInThisBuild), .unreachable, .noNodeKey, .stopped,
        ]
        let after: [RunnerTrouble] = [
            .keyRejected, .hostKeyChanged, .daemonMissing, .noIdentity, .keyNotTrusted, .other,
        ]
        for trouble in before { #expect(trouble.failedBeforeTheRunner, "\(trouble)") }
        for trouble in after { #expect(!trouble.failedBeforeTheRunner, "\(trouble)") }
    }

    /// One route at a time, named, with the move to the other.
    @Test func theRouteIsNamedWithTheMoveToTheOther() {
        #expect(TunnelRoute.label(.tunnel) == "Through Tailcat")
        #expect(TunnelRoute.label(.direct(address: "mini.local")) == "Directly to mini.local")
        #expect(TunnelRoute.switchLabel(from: .tunnel) == "Connect Directly")
        #expect(TunnelRoute.switchLabel(from: .direct(address: "x")) == "Use Tailcat")
        #expect(TunnelRoute.address(host: "mini.local", port: 22) == "mini.local")
        #expect(TunnelRoute.address(host: "mini.local", port: 2222) == "mini.local:2222")
    }
}
