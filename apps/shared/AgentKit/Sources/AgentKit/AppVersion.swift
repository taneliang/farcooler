import Foundation

/// What this build is, in the one form everything else quotes.
///
/// Three audiences, one source. A person reading Settings wants `0.2.0 (beta 3)`.
/// The relay wants something it can compare across a fleet and show back as
/// "this Mac is two versions behind". A bug report wants both, unambiguously —
/// which is the whole reason the channel exists at all: a beta and a release
/// that call themselves `0.2.0` make a report impossible to act on.
///
/// Read from the bundle rather than compiled in, because `build-app.sh` and
/// `generate-project.py` stamp it from `scripts/version.sh` at build time and
/// a constant here would be a second place to forget.
public enum AppVersion {
    /// `0.2.0` — the semver, matching the daemon's and the CLI's.
    public static var marketing: String {
        string("CFBundleShortVersionString") ?? "0.0.0"
    }

    /// The commit count. Monotonic, and it names the commit it came from.
    public static var build: String {
        string("CFBundleVersion") ?? "0"
    }

    /// `local`, `canary`, `preview`, or `stable`.
    public static var channel: String {
        let value = string("FarCoolerChannel") ?? ""
        // A build with nothing stamped is one somebody made by hand, which is a
        // local build whatever it says. Defaulting the other way would let an
        // unstamped build pass itself off as a release.
        return value.isEmpty ? "local" : value
    }

    public static var isRelease: Bool { channel == "stable" }

    /// What a person is shown: `0.2.0`, `0.2.0 (beta 3)`, `0.2.0 (dev a1b2c3)`.
    public static var display: String {
        if let stamped = string("FarCoolerDisplayVersion"), !stamped.isEmpty { return stamped }
        // Reconstructed rather than blank, so a bundle that missed the stamp
        // still says something true.
        return channel == "stable" ? marketing : "\(marketing) (\(channel))"
    }

    /// What the relay is told, and what it shows back on the devices screen.
    ///
    /// One field rather than three, because the relay's job here is to let
    /// someone glance at a list and see which runner is behind — not to hold a
    /// version model of its own.
    public static var reported: String { "\(display) · \(build)" }

    private static func string(_ key: String) -> String? {
        Bundle.main.object(forInfoDictionaryKey: key) as? String
    }
}
