//! `farcooler` command line interface.
//!
//! Two paths out of this process, and which one a command takes follows from
//! what it touches:
//!
//! - **Durable state** — roots, repositories, workspaces, terminal records —
//!   goes to the daemon over its Unix socket. One owner, so two commands
//!   running at once cannot both believe they are the authority, and so a
//!   second client can exist at all.
//!
//! - **Live runtime** — streaming, keystrokes, resize, screen capture — speaks
//!   to tmux directly. tmux is the authority for what is running and is safe
//!   for several readers, so serializing those through the daemon would buy
//!   nothing and would put a long-lived stream inside a request/response
//!   conversation where it does not belong.
//!
//! Every state printed here is DERIVED at the moment you ask. Nothing reads a
//! stored "running" flag, because none exists.

mod agent_host;
mod daemon_link;
mod remote;
mod runner_install;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod changes;
mod clients;
pub(crate) use daemon_link::{Link, connect_to, expect_value, req, req_for, with};
use farcooler_daemon::runtime::Runtime;
use farcooler_protocol::v1::{
    Repository, RepositoryRoot, Terminal, TerminalState, Workspace, WorkspaceState, request, result,
};
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "farcooler",
    // The BUILD STAMP, not `CARGO_PKG_VERSION`. Clap's bare `version` printed
    // "farcooler 0.1.0" — the same string in every build ever made — and two
    // things read this expecting to tell builds apart: `runner probe` captures it
    // from a remote runner, and `HostProbe.matchesThisMac` compares the two.
    // Comparing "0.1.0" to "0.1.0" always matched, so the "built from different
    // source than this Mac" warning could never fire. That warning is the
    // entire reason FARCOOLER_BUILD exists.
    version = farcooler_protocol::BUILD,
    about = "A terminal-first command center for parallel coding agents"
)]
struct Cli {
    /// Emit machine-readable JSON. The Mac app consumes this rather than
    /// scraping human output.
    #[arg(long, global = true)]
    json: bool,

    /// Operate on another runner, as `user@host` or an ssh config alias.
    ///
    /// Runs the same protocol over ssh. There is no Far Cooler network
    /// listener: a runner reachable by ssh is reachable by Far Cooler, and one
    /// that is not, is not.
    //
    // The alias is a plain comment, not a doc comment: `--host` is accepted
    // and hidden rather than removed — it lives in shell history and in
    // scripts, and a vocabulary change is not a reason to break either — but
    // saying so in `--help` would teach the word this rename is retiring. One
    // word is taught; both are understood.
    #[arg(long = "runner", alias = "host", global = true, value_name = "TARGET")]
    runner: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show runner and daemon facts.
    Status,
    /// Start, stop, or replace this runner's daemon.
    ///
    /// Local only. `--runner` reaches another runner's daemon through ssh, and
    /// stopping one from here would take away the connection carrying the
    /// request; installing and inspecting a remote daemon is `runner`'s job.
    #[command(subcommand)]
    Daemon(DaemonCmd),
    /// Manage allowlisted repository roots.
    #[command(subcommand)]
    Root(RootCmd),
    /// Manage registered repositories.
    #[command(subcommand)]
    Repo(RepoCmd),
    /// List the colour schemes available on this runner.
    #[command(subcommand)]
    Theme(ThemeCmd),
    /// Read and change what this runner's config.toml holds.
    ///
    /// The same writes the apps' runner-settings screens make, for scripting
    /// and for looking at what a screen actually did. Every write is
    /// format-preserving: comments and layout elsewhere in the file survive.
    #[command(subcommand)]
    Settings(SettingsCmd),
    /// Inspect and edit the ACP adapters that make chat mode possible.
    #[command(subcommand)]
    Adapter(AdapterCmd),
    /// Manage task workspaces (one worktree plus branch per task).
    #[command(subcommand)]
    Workspace(WorkspaceCmd),
    /// Manage terminals inside a workspace.
    #[command(subcommand)]
    Terminal(TerminalCmd),
    /// What a worktree changed.
    #[command(subcommand)]
    Changes(changes::ChangesCmd),
    /// Search a workspace's worktree files, for an agent chat's @-mention.
    #[command(subcommand)]
    Worktree(WorktreeCmd),
    /// Arrange terminals on screen: tile, zoom, focus, switch groups.
    ///
    /// Everything the Mac app's tiling does, because it is the same calls. An
    /// agent that can open a terminal but not place it is only half
    /// automatable, so this exists for agents as much as for people.
    #[command(subcommand)]
    Layout(LayoutCmd),
    /// Show how to attach to a workspace's live tmux session.
    Attach { workspace: String },
    /// Stream changes as they happen, one JSON object per line.
    ///
    /// A long-lived connection that prints only what changed. Clients used to
    /// poll, which forces a choice between noticing an agent's question late
    /// and spending a phone's battery asking constantly.
    Events,
    /// Pair this runner with your account so it can notify your devices.
    ///
    /// There is no sign-in here on purpose. The app does the signing in, asks
    /// the relay for a token that names nothing but the account, and hands that
    /// token to this runner over the ssh channel you already trust. So a
    /// headless Linux box never needs a browser, and the worst a leaked token
    /// can do is notify the phone of the person it was taken from.
    #[command(subcommand)]
    Push(PushCmd),
    /// Install or inspect a Far Cooler runner on a Linux host over ssh.
    //
    // `farcooler host …` still works, hidden, for the reason `--host` does:
    // it is in every runbook already written.
    #[command(subcommand, name = "runner", alias = "host")]
    RunnerCmd(RunnerCmd),
    /// Which devices may log in to a runner: list, enroll, revoke.
    ///
    /// The end of the enrollment ceremony and the only part of it that changes
    /// anything. Every one of these reads or writes the runner's own
    /// `~/.ssh/authorized_keys`, which is the authority on who may log in —
    /// nothing is cached anywhere else, so this and Settings › Devices cannot
    /// come to disagree about who has access.
    #[command(subcommand)]
    Client(clients::ClientCmd),
    /// Host a headless coding agent in this pane. Started by the daemon.
    ///
    /// Not a command a user types. It is the process a pane runs in agent pane
    /// mode, and it is a subcommand rather than a second binary so that shim
    /// and daemon can never be different versions.
    AgentHost {
        #[arg(long)]
        terminal: uuid::Uuid,
        #[arg(long)]
        socket: std::path::PathBuf,
        #[arg(long)]
        worktree: std::path::PathBuf,
        #[arg(long)]
        session: Option<String>,
        /// Which agent this pane hosts. The shim resolves it to an adapter
        /// through the same registry and config file the daemon used, so the
        /// two can never disagree about what a preset means — and so a program
        /// and its argument vector never have to survive tmux's shell quoting.
        #[arg(long)]
        preset: Option<String>,
    },
    /// Hold a pane open for a surface the client draws. Started by the daemon.
    ///
    /// Not a command a user types either. tmux has no concept of a pane without
    /// a process in it, so a pane whose contents are drawn by the app still
    /// needs something to own the rectangle — this is that something, and it
    /// does nothing but wait to be killed.
    PaneHost {
        /// What the pane is for. Only `changes` today; the argument exists so
        /// that the next such surface is a value here rather than a second
        /// subcommand that waits in a different way.
        #[arg(long, default_value = "changes")]
        kind: String,
    },
}

/// The local daemon's lifecycle, for whoever owns it.
///
/// On a Mac that is the app: it ships the daemon inside its own bundle and runs
/// `daemon ensure` at launch, so the pair that is talking is always the pair
/// that was built together. Elsewhere it is systemd or a person.
#[derive(Subcommand)]
enum DaemonCmd {
    /// Leave a daemon built from this same source running, replacing one that is not.
    Ensure,
    /// Stop the local daemon. Terminals keep running — they belong to tmux.
    Stop,
}

/// Tiling, in tmux's vocabulary — because it IS tmux.
///
/// A window is a layout and a pane is a terminal, so every one of these is a
/// tmux command with the identity bookkeeping done for you. The names are tmux's
/// on purpose: a great many people already know that `z` zooms and that a layout
/// is called `main-vertical`.
#[derive(Subcommand)]
enum LayoutCmd {
    /// Show a workspace's layouts and where tmux has put every pane.
    Show { workspace: String },
    /// Split a pane, running something in the new half.
    ///
    /// The arbitrary-split primitive: any edge of any pane, at any depth.
    Split {
        workspace: String,
        /// The pane to split. Defaults to the focused one.
        terminal: Option<String>,
        /// left, right, top, or bottom.
        #[arg(long, default_value = "right")]
        side: String,
        /// What to run. Defaults to your shell.
        #[arg(long, default_value = "shell")]
        preset: String,
    },
    /// Move an existing pane against another, on an edge.
    ///
    /// A drag and drop. Works across layouts: the pane leaves the one it was in.
    Move {
        workspace: String,
        terminal: String,
        onto: String,
        #[arg(long, default_value = "right")]
        side: String,
    },
    /// Set the arrangement: even-horizontal, even-vertical, main-vertical,
    /// main-horizontal, tiled.
    Preset { workspace: String, preset: String },
    /// Next arrangement of the same panes. tmux's `prefix Space`.
    Cycle { workspace: String },
    /// Move focus: a terminal, `--next`, `--prev`, or `--pane N`.
    Focus {
        workspace: String,
        terminal: Option<String>,
        #[arg(long)]
        next: bool,
        #[arg(long)]
        prev: bool,
        #[arg(long, value_name = "N")]
        pane: Option<u32>,
    },
    /// Fill the layout with one pane. tmux's `prefix z`.
    Zoom {
        workspace: String,
        terminal: Option<String>,
        #[arg(long)]
        off: bool,
    },
    /// Exchange two panes' positions.
    Swap { workspace: String, a: String, b: String },
    /// Move a divider, in cells. Negative moves it the other way.
    ///
    /// `allow_negative_numbers`, because half of every drag is negative and clap
    /// otherwise reads `--cells -5` as an unknown flag called `-5`. That failed
    /// silently through the app: dragging a divider right worked and dragging it
    /// left did nothing at all.
    #[command(allow_negative_numbers = true)]
    Resize {
        workspace: String,
        terminal: String,
        #[arg(long, default_value = "right")]
        side: String,
        #[arg(long, default_value_t = 2)]
        cells: i32,
    },
    /// Pull a pane out into a layout of its own. tmux's `break-pane`.
    Break { workspace: String, terminal: Option<String> },
    /// Name a layout.
    Rename { workspace: String, name: String },
    /// Tell tmux the size of the viewport showing this layout, in cells.
    Viewport { workspace: String, columns: u32, rows: u32 },
    /// Show a different layout: by number, by name, or the next one.
    Select {
        workspace: String,
        group: Option<String>,
        #[arg(long)]
        next: bool,
        #[arg(long)]
        prev: bool,
    },
}

#[derive(Subcommand)]
enum PushCmd {
    /// Store the token the app issued for this runner. Reads it from stdin.
    ///
    /// STDIN, not an argument, and that is the whole reason this reads oddly.
    /// A bearer token in argv is a bearer token in `ps aux` — world-readable on
    /// a default Linux box — on both the host running this and, over ssh, the
    /// runner being paired. It also ends up in shell history, in sshd's command
    /// logging, and in any error that echoes the command back. One observation
    /// by any local user is permanent: the token does not expire.
    Pair {
        /// A self-hosted relay, if you run your own.
        #[arg(long)]
        relay: Option<String>,
    },
    /// Say whether this runner is paired, without printing the token.
    Status,
    /// Forget the token. Notifications stop; nothing else changes.
    Forget,
}

#[derive(Subcommand)]
enum RunnerCmd {
    /// Copy the daemon and CLI to a Linux host and register a user service.
    Install {
        target: String,
        /// The Linux binaries to install. Defaults to ./dist/<arch>-linux/.
        #[arg(long)]
        from: Option<PathBuf>,
    },
    /// Report what is installed and running on a runner.
    Status { target: String },
    /// Report what a runner IS, changing nothing on it.
    ///
    /// What the Mac app asks before offering to install: the platform, whether
    /// tmux is there, what would keep the daemon alive, and what is already
    /// installed.
    Probe { target: String },
}

#[derive(Subcommand)]
enum RootCmd {
    /// Allowlist a directory Far Cooler may operate in.
    Add { path: PathBuf },
    List,
    /// Stop allowing Far Cooler to operate under a directory.
    ///
    /// Removes its registered repositories too, and touches nothing on disk.
    Remove {
        root: String,
        /// The root directory's exact name. Required, because this revokes
        /// access to a whole tree.
        #[arg(long)]
        confirm: String,
    },
}

#[derive(Subcommand)]
enum ThemeCmd {
    /// Every theme this runner offers: the built-ins, plus whatever
    /// `[themes.<name>]` in config.toml adds.
    List {
        /// Only the themes this runner's config.toml defines.
        ///
        /// What a settings editor wants: the merged list cannot say which
        /// entries the file actually owns, and a built-in shown as if it did
        /// would offer a Delete that does nothing.
        //
        // `--only-host` is kept, hidden, for the reason `--host` is: the Mac
        // app already spells it that way and so does anyone's script.
        #[arg(long = "only-runner", alias = "only-host")]
        only_runner: bool,
    },
    /// Write one `[themes.<name>]` table, from JSON on stdin.
    ///
    /// stdin rather than nineteen positional colours: nineteen of anything on a
    /// command line is a contract nobody can read and one transposition away
    /// from a colour nobody chose. This is the apps' channel, not a hand-typed
    /// one — `$EDITOR ~/.config/farcooler/config.toml` is better at that.
    Set {
        #[arg(long, required = true)]
        json_stdin: bool,
    },
    /// Remove one `[themes.<name>]` table.
    ///
    /// A name that is not there is success, because this is also how a client
    /// reverts a theme that shadows a built-in: the table goes and the shipped
    /// one takes over again.
    Delete { name: String },
}

#[derive(Subcommand)]
enum SettingsCmd {
    /// Show what this runner's config.toml decides.
    Show,
    /// Set what a derived branch name starts with. Empty opts out entirely.
    SetBranchPrefix { prefix: String },
}

#[derive(Subcommand)]
enum AdapterCmd {
    /// Every adapter in force, and whether it is shipped, overridden or yours.
    List,
    /// Write one `[adapters.<name>]` table, from JSON on stdin.
    ///
    /// The apps' channel, for the reason `theme set` is: an adapter is seven
    /// fields including four string arrays, and flags for all of them would be a
    /// worse editor than the file it replaces.
    Set {
        #[arg(long, required = true)]
        json_stdin: bool,
    },
    /// Prove one works: start it and complete an ACP handshake.
    ///
    /// Checks LAUNCH, not detection. A pass means the adapter starts and speaks
    /// ACP; whether the agent gets recognized in a pane depends on the
    /// `identity`, `blocked` and `working` strings, which nothing here can
    /// exercise.
    ///
    /// Naming a preset tests what is IN FORCE. `--json-stdin` tests whatever is
    /// piped in, which is what a form that has not been saved yet needs.
    Test {
        preset: Option<String>,
        #[arg(long, conflicts_with = "preset")]
        json_stdin: bool,
    },
    /// Remove one `[adapters.<name>]` table, restoring the shipped one if this
    /// was overriding it.
    Delete { preset: String },
}

#[derive(Subcommand)]
enum RepoCmd {
    /// Register an existing repository inside an allowlisted root.
    Register { path: PathBuf },
    List,
}

#[derive(Subcommand)]
enum WorkspaceCmd {
    /// Create a worktree and branch for one task.
    Create {
        repo: String,
        /// What to call the worktree. Becomes its directory, and cannot be
        /// changed afterwards: at most 60 characters, and at least one of them
        /// a letter or a number.
        name: String,
        #[arg(long)]
        branch: String,
        #[arg(long, default_value = "HEAD")]
        base: String,
        /// What to run in the terminal the new worktree opens with.
        ///
        /// A worktree with nothing in it is a directory, so this defaults to a
        /// shell — the same default `terminal create` uses, and for the same
        /// reason: you open a terminal and type into it.
        #[arg(long, default_value = "shell")]
        terminal: String,
        /// Create the worktree with no terminal at all.
        ///
        /// For a caller about to create its own. The Mac app's task flow does
        /// exactly this, and would otherwise leave every task with an unused
        /// shell sitting beside its agent.
        #[arg(long, conflicts_with = "terminal")]
        no_terminal: bool,
    },
    /// Show the fleet with freshly derived state.
    List,
    /// Resume work on a branch that already exists.
    ///
    /// The branch may be remote-only — pushed from another machine, by a
    /// colleague, or by a cloud agent. A local tracking branch is created for
    /// it, so pushing back goes where it came from.
    ///
    /// The worktree is named after the branch's last segment, so
    /// `feat/rate-limiting` resumes in a directory called `rate-limiting`.
    Adopt {
        repo: String,
        branch: String,
    },
    /// List branches you could resume work on.
    Branches {
        repo: String,
    },
    /// Take a worktree out of the main list. Never changes git data.
    Hide { workspace: String },
    /// Bring a hidden worktree back.
    Unhide { workspace: String },
    /// Remove the worktree. Keeps the branch and everything committed.
    RemoveWorktree {
        workspace: String,
        /// The workspace's exact name. Required only when the worktree has
        /// uncommitted work in it; the daemon is what decides.
        #[arg(long)]
        confirm: Option<String>,
    },
}

#[derive(Subcommand)]
enum WorktreeCmd {
    /// Search file paths inside a workspace's worktree, for an agent chat's
    /// @-mention picker. Results are worktree-relative, never a runner path —
    /// the same redaction `wire.rs` applies to every other path this scope
    /// can see.
    FileSearch {
        workspace: String,
        query: String,
        /// 0 asks the daemon for its own default rather than this CLI
        /// inventing a second one that could drift from it.
        #[arg(long, default_value_t = 0)]
        limit: u32,
    },
}

#[derive(Subcommand)]
enum TerminalCmd {
    /// Launch a preset in a new tagged tmux window.
    Create {
        workspace: String,
        /// What to launch. Defaults to your shell, which is almost always
        /// right: you open a terminal and type `claude` into it, and Far Cooler
        /// notices what is running rather than being told in advance.
        #[arg(long, default_value = "shell")]
        preset: String,
        #[arg(long)]
        title: Option<String>,
        /// Put it straight into the workspace's active group.
        ///
        /// tmux's `%`: you are looking at a layout and you want another pane in
        /// it. Does nothing when there is no layout, so it is safe to always
        /// pass from a key binding.
        #[arg(long)]
        tile: bool,
    },
    /// Send exact bytes to a terminal.
    Send { terminal: String, data: String },
    /// Send exact input bytes as hex. This is the terminal client input path.
    SendHex { terminal: String, hex: String },
    /// Paste a file into a terminal.
    ///
    /// The file is copied to the runner the terminal is on and its path is
    /// typed into the pane, which is how an agent in that pane gets to look at
    /// it. Any type: an image, a PDF, a log, a CSV. Prints the path on stdout.
    ///
    /// An agent can use this to hand a chart or a screenshot to a SIBLING
    /// pane. Pasting into its own pane would feed it its own stdin.
    PasteFile { terminal: String, file: PathBuf },
    /// Print the rendered visible screen with color escapes intact.
    Screen { terminal: String },
    /// Stream live output bytes to stdout until killed. The terminal data plane.
    Stream { terminal: String },
    /// Persistent input channel: one hex byte-run per stdin line.
    Input { terminal: String },
    /// Resize the terminal to a viewer's geometry.
    Resize { terminal: String, columns: u32, rows: u32 },
    /// Print recent output.
    Read {
        terminal: String,
        #[arg(long, default_value_t = 200)]
        lines: u32,
    },
    /// Stop a terminal.
    Stop { terminal: String },
    /// Forget a lost terminal. Refused unless it is lost.
    DismissLost { terminal: String },
    /// Relaunch from the same preset as a new epoch.
    Restart { terminal: String },
    /// Delete a terminal's record. Refused while it is still running.
    Remove { terminal: String },
    /// Mark a terminal as looked at, clearing a `done` badge.
    ///
    /// Its own command rather than a side effect of `screen`, because a
    /// one-shot dump is not the same as opening a terminal, and clearing a
    /// notification nobody read is worse than not sending one.
    Seen { terminal: String },

    // -- Agent channel: `terminal.set_pane_mode` and friends. Every one of
    // these is a daemon RPC, not a byte stream, so — like `Seen`/`Stop` above
    // and unlike `Send`/`Screen`/`Stream` below — none of them need `proxy()`:
    // `connect_to(runner)` already reaches a remote daemon over ssh, and these
    // never touch the local tmux runtime a remote runner does not have.
    /// Switch a pane between its terminal view and its agent chat view.
    ///
    /// The daemon is the sole owner of which mode a pane is in — `AgentStream.swift`'s
    /// own doc comment records why: two clients guessing for themselves is the
    /// disagreement this design exists to prevent.
    SetPaneMode {
        terminal: String,
        /// "terminal" or "agent".
        mode: String,
        /// Needed to leave agent mode mid-turn: `claude --resume` cannot
        /// reattach to a turn discarded out from under it, so the daemon
        /// refuses this unless told the user has already been warned.
        #[arg(long)]
        force: bool,
    },
    /// New agent-channel events since a cursor, as JSON. What the Mac app's
    /// chat view polls every 200ms (`AgentStream.pump`).
    AgentSubscribe {
        terminal: String,
        #[arg(long, default_value_t = 0)]
        from_seq: u64,
        /// The run of the stream `from_seq` counts positions in.
        ///
        /// A shim renumbers from zero every time it restarts, so a cursor on
        /// its own cannot say which stream it means. Pass back what the last
        /// batch reported; a mismatch returns the whole transcript.
        #[arg(long, default_value_t = 0)]
        epoch: u64,
    },
    /// Send a chat message to the pane's agent.
    AgentPrompt {
        terminal: String,
        text: String,
        /// Attach an image. Repeatable.
        #[arg(long = "image")]
        images: Vec<PathBuf>,
    },
    /// Answer a pending agent question, carrying the ids back exactly as the
    /// adapter sent them — inventing one here would make the answer
    /// unroutable and hang the agent on its own question.
    AgentAnswer { terminal: String, request_id: String, option_id: String },
    /// Switch the running agent's own mode (e.g. a permission mode), not the
    /// pane's mode — see `SetPaneMode` for that.
    AgentSetMode { terminal: String, agent_mode: String },

    /// Switch the model an agent session uses.
    AgentSetModel { terminal: String, model: String },

    /// Change one of an agent session's selectors: mode, model, subagent.
    AgentSetConfig { terminal: String, config_id: String, value: String },

    /// Rewrite a prompt that is still waiting for the current turn to end
    AgentEditQueued { terminal: String, queued_id: String, text: String },

    /// Withdraw a prompt that is still waiting for the current turn to end
    AgentCancelQueued { terminal: String, queued_id: String },

    /// Send a queued prompt into the turn already running, rather than waiting
    AgentSteerQueued { terminal: String, queued_id: String },
    /// Cancel the agent's current turn.
    AgentCancel { terminal: String },
}

#[tokio::main]
async fn main() {
    // Logs go to stderr. stdout is the DATA channel.
    //
    // tracing_subscriber writes to stdout by default, which put warnings in the
    // middle of `--json` output: the Mac app's decode then failed, it kept its
    // previous fleet, and a user who had just created a workspace was told
    // there were none. A CLI whose machine-readable output can be corrupted by
    // an unrelated log line is broken however good the log line is.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("FARCOOLER_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

pub(crate) type Fallible = Result<(), Box<dyn std::error::Error>>;

/// Pair, unpair, or report — on this runner or, with `--runner`, on another.
///
/// Forwarded over ssh rather than through the daemon protocol because a
/// credential should travel over the channel the user already authenticated,
/// not over one the daemon would have to be trusted to keep private.
async fn push(runner: Option<&str>, cmd: PushCmd) -> Fallible {
    use farcooler_daemon::push::Pairing;
    // The daemon's, not a third copy. This crate already had two spellings of
    // shell quoting — `remote::shell_quote`, which passes safe arguments
    // through unquoted, and the daemon's, which always wraps — and a third one
    // on the path that forwards a credential over ssh is the one that gets it
    // wrong later.
    use farcooler_daemon::service::shell_quote;

    if let Some(target) = runner {
        // This channel's CLI on that runner, not the bare name: it is the one
        // this build uploaded, it keeps its pairing beside its own daemon's
        // database, and asking for `farcooler` on a host that also runs the
        // release build would pair the wrong fleet to these devices.
        let remote = |rest: &str| format!("~/.local/bin/{} push {rest}", runner_install::cli_name());
        match &cmd {
            PushCmd::Pair { relay } => {
                let relay = relay
                    .as_deref()
                    .map(|r| format!(" --relay {}", shell_quote(r)))
                    .unwrap_or_default();
                // The token goes down the pipe, not into the command. Which
                // means it is also absent from the error if this fails — see
                // `remote_run`, which names the command it ran.
                let token = read_token()?;
                return runner_install::remote_run_with_stdin(
                    target,
                    &remote(&format!("pair{relay}")),
                    &token,
                )
                .await;
            }
            PushCmd::Status => {
                return runner_install::remote_run(target, &remote("status")).await;
            }
            PushCmd::Forget => {
                return runner_install::remote_run(target, &remote("forget")).await;
            }
        }
    }

    let dir = farcooler_daemon::paths::ensure_runtime_dir()?;
    match cmd {
        PushCmd::Pair { relay } => {
            // The default lives in the daemon, not restated here: two
            // spellings of a relay URL is one of them being wrong later.
            let relay = relay.unwrap_or_else(farcooler_daemon::push::default_relay);
            let token = read_token()?;
            Pairing { relay, token }.save_in(&dir)?;
            println!("paired · notifications will go to your signed-in devices");
        }
        PushCmd::Status => match Pairing::load_in(&dir) {
            // Never the token itself. A status command that prints a credential
            // is a credential in every terminal scrollback and CI log.
            Some(p) => println!("paired · relay {}", p.relay),
            None => println!("not paired"),
        },
        PushCmd::Forget => {
            Pairing::forget_in(&dir);
            println!("forgotten · this runner will not notify anything");
        }
    }
    Ok(())
}

/// One line from stdin, with nothing echoed.
///
/// Trimmed because whoever pipes this in — the Mac app, a shell heredoc, ssh —
/// will append a newline, and a token with a trailing newline authenticates
/// against nothing while looking exactly right in every error message.
fn read_token() -> Result<String, Box<dyn std::error::Error>> {
    use std::io::Read;
    let mut token = String::new();
    std::io::stdin().read_to_string(&mut token)?;
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("no token on stdin (the app pairs a runner for you)".into());
    }
    Ok(token)
}

async fn run() -> Fallible {
    let cli = Cli::parse();
    let runner = cli.runner.as_deref();
    match cli.command {
        Command::Status => status(runner, cli.json).await,
        Command::Daemon(c) => daemon(runner, c, cli.json).await,
        Command::Root(c) => root(runner, c, cli.json).await,
        Command::Repo(c) => repo(runner, c, cli.json).await,
        Command::Theme(c) => theme(runner, c, cli.json).await,
        Command::Settings(c) => settings(runner, c, cli.json).await,
        Command::Adapter(c) => adapter(runner, c, cli.json).await,
        Command::Workspace(c) => workspace(runner, c, cli.json).await,
        Command::Terminal(c) => terminal(runner, c, cli.json).await,
        Command::Changes(c) => changes::changes(runner, c, cli.json).await,
        Command::Worktree(c) => worktree(runner, c, cli.json).await,
        Command::Layout(c) => layout(runner, c, cli.json).await,
        Command::Attach { workspace } => attach(runner, &workspace).await,
        Command::Events => events(runner).await,
        Command::Push(c) => push(runner, c).await,
        Command::Client(c) => clients::client(runner, c, cli.json).await,
        Command::RunnerCmd(RunnerCmd::Install { target, from }) => {
            runner_install::install(&target, from.as_deref()).await
        }
        Command::RunnerCmd(RunnerCmd::Status { target }) => runner_install::status(&target).await,
        Command::RunnerCmd(RunnerCmd::Probe { target }) => {
            let probe = runner_install::probe(&target).await?;
            if cli.json {
                println!("{}", probe.to_json());
                return Ok(());
            }
            println!("{}", probe.target);
            println!("  platform    {}", probe.platform.name());
            println!("  os          {} {}", probe.os, probe.arch);
            println!("  tmux        {}", probe.tmux.as_deref().unwrap_or("MISSING"));
            println!("  persistence {}", probe.persistence.name());
            println!("  installed   {}", probe.installed_cli.as_deref().unwrap_or("nothing"));
            for blocker in &probe.blockers {
                println!("  BLOCKED     {blocker}");
            }
            Ok(())
        }
        Command::AgentHost { terminal, socket, worktree, session, preset } => {
            agent_host::run(terminal, socket, worktree, session, preset).await
        }
        Command::PaneHost { kind } => pane_host(&kind).await,
    }
}

/// Hold a pane open until tmux takes it away.
///
/// The line it prints is for the one reader who ever sees this screen: someone
/// who broke the pane out into a raw `tmux attach`, or a phone that renders the
/// capture because it has no diff view of its own. A pane that printed nothing
/// would look to both of them like a program that had crashed.
async fn pane_host(kind: &str) -> Fallible {
    let what = match kind {
        "changes" => "changes",
        other => return Err(format!("unknown pane kind: {other}").into()),
    };
    println!("Far Cooler is drawing this worktree's {what} here.");
    // No signal handling: tmux kills the pane's process group, and a wait that
    // caught SIGHUP to exit tidily would only be a slower way to be killed.
    std::future::pending::<()>().await;
    Ok(())
}

/// Start, stop, or replace the daemon on THIS runner.
async fn daemon(runner: Option<&str>, cmd: DaemonCmd, json: bool) -> Fallible {
    if runner.is_some() {
        return Err("daemon manages this runner's daemon; drop --runner".into());
    }

    match cmd {
        DaemonCmd::Ensure => {
            let (action, build) = daemon_link::ensure_local().await?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "action": action.as_str(), "daemonVersion": build })
                );
            } else {
                println!("{} {build}", action.as_str());
            }
            Ok(())
        }
        DaemonCmd::Stop => {
            let mut link = match daemon_link::connect_existing().await? {
                Some(link) => link,
                None => {
                    if json {
                        println!("{}", serde_json::json!({ "action": "not_running" }));
                    } else {
                        println!("not running");
                    }
                    return Ok(());
                }
            };
            daemon_link::stop(&mut link).await?;
            if json {
                println!("{}", serde_json::json!({ "action": "stopped" }));
            } else {
                println!("stopped");
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Durable state, through the daemon
// ---------------------------------------------------------------------------

async fn status(runner: Option<&str>, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    let roots = list_roots(&mut link).await?;
    let repos = list_repositories(&mut link).await?;
    let workspaces = list_workspaces(&mut link).await?;
    let terminals = list_terminals(&mut link, None).await?;
    let host_facts = host_get(&mut link).await?;
    let healthy = host_facts.self_health != farcooler_protocol::v1::SelfHealth::Degraded as i32;
    // What this runner can do, by name, from the handshake. The Mac app reads
    // it here because it drives remote runners through this CLI rather than
    // through the FFI the phones use — same answer, same source, one round trip
    // fewer than asking.
    let capabilities = link.daemon_capabilities().to_vec();

    if json {
        println!(
            "{}",
            serde_json::json!({
                "daemonVersion": host_facts.daemon_version,
                "cliVersion": farcooler_protocol::BUILD,
                // Two builds that cannot agree on what they are running is a
                // fact a client needs, not a detail. It is how a fix that was
                // compiled and tested goes on reproducing in the app.
                "buildsMatch": host_facts.daemon_version == farcooler_protocol::BUILD,
                // Distinct from `buildsMatch`, and they answer different
                // questions. That one is "are these the same build"; this is
                // "what can that runner do", which is the one a client acts on
                // when it is newer than the runner it reached.
                "capabilities": capabilities,
                "platform": host_facts.platform,
                "branchPrefix": host_facts.settings
                    .as_ref()
                    .map(|s| s.branch_prefix.clone())
                    .unwrap_or_default(),
                "runtimeHealthy": healthy,
                "livePanes": host_facts.live_terminal_count,
                "roots": roots.len(),
                "repositories": repos.len(),
                "workspaces": workspaces.len(),
                "terminals": terminals.len(),
            })
        );
        return Ok(());
    }

    println!("runner        {}", runner.unwrap_or("local"));
    println!("platform      {}", host_facts.platform);
    println!("daemon        {}", host_facts.daemon_version);
    println!("this cli      {}", farcooler_protocol::BUILD);
    if host_facts.daemon_version != farcooler_protocol::BUILD {
        // Said out loud rather than left to be noticed. A CLI and a daemon
        // built from different source can speak the same protocol perfectly
        // and still behave like two different programs, and the symptom of
        // that is a bug you already fixed still happening.
        println!("              ^ MISMATCH: these were built from different source");
    }
    println!(
        "tmux runtime  {}",
        if healthy { "reachable" } else { "UNAVAILABLE (all terminals derive lost)" }
    );
    for reason in &host_facts.self_health_reasons {
        println!("              {reason}");
    }
    println!("roots         {}", roots.len());
    println!("repositories  {}", repos.len());
    println!("workspaces    {}", workspaces.len());
    println!("live panes    {}", host_facts.live_terminal_count);

    // The recovery command names a tmux socket on THIS runner, so it is only
    // meaningful locally. Printing a local socket path while reporting a remote
    // runner would be an invitation to run the wrong thing.
    if runner.is_none() {
        let runtime = Runtime::open().await?;
        println!();
        println!("recovery: {}", runtime.tmux.recovery_command());
    }
    Ok(())
}

async fn root(runner: Option<&str>, cmd: RootCmd, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    match cmd {
        RootCmd::Add { path } => {
            // Canonicalised here so the daemon is handed an absolute path
            // regardless of which directory the user ran this from.
            let absolute = path.canonicalize().unwrap_or(path);
            let root = farcooler_client::actions::add_repository_root(
                link.client_mut(),
                &absolute.to_string_lossy(),
            )
            .await?;
            println!(
                "added root {}  {}",
                short_bytes(&root.id),
                root.display_path.unwrap_or_else(|| root.path_token.clone())
            );
        }
        RootCmd::Remove { root, confirm } => {
            let roots = list_roots(&mut link).await?;
            let target = resolve(&roots, &root, |r| &r.id, "root")?;
            link.call(with(
                req_for("repository_root.remove", uuid_of(&target.id)),
                request::Payload::TypedConfirmation(farcooler_protocol::v1::TypedConfirmation {
                    typed_confirmation: confirm,
                }),
            ))
            .await?;
            println!("removed root {} (nothing on disk was touched)", short_bytes(&target.id));
        }
        RootCmd::List => {
            let roots = list_roots(&mut link).await?;
            if json {
                let items: Vec<_> = roots
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "id": uuid_of(&r.id).to_string(),
                            "short": short_bytes(&r.id),
                            "path": r.display_path,
                            "repositories": r.repository_count,
                        })
                    })
                    .collect();
                println!("{}", serde_json::json!({ "roots": items }));
                return Ok(());
            }
            for r in roots {
                println!(
                    "{}  {}",
                    short_bytes(&r.id),
                    r.display_path.unwrap_or_else(|| r.path_token.clone())
                );
            }
        }
    }
    Ok(())
}

/// What this runner's config.toml decides, and how to change it.
async fn settings(runner: Option<&str>, cmd: SettingsCmd, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    match cmd {
        SettingsCmd::Show => {
            let facts = host_get(&mut link).await?;
            let prefix = facts
                .settings
                .as_ref()
                .map(|s| s.branch_prefix.clone())
                .unwrap_or_default();
            if json {
                println!("{}", serde_json::json!({ "branchPrefix": prefix }));
            } else {
                // Quoted, because the empty string is a real and deliberate
                // value here and an unquoted blank line would read as a bug.
                println!("branch prefix  \"{prefix}\"");
            }
        }
        SettingsCmd::SetBranchPrefix { prefix } => {
            let req = with(
                req("settings.set_branch_prefix"),
                request::Payload::HostSettings(farcooler_protocol::v1::HostSettings {
                    branch_prefix: prefix,
                }),
            );
            let r = link.call(req).await?;
            let result::Value::Host(h) = expect_value(r.value, "host")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            // Read back from the reply rather than echoing what was sent: the
            // writer trims, so the file may not say quite what arrived.
            let stored = h.settings.map(|s| s.branch_prefix).unwrap_or_default();
            if json {
                println!("{}", serde_json::json!({ "branchPrefix": stored }));
            } else {
                println!("branch prefix is now \"{stored}\"");
            }
        }
    }
    Ok(())
}

/// The ACP adapters in force, and the two things worth doing to one.
///
/// No `upsert` here on purpose. An adapter is seven fields including four string
/// arrays, and a command line for it would be a worse editor than the file it is
/// meant to replace — `$EDITOR ~/.config/farcooler/config.toml` is the right
/// tool for that, and the apps have a form. What a terminal is good for is
/// seeing what is in force and proving one works, which is what these do.
async fn adapter(runner: Option<&str>, cmd: AdapterCmd, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    match cmd {
        AdapterCmd::List => {
            let r = link.call(req("adapter.list")).await?;
            let result::Value::AdapterList(list) = expect_value(r.value, "adapters")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            if json {
                let items: Vec<_> = list
                    .items
                    .iter()
                    .map(|a| {
                        serde_json::json!({
                            "preset": a.preset,
                            "program": a.program,
                            "args": a.args,
                            "origin": origin_label(a.origin),
                            "backend": backend_label(a.backend),
                            "chatCapable": !a.program.is_empty(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::json!({ "adapters": items }));
                return Ok(());
            }
            for a in &list.items {
                // An adapter with no program is a recognized agent that Far
                // Cooler cannot host as a chat, which is a real state and not a
                // gap — so it is named rather than left blank.
                let launch = if a.program.is_empty() {
                    "(terminal only)".to_string()
                } else {
                    format!("{} {}", a.program, a.args.join(" "))
                };
                println!(
                    "{:12}  {:10}  {:7}  {}",
                    a.preset,
                    origin_label(a.origin),
                    backend_label(a.backend),
                    launch
                );
            }
        }

        AdapterCmd::Set { .. } => {
            let adapter = adapter_from_json(&read_json_stdin()?);
            let preset = adapter.preset.clone();
            let r = link.call(with(req("adapter.upsert"), request::Payload::Adapter(adapter))).await?;
            let result::Value::AdapterList(list) = expect_value(r.value, "adapters")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            if json {
                println!("{}", serde_json::json!({ "adapters": list.items.len() }));
            } else {
                println!("saved adapter {preset}");
            }
        }

        AdapterCmd::Test { preset, json_stdin } => {
            // Two callers, two inputs. A terminal names a preset and means
            // "test what is in force"; an app pipes a form and means "will this
            // work if I save it". Same method underneath.
            let found = if json_stdin {
                adapter_from_json(&read_json_stdin()?)
            } else {
                let preset = preset.ok_or("name an adapter, or pass --json-stdin")?;
                let r = link.call(req("adapter.list")).await?;
                let result::Value::AdapterList(list) = expect_value(r.value, "adapters")? else {
                    return Err("the daemon returned the wrong resource".into());
                };
                let found = list
                    .items
                    .into_iter()
                    .find(|a| a.preset == preset)
                    .ok_or_else(|| format!("no adapter named {preset}"))?;
                if found.program.is_empty() {
                    return Err(format!("{preset} has no adapter, so it stays a terminal").into());
                }
                found
            };
            let preset = found.preset.clone();

            let r = link.call(with(req("adapter.test"), request::Payload::Adapter(found))).await?;
            let result::Value::AdapterTestResult(outcome) = expect_value(r.value, "test result")?
            else {
                return Err("the daemon returned the wrong resource".into());
            };
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": outcome.ok,
                        "reported": outcome.reported,
                        "failure": outcome.failure,
                    })
                );
            } else if outcome.ok {
                println!("{preset}  OK  {}", outcome.reported);
                println!();
                println!("This proves it starts and speaks ACP. Whether the agent is");
                println!("RECOGNIZED in a pane depends on its detection strings, which");
                println!("nothing here can check.");
            } else {
                println!("{preset}  FAILED  {}", outcome.failure);
            }
            if !outcome.ok {
                std::process::exit(1);
            }
        }

        AdapterCmd::Delete { preset } => {
            let req = with(
                req("adapter.delete"),
                request::Payload::TypedConfirmation(farcooler_protocol::v1::TypedConfirmation {
                    typed_confirmation: preset.clone(),
                }),
            );
            let r = link.call(req).await?;
            let result::Value::AdapterList(list) = expect_value(r.value, "adapters")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            let restored = list.items.iter().find(|a| a.preset == preset);
            match restored {
                // The table went and a shipped adapter took over, which is what
                // reverting an override means.
                Some(a) => println!(
                    "removed the table for {preset}; the built-in is back ({})",
                    a.program
                ),
                None => println!("removed {preset}"),
            }
        }
    }
    Ok(())
}

/// Read one JSON value from stdin.
///
/// The apps' channel into the two commands that would otherwise need a flag per
/// field. Whatever they send is a value they already hold, so there is nothing
/// to spell out on a command line and nothing to transpose.
fn read_json_stdin() -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    use std::io::Read;
    let mut body = String::new();
    std::io::stdin().read_to_string(&mut body)?;
    Ok(serde_json::from_str(&body)?)
}

/// An adapter out of the JSON an app sends.
fn adapter_from_json(payload: &serde_json::Value) -> farcooler_protocol::v1::Adapter {
    let strings = |key: &str| -> Vec<String> {
        payload[key]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default()
    };
    farcooler_protocol::v1::Adapter {
        preset: payload["preset"].as_str().unwrap_or_default().to_string(),
        program: payload["program"].as_str().unwrap_or_default().to_string(),
        args: strings("args"),
        env: payload["env"]
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
        commands: strings("commands"),
        identity: strings("identity"),
        blocked: strings("blocked"),
        working: strings("working"),
        // Absent means acp, matching what an omitted `backend` key in the
        // config file means. An app that predates this field keeps working.
        backend: payload["backend"]
            .as_str()
            .map(|name| farcooler_core::activity::AdapterBackend::parse(name).to_proto() as i32)
            .unwrap_or_default(),
        // The daemon decides this on the way back out. A caller claiming an
        // origin would be claiming something only the daemon can know.
        origin: 0,
    }
}

/// Where an adapter came from, for a terminal to read.
fn origin_label(origin: i32) -> &'static str {
    match farcooler_protocol::v1::AdapterOrigin::try_from(origin) {
        Ok(farcooler_protocol::v1::AdapterOrigin::BuiltIn) => "built-in",
        Ok(farcooler_protocol::v1::AdapterOrigin::Override) => "override",
        Ok(farcooler_protocol::v1::AdapterOrigin::User) => "yours",
        _ => "unknown",
    }
}

/// Which protocol an adapter speaks, as a person reads it.
///
/// Unspecified reads as `acp` rather than `unknown`, because that is what it
/// means: a daemon or config file that says nothing about the backend is
/// asking for the one every adapter had before the field existed.
fn backend_label(backend: i32) -> &'static str {
    match farcooler_protocol::v1::AdapterBackend::try_from(backend) {
        Ok(farcooler_protocol::v1::AdapterBackend::Native) => "native",
        _ => "acp",
    }
}

/// The themes a runner offers.
///
/// Built-ins merged with the runner's own, resolved HERE rather than in the app,
/// so the Mac and the two phones cannot come to disagree about what "Nord"
/// means or about which of two definitions wins.
async fn theme(runner: Option<&str>, cmd: ThemeCmd, json: bool) -> Fallible {
    if let ThemeCmd::Delete { name } = &cmd {
        let mut link = connect_to(runner).await?;
        let req = with(
            req("theme.delete"),
            request::Payload::TypedConfirmation(farcooler_protocol::v1::TypedConfirmation {
                typed_confirmation: name.clone(),
            }),
        );
        let r = link.call(req).await?;
        let result::Value::ThemeList(list) = expect_value(r.value, "themes")? else {
            return Err("the daemon returned the wrong resource".into());
        };
        if json {
            println!("{}", serde_json::json!({ "themes": list.items.len() }));
        } else {
            println!("removed theme {name}  ({} runner themes left)", list.items.len());
        }
        return Ok(());
    }
    if let ThemeCmd::Set { .. } = &cmd {
        let payload: serde_json::Value = read_json_stdin()?;
        let ansi: Vec<u32> = payload["ansi"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
            .unwrap_or_default();
        if ansi.len() != 16 {
            return Err("a theme needs exactly sixteen ANSI colors".into());
        }
        let wire_theme = farcooler_protocol::v1::Theme {
            name: payload["name"].as_str().unwrap_or_default().to_string(),
            dark: payload["dark"].as_bool().unwrap_or(true),
            background: payload["background"].as_u64().unwrap_or(0) as u32,
            foreground: payload["foreground"].as_u64().unwrap_or(0) as u32,
            cursor: payload["cursor"].as_u64().unwrap_or(0) as u32,
            ansi,
        };
        let name = wire_theme.name.clone();

        let mut link = connect_to(runner).await?;
        let r = link.call(with(req("theme.upsert"), request::Payload::Theme(wire_theme))).await?;
        let result::Value::ThemeList(list) = expect_value(r.value, "themes")? else {
            return Err("the daemon returned the wrong resource".into());
        };
        if json {
            println!("{}", serde_json::json!({ "themes": list.items.len() }));
        } else {
            println!("saved theme {name}");
        }
        return Ok(());
    }

    let ThemeCmd::List { only_runner } = cmd else { unreachable!("handled above") };

    // The runner's own, over whichever transport reaches it. A runner that
    // cannot be reached still has the built-ins — the picker should not empty
    // itself because a laptop is asleep.
    let custom = match connect_to(runner).await {
        Ok(mut link) => list_themes(&mut link).await.unwrap_or_default(),
        Err(_) => Vec::new(),
    };

    if only_runner {
        // What the FILE defines, nothing else. A settings editor needs to know
        // which entries it can delete, and the merged list below cannot say.
        // Whether each one shadows something Far Cooler ships, computed here
        // because this is the one place both halves are already in hand: the
        // built-in table is compiled in and the runner's list just arrived. A
        // client would otherwise need a third call to work it out.
        let shipped: std::collections::BTreeSet<String> =
            farcooler_core::theme::built_in().into_iter().map(|t| t.name).collect();
        if json {
            let items: Vec<_> = custom
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "dark": t.dark,
                        "background": t.background,
                        "foreground": t.foreground,
                        "cursor": t.cursor,
                        "ansi": t.ansi,
                        "shadowsBuiltIn": shipped.contains(&t.name),
                    })
                })
                .collect();
            println!("{}", serde_json::json!({ "themes": items }));
        } else {
            for t in &custom {
                println!("{}", t.name);
            }
        }
        return Ok(());
    }

    let mut themes = farcooler_core::theme::built_in();
    for one in custom {
        let resolved = farcooler_core::theme::Theme {
            name: one.name,
            dark: one.dark,
            background: one.background,
            foreground: one.foreground,
            cursor: one.cursor,
            ansi: match <[u32; 16]>::try_from(one.ansi.as_slice()) {
                Ok(a) => a,
                // The daemon drops these before sending, so reaching here means
                // an older runner. Skipping beats padding a colour nobody chose.
                Err(_) => continue,
            },
        };
        // The runner wins a name collision: it is the more specific statement,
        // and the one somebody edited a file on purpose to make.
        match themes.iter().position(|t| t.name == resolved.name) {
            Some(i) => themes[i] = resolved,
            None => themes.push(resolved),
        }
    }

    if json {
        let items: Vec<_> = themes
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "dark": t.dark,
                    "background": t.background,
                    "foreground": t.foreground,
                    "cursor": t.cursor,
                    "ansi": t.ansi,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({ "themes": items, "default": farcooler_core::theme::DEFAULT_THEME })
        );
        return Ok(());
    }

    for t in &themes {
        println!("{:<20} {}", t.name, if t.dark { "dark" } else { "light" });
    }
    Ok(())
}

async fn repo(runner: Option<&str>, cmd: RepoCmd, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    match cmd {
        RepoCmd::Register { path } => {
            let absolute = path.canonicalize().unwrap_or(path);
            let repo = farcooler_client::actions::register_repository(
                link.client_mut(),
                &absolute.to_string_lossy(),
            )
            .await?;
            println!(
                "registered {}  {}  ({})",
                short_bytes(&repo.id),
                repo.display_name,
                repo.remote_summary
            );
        }
        RepoCmd::List => {
            let repos = list_repositories(&mut link).await?;
            if json {
                let items: Vec<_> = repos
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "id": uuid_of(&r.id).to_string(),
                            "short": short_bytes(&r.id),
                            "displayName": r.display_name,
                            "remote": r.remote_summary,
                            // Which root this repository is allowlisted under.
                            // Two repositories under the same root are removed
                            // together — there is no "remove just this one" at
                            // the daemon — so the Mac app needs this to warn
                            // about siblings before someone removes more than
                            // they meant to.
                            "repositoryRootId": uuid_of(&r.repository_root_id).to_string(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::json!({ "repositories": items }));
                return Ok(());
            }
            for r in repos {
                println!("{}  {:20}  {}", short_bytes(&r.id), r.display_name, r.remote_summary);
            }
        }
    }
    Ok(())
}

async fn workspace(runner: Option<&str>, cmd: WorkspaceCmd, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    match cmd {
        WorkspaceCmd::Create { repo, name, branch, base, terminal, no_terminal } => {
            let repos = list_repositories(&mut link).await?;
            let target = resolve_repository(&repos, &repo)?;
            let r = link
                .call(with(
                    req_for("workspace.create", uuid_of(&target.id)),
                    request::Payload::WorkspaceCreate(farcooler_protocol::v1::WorkspaceCreate {
                        // The field kept its wire name and changed meaning:
                        // this is the worktree's name now. Renaming it would
                        // have broken every shipped client to say the same
                        // thing in different words.
                        task_name: name,
                        branch,
                        base_revision: base,
                        terminal_preset: if no_terminal { String::new() } else { terminal },
                        adopt_existing: false,
                    }),
                ))
                .await?;
            let result::Value::Workspace(ws) = expect_value(r.value, "workspace")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            println!("created workspace {}  {}", short_bytes(&ws.id), ws.task_name);
            println!("  branch   {}", ws.branch);
            if let Some(path) = &ws.worktree_path {
                println!("  worktree {path}");
            }
        }

        WorkspaceCmd::List => {
            let workspaces = list_workspaces(&mut link).await?;
            let terminals = list_terminals(&mut link, None).await?;
            let repositories = list_repositories(&mut link).await?;
            let host_facts = host_get(&mut link).await?;
            let healthy =
                host_facts.self_health != farcooler_protocol::v1::SelfHealth::Degraded as i32;

            if json {
                let items: Vec<_> = workspaces
                    .iter()
                    .map(|w| {
                        serde_json::json!({
                            "id": uuid_of(&w.id).to_string(),
                            "short": short_bytes(&w.id),
                            "task": w.task_name,
                            "branch": w.branch,
                            // Which project this belongs to. A fleet is grouped
                            // by project in the UI, and a client cannot join
                            // ids to names by itself.
                            // Which runner. Empty means this one; a client
                            // merges several runners into one fleet and needs
                            // to know where to route an action back to.
                            //
                            // Still spelled `host` because the apps decode this
                            // key. `--json` is an interface, so renaming a
                            // field is a compatibility change with a migration
                            // in it, not a vocabulary one.
                            "host": runner.unwrap_or_default(),
                            "repository": repositories.iter()
                                .find(|r| r.id == w.repository_id)
                                .map(|r| r.display_name.clone())
                                .unwrap_or_default(),
                            "worktree": w.worktree_path,
                            "state": workspace_label(w.state()),
                            "is_main_checkout": w.is_main_checkout,
                            "terminals": terminals.iter()
                                .filter(|t| t.workspace_id == w.id)
                                .map(workspace_list_terminal_json)
                                .collect::<Vec<_>>(),
                        })
                    })
                    .collect();

                println!(
                    "{}",
                    serde_json::json!({
                        "runtime_healthy": healthy,
                        "live_panes": host_facts.live_terminal_count,
                        // The Mac app reads this on every refresh, which is why
                        // it rides the envelope that call already parses rather
                        // than costing a second subprocess per branch it names.
                        "branch_prefix": host_facts.settings
                            .as_ref()
                            .map(|s| s.branch_prefix.clone())
                            .unwrap_or_default(),
                        "workspaces": items,
                    })
                );
                return Ok(());
            }

            if workspaces.is_empty() {
                println!("no workspaces yet");
            }
            for w in &workspaces {
                println!(
                    "{}  {:22}  {:8}  {}",
                    short_bytes(&w.id),
                    truncate(&w.task_name, 22),
                    workspace_label(w.state()),
                    w.branch
                );
                for t in terminals.iter().filter(|t| t.workspace_id == w.id) {
                    let activity = activity_label(t.activity);
                    println!(
                        "    {}  {:16}  {:8}  {:8}  {}",
                        short_bytes(&t.id),
                        truncate(&t.title, 16),
                        terminal_label(t.state()),
                        if activity == "none" { "" } else { activity },
                        label(t)
                    );
                }
            }
        }

        WorkspaceCmd::Branches { repo } => {
            let repos = list_repositories(&mut link).await?;
            let target = resolve_repository(&repos, &repo)?;
            let r = link.call(req_for("branch.list", uuid_of(&target.id))).await?;
            let result::Value::BranchList(list) = expect_value(r.value, "branches")? else {
                return Err("the daemon returned the wrong list".into());
            };

            if json {
                let items: Vec<_> = list
                    .items
                    .iter()
                    .map(|b| {
                        serde_json::json!({
                            "name": b.name,
                            "local": b.local,
                            "remote": b.remote,
                            "checkedOut": b.checked_out,
                            "subject": b.subject,
                            "updatedAt": b.updated_at.as_ref().map(|t| t.seconds),
                        })
                    })
                    .collect();
                println!("{}", serde_json::json!({ "branches": items }));
                return Ok(());
            }

            for b in list.items {
                let where_ = match (b.local, &b.remote) {
                    (true, Some(r)) => format!("local + {r}"),
                    (true, None) => "local".into(),
                    (false, Some(r)) => format!("{r} only"),
                    (false, None) => "?".into(),
                };
                let busy = if b.checked_out { "  (checked out)" } else { "" };
                println!("{:40}  {:14}{}  {}", truncate(&b.name, 40), where_, busy, truncate(&b.subject, 50));
            }
        }

        WorkspaceCmd::Adopt { repo, branch } => {
            let repos = list_repositories(&mut link).await?;
            let target = resolve_repository(&repos, &repo)?;
            let r = link
                .call(with(
                    req_for("workspace.create", uuid_of(&target.id)),
                    request::Payload::WorkspaceCreate(farcooler_protocol::v1::WorkspaceCreate {
                        // Ignored for an adoption: the daemon names the worktree
                        // after the branch, so there is nothing to send.
                        task_name: String::new(),
                        branch,
                        base_revision: String::new(),
                        terminal_preset: String::new(),
                        adopt_existing: true,
                    }),
                ))
                .await?;
            let result::Value::Workspace(ws) = expect_value(r.value, "workspace")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            println!("adopted {}  {}", short_bytes(&ws.id), ws.branch);
            if let Some(path) = &ws.worktree_path {
                println!("  worktree {path}");
            }
        }

        WorkspaceCmd::Hide { workspace } => {
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            farcooler_client::actions::hide_workspace(link.client_mut(), uuid_of(&ws.id)).await?;
            println!("hidden {}  (git data untouched)", short_bytes(&ws.id));
        }

        WorkspaceCmd::Unhide { workspace } => {
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            farcooler_client::actions::unhide_workspace(link.client_mut(), uuid_of(&ws.id)).await?;
            println!("unhidden {}", short_bytes(&ws.id));
        }

        WorkspaceCmd::RemoveWorktree { workspace, confirm } => {
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            // The daemon checks this too, and its check is the one that counts:
            // a client that skips the prompt must still be refused.
            use farcooler_client::actions::RemoveWorktreeOutcome;
            match farcooler_client::actions::remove_worktree(
                link.client_mut(),
                uuid_of(&ws.id),
                &confirm.unwrap_or_default(),
            )
            .await?
            {
                RemoveWorktreeOutcome::Removed => {
                    println!("removed worktree for {} (branch kept)", short_bytes(&ws.id));
                }
                // Same substring the CLI's own prior direct call produced in
                // its error text — DaemonClient.swift on macOS still sniffs
                // for "confirmation" in whatever this prints to stderr.
                RemoveWorktreeOutcome::ConfirmationRequired => {
                    return Err("exact typed confirmation required".into());
                }
            }
        }
    }
    Ok(())
}

async fn attach(runner: Option<&str>, workspace: &str) -> Fallible {
    let mut link = connect_to(runner).await?;
    let all = list_workspaces(&mut link).await?;
    let ws = resolve(&all, workspace, |w| &w.id, "workspace")?;

    println!("Attaching to the live tmux session for {}.", ws.task_name);
    println!();
    println!("This is a recovery interface. It takes no writer lease and is");
    println!("outside lease enforcement, exactly like raw tmux attach.");
    println!();
    match runner {
        // The tmux socket is on the runner, so the command has to run there.
        Some(target) => {
            println!("  ssh -t {target} farcooler attach {workspace}");
        }
        None => {
            let runtime = Runtime::open().await?;
            println!("  {}", runtime.tmux.recovery_command());
        }
    }
    println!();
    println!("Run that to attach. Detach with your tmux prefix then d.");
    Ok(())
}

/// Print changes as they arrive, forever.
///
/// One JSON object per line, flushed immediately: a client reads this with a
/// line reader and reacts, rather than asking again and again. Line-delimited
/// rather than a JSON array because there is no end to wait for.
async fn events(runner: Option<&str>) -> Fallible {
    use std::io::Write;

    let mut link = connect_to(runner).await?;
    let mut out = std::io::stdout();

    loop {
        let event = link.next_event().await?;
        let Some(payload) = event.payload else { continue };

        let line = match payload {
            farcooler_protocol::v1::event::Payload::TerminalChanged(t) => terminal_event_json(&t),
            farcooler_protocol::v1::event::Payload::WorkspaceChanged(w) => serde_json::json!({
                "kind": "workspace",
                "id": uuid_of(&w.id).to_string(),
                "short": short_bytes(&w.id),
                "task": w.task_name,
                "state": workspace_label(w.state()),
            }),
            farcooler_protocol::v1::event::Payload::LayoutChanged(l) => serde_json::json!({
                "kind": "layout",
                "workspace": uuid_of(&l.workspace_id).to_string(),
                "groups": l.items.iter().map(|g| serde_json::json!({
                    "id": g.id,
                    "name": g.name,
                    "active": g.active,
                    "columns": g.columns,
                    "rows": g.rows,
                    "layout": g.layout,
                    "panes": g.panes.iter().map(|p| serde_json::json!({
                        "id": uuid_of(&p.terminal_id).to_string(),
                        "short": short_bytes(&p.terminal_id),
                        "left": p.left, "top": p.top,
                        "columns": p.columns, "rows": p.rows,
                        "focused": p.focused, "zoomed": p.zoomed,
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            }),
            farcooler_protocol::v1::event::Payload::FleetChanged(_) => serde_json::json!({
                "kind": "fleet",
            }),
            // Other resources have no events yet. Skipping is right: a client
            // that reacts to a line it cannot read would be worse.
            _ => continue,
        };

        writeln!(out, "{line}")?;
        // Unbuffered on purpose. A client blocked on a line that is sitting in
        // our buffer is exactly the latency this command exists to remove.
        out.flush()?;
    }
}

// ---------------------------------------------------------------------------
// Tiling
//
// Every one of these is a `layout.*` call against a workspace, and every one
// answers with the workspace's whole set of groups — so the printer is written
// once and each command is a payload.
// ---------------------------------------------------------------------------

async fn layout(runner: Option<&str>, cmd: LayoutCmd, json: bool) -> Fallible {
    use farcooler_daemon::layout::parse_preset;
    use farcooler_protocol::v1::LayoutUpdate;

    let mut link = connect_to(runner).await?;
    let workspaces = list_workspaces(&mut link).await?;

    let workspace_arg = match &cmd {
        LayoutCmd::Show { workspace }
        | LayoutCmd::Split { workspace, .. }
        | LayoutCmd::Move { workspace, .. }
        | LayoutCmd::Preset { workspace, .. }
        | LayoutCmd::Cycle { workspace }
        | LayoutCmd::Focus { workspace, .. }
        | LayoutCmd::Zoom { workspace, .. }
        | LayoutCmd::Swap { workspace, .. }
        | LayoutCmd::Resize { workspace, .. }
        | LayoutCmd::Break { workspace, .. }
        | LayoutCmd::Rename { workspace, .. }
        | LayoutCmd::Viewport { workspace, .. }
        | LayoutCmd::Select { workspace, .. } => workspace.clone(),
    };
    let ws = resolve(&workspaces, &workspace_arg, |w| &w.id, "workspace")?;
    let workspace_id = uuid_of(&ws.id);

    let terminals = list_terminals(&mut link, Some(workspace_id)).await?;
    let pick = |given: &str| -> Result<bytes::Bytes, String> {
        resolve(&terminals, given, |t| &t.id, "terminal").map(|t| t.id.clone())
    };

    let method = match &cmd {
        LayoutCmd::Show { .. } => "layout.list",
        LayoutCmd::Split { .. } => "layout.split",
        LayoutCmd::Move { .. } => "layout.move",
        LayoutCmd::Preset { .. } => "layout.preset",
        LayoutCmd::Cycle { .. } => "layout.cycle",
        LayoutCmd::Focus { .. } => "layout.focus",
        LayoutCmd::Zoom { .. } => "layout.zoom",
        LayoutCmd::Swap { .. } => "layout.swap",
        LayoutCmd::Resize { .. } => "layout.resize",
        LayoutCmd::Break { .. } => "layout.break",
        LayoutCmd::Rename { .. } => "layout.rename",
        LayoutCmd::Viewport { .. } => "layout.viewport",
        LayoutCmd::Select { .. } => "layout.group.select",
    };

    let mut update = LayoutUpdate::default();
    match &cmd {
        LayoutCmd::Show { .. } | LayoutCmd::Cycle { .. } => {}
        LayoutCmd::Split { terminal, side, preset, .. } => {
            update.side = parse_side(side)? as i32;
            update.command_preset = preset.clone();
            if let Some(given) = terminal {
                update.target = Some(pick(given)?);
            }
        }
        LayoutCmd::Move { terminal, onto, side, .. } => {
            update.terminals = vec![pick(terminal)?];
            update.target = Some(pick(onto)?);
            update.side = parse_side(side)? as i32;
        }
        LayoutCmd::Preset { preset, .. } => {
            update.preset =
                Some(parse_preset(preset).ok_or_else(|| unknown_preset(preset))? as i32);
        }
        LayoutCmd::Focus { terminal, prev, pane, .. } => match (terminal, pane) {
            (Some(given), _) => update.focus = Some(pick(given)?),
            (None, Some(n)) => update.pane = Some(*n),
            (None, None) => update.step = Some(if *prev { -1 } else { 1 }),
        },
        LayoutCmd::Zoom { terminal, off, .. } => {
            update.unzoom = *off;
            if let Some(given) = terminal {
                update.zoom = Some(pick(given)?);
            }
        }
        LayoutCmd::Swap { a, b, .. } => {
            update.terminals = vec![pick(a)?, pick(b)?];
        }
        LayoutCmd::Resize { terminal, side, cells, .. } => {
            update.target = Some(pick(terminal)?);
            update.side = parse_side(side)? as i32;
            update.resize = Some(*cells);
        }
        LayoutCmd::Break { terminal, .. } => {
            if let Some(given) = terminal {
                update.target = Some(pick(given)?);
            }
        }
        LayoutCmd::Rename { name, .. } => update.name = name.clone(),
        LayoutCmd::Viewport { columns, rows, .. } => {
            update.columns = Some(*columns);
            update.rows = Some(*rows);
        }
        LayoutCmd::Select { group, prev, .. } => match group {
            Some(given) => {
                let existing = fetch_layout(&mut link, workspace_id).await?;
                let found = match given.parse::<usize>() {
                    Ok(n) if n >= 1 && n <= existing.len() => existing[n - 1].id.clone(),
                    _ => existing
                        .iter()
                        .find(|g| g.name.eq_ignore_ascii_case(given) || g.id == *given)
                        .map(|g| g.id.clone())
                        .ok_or_else(|| format!("no layout matching \"{given}\""))?,
                };
                update.group_id = found;
            }
            None => update.step = Some(if *prev { -1 } else { 1 }),
        },
    }

    let request = if matches!(cmd, LayoutCmd::Show { .. }) {
        req_for(method, workspace_id)
    } else {
        with(req_for(method, workspace_id), request::Payload::LayoutUpdate(update))
    };
    let r = link.call(request).await?;
    let result::Value::PaneGroupList(list) = expect_value(r.value, "layout")? else {
        return Err("the daemon returned the wrong resource".into());
    };

    // Re-read: a split creates a terminal the first listing did not have.
    let terminals = list_terminals(&mut link, Some(workspace_id)).await?;
    print_layout(&list, &terminals, json);
    Ok(())
}

/// A drop edge by name.
fn parse_side(text: &str) -> Result<farcooler_protocol::v1::SplitSide, String> {
    use farcooler_protocol::v1::SplitSide;
    Ok(match text.trim().to_ascii_lowercase().as_str() {
        "left" | "l" => SplitSide::Left,
        "right" | "r" => SplitSide::Right,
        "top" | "up" | "u" | "above" => SplitSide::Top,
        "bottom" | "down" | "d" | "below" => SplitSide::Bottom,
        other => return Err(format!("unknown side `{other}`; try left, right, top or bottom")),
    })
}

fn print_layout(
    list: &farcooler_protocol::v1::PaneGroupList,
    terminals: &[Terminal],
    json: bool,
) {
    if json {
        println!("{}", layout_json(list, terminals));
        return;
    }
    if list.items.is_empty() {
        println!("nothing tiled");
        return;
    }
    for (position, group) in list.items.iter().enumerate() {
        println!(
            "{} {}. {}  {}x{}  {} pane{}",
            if group.active { "*" } else { " " },
            position + 1,
            group.name,
            group.columns,
            group.rows,
            group.panes.len(),
            if group.panes.len() == 1 { "" } else { "s" },
        );
        for (index, pane) in group.panes.iter().enumerate() {
            let title = terminals
                .iter()
                .find(|t| t.id == pane.terminal_id)
                .map(label)
                .unwrap_or_else(|| "?".into());
            let marks = format!(
                "{}{}",
                if pane.focused { ">" } else { " " },
                if pane.zoomed { "z" } else { " " },
            );
            println!(
                "   {marks} {}  {}  {:>3},{:<3} {:>3}x{:<3}  {}",
                index + 1,
                short_bytes(&pane.terminal_id),
                pane.left,
                pane.top,
                pane.columns,
                pane.rows,
                truncate(&title, 30)
            );
        }
    }
}

fn unknown_preset(text: &str) -> String {
    format!(
        "unknown layout `{text}`; try even-horizontal, even-vertical, \
         main-vertical, main-horizontal or tiled"
    )
}

async fn fetch_layout(
    link: &mut Link,
    workspace: Uuid,
) -> Result<Vec<farcooler_protocol::v1::PaneGroup>, Box<dyn std::error::Error>> {
    let r = link.call(req_for("layout.list", workspace)).await?;
    match expect_value(r.value, "layout")? {
        result::Value::PaneGroupList(l) => Ok(l.items),
        _ => Err("the daemon returned the wrong resource".into()),
    }
}

fn layout_json(
    list: &farcooler_protocol::v1::PaneGroupList,
    terminals: &[Terminal],
) -> serde_json::Value {
    serde_json::json!({
        "workspace": uuid_of(&list.workspace_id).to_string(),
        "groups": list.items.iter().map(|g| serde_json::json!({
            "id": g.id,
            "name": g.name,
            "active": g.active,
            "columns": g.columns,
            "rows": g.rows,
            "layout": g.layout,
            "panes": g.panes.iter().map(|p| serde_json::json!({
                "id": uuid_of(&p.terminal_id).to_string(),
                "short": short_bytes(&p.terminal_id),
                "left": p.left, "top": p.top,
                "columns": p.columns, "rows": p.rows,
                "focused": p.focused, "zoomed": p.zoomed,
                "title": terminals.iter().find(|t| t.id == p.terminal_id).map(label),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

// ---------------------------------------------------------------------------
// Terminals: records through the daemon, bytes through tmux
// ---------------------------------------------------------------------------

async fn terminal(runner: Option<&str>, cmd: TerminalCmd, json: bool) -> Fallible {
    match cmd {
        // Record changes. These write durable intent, so they go to the daemon.
        TerminalCmd::Create { workspace, preset, title, tile } => {
            let mut link = connect_to(runner).await?;
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            let title = title.unwrap_or_else(|| preset.clone());
            let r = link
                .call(with(
                    req_for("terminal.create", uuid_of(&ws.id)),
                    request::Payload::TerminalCreate(farcooler_protocol::v1::TerminalCreate {
                        title,
                        command_preset: preset,
                        join_active_group: tile,
                    }),
                ))
                .await?;
            let result::Value::Terminal(t) = expect_value(r.value, "terminal")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            println!("created terminal {}  {}", short_bytes(&t.id), t.title);
        }

        TerminalCmd::Stop { terminal } => {
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;
            link.call(req_for("terminal.stop", id)).await?;
            println!("stopped {}", short(id));
        }

        TerminalCmd::DismissLost { terminal } => {
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;
            link.call(req_for("terminal.dismiss_lost", id)).await?;
            println!("dismissed {}", short(id));
        }

        TerminalCmd::Remove { terminal } => {
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;
            link.call(req_for("terminal.remove", id)).await?;
            println!("removed {}", short(id));
        }

        TerminalCmd::Seen { terminal } => {
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;
            link.call(req_for("terminal.seen", id)).await?;
            println!("marked {} seen", short(id));
        }

        // Agent channel. Every payload here names its own `terminal_id`
        // rather than the envelope's `target_resource_id` — see the matching
        // comment in `crates/daemon/src/rpc.rs` — so `req(method)` plus
        // `with(...)` is right, not `req_for`.
        TerminalCmd::SetPaneMode { terminal, mode, force } => {
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;
            let pane_mode = match mode.as_str() {
                "terminal" => farcooler_protocol::v1::PaneMode::Terminal,
                "agent" => farcooler_protocol::v1::PaneMode::Agent,
                other => {
                    return Err(format!("unknown pane mode {other:?}, want terminal or agent").into());
                }
            };
            link.call(with(
                req("terminal.set_pane_mode"),
                request::Payload::SetPaneMode(farcooler_protocol::v1::SetPaneMode {
                    terminal_id: id_bytes(id),
                    pane_mode: pane_mode as i32,
                    force,
                }),
            ))
            .await?;
            println!("{} is now in {mode} mode", short(id));
        }

        TerminalCmd::AgentSubscribe { terminal, from_seq, epoch } => {
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;
            let r = link
                .call(with(
                    req("terminal.agent_subscribe"),
                    request::Payload::AgentSubscribe(farcooler_protocol::v1::AgentSubscribe {
                        epoch,
                        terminal_id: id_bytes(id),
                        from_seq,
                    }),
                ))
                .await?;
            let result::Value::AgentEventBatch(batch) = expect_value(r.value, "agent event batch")?
            else {
                return Err("the daemon returned the wrong resource".into());
            };
            if json {
                // `AgentStream.swift`'s `Batch`/`EventFrame` decode with the
                // stock `JSONDecoder` — no snake_case conversion configured —
                // so these keys must be exactly `events`/`seq`/`payloadJson`.
                // Renaming any of them here does not fail to compile, it just
                // makes the Mac app silently drop every batch it receives.
                println!(
                    "{}",
                    serde_json::json!({
                        "epoch": batch.epoch,
                        "events": batch.events.iter().map(|e| serde_json::json!({
                            "seq": e.seq,
                            "payloadJson": e.payload_json,
                        })).collect::<Vec<_>>(),
                    })
                );
            } else if batch.events.is_empty() {
                println!("no new agent events");
            } else {
                for e in &batch.events {
                    println!("{:>6}  {}", e.seq, e.payload_json);
                }
            }
        }

        // No `proxy()` arm: this is an RPC, not a byte stream, so `connect_to`
        // already reaches a remote daemon over ssh — the same reason `seen` and
        // `stop` have none.
        TerminalCmd::PasteFile { terminal, file } => {
            let data = std::fs::read(&file)?;
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;
            let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("file");
            let path = farcooler_client::actions::paste_file(
                link.client_mut(),
                id,
                name,
                mime_for(&file),
                &data,
                // Progress goes to stderr: stdout is the data channel, and the
                // one thing a caller wants from it is the path.
                |sent, total| {
                    if sent < total {
                        eprintln!("sent {sent}/{total} bytes");
                    }
                },
            )
            .await?;
            println!("{path}");
        }

        TerminalCmd::AgentPrompt { terminal, text, images } => {
            use farcooler_protocol::v1::agent_prompt_block::Content;
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;

            let mut blocks = Vec::new();
            for path in &images {
                let data = std::fs::read(path)?;
                blocks.push(farcooler_protocol::v1::AgentPromptBlock {
                    content: Some(Content::Image(farcooler_protocol::v1::ImageBlock {
                        // From the extension, because that is all a file gives
                        // us and the adapter only needs to know how to decode.
                        mime_type: mime_for(path).to_string(),
                        data: bytes::Bytes::from(data),
                    })),
                });
            }
            blocks.push(farcooler_protocol::v1::AgentPromptBlock {
                content: Some(Content::Text(text)),
            });

            link.call(with(
                req("terminal.agent_prompt"),
                request::Payload::AgentPrompt(farcooler_protocol::v1::AgentPrompt {
                    terminal_id: id_bytes(id),
                    blocks,
                }),
            ))
            .await?;
            println!("sent to {}", short(id));
        }

        TerminalCmd::AgentAnswer { terminal, request_id, option_id } => {
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;
            link.call(with(
                req("terminal.agent_answer"),
                request::Payload::AgentAnswer(farcooler_protocol::v1::AgentAnswer {
                    terminal_id: id_bytes(id),
                    request_id,
                    option_id,
                }),
            ))
            .await?;
            println!("answered {}", short(id));
        }

        TerminalCmd::AgentSetMode { terminal, agent_mode } => {
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;
            link.call(with(
                req("terminal.agent_set_mode"),
                request::Payload::AgentSetMode(farcooler_protocol::v1::AgentSetMode {
                    terminal_id: id_bytes(id),
                    agent_mode: agent_mode.clone(),
                }),
            ))
            .await?;
            println!("set {} to agent mode {agent_mode}", short(id));
        }

        TerminalCmd::AgentSetModel { terminal, model } => {
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;
            link.call(with(
                req("terminal.agent_set_model"),
                request::Payload::AgentSetModel(farcooler_protocol::v1::AgentSetModel {
                    terminal_id: id_bytes(id),
                    model: model.clone(),
                }),
            ))
            .await?;
            println!("set {} to agent mode {model}", short(id));
        }

        TerminalCmd::AgentSetConfig { terminal, config_id, value } => {
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;
            link.call(with(
                req("terminal.agent_set_config"),
                request::Payload::AgentSetConfig(farcooler_protocol::v1::AgentSetConfig {
                    terminal_id: id_bytes(id),
                    config_id: config_id.clone(),
                    value: value.clone(),
                }),
            ))
            .await?;
            println!("set {} {config_id} to {value}", short(id));
        }

        TerminalCmd::AgentEditQueued { terminal, queued_id, text } => {
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;
            link.call(with(
                req("terminal.agent_edit_queued"),
                request::Payload::AgentEditQueued(farcooler_protocol::v1::AgentEditQueued {
                    terminal_id: id_bytes(id),
                    queued_id: queued_id.clone(),
                    text: text.clone(),
                }),
            ))
            .await?;
            println!("edited queued message {queued_id} on {}", short(id));
        }

        TerminalCmd::AgentCancelQueued { terminal, queued_id } => {
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;
            link.call(with(
                req("terminal.agent_cancel_queued"),
                request::Payload::AgentCancelQueued(farcooler_protocol::v1::AgentCancelQueued {
                    terminal_id: id_bytes(id),
                    queued_id: queued_id.clone(),
                }),
            ))
            .await?;
            println!("withdrew queued message {queued_id} on {}", short(id));
        }

        TerminalCmd::AgentSteerQueued { terminal, queued_id } => {
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;
            link.call(with(
                req("terminal.agent_steer_queued"),
                request::Payload::AgentSteerQueued(farcooler_protocol::v1::AgentSteerQueued {
                    terminal_id: id_bytes(id),
                    queued_id: queued_id.clone(),
                }),
            ))
            .await?;
            println!("sent queued message {queued_id} into the running turn on {}", short(id));
        }

        TerminalCmd::AgentCancel { terminal } => {
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;
            link.call(with(
                req("terminal.agent_cancel"),
                request::Payload::AgentCancel(farcooler_protocol::v1::AgentCancel {
                    terminal_id: id_bytes(id),
                }),
            ))
            .await?;
            println!("cancelled {}", short(id));
        }

        TerminalCmd::Restart { terminal } => {
            let (mut link, id) = terminal_by_record(runner, &terminal).await?;
            let r = link.call(req_for("terminal.restart", id)).await?;
            let result::Value::Terminal(t) = expect_value(r.value, "terminal")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            println!("restarted {} as epoch {}", short_bytes(&t.id), t.epoch);
        }

        // Live bytes. No daemon and no database — tmux is the authority, and
        // holding a request/response conversation open for the life of a
        // stream would be the wrong shape for both.
        //
        // On a remote runner they run over their own ssh session, hitting the
        // runner's own CLI. ssh is already a byte pipe and so are these, so the
        // honest implementation is to connect the two and get out of the way.
        TerminalCmd::Send { terminal, data } if runner.is_some() => {
            return proxy(runner, &["terminal".into(), "send".into(), terminal, data]).await;
        }
        TerminalCmd::SendHex { terminal, hex } if runner.is_some() => {
            return proxy(runner, &["terminal".into(), "send-hex".into(), terminal, hex]).await;
        }
        TerminalCmd::Stream { terminal } if runner.is_some() => {
            return proxy(runner, &["terminal".into(), "stream".into(), terminal]).await;
        }
        TerminalCmd::Input { terminal } if runner.is_some() => {
            return proxy(runner, &["terminal".into(), "input".into(), terminal]).await;
        }
        TerminalCmd::Screen { terminal } if runner.is_some() => {
            let mut args = vec!["terminal".into(), "screen".into(), terminal];
            if json {
                args.push("--json".into());
            }
            return proxy(runner, &args).await;
        }
        TerminalCmd::Resize { terminal, columns, rows } if runner.is_some() => {
            return proxy(
                runner,
                &["terminal".into(), "resize".into(), terminal, columns.to_string(), rows.to_string()],
            )
            .await;
        }
        TerminalCmd::Read { terminal, lines } if runner.is_some() => {
            return proxy(
                runner,
                &["terminal".into(), "read".into(), terminal, "--lines".into(), lines.to_string()],
            )
            .await;
        }

        TerminalCmd::Send { terminal, data } => {
            let runtime = Runtime::open().await?;
            let id = runtime.resolve_terminal(&terminal)?;
            runtime.send_input(id, &data).await?;
            println!("sent {} bytes", data.len());
        }

        TerminalCmd::SendHex { terminal, hex } => {
            let runtime = Runtime::open().await?;
            let id = runtime.resolve_terminal(&terminal)?;
            runtime.send_bytes_hex(id, &hex).await?;
        }

        TerminalCmd::Stream { terminal } => {
            let runtime = Runtime::open().await?;
            let id = runtime.resolve_terminal(&terminal)?;
            runtime.stream(id).await?;
        }

        TerminalCmd::Input { terminal } => {
            let runtime = Runtime::open().await?;
            let id = runtime.resolve_terminal(&terminal)?;
            runtime.input_channel(id).await?;
        }

        TerminalCmd::Screen { terminal } => {
            let runtime = Runtime::open().await?;
            let id = runtime.resolve_terminal(&terminal)?;
            let (text, cols, rows) = runtime.screen(id).await?;
            if json {
                println!(
                    "{}",
                    serde_json::json!({ "screen": text, "columns": cols, "rows": rows })
                );
            } else {
                print!("{text}");
            }
        }

        TerminalCmd::Resize { terminal, columns, rows } => {
            let runtime = Runtime::open().await?;
            let id = runtime.resolve_terminal(&terminal)?;
            runtime.resize_terminal(id, columns, rows).await?;
        }

        TerminalCmd::Read { terminal, lines } => {
            let runtime = Runtime::open().await?;
            let id = runtime.resolve_terminal(&terminal)?;
            print!("{}", runtime.capture(id, lines).await?);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Worktree file search: a daemon RPC, so — like the agent channel above and
// unlike the tmux-backed reads further up — `connect_to(runner)` alone reaches
// a remote runner correctly and no `proxy()` is needed.
// ---------------------------------------------------------------------------

async fn worktree(runner: Option<&str>, cmd: WorktreeCmd, json: bool) -> Fallible {
    match cmd {
        WorktreeCmd::FileSearch { workspace, query, limit } => {
            let mut link = connect_to(runner).await?;
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            let id = uuid_of(&ws.id);
            let r = link
                .call(with(
                    req("worktree.file_search"),
                    request::Payload::WorktreeFileSearch(farcooler_protocol::v1::WorktreeFileSearch {
                        workspace_id: id_bytes(id),
                        query,
                        limit,
                    }),
                ))
                .await?;
            let result::Value::WorktreeFileList(list) = expect_value(r.value, "worktree file list")?
            else {
                return Err("the daemon returned the wrong resource".into());
            };
            if json {
                println!("{}", serde_json::json!({ "paths": list.paths }));
            } else if list.paths.is_empty() {
                println!("no matches");
            } else {
                for p in &list.paths {
                    println!("{p}");
                }
            }
        }
    }
    Ok(())
}

/// Run a command on the runner's own CLI and adopt its exit status.
async fn proxy(runner: Option<&str>, args: &[String]) -> Fallible {
    let Some(target) = runner else { return Ok(()) };
    let code = remote::exec(target, args, false).await?;
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

/// Resolve a terminal against the daemon's RECORDS, not the live panes.
///
/// Stopping, restarting or dismissing a terminal has to work on one that is
/// already dead, which is precisely when it has no pane to be found by.
async fn terminal_by_record(
    runner: Option<&str>,
    prefix: &str,
) -> Result<(Link, Uuid), Box<dyn std::error::Error>> {
    let mut link = connect_to(runner).await?;
    let terminals = list_terminals(&mut link, None).await?;
    let t = resolve(&terminals, prefix, |t| &t.id, "terminal")?;
    let id = uuid_of(&t.id);
    Ok((link, id))
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

async fn host_get(
    link: &mut Link,
) -> Result<farcooler_protocol::v1::Host, Box<dyn std::error::Error>> {
    let r = link.call(req("host.health")).await?;
    match expect_value(r.value, "host")? {
        result::Value::Host(h) => Ok(h),
        _ => Err("the daemon returned the wrong resource".into()),
    }
}

async fn list_roots(link: &mut Link) -> Result<Vec<RepositoryRoot>, Box<dyn std::error::Error>> {
    let r = link.call(req("repository_root.list")).await?;
    match expect_value(r.value, "roots")? {
        result::Value::RepositoryRootList(l) => Ok(l.items),
        _ => Err("the daemon returned the wrong list".into()),
    }
}

pub(crate) async fn list_repositories(link: &mut Link) -> Result<Vec<Repository>, Box<dyn std::error::Error>> {
    let r = link.call(req("repository.list")).await?;
    match expect_value(r.value, "repositories")? {
        result::Value::RepositoryList(l) => Ok(l.items),
        _ => Err("the daemon returned the wrong list".into()),
    }
}

async fn list_themes(
    link: &mut Link,
) -> Result<Vec<farcooler_protocol::v1::Theme>, Box<dyn std::error::Error>> {
    let r = link.call(req("theme.list")).await?;
    match expect_value(r.value, "themes")? {
        result::Value::ThemeList(l) => Ok(l.items),
        _ => Err("the daemon returned the wrong list".into()),
    }
}

async fn list_workspaces(link: &mut Link) -> Result<Vec<Workspace>, Box<dyn std::error::Error>> {
    let r = link.call(req("workspace.list")).await?;
    match expect_value(r.value, "workspaces")? {
        result::Value::WorkspaceList(l) => Ok(l.items),
        _ => Err("the daemon returned the wrong list".into()),
    }
}

async fn list_terminals(
    link: &mut Link,
    workspace: Option<Uuid>,
) -> Result<Vec<Terminal>, Box<dyn std::error::Error>> {
    let request = match workspace {
        Some(id) => req_for("terminal.list", id),
        None => req("terminal.list"),
    };
    let r = link.call(request).await?;
    match expect_value(r.value, "terminals")? {
        result::Value::TerminalList(l) => Ok(l.items),
        _ => Err("the daemon returned the wrong list".into()),
    }
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// UUIDv7 is time-ordered, so its LEADING hex is a timestamp that is identical
/// for anything created in the same millisecond. Prefix matching on the head is
/// useless. The trailing bytes are random, so short ids use those.
fn short(id: Uuid) -> String {
    let s = id.simple().to_string();
    s[s.len() - 8..].to_string()
}

pub(crate) fn uuid_of(bytes: &[u8]) -> Uuid {
    Uuid::from_slice(bytes).unwrap_or(Uuid::nil())
}

/// The inverse of `uuid_of`: an id going INTO a payload rather than out of
/// one, for the agent-channel and worktree-search messages that carry their
/// own id fields instead of using the envelope's `target_resource_id`.
pub(crate) fn id_bytes(id: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(id.as_bytes())
}

pub(crate) fn short_bytes(bytes: &[u8]) -> String {
    short(uuid_of(bytes))
}

pub(crate) fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "~"
    }
}

/// The agent's activity, as the daemon derived it.
///
/// Distinct from `state`, which is about the process. A Claude Code sitting at
/// a permission prompt and one halfway through a file edit are both `running`;
/// the difference between them is the reason to look at a fleet at all.
/// The pane mode, as a word rather than a number.
///
/// Same reason `activity_label` exists: a client switching on an integer would
/// hold a second copy of the enum and drift from it silently. An unknown or
/// unspecified mode is `terminal` — the mode that needs no ACP adapter and
/// always works, which is the right guess for an older daemon.
/// The image type, from the file's extension.
///
/// Not sniffed from the bytes: the adapter needs a MIME type to decode with,
/// every real attachment comes from a picker that named it, and a wrong guess
/// here fails loudly at the far end rather than corrupting anything.
fn mime_for(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_lowercase().as_str() {
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        _ => "image/jpeg",
    }
}

fn pane_mode_label(mode: i32) -> &'static str {
    match farcooler_protocol::v1::PaneMode::try_from(mode) {
        Ok(farcooler_protocol::v1::PaneMode::Agent) => "agent",
        Ok(farcooler_protocol::v1::PaneMode::Changes) => "changes",
        _ => "terminal",
    }
}

fn activity_label(a: i32) -> &'static str {
    use farcooler_protocol::v1::AgentActivity;
    match AgentActivity::try_from(a).unwrap_or(AgentActivity::Unspecified) {
        AgentActivity::None => "none",
        AgentActivity::Idle => "idle",
        AgentActivity::Working => "working",
        AgentActivity::Blocked => "blocked",
        AgentActivity::Done => "done",
        AgentActivity::Unknown => "unknown",
        AgentActivity::Unspecified => "none",
    }
}

/// What to call a terminal's contents.
///
/// The daemon resolves this from the running process and the screen together,
/// so it says `claude` for a shell someone typed `claude` into. The preset is
/// only a fallback for a terminal the watcher has not sampled yet.
fn label(t: &farcooler_protocol::v1::Terminal) -> String {
    if t.current_command.is_empty() { t.command_preset.clone() } else { t.current_command.clone() }
}

/// When the activity last changed, as Unix milliseconds.
///
/// A client shows "working for 4m" from this rather than timing it locally,
/// which would restart at every reconnect and lie after a laptop sleeps.
fn activity_since(t: &farcooler_protocol::v1::Terminal) -> Option<i64> {
    t.activity_changed_at.as_ref().map(|ts| ts.seconds * 1000 + (ts.nanos as i64) / 1_000_000)
}

/// When the current turn started, as Unix milliseconds.
///
/// Same conversion as `activity_since`, on a different clock: `turn_started_at`
/// is held across a permission prompt and cleared when the turn ends, so it
/// answers "how long has this been running" rather than "how long has it been
/// in this particular state". A client that shows only `activity_since` for a
/// Working row is answering the wrong question — see the Mac's
/// `Terminal.turnDuration`, which exists because a single clock already did
/// this once.
fn turn_started_at(t: &farcooler_protocol::v1::Terminal) -> Option<i64> {
    t.turn_started_at.as_ref().map(|ts| ts.seconds * 1000 + (ts.nanos as i64) / 1_000_000)
}

/// One terminal, projected for a client — the shape `WorkspaceCmd::List` and
/// `terminal_event_json` both need and must agree on.
///
/// Pulled out for the exact reason `terminal_event_json` was: this was
/// inline in `workspace()`'s `json!` closure until the exit code, the turn
/// clock and the blocked question were all found missing from it in the same
/// afternoon — the THIRD time this file has built a terminal's client-facing
/// JSON by hand and left a field out, after `chatCapable` and then
/// `exitCode`/`exitSignal` on the event side. This is the one the Mac app's
/// `refresh()` actually calls (`workspace list --json`) — `crates/client`
/// builds a separate projection for iOS, and it is not what this app reads,
/// however alike the two look.
fn workspace_list_terminal_json(t: &farcooler_protocol::v1::Terminal) -> serde_json::Value {
    serde_json::json!({
        "id": uuid_of(&t.id).to_string(),
        "short": short_bytes(&t.id),
        "title": t.title,
        "preset": label(t),
        "state": terminal_label(t.state()),
        "activity": activity_label(t.activity),
        "activitySince": activity_since(t),
        // How it ENDED, which is the difference between a shell you closed
        // and a build that broke — see `crates/core::activity::exit_wants_attention`.
        "exitCode": t.exit_status.as_ref().and_then(|e| e.code),
        "exitSignal": t.exit_status.as_ref().and_then(|e| e.signal),
        // The other clock, held across a permission prompt rather than reset
        // by one. Sent beside `activitySince` because they answer different
        // questions and a client that only had one of them was answering the
        // wrong one for half of its rows.
        "turnStartedAt": turn_started_at(t),
        // What the agent is asking, when it is blocked and legible. Without
        // this a row can say "Needs you" and never say what for.
        "blockedQuestion": t.blocked_question,
        // The last three things it SAID, in the agent's own words. Already
        // redacted and already cut to a row's width — a client renders these
        // and decides nothing about them.
        "feed": t.feed,
        // The last of those messages WHOLE and from its start, which is what a
        // notification quotes. Not derivable from `feed`: those are wrapped
        // rows, so their last entry is the end of the window rather than the
        // beginning of a sentence — see `farcooler_core::feed::Feed::said`.
        // This is the app whose `Notifier` reads it.
        "said": t.said,
        // The agents it spawned and has not finished with, named. On the same
        // terms as the feed, and here for the same reason the rungs are: a
        // field on the wire and missing from a projection is this function's
        // own three strikes, and the fix is to plumb it when it is added.
        "subagents": t.subagents,
        // The compact ladder, in the daemon's words as well. Projected even
        // though nothing here renders them yet, on purpose: the rungs exist so
        // a Live Activity can pick the one its surface has room for, and the
        // client that goes looking for them will look in exactly the two
        // projections this file builds. A field present on the wire and
        // dropped here is precisely the shape of this function's own three
        // strikes — the fix is to plumb them when they are added, not when
        // somebody finally reads them.
        "glyph": t.glyph,
        "headline": t.headline,
        "line": t.line,
        "rank": t.rank,
        // How the last turn ENDED, which `activity` alone cannot say: a turn
        // that died and one that succeeded are both `done` there. Rendered by
        // the app rather than only by the ladder, because this is the app that
        // draws its own indicator off `activity` — see `Terminal.status`.
        "turnFailed": t.turn_failed,
        "epoch": t.epoch,
        // The Mac app decodes THIS, not the JSON
        // `crates/client` builds for iOS. Leaving
        // these out here left `isAgentPane` false
        // forever, so an agent pane silently drew
        // a terminal and chat mode looked missing.
        "paneMode": pane_mode_label(t.pane_mode),
        "chatCapable": t.chat_capable,
        "agentSessionId": t.agent_session_id,
        "agentMode": t.agent_mode,
        "availableAgentModes": t.available_agent_modes,
    })
}

/// The JSON line `events` pushes for a `TerminalChanged` message.
///
/// A free function rather than an inline object literal in the event loop:
/// this is exactly the shape `chatCapable` went missing from. The daemon
/// broadcasts it (see `watch.rs`) and `list --json` prints it, but this
/// projection built its own object field by field and simply left it out —
/// so a shell pane the user typed `codex` into relabeled itself live from
/// this same event, while `canSwitchPaneMode` on the client stayed false
/// forever, because the app is push-only and never re-fetches a terminal it
/// already knows. `⌃B a` then refused, on the exact agent this branch
/// shipped an adapter for. Pulling the object out where a test can call it
/// directly is what keeps the next field from going missing the same way —
/// which it did not, twice more: `exitCode`/`exitSignal` were missing here
/// AND, it turned out, `list --json` had never had them either; `turnStartedAt`
/// and `blockedQuestion` were missing from both at once. Three strikes in one
/// function is why `workspace_list_terminal_json` above now exists as a named,
/// tested thing instead of a second hand-built object this one could drift
/// from again.
fn terminal_event_json(t: &farcooler_protocol::v1::Terminal) -> serde_json::Value {
    serde_json::json!({
        "kind": "terminal",
        "id": uuid_of(&t.id).to_string(),
        "short": short_bytes(&t.id),
        "workspace": uuid_of(&t.workspace_id).to_string(),
        "title": t.title,
        "preset": label(t),
        "state": terminal_label(t.state()),
        "activity": activity_label(t.activity),
        "activitySince": activity_since(t),
        // Watched for the same reason `chatCapable` below is: a live push that
        // moves a terminal to `exited` without this leaves the client unable
        // to tell a shell you closed from a build that broke until something
        // else forces a full re-read — which is exactly the bug this
        // function's own history is a list of.
        "exitCode": t.exit_status.as_ref().and_then(|e| e.code),
        "exitSignal": t.exit_status.as_ref().and_then(|e| e.signal),
        // Watched for the identical reason, one tick later: a row that just
        // went Blocked over this event has a question to show NOW, not after
        // whatever next forces a full refresh — and the turn clock is exactly
        // what tells "Working 12m" apart from "Working 2s" the moment a
        // permission prompt resolves.
        "turnStartedAt": turn_started_at(t),
        "blockedQuestion": t.blocked_question,
        // Watched for the same reason again, and this is the field with the
        // shortest useful life of any of them: a line is news for as long as
        // the agent is on it, and a transcript that only arrived on a full
        // refresh would always be describing the previous minute.
        "feed": t.feed,
        // Watched for a sharper version of the same reason: this is the
        // sentence the app's own `Notifier` puts in a banner the instant a
        // turn ends, and the event carrying `activity: done` is the one that
        // makes it fire. Arriving only on the next full refresh would mean the
        // banner quoting the turn before it.
        "said": t.said,
        // Watched for the identical reason, one field along: a subagent that
        // reached a row only at the next full refresh would be finished before
        // anyone saw it start.
        "subagents": t.subagents,
        // Watched for the same reason the feed is: every rung is derived from
        // `activity`, `blockedQuestion` and `feed`, all of which move on this
        // event, so a ladder that only arrived on a full refresh would be
        // describing a state the terminal has already left.
        "glyph": t.glyph,
        "headline": t.headline,
        "line": t.line,
        "rank": t.rank,
        // Watched for the same reason the ladder is, and it moves on exactly
        // the event the ladder does: the tick a turn ends. A row that learned
        // about the failure only on a full refresh would show a clean `Done`
        // for as long as nothing else happened in that pane — which, for a
        // pane whose agent has just died, is indefinitely.
        "turnFailed": t.turn_failed,
        // Watched as well as listed. A pane mode that only arrived on a full
        // refresh would mean toggling to chat did nothing until something
        // else happened to reload the fleet.
        "paneMode": pane_mode_label(t.pane_mode),
        // Watched for the identical reason — see the function comment above.
        "chatCapable": t.chat_capable,
        "agentSessionId": t.agent_session_id,
        "agentMode": t.agent_mode,
        "availableAgentModes": t.available_agent_modes,
    })
}

fn workspace_label(s: WorkspaceState) -> &'static str {
    match s {
        WorkspaceState::Unspecified => "?",
        WorkspaceState::Creating => "creating",
        WorkspaceState::Ready => "ready",
        WorkspaceState::Active => "active",
        WorkspaceState::Error => "ERROR",
        WorkspaceState::Hidden => "hidden",
        WorkspaceState::WorktreeMissing => "worktree_missing",
    }
}

fn terminal_label(s: TerminalState) -> &'static str {
    match s {
        TerminalState::Unspecified => "?",
        TerminalState::Starting => "starting",
        TerminalState::Running => "running",
        TerminalState::Exited => "exited",
        TerminalState::Error => "error",
        TerminalState::Lost => "LOST",
        // Lower case, unlike LOST. LOST shouts because it is a finding that
        // wants a decision from you; this is the runner being unreadable for a
        // moment, and it usually resolves before anyone could act on it.
        TerminalState::Unknown => "unknown",
    }
}

/// Normalize an id for matching.
///
/// Ids are compared in their dashless form, so a short id and a full hyphenated
/// UUID both work. Without this, pasting an id straight out of `--json` — which
/// is hyphenated — matched nothing, because the stored form is not.
fn normalize_id(text: &str) -> String {
    text.trim().to_lowercase().replace('-', "")
}

/// Resolve a repository by name, or by id like everything else.
///
/// Names first, because a repository is the one resource people know by name:
/// they typed it when they registered it, they see it in every listing, and
/// `farcooler workspace discover myrepo` is what anyone would write. Ids still
/// work, and an ambiguous name is refused rather than guessed at — two projects
/// called `api` on one runner is a thing that happens.
pub(crate) fn resolve_repository<'a>(
    repositories: &'a [Repository],
    given: &str,
) -> Result<&'a Repository, String> {
    let needle = given.trim().to_lowercase();
    let by_name: Vec<&Repository> = repositories
        .iter()
        .filter(|r| r.display_name.to_lowercase() == needle)
        .collect();
    match by_name.len() {
        1 => return Ok(by_name[0]),
        n if n > 1 => {
            return Err(format!(
                "{n} repositories are called \"{given}\"; name one by id instead"
            ));
        }
        _ => {}
    }
    resolve(repositories, given, |r| &r.id, "repository")
}

/// Resolve a short id suffix, refusing an ambiguous match rather than guessing.
/// A workspace by id prefix or by task name.
pub(crate) async fn resolve_workspace_id(
    link: &mut Link,
    needle: &str,
) -> Result<Uuid, Box<dyn std::error::Error>> {
    let workspaces = list_workspaces(link).await?;
    // Name first: people type the task they gave it, and an id prefix is the
    // fallback rather than the other way round.
    let by_name: Vec<&Workspace> =
        workspaces.iter().filter(|w| w.task_name == needle).collect();
    if by_name.len() == 1 {
        return Ok(uuid_of(&by_name[0].id));
    }
    let w = resolve(&workspaces, needle, |w| &w.id, "workspace")?;
    Ok(uuid_of(&w.id))
}

fn resolve<'a, T>(
    items: &'a [T],
    prefix: &str,
    id_of: impl Fn(&T) -> &[u8],
    kind: &str,
) -> Result<&'a T, String> {
    let needle = normalize_id(prefix);
    let matches: Vec<&T> = items
        .iter()
        .filter(|i| uuid_of(id_of(i)).simple().to_string().ends_with(&needle))
        .collect();

    match matches.len() {
        1 => Ok(matches[0]),
        0 => Err(format!("no {kind} matching {prefix:?}")),
        n => Err(format!("{prefix:?} matches {n} {kind}s, be more specific")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminal_changed_event_carries_chat_capable() {
        // The regression this test exists to catch: `events` used to build
        // its own JSON object field by field and simply left `chatCapable`
        // out, so a codex pane that relabeled itself live from this exact
        // event never told the client it could be switched to chat.
        let t = farcooler_protocol::v1::Terminal { chat_capable: true, ..Default::default() };
        let json = terminal_event_json(&t);
        assert_eq!(json["chatCapable"], serde_json::json!(true));
    }

    #[test]
    fn a_chat_incapable_terminal_says_so_rather_than_omitting_the_key() {
        let t = farcooler_protocol::v1::Terminal { chat_capable: false, ..Default::default() };
        let json = terminal_event_json(&t);
        assert_eq!(json["chatCapable"], serde_json::json!(false));
    }

    #[test]
    fn a_terminal_changed_event_carries_the_exit_status() {
        // The same regression as `chatCapable`, one field later: a live push
        // that moves a terminal to `exited` is exactly the moment a client
        // needs to tell a clean exit from a failed one apart, and this
        // function had already left one field out of this same object before.
        let t = farcooler_protocol::v1::Terminal {
            exit_status: Some(farcooler_protocol::v1::ExitStatus { code: Some(101), signal: None }),
            ..Default::default()
        };
        let json = terminal_event_json(&t);
        assert_eq!(json["exitCode"], serde_json::json!(101));
        assert_eq!(json["exitSignal"], serde_json::json!(null));
    }

    #[test]
    fn a_terminal_changed_event_carries_the_turn_clock_and_the_question() {
        // The regression this exists to catch: a row that just went Blocked
        // over this exact event is the one moment "Needs you" and the
        // question under it both need to be true at once, and this function
        // had already left two other fields out of this same object twice
        // before.
        let t = farcooler_protocol::v1::Terminal {
            turn_started_at: Some(prost_types::Timestamp { seconds: 1_700_000_000, nanos: 0 }),
            blocked_question: Some("Overwrite config.toml?".to_string()),
            ..Default::default()
        };
        let json = terminal_event_json(&t);
        assert_eq!(json["turnStartedAt"], serde_json::json!(1_700_000_000_000_i64));
        assert_eq!(json["blockedQuestion"], serde_json::json!("Overwrite config.toml?"));
    }

    /// The other terminal-to-JSON function in this file, and the one the Mac
    /// app's `refresh()` actually calls (`workspace list --json`) — see the
    /// function's own doc comment for why that distinction matters. Every
    /// field this stage of the branch added is checked here, because this is
    /// the function where three of them turned out to be missing at once.
    #[test]
    fn the_full_list_carries_every_field_this_branch_added() {
        let t = farcooler_protocol::v1::Terminal {
            exit_status: Some(farcooler_protocol::v1::ExitStatus { code: Some(101), signal: None }),
            turn_started_at: Some(prost_types::Timestamp { seconds: 1_700_000_000, nanos: 0 }),
            blocked_question: Some("Overwrite config.toml?".to_string()),
            chat_capable: true,
            feed: vec!["Written to haiku.txt.".to_string(), "Both tests pass.".to_string()],
            subagents: vec!["Auditing the redaction rules".to_string()],
            ..Default::default()
        };
        let json = workspace_list_terminal_json(&t);
        assert_eq!(json["exitCode"], serde_json::json!(101));
        assert_eq!(json["exitSignal"], serde_json::json!(null));
        assert_eq!(json["turnStartedAt"], serde_json::json!(1_700_000_000_000_i64));
        assert_eq!(json["blockedQuestion"], serde_json::json!("Overwrite config.toml?"));
        // Not new this round, but this is the function `chatCapable` was
        // already known to be present in — kept as a canary so a future
        // refactor of this function trips a test rather than a support ticket.
        assert_eq!(json["chatCapable"], serde_json::json!(true));
        assert_eq!(json["feed"], serde_json::json!(["Written to haiku.txt.", "Both tests pass."]));
        assert_eq!(json["subagents"], serde_json::json!(["Auditing the redaction rules"]));
    }

    /// The feed, on the push path.
    ///
    /// A step is news for as long as the agent is on it. A feed that only
    /// arrived on a full refresh would always be describing the previous
    /// minute, which for the one field whose whole job is "what is it doing
    /// RIGHT NOW" is the same as not sending it.
    #[test]
    fn a_terminal_changed_event_carries_the_feed_and_the_subagents() {
        let t = farcooler_protocol::v1::Terminal {
            feed: vec!["Written to haiku.txt.".to_string(), "Both tests pass.".to_string()],
            subagents: vec!["Auditing the redaction rules".to_string()],
            ..Default::default()
        };
        let json = terminal_event_json(&t);
        assert_eq!(json["feed"], serde_json::json!(["Written to haiku.txt.", "Both tests pass."]));
        assert_eq!(json["subagents"], serde_json::json!(["Auditing the redaction rules"]));
    }

    /// A plain shell has nothing to report, and says so with an empty list
    /// rather than by omitting the key — a client that had to tell "no feed"
    /// from "feed missing" would be guessing.
    #[test]
    fn a_terminal_with_nothing_to_report_sends_an_empty_feed() {
        let t = farcooler_protocol::v1::Terminal::default();
        assert_eq!(terminal_event_json(&t)["feed"], serde_json::json!([]));
        assert_eq!(workspace_list_terminal_json(&t)["feed"], serde_json::json!([]));
        assert_eq!(terminal_event_json(&t)["subagents"], serde_json::json!([]));
        assert_eq!(workspace_list_terminal_json(&t)["subagents"], serde_json::json!([]));
    }

    /// Keys the EVENT projection carries that the list one has no business
    /// carrying: an event has to say what kind of thing changed and which
    /// workspace it is in, because it arrives on its own with no surrounding
    /// document. A list entry is already inside both.
    const EVENT_ONLY: &[&str] = &["kind", "workspace"];

    /// Keys the LIST projection carries that the event one deliberately does
    /// not. `epoch` is write-conflict bookkeeping for a client about to send a
    /// command, not something a row renders, and it was left off the event path
    /// on purpose.
    const LIST_ONLY: &[&str] = &["epoch"];

    /// The two projections of a `Terminal` must not drift apart again.
    ///
    /// This is the test that would have caught this branch's own worst bug, and
    /// the two before it. Three times a field was added to one of these
    /// functions and not the other — `chatCapable`, then
    /// `exitCode`/`exitSignal`, then `turnStartedAt`/`blockedQuestion`, that
    /// last pair missing from BOTH — and every suite stayed green while every
    /// headline feature of this branch was invisible on the shipped Mac app.
    ///
    /// Field-by-field assertions cannot catch that: they test the fields
    /// somebody remembered. This walks the KEY SETS, so a field added to one
    /// projection and forgotten in the other fails here with no test change at
    /// all. The only way past it is to name the new key in `EVENT_ONLY` or
    /// `LIST_ONLY` above, which is a deliberate act with a comment attached.
    #[test]
    fn the_two_terminal_projections_agree_on_every_field() {
        // Fully populated, because a projection that reads an absent optional
        // still emits its key — but a reader comparing the two by hand would
        // rather see real values in the failure message.
        let t = farcooler_protocol::v1::Terminal {
            exit_status: Some(farcooler_protocol::v1::ExitStatus { code: Some(101), signal: None }),
            turn_started_at: Some(prost_types::Timestamp { seconds: 1_700_000_000, nanos: 0 }),
            blocked_question: Some("Overwrite config.toml?".to_string()),
            chat_capable: true,
            feed: vec!["Written to haiku.txt.".to_string(), "Both tests pass.".to_string()],
            subagents: vec!["Auditing the redaction rules".to_string()],
            glyph: "?".to_string(),
            headline: "claude needs you".to_string(),
            line: "Overwrite config.toml?".to_string(),
            rank: 1,
            turn_failed: true,
            ..Default::default()
        };

        let keys = |value: &serde_json::Value, except: &[&str]| -> std::collections::BTreeSet<String> {
            value
                .as_object()
                .expect("a terminal projects to an object")
                .keys()
                .filter(|k| !except.contains(&k.as_str()))
                .cloned()
                .collect()
        };
        let event = keys(&terminal_event_json(&t), EVENT_ONLY);
        let list = keys(&workspace_list_terminal_json(&t), LIST_ONLY);

        assert_eq!(
            event, list,
            "the two terminal projections disagree.\n\
             only in the event JSON: {:?}\n\
             only in `workspace list --json`: {:?}\n\
             Add the field to both, or name it in EVENT_ONLY/LIST_ONLY with a reason.",
            event.difference(&list).collect::<Vec<_>>(),
            list.difference(&event).collect::<Vec<_>>(),
        );

        // Not vacuous by construction either: an empty set equals an empty set,
        // so the branch's own fields are named here to prove the sets are real.
        for field in [
            "exitCode",
            "exitSignal",
            "turnStartedAt",
            "blockedQuestion",
            "chatCapable",
            "feed",
            "said",
            "subagents",
            "glyph",
            "headline",
            "line",
            "rank",
            "turnFailed",
        ] {
            assert!(event.contains(field), "{field} is in neither projection");
        }
    }

    #[test]
    fn a_terminal_with_no_turn_or_question_sends_neither_as_present() {
        // The ordinary case — idle, or working outside a permission prompt —
        // must not invent a clock or a question that is not there.
        let t = farcooler_protocol::v1::Terminal::default();
        assert_eq!(workspace_list_terminal_json(&t)["turnStartedAt"], serde_json::json!(null));
        assert_eq!(workspace_list_terminal_json(&t)["blockedQuestion"], serde_json::json!(null));
    }
}
