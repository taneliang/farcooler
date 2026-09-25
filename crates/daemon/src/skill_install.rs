//! Installing the manager skill without owning the directory it lives in.
//!
//! The manager is a skill every agent Far Cooler launches can be asked to
//! become. How each agent finds it was measured on this runner, not read off a
//! doc (claude 2.1.282, codex-cli 0.153.4, cursor-agent 2026.09.23):
//!
//! - **claude** loads a plugin directory named by `--plugin-dir`, for that
//!   session only, and invokes its skill as `/farcooler:manager`.
//! - **cursor-agent** accepts `--plugin-dir` too, and invokes the same skill
//!   as `/manager`, with no namespace. One directory carrying both
//!   `.claude-plugin/plugin.json` and `.cursor-plugin/plugin.json` loads in
//!   both. Far Cooler writes it into its own runtime directory (`PLUGIN_DIR`),
//!   the way it writes `claude-hooks.json` for `--settings`, so nothing of the
//!   user's and nothing in the repository is touched, and it is regenerated on
//!   every launch.
//! - **codex** reads `.agents/skills/<name>/SKILL.md` in the directory it was
//!   opened in, and nothing per session: `-c skills.config=[{path=…}]` from
//!   outside a repository did not add a skill, whether `path` named the file
//!   or its directory. So codex alone gets a copy in the worktree, invoked as
//!   `$farcooler-manager`, and `agents/openai.yaml` keeps the model from
//!   reaching for it on its own.
//!
//! cursor ALSO reads `.agents/skills`, so in a worktree where codex has run a
//! cursor pane lists codex's copy as `/farcooler-manager` beside its own
//! `/manager`. That copy keeps `disable-model-invocation: true` so cursor's
//! model can't reach for it either, and it is deliberately named differently
//! from the plugin's: see `the_worktree_copy_never_shadows_the_plugin_skill`.
//!
//! **Ours is recorded in the file**, because a skill is a whole file rather
//! than an entry in a shared list the way a hook is. The last line of every
//! file written into a worktree is a marker carrying the sha256 of everything
//! above it. A file whose marker still matches is one we wrote and nobody has
//! edited, and it is the only kind this module ever replaces or removes.
//! Anything else — edited, or never ours — is left exactly as it is.
//!
//! **Opting out** of codex's copy in one repository: put a file of your own,
//! without our marker, at `.agents/skills/farcooler-manager/SKILL.md` (an
//! empty one will do). Deleting our copy doesn't last, because the next codex
//! launch writes it again.

use std::path::Path;

use sha2::{Digest, Sha256};

/// Which agent a copy of the skill is rendered for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Harness {
    Claude,
    Codex,
    Cursor,
}

/// One file of a rendered skill: where it goes, relative to the directory the
/// harness reads, and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFile {
    pub relative: &'static str,
    pub contents: String,
}

/// What an install did, so a caller can log it and a test can pin it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Installed {
    /// The file was missing, or was an unedited copy of ours that said
    /// something else.
    Wrote,
    /// The file already says exactly this. Nothing was touched, not even the
    /// mtime.
    Unchanged,
    /// A file is there that isn't an unedited copy of ours, so it's somebody's.
    LeftAlone(&'static str),
    /// It should have been written and the write failed.
    Failed,
}

/// Where the plugin claude and cursor are handed lives, relative to this
/// daemon's runtime directory.
pub const PLUGIN_DIR: &str = "farcooler-plugin";

/// The skill codex reads, relative to a worktree's root. cursor sees it too.
pub const PROJECT_SKILL: &str = ".agents/skills/farcooler-manager/SKILL.md";

/// codex's invocation policy for it. cursor never reads this file.
pub const PROJECT_SKILL_POLICY: &str = ".agents/skills/farcooler-manager/agents/openai.yaml";

/// Every file Far Cooler writes into a worktree for the skill.
///
/// Each is added to the repository's `info/exclude` before it is written
/// (`service::exclude_locally`), and `service::holds_an_unseen_skill_file`
/// reads this list so that removing a worktree still asks when one of them
/// holds something that isn't an unedited copy of ours.
pub const PROJECT_SKILL_FILES: &[&str] = &[PROJECT_SKILL, PROJECT_SKILL_POLICY];

/// The skill's text, with `{{frontmatter}}`, `{{cli}}` and `{{wait}}` still to
/// fill. Kept as a file so it reads as what an agent reads.
const BODY: &str = include_str!("../assets/manager/SKILL.md");

/// The frontmatter every copy carries, differing only in `name`: the plugin's
/// is `manager` (claude namespaces it as `/farcooler:manager`, cursor shows it
/// as `/manager`), and codex's `.agents/skills` copy, which sits beside
/// whatever other skills a repository has, says whose it is in its name.
///
/// `disable-model-invocation: true` for claude and cursor, which both read it:
/// measured, a model with this skill installed answered that it had no such
/// skill available. codex ignores the key (measured: it still loads the file)
/// and reads `agents/openai.yaml` instead, but its copy keeps the key because
/// cursor reads `.agents/skills` as well.
fn frontmatter(name: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: Manage this repository's Far Cooler task board. Reads the \
         charter, reads the board, then creates, revises, answers or reports. Never does the work \
         itself. Use only when the owner asks for the manager.\ndisable-model-invocation: true\n---\n"
    )
}

/// The headings a charter carries, in order. The interview is done when the
/// charter has every one of them; a charter missing some is interviewed for
/// those alone.
pub const CHARTER_SECTIONS: &[&str] = &[
    "## Workflow",
    "## Done means",
    "## Review",
    "## Who decides",
    "## Reaching me",
    "## Lanes",
    "## Autonomy",
    "## Anything else",
];

/// Whether anything can wake a manager that has ended its turn. Nothing can
/// yet (see `WAIT_STEP`).
pub const WAKE_LOOP_EXISTS: bool = false;

/// Step 4 of the skill: stop, and say so honestly.
///
/// The spec's step 4 is "wait to be woken", and the wake loop that would do
/// the waking needs Phase 3 of live-agent-sessions (typing into a running
/// agent's pane when its screen is ready), which doesn't exist yet. Until it
/// does, a manager that says "I'll check back" is making a promise nothing on
/// the runner keeps, and the owner finds out by waiting. So the step tells the
/// manager to end its turn and say what it's waiting on.
///
/// `the_skill_promises_no_wake_while_none_exists` holds this to that while
/// `WAKE_LOOP_EXISTS` is false.
pub const WAIT_STEP: &str = "Stop. Nothing will wake you: Far Cooler can't yet type into this pane when \
the board changes. End your turn by telling the owner in one line what you're waiting on, and that \
you'll look again when they next talk to you. Don't poll the board in a loop, don't sleep, and \
don't say you'll check back.";

/// codex's switch for the same thing, measured the same way.
const CODEX_POLICY: &str = "policy:\n  allow_implicit_invocation: false\n";

/// Every copy of the skill for `harness`, with `cli` standing in for every
/// `{{cli}}` in the body. `cli` is expected to be shell-quoted already.
///
/// The worktree copies are signed (`sign_markdown`, `sign_yaml`), because a
/// worktree is somebody else's directory and the signature is how a later
/// install or `remove_ours` tells our unedited file from theirs. The plugin
/// claude and cursor get goes to Far Cooler's own runtime directory and is not
/// signed: nobody else writes there, and it is rewritten whenever it differs.
pub fn render(harness: Harness, cli: &str) -> Vec<SkillFile> {
    match harness {
        // One plugin directory for both: each reads its own manifest and the
        // same `skills/manager/SKILL.md`.
        Harness::Claude | Harness::Cursor => vec![
            SkillFile {
                relative: ".claude-plugin/plugin.json",
                contents: "{\"name\":\"farcooler\"}\n".to_string(),
            },
            SkillFile {
                relative: ".cursor-plugin/plugin.json",
                contents: "{\"name\":\"farcooler\"}\n".to_string(),
            },
            SkillFile { relative: "skills/manager/SKILL.md", contents: body("manager", cli) },
        ],
        Harness::Codex => vec![
            SkillFile { relative: PROJECT_SKILL, contents: sign_markdown(&body("farcooler-manager", cli)) },
            SkillFile { relative: PROJECT_SKILL_POLICY, contents: sign_yaml(CODEX_POLICY) },
        ],
    }
}

/// The body with its frontmatter and CLI filled in.
fn body(name: &str, cli: &str) -> String {
    BODY.replace("{{frontmatter}}", &frontmatter(name))
        .replace("{{wait}}", WAIT_STEP)
        .replace("{{cli}}", cli)
}

/// The word every marker carries.
const MARKER: &str = "farcooler-managed sha256:";

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// `body`, then a last line naming the sha256 of `body`, as an HTML comment
/// a Markdown reader doesn't render.
pub(crate) fn sign_markdown(body: &str) -> String {
    format!("{body}<!-- {MARKER}{} -->\n", digest(body))
}

/// The same for YAML, as a comment.
fn sign_yaml(body: &str) -> String {
    format!("{body}# {MARKER}{}\n", digest(body))
}

/// What a file's own last line says about it.
#[derive(Debug, PartialEq, Eq)]
enum Ownership {
    /// Our marker is the last line and its hash matches everything above it.
    OursUnedited,
    /// Our marker is in it somewhere, but the file is not what we wrote.
    Edited,
    /// No marker at all.
    NotOurs,
}

fn ownership(text: &str) -> Ownership {
    if !text.contains(MARKER) {
        return Ownership::NotOurs;
    }
    let Some(trimmed) = text.strip_suffix('\n') else { return Ownership::Edited };
    let start = trimmed.rfind('\n').map_or(0, |i| i + 1);
    let (above, last) = (&text[..start], &trimmed[start..]);
    let hex = last
        .strip_prefix("<!-- ")
        .and_then(|l| l.strip_suffix(" -->"))
        .or_else(|| last.strip_prefix("# "))
        .and_then(|l| l.strip_prefix(MARKER));
    match hex {
        Some(hex) if hex == digest(above) => Ownership::OursUnedited,
        _ => Ownership::Edited,
    }
}

/// What sits at one of our paths, read the way `install_file` reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holds {
    /// Nothing is there.
    Nothing,
    /// An unedited copy of ours, of this Far Cooler or an older one.
    OursUnedited,
    /// Anything else: an edited copy, a file we never wrote, or something we
    /// can't read. Somebody's, and not ours to lose or replace.
    Somebodys,
}

/// What `path` holds. `Somebodys` for anything we can't read, a directory
/// included, because every caller would rather ask than guess.
pub fn holds(path: &Path) -> Holds {
    match std::fs::read(path) {
        Ok(bytes) => match ownership(&String::from_utf8_lossy(&bytes)) {
            Ownership::OursUnedited => Holds::OursUnedited,
            Ownership::Edited | Ownership::NotOurs => Holds::Somebodys,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Holds::Nothing,
        Err(_) => Holds::Somebodys,
    }
}

/// Whether `relative` under `root` passes through a symbolic link: any
/// directory on the way, or the file itself. Every component that exists is
/// asked with `symlink_metadata`, which doesn't follow the link it is asked
/// about. A component that doesn't exist yet ends the walk, since nothing
/// below it exists either.
///
/// A repository whose `.agents` links to a shared or user-level skills
/// directory would otherwise have our copy written into that directory, which
/// is outside the worktree and outside anything Far Cooler was asked to touch.
/// A `SKILL.md` that is itself a link is somebody's arrangement, and replacing
/// it with a file would undo it.
pub fn crosses_a_symlink(root: &Path, relative: &str) -> bool {
    let mut at = root.to_path_buf();
    for part in Path::new(relative).components() {
        at.push(part);
        match std::fs::symlink_metadata(&at) {
            Ok(meta) if meta.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

/// Write `contents` to `path` unless somebody else's file is there.
///
/// - A missing file is written.
/// - A file that already says `contents` is not touched at all. Every launch
///   of a codex or cursor pane comes through here, and a byte-identical
///   rewrite would still bump the mtime, wake every watcher on the runner and
///   push the file out of git's stat cache.
/// - An unedited copy of ours that says something else — an older Far
///   Cooler's — is replaced.
/// - Anything else is left exactly as it is and the reason is returned: an
///   owner who edited our copy has taken it, and a file we never wrote is not
///   ours to touch.
///
/// The write goes through a temporary file beside the target and a rename, so
/// a harness that reads the skill mid-write never sees half of one. The file
/// is read again just before the rename, and a save that landed since the
/// first read is kept rather than overwritten. That narrows the window between
/// deciding and replacing to one read; it doesn't close it, and nothing short
/// of a lock every editor honors could.
pub fn install_file(path: &Path, contents: &str) -> Installed {
    install_file_between(path, contents, || {})
}

/// `install_file`, running `between` after the temporary file is written and
/// before the file is read again, which is where a test puts the owner's save.
fn install_file_between(path: &Path, contents: &str, between: impl FnOnce()) -> Installed {
    let before = match std::fs::read(path) {
        Ok(existing) => {
            if existing == contents.as_bytes() {
                return Installed::Unchanged;
            }
            match ownership(&String::from_utf8_lossy(&existing)) {
                Ownership::OursUnedited => {}
                Ownership::Edited => return Installed::LeftAlone("edited"),
                Ownership::NotOurs => return Installed::LeftAlone("not ours"),
            }
            Some(existing)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Installed::LeftAlone("unreadable"),
    };
    let unmoved = || {
        between();
        std::fs::read(path).ok() == before
    };
    match write_through_temp(path, contents, unmoved) {
        Ok(true) => Installed::Wrote,
        Ok(false) => Installed::LeftAlone("changed while writing"),
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "could not write the manager skill");
            Installed::Failed
        }
    }
}

/// Write through a sibling temporary file and a rename.
pub(crate) fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    write_through_temp(path, contents, || true).map(|_| ())
}

/// Tells one write's temporary file from another's in the same process.
static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Write `contents` to a temporary file beside `path`, then rename it over
/// `path` if `still` says to, and say whether it did.
///
/// The temporary name carries the process id and a count of writes in this
/// process, so no two writes share one: two panes launching at once in one
/// daemon, each writing the plugin, would otherwise truncate the file the
/// other was about to rename.
fn write_through_temp(path: &Path, contents: &str, still: impl FnOnce() -> bool) -> std::io::Result<bool> {
    let dir = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().ok_or(std::io::ErrorKind::InvalidInput)?.to_string_lossy();
    let write = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = dir.join(format!(".{name}.farcooler-{}-{write}.tmp", std::process::id()));
    std::fs::write(&temp, contents)?;
    if !still() {
        let _ = std::fs::remove_file(&temp);
        return Ok(false);
    }
    std::fs::rename(&temp, path).map(|()| true).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

/// Remove `path` only if it is an unedited copy of ours, and then the
/// directories it leaves empty that are ours too: `agents/` and the
/// `farcooler-manager` directory. `.agents/skills` and `.agents` are shared
/// with every other skill a repository has and are never removed.
///
/// `std::fs::remove_dir` refuses a directory with anything in it, which is
/// the whole of "only if empty".
///
/// **No production caller**, exactly like `hook_install::remove_ours`: Far
/// Cooler has no uninstall flow. This exists so one can be written without
/// working out the ownership rule again.
pub fn remove_ours(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    if ownership(&text) != Ownership::OursUnedited || std::fs::remove_file(path).is_err() {
        return false;
    }
    let mut dir = path.parent();
    while let Some(d) = dir {
        let name = d.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if !matches!(name, "agents" | "farcooler-manager") || std::fs::remove_dir(d).is_err() {
            break;
        }
        if name == "farcooler-manager" {
            break;
        }
        dir = d.parent();
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    /// A file of ours with `body` above the marker, the shape `render` gives
    /// every file it writes into a worktree.
    fn signed(body: &str) -> String {
        sign_markdown(body)
    }

    /// The first install writes, and says so.
    #[test]
    fn a_missing_skill_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROJECT_SKILL);
        let v1 = signed("# v1\n");
        assert_eq!(install_file(&path, &v1), Installed::Wrote);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), v1);
    }

    /// The second install costs nothing: same bytes, same file. This is the
    /// rule `install_project_hook_file` keeps for hooks, and for the same
    /// reason: every launch comes through here. A rewrite goes through a
    /// rename, which would give the path a new inode, so the inode is what
    /// catches a rewrite even inside one tick of the mtime clock.
    #[test]
    fn installing_the_same_skill_again_does_not_touch_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROJECT_SKILL);
        let v1 = signed("# v1\n");
        install_file(&path, &v1);
        let before = std::fs::metadata(&path).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(install_file(&path, &v1), Installed::Unchanged);
        let after = std::fs::metadata(&path).unwrap();
        assert_eq!(before.ino(), after.ino(), "the file was replaced");
        assert_eq!(before.modified().unwrap(), after.modified().unwrap(), "the file was rewritten");
    }

    /// Two writes at once in one process, the way two panes launching
    /// together each write the plugin. Every write lands whole and none fails:
    /// with one temporary name per process, one write's rename takes the
    /// other's file away and that write fails.
    #[test]
    fn two_writes_at_once_both_land_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SKILL.md");
        let texts = ["a".repeat(256 * 1024), "b".repeat(256 * 1024)];
        std::thread::scope(|scope| {
            for text in &texts {
                let path = &path;
                scope.spawn(move || {
                    for _ in 0..200 {
                        write_atomically(path, text).expect("a write failed");
                    }
                });
            }
        });
        let now = std::fs::read_to_string(&path).unwrap();
        assert!(texts.contains(&now), "a torn file of {} bytes", now.len());
        let left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left.len(), 1, "temporary files were left behind: {left:?}");
    }

    /// A copy the owner saves between the first read and the rename is kept.
    /// The save lands after our temporary file is written, the last moment
    /// before `install_file` would rename it over theirs.
    #[test]
    fn a_save_that_lands_mid_install_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROJECT_SKILL);
        install_file(&path, &signed("# v1\n"));
        let save = || std::fs::write(&path, "the owner's save\n").unwrap();
        assert_eq!(
            install_file_between(&path, &signed("# v2\n"), save),
            Installed::LeftAlone("changed while writing")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "the owner's save\n");
        let left: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten().collect();
        assert_eq!(left.len(), 1, "the temporary file was left behind");
    }

    /// `crosses_a_symlink` sees a link at any depth, the file included, and
    /// nothing in a plain tree or one that doesn't exist yet.
    #[test]
    fn a_link_anywhere_on_the_way_is_seen() {
        let root = tempfile::tempdir().unwrap();
        assert!(!crosses_a_symlink(root.path(), PROJECT_SKILL), "nothing exists yet");
        std::fs::create_dir_all(root.path().join(PROJECT_SKILL).parent().unwrap()).unwrap();
        assert!(!crosses_a_symlink(root.path(), PROJECT_SKILL), "plain directories");
        std::os::unix::fs::symlink("/nonexistent", root.path().join(PROJECT_SKILL)).unwrap();
        assert!(crosses_a_symlink(root.path(), PROJECT_SKILL), "the file is a link");

        let linked = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::create_dir(linked.path().join(".agents")).unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), linked.path().join(".agents/skills")).unwrap();
        assert!(crosses_a_symlink(linked.path(), PROJECT_SKILL), "a directory on the way is a link");
    }

    /// A newer Far Cooler replaces a copy an older one wrote and nobody edited.
    #[test]
    fn an_unedited_older_copy_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROJECT_SKILL);
        install_file(&path, &signed("# v1\n"));
        let v2 = signed("# v2\n");
        assert_eq!(install_file(&path, &v2), Installed::Wrote);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), v2);
    }

    /// Never clobber. An owner who edited our copy has taken it.
    #[test]
    fn an_edited_copy_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROJECT_SKILL);
        install_file(&path, &signed("# v1\n"));
        let written = std::fs::read_to_string(&path).unwrap();
        let edited = written.replacen("# v1\n", "# v1\nmy own rule\n", 1);
        std::fs::write(&path, &edited).unwrap();
        assert_eq!(install_file(&path, &signed("# v2\n")), Installed::LeftAlone("edited"));
        let now = std::fs::read_to_string(&path).unwrap();
        assert!(now.contains("my own rule"), "{now}");
        assert_eq!(now, edited);
    }

    /// A line typed after our marker is an edit too, not a copy of ours.
    #[test]
    fn a_line_after_the_marker_is_an_edit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROJECT_SKILL);
        install_file(&path, &signed("# v1\n"));
        let edited = format!("{}and one more\n", std::fs::read_to_string(&path).unwrap());
        std::fs::write(&path, &edited).unwrap();
        assert_eq!(install_file(&path, &signed("# v2\n")), Installed::LeftAlone("edited"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    }

    /// A file of the same name that we never wrote is somebody else's.
    #[test]
    fn a_file_without_our_marker_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PROJECT_SKILL);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "---\nname: farcooler-manager\n---\ntheirs\n").unwrap();
        assert_eq!(install_file(&path, &signed("# v1\n")), Installed::LeftAlone("not ours"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "---\nname: farcooler-manager\n---\ntheirs\n"
        );
    }

    /// Remove only ours: the edited copy and the foreign one both survive,
    /// and a sibling skill in the same `.agents/skills` is never touched.
    #[test]
    fn removing_takes_only_an_unedited_copy_of_ours() {
        let root = tempfile::tempdir().unwrap();
        let ours = root.path().join(PROJECT_SKILL);
        let ours_policy = root.path().join(PROJECT_SKILL_POLICY);
        install_file(&ours, &signed("# v1\n"));
        install_file(&ours_policy, &sign_yaml("policy:\n  allow_implicit_invocation: false\n"));
        let theirs = root.path().join(".agents/skills/someone-else/SKILL.md");
        std::fs::create_dir_all(theirs.parent().unwrap()).unwrap();
        std::fs::write(&theirs, "theirs\n").unwrap();

        let edited_root = tempfile::tempdir().unwrap();
        let edited = edited_root.path().join(PROJECT_SKILL);
        install_file(&edited, &signed("# v1\n"));
        let text = std::fs::read_to_string(&edited).unwrap().replacen("# v1", "# mine", 1);
        std::fs::write(&edited, &text).unwrap();

        assert!(remove_ours(&ours_policy));
        assert!(remove_ours(&ours));
        assert!(!remove_ours(&edited));
        assert!(!remove_ours(&theirs));

        assert!(!root.path().join(".agents/skills/farcooler-manager").exists());
        assert_eq!(std::fs::read_to_string(&theirs).unwrap(), "theirs\n");
        assert!(root.path().join(".agents/skills").is_dir(), "not ours to remove");
        assert_eq!(std::fs::read_to_string(&edited).unwrap(), text);
    }

    /// A marker whose hash covered the marker line itself could never match.
    /// Pins the hash to exactly "everything above the marker line".
    #[test]
    fn the_marker_hash_covers_the_body_and_not_itself() {
        let mut checked = 0;
        for h in [Harness::Codex] {
            for f in render(h, "farcooler") {
                checked += 1;
                let body_end = f.contents.trim_end_matches('\n').rfind('\n').unwrap() + 1;
                let (above, marker) = f.contents.split_at(body_end);
                let hex = format!("{:x}", Sha256::digest(above.as_bytes()));
                assert!(marker.contains(&format!("farcooler-managed sha256:{hex}")), "{h:?} {marker}");
                assert!(marker.ends_with('\n') && marker.matches('\n').count() == 1, "{marker:?}");
            }
        }
        // codex's two files, the only ones written into a worktree. A render
        // that wrote nothing would otherwise pass this by checking nothing.
        assert_eq!(checked, 2);
    }

    const ALL: [Harness; 3] = [Harness::Claude, Harness::Codex, Harness::Cursor];

    /// The `SKILL.md` a harness gets, with `cli` filled in.
    fn skill_body_with_cli(h: Harness, cli: &str) -> String {
        render(h, cli)
            .into_iter()
            .find(|f| f.relative.ends_with("SKILL.md"))
            .expect("every harness gets a SKILL.md")
            .contents
    }

    fn skill_body(h: Harness) -> String {
        skill_body_with_cli(h, "farcooler")
    }

    /// The two rules the spec says must be explicit, word for word, in every
    /// harness's copy. A harness-specific render that dropped one would still
    /// "have a skill".
    #[test]
    fn every_harness_states_both_rules() {
        for h in ALL {
            let body = skill_body(h);
            assert!(body.contains("**Never execute a task yourself.**"), "{h:?}");
            assert!(body.contains("**Writing it down is the work.**"), "{h:?}");
        }
    }

    /// The owner's ruling: customization lives in the charter, which wins over
    /// everything in the skill but the two rules. The skill text itself is
    /// regenerated on every launch, so an edit to it is not where taste goes.
    #[test]
    fn the_charter_overrides_everything_but_the_two_rules() {
        for h in ALL {
            let body = skill_body(h);
            assert!(
                body.contains("The charter overrides anything in this skill except the two rules above."),
                "{h:?}"
            );
        }
    }

    #[test]
    fn the_cli_path_is_filled_in_and_nothing_else_is_left_open() {
        let cli = "'/Applications/Far Cooler.app/x/farcooler-preview'";
        for h in ALL {
            let body = skill_body_with_cli(h, cli);
            assert!(body.contains(&format!("{cli} task list")), "{h:?}");
            assert!(!body.contains("{{"), "{h:?}: an unfilled placeholder reached an agent");
        }
    }

    /// Nothing wakes the manager yet, and nothing pushes a question to the
    /// owner's phone. The day something does, this test forces whoever flips
    /// `WAKE_LOOP_EXISTS` to rewrite `WAIT_STEP` too.
    #[test]
    fn the_skill_promises_no_wake_while_none_exists() {
        const { assert!(!WAKE_LOOP_EXISTS) };
        for h in ALL {
            let body = skill_body(h).to_lowercase();
            for promise in [
                "will be woken",
                "wake you when",
                "i'll check back",
                "i will check back",
                "notify you",
                "you'll be notified",
                "i'll let you know",
                "will notify",
            ] {
                assert!(!body.contains(promise), "{h:?} promises a wake: {promise}");
            }
            assert!(body.contains("nothing will wake you"), "{h:?}");
            assert!(body.contains(&WAIT_STEP.to_lowercase()), "{h:?}");
        }
    }

    /// A manager in an agent pane inherits `FARCOOLER_ACTOR=agent:<id>`, so a
    /// write without the flag would be filed as a dispatched agent's.
    #[test]
    fn every_write_the_skill_shows_names_the_manager() {
        let body = skill_body(Harness::Claude);
        let writes: Vec<&str> = body
            .lines()
            .filter(|l| {
                ["create", "set", "note", "ask", "block", "dispatch"]
                    .iter()
                    .any(|verb| l.contains(&format!("farcooler task {verb} ")))
            })
            .collect();
        assert!(writes.len() >= 4, "the skill shows too few writes to be checked: {writes:?}");
        for line in writes {
            assert!(line.contains("--actor manager"), "a write that doesn't name the manager: {line}");
        }
    }

    /// The part of the body under `## The interview`, up to the next `## `.
    fn interview(body: &str) -> &str {
        let start = body.find("\n## The interview\n").expect("the skill has an interview section");
        let rest = &body[start + 1..];
        let end = rest[3..].find("\n## ").map_or(rest.len(), |i| i + 3);
        &rest[..end]
    }

    /// The charter's headings are one list, and the interview asks for every
    /// one, so it can't silently stop asking about one of them.
    #[test]
    fn the_interview_covers_every_charter_section() {
        assert_eq!(CHARTER_SECTIONS.len(), 8);
        for h in ALL {
            let body = skill_body(h);
            let asks = interview(&body);
            for heading in CHARTER_SECTIONS {
                assert!(asks.contains(heading), "{h:?}: the interview never asks for {heading}");
            }
        }
    }

    /// The interview is how the charter gets written, and the owner approves
    /// it before it exists: the spec's "don't guess a workflow".
    #[test]
    fn the_charter_is_written_only_after_the_owner_approves_it() {
        let body = skill_body(Harness::Claude);
        let asks = interview(&body);
        assert!(asks.contains("One question at a time"), "{asks}");
        assert!(asks.contains("Read the whole draft back"), "{asks}");
        assert!(asks.contains("only after they say yes"), "{asks}");
    }

    /// Keeping the charter local goes in `.git/info/exclude`, which is not
    /// itself a tracked change the way `.gitignore` is.
    #[test]
    fn a_local_charter_goes_in_info_exclude() {
        let asks = skill_body(Harness::Claude);
        let asks = interview(&asks);
        assert!(asks.contains("info/exclude"), "{asks}");
        assert!(!asks.contains("to .gitignore"), "{asks}");
    }

    /// A manager unsure whether a dispatch took reads the board before
    /// dispatching again: a pressure run (S10) planned to "do it again", which
    /// would put a second agent on the task. And dispatching is one of the
    /// things it may write.
    #[test]
    fn the_skill_checks_the_board_before_dispatching_again() {
        for h in ALL {
            // Read as prose: where a line wraps is not what's being checked.
            let body = skill_body(h).split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(body.contains("before dispatching again"), "{h:?}");
            assert!(body.contains("the agent panes dispatch opens"), "{h:?}");
        }
    }

    /// With no charter, nothing goes on the board first. A pressure run (S7)
    /// had the manager create all three tasks "so it isn't lost" and only
    /// then start the interview. The owner's request survives in the reply,
    /// and reading the board is still allowed.
    #[test]
    fn nothing_goes_on_the_board_before_the_charter() {
        for h in ALL {
            let body = skill_body(h).split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(body.contains("no task, note or dispatch until the charter is written"), "{h:?}");
            assert!(body.contains("say it back in your reply"), "{h:?}");
            // Only writes wait. An S9 run read "before anything else" as "don't
            // read the board either" and wouldn't answer "What's on the board?".
            assert!(body.contains("interview the owner before you write anything"), "{h:?}");
            assert!(body.contains("Reading the board and answering what the owner asked is fine"), "{h:?}");
        }
    }

    /// The spec wants the skill read in a minute.
    #[test]
    fn the_skill_is_short() {
        let lines = skill_body(Harness::Claude).lines().count();
        assert!(lines <= 120, "{lines} lines");
    }

    /// Not a check: the pressure harness's way to get the skill exactly as an
    /// agent reads it, with `{{cli}}` pointing at its fake CLI.
    /// `scripts/manager-skill-pressure/render-skill.sh` runs it.
    #[test]
    #[ignore = "a tool for scripts/manager-skill-pressure, not a check"]
    fn render_for_the_pressure_harness() {
        let cli = std::env::var("FAKE_CLI").expect("FAKE_CLI names the fake farcooler");
        let out = std::env::var("SKILL_OUT").expect("SKILL_OUT names the file to write");
        std::fs::write(out, skill_body_with_cli(Harness::Claude, &crate::service::shell_quote(&cli)))
            .unwrap();
    }

    #[test]
    fn each_harness_gets_its_own_files() {
        let names = |h| render(h, "farcooler").into_iter().map(|f| f.relative).collect::<Vec<_>>();
        let plugin = vec![".claude-plugin/plugin.json", ".cursor-plugin/plugin.json", "skills/manager/SKILL.md"];
        assert_eq!(names(Harness::Claude), plugin);
        assert_eq!(names(Harness::Codex), vec![PROJECT_SKILL, PROJECT_SKILL_POLICY]);
        // cursor takes the same plugin directory as claude, through its own
        // manifest (measured: one directory holding both loads in both).
        assert_eq!(render(Harness::Cursor, "farcooler"), render(Harness::Claude, "farcooler"));

        let claude = render(Harness::Claude, "farcooler");
        for manifest in &claude[..2] {
            let v: serde_json::Value = serde_json::from_str(&manifest.contents).unwrap();
            assert_eq!(v, serde_json::json!({"name": "farcooler"}), "{}", manifest.relative);
        }
        assert!(claude[2].contents.starts_with("---\nname: manager\n"), "{}", claude[2].contents);
        assert!(claude[2].contents.contains("\ndisable-model-invocation: true\n"));

        let codex = render(Harness::Codex, "farcooler");
        assert!(codex[0].contents.starts_with("---\nname: farcooler-manager\n"));
        // cursor also reads `.agents/skills`, so codex's copy keeps the key
        // that stops cursor's model reaching for it on its own.
        assert!(codex[0].contents.contains("\ndisable-model-invocation: true\n"));
        assert!(codex[1].contents.contains("allow_implicit_invocation: false"));
    }

    /// Measured: cursor-agent handed `--plugin-dir` in a worktree that also
    /// holds codex's `.agents/skills/farcooler-manager/` lists both skills
    /// when their names differ, and when they're the same it lists one and
    /// runs the WORKTREE copy. So a plugin skill named like the worktree one
    /// would be silently replaced by whatever codex last wrote there (or the
    /// owner edited), and cursor would lose "regenerated on every launch"
    /// exactly where codex has run. The names are kept apart on purpose.
    #[test]
    fn the_worktree_copy_never_shadows_the_plugin_skill() {
        let name = |text: &str| {
            text.lines().find_map(|l| l.strip_prefix("name: ")).map(str::to_string).expect("a name")
        };
        let plugin = render(Harness::Cursor, "farcooler")
            .into_iter()
            .find(|f| f.relative.ends_with("SKILL.md"))
            .unwrap();
        let project = render(Harness::Codex, "farcooler").into_iter().find(|f| f.relative == PROJECT_SKILL).unwrap();
        assert_ne!(name(&plugin.contents), name(&project.contents));
    }
}
