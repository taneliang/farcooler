//! `farcooler runner mint-node-key` and `store-node-key`: this machine's
//! tunnel identity, for the Mac app.
//!
//! The Mac app links the client core WITHOUT the Go archive, so it cannot mint
//! a node key itself; the bundled CLI is built with the archive and can. The
//! split keeps the secret where each half belongs. Minting writes nothing and
//! prints the pair once, to the app that asked. The app puts it in the Keychain,
//! which is the source of truth, and calls `store-node-key` to leave the copy
//! that `runner pipe` reads — a `ProxyCommand` runs with no app around it.

use std::io::{IsTerminal as _, Read as _};

use farcooler_daemon::paths;

use crate::runner_pipe::RunnerStore;
use crate::Fallible;

/// A node private key as tailcat spells it: 43 characters of unpadded
/// base64-URL. Checked here so a stray newline, a prompt or an error message
/// piped in by mistake is refused instead of becoming the key every dial uses.
fn well_formed(key: &str) -> bool {
    key.len() == 43 && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn mint() -> Fallible {
    // A private key printed to a terminal lands in scrollback and logs, and
    // nothing stores it. This is app plumbing; the app reads a pipe.
    if std::io::stdout().is_terminal() {
        return Err("this prints a private key for the Mac app to keep; it will not print to a terminal".into());
    }
    // `minted` is the phones' own check, reused: a pair whose public half the
    // fence would refuse never leaves here.
    println!("{}", farcooler_client::ffi::minted(farcooler_tailcat::mint_node_key()));
    Ok(())
}

pub fn store() -> Fallible {
    if std::io::stdin().is_terminal() {
        return Err("this reads a private key from the Mac app on stdin, not from a terminal".into());
    }
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let dir = paths::ensure_runtime_dir()?;
    keep(&dir, text.trim())?;
    Ok(())
}

/// Put `key` in `runners.json` under `dir`, keeping what else is there.
fn keep(dir: &std::path::Path, key: &str) -> Result<(), crate::runner_pipe::PipeError> {
    if !well_formed(key) {
        return Err(crate::runner_pipe::PipeError::StoreUnreadable(
            "that is not a tunnel private key".into(),
        ));
    }
    let mut store = RunnerStore::load_in(dir)?;
    store.node_key = key.to_string();
    store.save_in(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_AbCde";

    #[test]
    fn the_key_lands_owner_only_and_the_runners_beside_it_stay() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            RunnerStore::path_in(dir.path()),
            r#"{"node_key":"old","runners":[{"id":"box","reach":{"kind":"tailcat","token":"tc-x"}}]}"#,
        )
        .unwrap();
        std::fs::set_permissions(
            RunnerStore::path_in(dir.path()),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();

        keep(dir.path(), KEY).unwrap();

        let mode = std::fs::metadata(RunnerStore::path_in(dir.path())).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the private key was written readable by others");
        let back = RunnerStore::load_in(dir.path()).unwrap();
        assert_eq!(back.node_key, KEY);
        assert!(back.runner("box").is_some(), "storing the key dropped a runner");
        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "a temporary file was left behind: {leftovers:?}");
    }

    #[test]
    fn a_first_key_creates_the_file_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        keep(dir.path(), KEY).unwrap();
        let mode = std::fs::metadata(RunnerStore::path_in(dir.path())).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(RunnerStore::load_in(dir.path()).unwrap().node_key, KEY);
    }

    #[test]
    fn something_that_is_not_a_key_is_refused_and_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["", "short", "{\"error\":\"no_tailcat\"}", &format!("{KEY}\n{KEY}")] {
            assert!(keep(dir.path(), bad).is_err(), "kept {bad:?}");
        }
        assert!(!RunnerStore::path_in(dir.path()).exists());
    }

    #[test]
    fn a_store_other_users_can_read_is_not_rewritten() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = RunnerStore::path_in(dir.path());
        std::fs::write(&path, r#"{"node_key":"old","runners":[]}"#).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(keep(dir.path(), KEY).is_err());
        assert!(std::fs::read_to_string(&path).unwrap().contains("old"));
    }
}
