//! Which devices the tunnel admits, derived from `authorized_keys` and stored
//! nowhere.
//!
//! This is `enrollment::list` with a different projection, and that is the
//! whole design. An allowlist held in its own file would be a second place the
//! truth can be wrong, and the way it would be wrong is that `client.revoke`
//! removes a device's key and forgets its node key — leaving a revoked device
//! with a route to sshd. There is nothing here to forget to update.

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

/// Start this runner's tunnel, if it has one to start.
///
/// Two things must both be true, and neither is a default. The runner must
/// hold a persistent identity — an operator who never ran a QR ceremony has no
/// key file and gets no tunnel. And `authorized_keys` must admit at least one
/// device, because tailcat reads an empty allowlist as "admit everyone" and a
/// runner that opened its sshd to the world would look, from here, exactly
/// like one that worked.
///
/// Reads `authorized_keys` through `farcooler_fence::read` directly rather
/// than `enrollment::list`: the wire's `ClientList` never carries a node key —
/// nothing a client displays needs one, so `EnrolledClient` has no field for
/// it — and it is the one thing this function exists to ask about.
///
/// `farcooler_tailcat::serve` and `conn_blob` are synchronous and can block
/// for 30-45 seconds: the Go side holds a package-wide mutex for the whole of
/// `serve`, which includes up to two 15-second region-pick budgets plus two
/// `Start` attempts. Both run on the blocking pool so a slow tunnel start
/// never stalls the runtime workers serving this daemon's Unix socket.
pub async fn start_tunnel(service: &crate::service::Service) -> Option<String> {
    let key_path = service.tailcat_key();
    if !key_path.exists() {
        tracing::debug!("no tailcat identity; this runner serves no tunnel");
        return None;
    }

    let auth_path = service.authorized_keys().to_path_buf();
    let entries = tokio::task::spawn_blocking(move || {
        farcooler_fence::read(&auth_path, farcooler_fence::AUTHORIZED_KEYS)
    })
    .await
    .inspect_err(|error| tracing::warn!(%error, "the authorized_keys read task did not finish"))
    .ok()?
    .inspect_err(|error| {
        tracing::warn!(%error, "authorized_keys could not be read; this runner serves no tunnel")
    })
    .ok()?;

    let Some(allowed) = from_entries(&entries) else {
        tracing::info!("no enrolled device carries a node key; this runner serves no tunnel");
        return None;
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
            return None;
        }
        Err(error) => {
            tracing::warn!(%error, "the tunnel-serving task did not finish");
            return None;
        }
    }

    tokio::task::spawn_blocking(farcooler_tailcat::conn_blob).await.ok()?.ok()
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
}
