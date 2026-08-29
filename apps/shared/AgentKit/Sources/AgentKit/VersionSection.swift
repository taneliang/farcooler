import SwiftUI

/// What you are running, on both ends.
///
/// Two versions rather than one, because Far Cooler is two programs: the app in
/// your hand and the daemon on the runner doing the work. They are meant to
/// match — everything else in this project's versioning exists to make that
/// true — so the interesting case is the one where they do not, and that is the
/// case a person needs to be able to see without being told to run a command.
///
/// This is also the answer to "which build did you have?" on a bug report. A
/// beta and a release share a marketing version, so `0.2.0` alone is not an
/// answer; `0.2.0 (beta 3) · 1284` is.
public struct VersionSection: View {
    /// The ownership and license information shipped with every app build.
    ///
    /// Kept next to the version rather than in a platform-specific About view,
    /// so the Mac's About sheet, iOS Settings, and copied diagnostics all say
    /// the same thing.
    private static let copyrightNotice = "© 2026 E-Liang Tan · MIT License"

    /// What the daemon reported, or nil if nothing has asked it yet.
    private let daemon: DaemonBuild?
    /// Which machine that daemon is on, for when it is not this one. Blank
    /// means local — and the row says "Daemon" rather than naming a host,
    /// because "this machine" is the case that needs no explaining.
    private let host: String
    private let onCopy: (String) -> Void

    public init(daemon: DaemonBuild?, host: String = "", onCopy: @escaping (String) -> Void) {
        self.daemon = daemon
        self.host = host
        self.onCopy = onCopy
    }

    public var body: some View {
        Section {
            LabeledContent("App", value: AppVersion.display)
            LabeledContent("Build", value: AppVersion.build)
            LabeledContent("Copyright", value: Self.copyrightNotice)

            if let daemon {
                LabeledContent(host.isEmpty ? "Daemon" : "Daemon on \(host)", value: daemon.readable)
                if !daemon.matches {
                    // Said out loud rather than left to be noticed. A client and
                    // a daemon built from different source speak the protocol
                    // perfectly and still behave like two programs — and the
                    // symptom is a bug you already fixed still happening.
                    Label(
                        "This app and that runner were built from different source. "
                            + "Reinstall on the runner to match.",
                        systemImage: "exclamationmark.triangle.fill"
                    )
                    .font(.caption)
                    .foregroundStyle(.orange)
                }
            }

            // One tap, because the moment this matters is when someone is
            // typing it into a bug report and would otherwise transcribe a
            // commit hash by hand.
            Button("Copy version details") { onCopy(details) }
        } header: {
            Text("Version")
        }
    }

    private var details: String {
        var lines = [
            "Far Cooler \(AppVersion.display)",
            "build \(AppVersion.build) · \(AppVersion.channel)",
            Self.copyrightNotice,
        ]
        if let daemon {
            // The RAW stamp here, not the readable one. This string is going
            // into a bug report, and the raw form is what someone can compare
            // against a build without having to un-prettify it first.
            let where_ = host.isEmpty ? "" : " on \(host)"
            lines.append("daemon\(where_) \(daemon.version)\(daemon.matches ? "" : " (MISMATCH)")")
            if !daemon.platform.isEmpty { lines.append("platform \(daemon.platform)") }
        }
        return lines.joined(separator: "\n")
    }
}

/// What the daemon on the other end said about itself.
public struct DaemonBuild: Equatable, Sendable {
    public let version: String
    /// Whether it was built from the same source as this client. The daemon
    /// answers this rather than the app deriving it, because the daemon is the
    /// one that knows both stamps.
    public let matches: Bool
    public let platform: String
    /// What that runner can do, by name.
    ///
    /// Distinct from `matches`, and they answer different questions. That one
    /// is "were these built from the same source"; this is "what can that
    /// runner do", which is the one an app acts on when it is newer than the
    /// runner it reached — App Store review takes days, `host install` takes
    /// one command, and those clocks do not tick together.
    ///
    /// Empty means a daemon old enough to predate the question. See
    /// `can(_:)`, which reads that as the features that existed then rather
    /// than as a runner that can do nothing.
    public let capabilities: Set<String>

    /// What THIS connection may ask that runner for, as `authorized_keys`
    /// spells it: `read`, `control`, `host_admin`, or `unspecified`.
    ///
    /// The pair to `capabilities` and not a substitute for it — a capability is
    /// what the runner can serve, this is what this connection may ask it for,
    /// and a control needs both. It rides the handshake, so it is known before
    /// the first request; see `Session::granted_scope` in
    /// `crates/client/src/session.rs`.
    ///
    /// A fact about the CONNECTION, not about the daemon, which is why it is
    /// read fresh on every connect rather than cached: sshd applies
    /// `authorized_keys` once at authentication, so this is fixed for the life
    /// of the session and a re-enrollment does not reach it.
    ///
    /// `unspecified` is "no answer", NEVER "no permission" — see
    /// `mayAdministerRunner`.
    public let grantedScope: String

    public init(
        version: String, matches: Bool, platform: String, capabilities: Set<String> = [],
        grantedScope: String = "unspecified"
    ) {
        self.version = version
        self.matches = matches
        self.platform = platform
        self.capabilities = capabilities
        self.grantedScope = grantedScope
    }

    /// Whether this connection may ask for the calls that change the runner
    /// itself — its watched folders, its branch prefix, its themes, its agents.
    ///
    /// `can(_:)`'s sibling, and read the same way: a control this answers
    /// `false` for is shown DIMMED with a reason, never hidden. Hiding it would
    /// make the same app look different on two runners for no stated cause, and
    /// leaving it live would teach people that buttons sometimes do nothing.
    ///
    /// Written as a list of the grants known to be NARROWER than what those
    /// calls need, rather than as `grantedScope == "host_admin"`, and that
    /// direction is the whole point. `unspecified` is what a runner NEWER than
    /// this build answers when it names a grant this build has no word for —
    /// reading that as a refusal would let a new runner silently strip controls
    /// off an older app. So anything this build cannot recognize keeps offering
    /// exactly what it offers today, which is the shape `can(_:)` already has
    /// for capabilities.
    ///
    /// **The Mac never reaches `false` here, and that is not an oversight.** It
    /// talks to a runner through the CLI over its own plain shell key, which
    /// carries no forced command, so `Session::granted` in
    /// `crates/daemon/src/main.rs` reads it as host_admin — and `DaemonBuild`
    /// on that side is built without this field at all, which defaults to
    /// `unspecified` and lands in the same place. There is nothing to dim on a
    /// Mac. Only the two phones enroll at `control`.
    public var mayAdministerRunner: Bool {
        grantedScope != "read" && grantedScope != "control"
    }

    /// Whether this runner can do something, by name.
    ///
    /// A control whose capability is missing is shown DIMMED with a reason,
    /// never hidden: the same app showing different controls on two runners
    /// with nothing said about why reads as a bug, and this app already tells
    /// you when a runner has no Far Cooler installed rather than omitting it.
    public func can(_ capability: String) -> Bool {
        // A daemon that answered nothing predates capabilities entirely, so it
        // has exactly the feature set that existed then — workspaces and
        // terminals. Treating silence as "can do nothing" would blank the UI
        // against every daemon older than this change.
        if capabilities.isEmpty { return capability == "workspaces" || capability == "terminals" }
        return capabilities.contains(capability)
    }

    /// The same build, spelled the way the app spells its own.
    ///
    /// The daemon reports `0.1.0+d8c3877-dirty` while the app says
    /// `0.1.0 (dev d8c3877)` — the same information in two idioms, which makes
    /// two rows sitting next to each other look like two different builds. The
    /// raw form is still what "Copy version details" writes, because that is
    /// what someone compares against.
    public var readable: String {
        let parts = version.split(separator: "+", maxSplits: 1)
        guard parts.count == 2 else { return version }
        let marketing = String(parts[0])
        var detail = String(parts[1])
        var suffix = ""
        if detail.hasSuffix("-dirty") {
            detail.removeLast("-dirty".count)
            suffix = ", uncommitted"
        }
        return "\(marketing) (\(detail)\(suffix))"
    }
}
