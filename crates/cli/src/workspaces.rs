//! `farcooler workspace` — workstreams: a board of their own, a task prefix
//! of their own, and at most one orchestrator, inside one repository.
//!
//! A workspace is NOT a directory. The directories are worktrees (`farcooler
//! worktree`), and a workspace owns some of them: the ones it made, the ones
//! its agents were seen working in, and the ones assigned to it by hand
//! (`farcooler worktree assign`). Every repository has a Main, which can be
//! renamed but not deleted.
//!
//! **Naming one.** A workspace is named the way a person knows it: by its
//! name, then by its task prefix, then by the end of its id. A name is looked
//! for in one repository: `--repo`, or the only repository, or the one the
//! pane's own workspace (`FARCOOLER_WORKSPACE`) is in. Otherwise every
//! repository is searched, and a name two of them use is refused with both
//! listed rather than picked from, the way `resolve_repository` refuses.
//!
//! **Refusals** are said in this CLI's sentence for the word the runner sent
//! (`tasks::said_about`, in clap's style), or the runner's own
//! (`farcooler_core::error::sentence`) for a word newer than this build;
//! never the word itself. See `workspace_refused`.

use std::error::Error;

use clap::{Args, Subcommand, ValueEnum};
use farcooler_client::workspaces_json;
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_transport::ClientError;
use uuid::Uuid;

use crate::tasks::{DispatchLink, Refused, refusal, said_about};
use crate::{
    Fallible, Link, connect_to, expect_value, id_bytes, list_worktrees, req, req_for, resolve,
    resolve_repository, short_bytes, truncate, uuid_of, with,
};

/// The workspace a pane Far Cooler launched belongs to, as a uuid. See
/// `farcooler_core::pane_env::WORKSPACE`, which is where both ends read the
/// name from.
pub(crate) const WORKSPACE_ENV: &str = farcooler_core::pane_env::WORKSPACE;

/// What a runner without workspaces is told. Local, and never a `code:` line:
/// nothing was sent.
pub(crate) const NO_WORKSPACES: &str =
    "this runner's Far Cooler has no workspaces yet. update it and try again";

#[derive(Subcommand)]
pub enum WorkspaceCmd {
    /// Make a workstream: a board, a task prefix, and later an orchestrator.
    ///
    /// Its charter starts as a copy of Main's. It owns no worktree until one
    /// is made for it, claimed by its agents, or assigned to it.
    Create(CreateArgs),
    /// Every workspace, Main first, repository by repository.
    List {
        /// Only this repository's. Defaults to every repository.
        repo: Option<String>,
    },
    /// One workspace: its prefix, its orchestrator, and the worktrees it owns.
    Show {
        /// By name, task prefix, or the end of its id.
        workspace: String,
        /// Which repository the name is in. Only needed when two use it.
        #[arg(long)]
        repo: Option<String>,
    },
    /// Rename a workspace. Its tasks keep their keys.
    Rename {
        workspace: String,
        name: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Change the prefix new task keys get. Keys already given out stay as
    /// they are.
    ///
    /// A letter followed by up to seven letters or digits, unused by any
    /// other workspace on this runner.
    SetPrefix {
        workspace: String,
        prefix: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Delete a workspace that holds nothing. Main can't be deleted.
    ///
    /// Refused while any task, worktree or terminal still belongs to it: move
    /// them first (`task move`, `worktree assign`). Its charter stays on disk.
    Delete {
        workspace: String,
        #[arg(long)]
        repo: Option<String>,
    },
    /// Start the workspace's orchestrator: an agent that runs its board.
    ///
    /// Claude Code and Cursor start in the workspace's home, beside its
    /// charter, pointed back at the repository; Codex starts in the
    /// repository. A workspace has at most one: a second is refused unless
    /// `--replace`, which closes the one running first.
    StartOrchestrator {
        workspace: String,
        /// claude, codex or cursor, with an optional `:model`.
        #[arg(long, default_value = "claude")]
        harness: String,
        /// Close the orchestrator already running, and start this one.
        #[arg(long)]
        replace: bool,
        #[arg(long)]
        repo: Option<String>,
    },
}

/// `workspace create`'s words, including the ones it used to take.
///
/// Until the rename, `workspace create <repo> <name> --branch <b>` made a
/// worktree. That spelling is in scripts and in shell history, and read now it
/// would make a workspace named after a branch. So the old flags and the old
/// second positional are still accepted, hidden, and only so `checked` can
/// refuse them by name and point at `worktree create`.
#[derive(Args, Debug)]
pub struct CreateArgs {
    /// Which repository. Defaults to the one `FARCOOLER_WORKSPACE` is in,
    /// then to the only one there is.
    repo: Option<String>,
    /// The repository, as a flag.
    #[arg(long = "repo", conflicts_with = "repo")]
    repo_flag: Option<String>,
    /// What to call it. Unique only in spirit: two may share a name.
    #[arg(long)]
    name: Option<String>,
    /// What its task keys start with: a letter followed by up to seven
    /// letters or digits, unused by any other workspace on this runner.
    #[arg(long)]
    prefix: Option<String>,
    // The old spelling, hidden. See the struct's doc.
    #[arg(hide = true)]
    rest: Vec<String>,
    #[arg(long, hide = true)]
    branch: Option<String>,
    #[arg(long, hide = true)]
    base: Option<String>,
    #[arg(long, hide = true)]
    terminal: Option<String>,
    #[arg(long, hide = true)]
    no_terminal: bool,
    #[arg(long, hide = true)]
    fork_only: bool,
}

/// A `workspace create` that can be sent: the repository as typed, if it
/// was, and the two words the workspace needs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Creating<'a> {
    pub(crate) repo: Option<&'a str>,
    pub(crate) name: &'a str,
    pub(crate) prefix: &'a str,
}

impl CreateArgs {
    /// The create, or why it isn't one. Runs before anything is sent, so the
    /// old spelling can never reach the runner as a workspace.
    pub(crate) fn checked(&self) -> Result<Creating<'_>, String> {
        let repo = self.repo.as_deref().or(self.repo_flag.as_deref());
        let worktree_words = self.branch.is_some()
            || self.base.is_some()
            || self.terminal.is_some()
            || self.no_terminal
            || self.fork_only;
        if worktree_words {
            let name = self.rest.first().map_or("<name>", String::as_str);
            return Err(format!(
                "workspace create makes a workstream now. To make a worktree, run: farcooler \
                 worktree create {} {name} --branch {}",
                repo.unwrap_or("<repo>"),
                self.branch.as_deref().unwrap_or("<branch>"),
            ));
        }
        if !self.rest.is_empty() {
            return Err(format!(
                "name the workspace with --name: farcooler workspace create {}--name {} --prefix <prefix>",
                repo.map(|r| format!("{r} ")).unwrap_or_default(),
                self.rest.join(" "),
            ));
        }
        let name = self.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
        let Some(name) = name else {
            return Err("name the workspace with --name".into());
        };
        let prefix = self.prefix.as_deref().map(str::trim).filter(|p| !p.is_empty());
        let Some(prefix) = prefix else {
            return Err("give the workspace a task prefix with --prefix, like --prefix bil".into());
        };
        Ok(Creating { repo, name, prefix })
    }
}

impl WorkspaceCmd {
    /// What can be refused without asking the runner. `run_parse_only`
    /// calls this for the whole command line, before anything connects.
    pub(crate) fn check(&self) -> Result<(), String> {
        match self {
            WorkspaceCmd::Create(args) => args.checked().map(drop),
            _ => Ok(()),
        }
    }
}

/// A terminal's role, as `terminal set-role` takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Role {
    /// Runs its workspace's board. At most one per workspace.
    Orchestrator,
    /// Works a task.
    Agent,
    /// A person's shell.
    Shell,
}

impl Role {
    pub(crate) fn wire(self) -> pb::TerminalRole {
        match self {
            Role::Orchestrator => pb::TerminalRole::Orchestrator,
            Role::Agent => pb::TerminalRole::Agent,
            Role::Shell => pb::TerminalRole::Shell,
        }
    }
}

pub async fn workspace(runner: Option<&str>, cmd: WorkspaceCmd, json: bool) -> Fallible {
    workspace_via(cmd, json, connect_to(runner)).await
}

/// `workspace`, on the link `connecting` makes. `cmd.check()` refuses what
/// the command line alone can before `connecting` is ever awaited, so the old
/// spelling of `workspace create` never reaches a runner.
async fn workspace_via(
    cmd: WorkspaceCmd,
    json: bool,
    connecting: impl std::future::Future<Output = Result<Link, Box<dyn Error>>>,
) -> Fallible {
    cmd.check()?;
    let mut link = connecting.await?;
    require_workstreams(&link)?;
    let env = pane_workspace(std::env::var(WORKSPACE_ENV).ok());

    match cmd {
        WorkspaceCmd::Create(args) => {
            let asked = args.checked()?;
            let repositories = crate::list_repositories(&mut link).await?;
            let all = workspaces_on(&mut link, None).await?;
            let repository = repository_to_create_in(&repositories, &all, asked.repo, env)?;
            let r = link
                .call(with(
                    req_for("workspace.create", repository),
                    request::Payload::WorkspaceCreate(pb::WorkspaceCreate {
                        name: asked.name.to_string(),
                        task_prefix: asked.prefix.to_string(),
                    }),
                ))
                .await
                .map_err(|e| workspace_refused(e, "that repository isn't on this runner", "that workspace could not be made"))?;
            let result::Value::Workspace(made) = expect_value(r.value)? else {
                return Err(crate::daemon_link::UNREADABLE.into());
            };
            if json {
                println!("{}", workspaces_json::workspace_json(&made));
                return Ok(());
            }
            println!("created workspace {}  {}  (tasks {}-1, {}-2, …)", short_bytes(&made.id), made.name, made.task_prefix, made.task_prefix);
        }

        WorkspaceCmd::List { repo } => {
            let repositories = crate::list_repositories(&mut link).await?;
            let only = match repo.as_deref() {
                Some(name) => Some(uuid_of(&resolve_repository(&repositories, name)?.id)),
                None => None,
            };
            let all = workspaces_on(&mut link, only).await?;
            if json {
                println!("{}", list_json(&all));
                return Ok(());
            }
            print!("{}", render_list(&all, &repositories));
        }

        WorkspaceCmd::Show { workspace, repo } => {
            let (ws, repositories) = named(&mut link, repo.as_deref(), &workspace, env).await?;
            if json {
                println!("{}", workspaces_json::workspace_json(&ws));
                return Ok(());
            }
            let worktrees = list_worktrees(&mut link).await?;
            print!("{}", render_show(&ws, &repositories, &worktrees));
        }

        WorkspaceCmd::Rename { workspace, name, repo } => {
            let (ws, _) = named(&mut link, repo.as_deref(), &workspace, env).await?;
            let r = link
                .call(with(
                    req_for("workspace.rename", uuid_of(&ws.id)),
                    request::Payload::WorkspaceRename(pb::WorkspaceRename {
                        name: name.trim().to_string(),
                        expected_version: Some(ws.resource_version),
                    }),
                ))
                .await
                .map_err(|e| workspace_refused(e, GONE, "that workspace could not be renamed"))?;
            done(r.value, json, |w| format!("renamed {} to {}", ws.name, w.name))?;
        }

        WorkspaceCmd::SetPrefix { workspace, prefix, repo } => {
            let (ws, _) = named(&mut link, repo.as_deref(), &workspace, env).await?;
            let r = link
                .call(with(
                    req_for("workspace.set_prefix", uuid_of(&ws.id)),
                    request::Payload::WorkspaceSetPrefix(pb::WorkspaceSetPrefix {
                        task_prefix: prefix.trim().to_string(),
                        expected_version: Some(ws.resource_version),
                    }),
                ))
                .await
                .map_err(|e| workspace_refused(e, GONE, "that prefix could not be set"))?;
            done(r.value, json, |w| {
                format!("new tasks in {} are {}-N now. keys already given out stay as they are", w.name, w.task_prefix)
            })?;
        }

        WorkspaceCmd::Delete { workspace, repo } => {
            let (ws, _) = named(&mut link, repo.as_deref(), &workspace, env).await?;
            link.call(req_for("workspace.delete", uuid_of(&ws.id)))
                .await
                .map_err(|e| workspace_refused(e, GONE, "that workspace could not be deleted"))?;
            if json {
                println!("{}", serde_json::json!({ "deleted": uuid_of(&ws.id).to_string() }));
            } else {
                println!("deleted workspace {}  {}  (its charter is still on disk)", short_bytes(&ws.id), ws.name);
            }
        }

        WorkspaceCmd::StartOrchestrator { workspace, harness, replace, repo } => {
            if !farcooler_core::pane_env::takes_a_task(harness.trim()) {
                return Err(format!("--harness {harness:?} can't orchestrate. use claude, codex or cursor").into());
            }
            let (ws, _) = named(&mut link, repo.as_deref(), &workspace, env).await?;
            let r = link
                .call(with(
                    req_for("workspace.start_orchestrator", uuid_of(&ws.id)),
                    request::Payload::WorkspaceStartOrchestrator(pb::WorkspaceStartOrchestrator {
                        harness: harness.trim().to_string(),
                        replace,
                    }),
                ))
                .await
                .map_err(|e| {
                    workspace_refused(e, "that workspace, or its repository's main checkout, isn't on this runner", "the orchestrator could not be started")
                })?;
            let result::Value::Terminal(t) = expect_value(r.value)? else {
                return Err(crate::daemon_link::UNREADABLE.into());
            };
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "id": uuid_of(&t.id).to_string(),
                        "short": short_bytes(&t.id),
                        "workspace": uuid_of(&ws.id).to_string(),
                    })
                );
            } else {
                println!("started the orchestrator for {}: terminal {}  {}", ws.name, short_bytes(&t.id), t.command_preset);
            }
        }
    }
    Ok(())
}

/// What a workspace command that changed one prints: the workspace under
/// `--json`, a line otherwise.
fn done(value: Option<result::Value>, json: bool, said: impl FnOnce(&pb::Workspace) -> String) -> Fallible {
    let result::Value::Workspace(w) = expect_value(value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    if json {
        println!("{}", workspaces_json::workspace_json(&w));
    } else {
        println!("{}", said(&w));
    }
    Ok(())
}

const GONE: &str = "that workspace isn't on this runner any more";

/// `farcooler worktree assign`: give a worktree to a workspace, whoever
/// claimed it before. The workspace is looked for in the worktree's own
/// repository.
pub(crate) async fn assign_worktree(link: &mut Link, worktree: &str, to: &str, json: bool) -> Fallible {
    require_workstreams(link)?;
    let worktrees = list_worktrees(link).await?;
    let wt = crate::find_worktree(&worktrees, worktree)?;
    let repository = uuid_of(&wt.repository_id);
    let repositories = crate::list_repositories(link).await?;
    let mine = workspaces_on(link, Some(repository)).await?;
    let ws = resolve_workspace(&mine, &repositories, to)?;
    let r = link
        .call(assign_request(uuid_of(&wt.id), uuid_of(&ws.id)))
        .await
        .map_err(|e| workspace_refused(e, "that worktree or workspace isn't on this runner any more", "that worktree could not be assigned"))?;
    let result::Value::Worktree(w) = expect_value(r.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    if json {
        println!(
            "{}",
            serde_json::json!({
                "id": uuid_of(&w.id).to_string(),
                "short": short_bytes(&w.id),
                "workspace": workspaces_json::workspace_of(w.workspace_id.as_deref()),
                "claim_source": w.claim_source,
            })
        );
    } else {
        println!("{} belongs to {} now", w.task_name, ws.name);
    }
    Ok(())
}

/// `worktree.assign`, naming the capability without which an older runner
/// would not know the method.
pub(crate) fn assign_request(worktree: Uuid, workspace: Uuid) -> pb::Request {
    needs_workstreams(with(
        req_for("worktree.assign", worktree),
        request::Payload::WorktreeAssign(pb::WorktreeAssign { workspace_id: id_bytes(workspace) }),
    ))
}

/// `farcooler terminal set-role`.
pub(crate) async fn set_role(runner: Option<&str>, terminal: &str, role: Role, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    require_workstreams(&link)?;
    let terminals = crate::list_terminals(&mut link, None).await?;
    let t = resolve(&terminals, terminal, |t| &t.id, "terminal")?;
    let id = uuid_of(&t.id);
    let r = link
        .call(set_role_request(id, role))
        .await
        .map_err(|e| workspace_refused(e, "that terminal isn't on this runner any more", "that role could not be set"))?;
    let result::Value::Terminal(t) = expect_value(r.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    if json {
        println!(
            "{}",
            serde_json::json!({
                "id": uuid_of(&t.id).to_string(),
                "short": short_bytes(&t.id),
                "role": workspaces_json::role_word(t.role),
                "workspace": workspaces_json::workspace_of(t.workspace_id.as_deref()),
            })
        );
    } else {
        let said = match workspaces_json::role_word(t.role) {
            Some("orchestrator") => "its workspace's orchestrator",
            Some("agent") => "an agent",
            Some("shell") => "a shell",
            _ => "a terminal of a role this Far Cooler doesn't know",
        };
        println!("terminal {} is {said} now", short_bytes(&t.id));
    }
    Ok(())
}

pub(crate) fn set_role_request(terminal: Uuid, role: Role) -> pb::Request {
    needs_workstreams(with(
        req_for("terminal.set_role", terminal),
        request::Payload::TerminalSetRole(pb::TerminalSetRole { role: role.wire() as i32 }),
    ))
}

/// `r`, refused by a runner too old to have workspaces rather than answered
/// with a field or method it doesn't know.
pub(crate) fn needs_workstreams(mut r: pb::Request) -> pb::Request {
    r.required_capabilities.push(capability::WORKSTREAMS.to_string());
    r
}

/// Whether the runner said it has workspaces, in its handshake.
pub(crate) fn has_workstreams(capabilities: &[String]) -> bool {
    capabilities.iter().any(|c| c == capability::WORKSTREAMS)
}

fn require_workstreams(link: &Link) -> Result<(), String> {
    if has_workstreams(link.daemon_capabilities()) { Ok(()) } else { Err(NO_WORKSPACES.to_string()) }
}

/// `FARCOOLER_WORKSPACE`, if it names a workspace at all. Empty, or not a
/// uuid, is no workspace: a shell that exported it empty is not in one.
pub(crate) fn pane_workspace(raw: Option<String>) -> Option<Uuid> {
    raw.as_deref().map(str::trim).filter(|s| !s.is_empty()).and_then(|s| Uuid::parse_str(s).ok())
}

/// Every workspace on the runner, or one repository's, Main first.
pub(crate) async fn workspaces_on<L: DispatchLink>(
    link: &mut L,
    repository: Option<Uuid>,
) -> Result<Vec<pb::Workspace>, Box<dyn Error>> {
    let envelope = match repository {
        Some(id) => req_for("workspace.list", id),
        None => req("workspace.list"),
    };
    let r = link
        .call(needs_workstreams(envelope))
        .await
        .map_err(|e| workspace_refused(e, "that repository isn't on this runner", "the workspaces could not be read"))?;
    let result::Value::WorkspaceList(l) = expect_value(r.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    Ok(l.items)
}

/// One workspace by the name a person typed, and the repositories, for
/// saying which repository it is in.
async fn named(
    link: &mut Link,
    repo: Option<&str>,
    given: &str,
    env: Option<Uuid>,
) -> Result<(pb::Workspace, Vec<pb::Repository>), Box<dyn Error>> {
    let repositories = crate::list_repositories(link).await?;
    let all = workspaces_on(link, None).await?;
    let scoped = scope(&all, &repositories, repo, env)?;
    let ws = resolve_workspace(&scoped, &repositories, given)?.clone();
    Ok((ws, repositories))
}

/// The workspaces a bare name is looked for among: `--repo`'s, or the only
/// repository's, or those of the repository the pane's own workspace is in.
/// Failing all three, every one — and `resolve_workspace` refuses a name two
/// repositories share rather than picking.
pub(crate) fn scope(
    all: &[pb::Workspace],
    repositories: &[pb::Repository],
    repo: Option<&str>,
    env: Option<Uuid>,
) -> Result<Vec<pb::Workspace>, String> {
    let in_repository = |id: &[u8]| all.iter().filter(|w| w.repository_id.as_ref() == id).cloned().collect();
    if let Some(name) = repo {
        return Ok(in_repository(&resolve_repository(repositories, name)?.id));
    }
    if let [only] = repositories {
        return Ok(in_repository(&only.id));
    }
    if let Some(own) = env.and_then(|id| all.iter().find(|w| uuid_of(&w.id) == id)) {
        return Ok(in_repository(&own.repository_id));
    }
    Ok(all.to_vec())
}

/// A workspace by name, ignoring case, then by task prefix, then by the end
/// of its id. A name two workspaces share is refused with both named, and
/// the repository each is in.
pub(crate) fn resolve_workspace<'a>(
    workspaces: &'a [pb::Workspace],
    repositories: &[pb::Repository],
    given: &str,
) -> Result<&'a pb::Workspace, String> {
    let needle = given.trim().to_lowercase();
    let by_name: Vec<&pb::Workspace> = workspaces.iter().filter(|w| w.name.to_lowercase() == needle).collect();
    match by_name.as_slice() {
        [one] => return Ok(one),
        [] => {}
        several => {
            let listed: Vec<String> = several
                .iter()
                .map(|w| format!("{} ({}, in {})", w.name, w.task_prefix, repository_name(repositories, &w.repository_id)))
                .collect();
            return Err(format!(
                "{} workspaces are called \"{given}\": {}. name one by its prefix, or say which repository with --repo",
                several.len(),
                listed.join(", ")
            ));
        }
    }
    if let Some(w) = workspaces.iter().find(|w| w.task_prefix.to_lowercase() == needle) {
        return Ok(w);
    }
    resolve(workspaces, given, |w| &w.id, "workspace")
}

fn repository_name(repositories: &[pb::Repository], id: &[u8]) -> String {
    repositories.iter().find(|r| r.id.as_ref() == id).map_or_else(|| short_bytes(id), |r| r.display_name.clone())
}

/// The repository a new workspace goes in: the one named, then the one the
/// pane's own workspace is in, then the only one there is.
pub(crate) fn repository_to_create_in(
    repositories: &[pb::Repository],
    all: &[pb::Workspace],
    repo: Option<&str>,
    env: Option<Uuid>,
) -> Result<Uuid, String> {
    if let Some(name) = repo {
        return Ok(uuid_of(&resolve_repository(repositories, name)?.id));
    }
    if let Some(own) = env.and_then(|id| all.iter().find(|w| uuid_of(&w.id) == id)) {
        return Ok(uuid_of(&own.repository_id));
    }
    match repositories {
        [only] => Ok(uuid_of(&only.id)),
        [] => Err("no repository is registered here. add one with `farcooler repo register`".into()),
        several => Err(format!(
            "this runner has {} repositories. name one with --repo: {}",
            several.len(),
            several.iter().map(|r| r.display_name.as_str()).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// `workspace list --json`: the objects `workspaces_json` builds, which are
/// the same ones `worktree list --json` carries in its envelope.
pub(crate) fn list_json(workspaces: &[pb::Workspace]) -> serde_json::Value {
    serde_json::json!({ "workspaces": workspaces.iter().map(workspaces_json::workspace_json).collect::<Vec<_>>() })
}

fn render_list(workspaces: &[pb::Workspace], repositories: &[pb::Repository]) -> String {
    if workspaces.is_empty() {
        return "no workspaces yet\n".into();
    }
    let mut out = String::new();
    for w in workspaces {
        let orchestrator = workspaces_json::workspace_json(w)["orchestrator"]
            .as_str()
            .map(|id| format!("  orchestrator {}", &id.replace('-', "")[24..]))
            .unwrap_or_default();
        let line = format!(
            "{}  {:20}  {:8}  {:16}{}{}",
            short_bytes(&w.id),
            truncate(&w.name, 20),
            w.task_prefix,
            truncate(&repository_name(repositories, &w.repository_id), 16),
            if w.is_main { "  main" } else { "" },
            orchestrator,
        );
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn render_show(ws: &pb::Workspace, repositories: &[pb::Repository], worktrees: &[pb::Worktree]) -> String {
    let json = workspaces_json::workspace_json(ws);
    let mut out = format!("{}  {}{}\n", short_bytes(&ws.id), ws.name, if ws.is_main { "  (main)" } else { "" });
    out.push_str(&format!("  repository    {}\n", repository_name(repositories, &ws.repository_id)));
    out.push_str(&format!("  task prefix   {}\n", ws.task_prefix));
    out.push_str(&format!("  orchestrator  {}\n", json["orchestrator"].as_str().unwrap_or("none running")));
    if let Some(charter) = json["charter"].as_str() {
        out.push_str(&format!("  charter       {charter}\n"));
    }
    let owned: Vec<&pb::Worktree> =
        worktrees.iter().filter(|w| w.workspace_id.as_deref() == Some(ws.id.as_ref())).collect();
    if owned.is_empty() {
        out.push_str("  worktrees     none yet\n");
    }
    for (n, w) in owned.iter().enumerate() {
        let label = if n == 0 { "worktrees" } else { "" };
        out.push_str(&format!("  {label:13} {}  {}\n", short_bytes(&w.id), w.task_name));
    }
    out
}

/// A refusal of a workspace command, in words.
///
/// An `invalid-argument` is the runner's own sentence for the word it sent
/// (`farcooler_core::error::sentence`) — "That prefix is already used by
/// another workspace." — and never the word. `not-found` is `missing`, which
/// each command words for what it looked up. Anything else is the board's
/// `refusal`, with `fallback` for a word this build has no sentence for. The
/// runner's code rides along either way, for `--json`'s `code:` line.
pub(crate) fn workspace_refused(e: ClientError, missing: &str, fallback: &str) -> Refused {
    let (code, what) = match &e {
        ClientError::Daemon { code, what, .. } => (*code, what.as_str()),
        _ => return refusal(e, fallback),
    };
    let said = match farcooler_core::error::word_for(code) {
        "invalid-argument" if what == "command_preset" => "an orchestrator runs claude, codex or cursor".to_string(),
        "invalid-argument" => said_about(what)
            .or_else(|| farcooler_core::error::sentence(what))
            .unwrap_or(fallback)
            .to_string(),
        "not-found" => missing.to_string(),
        "resource-conflict" => "that workspace changed while you were reading it. run the command again".to_string(),
        "scope-denied" => "this client may read workspaces but not change them".to_string(),
        "capability-unsupported" => NO_WORKSPACES.to_string(),
        _ => return refusal(e, fallback),
    };
    Refused::new(said, Some(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> bytes::Bytes {
        bytes::Bytes::copy_from_slice(&[n; 16])
    }

    fn repository(n: u8, name: &str) -> pb::Repository {
        pb::Repository { id: id(n), display_name: name.into(), ..Default::default() }
    }

    fn workspace(n: u8, repository: u8, name: &str, prefix: &str) -> pb::Workspace {
        pb::Workspace {
            id: id(n),
            repository_id: id(repository),
            name: name.into(),
            task_prefix: prefix.into(),
            is_main: name == "Main",
            ..Default::default()
        }
    }

    /// Two repositories, each with a Main and a Billing.
    fn two_repositories() -> (Vec<pb::Repository>, Vec<pb::Workspace>) {
        (
            vec![repository(1, "api"), repository(2, "web")],
            vec![
                workspace(11, 1, "Main", "api"),
                workspace(12, 1, "Billing", "bil"),
                workspace(21, 2, "Main", "web"),
                workspace(22, 2, "Billing", "wbil"),
            ],
        )
    }

    /// Name, ignoring case, then prefix, then id; a name two workspaces share
    /// is refused, naming both and where they are.
    #[test]
    fn a_workspace_is_named_by_name_then_prefix_then_id() {
        let (repositories, all) = two_repositories();
        let api = scope(&all, &repositories, Some("api"), None).unwrap();
        assert_eq!(resolve_workspace(&api, &repositories, "billing").unwrap().id, id(12));
        assert_eq!(resolve_workspace(&all, &repositories, "WBIL").unwrap().id, id(22));
        let tail = &uuid_of(&id(21)).simple().to_string()[24..];
        assert_eq!(resolve_workspace(&all, &repositories, tail).unwrap().id, id(21));

        let refused = resolve_workspace(&all, &repositories, "Billing").unwrap_err();
        assert!(refused.contains("in api") && refused.contains("in web") && refused.contains("--repo"), "{refused}");
        assert!(resolve_workspace(&all, &repositories, "Payments").is_err());
    }

    /// A bare name is looked for in `--repo`, then the only repository, then
    /// the pane's own workspace's repository, and only then everywhere.
    #[test]
    fn a_bare_name_is_looked_for_in_the_panes_own_repository() {
        let (repositories, all) = two_repositories();
        let in_web = scope(&all, &repositories, None, Some(uuid_of(&id(21)))).unwrap();
        assert_eq!(resolve_workspace(&in_web, &repositories, "Billing").unwrap().id, id(22));
        assert_eq!(resolve_workspace(&in_web, &repositories, "main").unwrap().id, id(21));
        // `--repo` beats the pane.
        let in_api = scope(&all, &repositories, Some("api"), Some(uuid_of(&id(21)))).unwrap();
        assert_eq!(resolve_workspace(&in_api, &repositories, "Main").unwrap().id, id(11));
        // A pane whose workspace is gone is no hint at all.
        assert_eq!(scope(&all, &repositories, None, Some(Uuid::from_u128(9))).unwrap().len(), 4);
        // One repository needs no hint.
        let one = scope(&all[..2], &repositories[..1], None, None).unwrap();
        assert_eq!(resolve_workspace(&one, &repositories, "Billing").unwrap().id, id(12));
    }

    /// F50: `create`'s repository is optional. The pane's workspace's
    /// repository, then the only one, and otherwise a sentence naming
    /// `--repo`.
    #[test]
    fn a_new_workspace_goes_in_the_panes_repository_or_the_only_one() {
        let (repositories, all) = two_repositories();
        assert_eq!(repository_to_create_in(&repositories, &all, None, Some(uuid_of(&id(22)))), Ok(uuid_of(&id(2))));
        assert_eq!(repository_to_create_in(&repositories, &all, Some("api"), Some(uuid_of(&id(22)))), Ok(uuid_of(&id(1))));
        assert_eq!(repository_to_create_in(&repositories[..1], &all, None, None), Ok(uuid_of(&id(1))));
        let asked = repository_to_create_in(&repositories, &all, None, None).unwrap_err();
        assert!(asked.contains("--repo"), "{asked}");
    }

    #[test]
    fn a_pane_workspace_is_a_uuid_or_nothing() {
        let ws = Uuid::now_v7();
        assert_eq!(pane_workspace(Some(format!(" {ws} "))), Some(ws));
        assert_eq!(pane_workspace(Some(String::new())), None);
        assert_eq!(pane_workspace(Some("Billing".into())), None);
        assert_eq!(pane_workspace(None), None);
    }

    /// The spec's spelling works as written, and the repository may be a
    /// positional or `--repo`.
    #[test]
    fn create_takes_a_name_and_a_prefix_and_an_optional_repository() {
        use clap::Parser;
        let parsed = |line: &str| match crate::Cli::try_parse_from(line.split_whitespace()).expect(line).command {
            crate::Command::Workspace(WorkspaceCmd::Create(args)) => args,
            _ => panic!("{line}"),
        };
        let a = parsed("farcooler workspace create --name Billing --prefix bil");
        assert_eq!(a.checked(), Ok(Creating { repo: None, name: "Billing", prefix: "bil" }));
        let a = parsed("farcooler workspace create api --name Billing --prefix bil");
        assert_eq!(a.checked(), Ok(Creating { repo: Some("api"), name: "Billing", prefix: "bil" }));
        let a = parsed("farcooler workspace create --repo api --name Billing --prefix bil");
        assert_eq!(a.checked(), Ok(Creating { repo: Some("api"), name: "Billing", prefix: "bil" }));
        assert!(parsed("farcooler workspace create --name Billing").checked().is_err());
        assert!(parsed("farcooler workspace create --prefix bil").checked().is_err());
        // A name typed where the old spelling put one is asked for by flag,
        // not taken as a worktree.
        let said = parsed("farcooler workspace create api Billing --prefix bil").checked().unwrap_err();
        assert!(said.contains("--name Billing") && !said.contains("worktree"), "{said}");
    }

    /// `workspace` refuses the old spelling of `workspace create` before it
    /// connects to anything: the link it would have used is never awaited.
    #[tokio::test]
    async fn the_old_create_spelling_is_refused_before_connecting() {
        use clap::Parser;
        let argv = "farcooler workspace create repo fix-it --branch fix-it".split_whitespace();
        let crate::Command::Workspace(cmd) = crate::Cli::try_parse_from(argv).expect("parses").command else {
            panic!("workspace create")
        };
        let connecting = async { Err::<Link, Box<dyn Error>>("connected to a runner".into()) };
        let said = workspace_via(cmd, false, connecting).await.expect_err("refused").to_string();
        assert!(said.contains("farcooler worktree create repo fix-it --branch fix-it"), "{said}");
    }

    /// Every refusal the runner names a word for is this CLI's sentence for
    /// it, never the word, and keeps the code for `--json`.
    #[test]
    fn a_workspace_refusal_is_said_for_its_word_and_keeps_its_code() {
        for what in ["task_prefix_taken", "main_workspace", "workspace_not_empty", "orchestrator_taken", "other_repository"] {
            let refused = workspace_refused(
                ClientError::Daemon {
                    code: pb::ErrorCode::InvalidArgument as i32,
                    retryable: false,
                    message: format!("invalid argument: {what}"),
                    what: what.into(),
                },
                "missing",
                "fallback",
            );
            assert_eq!(refused.to_string(), said_about(what).unwrap(), "{what}");
            assert_eq!(refused.word(), Some("invalid-argument"));
        }
        let unknown = workspace_refused(
            ClientError::Daemon {
                code: pb::ErrorCode::InvalidArgument as i32,
                retryable: false,
                message: "invalid argument: some_future_field".into(),
                what: "some_future_field".into(),
            },
            "missing",
            "fallback",
        );
        assert_eq!(unknown.to_string(), "fallback");
    }

    /// Each write names the capability, so an older runner refuses rather
    /// than answering a method or field it doesn't know.
    #[test]
    fn every_workspace_write_names_the_capability_it_needs() {
        let a = assign_request(Uuid::from_u128(1), Uuid::from_u128(2));
        assert_eq!(a.required_capabilities, [capability::WORKSTREAMS]);
        assert_eq!(a.target_resource_id.as_deref(), Some(Uuid::from_u128(1).as_bytes().as_slice()));
        let Some(request::Payload::WorktreeAssign(p)) = a.payload else { panic!("payload") };
        assert_eq!(p.workspace_id.as_ref(), Uuid::from_u128(2).as_bytes());

        for (role, wire) in [
            (Role::Orchestrator, pb::TerminalRole::Orchestrator),
            (Role::Agent, pb::TerminalRole::Agent),
            (Role::Shell, pb::TerminalRole::Shell),
        ] {
            let r = set_role_request(Uuid::from_u128(3), role);
            assert_eq!(r.required_capabilities, [capability::WORKSTREAMS]);
            let Some(request::Payload::TerminalSetRole(p)) = r.payload else { panic!("payload") };
            assert_eq!(p.role, wire as i32, "{role:?}");
        }
    }
}
