//! Inputs to the runner's terminal launch builders.

use std::path::{Path, PathBuf};

use uuid::Uuid;

/// What Far Cooler hands a claude pane beyond its preset: both are files in
/// this daemon's runtime directory. `--settings` is claude's alone;
/// `--plugin-dir` is claude's and cursor's.
///
/// - `settings` is `--settings <file>`, the hooks that make the pane report
///   itself (`write_claude_hook_settings`).
/// - `plugin_dir` is `--plugin-dir <dir>`, the plugin that carries the manager
///   skill (`write_plugin`), invoked as `/farcooler:manager` in claude and
///   `/manager` in cursor.
///
/// One struct rather than a parameter each, because every launch path gets
/// both from one call to `Service::prepare_launch_hooks` and hands both to
/// the same builders; a second parameter threaded beside the first is a
/// second thing each of the four launch paths could forget.
///
/// Two more that are not files:
///
/// - `trust_workspace` is cursor's `--trust`, set by `prepare_launch_hooks`
///   for a worktree this runner made itself (`Service::made_this_worktree`).
///   cursor trusts per directory, and every new worktree is a new directory,
///   so without it every task started on a worktree Far Cooler had just made
///   sat on "Trust this workspace?" until someone answered it. It is on every
///   launch in such a worktree, restarts included: a restarted pane that
///   asked again would be the same gate one click later.
/// - `prompt` is the agent's first message, as its launch argument. **Set by
///   `create_terminal_with_prompt` and nowhere else**, which is how "first
///   launch only" is kept: every other launch path builds its extras from
///   `prepare_launch_hooks`, which never sets it, so a restart, a split or a
///   pane coming back from chat cannot send the task a second time.
/// - `orchestrator` is set when the pane is its workspace's orchestrator
///   (`Service::orchestrator_launch`), and adds the arguments that point the
///   harness back at the repository (`orchestrator::extra_args`). For such a
///   claude pane, `settings` is the orchestrator's own file
///   (`write_orchestrator_settings`), not `claude-hooks.json`.
/// - `rep` is what a member's rep launches with (`reps::RepLaunch`): its
///   permission arguments, its gate's settings file in place of `settings`,
///   and its first message for a restart with nothing to resume. Set by
///   `orchestrate` from the record on every launch, so a resumed rep carries
///   it as a fresh one does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchExtras {
    pub settings: Option<PathBuf>,
    pub plugin_dir: Option<PathBuf>,
    pub trust_workspace: bool,
    pub prompt: Option<LaunchPrompt>,
    pub orchestrator: Option<OrchestratorLaunch>,
    pub rep: Option<RepLaunch>,
    /// A member's `CLAUDE_CONFIG_DIR`, exactly as recorded (`claude_logins`);
    /// `None`, and no variable at all, for the runner's own login.
    pub claude_dir: Option<String>,
    /// A member's pane on the runner's own login: what it's kept from (`shared_logins`).
    pub shared: Option<Kept>,
    /// A member's pane's `CODEX_HOME`, an empty one of theirs (`shared_logins`).
    pub codex_home: Option<String>,
    /// Private file containing the engine-pinned config override, independent of project discovery.
    pub codex_hooks: Option<PathBuf>,
}

/// An agent's first message, and how it reaches the agent's argv.
///
/// Two forms because tmux has a ceiling. The whole `new-window` command,
/// every argument of it, travels from the tmux client to the server as one
/// message, and a message is capped: measured on tmux 3.7c, a 16 300-byte
/// command is accepted and a 17 000-byte one is refused with "command too
/// long". A prompt is quoted twice on its way in, and a pasted stack trace is
/// easily past that. So a long one goes into a file instead, and the command
/// carries the file's name (`MAX_INLINE_LAUNCH_BYTES`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchPrompt {
    /// The text itself, `shell_quote`d into the command.
    Inline(String),
    /// A file in the runtime directory holding the text. The launch reads it
    /// with `"$(cat …)"` and removes it; see `prompt_argument`.
    File(PathBuf),
}

impl LaunchExtras {
    /// An ordinary launch: nothing handed over.
    pub const NONE: LaunchExtras = LaunchExtras {
        settings: None,
        plugin_dir: None,
        trust_workspace: false,
        prompt: None,
        orchestrator: None,
        rep: None,
        claude_dir: None,
        shared: None,
        codex_home: None,
        codex_hooks: None,
    };
}

/// Where an orchestrator runs and what it's pointed back at. Built by
/// `OrchestratorLaunch::new`, which resolves the main checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrchestratorLaunch {
    pub workspace: Uuid,
    /// `<runtime dir>/workspaces/<workspace>`.
    pub home: PathBuf,
    /// The repository's main checkout, resolved.
    pub main_checkout: PathBuf,
    /// `home/charter.md`. It may not exist yet: that's the manager skill's
    /// cue to interview for one.
    pub charter: PathBuf,
    /// Claude Code's `autoMemoryDirectory`. `None` when this user's home
    /// directory can't be found, and then Claude Code picks its own.
    pub memory_dir: Option<PathBuf>,
}

impl OrchestratorLaunch {
    /// The launch for `workspace`, whose home and charter are under `root`
    /// (`workspace_home`), in the repository checked out at `main_checkout`.
    /// `user_home` is this user's home directory, where Claude Code keeps its
    /// memory.
    pub fn new(root: &Path, workspace: Uuid, main_checkout: &Path, user_home: Option<&Path>) -> Self {
        let main_checkout = resolved(main_checkout);
        OrchestratorLaunch {
            workspace,
            home: crate::workspace_paths::home(root, workspace),
            charter: crate::workspace_paths::charter_path(root, workspace),
            memory_dir: user_home.map(|home| claude_memory_dir(home, &main_checkout)),
            main_checkout,
        }
    }
}

/// `path` with every link and `..` resolved, or `path` itself when it can't
/// be (it doesn't exist). See the module doc for why.
fn resolved(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// The memory directory Claude Code gives every session in the repository
/// whose main checkout is `main_checkout`:
/// `<user home>/.claude/projects/<slug>/memory`.
///
/// Measured: every worktree of a repository shares the main checkout's
/// memory directory, and the slug is the resolved path with every character
/// outside `[A-Za-z0-9]` replaced by `-`, so `/private/tmp/r/.worktrees/n`
/// is `-private-tmp-r--worktrees-n`. Nothing here creates it or writes to it:
/// it's handed to Claude Code as `autoMemoryDirectory`.
pub fn claude_memory_dir(user_home: &Path, main_checkout: &Path) -> PathBuf {
    user_home.join(".claude/projects").join(slug(&resolved(main_checkout))).join("memory")
}

/// Claude Code's name for a directory under `~/.claude/projects`.
pub fn slug(path: &Path) -> String {
    path.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// What a rep's launch carries beyond any agent's (`LaunchExtras::rep`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepLaunch {
    /// `permission_args`.
    pub args: Vec<String>,
    /// Its settings file, with the gate hook in it (`rep_settings`).
    pub settings: std::path::PathBuf,
    /// Its first message, sent again by a restart that has no conversation
    /// to resume, so it still starts on its skill.
    pub opening: String,
}

/// What a member's pane on the runner's own login is kept from (review 1,
/// item 1): the owner's connected apps and MCP servers, which aren't the
/// owner's account but things it reaches in the owner's name. Claude:
/// `--strict-mcp-config` with no `--mcp-config`, which drops the user's and
/// project's servers and claude.ai's connectors (claude 2.1.301, as `reps`
/// measured). Codex: its connected apps off (`features.apps`), and each MCP
/// server the owner's or the checkout's config.toml names turned off by
/// name, since `-c mcp_servers={}` merges rather than replaces (codex
/// 0.153.4, measured in a scratch `CODEX_HOME`). Cursor has no such switch:
/// its MCP servers stay, and the admins' copy says so. Hooks, plugins and
/// CLAUDE.md stay: Far Cooler's own hooks need them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Kept {
    /// The codex MCP servers to turn off, by name.
    pub codex_servers: Vec<String>,
}
