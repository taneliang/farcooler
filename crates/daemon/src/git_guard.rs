//! What every git the daemon starts is told, so that a repository's own
//! config cannot make it run a program.
//!
//! An agent can write anything in its worktree, and the worktree's `.git` is
//! a file it can rewrite to point at a repository of its own making, config
//! and all. The daemon runs git there as the user — status for review, diff,
//! log, `worktree add` and `remove` — and git runs programs its config names.
//! Measured against git 2.54, these run under the daemon's own calls:
//!
//! - `core.fsmonitor`: on any index refresh, so `status`, `diff`, `ls-files`.
//! - hooks, from `core.hooksPath` or the hooks directory: `post-checkout` on
//!   `worktree add`, `reference-transaction` on any ref change,
//!   `post-index-change` on a status that rewrites the index.
//! - hooks named in config (`hook.<name>.command`, git 2.54), which
//!   `core.hooksPath` does not reach.
//! - `filter.<name>.clean`/`process` when status or diff has to read a file,
//!   `smudge` when `worktree add` writes one.
//! - `diff.<name>.textconv`, `diff.<name>.command` and `diff.external` on a
//!   patch from `diff`, and textconv on `log -p`/`show`.
//! - `gpg.program` from `git log`, once `log.showSignature` is on and a
//!   commit carries a signature header.
//!
//! **Pinned on every call**, as config from the environment
//! (`GIT_CONFIG_COUNT`), which outranks every config file:
//!
//! - `core.fsmonitor=false`.
//! - `core.hooksPath=/dev/null`: every hook git looks up is then
//!   `/dev/null/<name>`, which is never there.
//! - `log.showSignature=false`: the daemon's `log` formats don't ask for a
//!   signature, so verifying one only ever ran a program.
//! - `core.pager=cat`: git pages only to a terminal and the daemon never
//!   gives it one, so this is for the call that some day does.
//! - `protocol.allow=never`, with `GIT_NO_LAZY_FETCH=1`: no daemon call
//!   reaches a remote, but a partial clone fetches missing objects on demand,
//!   from a remote the config names, through `core.sshCommand`, a credential
//!   helper, `remote.<name>.uploadpack` or `protocol.ext.allow` — all of them
//!   programs. Refusing every transport closes that whole family at once. The
//!   cost: in a partial clone, a diff or checkout that needs an object it
//!   doesn't have fails rather than fetching it.
//!
//! **Pinned by name**, after one `git config --get-regexp` in the same place
//! ([`pins`]): config hooks and filters are keyed by a name the config itself
//! chooses, so there is no single key to turn them off.
//!
//! - Every config hook gets `hook.<name>.enabled=false`. A hook from the
//!   user's own config is turned off too, like the hooks directory is.
//! - Every filter, whoever configured it, gets `clean`, `smudge` and
//!   `process` emptied and `required=false`. That includes the user's own
//!   git-lfs. git starts a filter with arguments (`git-lfs filter-process`)
//!   through `sh -c`, and the exec allowlist (`crate::git_sandbox`) never
//!   allows a shell, so not even a git-lfs at a trusted absolute path could
//!   run; left configured, a `required` filter would fail the call instead.
//!   Emptied, git reads the file as it is. What that changes: a worktree the
//!   daemon creates holds LFS pointer files, not their content (the agent
//!   runs `git lfs pull` itself), and an LFS file whose content was fetched
//!   shows as changed once its mtime moves. It also closes git-lfs's own
//!   reach: custom transfer agents named in the repository's config.
//!
//! **Passed on a diff** (`args`): `--no-ext-diff --no-textconv`. These are
//! also what keeps the patch parseable at all; a user whose global config
//! sets `diff.external` to difftastic used to get its output in the review.
//!
//! **Submodules are not entered.** A submodule is a repository with a config
//! of its own that the listing never reads, and git starts a child git in it:
//! status, to see whether its files changed (where its filters and hooks
//! ran), and diff under `diff.submodule=diff`, a child that does not inherit
//! `--no-ext-diff` (where its `diff.external` ran). So status and every diff
//! get `--ignore-submodules=dirty`, every diff `--submodule=short`, and the
//! same four as pins for the gits started by git (`submodule.recurse=false`,
//! `diff.submodule=short`, `diff.ignoreSubmodules=dirty`,
//! `status.submoduleSummary=false`). `dirty` rather than `all`: a submodule
//! moved to another commit still shows, which git learns from the gitlink
//! and the submodule's `HEAD` without starting anything. What no longer
//! shows is uncommitted work inside a submodule.
//!
//! **What this leaves.** The by-name pins are read a moment before the call
//! they protect, so an agent rewriting its config in a loop can land a new
//! hook or filter name in that gap; measured, about one guarded status in
//! five. And the listing is the config of ITS context: a git that starts a
//! child git in another worktree reads that worktree's `includeIf`s. The
//! daemon's two such calls are taken apart instead (`worktree add` checks out
//! in a second git in the new worktree; the non-forced `worktree remove`
//! became a guarded status and a forced remove). Both are closed by the exec
//! allowlist every one of these gits runs in (`crate::git_sandbox`), which
//! doesn't read config; the pins stay so that it never has to refuse
//! anything in an ordinary repository, and so that a host without it keeps
//! them. And none of this guards against an agent that is not sandboxed: one
//! that can write `~/.zshrc` doesn't need git.
//!
//! **Also pinned**: `PATH`, with every entry that isn't absolute dropped
//! ([`child_path`]), and the work tree, to the directory the daemon asked
//! about ([`pin_work_tree`]).
//!
//! Every inherited `GIT_` variable is removed first, so a daemon started
//! from inside a git hook (`GIT_DIR`, `GIT_INDEX_FILE`) or by a shell with
//! `GIT_EXTERNAL_DIFF` set doesn't carry either into a worktree.
//!
//! Not set, on purpose: `GIT_CONFIG_NOSYSTEM`, `GIT_ATTR_NOSYSTEM` and
//! `core.attributesFile=/dev/null`. The system files are root's to write, and
//! neither the global attributes file nor any attributes file can run a
//! program without a driver from config, which is pinned above. Turning them
//! off would only change answers for repositories nobody planted anything in.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;

/// The keys pinned on every call, whatever any config file says.
pub const FIXED: &[(&str, &str)] = &[
    ("core.fsmonitor", "false"),
    ("core.hooksPath", "/dev/null"),
    ("log.showSignature", "false"),
    ("core.pager", "cat"),
    ("protocol.allow", "never"),
    ("submodule.recurse", "false"),
    ("diff.submodule", "short"),
    ("diff.ignoreSubmodules", "dirty"),
    ("status.submoduleSummary", "false"),
];

/// Subcommands that can print a patch, and so run a textconv or an
/// external diff.
const DIFFS: &[&str] =
    &["diff", "log", "show", "diff-tree", "diff-index", "diff-files", "whatchanged", "format-patch"];

/// What goes after a diff subcommand: no external diff, no textconv, and no
/// git started inside a submodule.
const NO_PROGRAMS: [&str; 4] =
    ["--no-ext-diff", "--no-textconv", "--ignore-submodules=dirty", "--submodule=short"];

/// What goes after `status`: no git started inside a submodule.
const STATUS: [&str; 1] = ["--ignore-submodules=dirty"];

/// The arguments for `git config` that list what [`pins_from`] reads.
pub const LISTING: &[&str] =
    &["config", "--null", "--show-scope", "--get-regexp", r"^(filter|hook)\."];

/// One config key and the value it is pinned to. Bytes, because a hook or
/// filter name is whatever the config spelled, and a name mangled on the way
/// through would pin a different key.
pub type Pin = (OsString, OsString);

/// `args`, with [`NO_PROGRAMS`] after the subcommand when it is one that can
/// print a patch, and [`STATUS`] after `status`.
///
/// Right after the subcommand rather than at the end, so they land before
/// any `--` and are read as options.
pub fn args<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut out = Vec::with_capacity(args.len() + NO_PROGRAMS.len());
    match args.split_first() {
        Some((sub, rest)) if DIFFS.contains(sub) => {
            out.push(*sub);
            out.extend(NO_PROGRAMS);
            out.extend_from_slice(rest);
        }
        Some((sub, rest)) if *sub == "status" => {
            out.push(*sub);
            out.extend(STATUS);
            out.extend_from_slice(rest);
        }
        _ => out.extend_from_slice(args),
    }
    out
}

/// [`FIXED`] as pins.
pub fn fixed() -> Vec<Pin> {
    FIXED.iter().map(|(k, v)| (OsString::from(k), OsString::from(v))).collect()
}

/// Strip every inherited `GIT_` variable from `cmd`, then give it `pins` as
/// config and the two switches that keep git from asking or fetching.
///
/// For git itself and for a program that runs git (`gh`): the config travels
/// in the environment, so git's children, and git run by someone else, get
/// it too.
pub fn apply(cmd: &mut std::process::Command, pins: &[Pin]) {
    for (key, _) in std::env::vars_os() {
        if key.as_bytes().starts_with(b"GIT_") {
            cmd.env_remove(key);
        }
    }
    cmd.env("GIT_CONFIG_COUNT", pins.len().to_string());
    for (i, (key, value)) in pins.iter().enumerate() {
        cmd.env(format!("GIT_CONFIG_KEY_{i}"), key);
        cmd.env(format!("GIT_CONFIG_VALUE_{i}"), value);
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_NO_LAZY_FETCH", "1");
    if let Some(path) = child_path() {
        cmd.env("PATH", path);
    }
}

/// The `PATH` git and gh are handed: the daemon's, with every entry that
/// isn't an absolute directory dropped (`crate::git_sandbox::absolute_path_entries`).
///
/// git is started by absolute path, but it hands its `PATH` on, and a filter
/// or a gh looks names up on it from inside the worktree: with `.` on it, a
/// user's own trusted `filter.lfs.process = git-lfs filter-process` ran a
/// `git-lfs` the agent left in the worktree root (ov-129 review 2). `None`
/// when the daemon has no `PATH` at all, which leaves the child the system's
/// default.
pub fn child_path() -> Option<OsString> {
    std::env::var_os("PATH").map(|path| crate::git_sandbox::absolute_path_entries(&path))
}

/// Tell git its work tree is `cwd` whenever `cwd` holds the `.git` git would
/// find first, which is what git concludes on its own unless a config says
/// otherwise.
///
/// A config can: `core.worktree` in a repository the worktree's `.git` file
/// points at, or in `worktrees/<id>/config.worktree`, which leaves the `.git`
/// file untouched. Either silently points status and diff at another
/// directory, so review shows that directory's changes as the worktree's.
/// `GIT_WORK_TREE` outranks the config. Not set when `cwd` has no `.git`
/// (a subdirectory, where git searches upward and the top is elsewhere).
/// Child gits git starts in another repository (a submodule) don't inherit it;
/// git clears it for them.
pub fn pin_work_tree(cmd: &mut std::process::Command, cwd: &std::path::Path) {
    if std::fs::symlink_metadata(cwd.join(".git")).is_ok() {
        cmd.env("GIT_WORK_TREE", cwd);
    }
}

/// The by-name pins for what `git config` printed for [`LISTING`]: every
/// config hook off, and every filter, whoever configured it, emptied.
///
/// The listing is `scope NUL key LF value NUL` per entry, or `scope NUL key
/// NUL` for a key written with no value at all.
pub fn pins_from(listing: &[u8]) -> Vec<Pin> {
    let mut fields = listing.split(|b| *b == 0);
    let mut hooks: Vec<&[u8]> = Vec::new();
    let mut filters: Vec<&[u8]> = Vec::new();
    while let (Some(_scope), Some(entry)) = (fields.next(), fields.next()) {
        let key = match entry.iter().position(|b| *b == b'\n') {
            Some(i) => &entry[..i],
            None => entry,
        };
        // `section.<name>.var`: the name is everything between the first
        // dot and the last, dots and all.
        let Some(first) = key.iter().position(|b| *b == b'.') else { continue };
        let Some(last) = key.iter().rposition(|b| *b == b'.') else { continue };
        if last <= first {
            continue;
        }
        let (section, name) = (&key[..first], &key[first + 1..last]);
        let seen = match section {
            b"hook" => &mut hooks,
            b"filter" => &mut filters,
            _ => continue,
        };
        if !seen.contains(&name) {
            seen.push(name);
        }
    }

    let key = |section: &str, name: &[u8], var: &str| {
        let mut k = Vec::with_capacity(section.len() + name.len() + var.len() + 2);
        k.extend_from_slice(section.as_bytes());
        k.push(b'.');
        k.extend_from_slice(name);
        k.push(b'.');
        k.extend_from_slice(var.as_bytes());
        OsStr::from_bytes(&k).to_os_string()
    };
    let mut pins: Vec<Pin> = Vec::new();
    for name in hooks {
        pins.push((key("hook", name, "enabled"), OsString::from("false")));
    }
    for name in filters {
        for (var, value) in [("clean", ""), ("smudge", ""), ("process", ""), ("required", "false")] {
            pins.push((key("filter", name, var), OsString::from(value)));
        }
    }
    pins
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(k: &str, v: &str) -> Pin {
        (OsString::from(k), OsString::from(v))
    }

    #[test]
    fn a_diff_or_status_gets_its_flags_before_anything_else() {
        let diff = ["--no-ext-diff", "--no-textconv", "--ignore-submodules=dirty", "--submodule=short"];
        assert_eq!(args(&["diff", "--numstat", "--", "a"]), [&["diff"][..], &diff, &["--numstat", "--", "a"]].concat());
        assert_eq!(args(&["log", "-1"]), [&["log"][..], &diff, &["-1"]].concat());
        assert_eq!(args(&["status", "-z"]), ["status", "--ignore-submodules=dirty", "-z"]);
        assert_eq!(args(&["rev-parse", "HEAD"]), ["rev-parse", "HEAD"]);
        // A path named like a subcommand is not one.
        assert_eq!(args(&["ls-files", "--", "diff"]), ["ls-files", "--", "diff"]);
        assert_eq!(args(&[]), Vec::<&str>::new());
    }

    #[test]
    fn every_config_hook_is_turned_off_whoever_wrote_it() {
        let listing = b"global\0hook.mine.command\nlint\0local\0hook.a=b.c.event\npost-checkout\0\
                        local\0hook.mine.event\npre-commit\0";
        assert_eq!(
            pins_from(listing),
            [pin("hook.mine.enabled", "false"), pin("hook.a=b.c.enabled", "false")]
        );
    }

    #[test]
    fn every_filter_is_emptied_whoever_wrote_it_lfs_included() {
        let listing = b"global\0filter.lfs.clean\ngit-lfs clean -- %f\0\
                        global\0filter.lfs.process\ngit-lfs filter-process\0\
                        global\0filter.lfs.required\0\
                        local\0filter.lfs.process\n./evil\0\
                        worktree\0filter.x.clean\n./evil\0";
        assert_eq!(
            pins_from(listing),
            [
                pin("filter.lfs.clean", ""),
                pin("filter.lfs.smudge", ""),
                pin("filter.lfs.process", ""),
                pin("filter.lfs.required", "false"),
                pin("filter.x.clean", ""),
                pin("filter.x.smudge", ""),
                pin("filter.x.process", ""),
                pin("filter.x.required", "false"),
            ]
        );
    }

    #[test]
    fn a_name_that_is_not_utf8_is_pinned_as_written() {
        let listing = b"local\0hook.\xff.command\nx\0";
        let pins = pins_from(listing);
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].0.as_bytes(), b"hook.\xff.enabled");
    }

    #[test]
    fn apply_strips_inherited_git_variables_and_sets_the_pins() {
        let mut cmd = std::process::Command::new("git");
        // What a daemon started from inside a hook would carry. Set on the
        // command rather than the process: `apply` must remove what the
        // process has, and this checks the shape of what it leaves.
        apply(&mut cmd, &[pin("core.fsmonitor", "false"), pin("hook.x.enabled", "false")]);
        let envs: Vec<(String, Option<String>)> = cmd
            .get_envs()
            .map(|(k, v)| (k.to_string_lossy().into_owned(), v.map(|v| v.to_string_lossy().into_owned())))
            .collect();
        let get = |k: &str| envs.iter().find(|(key, _)| key == k).and_then(|(_, v)| v.clone());
        assert_eq!(get("GIT_CONFIG_COUNT").as_deref(), Some("2"));
        assert_eq!(get("GIT_CONFIG_KEY_1").as_deref(), Some("hook.x.enabled"));
        assert_eq!(get("GIT_CONFIG_VALUE_1").as_deref(), Some("false"));
        assert_eq!(get("GIT_NO_LAZY_FETCH").as_deref(), Some("1"));
        // The daemon's own PATH, less what isn't absolute.
        let expected = std::env::var_os("PATH")
            .map(|p| crate::git_sandbox::absolute_path_entries(&p).to_string_lossy().into_owned());
        assert_eq!(get("PATH"), expected);
        for (key, _) in std::env::vars_os() {
            let key = key.to_string_lossy().into_owned();
            if key.starts_with("GIT_") && !key.starts_with("GIT_CONFIG_") {
                assert!(envs.iter().any(|(k, v)| *k == key && v.is_none()), "{key} is removed");
            }
        }
    }

    #[test]
    fn the_work_tree_is_pinned_only_where_git_would_find_it_first() {
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path().to_path_buf();
        std::fs::write(top.join(".git"), "gitdir: elsewhere\n").unwrap();
        std::fs::create_dir(top.join("sub")).unwrap();
        let work_tree = |cwd: &std::path::Path| {
            let mut cmd = std::process::Command::new("git");
            pin_work_tree(&mut cmd, cwd);
            cmd.get_envs().find(|(k, _)| *k == "GIT_WORK_TREE").and_then(|(_, v)| v.map(OsStr::to_os_string))
        };
        assert_eq!(work_tree(&top), Some(top.clone().into_os_string()));
        assert_eq!(work_tree(&top.join("sub")), None);
    }
}
