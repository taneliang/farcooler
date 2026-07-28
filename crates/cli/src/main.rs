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

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use daemon_link::{Link, connect, expect_value, req, req_for, with};
use overnight_core::inventory::RuntimeInventory;
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
        #[arg(long, default_value = "shell")]
        preset: String,
        #[arg(long)]
        title: Option<String>,
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

type Fallible = Result<(), Box<dyn std::error::Error>>;

async fn run() -> Fallible {
    let cli = Cli::parse();
    match cli.command {
        Command::Status => status(cli.json).await,
        Command::Root(c) => root(c, cli.json).await,
        Command::Repo(c) => repo(c, cli.json).await,
        Command::Workspace(c) => workspace(c, cli.json).await,
        Command::Terminal(c) => terminal(c, cli.json).await,
        Command::Attach { workspace } => attach(&workspace).await,
    }
}

// ---------------------------------------------------------------------------
// Durable state, through the daemon
// ---------------------------------------------------------------------------

async fn status(json: bool) -> Fallible {
    let mut link = connect().await?;
    let roots = list_roots(&mut link).await?;
    let repos = list_repositories(&mut link).await?;
    let workspaces = list_workspaces(&mut link).await?;
    let terminals = list_terminals(&mut link, None).await?;

    // tmux facts come from the runtime, not the daemon: they describe what is
    // live, and the runtime is where that is known.
    let runtime = Runtime::open().await?;
    let live = runtime.inventory.snapshot();

    if json {
        println!(
            "{}",
            serde_json::json!({
                "daemonVersion": link.server_hello().daemon_version,
                "runtimeHealthy": live.inventory_healthy,
                "livePanes": live.panes.len(),
                "roots": roots.len(),
                "repositories": repos.len(),
                "workspaces": workspaces.len(),
                "terminals": terminals.len(),
            })
        );
        return Ok(());
    }

    println!("daemon        {}", link.server_hello().daemon_version);
    println!("tmux socket   {}", runtime.tmux.socket());
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
    println!("workspaces    {}", workspaces.len());
    println!("live panes    {}", live.panes.len());
    println!();
    println!("recovery: {}", runtime.tmux.recovery_command());
    Ok(())
}

async fn root(cmd: RootCmd, json: bool) -> Fallible {
    let mut link = connect().await?;
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

async fn repo(cmd: RepoCmd, json: bool) -> Fallible {
    let mut link = connect().await?;
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

async fn workspace(cmd: WorkspaceCmd, json: bool) -> Fallible {
    let mut link = connect().await?;
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
            let runtime = Runtime::open().await?;
            let live = runtime.inventory.snapshot();

            if json {
                let items: Vec<_> = workspaces
                    .iter()
                    .map(|w| {
                        serde_json::json!({
                            "id": uuid_of(&w.id).to_string(),
                            "short": short_bytes(&w.id),
                            "task": w.task_name,
                            "branch": w.branch,
                            "worktree": w.worktree_path,
                            "state": workspace_label(w.state()),
                            "terminals": terminals.iter()
                                .filter(|t| t.workspace_id == w.id)
                                .map(|t| serde_json::json!({
                                    "id": uuid_of(&t.id).to_string(),
                                    "short": short_bytes(&t.id),
                                    "title": t.title,
                                    "preset": t.command_preset,
                                    "state": terminal_label(t.state()),
                                    "epoch": t.epoch,
                                }))
                                .collect::<Vec<_>>(),
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
                    println!(
                        "    {}  {:16}  {:8}  {}",
                        short_bytes(&t.id),
                        truncate(&t.title, 16),
                        terminal_label(t.state()),
                        t.command_preset
                    );
                }
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

async fn attach(workspace: &str) -> Fallible {
    let mut link = connect().await?;
    let all = list_workspaces(&mut link).await?;
    let ws = resolve(&all, workspace, |w| &w.id, "workspace")?;
    let runtime = Runtime::open().await?;

    println!("Attaching to the live tmux session for {}.", ws.task_name);
    println!();
    println!("This is a recovery interface. It takes no writer lease and is");
    println!("outside lease enforcement, exactly like raw tmux attach.");
    println!();
    println!("  {}", runtime.tmux.recovery_command());
    println!();
    println!("Run that to attach. Detach with your tmux prefix then d.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Terminals: records through the daemon, bytes through tmux
// ---------------------------------------------------------------------------

async fn terminal(cmd: TerminalCmd, json: bool) -> Fallible {
    match cmd {
        // Record changes. These write durable intent, so they go to the daemon.
        TerminalCmd::Create { workspace, preset, title } => {
            let mut link = connect().await?;
            let all = list_workspaces(&mut link).await?;
            let ws = resolve(&all, &workspace, |w| &w.id, "workspace")?;
            let title = title.unwrap_or_else(|| preset.clone());
            let r = link
                .call(with(
                    req_for("terminal.create", uuid_of(&ws.id)),
                    request::Payload::TerminalCreate(overnight_protocol::v1::TerminalCreate {
                        title,
                        command_preset: preset,
                    }),
                ))
                .await?;
            let result::Value::Terminal(t) = expect_value(r.value, "terminal")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            println!("created terminal {}  {}", short_bytes(&t.id), t.title);
        }

        TerminalCmd::Stop { terminal } => {
            let (mut link, id) = terminal_by_record(&terminal).await?;
            link.call(req_for("terminal.stop", id)).await?;
            println!("stopped {}", short(id));
        }

        TerminalCmd::DismissLost { terminal } => {
            let (mut link, id) = terminal_by_record(&terminal).await?;
            link.call(req_for("terminal.dismiss_lost", id)).await?;
            println!("dismissed {} (still truthfully lost, no exit claimed)", short(id));
        }

        TerminalCmd::Restart { terminal } => {
            let (mut link, id) = terminal_by_record(&terminal).await?;
            let r = link.call(req_for("terminal.restart", id)).await?;
            let result::Value::Terminal(t) = expect_value(r.value, "terminal")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            println!("restarted {} as epoch {}", short_bytes(&t.id), t.epoch);
        }

        // Live bytes. No daemon and no database — tmux is the authority, and
        // holding a request/response conversation open for the life of a
        // stream would be the wrong shape for both.
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

/// Resolve a terminal against the daemon's RECORDS, not the live panes.
///
/// Stopping, restarting or dismissing a terminal has to work on one that is
/// already dead, which is precisely when it has no pane to be found by.
async fn terminal_by_record(prefix: &str) -> Result<(Link, Uuid), Box<dyn std::error::Error>> {
    let mut link = connect().await?;
    let terminals = list_terminals(&mut link, None).await?;
    let t = resolve(&terminals, prefix, |t| &t.id, "terminal")?;
    let id = uuid_of(&t.id);
    Ok((link, id))
}

// ---------------------------------------------------------------------------
// Calls
// ---------------------------------------------------------------------------

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

/// Resolve a short id suffix, refusing an ambiguous match rather than guessing.
fn resolve<'a, T>(
    items: &'a [T],
    prefix: &str,
    id_of: impl Fn(&T) -> &[u8],
    kind: &str,
) -> Result<&'a T, String> {
    let matches: Vec<&T> = items
        .iter()
        .filter(|i| uuid_of(id_of(i)).simple().to_string().ends_with(prefix))
        .collect();

    match matches.len() {
        1 => Ok(matches[0]),
        0 => Err(format!("no {kind} matching {prefix:?}")),
        n => Err(format!("{prefix:?} matches {n} {kind}s, be more specific")),
    }
}
