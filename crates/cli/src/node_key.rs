//! `farcooler runner mint-node-key` and `store-node-key`: this machine's
//! tunnel identity, for the Mac app.
//!
//! The Mac app links the client core WITHOUT the Go archive, so it cannot mint
//! a node key itself; the bundled CLI is built with the archive and can. The
//! split keeps the secret where each half belongs. Minting writes nothing and
//! prints the pair once, to the app that asked. The app puts it in the Keychain,
//! which is the source of truth, and calls `store-node-key` to leave the copy
//! that `runner pipe` reads — a `ProxyCommand` runs with no app around it.
//!
//! `store-runners` is the same door for the other half of that file: the
//! tunneled runners this Mac was granted, by the id a `ProxyCommand` names.

use std::io::{IsTerminal as _, Read as _};

use farcooler_daemon::paths;

use farcooler_client::ssh::Reach;

use crate::runner_pipe::{PipeError, RunnerStore, StoredRunner};
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
fn keep(dir: &std::path::Path, key: &str) -> Result<(), PipeError> {
    if !well_formed(key) {
        return Err(PipeError::StoreUnreadable("that is not a tunnel private key".into()));
    }
    locked(dir, || {
        let mut store = RunnerStore::load_in(dir)?;
        store.node_key = key.to_string();
        store.save_in(dir)
    })
}

/// One tunneled runner this Mac was granted: what the app sends on stdin.
#[derive(serde::Deserialize)]
struct Granted {
    id: String,
    token: String,
    /// `SHA256:` and 43 base64 characters: the host key the granter pinned.
    host_key: String,
}

/// A fingerprint as `ssh-keygen -l` spells it. Checked on the way in so a
/// pin that can never match is refused when it is stored, not when it is used.
fn well_formed_pin(pin: &str) -> bool {
    pin.strip_prefix("SHA256:").is_some_and(|b| {
        b.len() == 43 && b.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/')
    })
}

/// An id is typed into a `ProxyCommand` line, which `ssh` hands to a shell, so
/// it is held to a spelling that no shell reads as anything else.
fn safe_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && !id.starts_with(['-', '.'])
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

/// `[{"id","token"}, ...]` on stdin: keep each as a tunneled runner in
/// `runners.json`, replacing one with the same id and leaving the rest.
///
/// The token travels on stdin like the key does. It is not as secret as the
/// key, but it is the one field in the file worth stealing, and an argument
/// list is visible to every process on the machine.
pub fn store_runners() -> Fallible {
    if std::io::stdin().is_terminal() {
        return Err("this reads the runners the Mac app was granted on stdin, not from a terminal".into());
    }
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let granted: Vec<Granted> = serde_json::from_str(&text)?;
    let dir = paths::ensure_runtime_dir()?;
    keep_runners(&dir, &granted)?;
    Ok(())
}

/// Nothing is written unless every entry is acceptable.
fn keep_runners(dir: &std::path::Path, granted: &[Granted]) -> Result<(), PipeError> {
    for g in granted {
        if !safe_id(&g.id) {
            return Err(PipeError::StoreUnreadable("that is not a runner id".into()));
        }
        if g.token.is_empty() || g.token.bytes().any(|b| !b.is_ascii_graphic()) {
            return Err(PipeError::StoreUnreadable("that is not a tunnel token".into()));
        }
        if !well_formed_pin(&g.host_key) {
            return Err(PipeError::StoreUnreadable("that is not a host key fingerprint".into()));
        }
    }
    locked(dir, || {
        let mut store = RunnerStore::load_in(dir)?;
        for g in granted {
            let reach = Reach::Tailcat { token: g.token.clone(), client_key: String::new() };
            // A grant again replaces the pin along with the token, which is how
            // a runner whose host key changed on purpose is re-trusted.
            match store.runners.iter_mut().find(|r| r.id == g.id) {
                Some(have) => {
                    have.reach = reach;
                    have.host_key = g.host_key.clone();
                }
                None => store.runners.push(StoredRunner {
                    id: g.id.clone(),
                    reach,
                    host_key: g.host_key.clone(),
                }),
            }
        }
        store.save_in(dir)
    })
}

/// `["id", ...]` on stdin: drop each from `runners.json`, token and pin with it.
/// An id that is not there is not an error: forgetting twice is the same as once.
pub fn forget_runners() -> Fallible {
    if std::io::stdin().is_terminal() {
        return Err("this reads runner ids from the Mac app on stdin, not from a terminal".into());
    }
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let ids: Vec<String> = serde_json::from_str(&text)?;
    let dir = paths::ensure_runtime_dir()?;
    forget(&dir, &ids)?;
    Ok(())
}

fn forget(dir: &std::path::Path, ids: &[String]) -> Result<(), PipeError> {
    locked(dir, || {
        let mut store = RunnerStore::load_in(dir)?;
        store.runners.retain(|r| !ids.contains(&r.id));
        store.save_in(dir)
    })
}

/// `ssh`'s `KnownHostsCommand` for a tunneled runner: `known-host <id> <reason>
/// <host> [<type> <key>]`, from `%I %H %t %K`.
///
/// Prints a `known_hosts` line only when the key `ssh` was offered has the
/// fingerprint the granting device pinned. Both known-hosts files are
/// `/dev/null` in the block, so this is the only key `ssh` will trust for the
/// runner: nothing learned on first use, nothing stale to block it.
/// `reason` `ORDER` comes with no key (ssh is choosing algorithms, and expands
/// `%t` and `%K` to the word `NONE`), and the answer is nothing, because only a
/// fingerprint is stored.
pub fn known_host(id: &str, host: &str, key_type: Option<&str>, key: Option<&str>) -> Fallible {
    let (Some(key_type), Some(key)) = (key_type, key) else { return Ok(()) };
    if key_type == "NONE" || key == "NONE" {
        return Ok(());
    }
    let dir = paths::runtime_dir()?;
    let store = RunnerStore::load_in(&dir)?;
    let pinned = store.runner(id).map(|r| r.host_key.as_str()).unwrap_or_default();
    if pinned.is_empty() {
        return Err("this Mac has no host key on file for that runner".into());
    }
    let offered = farcooler_client::ceremony::fingerprint(&format!("{key_type} {key}"));
    if offered.as_deref() != Some(pinned) {
        return Err("this runner's host key isn't the one it was granted with".into());
    }
    println!("{host} {key_type} {key}");
    Ok(())
}

/// Hold an exclusive lock on the directory for one read, change and write, so
/// the two commands above cannot lose each other's update. Advisory, and
/// released when the handle drops.
fn locked<T>(
    dir: &std::path::Path,
    work: impl FnOnce() -> Result<T, PipeError>,
) -> Result<T, PipeError> {
    use std::os::fd::AsRawFd as _;
    let handle = std::fs::File::open(dir).map_err(|e| PipeError::StoreUnreadable(e.to_string()))?;
    // SAFETY: `handle` owns a live descriptor for the whole call.
    if unsafe { libc::flock(handle.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(PipeError::StoreUnreadable(std::io::Error::last_os_error().to_string()));
    }
    work()
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

    const PIN: &str = "SHA256:iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4";

    fn granted(id: &str, token: &str) -> Granted {
        Granted { id: id.into(), token: token.into(), host_key: PIN.into() }
    }

    #[test]
    fn a_pin_that_can_never_match_is_refused_when_stored() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["", "SHA256:short", "MD5:aa:bb", "iDqoxaySm9gzxtvLrNXXpM5PimPLeBknaaNj0Rg7vz4"] {
            let g = Granted { id: "fc-a".into(), token: "tc".into(), host_key: bad.into() };
            assert!(keep_runners(dir.path(), &[g]).is_err(), "kept pin {bad:?}");
        }
        assert!(!RunnerStore::path_in(dir.path()).exists());
    }

    #[test]
    fn granting_again_replaces_the_pin() {
        let dir = tempfile::tempdir().unwrap();
        keep_runners(dir.path(), &[granted("fc-a", "tc-1")]).unwrap();
        let other = "SHA256:abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ";
        let g = Granted { id: "fc-a".into(), token: "tc-1".into(), host_key: other.into() };
        keep_runners(dir.path(), &[g]).unwrap();
        let back = RunnerStore::load_in(dir.path()).unwrap();
        assert_eq!(back.runner("fc-a").unwrap().host_key, other);
    }

    #[test]
    fn forgetting_drops_the_runner_and_leaves_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        keep(dir.path(), KEY).unwrap();
        keep_runners(dir.path(), &[granted("fc-a", "t1"), granted("fc-b", "t2")]).unwrap();
        forget(dir.path(), &["fc-a".into(), "fc-never".into()]).unwrap();
        let back = RunnerStore::load_in(dir.path()).unwrap();
        assert!(back.runner("fc-a").is_none(), "the token survived forgetting");
        assert!(back.runner("fc-b").is_some());
        assert_eq!(back.node_key, KEY);
    }

    #[test]
    fn a_granted_runner_lands_beside_the_key_and_replaces_its_own_old_token() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        keep(dir.path(), KEY).unwrap();
        keep_runners(dir.path(), &[granted("fc-a", "tc-1"), granted("fc-b", "tc-2")]).unwrap();
        keep_runners(dir.path(), &[granted("fc-a", "tc-3")]).unwrap();

        let back = RunnerStore::load_in(dir.path()).unwrap();
        assert_eq!(back.node_key, KEY, "storing runners dropped the key");
        assert_eq!(back.runners.len(), 2, "{:?}", back.runners);
        let token = |id: &str| match &back.runner(id).unwrap().reach {
            Reach::Tailcat { token, .. } => token.clone(),
            other => panic!("{id} is not tunneled: {other:?}"),
        };
        assert_eq!(token("fc-a"), "tc-3");
        assert_eq!(token("fc-b"), "tc-2");
        let mode = std::fs::metadata(RunnerStore::path_in(dir.path())).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "a temporary file was left");
    }

    /// The id is typed into a `ProxyCommand` line that `ssh` hands to a shell.
    #[test]
    fn an_id_a_shell_would_read_is_refused_and_nothing_in_the_batch_is_written() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["", "a b", "x;rm", "$(id)", "a'b", "a\"b", "-oProxyCommand=x", ".hidden", "a/b", "a%h", &"x".repeat(65)] {
            let batch = [granted("fc-ok", "tc-1"), granted(bad, "tc-2")];
            assert!(keep_runners(dir.path(), &batch).is_err(), "kept id {bad:?}");
        }
        assert!(!RunnerStore::path_in(dir.path()).exists(), "a refused batch wrote a file");
    }

    #[test]
    fn something_that_is_not_a_token_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["", "two words", "line\nbreak"] {
            assert!(keep_runners(dir.path(), &[granted("fc-a", bad)]).is_err(), "kept {bad:?}");
        }
        assert!(!RunnerStore::path_in(dir.path()).exists());
    }

    /// The lock is what keeps two writers from losing each other's update.
    #[test]
    fn two_writers_at_once_lose_neither_update() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let path = path.clone();
                std::thread::spawn(move || {
                    keep_runners(&path, &[granted(&format!("fc-{n}"), "tc-x")]).unwrap();
                    keep(&path, KEY).unwrap();
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let back = RunnerStore::load_in(dir.path()).unwrap();
        assert_eq!(back.runners.len(), 8, "an update was lost: {:?}", back.runners);
        assert_eq!(back.node_key, KEY);
    }
}
