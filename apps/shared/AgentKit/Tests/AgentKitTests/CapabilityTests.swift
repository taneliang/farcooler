import Testing

@testable import AgentKit

/// What an app is allowed to assume about a runner it is newer than.
///
/// App Store review takes days; `farcooler host install` takes one command. A
/// phone weeks behind talking to a daemon updated this morning is the ordinary
/// case, not the exotic one, and these are the rules that make it survivable.

@Test func aRunnerThatNamesAFeatureCanDoIt() {
    let daemon = DaemonBuild(
        version: "0.1.0+abc", matches: true, platform: "linux",
        capabilities: ["workspaces", "terminals", "changes"])
    #expect(daemon.can("changes"))
    #expect(daemon.can("workspaces"))
}

@Test func aRunnerThatDoesNotNameAFeatureCannotDoIt() {
    // The whole point: a control whose capability is absent gets dimmed with a
    // reason rather than offered and failing.
    let daemon = DaemonBuild(
        version: "0.1.0+abc", matches: true, platform: "linux",
        capabilities: ["workspaces", "terminals"])
    #expect(!daemon.can("changes"))
    #expect(!daemon.can("stack"))
}

@Test func aDaemonTooOldToAnswerStillGetsItsOldFeatures() {
    // Silence means a daemon that predates capabilities entirely, so it has
    // exactly the feature set that existed then. Reading that as "can do
    // nothing" would blank the UI against every runner older than the change
    // that introduced the question — which is the opposite of the point.
    let ancient = DaemonBuild(version: "0.1.0+old", matches: false, platform: "linux")
    #expect(ancient.can("workspaces"))
    #expect(ancient.can("terminals"))
    #expect(!ancient.can("changes"))
    #expect(!ancient.can("stack"))
}

@Test func capabilitiesAreSeparateFromWhetherTheBuildsMatch() {
    // Two different questions. `matches` is "were these built from the same
    // source"; `can` is "what does that runner do". A runner can be a
    // different build and still do everything this app needs.
    let different = DaemonBuild(
        version: "0.9.0+other", matches: false, platform: "linux",
        capabilities: ["workspaces", "terminals", "changes", "stack"])
    #expect(!different.matches)
    #expect(different.can("stack"))
}
