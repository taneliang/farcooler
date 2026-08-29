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

@Test func aNarrowerGrantIsWhatDimsARunnerControl() {
    // The two grants that are genuinely narrower than host_admin.
    let read = DaemonBuild(
        version: "1.0.0", matches: true, platform: "linux", grantedScope: "read")
    let control = DaemonBuild(
        version: "1.0.0", matches: true, platform: "linux", grantedScope: "control")
    #expect(!read.mayAdministerRunner)
    #expect(!control.mayAdministerRunner)

    let admin = DaemonBuild(
        version: "1.0.0", matches: true, platform: "linux", grantedScope: "host_admin")
    #expect(admin.mayAdministerRunner)
}

@Test func anUnrecognizedGrantKeepsEveryControlOffered() {
    // "No answer", never "no permission" — the distinction the whole predicate
    // turns on, and the reason it is written as a deny-list of the two narrow
    // grants rather than as `== "host_admin"`.
    //
    // `unspecified` is what a runner too old to name a scope sends, and what a
    // NEWER runner naming a scope this build has no word for looks like from
    // here. Reading either as a refusal would let a new runner silently strip
    // controls off an older app — a regression that would arrive without
    // anybody changing this app at all.
    let silent = DaemonBuild(version: "1.0.0", matches: true, platform: "linux")
    #expect(silent.grantedScope == "unspecified")
    #expect(silent.mayAdministerRunner)

    let fromTheFuture = DaemonBuild(
        version: "9.0.0", matches: false, platform: "linux",
        grantedScope: "some_scope_this_build_has_never_heard_of")
    #expect(fromTheFuture.mayAdministerRunner)
}
