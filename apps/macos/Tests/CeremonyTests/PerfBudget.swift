import Foundation

/// A timing gate's limit, by where it runs (ov-703).
///
/// The FARCOOLER_PERF gates' limits were set on an M-series Mac. The
/// nightly's `xcode-27` runner is slower and noisier: on its first night
/// (run 39809392286, Oct 22) `flingKeepsUp`'s four flings hitched 52.8 to
/// 71.9 ms/s, with the main thread's share 31.4 to 50.4 and a median of
/// 48.6, against a budget of 40 that this Mac meets at 12 to 17. A budget
/// that flaps teaches everyone to ignore the job. So the runner (`CI` set)
/// gets its own limit, wide enough to clear that band with margin and
/// tight enough to catch a regression that doubles the cost. Each gate
/// prints what it measured either way, so the nightly log keeps a history
/// to tighten the runner's limit by.
enum PerfBudget {
    /// Whether this is a CI runner: GitHub sets `CI=true`.
    static var onRunner: Bool { ProcessInfo.processInfo.environment["CI"] == "true" }

    /// `local` here, `runner` on CI.
    static func limit(local: Double, runner: Double) -> Double { onRunner ? runner : local }

    /// For the log: the limit in force and where.
    static func describe(_ limit: Double, unit: String) -> String {
        String(format: "budget %.1f %@ (%@)", limit, unit, onRunner ? "CI runner" : "local")
    }
}
