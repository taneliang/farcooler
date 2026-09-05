//! Which devices the tunnel admits, derived from `authorized_keys` and stored
//! nowhere.
//!
//! This is `enrollment::list` with a different projection, and that is the
//! whole design. An allowlist held in its own file would be a second place the
//! truth can be wrong, and the way it would be wrong is that `client.revoke`
//! removes a device's key and forgets its node key — leaving a revoked device
//! with a route to sshd. There is nothing here to forget to update.
//!
//! What this does NOT make instant is the RUNNING server, which holds the copy
//! of the allowlist it was handed at `Start`. A revocation lands here at once
//! and reaches the tunnel at the next `serve` — today, the next boot. See
//! `enrollment::revoke`.

use std::path::Path;

use farcooler_fence::Entry;

/// A non-empty set of node keys the tunnel will admit.
///
/// The type cannot hold zero, and that is the point. Tailcat's
/// `AllowedClients` treats an empty slice as "admit everyone", so a runner
/// that built one from a fleet enrolled before this feature existed would open
/// its sshd to anyone holding the token. `from_entries` answers `None` instead,
/// and a `None` starts no server at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allowlist(Vec<String>);

impl Allowlist {
    pub fn keys(&self) -> &[String] {
        &self.0
    }
}

/// The devices this runner's `authorized_keys` admits to the tunnel.
///
/// A line admits a device when it enrolls one — a non-empty `client_id`, which
/// is what `fence` uses to say a line is ours rather than foreign — and when
/// that line carries a node key. Order follows the file so the result is
/// stable between reads and a diff of two allowlists is legible.
pub fn from_entries(entries: &[Entry]) -> Option<Allowlist> {
    let mut keys: Vec<String> = Vec::new();
    for entry in entries {
        if entry.client_id.is_empty() || entry.node_key.is_empty() {
            continue;
        }
        if !keys.iter().any(|k| k == &entry.node_key) {
            keys.push(entry.node_key.clone());
        }
    }
    (!keys.is_empty()).then_some(Allowlist(keys))
}

/// Why this runner is not serving a tunnel, or the token a client would dial
/// if it is.
///
/// A plain `Option<String>` cannot say which of these is true, and each one
/// means something different to whoever reads the log. `NoIdentity` and
/// `NobodyAdmitted` are this runner's own decision, and ordinary for a fleet
/// that has not run a QR ceremony yet. `FenceUnreadable` is `authorized_keys`
/// itself refusing to parse. `ServeFailed` is `farcooler_tailcat::serve`
/// refusing, named with its own stable code — `"no_tailcat"` for every build
/// with no archive linked, which is every build `cargo test`/`cargo build`
/// produce by default. `ConnBlobFailed` is the dangerous one: `serve`
/// SUCCEEDED — the tunnel is live — and only the value this runner would use
/// to report its own token is missing, which without a distinct variant here
/// would read identically to "no tunnel" in every caller that only checked
/// for `Some`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelOutcome {
    /// `serve` and `conn_blob` both succeeded; this is the token a client
    /// dials.
    Serving(String),
    /// No persistent tailcat identity — see `Service::tailcat_key`.
    NoIdentity,
    /// `authorized_keys` admits nobody with a node key — see `from_entries`.
    NobodyAdmitted,
    /// `authorized_keys` could not be read at all.
    FenceUnreadable,
    /// `farcooler_tailcat::serve` refused or failed, with its own stable code.
    ServeFailed(&'static str),
    /// `serve` succeeded but `farcooler_tailcat::conn_blob` did not — the
    /// tunnel is live regardless.
    ConnBlobFailed(&'static str),
}

impl TunnelOutcome {
    /// The token a client would dial, for every outcome that has one.
    pub fn blob(self) -> Option<String> {
        match self {
            Self::Serving(blob) => Some(blob),
            _ => None,
        }
    }
}

/// The plan for this runner's tunnel: who it would admit — or why there is no
/// plan at all.
///
/// Synchronous and total, and touches nothing but a `Path::exists` stat: no
/// FFI, no Go, nothing that can only fail in a build with the archive linked.
/// This is deliberately the ONLY place `start_tunnel`'s two admission guards
/// live, and it is `pub` so a test can call it directly.
///
/// That directness is load-bearing, not decoration. `cargo test` builds this
/// crate's DEFAULT features, which means no archive linked, which means
/// `farcooler_tailcat::serve` returns `Err(NoTailcatLinked)` UNCONDITIONALLY —
/// regardless of what it is asked to serve. A test that asserts only on
/// `start_tunnel`'s end-to-end return value is therefore vacuous in the one
/// configuration `cargo test` actually exercises: deleting either guard in
/// `start_tunnel` still routes through `serve`, which the stub still refuses,
/// so the observable outcome does not change and the test cannot go red. This
/// function is what makes it possible to prove otherwise: assert on it
/// directly, with entries built in memory, and deleting either guard here
/// turns a test red with no Go toolchain and no archive required.
pub fn tunnel_plan(key_path: &Path, entries: &[Entry]) -> Result<Allowlist, TunnelOutcome> {
    if !key_path.exists() {
        return Err(TunnelOutcome::NoIdentity);
    }
    from_entries(entries).ok_or(TunnelOutcome::NobodyAdmitted)
}

/// Start this runner's tunnel, if it has one to start.
///
/// `tunnel_plan` makes the admit/refuse decision; this function is only the
/// effect of it — reading `authorized_keys`, and then, only once admitted,
/// reaching into `farcooler_tailcat`. See `tunnel_plan`'s doc comment for why
/// the decision is split out rather than inlined here.
///
/// `farcooler_tailcat::serve` and `conn_blob` are synchronous and can block
/// for 30-45 seconds: the Go side holds a package-wide mutex for the whole of
/// `serve`, which includes up to two 15-second region-pick budgets plus two
/// `Start` attempts. Both run on the blocking pool so a slow tunnel start
/// never stalls the runtime workers serving this daemon's Unix socket. The
/// `authorized_keys` read runs there too, for the same reason
/// `enrollment.rs`'s file work does.
///
/// Callers must not `.await` this inline before the daemon binds its Unix
/// socket: the same 30-45 seconds happens before this runner is reachable at
/// all if they do. See the call site in `main.rs`.
pub async fn start_tunnel(service: &crate::service::Service) -> TunnelOutcome {
    let key_path = service.tailcat_key();

    let auth_path = service.authorized_keys().to_path_buf();
    let entries = tokio::task::spawn_blocking(move || {
        farcooler_fence::read(&auth_path, farcooler_fence::AUTHORIZED_KEYS)
    })
    .await;
    let entries = match entries {
        Ok(Ok(entries)) => entries,
        Ok(Err(error)) => {
            tracing::warn!(
                %error,
                "authorized_keys could not be read; this runner serves no tunnel"
            );
            return TunnelOutcome::FenceUnreadable;
        }
        Err(error) => {
            tracing::warn!(%error, "the authorized_keys read task did not finish");
            return TunnelOutcome::FenceUnreadable;
        }
    };

    let allowed = match tunnel_plan(&key_path, &entries) {
        Ok(allowed) => allowed,
        Err(TunnelOutcome::NoIdentity) => {
            tracing::debug!("no tailcat identity; this runner serves no tunnel");
            return TunnelOutcome::NoIdentity;
        }
        Err(TunnelOutcome::NobodyAdmitted) => {
            tracing::info!("no enrolled device carries a node key; this runner serves no tunnel");
            return TunnelOutcome::NobodyAdmitted;
        }
        // `tunnel_plan` only ever returns these two variants; the rest exist
        // for `start_tunnel`'s own later steps.
        Err(other) => return other,
    };

    let ssh_port = service.ssh_port();
    let serve_key_path = key_path.clone();
    let allowed_keys = allowed.keys().to_vec();
    let served = tokio::task::spawn_blocking(move || {
        farcooler_tailcat::serve(&serve_key_path, ssh_port, &allowed_keys)
    })
    .await;
    match served {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::warn!(code = error.code(), "the tunnel did not start");
            return TunnelOutcome::ServeFailed(error.code());
        }
        Err(error) => {
            tracing::warn!(%error, "the tunnel-serving task did not finish");
            return TunnelOutcome::ServeFailed("io");
        }
    }

    match tokio::task::spawn_blocking(farcooler_tailcat::conn_blob).await {
        Ok(Ok(blob)) => TunnelOutcome::Serving(blob),
        Ok(Err(error)) => {
            // The tunnel IS live at this point — `serve` already succeeded.
            // Only the token used to report it is missing, which is why this
            // gets its own variant rather than folding into `ServeFailed` or
            // being swallowed as `None`: a runner in this state is serving
            // real traffic while every caller that only checked `Some`
            // would conclude it was not.
            tracing::warn!(
                code = error.code(),
                "the tunnel is serving, but its connection token could not be read"
            );
            TunnelOutcome::ConnBlobFailed(error.code())
        }
        Err(error) => {
            tracing::warn!(%error, "the connection-blob task did not finish");
            TunnelOutcome::ConnBlobFailed("io")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_fence::Entry;
    use farcooler_protocol::v1::Scope;

    fn entry(client_id: &str, node_key: &str) -> Entry {
        Entry {
            fingerprint: "SHA256:x".into(),
            client_id: client_id.into(),
            scope: Scope::Read,
            label: "l".into(),
            account: None,
            node_key: node_key.into(),
            line: String::new(),
            shell_access: false,
        }
    }

    // Two node keys that are genuinely two keys. 43 base64 characters carry
    // 258 bits, so the last character's low two bits are padding: "...AAA"
    // through "...AAD" all decode to the same 32 bytes, and only "...AAE"
    // moves a bit that is really there. These tests compare strings and so
    // would pass either way, but "two devices" has to mean two devices to the
    // runner, which decodes them.
    const A: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const B: &str = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE";

    #[test]
    fn no_entries_admits_nobody() {
        assert!(from_entries(&[]).is_none());
    }

    /// The one that matters. A fleet enrolled before the tunnel existed has
    /// lines but no node keys, and an empty allowlist would admit the world.
    #[test]
    fn entries_without_node_keys_admit_nobody() {
        let entries = [entry("c1", ""), entry("c2", "")];
        assert!(from_entries(&entries).is_none(), "an empty allowlist was built");
    }

    #[test]
    fn only_entries_with_node_keys_are_admitted() {
        let entries = [entry("c1", A), entry("c2", ""), entry("c3", B)];
        let allowed = from_entries(&entries).expect("two devices carry node keys");
        assert_eq!(allowed.keys(), [A, B]);
    }

    /// A foreign line is somebody else's key in this user's file. It has no
    /// client id, so it enrolls no device, so it admits no device.
    #[test]
    fn a_foreign_line_is_never_admitted() {
        let entries = [entry("", A)];
        assert!(from_entries(&entries).is_none());
    }

    #[test]
    fn one_device_enrolled_twice_is_admitted_once() {
        let entries = [entry("c1", A), entry("c1", A)];
        assert_eq!(from_entries(&entries).unwrap().keys(), [A]);
    }

    /// `tunnel_plan` in isolation — no `Service`, no runtime, no FFI. This is
    /// the assertion the daemon's `start_tunnel` guards live or die by: it
    /// cannot be satisfied by a stub that refuses every call regardless of
    /// input, because nothing here ever reaches `farcooler_tailcat` at all.
    #[test]
    fn tunnel_plan_refuses_a_runner_with_no_identity() {
        let missing = Path::new("/nonexistent/tailcat.key");
        let entries = [entry("c1", A)];
        assert_eq!(tunnel_plan(missing, &entries), Err(TunnelOutcome::NoIdentity));
    }

    /// The guard this whole design exists to protect, tested directly against
    /// the decision rather than through anything that could mask it.
    #[test]
    fn tunnel_plan_refuses_a_runner_nobody_is_admitted_on() {
        let key = tempfile::NamedTempFile::new().expect("a scratch key file");
        let entries = [entry("c1", ""), entry("c2", "")];
        assert_eq!(
            tunnel_plan(key.path(), &entries),
            Err(TunnelOutcome::NobodyAdmitted),
            "an admits-nobody allowlist was allowed through"
        );
    }

    #[test]
    fn tunnel_plan_admits_a_runner_with_identity_and_a_real_device() {
        let key = tempfile::NamedTempFile::new().expect("a scratch key file");
        let entries = [entry("c1", A)];
        let allowed =
            tunnel_plan(key.path(), &entries).expect("identity present, one device admitted");
        assert_eq!(allowed.keys(), [A]);
    }
}
