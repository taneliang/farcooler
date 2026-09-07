import Foundation

/// Which of the runners a ceremony granted this device can actually write a key
/// into.
///
/// # Why this is a function and not two lines at the call site
///
/// The call site is `CeremonyStore.throughTheLiveConnection`, and what it does
/// with this answer is call `client.enroll` — which asks a daemon to append a
/// public key to `~/.ssh/authorized_keys`. That is the most consequential write
/// this app makes. It had one rule while the phone held one connection, and the
/// rule was invisible because the shape of the code enforced it: there was one
/// session, so at most one runner could be written to, so the intersection was
/// never computed anywhere.
///
/// With a connection per runner the intersection is real, and both sides of it
/// matter:
///
/// - **Granted but not live** is ordinary and must be silent. A phone is not
///   asked to reach every runner in a manifest; `confirm()` marks the rest
///   pending, which is exactly "the trusted device has not yet written this key
///   into that runner's `authorized_keys`".
/// - **Live but not granted must never be written to.** This is the direction
///   with a cost. The ceremony's manifest is the authorization; a live
///   connection is merely a session that happens to exist. Enrolling a device
///   into a runner nobody granted would put somebody's key on a machine the
///   ceremony said nothing about, and nothing downstream would report it.
///
/// # Why it lives here
///
/// `swift test --package-path apps/shared/AgentKit` executes this package
/// (`.github/workflows/ci.yml`). The iOS target's suite is a UI suite that CI
/// compiles and never runs, so a rule about `authorized_keys` left in
/// `CeremonyStore` is a rule with nothing on it at all.
public enum CeremonyReach {
    /// The granted runners this device has a live connection to, in the order
    /// the manifest listed them.
    ///
    /// Manifest order rather than connection order, so what a screen reports
    /// reads in the order somebody ticked the rows. Duplicates in either list
    /// collapse: a runner is written to once, and an id listed twice is one
    /// runner — the same rule `FleetMembership.plan` states about a runner
    /// appearing twice in a reconcile.
    public static func writable(granted: [String], live: [String]) -> [String] {
        let reachable = Set(live)
        var seen: Set<String> = []
        return granted.filter { reachable.contains($0) && seen.insert($0).inserted }
    }
}
