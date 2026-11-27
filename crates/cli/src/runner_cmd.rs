use std::path::PathBuf;

use clap::Subcommand;

use crate::runner_proof;

#[derive(Subcommand)]
pub(crate) enum RunnerCmd {
    /// Copy the daemon and CLI to a Linux host and register a user service.
    Install {
        target: String,
        /// The Linux binaries to install. Defaults to ./dist/<arch>-linux/.
        #[arg(long)]
        from: Option<PathBuf>,
    },
    /// Update this Mac's standalone canary runner.
    #[cfg(target_os = "macos")]
    Update {
        /// Wait for in-flight landing checks and pushes to finish.
        #[arg(long)]
        wait: bool,
    },
    /// Activate an already verified standalone runner payload.
    #[cfg(target_os = "macos")]
    #[command(hide = true)]
    StandaloneActivate {
        #[arg(long)]
        build: u64,
        #[arg(long)]
        payload: PathBuf,
        #[arg(long)]
        migrate_app: Option<PathBuf>,
        #[arg(long)]
        wait: bool,
    },
    /// Report what is installed and running on a runner.
    Status { target: String },
    /// Report what a runner IS, changing nothing on it.
    ///
    /// What the Mac app asks before offering to install: the platform, whether
    /// tmux is there, what would keep the daemon alive, and what is already
    /// installed.
    Probe { target: String },
    /// Carry one SSH session to a tunneled runner over stdin and stdout.
    ///
    /// Not a command a user types. It is what `~/.ssh/config` names in
    /// `ProxyCommand` for a runner that has a tunnel token rather than an
    /// address, so Zed, git and plain `ssh` reach it with no address anywhere.
    /// The Mac app already writes that line — `SshConfig.swift` — so the
    /// spelling of these three words is a contract with a file on somebody's
    /// disk. See `runner_pipe.rs`.
    ///
    /// Takes an id and never a token: an argument list is visible to every
    /// process on the machine, and `~/.ssh/config` is readable by every tool
    /// on it.
    Pipe {
        /// The runner id from the ProxyCommand line.
        id: String,
    },
    /// Make this machine's tunnel identity and print it as JSON.
    ///
    /// Writes nothing: the Mac app keeps the private half in its Keychain and
    /// then hands it to `store-node-key`. Prints `{"private_key","public_key"}`,
    /// or `{"error":"no_tailcat"}` from a build with no tunnel linked.
    #[command(hide = true)]
    MintNodeKey,
    /// Keep the private tunnel key read from stdin where `runner pipe` finds it.
    ///
    /// Stdin and never an argument, because an argument list is visible to every
    /// process on the machine. Writes `runners.json` owner-only and leaves the
    /// runners already in it alone.
    #[command(hide = true)]
    StoreNodeKey,
    /// Keep a first-key code's hash where the daemon reads it (ov-988), and
    /// print `kept`, `waiting` (one was made before) or `used`.
    ///
    /// What `runner install` runs on the runner. Never replaces a code that
    /// was kept, so no command here makes a second one; the code itself is
    /// only ever on the computer the installer ran on.
    #[command(hide = true)]
    KeepFirstKey { hash: String },
    /// Keep the tunneled runners read from stdin where `runner pipe` finds them.
    ///
    /// Stdin is `[{"id","token"}]`. Writes `runners.json` owner-only and leaves
    /// the key and the other runners in it alone.
    #[command(hide = true)]
    StoreRunners,
    /// Drop the runner ids read from stdin (`["id"]`) from where `runner pipe`
    /// finds them, token and host key with them.
    #[command(hide = true)]
    ForgetRunners,
    /// Print the host key fingerprint a tunneled runner was granted with, for
    /// the Mac app's conversation view, which checks the key its own session
    /// is shown against it. Fails when there is none on file.
    #[command(hide = true)]
    PinnedKey { id: String },
    /// `ssh`'s `KnownHostsCommand` for a tunneled runner: prints the offered
    /// host key only when its fingerprint is the one that runner was granted with.
    #[command(hide = true)]
    KnownHost {
        id: String,
        /// `%I`: why ssh is asking. Unused; only a key can be checked.
        reason: String,
        /// `%H`: the name ssh is looking up, echoed back on the line.
        host: String,
        /// `%t` and `%K`: absent when ssh is only choosing algorithms.
        key_type: Option<String>,
        key: Option<String>,
    },
    /// Sign a request to put this runner in a team, for the Mac app.
    ///
    /// Signs with the key of the runner it runs on, so with `--runner` it runs
    /// there over ssh and the box signs. Prints the proof as JSON; the private
    /// key is never printed.
    #[command(hide = true)]
    SignRegistration(runner_proof::ProofArgs),
    /// Take this runner over: sign a claim to move it to another account's team.
    ///
    /// Anyone who can run as this runner's Unix user can do this, and that is
    /// the trust boundary (docs/runners.md). With `--runner` it runs there over
    /// ssh, as `sign-registration` does. Prints the proof as JSON for the
    /// relay's `runners/claim`.
    Claim(runner_proof::ProofArgs),
}
