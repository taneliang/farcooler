//! `overnight` command line interface.
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
//!   for several readers, so serialising those through the daemon would buy
//!   nothing and would put a long-lived stream inside a request/response
//!   conversation where it does not belong.
//!
//! Every state printed here is DERIVED at the moment you ask. Nothing reads a
//! stored "running" flag, because none exists.

mod daemon_link;
mod host_install;
mod remote;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use daemon_link::{Link, connect_to, expect_value, req, req_for, with};
use overnight_daemon::runtime::Runtime;
use overnight_protocol::v1::{
    Repository, RepositoryRoot, Terminal, TerminalState, Workspace, WorkspaceState, request, result,
};
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "overnight",
    version,
    about = "A terminal-first command center for parallel coding agents"
)]
struct Cli {
    /// Emit machine-readable JSON. The Mac app consumes this rather than
    /// scraping human output.
    #[arg(long, global = true)]
    json: bool,

    /// Operate on another machine, as `user@host` or an ssh config alias.
    ///
    /// Runs the same protocol over ssh. There is no Overnight network
    /// listener: a host reachable by ssh is reachable by Overnight, and one
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
    /// Install or inspect Overnight on a Linux host over ssh.
    #[command(subcommand, name = "host")]
    HostCmd(HostCmd),
}

/// Tiling, in tmux's vocabulary.
///
/// The names are tmux's on purpose. A great many people already know that `z`
/// zooms and that a layout is called `main-vertical`, and inventing a second
/// vocabulary for the same five arrangements would have cost them that for
/// nothing.
#[derive(Subcommand)]
enum LayoutCmd {
    /// Show a workspace's groups and which panes are in them.
    Show { workspace: String },
    /// Put terminals on screen together, replacing what was there.
    ///
    /// With no terminals named, tiles every live terminal in the workspace —
    /// the one-word version of the whole feature.
    Tile {
        workspace: String,
        terminals: Vec<String>,
        /// even-horizontal, even-vertical, main-vertical, main-horizontal, tiled.
        #[arg(long)]
        preset: Option<String>,
    },
    /// Add terminals to the group without disturbing the rest.
    Add { workspace: String, terminals: Vec<String> },
    /// Take terminals off screen. They keep running.
    Drop { workspace: String, terminals: Vec<String> },
    /// Set the arrangement, the main-pane share, or the group's name.
    Preset {
        workspace: String,
        preset: Option<String>,
        /// Fraction of the long axis for the main pane, 0.15 to 0.85.
        #[arg(long)]
        ratio: Option<f64>,
        #[arg(long)]
        name: Option<String>,
    },
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
        /// One-based, the way `prefix 1` reads.
        #[arg(long, value_name = "N")]
        pane: Option<usize>,
    },
    /// Fill the group with one pane. tmux's `prefix z`.
    ///
    /// With no terminal, toggles on whatever is focused. While zoomed, moving
    /// focus keeps the zoom and brings the new pane forward — which is the point
    /// of zooming when you have four agents rather than one.
    Zoom {
        workspace: String,
        terminal: Option<String>,
        #[arg(long)]
        off: bool,
    },
    /// Exchange two panes' positions.
    Swap { workspace: String, a: String, b: String },
    /// Move the focused pane one place along. tmux's `prefix {` and `}`.
    Shift {
        workspace: String,
        #[arg(long)]
        back: bool,
    },
    /// Several layouts per workspace, one on screen. tmux's windows.
    #[command(subcommand)]
    Group(LayoutGroupCmd),
}

#[derive(Subcommand)]
enum LayoutGroupCmd {
    /// A new, empty group, and show it.
    New { workspace: String, name: Option<String> },
    /// Show a group by name, by number, or the next one.
    Select {
        workspace: String,
        group: Option<String>,
        #[arg(long)]
        next: bool,
        #[arg(long)]
        prev: bool,
    },
    /// Stop showing a group. Its terminals keep running, backgrounded.
    Close { workspace: String },
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
}

#[derive(Subcommand)]
enum RootCmd {
    /// Allowlist a directory Overnight may operate in.
    Add { path: PathBuf },
    List,
    /// Stop allowing Overnight to operate under a directory.
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
    /// Hide a workspace. Never changes git data.
    Archive { workspace: String },
    /// Bring an archived workspace back.
    Restore { workspace: String },
    /// Remove the worktree. Keeps the branch and everything committed.
    RemoveWorktree {
        workspace: String,
        /// The workspace's exact task name. Required, because this deletes files.
        #[arg(long)]
        confirm: String,
    },
}

#[derive(Subcommand)]
enum TerminalCmd {
    /// Launch a preset in a new tagged tmux window.
    Create {
        workspace: String,
        /// What to launch. Defaults to your shell, which is almost always
        /// right: you open a terminal and type `claude` into it, and Overnight
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
    /// Print the rendered visible screen with colour escapes intact.
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
    /// Acknowledge a loss without claiming an exit.
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
            tracing_subscriber::EnvFilter::try_from_env("OVERNIGHT_LOG")
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

async fn run() -> Fallible {
    let cli = Cli::parse();
    let host = cli.host.as_deref();
    match cli.command {
        Command::Status => status(host, cli.json).await,
        Command::Root(c) => root(host, c, cli.json).await,
        Command::Repo(c) => repo(host, c, cli.json).await,
        Command::Workspace(c) => workspace(host, c, cli.json).await,
        Command::Terminal(c) => terminal(host, c, cli.json).await,
        Command::Layout(c) => layout(host, c, cli.json).await,
        Command::Attach { workspace } => attach(host, &workspace).await,
        Command::Events => events(host).await,
        Command::HostCmd(HostCmd::Install { target, from }) => {
            host_install::install(&target, from.as_deref()).await
        }
        Command::HostCmd(HostCmd::Status { target }) => host_install::status(&target).await,
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
    let healthy = host_facts.self_health != overnight_protocol::v1::SelfHealth::Degraded as i32;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "daemonVersion": host_facts.daemon_version,
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
                        overnight_protocol::v1::RepositoryRootAdd {
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
                request::Payload::TypedConfirmation(overnight_protocol::v1::TypedConfirmation {
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
                        overnight_protocol::v1::RepositoryRegister {
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
            let target = resolve(&repos, &repo, |r| &r.id, "repository")?;
            let r = link
                .call(with(
                    req_for("workspace.create", uuid_of(&target.id)),
                    request::Payload::WorkspaceCreate(overnight_protocol::v1::WorkspaceCreate {
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
                host_facts.self_health != overnight_protocol::v1::SelfHealth::Degraded as i32;

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
                            "terminals": terminals.iter()
                                .filter(|t| t.workspace_id == w.id)
                                .map(|t| serde_json::json!({
                                    "id": uuid_of(&t.id).to_string(),
                                    "short": short_bytes(&t.id),
                                    "title": t.title,
                                    "preset": label(t),
                                    "state": terminal_label(t.state()),
                                    "activity": activity_label(t.activity),
                                    "activitySince": activity_since(&t),
                                    "epoch": t.epoch,
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
                        label(&t)
                    );
                }
            }
        }

        WorkspaceCmd::Branches { repo } => {
            let repos = list_repositories(&mut link).await?;
            let target = resolve(&repos, &repo, |r| &r.id, "repository")?;
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
            let target = resolve(&repos, &repo, |r| &r.id, "repository")?;
            let task = task.unwrap_or_else(|| branch.clone());
            let r = link
                .call(with(
                    req_for("workspace.create", uuid_of(&target.id)),
                    request::Payload::WorkspaceCreate(overnight_protocol::v1::WorkspaceCreate {
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

        WorkspaceCmd::Archive { workspace } => {
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            link.call(req_for("workspace.archive", uuid_of(&ws.id))).await?;
            println!("archived {}  (git data untouched)", short_bytes(&ws.id));
        }

        WorkspaceCmd::Restore { workspace } => {
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            link.call(req_for("workspace.restore", uuid_of(&ws.id))).await?;
            println!("restored {}", short_bytes(&ws.id));
        }

        WorkspaceCmd::RemoveWorktree { workspace, confirm } => {
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            // The daemon checks this too, and its check is the one that counts:
            // a client that skips the prompt must still be refused.
            link.call(with(
                req_for("workspace.remove_worktree", uuid_of(&ws.id)),
                request::Payload::TypedConfirmation(overnight_protocol::v1::TypedConfirmation {
                    typed_confirmation: confirm,
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
            println!("  ssh -t {target} overnight attach {workspace}");
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
            overnight_protocol::v1::event::Payload::TerminalChanged(t) => serde_json::json!({
                "kind": "terminal",
                "id": uuid_of(&t.id).to_string(),
                "short": short_bytes(&t.id),
                "workspace": uuid_of(&t.workspace_id).to_string(),
                "title": t.title,
                "preset": label(&t),
                "state": terminal_label(t.state()),
                "activity": activity_label(t.activity),
                                    "activitySince": activity_since(&t),
            }),
            overnight_protocol::v1::event::Payload::WorkspaceChanged(w) => serde_json::json!({
                "kind": "workspace",
                "id": uuid_of(&w.id).to_string(),
                "short": short_bytes(&w.id),
                "task": w.task_name,
                "state": workspace_label(w.state()),
            }),
            overnight_protocol::v1::event::Payload::LayoutChanged(l) => serde_json::json!({
                "kind": "layout",
                "workspace": uuid_of(&l.workspace_id).to_string(),
                "groups": l.items.iter().map(|g| serde_json::json!({
                    "id": uuid_of(&g.id).to_string(),
                    "short": short_bytes(&g.id),
                    "name": g.name,
                    "preset": overnight_daemon::layout::preset_name(g.preset()),
                    "ratio": g.ratio,
                    "active": g.active,
                    "zoomed": g.zoomed.as_ref().map(|z| uuid_of(z).to_string()),
                    "focused": g.focused.as_ref().map(|f| uuid_of(f).to_string()),
                    "members": g.members.iter()
                        .map(|m| uuid_of(m).to_string()).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
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
    use overnight_daemon::layout::{parse_preset, preset_name};
    use overnight_protocol::v1::LayoutUpdate;

    let mut link = connect_to(host).await?;
    let workspaces = list_workspaces(&mut link).await?;

    let name_of = |cmd: &LayoutCmd| -> &'static str {
        match cmd {
            LayoutCmd::Show { .. } => "layout.list",
            LayoutCmd::Tile { .. } => "layout.tile",
            LayoutCmd::Add { .. } => "layout.add",
            LayoutCmd::Drop { .. } => "layout.drop",
            LayoutCmd::Preset { .. } => "layout.preset",
            LayoutCmd::Cycle { .. } => "layout.cycle",
            LayoutCmd::Focus { .. } => "layout.focus",
            LayoutCmd::Zoom { .. } => "layout.zoom",
            LayoutCmd::Swap { .. } => "layout.swap",
            LayoutCmd::Shift { .. } => "layout.shift",
            LayoutCmd::Group(LayoutGroupCmd::New { .. }) => "layout.group.new",
            LayoutCmd::Group(LayoutGroupCmd::Select { .. }) => "layout.group.select",
            LayoutCmd::Group(LayoutGroupCmd::Close { .. }) => "layout.group.close",
        }
    };
    let method = name_of(&cmd);

    let workspace_arg = match &cmd {
        LayoutCmd::Show { workspace }
        | LayoutCmd::Tile { workspace, .. }
        | LayoutCmd::Add { workspace, .. }
        | LayoutCmd::Drop { workspace, .. }
        | LayoutCmd::Preset { workspace, .. }
        | LayoutCmd::Cycle { workspace }
        | LayoutCmd::Focus { workspace, .. }
        | LayoutCmd::Zoom { workspace, .. }
        | LayoutCmd::Swap { workspace, .. }
        | LayoutCmd::Shift { workspace, .. }
        | LayoutCmd::Group(
            LayoutGroupCmd::New { workspace, .. }
            | LayoutGroupCmd::Select { workspace, .. }
            | LayoutGroupCmd::Close { workspace },
        ) => workspace.clone(),
    };
    let ws = resolve(&workspaces, &workspace_arg, |w| &w.id, "workspace")?;
    let workspace_id = uuid_of(&ws.id);

    // Terminals are named by short id, so they have to be resolved against the
    // workspace before anything can be said about them.
    let terminals = list_terminals(&mut link, Some(workspace_id)).await?;
    let pick = |given: &str| -> Result<bytes::Bytes, String> {
        resolve(&terminals, given, |t| &t.id, "terminal").map(|t| t.id.clone())
    };
    let pick_all = |given: &[String]| -> Result<Vec<bytes::Bytes>, String> {
        given.iter().map(|g| pick(g)).collect()
    };

    let mut update = LayoutUpdate::default();
    match &cmd {
        LayoutCmd::Show { .. } | LayoutCmd::Cycle { .. } => {}
        LayoutCmd::Tile { terminals, preset, .. } => {
            update.terminals = pick_all(terminals)?;
            if let Some(text) = preset {
                update.preset =
                    Some(parse_preset(text).ok_or_else(|| unknown_preset(text))? as i32);
            }
        }
        LayoutCmd::Add { terminals, .. } | LayoutCmd::Drop { terminals, .. } => {
            update.terminals = pick_all(terminals)?;
        }
        LayoutCmd::Preset { preset, ratio, name, .. } => {
            if let Some(text) = preset {
                update.preset =
                    Some(parse_preset(text).ok_or_else(|| unknown_preset(text))? as i32);
            }
            update.ratio = *ratio;
            update.name = name.clone().unwrap_or_default();
        }
        LayoutCmd::Focus { terminal, prev, pane, .. } => match (terminal, pane) {
            (Some(given), _) => update.focus = Some(pick(given)?),
            (None, Some(n)) => update.pane = Some(*n as u32),
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
        LayoutCmd::Shift { back, .. } => {
            update.step = Some(if *back { -1 } else { 1 });
        }
        LayoutCmd::Group(LayoutGroupCmd::New { name, .. }) => {
            update.name = name.clone().unwrap_or_default();
        }
        LayoutCmd::Group(LayoutGroupCmd::Select { group, prev, .. }) => match group {
            Some(given) => {
                // A group can be named by number, which is what people count on
                // screen, or by its short id, which is what a script has.
                let existing = fetch_layout(&mut link, workspace_id).await?;
                let found = match given.parse::<usize>() {
                    Ok(n) if n >= 1 && n <= existing.len() => existing[n - 1].id.clone(),
                    _ => resolve(&existing, given, |g| &g.id, "group")?.id.clone(),
                };
                update.group_id = Some(found);
            }
            None => update.step = Some(if *prev { -1 } else { 1 }),
        },
        LayoutCmd::Group(LayoutGroupCmd::Close { .. }) => {}
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

    if json {
        println!("{}", layout_json(&list, &terminals));
        return Ok(());
    }

    if list.items.is_empty() {
        println!("nothing tiled");
        return Ok(());
    }
    for group in &list.items {
        println!(
            "{} {}  {}  {} pane{}",
            if group.active { "*" } else { " " },
            group.name,
            preset_name(group.preset()),
            group.members.len(),
            if group.members.len() == 1 { "" } else { "s" },
        );
        for (index, member) in group.members.iter().enumerate() {
            let title = terminals
                .iter()
                .find(|t| t.id == *member)
                .map(label)
                .unwrap_or_else(|| "?".into());
            // Zoom and focus are shown as marks rather than columns: they are
            // one pane each, and a column of blanks reads as missing data.
            let marks = format!(
                "{}{}",
                if group.focused.as_ref() == Some(member) { ">" } else { " " },
                if group.zoomed.as_ref() == Some(member) { "z" } else { " " },
            );
            println!("   {marks} {}  {}  {}", index + 1, short_bytes(member), truncate(&title, 40));
        }
    }
    Ok(())
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
) -> Result<Vec<overnight_protocol::v1::PaneGroup>, Box<dyn std::error::Error>> {
    let r = link.call(req_for("layout.list", workspace)).await?;
    match expect_value(r.value, "layout")? {
        result::Value::PaneGroupList(l) => Ok(l.items),
        _ => Err("the daemon returned the wrong resource".into()),
    }
}

fn layout_json(
    list: &overnight_protocol::v1::PaneGroupList,
    terminals: &[Terminal],
) -> serde_json::Value {
    use overnight_daemon::layout::preset_name;
    serde_json::json!({
        "workspace": uuid_of(&list.workspace_id).to_string(),
        "groups": list.items.iter().map(|g| serde_json::json!({
            "id": uuid_of(&g.id).to_string(),
            "short": short_bytes(&g.id),
            "name": g.name,
            "preset": preset_name(g.preset()),
            "ratio": g.ratio,
            "active": g.active,
            "zoomed": g.zoomed.as_ref().map(|z| uuid_of(z).to_string()),
            "focused": g.focused.as_ref().map(|f| uuid_of(f).to_string()),
            "members": g.members.iter().map(|m| serde_json::json!({
                "id": uuid_of(m).to_string(),
                "short": short_bytes(m),
                "title": terminals.iter().find(|t| t.id == *m).map(label),
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
                    request::Payload::TerminalCreate(overnight_protocol::v1::TerminalCreate {
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
            println!("dismissed {} (still truthfully lost, no exit claimed)", short(id));
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
) -> Result<overnight_protocol::v1::Host, Box<dyn std::error::Error>> {
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
fn activity_label(a: i32) -> &'static str {
    use overnight_protocol::v1::AgentActivity;
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
fn label(t: &overnight_protocol::v1::Terminal) -> String {
    if t.current_command.is_empty() { t.command_preset.clone() } else { t.current_command.clone() }
}

/// When the activity last changed, as Unix milliseconds.
///
/// A client shows "working for 4m" from this rather than timing it locally,
/// which would restart at every reconnect and lie after a laptop sleeps.
fn activity_since(t: &overnight_protocol::v1::Terminal) -> Option<i64> {
    t.activity_changed_at.as_ref().map(|ts| ts.seconds * 1000 + (ts.nanos as i64) / 1_000_000)
}

fn workspace_label(s: WorkspaceState) -> &'static str {
    match s {
        WorkspaceState::Unspecified => "?",
        WorkspaceState::Creating => "creating",
        WorkspaceState::Ready => "ready",
        WorkspaceState::Active => "active",
        WorkspaceState::Error => "ERROR",
        WorkspaceState::Archived => "archived",
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

/// Normalise an id for matching.
///
/// Ids are compared in their dashless form, so a short id and a full hyphenated
/// UUID both work. Without this, pasting an id straight out of `--json` — which
/// is hyphenated — matched nothing, because the stored form is not.
fn normalise_id(text: &str) -> String {
    text.trim().to_lowercase().replace('-', "")
}

/// Resolve a short id suffix, refusing an ambiguous match rather than guessing.
fn resolve<'a, T>(
    items: &'a [T],
    prefix: &str,
    id_of: impl Fn(&T) -> &[u8],
    kind: &str,
) -> Result<&'a T, String> {
    let needle = normalise_id(prefix);
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
