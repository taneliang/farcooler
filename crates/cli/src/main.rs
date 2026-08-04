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
mod host_install;
mod remote;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use daemon_link::{Link, connect_to, expect_value, req, req_for, with};
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
    // things read this expecting to tell builds apart: `host probe` captures it
    // from a remote machine, and `HostProbe.matchesThisMac` compares the two.
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

    /// Operate on another machine, as `user@host` or an ssh config alias.
    ///
    /// Runs the same protocol over ssh. There is no Far Cooler network
    /// listener: a host reachable by ssh is reachable by Far Cooler, and one
    /// that is not, is not.
    #[arg(long, global = true, value_name = "TARGET")]
    host: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show host and daemon facts.
    Status,
    /// Start, stop, or replace this machine's daemon.
    ///
    /// Local only. `--host` reaches another machine's daemon through ssh, and
    /// stopping one from here would take away the connection carrying the
    /// request; installing and inspecting a remote daemon is `host`'s job.
    #[command(subcommand)]
    Daemon(DaemonCmd),
    /// Manage allowlisted repository roots.
    #[command(subcommand)]
    Root(RootCmd),
    /// Manage registered repositories.
    #[command(subcommand)]
    Repo(RepoCmd),
    /// Manage task workspaces (one worktree plus branch per task).
    #[command(subcommand)]
    Workspace(WorkspaceCmd),
    /// Manage terminals inside a workspace.
    #[command(subcommand)]
    Terminal(TerminalCmd),
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
    /// Pair this machine with your account so it can notify your devices.
    ///
    /// There is no sign-in here on purpose. The app does the signing in, asks
    /// the relay for a token that names nothing but the account, and hands that
    /// token to this machine over the ssh channel you already trust. So a
    /// headless Linux box never needs a browser, and the worst a leaked token
    /// can do is notify the phone of the person it was taken from.
    #[command(subcommand)]
    Push(PushCmd),
    /// Install or inspect Far Cooler on a Linux host over ssh.
    #[command(subcommand, name = "host")]
    HostCmd(HostCmd),
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
    /// Store the token the app issued for this machine. Reads it from stdin.
    ///
    /// STDIN, not an argument, and that is the whole reason this reads oddly.
    /// A bearer token in argv is a bearer token in `ps aux` — world-readable on
    /// a default Linux box — on both the machine running this and, over ssh, the
    /// machine being paired. It also ends up in shell history, in sshd's command
    /// logging, and in any error that echoes the command back. One observation
    /// by any local user is permanent: the token does not expire.
    Pair {
        /// A self-hosted relay, if you run your own.
        #[arg(long)]
        relay: Option<String>,
    },
    /// Say whether this machine is paired, without printing the token.
    Status,
    /// Forget the token. Notifications stop; nothing else changes.
    Forget,
}

#[derive(Subcommand)]
enum HostCmd {
    /// Copy the daemon and CLI to a Linux host and register a user service.
    Install {
        target: String,
        /// The Linux binaries to install. Defaults to ./dist/<arch>-linux/.
        #[arg(long)]
        from: Option<PathBuf>,
    },
    /// Report what is installed and running on a host.
    Status { target: String },
    /// Report what a host IS, changing nothing on it.
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
        task: String,
        #[arg(long)]
        branch: String,
        #[arg(long, default_value = "HEAD")]
        base: String,
    },
    /// Show the fleet with freshly derived state.
    List,
    /// Resume work on a branch that already exists.
    ///
    /// The branch may be remote-only — pushed from another machine, by a
    /// colleague, or by a cloud agent. A local tracking branch is created for
    /// it, so pushing back goes where it came from.
    Adopt {
        repo: String,
        branch: String,
        /// What to call it in the fleet. Defaults to the branch name.
        #[arg(long)]
        task: Option<String>,
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
    /// @-mention picker. Results are worktree-relative, never a host path —
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
    // `connect_to(host)` already reaches a remote daemon over ssh, and these
    // never touch the local tmux runtime a remote host does not have.
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

type Fallible = Result<(), Box<dyn std::error::Error>>;

/// Pair, unpair, or report — on this machine or, with `--host`, on another.
///
/// Forwarded over ssh rather than through the daemon protocol because a
/// credential should travel over the channel the user already authenticated,
/// not over one the daemon would have to be trusted to keep private.
async fn push(host: Option<&str>, cmd: PushCmd) -> Fallible {
    use farcooler_daemon::push::Pairing;
    // The daemon's, not a third copy. This crate already had two spellings of
    // shell quoting — `remote::shell_quote`, which passes safe arguments
    // through unquoted, and the daemon's, which always wraps — and a third one
    // on the path that forwards a credential over ssh is the one that gets it
    // wrong later.
    use farcooler_daemon::service::shell_quote;

    if let Some(target) = host {
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
                return host_install::remote_run_with_stdin(
                    target,
                    &format!("farcooler push pair{relay}"),
                    &token,
                )
                .await;
            }
            PushCmd::Status => return host_install::remote_run(target, "farcooler push status").await,
            PushCmd::Forget => return host_install::remote_run(target, "farcooler push forget").await,
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
            println!("forgotten · this machine will not notify anything");
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
        return Err("no token on stdin (the app pairs a machine for you)".into());
    }
    Ok(token)
}

async fn run() -> Fallible {
    let cli = Cli::parse();
    let host = cli.host.as_deref();
    match cli.command {
        Command::Status => status(host, cli.json).await,
        Command::Daemon(c) => daemon(host, c, cli.json).await,
        Command::Root(c) => root(host, c, cli.json).await,
        Command::Repo(c) => repo(host, c, cli.json).await,
        Command::Workspace(c) => workspace(host, c, cli.json).await,
        Command::Terminal(c) => terminal(host, c, cli.json).await,
        Command::Worktree(c) => worktree(host, c, cli.json).await,
        Command::Layout(c) => layout(host, c, cli.json).await,
        Command::Attach { workspace } => attach(host, &workspace).await,
        Command::Events => events(host).await,
        Command::Push(c) => push(host, c).await,
        Command::HostCmd(HostCmd::Install { target, from }) => {
            host_install::install(&target, from.as_deref()).await
        }
        Command::HostCmd(HostCmd::Status { target }) => host_install::status(&target).await,
        Command::HostCmd(HostCmd::Probe { target }) => {
            let probe = host_install::probe(&target).await?;
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
    }
}

/// Start, stop, or replace the daemon on THIS machine.
async fn daemon(host: Option<&str>, cmd: DaemonCmd, json: bool) -> Fallible {
    if host.is_some() {
        return Err("daemon manages this machine's daemon; drop --host".into());
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

async fn status(host: Option<&str>, json: bool) -> Fallible {
    let mut link = connect_to(host).await?;
    let roots = list_roots(&mut link).await?;
    let repos = list_repositories(&mut link).await?;
    let workspaces = list_workspaces(&mut link).await?;
    let terminals = list_terminals(&mut link, None).await?;
    let host_facts = host_get(&mut link).await?;
    let healthy = host_facts.self_health != farcooler_protocol::v1::SelfHealth::Degraded as i32;

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
                "platform": host_facts.platform,
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

    println!("host          {}", host.unwrap_or("local"));
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

    // The recovery command names a tmux socket on THIS machine, so it is only
    // meaningful locally. Printing a local socket path while reporting a remote
    // host would be an invitation to run the wrong thing.
    if host.is_none() {
        let runtime = Runtime::open().await?;
        println!();
        println!("recovery: {}", runtime.tmux.recovery_command());
    }
    Ok(())
}

async fn root(host: Option<&str>, cmd: RootCmd, json: bool) -> Fallible {
    let mut link = connect_to(host).await?;
    match cmd {
        RootCmd::Add { path } => {
            // Canonicalised here so the daemon is handed an absolute path
            // regardless of which directory the user ran this from.
            let absolute = path.canonicalize().unwrap_or(path);
            let r = link
                .call(with(
                    req("repository_root.add"),
                    request::Payload::RepositoryRootAdd(
                        farcooler_protocol::v1::RepositoryRootAdd {
                            absolute_path: absolute.to_string_lossy().into_owned(),
                            typed_confirmation: String::new(),
                        },
                    ),
                ))
                .await?;
            let result::Value::RepositoryRoot(root) = expect_value(r.value, "root")? else {
                return Err("the daemon returned the wrong resource".into());
            };
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

async fn repo(host: Option<&str>, cmd: RepoCmd, json: bool) -> Fallible {
    let mut link = connect_to(host).await?;
    match cmd {
        RepoCmd::Register { path } => {
            let absolute = path.canonicalize().unwrap_or(path);
            let r = link
                .call(with(
                    req("repository.register"),
                    request::Payload::RepositoryRegister(
                        farcooler_protocol::v1::RepositoryRegister {
                            relative_path: absolute.to_string_lossy().into_owned(),
                        },
                    ),
                ))
                .await?;
            let result::Value::Repository(repo) = expect_value(r.value, "repository")? else {
                return Err("the daemon returned the wrong resource".into());
            };
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

async fn workspace(host: Option<&str>, cmd: WorkspaceCmd, json: bool) -> Fallible {
    let mut link = connect_to(host).await?;
    match cmd {
        WorkspaceCmd::Create { repo, task, branch, base } => {
            let repos = list_repositories(&mut link).await?;
            let target = resolve_repository(&repos, &repo)?;
            let r = link
                .call(with(
                    req_for("workspace.create", uuid_of(&target.id)),
                    request::Payload::WorkspaceCreate(farcooler_protocol::v1::WorkspaceCreate {
                        task_name: task,
                        branch,
                        base_revision: base,
                        cli_preset: String::new(),
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
                            // Which machine. Empty means this one; a client
                            // merges several hosts into one fleet and needs to
                            // know where to route an action back to.
                            "host": host.unwrap_or_default(),
                            "repository": repositories.iter()
                                .find(|r| r.id == w.repository_id)
                                .map(|r| r.display_name.clone())
                                .unwrap_or_default(),
                            "worktree": w.worktree_path,
                            "state": workspace_label(w.state()),
                            "is_main_checkout": w.is_main_checkout,
                            "terminals": terminals.iter()
                                .filter(|t| t.workspace_id == w.id)
                                .map(|t| serde_json::json!({
                                    "id": uuid_of(&t.id).to_string(),
                                    "short": short_bytes(&t.id),
                                    "title": t.title,
                                    "preset": label(t),
                                    "state": terminal_label(t.state()),
                                    "activity": activity_label(t.activity),
                                    "activitySince": activity_since(t),
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
                                }))
                                .collect::<Vec<_>>(),
                        })
                    })
                    .collect();

                println!(
                    "{}",
                    serde_json::json!({
                        "runtime_healthy": healthy,
                        "live_panes": host_facts.live_terminal_count,
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

        WorkspaceCmd::Adopt { repo, branch, task } => {
            let repos = list_repositories(&mut link).await?;
            let target = resolve_repository(&repos, &repo)?;
            let task = task.unwrap_or_else(|| branch.clone());
            let r = link
                .call(with(
                    req_for("workspace.create", uuid_of(&target.id)),
                    request::Payload::WorkspaceCreate(farcooler_protocol::v1::WorkspaceCreate {
                        task_name: task,
                        branch,
                        base_revision: String::new(),
                        cli_preset: String::new(),
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
            link.call(req_for("workspace.hide", uuid_of(&ws.id))).await?;
            println!("hidden {}  (git data untouched)", short_bytes(&ws.id));
        }

        WorkspaceCmd::Unhide { workspace } => {
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            link.call(req_for("workspace.unhide", uuid_of(&ws.id))).await?;
            println!("unhidden {}", short_bytes(&ws.id));
        }

        WorkspaceCmd::RemoveWorktree { workspace, confirm } => {
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            // The daemon checks this too, and its check is the one that counts:
            // a client that skips the prompt must still be refused.
            link.call(with(
                req_for("workspace.remove_worktree", uuid_of(&ws.id)),
                request::Payload::TypedConfirmation(farcooler_protocol::v1::TypedConfirmation {
                    typed_confirmation: confirm.unwrap_or_default(),
                }),
            ))
            .await?;
            println!("removed worktree for {} (branch kept)", short_bytes(&ws.id));
        }
    }
    Ok(())
}

async fn attach(host: Option<&str>, workspace: &str) -> Fallible {
    let mut link = connect_to(host).await?;
    let all = list_workspaces(&mut link).await?;
    let ws = resolve(&all, workspace, |w| &w.id, "workspace")?;

    println!("Attaching to the live tmux session for {}.", ws.task_name);
    println!();
    println!("This is a recovery interface. It takes no writer lease and is");
    println!("outside lease enforcement, exactly like raw tmux attach.");
    println!();
    match host {
        // The tmux socket is on the host, so the command has to run there.
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
async fn events(host: Option<&str>) -> Fallible {
    use std::io::Write;

    let mut link = connect_to(host).await?;
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

async fn layout(host: Option<&str>, cmd: LayoutCmd, json: bool) -> Fallible {
    use farcooler_daemon::layout::parse_preset;
    use farcooler_protocol::v1::LayoutUpdate;

    let mut link = connect_to(host).await?;
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

async fn terminal(host: Option<&str>, cmd: TerminalCmd, json: bool) -> Fallible {
    match cmd {
        // Record changes. These write durable intent, so they go to the daemon.
        TerminalCmd::Create { workspace, preset, title, tile } => {
            let mut link = connect_to(host).await?;
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
            let (mut link, id) = terminal_by_record(host, &terminal).await?;
            link.call(req_for("terminal.stop", id)).await?;
            println!("stopped {}", short(id));
        }

        TerminalCmd::DismissLost { terminal } => {
            let (mut link, id) = terminal_by_record(host, &terminal).await?;
            link.call(req_for("terminal.dismiss_lost", id)).await?;
            println!("dismissed {}", short(id));
        }

        TerminalCmd::Remove { terminal } => {
            let (mut link, id) = terminal_by_record(host, &terminal).await?;
            link.call(req_for("terminal.remove", id)).await?;
            println!("removed {}", short(id));
        }

        TerminalCmd::Seen { terminal } => {
            let (mut link, id) = terminal_by_record(host, &terminal).await?;
            link.call(req_for("terminal.seen", id)).await?;
            println!("marked {} seen", short(id));
        }

        // Agent channel. Every payload here names its own `terminal_id`
        // rather than the envelope's `target_resource_id` — see the matching
        // comment in `crates/daemon/src/rpc.rs` — so `req(method)` plus
        // `with(...)` is right, not `req_for`.
        TerminalCmd::SetPaneMode { terminal, mode, force } => {
            let (mut link, id) = terminal_by_record(host, &terminal).await?;
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
            let (mut link, id) = terminal_by_record(host, &terminal).await?;
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

        TerminalCmd::AgentPrompt { terminal, text, images } => {
            use farcooler_protocol::v1::agent_prompt_block::Content;
            let (mut link, id) = terminal_by_record(host, &terminal).await?;

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
            let (mut link, id) = terminal_by_record(host, &terminal).await?;
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
            let (mut link, id) = terminal_by_record(host, &terminal).await?;
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
            let (mut link, id) = terminal_by_record(host, &terminal).await?;
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
            let (mut link, id) = terminal_by_record(host, &terminal).await?;
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
            let (mut link, id) = terminal_by_record(host, &terminal).await?;
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
            let (mut link, id) = terminal_by_record(host, &terminal).await?;
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
            let (mut link, id) = terminal_by_record(host, &terminal).await?;
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
            let (mut link, id) = terminal_by_record(host, &terminal).await?;
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
            let (mut link, id) = terminal_by_record(host, &terminal).await?;
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
        // On a remote host they run over their own ssh session, hitting the
        // host's own CLI. ssh is already a byte pipe and so are these, so the
        // honest implementation is to connect the two and get out of the way.
        TerminalCmd::Send { terminal, data } if host.is_some() => {
            return proxy(host, &["terminal".into(), "send".into(), terminal, data]).await;
        }
        TerminalCmd::SendHex { terminal, hex } if host.is_some() => {
            return proxy(host, &["terminal".into(), "send-hex".into(), terminal, hex]).await;
        }
        TerminalCmd::Stream { terminal } if host.is_some() => {
            return proxy(host, &["terminal".into(), "stream".into(), terminal]).await;
        }
        TerminalCmd::Input { terminal } if host.is_some() => {
            return proxy(host, &["terminal".into(), "input".into(), terminal]).await;
        }
        TerminalCmd::Screen { terminal } if host.is_some() => {
            let mut args = vec!["terminal".into(), "screen".into(), terminal];
            if json {
                args.push("--json".into());
            }
            return proxy(host, &args).await;
        }
        TerminalCmd::Resize { terminal, columns, rows } if host.is_some() => {
            return proxy(
                host,
                &["terminal".into(), "resize".into(), terminal, columns.to_string(), rows.to_string()],
            )
            .await;
        }
        TerminalCmd::Read { terminal, lines } if host.is_some() => {
            return proxy(
                host,
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
// unlike the tmux-backed reads further up — `connect_to(host)` alone reaches
// a remote host correctly and no `proxy()` is needed.
// ---------------------------------------------------------------------------

async fn worktree(host: Option<&str>, cmd: WorktreeCmd, json: bool) -> Fallible {
    match cmd {
        WorktreeCmd::FileSearch { workspace, query, limit } => {
            let mut link = connect_to(host).await?;
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

/// Run a command on the host's own CLI and adopt its exit status.
async fn proxy(host: Option<&str>, args: &[String]) -> Fallible {
    let Some(target) = host else { return Ok(()) };
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
    host: Option<&str>,
    prefix: &str,
) -> Result<(Link, Uuid), Box<dyn std::error::Error>> {
    let mut link = connect_to(host).await?;
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

async fn list_repositories(link: &mut Link) -> Result<Vec<Repository>, Box<dyn std::error::Error>> {
    let r = link.call(req("repository.list")).await?;
    match expect_value(r.value, "repositories")? {
        result::Value::RepositoryList(l) => Ok(l.items),
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

fn uuid_of(bytes: &[u8]) -> Uuid {
    Uuid::from_slice(bytes).unwrap_or(Uuid::nil())
}

/// The inverse of `uuid_of`: an id going INTO a payload rather than out of
/// one, for the agent-channel and worktree-search messages that carry their
/// own id fields instead of using the envelope's `target_resource_id`.
fn id_bytes(id: Uuid) -> bytes::Bytes {
    bytes::Bytes::copy_from_slice(id.as_bytes())
}

fn short_bytes(bytes: &[u8]) -> String {
    short(uuid_of(bytes))
}

fn truncate(s: &str, n: usize) -> String {
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
/// directly is what keeps the next field from going missing the same way.
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
/// called `api` on one host is a thing that happens.
fn resolve_repository<'a>(
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
}
