import Foundation

/// Which route a runner is reached by, and what is said when the tunnel fails
/// for one that also has an address (ov-485).
///
/// **The owner's rule, Oct 13: the tunnel first, the address only on request.**
/// A ceremony grants the tunnel and keeps the address beside it. Nothing ever
/// dials the address on its own: when the tunnel fails, the app says so and
/// asks, and only a person's answer moves the runner to its address. A runner
/// is reached one way at a time, and the screens say which.
///
/// Shared by the Mac and the iPhone, so the sentence and the buttons are one
/// thing; Android's `DirectRoute.kt` says the same words, and copy-lint reads
/// both. The rule for WHICH failures offer the address lives with each app's
/// failure vocabulary (`RunnerTrouble.failedBeforeTheRunner` on the phone), but
/// the offer itself is built only here.
public enum TunnelRoute {
    /// How a runner is reached right now.
    public enum Current: Sendable, Equatable {
        case tunnel
        case direct(address: String)
    }

    /// What a person reads for a route. "Tailcat", not "the tunnel": it is the
    /// name the owner's own sentence uses, and the one a person can search for.
    public static func label(_ route: Current) -> String {
        switch route {
        case .tunnel: return "Through Tailcat"
        case .direct(let address): return "Directly to \(address)"
        }
    }

    /// The route in a word, for a screen that already shows the address
    /// beside it (the iPhone's runner editor).
    public static func shortLabel(_ route: Current) -> String {
        switch route {
        case .tunnel: return "Tailcat"
        case .direct: return "Direct"
        }
    }

    /// The row's heading. One route at a time, so the row names one.
    public static let heading = "Route"

    /// The button that moves a runner to the other route. Title case.
    public static func switchLabel(from route: Current) -> String {
        switch route {
        case .tunnel: return "Connect Directly"
        case .direct: return "Use Tailcat"
        }
    }

    /// An address as a person reads it: the port only when it isn't 22, which
    /// is the port nobody types.
    public static func address(host: String, port: Int) -> String {
        port == 22 ? host : "\(host):\(port)"
    }

    /// What a PHONE grants for one runner it is sharing: the route the new
    /// device dials, and the one kept beside it.
    ///
    /// Generic because the phone's `Reach` is declared in each app, not here.
    /// The Mac decides in `Enrollment` with the same rule:
    ///
    /// - a runner that just answered `client.enroll` with a token is granted
    ///   through the tunnel, its address kept beside it;
    /// - a runner already on the tunnel stays there, keeping an address beside
    ///   it if this phone holds one;
    /// - a device with no node key can't be granted a tunnel, so a tunneled
    ///   runner with an address goes as that address alone (one with none
    ///   still goes as the tunnel, and the core refuses the reply, as before);
    /// - anything else keeps the address it has, with nothing beside it.
    ///
    /// A token this phone merely holds is never granted on its own: only the
    /// one the runner answered with now says the NEW device's key is admitted.
    public static func grant<Reach: Equatable>(
        reach: Reach, kept alternative: Reach?, token: String, deviceHasNodeKey: Bool,
        isTunnel: (Reach) -> Bool, tunnel: (String) -> Reach
    ) -> (reach: Reach, alternative: Reach?) {
        let address = [reach, alternative].compactMap { $0 }.first { !isTunnel($0) }
        if !deviceHasNodeKey {
            if isTunnel(reach), let address { return (address, nil) }
            return (reach, nil)
        }
        if isTunnel(reach) { return (reach, address) }
        if !token.isEmpty { return (tunnel(token), reach) }
        return (reach, nil)
    }

    /// The question asked when the tunnel failed, or nil when there is nothing
    /// to offer.
    ///
    /// `onTheTunnel` is the route the attempt just took; `failedBeforeTheRunner`
    /// is whether the failure is one where nothing on the runner answered — the
    /// tunnel's own failures, a dial nobody answered, no tunnel key, giving up
    /// waiting. A runner that answered and refused this device's key would
    /// refuse it at its address too, so offering the address there would send
    /// somebody down a road that ends at the same refusal.
    public static func offer(
        onTheTunnel: Bool, failedBeforeTheRunner: Bool, alternative: String?
    ) -> DirectRouteOffer? {
        guard onTheTunnel, failedBeforeTheRunner, let alternative, !alternative.isEmpty
        else { return nil }
        return DirectRouteOffer(address: alternative)
    }
}

/// "Connecting through Tailcat failed. Connect directly to <address> instead?"
///
/// The owner's sentence, close to word for word. Split in two because both
/// screens draw it as a title and a question: an alert on the Mac, a row on
/// the iPhone.
public struct DirectRouteOffer: Sendable, Equatable {
    public let address: String

    public init(address: String) { self.address = address }

    public var title: String { "Connecting through Tailcat failed." }
    public var question: String { "Connect directly to \(address) instead?" }
    /// The whole sentence, for a screen with room for one line of copy.
    public var sentence: String { "\(title) \(question)" }

    /// Choosing it moves the runner to its address until somebody moves it
    /// back. Said beside the question, because a choice that quietly stuck
    /// would be a route nobody remembers taking.
    public static let afterwards =
        "Far Cooler will connect to it directly from now on. You can switch back to Tailcat in its settings."

    /// The same, for several runners at once.
    public static let afterwardsForSeveral =
        "Far Cooler will connect to them directly from now on. You can switch back to Tailcat in each one’s settings."

    /// The question for several runners that failed together, as after a wake
    /// or a network change: one question, naming each runner and its address.
    /// Nil for none; the owner's own sentence for one.
    public static func question(for runners: [(name: String, address: String)]) -> String? {
        switch runners.count {
        case 0: return nil
        case 1: return DirectRouteOffer(address: runners[0].address).question
        default:
            let named = runners.map { "\($0.name) at \($0.address)" }
            let list =
                named.count == 2
                ? "\(named[0]) and \(named[1])"
                : named.dropLast().joined(separator: ", ") + ", and " + named[named.count - 1]
            return "Connect directly to \(list) instead?"
        }
    }

    public static let connectDirectly = "Connect Directly"
    public static let notNow = "Not Now"
}
