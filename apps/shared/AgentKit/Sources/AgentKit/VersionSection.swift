import SwiftUI

/// What you are running, on both ends.
///
/// Two versions rather than one, because Far Cooler is two programs: the app in
/// your hand and the daemon on the machine doing the work. They are meant to
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
                        "This app and that machine were built from different source. "
                            + "Reinstall on the machine to match.",
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

    public init(version: String, matches: Bool, platform: String) {
        self.version = version
        self.matches = matches
        self.platform = platform
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
