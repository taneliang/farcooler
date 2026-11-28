//! One `git worktree add` at a time in this daemon, and never beside a git that reads every worktree (ov-1526).
//!
//! `worktree add` writes a new worktree's `.git/worktrees/<id>/commondir` a moment after its `gitdir`, and between
//! the two the file is there but empty. A git that walks every worktree's refs meanwhile (another `worktree add -b`,
//! checking that its new branch isn't checked out elsewhere; `worktree list`; `branch -D`) reads that empty file and
//! dies with `fatal: failed to read .git/worktrees/<id>/commondir: Undefined error: 0`. A `worktree add -b` that dies
//! there has already made its branch, so a run's Provision was left uncertain: four provisions in one sweep on macOS
//! CI lost one about once in fifty.
//!
//! So an add holds this lock exclusively, and those readers share it. Other gits don't take it: one that reads every
//! worktree isn't known to, and holding it across a long fetch or a slow `worktree remove` would hold provisions back.
//! It orders only this process's gits; a person's own git can still race an add.

use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

static METADATA: RwLock<()> = RwLock::const_new(());

/// What a git with `args` must hold while it runs: nothing, a shared hold, or the exclusive one.
pub(super) enum Hold {
    Shared { _guard: RwLockReadGuard<'static, ()> },
    Exclusive { _guard: RwLockWriteGuard<'static, ()> },
}

/// Take what a git with `args` needs; `None` for one that neither writes nor walks worktree metadata.
pub(super) async fn hold(args: &[&str]) -> Option<Hold> {
    hold_on(&METADATA, args).await
}

async fn hold_on(lock: &'static RwLock<()>, args: &[&str]) -> Option<Hold> {
    match access(args)? {
        Access::Writes => Some(Hold::Exclusive { _guard: lock.write().await }),
        Access::Walks => Some(Hold::Shared { _guard: lock.read().await }),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Access {
    /// Makes a worktree's metadata: `worktree add`.
    Writes,
    /// Reads every worktree's refs, so a half-made one kills it.
    Walks,
}

/// How a git with `args` touches worktree metadata, read past git's own options (`-c k=v`, `-C dir`).
pub(super) fn access(args: &[&str]) -> Option<Access> {
    let mut rest = args.iter().copied();
    let sub = loop {
        match rest.next()? {
            "-c" | "-C" => { rest.next()?; }
            option if option.starts_with('-') => {}
            sub => break sub,
        }
    };
    match (sub, rest.find(|a| !a.starts_with('-'))) {
        ("worktree", Some("add")) => Some(Access::Writes),
        ("worktree", Some("list")) | ("branch", _) => Some(Access::Walks),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_add_writes_and_a_list_or_a_branch_walks() {
        assert_eq!(access(&["worktree", "add", "--no-checkout", "-b", "b", "/t", "c"]), Some(Access::Writes));
        assert_eq!(access(&["-c", "user.name=T", "worktree", "add", "--detach", "/t"]), Some(Access::Writes));
        assert_eq!(access(&["worktree", "list", "--porcelain"]), Some(Access::Walks));
        assert_eq!(access(&["branch", "-D", "wf-x"]), Some(Access::Walks));
        assert_eq!(access(&["worktree", "remove", "--force", "/t"]), None);
        assert_eq!(access(&["rev-parse", "HEAD"]), None);
        assert_eq!(access(&["-c", "branch.x=y", "status"]), None);
        assert_eq!(access(&[]), None);
    }

    /// A list waits for an add in progress, and an add for a list. On a lock of its own, so the gits of tests
    /// running beside it can't hold it.
    #[tokio::test]
    async fn a_list_waits_while_an_add_holds_the_metadata() {
        static LOCK: RwLock<()> = RwLock::const_new(());
        let hold = |args: &'static [&'static str]| hold_on(&LOCK, args);
        let wait = std::time::Duration::from_millis(200);
        let add = hold(&["worktree", "add", "/t"]).await;
        assert!(tokio::time::timeout(wait, hold(&["worktree", "list"])).await.is_err(), "a list ran beside an add");
        drop(add);
        let list = hold(&["worktree", "list"]).await;
        assert!(tokio::time::timeout(wait, hold(&["branch", "-D", "b"])).await.is_ok(), "two walks share the hold");
        assert!(tokio::time::timeout(wait, hold(&["worktree", "add", "/t"])).await.is_err(), "an add ran beside a list");
        drop(list);
        assert!(hold(&["rev-parse", "HEAD"]).await.is_none());
    }
}
