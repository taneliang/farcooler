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

    // MARK: - What a phone grants

    private enum R: Equatable { case address(String), tunnel(String) }
    private func grant(_ reach: R, kept: R? = nil, token: String = "", key: Bool = true) -> (R, R?) {
        let granted = TunnelRoute.grant(
            reach: reach, kept: kept, token: token, deviceHasNodeKey: key,
            isTunnel: { if case .tunnel = $0 { return true } else { return false } }, tunnel: { .tunnel($0) })
        return (granted.reach, granted.alternative)
    }

    /// Tunnel first: a runner that answered with a token goes through it, its
    /// address beside it.
    @Test func aRunnerThatAnsweredWithATokenIsGrantedTheTunnelFirst() {
        let (reach, beside) = grant(.address("mini.local"), token: "t")
        #expect(reach == .tunnel("t"))
        #expect(beside == .address("mini.local"))
    }

    /// No token, no tunnel: the address alone, and never a token this phone
    /// merely holds, which says nothing about the new device's key.
    @Test func noTokenKeepsTheAddressAlone() {
        let (reach, beside) = grant(.address("mini.local"), kept: .tunnel("held"))
        #expect(reach == .address("mini.local"))
        #expect(beside == nil)
    }

    /// A runner this phone reaches through the tunnel stays there, with the
    /// address this phone kept beside it.
    @Test func aTunneledRunnerStaysOnTheTunnelWithItsAddress() {
        let (reach, beside) = grant(.tunnel("its"), kept: .address("mini.local"), token: "fresh")
        #expect(reach == .tunnel("its"))
        #expect(beside == .address("mini.local"))
    }

    /// A device with no node key can't dial a tunnel, so a tunneled runner
    /// with an address goes as that address; with none, as before.
    @Test func aDeviceWithNoNodeKeyGetsTheAddress() {
        #expect(grant(.tunnel("its"), kept: .address("mini.local"), key: false) == (.address("mini.local"), nil))
        #expect(grant(.tunnel("its"), key: false) == (.tunnel("its"), nil))
        #expect(grant(.address("mini.local"), token: "t", key: false) == (.address("mini.local"), nil))
    }

    /// Several runners failing together are one question, naming each.
    @Test func severalRunnersAreOneQuestion() {
        #expect(DirectRouteOffer.question(for: []) == nil)
        #expect(DirectRouteOffer.question(for: [("cosmo", "10.0.0.4")]) == "Connect directly to 10.0.0.4 instead?")
        #expect(
            DirectRouteOffer.question(for: [("cosmo", "10.0.0.4"), ("mini", "mini.local"), ("box", "fe80::1")])
                == "Connect directly to cosmo at 10.0.0.4, mini at mini.local, and box at fe80::1 instead?")
    }
}
