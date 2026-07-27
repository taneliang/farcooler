//! `overnight` command line interface.
//!
//! Every state this prints is DERIVED from the live tmux inventory at the moment
//! you ask. Nothing here reads a stored "running" flag, because none exists.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use overnight_daemon::service::Service;
use overnight_protocol::v1::{TerminalState, WorkspaceState};
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
    /// Show how to attach to a workspace's live tmux session.
    Attach { workspace: String },
}

#[derive(Subcommand)]
enum RootCmd {
    /// Allowlist a directory Overnight may operate in.
    Add { path: PathBuf },
    List,
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
    /// Hide a workspace. Never changes git data.
    Archive { workspace: String },
}

#[derive(Subcommand)]
enum TerminalCmd {
    /// Launch a preset in a new tagged tmux window.
    Create {
        workspace: String,
        #[arg(long, default_value = "shell")]
        preset: String,
        #[arg(long)]
        title: Option<String>,
    },
    /// Send exact bytes to a terminal.
    Send { terminal: String, data: String },
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
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("OVERNIGHT_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .with_target(false)
        .init();

    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let svc = Service::open().await?;

    match cli.command {
        Command::Status => {
            let roots = svc.list_roots()?;
            let repos = svc.list_repositories()?;
            let fleet = svc.fleet().await?;
            let live = svc.inventory_snapshot();

            println!("host          {}", short(svc.host_id));
            println!("tmux socket   {}", svc.tmux.socket());
            println!(
                "tmux runtime  {}",
                if live.inventory_healthy {
                    "reachable"
                } else {
                    "UNAVAILABLE (all terminals derive lost)"
                }
            );
            println!("roots         {}", roots.len());
            println!("repositories  {}", repos.len());
            println!("workspaces    {}", fleet.len());
            println!("live panes    {}", live.panes.len());
            println!();
            println!("recovery: {}", svc.tmux.recovery_command());
        }

        Command::Root(RootCmd::Add { path }) => {
            let r = svc.add_root(&path).await?;
            println!("added root {}  {}", short(r.id), r.path);
        }
        Command::Root(RootCmd::List) => {
            for r in svc.list_roots()? {
                println!("{}  {}", short(r.id), r.path);
            }
        }

        Command::Repo(RepoCmd::Register { path }) => {
            let r = svc.register_repository(&path).await?;
            println!("registered {}  {}  ({})", short(r.id), r.display_name, r.remote_summary);
        }
        Command::Repo(RepoCmd::List) => {
            for r in svc.list_repositories()? {
                println!("{}  {:20}  {}", short(r.id), r.display_name, r.remote_summary);
            }
        }

        Command::Workspace(WorkspaceCmd::Create { repo, task, branch, base }) => {
            let repos = svc.list_repositories()?;
            let r = resolve(&repos, &repo, |x| x.id, "repository")?;
            let ws = svc.create_workspace(r.id, &task, &branch, &base).await?;
            println!("created workspace {}  {}", short(ws.id), ws.task_name);
            println!("  branch   {}", ws.branch);
            println!("  worktree {}", ws.worktree_path);
        }

        Command::Workspace(WorkspaceCmd::List) => {
            let fleet = svc.fleet().await?;

            if cli.json {
                let live = svc.inventory_snapshot();
                let items: Vec<_> = fleet
                    .iter()
                    .map(|v| {
                        serde_json::json!({
                            "id": v.workspace.id.to_string(),
                            "short": short(v.workspace.id),
                            "task": v.workspace.task_name,
                            "branch": v.workspace.branch,
                            "worktree": v.workspace.worktree_path,
                            "state": workspace_label(v.state),
                            "terminals": v.terminals.iter().map(|t| serde_json::json!({
                                "id": t.terminal.id.to_string(),
                                "short": short(t.terminal.id),
                                "title": t.terminal.title,
                                "preset": t.terminal.command_preset,
                                "state": terminal_label(t.state()),
                                "epoch": t.terminal.epoch,
                            })).collect::<Vec<_>>(),
                        })
                    })
                    .collect();

                println!(
                    "{}",
                    serde_json::json!({
                        "runtime_healthy": live.inventory_healthy,
                        "live_panes": live.panes.len(),
                        "workspaces": items,
                    })
                );
                return Ok(());
            }

            if fleet.is_empty() {
                println!("no workspaces yet");
            }
            for view in fleet {
                println!(
                    "{}  {:22}  {:8}  {}",
                    short(view.workspace.id),
                    truncate(&view.workspace.task_name, 22),
                    workspace_label(view.state),
                    view.workspace.branch
                );
                for t in &view.terminals {
                    println!(
                        "    {}  {:16}  {:8}  {}",
                        short(t.terminal.id),
                        truncate(&t.terminal.title, 16),
                        terminal_label(t.state()),
                        t.terminal.command_preset
                    );
                }
            }
        }

        Command::Workspace(WorkspaceCmd::Archive { workspace }) => {
            let all = svc.list_workspaces()?;
            let ws = resolve(&all, &workspace, |x| x.id, "workspace")?;
            let updated = svc.archive_workspace(ws.id).await?;
            println!("archived {}  (git data untouched)", short(updated.id));
        }

        Command::Terminal(TerminalCmd::Create { workspace, preset, title }) => {
            let all = svc.list_workspaces()?;
            let ws = resolve(&all, &workspace, |x| x.id, "workspace")?;
            let title = title.unwrap_or_else(|| preset.clone());
            let t = svc.create_terminal(ws.id, &title, &preset).await?;
            println!("created terminal {}  {}", short(t.id), t.title);
        }

        Command::Terminal(TerminalCmd::Send { terminal, data }) => {
            let id = resolve_terminal(&svc, &terminal).await?;
            svc.send_input(id, &data).await?;
            println!("sent {} bytes", data.len());
        }

        Command::Terminal(TerminalCmd::Read { terminal, lines }) => {
            let id = resolve_terminal(&svc, &terminal).await?;
            print!("{}", svc.capture(id, lines).await?);
        }

        Command::Terminal(TerminalCmd::Stop { terminal }) => {
            let id = resolve_terminal(&svc, &terminal).await?;
            svc.stop_terminal(id).await?;
            println!("stopped {}", short(id));
        }

        Command::Terminal(TerminalCmd::DismissLost { terminal }) => {
            let id = resolve_terminal(&svc, &terminal).await?;
            svc.dismiss_lost(id).await?;
            println!("dismissed {} (still truthfully lost, no exit claimed)", short(id));
        }

        Command::Terminal(TerminalCmd::Restart { terminal }) => {
            let id = resolve_terminal(&svc, &terminal).await?;
            let t = svc.restart_terminal(id).await?;
            println!("restarted {} as epoch {}", short(t.id), t.epoch);
        }

        Command::Attach { workspace } => {
            let all = svc.list_workspaces()?;
            let ws = resolve(&all, &workspace, |x| x.id, "workspace")?;
            println!("Attaching to the live tmux session for {}.", ws.task_name);
            println!();
            println!("This is a recovery interface. It takes no writer lease and is");
            println!("outside lease enforcement, exactly like raw tmux attach.");
            println!();
            println!("  {}", svc.tmux.recovery_command());
            println!();
            println!("Run that to attach. Detach with your tmux prefix then d.");
        }
    }

    Ok(())
}

/// UUIDv7 is time-ordered, so its LEADING hex is a timestamp that is identical
/// for anything created in the same millisecond. Prefix matching on the head is
/// useless. The trailing bytes are random, so short ids use those.
fn short(id: Uuid) -> String {
    let s = id.simple().to_string();
    s[s.len() - 8..].to_string()
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "~"
    }
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

/// Resolve a short id prefix, refusing an ambiguous match rather than guessing.
fn resolve<'a, T>(
    items: &'a [T],
    prefix: &str,
    id_of: impl Fn(&T) -> Uuid,
    kind: &str,
) -> Result<&'a T, String> {
    let matches: Vec<&T> =
        items.iter().filter(|i| id_of(i).simple().to_string().ends_with(prefix)).collect();

    match matches.len() {
        1 => Ok(matches[0]),
        0 => Err(format!("no {kind} matching {prefix:?}")),
        n => Err(format!("{prefix:?} matches {n} {kind}s, be more specific")),
    }
}

async fn resolve_terminal(svc: &Service, prefix: &str) -> Result<Uuid, String> {
    let fleet = svc.fleet().await.map_err(|e| e.to_string())?;
    let ids: Vec<Uuid> =
        fleet.iter().flat_map(|w| w.terminals.iter().map(|t| t.terminal.id)).collect();

    let matches: Vec<Uuid> =
        ids.into_iter().filter(|id| id.simple().to_string().ends_with(prefix)).collect();

    match matches.len() {
        1 => Ok(matches[0]),
        0 => Err(format!("no terminal matching {prefix:?}")),
        n => Err(format!("{prefix:?} matches {n} terminals, be more specific")),
    }
}
