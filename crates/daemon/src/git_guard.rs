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
//! - A filter the repository's config touches at all gets `clean`, `smudge`
//!   and `process` set to what the user's global or system config says, or
//!   to nothing. Not every filter: git-lfs keeps its filter in the global
//!   config, and blanking it would show every LFS file as changed.
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
//! became a guarded status and a forced remove). Closing the race needs a
//! guard that does not depend on names: an exec allowlist around git (see
//! the ov-129 report). And none of this guards against an agent that is not
//! sandboxed: one that can write `~/.zshrc` doesn't need git.
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
}

/// The by-name pins for what `git config` printed for [`LISTING`]: every
/// config hook off, and every filter the repository touched reset to the
/// user's own.
///
/// The listing is `scope NUL key LF value NUL` per entry, or `scope NUL key
/// NUL` for a key written with no value at all.
pub fn pins_from(listing: &[u8]) -> Vec<Pin> {
    let mut fields = listing.split(|b| *b == 0);
    let mut hooks: Vec<&[u8]> = Vec::new();
    let mut touched: Vec<&[u8]> = Vec::new();
    // What the user's own config says, last one winning, as git reads it.
    let mut trusted: Vec<(&[u8], &[u8], &[u8])> = Vec::new();
    while let (Some(scope), Some(entry)) = (fields.next(), fields.next()) {
        let (key, value) = match entry.iter().position(|b| *b == b'\n') {
            Some(i) => (&entry[..i], &entry[i + 1..]),
            None => (entry, &b"true"[..]),
        };
        // `section.<name>.var`: the name is everything between the first
        // dot and the last, dots and all.
        let Some(first) = key.iter().position(|b| *b == b'.') else { continue };
        let Some(last) = key.iter().rposition(|b| *b == b'.') else { continue };
        if last <= first {
            continue;
        }
        let (section, name, var) = (&key[..first], &key[first + 1..last], &key[last + 1..]);
        match section {
            b"hook" => {
                if !hooks.contains(&name) {
                    hooks.push(name);
                }
            }
            b"filter" => {
                if matches!(scope, b"system" | b"global") {
                    trusted.push((name, var, value));
                } else if !touched.contains(&name) {
                    touched.push(name);
                }
            }
            _ => {}
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
    for name in touched {
        for (var, otherwise) in [("clean", ""), ("smudge", ""), ("process", ""), ("required", "false")] {
            let users = trusted.iter().rev().find(|(n, v, _)| *n == name && v == &var.as_bytes());
            let value = users.map_or(OsStr::new(otherwise), |(_, _, value)| OsStr::from_bytes(value));
            pins.push((key("filter", name, var), value.to_os_string()));
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
    fn a_filter_only_the_user_configured_is_left_alone() {
        let listing = b"global\0filter.lfs.clean\ngit-lfs clean -- %f\0\
                        global\0filter.lfs.process\ngit-lfs filter-process\0\
                        global\0filter.lfs.required\0";
        assert_eq!(pins_from(listing), Vec::<Pin>::new());
    }

    #[test]
    fn a_filter_the_repository_touches_goes_back_to_the_users_or_to_nothing() {
        let listing = b"global\0filter.lfs.clean\ngit-lfs clean -- %f\0\
                        system\0filter.lfs.smudge\nold\0\
                        global\0filter.lfs.smudge\ngit-lfs smudge -- %f\0\
                        local\0filter.lfs.process\n./evil\0\
                        worktree\0filter.x.clean\n./evil\0";
        assert_eq!(
            pins_from(listing),
            [
                pin("filter.lfs.clean", "git-lfs clean -- %f"),
                pin("filter.lfs.smudge", "git-lfs smudge -- %f"),
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
        for (key, _) in std::env::vars_os() {
            let key = key.to_string_lossy().into_owned();
            if key.starts_with("GIT_") && !key.starts_with("GIT_CONFIG_") {
                assert!(envs.iter().any(|(k, v)| *k == key && v.is_none()), "{key} is removed");
            }
        }
    }
}
