import Foundation

/// Putting a device's key into a runner's `~/.ssh/authorized_keys`.
///
/// **The daemon owns the write**, on every runner including this Mac. Not a
/// shell command appending a line: that file is the one whose corruption costs
/// somebody SSH access to their own machine, so the write is
/// descriptor-anchored, `O_NOFOLLOW`, locked, atomic, `fsync`ed twice and
/// backed up — `crates/daemon/src/enrollment.rs` over `crates/daemon/src/fence.rs`.
/// The app's part is to ask.
///
/// ## What this cannot do yet
///
/// Asking is the part that has no route from a Mac. The daemon serves
/// `client.enroll`, `client.list` and `client.revoke`, but:
///
/// - the CLI, which is how this app reaches every runner, has no `client`
///   subcommand — so `farcooler --runner you@box client enroll …` is not a
///   command;
/// - the client core's `farcooler_client_call` dispatch in
///   `crates/client/src/ffi.rs` has no arm for those three methods either, so
///   the phone cannot reach them any more than the Mac can.
///
/// The CLI is the right route for a Mac and the wrong one for a phone, for the
/// same reason each: the CLI runs real `ssh`, so it gets the agent, the
/// passphrase prompt, `ProxyJump` and everything else a person has already set
/// up — and a phone has no `ssh` at all. So both surfaces are wanted, and this
/// enum is the Mac's half, ready for the subcommand:
///
/// ```
/// farcooler --json --runner you@box client enroll --key <public key> \
///     --label "iPhone 17" --client-id <uuid> --scope control
/// ```
///
/// Until that exists this reports what it could not do, in a sentence, and the
/// ceremony still shows the reply — because the reply is what the new device
/// needs, and a runner that was asleep is already an ordinary outcome the
/// manifest carries as `pending`.
///
/// ## And Key B cannot be enrolled by this call at all
///
/// Key A's line is `restrict,command="farcoolerd --stdio …"`, which is what
/// `ClientEnroll` writes and the ONLY thing it writes — the message has
/// deliberately no field for options, a forced command, or a key written as it
/// arrived, and the absence of a way to ask for anything else is the guard
/// rail.
///
/// Key B's line is plain. That is the whole point of it: a forced command means
/// sshd runs that program and only that program, so a Key B carrying one could
/// not open a shell and Zed would still be locked out. So enrolling Key B needs
/// something `ClientEnroll` does not have — a scope, or a message, that means
/// "a plain line, and this app owns it" — and that is a decision for the
/// daemon's own surface rather than something to smuggle past the guard rail
/// from Swift. Until it exists, this enrolls Key A and the Mac's shell access
/// is the manual path: its public key, appended by whoever already has one.
enum Enrollment {
    /// Enroll one key on each runner, and answer with a transcript for the
    /// screen. Nil when everything worked.
    ///
    /// One runner at a time rather than concurrently: these are writes to the
    /// same kind of file on different machines, and a transcript that
    /// interleaves is a transcript nobody can read. The list is short.
    static func enroll(
        key: String, label: String, clientID: String, scope: String, on runners: [CeremonyRunner]
    ) async -> String? {
        var failures: [String] = []
        for runner in runners {
            let result = await CLI.run([
                "--json", "--runner", runner.id, "client", "enroll",
                "--key", key, "--label", label, "--client-id", clientID, "--scope", scope,
            ])
            if !result.ok { failures.append("\(runner.label): \(result.output)") }
        }
        guard !failures.isEmpty else { return nil }
        return failures.joined(separator: "\n\n")
    }

    /// The sentence shown when some runners could not be written to.
    ///
    /// It does not say why. From here the cause is not knowable — a runner
    /// asleep, an ssh key that is not loaded, a daemon that is not installed
    /// and a fence that is damaged all look identical — and a screen that
    /// guesses is how an app ends up telling somebody to loosen an sshd setting
    /// that was never the problem.
    static let couldNotReachAll =
        "Some runners didn't take the key. They'll be added when this Mac next reaches them."
}
