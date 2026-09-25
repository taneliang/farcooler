//! Installing the manager skill without owning the directory it lives in.
//!
//! The manager is a skill every agent Far Cooler launches can be asked to
//! become. How each agent finds it was measured on this runner, not read off a
//! doc (claude 2.1.282, codex-cli 0.153.4, cursor-agent 2026.09.23):
//!
//! - **claude** loads a plugin directory named by `--plugin-dir`, for that
//!   session only. Far Cooler writes the plugin into its own runtime directory
//!   (`CLAUDE_PLUGIN_DIR`), the way it writes `claude-hooks.json` for
//!   `--settings`, so no file of the user's and none in the repository is
//!   touched. It is invoked as `/farcooler:manager`.
//! - **codex** reads `.agents/skills/<name>/SKILL.md` in the directory it was
//!   opened in, and nothing per session: `-c skills.config=[{path=…}]` from
//!   outside a repository did not add a skill, whether `path` named the file
//!   or its directory. It is invoked as `$farcooler-manager`, and
//!   `agents/openai.yaml` keeps the model from reaching for it on its own.
//! - **cursor-agent** reads the same `.agents/skills/` and is invoked as
//!   `/farcooler-manager`. It also accepts `--plugin-dir`, measured, but the
//!   owner's install rule for cursor is the project-local one, so that flag is
//!   not used.
//!
//! codex and cursor share one `SKILL.md`, byte for byte. codex was measured
//! loading a file that carries cursor's `disable-model-invocation: true`, so
//! one file serves both, and neither harness's install can read the other's
//! copy as an edit of its own.
//!
//! **Ours is recorded in the file**, because a skill is a whole file rather
//! than an entry in a shared list the way a hook is. The last line of every
//! file written into a worktree is a marker carrying the sha256 of everything
//! above it. A file whose marker still matches is one we wrote and nobody has
//! edited, and it is the only kind this module ever replaces or removes.
//! Anything else — edited, or never ours — is left exactly as it is.

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

/// Where claude's plugin lives, relative to this daemon's runtime directory.
pub const CLAUDE_PLUGIN_DIR: &str = "claude-plugin";

/// The skill codex and cursor both read, relative to a worktree's root.
pub const PROJECT_SKILL: &str = ".agents/skills/farcooler-manager/SKILL.md";

/// codex's invocation policy for it. cursor never reads this file.
pub const PROJECT_SKILL_POLICY: &str = ".agents/skills/farcooler-manager/agents/openai.yaml";

/// Every file Far Cooler writes into a worktree for the skill.
///
/// `hook_install::project_hook_exclusions` reads this beside
/// `PROJECT_HOOK_FILES`, so `git::is_dirty` and `change_set::working_tree`
/// subtract exactly what the installer writes and nothing else.
pub const PROJECT_SKILL_FILES: &[&str] = &[PROJECT_SKILL, PROJECT_SKILL_POLICY];

/// The skill's text, with `{{frontmatter}}` and `{{cli}}` still to fill.
///
/// A stub until the real body lands; the rendering around it is what this
/// module's tests pin.
const BODY: &str = "{{frontmatter}}\n# Managing this repository's board\n\nEvery command is `{{cli}}`.\n";

/// The frontmatter every copy carries, differing only in `name`: claude
/// namespaces a plugin's skill itself (`/farcooler:manager`), and codex and
/// cursor read `.agents/skills`, which is shared with whatever other skills a
/// repository has, so that copy says whose it is in its name.
///
/// `disable-model-invocation: true` for claude and cursor, which both read it:
/// measured, a model with this skill installed answered that it had no such
/// skill available. codex ignores the key (measured: it still loads the file)
/// and reads `agents/openai.yaml` instead.
fn frontmatter(name: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: Manage this repository's Far Cooler task board. Reads the \
         charter, reads the board, then creates, revises, answers or reports. Never does the work \
         itself. Use only when the owner asks for the manager.\ndisable-model-invocation: true\n---\n"
    )
}

/// codex's switch for the same thing, measured the same way.
const CODEX_POLICY: &str = "policy:\n  allow_implicit_invocation: false\n";

/// Every copy of the skill for `harness`, with `cli` standing in for every
/// `{{cli}}` in the body. `cli` is expected to be shell-quoted already.
///
/// The worktree copies are signed (`sign_markdown`, `sign_yaml`), because a
/// worktree is somebody else's directory and the signature is how a later
/// install or `remove_ours` tells our unedited file from theirs. claude's copy
/// goes to Far Cooler's own runtime directory and is not signed: nobody else
/// writes there, and it is rewritten whenever it differs.
pub fn render(harness: Harness, cli: &str) -> Vec<SkillFile> {
    match harness {
        Harness::Claude => vec![
            SkillFile {
                relative: ".claude-plugin/plugin.json",
                contents: "{\"name\":\"farcooler\"}\n".to_string(),
            },
            SkillFile { relative: "skills/manager/SKILL.md", contents: body("manager", cli) },
        ],
        Harness::Codex => vec![
            SkillFile { relative: PROJECT_SKILL, contents: sign_markdown(&body("farcooler-manager", cli)) },
            SkillFile { relative: PROJECT_SKILL_POLICY, contents: sign_yaml(CODEX_POLICY) },
        ],
        Harness::Cursor => vec![SkillFile {
            relative: PROJECT_SKILL,
            contents: sign_markdown(&body("farcooler-manager", cli)),
        }],
    }
}

/// The body with its frontmatter and CLI filled in.
fn body(name: &str, cli: &str) -> String {
    BODY.replace("{{frontmatter}}", &frontmatter(name)).replace("{{cli}}", cli)
}

/// The word every marker carries.
const MARKER: &str = "farcooler-managed sha256:";

fn digest(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// `body`, then a last line naming the sha256 of `body`, as an HTML comment
/// a Markdown reader doesn't render.
fn sign_markdown(body: &str) -> String {
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
/// a harness that reads the skill mid-write never sees half of one.
pub fn install_file(path: &Path, contents: &str) -> Installed {
    match std::fs::read(path) {
        Ok(existing) => {
            if existing == contents.as_bytes() {
                return Installed::Unchanged;
            }
            match ownership(&String::from_utf8_lossy(&existing)) {
                Ownership::OursUnedited => {}
                Ownership::Edited => return Installed::LeftAlone("edited"),
                Ownership::NotOurs => return Installed::LeftAlone("not ours"),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Installed::LeftAlone("unreadable"),
    }
    match write_atomically(path, contents) {
        Ok(()) => Installed::Wrote,
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "could not write the manager skill");
            Installed::Failed
        }
    }
}

/// Write through a sibling temporary file and a rename.
pub(crate) fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let dir = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().ok_or(std::io::ErrorKind::InvalidInput)?.to_string_lossy();
    let temp = dir.join(format!(".{name}.farcooler-{}.tmp", std::process::id()));
    std::fs::write(&temp, contents)?;
    std::fs::rename(&temp, path).inspect_err(|_| {
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
        for h in [Harness::Codex, Harness::Cursor] {
            for f in render(h, "farcooler") {
                checked += 1;
                let body_end = f.contents.trim_end_matches('\n').rfind('\n').unwrap() + 1;
                let (above, marker) = f.contents.split_at(body_end);
                let hex = format!("{:x}", Sha256::digest(above.as_bytes()));
                assert!(marker.contains(&format!("farcooler-managed sha256:{hex}")), "{h:?} {marker}");
                assert!(marker.ends_with('\n') && marker.matches('\n').count() == 1, "{marker:?}");
            }
        }
        // codex's two files and cursor's one. A render that wrote nothing
        // would otherwise pass this by checking nothing.
        assert_eq!(checked, 3);
    }

    #[test]
    fn each_harness_gets_its_own_files() {
        let names = |h| render(h, "farcooler").into_iter().map(|f| f.relative).collect::<Vec<_>>();
        assert_eq!(names(Harness::Claude), vec![".claude-plugin/plugin.json", "skills/manager/SKILL.md"]);
        assert_eq!(names(Harness::Codex), vec![PROJECT_SKILL, PROJECT_SKILL_POLICY]);
        assert_eq!(names(Harness::Cursor), vec![PROJECT_SKILL]);

        let claude = render(Harness::Claude, "farcooler");
        let manifest: serde_json::Value = serde_json::from_str(&claude[0].contents).unwrap();
        assert_eq!(manifest, serde_json::json!({"name": "farcooler"}));
        assert!(claude[1].contents.starts_with("---\nname: manager\n"), "{}", claude[1].contents);
        assert!(claude[1].contents.contains("\ndisable-model-invocation: true\n"));

        let codex = render(Harness::Codex, "farcooler");
        assert!(codex[1].contents.contains("allow_implicit_invocation: false"));
        let cursor = render(Harness::Cursor, "farcooler");
        assert!(cursor[0].contents.starts_with("---\nname: farcooler-manager\n"));
        assert!(cursor[0].contents.contains("\ndisable-model-invocation: true\n"));
        // One file serves both: whichever launches first writes it, and the
        // other must not read it as an edit.
        assert_eq!(codex[0].contents, cursor[0].contents);
    }
}
