//! How each harness is launched as a workspace's orchestrator.
//!
//! An orchestrator runs from its workspace's home (`workspace_home`), not
//! from the repository, so the charter and the conversation are the
//! workstream's own. It still has to see the repository's instructions,
//! skills and memory, and each harness finds those differently. Measured on
//! this runner on 2026-09-27 (see "Spike findings" in
//! `docs/superpowers/specs/2026-09-27-workspaces-as-workstreams-design.md`):
//!
//! | | Working directory | Repository context | Memory |
//! |---|---|---|---|
//! | Claude Code | the home | `--add-dir <main>`, `--project-config-root <main>` and `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1` | `autoMemoryDirectory` in the `--settings` file |
//! | Cursor | the home | `--workspace <main>` | none |
//! | Codex | the main checkout | `--cd <main>`, which only restates it | none |
//!
//! - **Claude Code** reads `CLAUDE.md` and skills from an added directory only
//!   with that variable. Its memory directory is chosen from the git root of
//!   the working directory, which the home doesn't have, so it's pointed back
//!   at the repository's (`claude_memory_dir`). An added directory's
//!   `.claude/settings.json` and `.claude/settings.local.json` are NOT read
//!   (measured: a `SessionStart` hook in either didn't fire from the home).
//!   `--project-config-root <main>` reads them, and `.mcp.json`, from the
//!   repository instead (measured on 2026-09-28: the hook fired). Its
//!   permission allowlist is applied only once claude trusts the home, which
//!   is claude's own prompt to ask; Far Cooler doesn't write that trust.
//!   `--add-dir` stays, for `CLAUDE.md` and access to the files. Far Cooler's
//!   hooks reach it through `--settings`.
//! - **Cursor** reads `AGENTS.md`, `CLAUDE.md` and `.cursor/rules` from the
//!   root `--workspace` names.
//! - **Codex** reads `AGENTS.md`, skills and `.codex/config.toml` from its
//!   working directory's walk up to `.git`, and its `--add-dir` is only a
//!   sandbox grant, so it has to run in the repository.
//!
//! **Every path handed to a harness is resolved** (`realpath`). Cursor keeps
//! workspace trust per path as spelled, and refused `--workspace /tmp/…` in a
//! directory it had trusted as `/private/tmp/…`. Codex keys hook trust to the
//! resolved path, and skipped a repository's hooks when handed `--cd /tmp/…`.
//! Claude Code names its memory directory after the resolved path too.

use std::path::{Path, PathBuf};

use uuid::Uuid;

pub use crate::skill_install::Harness;

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
            home: crate::workspace_home::home(root, workspace),
            charter: crate::workspace_home::charter_path(root, workspace),
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

/// The harness a preset runs, with or without a model: `claude:opus` is
/// Claude Code. `None` for anything that can't orchestrate.
pub fn harness_of(preset: &str) -> Option<Harness> {
    match preset.split_once(':').map_or(preset, |(agent, _)| agent) {
        "claude" => Some(Harness::Claude),
        "codex" => Some(Harness::Codex),
        "cursor" => Some(Harness::Cursor),
        _ => None,
    }
}

/// The directory the orchestrator's pane starts in: the home for Claude Code
/// and Cursor, the main checkout for Codex.
pub fn working_directory(h: Harness, l: &OrchestratorLaunch) -> PathBuf {
    match h {
        Harness::Claude | Harness::Cursor => l.home.clone(),
        Harness::Codex => l.main_checkout.clone(),
    }
}

/// The arguments that point the harness back at the repository, unquoted.
///
/// Codex's `--cd` names the directory it already starts in, so that it
/// holds the resolved spelling whatever the pane's shell reports.
///
/// Claude Code's `--project-config-root` is hidden (2.1.283 doesn't list it
/// in `--help`) and is passed without checking the installed claude takes
/// it: a claude that didn't would stop at "unknown option". claude 2.1.283
/// takes it on a fresh launch, with `--resume`, and with the native chat's
/// flags, and refuses a directory that doesn't exist.
pub fn extra_args(h: Harness, l: &OrchestratorLaunch) -> Vec<String> {
    let main = l.main_checkout.to_string_lossy().into_owned();
    match h {
        Harness::Claude => vec!["--add-dir".into(), main.clone(), "--project-config-root".into(), main],
        Harness::Cursor => vec!["--workspace".into(), main],
        Harness::Codex => vec!["--cd".into(), main],
    }
}

/// The variables the orchestrator's pane exports, unquoted.
///
/// Every harness: its workspace, its charter, and `FARCOOLER_ACTOR=manager`,
/// so the board files its writes as the manager rather than as an agent pane.
/// Claude Code also gets the switch that makes it read the added directory's
/// `CLAUDE.md`.
pub fn extra_env(h: Harness, l: &OrchestratorLaunch) -> Vec<(String, String)> {
    use farcooler_core::pane_env;
    let mut env = vec![
        (pane_env::WORKSPACE.to_string(), l.workspace.to_string()),
        (pane_env::CHARTER.to_string(), l.charter.to_string_lossy().into_owned()),
        (pane_env::ACTOR.to_string(), "manager".to_string()),
    ];
    if h == Harness::Claude {
        env.push((ADDITIONAL_DIRECTORIES_CLAUDE_MD.to_string(), "1".to_string()));
    }
    env
}

/// The first message an orchestrator starts on, so its first turn runs the
/// manager skill instead of waiting for the owner to ask for it.
///
/// The skill is installed with `disable-model-invocation: true`
/// (`skill_install::frontmatter`), so a model is never told it exists and
/// can't reach for it: it runs only when it's invoked by name, and that's
/// what this is, in each harness's own spelling (`skill_install`'s module
/// doc):
///
/// - **claude**: `/farcooler:manager`, the plugin's namespaced command.
/// - **codex**: `$farcooler-manager`, an explicit mention of the copy in the
///   main checkout. `agents/openai.yaml` turns off only implicit invocation.
/// - **cursor**: `/manager`. cursor-agent submits its `[prompt...]` through
///   the same send path as a typed message, which reads the skills a message
///   names (read in cursor-agent 2026.09.26's bundle, not run).
///
/// Each harness takes it as its positional prompt, sent once on the first
/// launch (`Service::open_terminal`); a restart doesn't run it again.
///
/// `handoff` is a task on the workspace's own board whose notes the new
/// orchestrator should read first: the one a split writes its handoff on.
/// It goes after the invocation, as the skill's argument.
pub fn first_prompt(h: Harness, handoff: Option<&str>) -> String {
    let skill = match h {
        Harness::Claude => "/farcooler:manager",
        Harness::Codex => "$farcooler-manager",
        Harness::Cursor => "/manager",
    };
    match handoff {
        Some(key) => format!("{skill} Read the handoff note on {key} first."),
        None => skill.to_string(),
    }
}

/// Claude Code's switch for reading `CLAUDE.md` from an `--add-dir`.
pub const ADDITIONAL_DIRECTORIES_CLAUDE_MD: &str = "CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD";

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

#[cfg(test)]
mod tests {
    use super::*;

    fn launch() -> OrchestratorLaunch {
        OrchestratorLaunch {
            workspace: Uuid::nil(),
            home: "/h/ws".into(),
            main_checkout: "/r".into(),
            charter: "/h/ws/charter.md".into(),
            memory_dir: Some("/m".into()),
        }
    }

    #[test]
    fn claude_and_cursor_live_in_the_home_and_codex_in_the_repository() {
        assert_eq!(working_directory(Harness::Claude, &launch()), PathBuf::from("/h/ws"));
        assert_eq!(working_directory(Harness::Cursor, &launch()), PathBuf::from("/h/ws"));
        assert_eq!(working_directory(Harness::Codex, &launch()), PathBuf::from("/r"));
    }

    #[test]
    fn each_harness_is_pointed_back_at_the_repository() {
        assert_eq!(extra_args(Harness::Claude, &launch()), ["--add-dir", "/r", "--project-config-root", "/r"]);
        assert_eq!(extra_args(Harness::Cursor, &launch()), ["--workspace", "/r"]);
        assert_eq!(extra_args(Harness::Codex, &launch()), ["--cd", "/r"]);
    }

    #[test]
    fn every_orchestrator_is_told_its_workspace_and_charter_and_is_the_manager() {
        let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
        for h in [Harness::Claude, Harness::Codex, Harness::Cursor] {
            let env = extra_env(h, &launch());
            assert!(env.contains(&pair("FARCOOLER_WORKSPACE", &Uuid::nil().to_string())), "{h:?}");
            assert!(env.contains(&pair("FARCOOLER_CHARTER", "/h/ws/charter.md")), "{h:?}");
            assert!(env.contains(&pair("FARCOOLER_ACTOR", "manager")), "{h:?}");
            let claude_md = env.contains(&pair(ADDITIONAL_DIRECTORIES_CLAUDE_MD, "1"));
            assert_eq!(claude_md, h == Harness::Claude, "{h:?}");
        }
    }

    /// Each harness starts on its own spelling of the manager skill, and
    /// a handoff is named after it.
    #[test]
    fn each_harness_starts_on_the_manager_skill() {
        assert_eq!(first_prompt(Harness::Claude, None), "/farcooler:manager");
        assert_eq!(first_prompt(Harness::Codex, None), "$farcooler-manager");
        assert_eq!(first_prompt(Harness::Cursor, None), "/manager");
        assert_eq!(
            first_prompt(Harness::Claude, Some("BIL-4")),
            "/farcooler:manager Read the handoff note on BIL-4 first."
        );
        assert_eq!(
            first_prompt(Harness::Codex, Some("BIL-4")),
            "$farcooler-manager Read the handoff note on BIL-4 first."
        );
        assert_eq!(first_prompt(Harness::Cursor, Some("BIL-4")), "/manager Read the handoff note on BIL-4 first.");
    }

    /// The rule the spike measured, on its own examples.
    #[test]
    fn claude_memory_follows_the_repository() {
        assert_eq!(
            claude_memory_dir(Path::new("/Users/me"), Path::new("/nonexistent/Dev/overnight")),
            PathBuf::from("/Users/me/.claude/projects/-nonexistent-Dev-overnight/memory"),
        );
        assert_eq!(
            slug(Path::new("/private/tmp/fc-ws/spike/repo/.worktrees/nested")),
            "-private-tmp-fc-ws-spike-repo--worktrees-nested"
        );
        assert_eq!(slug(Path::new("/a b/c_d.e")), "-a-b-c-d-e");
    }

    /// A main checkout reached through a link is named by where it really
    /// is, in the memory directory and in every argument a harness gets.
    #[test]
    fn the_repository_is_named_by_its_resolved_path() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let real = real.canonicalize().unwrap();

        let l = OrchestratorLaunch::new(Path::new("/root"), Uuid::nil(), &link, Some(Path::new("/u")));
        assert_eq!(l.main_checkout, real);
        assert_eq!(extra_args(Harness::Cursor, &l)[1], real.to_string_lossy());
        assert_eq!(working_directory(Harness::Codex, &l), real);
        assert_eq!(l.memory_dir, Some(Path::new("/u/.claude/projects").join(slug(&real)).join("memory")));
        assert_eq!(l.home, PathBuf::from(format!("/root/workspaces/{}", Uuid::nil())));
        assert_eq!(l.charter, l.home.join("charter.md"));
    }

    #[test]
    fn only_the_three_harnesses_orchestrate() {
        assert_eq!(harness_of("claude"), Some(Harness::Claude));
        assert_eq!(harness_of("claude:opus"), Some(Harness::Claude));
        assert_eq!(harness_of("codex:gpt-5.6-sol"), Some(Harness::Codex));
        assert_eq!(harness_of("cursor"), Some(Harness::Cursor));
        for other in ["shell", "changes", "aider", "claudette", ""] {
            assert_eq!(harness_of(other), None, "{other}");
        }
    }
}
