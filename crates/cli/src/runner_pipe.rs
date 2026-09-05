//! `farcooler runner pipe <id>` — the tunnel as a byte pipe on stdio.
//!
//! This exists so `~/.ssh/config` can name something OpenSSH can execute. A
//! tailcat token is not an address, so a tunneled runner has nothing true to
//! put after `HostName`, and without a command to run in its place Zed, git
//! and plain `ssh` would each lose every tunneled runner — the failure three
//! separate comments in this workspace already warn about.
//!
//! It is the same shape as `farcooler_transport::serve_stdio`, and for the
//! same reason: SSH is a byte pipe and so is this.
//!
//! **The spelling is a contract with a file that already exists.**
//! `apps/macos/Sources/FarCooler/SshConfig.swift` writes
//! `ProxyCommand farcooler runner pipe <id>` today, and the three tokens after
//! the binary name are the whole of what it says. Renaming this subcommand
//! rewrites nobody's `~/.ssh/config`; it only stops the ones already written
//! from working.
//!
//! **The token is not an argument, and that is the point of taking an id.**
//! `~/.ssh/config` is read by every editor and tool on the machine, and an
//! argument list is visible to every process on it. The id names the runner;
//! this resolves the reachability credential itself, from a file only its
//! owner can read.
//!
//! **No `ControlMaster`.** The plan left that to a measurement and the
//! measurement came back fast: `docs/superpowers/specs/2026-08-31-tailcat-spike-findings.md`
//! finding 5 puts cold process-start to first byte at a median of ~168 ms on a
//! Mac and warm round trips on an established tunnel at 6–12 ms. A control
//! socket would buy back a sixth of a second per `git fetch` and pay for it
//! with stale state pointing at a tunnel that can die underneath it. The one
//! measurement that would reopen it is cellular, which is unmeasured; that
//! finding says so and says the threshold is about a second.

use std::path::{Path, PathBuf};

use farcooler_client::ssh::{Reach, SshError, tunnel_error};
use tokio::io::AsyncWriteExt;

use crate::Fallible;

/// What this machine knows about the runners it reaches through a tunnel.
///
/// **Nothing in this repository writes this file yet**, and that is a real gap
/// rather than a detail of this module. No enrolment path anywhere produces a
/// tunneled runner: an offer's `node_key` is never populated and no granting
/// path calls `client.set_node_key`, so every runner every ceremony has ever
/// produced is `Reach::Direct`. The open decision is recorded at
/// `.claude/agent/needs-planning/nothing-mints-a-node-key-for-a-phone.md`.
/// What is here is the reader that a writer will need, exercised against a
/// file a test writes — not a path anybody is walking.
///
/// One file rather than two, because `node_key` is a private key and the
/// runner list travels beside it — which is also why `load_in` refuses to read
/// one anybody else can.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunnerStore {
    /// This DEVICE's own tailcat node private key.
    ///
    /// Held here, once, rather than inside each `Reach::Tailcat` — because it
    /// could not be held there. `Reach::Tailcat::client_key` is
    /// `#[serde(skip)]` globally (see its doc in `crates/client/src/ssh.rs`:
    /// a manifest is photographable and a private key in one was draft
    /// three's mistake), so a `Reach` that has been through serde always
    /// comes back with that field empty. A store that expected it to survive
    /// a round trip would read every runner as tunneled-but-keyless and dial
    /// with an empty key.
    ///
    /// It is also the right shape independently: the key is per DEVICE, not
    /// per runner. Ten tunneled runners are ten tokens and one key.
    ///
    /// Not to be confused with `Service::tailcat_key`, which is the identity a
    /// runner SERVES from. A Mac can be both, and the two files are not the
    /// same file.
    #[serde(default)]
    pub node_key: String,
    #[serde(default)]
    pub runners: Vec<StoredRunner>,
}

/// One runner, by the id `~/.ssh/config` names.
///
/// The id and the reach, and nothing else. A user, a host key and an alias all
/// belong to the `Host` block that OpenSSH is already reading by the time this
/// process starts — repeating them here would be two authorities on the same
/// answer, and this one is the copy that goes stale.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredRunner {
    pub id: String,
    pub reach: Reach,
}

impl RunnerStore {
    /// `<FARCOOLER_HOME>/runners.json`, beside the pairing this crate already
    /// reads from the same directory.
    pub fn path_in(runtime_dir: &Path) -> PathBuf {
        runtime_dir.join("runners.json")
    }

    /// Read the store, or an empty one.
    ///
    /// A missing file is an empty store, not an error: a machine that has
    /// never been granted a tunneled runner has nothing to say here, and the
    /// error a caller actually needs is about the id it asked for. A file that
    /// does not parse is NOT swallowed the same way — a store this cannot read
    /// is a store whose runner it cannot honestly report as absent.
    ///
    /// The permission check comes before the read, not after. `node_key` is a
    /// private key, and a file the rest of the machine can write is one whose
    /// contents were never this device's to begin with.
    pub fn load_in(runtime_dir: &Path) -> Result<Self, PipeError> {
        let path = Self::path_in(runtime_dir);
        let meta = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(PipeError::StoreUnreadable(e.to_string())),
        };
        refuse_loose_permissions(&path, &meta)?;
        let text = std::fs::read_to_string(&path)
            .map_err(|e| PipeError::StoreUnreadable(e.to_string()))?;
        serde_json::from_str(&text).map_err(|e| PipeError::StoreUnreadable(e.to_string()))
    }

    pub fn runner(&self, id: &str) -> Option<&StoredRunner> {
        self.runners.iter().find(|r| r.id == id)
    }
}

/// Refuse a store any other user can read or write.
///
/// The same rule `push.rs` applies when it WRITES a bearer token — owner-only
/// from the moment the file exists — applied here on the way in, because this
/// reader is the only half of that pair that exists yet. Nothing in this
/// repository creates `runners.json`, so the file a person has is one they or
/// some future writer made, and "it was created 0600" is not something this
/// can assume on their behalf.
///
/// Refusing rather than repairing. Silently `chmod`ing somebody's file would
/// hide the interesting question, which is who else has already read it.
#[cfg(unix)]
fn refuse_loose_permissions(path: &Path, meta: &std::fs::Metadata) -> Result<(), PipeError> {
    use std::os::unix::fs::MetadataExt;
    let mode = meta.mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(PipeError::StoreTooOpen {
            path: path.display().to_string(),
            mode: format!("{mode:03o}"),
        });
    }
    Ok(())
}

/// Windows has no mode bits to read, and no `~/.ssh/config` for a
/// `ProxyCommand` to be written into either.
#[cfg(not(unix))]
fn refuse_loose_permissions(_path: &Path, _meta: &std::fs::Metadata) -> Result<(), PipeError> {
    Ok(())
}

/// Why a pipe did not open.
///
/// Sentences, not words, and that is the difference between this and the FFI.
/// `TunnelError::code` exists because an app owns the sentence a person reads;
/// out here the CLI IS the app, and the person reading is looking at a terminal
/// that ssh has already told "Connection closed". So each of these has to say
/// enough to act on by itself.
///
/// No raw error string from the tunnel reaches any of them. The one place a
/// foreign string is carried is `StoreUnreadable`, which is serde_json naming a
/// line in a file the reader owns and can open.
#[derive(Debug, thiserror::Error)]
pub enum PipeError {
    #[error(
        "no runner with id {0} on this machine.\n\
         The id comes from the ProxyCommand line in ~/.ssh/config; if that block \
         names a runner this machine no longer has, re-run enrollment."
    )]
    Unknown(String),
    #[error(
        "runner {0} is reached directly, so it needs no pipe.\n\
         Its ~/.ssh/config block should carry HostName and Port rather than a \
         ProxyCommand."
    )]
    NotTunneled(String),
    #[error(
        "this machine has no tailcat identity, so it cannot dial a tunnel.\n\
         Nothing creates one yet: no enrollment path in this build grants a \
         tunneled runner."
    )]
    NoIdentity,
    #[error("the runner store could not be read: {0}")]
    StoreUnreadable(String),
    #[error(
        "{path} is mode {mode}, so other users on this machine can read it.\n\
         It holds this device's own tunnel private key. Fix it with: chmod 600 {path}"
    )]
    StoreTooOpen { path: String, mode: String },
    #[error("{0}")]
    Tunnel(String),
    #[error("the pipe stopped carrying bytes: {0}")]
    Io(#[from] std::io::Error),
}

/// Open the tunnel to `runner_id` and shuttle it over stdin and stdout.
///
/// Runs until the far end closes, which for a `ProxyCommand` is when the SSH
/// session ends.
pub async fn run(runner_id: &str) -> Fallible {
    let runtime_dir = farcooler_daemon::paths::runtime_dir()?;
    let store = RunnerStore::load_in(&runtime_dir)?;
    let stream = dial(&store, runner_id).await?;
    pump(stream).await?;
    Ok(())
}

/// Resolve an id to a live tunnel, or say precisely why not.
///
/// Split from `run` so every refusal above the byte pump is reachable from a
/// test without a tunnel, a runner or a terminal.
async fn dial(
    store: &RunnerStore,
    runner_id: &str,
) -> Result<tokio::net::UnixStream, PipeError> {
    let Some(runner) = store.runner(runner_id) else {
        return Err(PipeError::Unknown(runner_id.to_string()));
    };
    let Reach::Tailcat { token, .. } = &runner.reach else {
        return Err(PipeError::NotTunneled(runner_id.to_string()));
    };
    // The store's key, never the `Reach`'s: `client_key` is `#[serde(skip)]`,
    // so the one on a runner that has been read off disk is always empty. See
    // `RunnerStore::node_key`.
    if store.node_key.is_empty() {
        return Err(PipeError::NoIdentity);
    }
    // 22 is a name, not an address. It is the number the tunnel is keyed on;
    // where the runner's sshd actually listens is a fact only the runner has,
    // and its `OnTCP` handler maps it. `farcooler_tailcat::dial` carries a
    // `debug_assert_eq!` on this.
    farcooler_tailcat::dial(token, &store.node_key, 22)
        .await
        .map_err(|e| PipeError::Tunnel(sentence(tunnel_error(e))))
}

/// The sentence for a failed dial.
///
/// The mapping from `TunnelError` to what happened is
/// `farcooler_client::ssh::tunnel_error` — the same one the apps get, reused
/// rather than restated, because a second spelling of "connection refused
/// means the tunnel worked and sshd did not" is a second dialect and this
/// product has room for one.
fn sentence(error: SshError) -> String {
    match error {
        SshError::TunnelPortClosed { .. } => {
            "the tunnel reached the runner, but nothing is listening for SSH there.".to_string()
        }
        SshError::Tunnel { code } => match code {
            "no_tailcat" => "this build of farcooler has no tunnel it can dial. \
                 The Go archive is linked by the platform build scripts; a plain \
                 cargo build does direct SSH and nothing else."
                .to_string(),
            "derp" => "cannot reach the rendezvous service that introduces this machine \
                 to the runner. Check this machine's own network."
                .to_string(),
            "no_answer" => "the runner did not answer. It may be off, or this device's \
                 access to it may have been revoked."
                .to_string(),
            // `io` is deliberately generic upstream — a malformed token, a
            // dead sshd whose errno differs by platform, and EMFILE all wear
            // it — so the sentence claims nothing about which.
            _ => "the tunnel could not be opened.".to_string(),
        },
        // `tunnel_error` returns only the two variants above. This arm exists
        // so a third one is a sentence somebody wrote rather than a panic.
        other => format!("the tunnel could not be opened: {other}"),
    }
}

/// Carry bytes both ways until the runner closes.
///
/// The downlink is what this waits on, and the uplink runs beside it. The
/// ordering is the whole of the difference between this and a `select!` over
/// both halves: a `select!` returns when EITHER copy finishes, so an `ssh`
/// that closed its end of the pipe first would take the last unread bytes of
/// the runner's answer with it. Waiting on the downlink drains them, and the
/// uplink's `shutdown` is how the runner is told there is no more input
/// coming rather than being left waiting for some.
async fn pump(stream: tokio::net::UnixStream) -> Result<(), PipeError> {
    let (mut read, mut write) = stream.into_split();

    let uplink = tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let _ = tokio::io::copy(&mut stdin, &mut write).await;
        // A half-close, not a hang-up: the downlink is still ours to drain.
        let _ = write.shutdown().await;
    });

    let mut stdout = tokio::io::stdout();
    let carried = tokio::io::copy(&mut read, &mut stdout).await;
    // Flushed before the error is reported, so whatever DID arrive reaches ssh
    // even when the tunnel died mid-answer.
    let flushed = stdout.flush().await;
    uplink.abort();
    carried?;
    flushed?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tunneled(id: &str, token: &str) -> StoredRunner {
        StoredRunner {
            id: id.into(),
            reach: Reach::Tailcat { token: token.into(), client_key: String::new() },
        }
    }

    fn direct(id: &str) -> StoredRunner {
        StoredRunner {
            id: id.into(),
            reach: Reach::Direct { host: "10.0.0.4".into(), port: 22 },
        }
    }

    fn store(runners: Vec<StoredRunner>) -> RunnerStore {
        RunnerStore { node_key: "a-node-key".into(), runners }
    }

    /// Owner-only, because `load_in` refuses anything else — the umask a test
    /// inherits is usually 022, which would make every fixture here a
    /// world-readable private key and every assertion below the wrong one.
    fn write_store(dir: &Path, text: &str) {
        let path = RunnerStore::path_in(dir);
        std::fs::write(&path, text).expect("write the store");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .expect("chmod");
        }
    }

    /// A machine that has never been granted a tunneled runner has an empty
    /// store, not a broken one — and the error a person then sees is about the
    /// id they asked for rather than about a missing file they never made.
    #[test]
    fn a_missing_store_is_an_empty_store() {
        let dir = tempfile::tempdir().expect("tempdir");
        let loaded = RunnerStore::load_in(dir.path()).expect("a missing file is not an error");
        assert!(loaded.runners.is_empty());
        assert!(loaded.node_key.is_empty());
    }

    /// A store that does not parse is NOT an empty store. Reading it as one
    /// would report every runner in it as absent, which sends whoever reads
    /// that message to re-run enrollment over a file that is merely damaged.
    #[test]
    fn a_damaged_store_is_refused_rather_than_read_as_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_store(dir.path(), "{ not json");
        let out = RunnerStore::load_in(dir.path());
        assert!(matches!(out, Err(PipeError::StoreUnreadable(_))), "{out:?}");
    }

    /// A store the rest of the machine can read is refused, not read.
    ///
    /// It holds this device's own tunnel private key. The same rule `push.rs`
    /// applies when it writes a bearer token, on the way in instead — and the
    /// message names the file and the fix, because the person who can act on
    /// this is the one whose umask made it.
    #[cfg(unix)]
    #[test]
    fn a_store_other_users_can_read_is_refused() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = RunnerStore::path_in(dir.path());
        std::fs::write(&path, r#"{"node_key":"k","runners":[]}"#).expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");

        let out = RunnerStore::load_in(dir.path());
        let Err(PipeError::StoreTooOpen { mode, .. }) = &out else {
            panic!("a world-readable private key was read: {out:?}");
        };
        assert_eq!(mode, "644");

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        assert!(RunnerStore::load_in(dir.path()).is_ok(), "0600 must still be readable");
    }

    /// The field that cannot survive serde is the one this store holds
    /// separately. If `client_key` ever stopped being `#[serde(skip)]` this
    /// would still pass — what it guards is the other direction: that a
    /// round-tripped store still has a key to dial with.
    #[test]
    fn the_device_key_survives_a_round_trip_and_the_reach_key_does_not() {
        let written = store(vec![tunneled("box", "tc-x")]);
        let json = serde_json::to_string(&written).expect("encode");
        let back: RunnerStore = serde_json::from_str(&json).expect("decode");

        assert_eq!(back.node_key, "a-node-key", "the device key must survive: {json}");
        let Reach::Tailcat { token, client_key } = &back.runner("box").expect("box").reach else {
            panic!("box came back direct: {json}");
        };
        assert_eq!(token, "tc-x");
        assert!(
            client_key.is_empty(),
            "Reach::client_key came back populated, so this store's separate node_key \
             is no longer the only key a dial can use: {json}"
        );
    }

    /// The credential is resolved from the store, so it is never in the
    /// argument list — which is the entire reason this subcommand takes an id.
    ///
    /// Only without the archive, which is the default and what CI runs: with
    /// `--features tailcat` this reaches a real DERP dial for a token nobody
    /// minted, and a test that waits on a network timeout is a test that
    /// eventually gets deleted.
    #[cfg(not(feature = "tailcat"))]
    #[tokio::test]
    async fn a_token_is_resolved_from_the_store_rather_than_named() {
        let store = store(vec![tunneled("box", "tc-secret")]);
        // No archive is linked in a plain build, so the dial fails — but it
        // fails at the DIAL, which is only reachable once the id resolved to a
        // token and a key.
        let out = dial(&store, "box").await;
        let Err(PipeError::Tunnel(message)) = out else {
            panic!("expected a tunnel failure, got {out:?}");
        };
        assert!(
            !message.contains("tc-secret"),
            "the token reached an error message: {message}"
        );
    }

    #[tokio::test]
    async fn an_unknown_id_is_named() {
        let out = dial(&store(vec![]), "box").await;
        assert!(matches!(&out, Err(PipeError::Unknown(id)) if id == "box"), "{out:?}");
        assert!(out.unwrap_err().to_string().contains("box"));
    }

    /// A direct runner is refused rather than dialed. The tunnel has nothing
    /// to carry for it, and a `ProxyCommand` on its block is a mistake in
    /// `~/.ssh/config` worth naming as one.
    #[tokio::test]
    async fn a_direct_runner_is_refused_rather_than_dialed() {
        let out = dial(&store(vec![direct("box")]), "box").await;
        assert!(matches!(&out, Err(PipeError::NotTunneled(id)) if id == "box"), "{out:?}");
    }

    /// The store's key is what a dial uses, so a store without one is refused
    /// before `dial` is reached rather than dialing with an empty key — which
    /// tailcat would answer with a timeout that reads like a revoked device.
    #[tokio::test]
    async fn a_store_with_no_device_key_is_refused_before_the_dial() {
        let store = RunnerStore { node_key: String::new(), runners: vec![tunneled("box", "tc-x")] };
        let out = dial(&store, "box").await;
        assert!(matches!(out, Err(PipeError::NoIdentity)), "{out:?}");
    }

    /// Every stable word from `TunnelError::code` gets a sentence of its own,
    /// and none of them is a Rust error string.
    #[test]
    fn every_tunnel_word_becomes_a_sentence() {
        for code in ["no_tailcat", "derp", "no_answer", "io"] {
            let text = sentence(SshError::Tunnel { code });
            assert!(!text.contains(code), "the stable word leaked into the copy: {text}");
            assert!(text.ends_with('.'), "not a sentence: {text}");
        }
        let closed = sentence(SshError::TunnelPortClosed {
            source: std::io::Error::from(std::io::ErrorKind::ConnectionRefused),
        });
        assert!(closed.contains("nothing is listening"), "{closed}");
    }
}
