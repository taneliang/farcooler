//! Whether this runner's sshd accepts a browser's passkey (ov-765), which
//! decides whether the daemon offers `passkey_keys`.
//!
//! A browser signs with WebAuthn, so its signatures are
//! `webauthn-sk-ecdsa-sha2-nistp256@openssh.com`, and OpenSSH verifies those
//! from 8.4 on. Ubuntu 20.04 ships 8.2 and RHEL 8 ships 8.0. On those, a line
//! the fence writes for a passkey admits nobody, so the runner doesn't offer
//! one: a client reads `passkey_keys` absent and says the runner needs
//! OpenSSH 8.4 or newer (R-88: no fallback key).
//!
//! The capability starts withheld (`capability::WITHHELD_UNTIL_OFFERED`), and
//! only `offer_passkey_keys` offers it. A version this can't read withholds
//! it too: a runner wrongly saying no costs a browser a message, and a runner
//! wrongly saying yes costs it a key that never logs in.

use std::path::{Path, PathBuf};
use std::time::Duration;

use farcooler_protocol::capability::{self, PASSKEY_KEYS};

/// The first OpenSSH that verifies `webauthn-sk` signatures.
pub const PASSKEY_FLOOR: (u32, u32) = (8, 4);

/// Where sshd usually is: Linux and macOS's own, then a local build's, then
/// Homebrew's. `FARCOOLER_SSHD` names another, and is how a test stubs it.
const CANDIDATES: &[&str] = &["/usr/sbin/sshd", "/usr/local/sbin/sshd", "/opt/homebrew/sbin/sshd"];

/// How long `sshd -V` may take. It prints one line and exits, so this only
/// matters for an sshd that hangs, which must not hold the daemon's start.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The `(major, minor)` in an OpenSSH version banner, such as
/// `OpenSSH_10.3p1, LibreSSL 3.3.6` or `OpenSSH_8.2p1 Ubuntu-4ubuntu0.5, …`.
///
/// `sshd -V` prints this on 9.x and later; older ones refuse `-V` as an
/// unknown option and print the same line above their usage, so one reader
/// takes both. `None` for anything else, such as a portable build that
/// renamed itself.
pub fn openssh_version(banner: &str) -> Option<(u32, u32)> {
    let start = banner.find("OpenSSH_")? + "OpenSSH_".len();
    let rest = &banner[start..];
    let (major, rest) = rest.split_once('.')?;
    let minor: String = rest.chars().take_while(char::is_ascii_digit).collect();
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// Whether the sshd that printed `banner` takes a browser's passkey.
pub fn takes_passkeys(banner: &str) -> bool {
    openssh_version(banner).is_some_and(|version| version >= PASSKEY_FLOOR)
}

/// Offer `passkey_keys` if this runner's sshd takes passkeys, and withhold
/// it otherwise. Called once as a serving process starts, before any hello.
pub async fn offer_passkey_keys() {
    let banner = match sshd() {
        Some(path) => probe(&path).await,
        None => None,
    };
    let takes = banner.as_deref().is_some_and(takes_passkeys);
    tracing::info!(sshd = banner.as_deref().unwrap_or("not found"), passkeys = takes, "read this runner's sshd");
    match takes {
        true => capability::offer(PASSKEY_KEYS),
        false => capability::withhold(PASSKEY_KEYS),
    }
}

/// The sshd to ask: `FARCOOLER_SSHD` if set, else the first candidate that
/// exists.
fn sshd() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("FARCOOLER_SSHD") {
        return Some(PathBuf::from(named));
    }
    CANDIDATES.iter().map(PathBuf::from).find(|path| path.exists())
}

/// `sshd -V`'s banner, from stderr where OpenSSH prints it, or stdout if a
/// build prints it there.
async fn probe(path: &Path) -> Option<String> {
    let run = tokio::process::Command::new(path)
        .arg("-V")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(PROBE_TIMEOUT, run).await.ok()?.ok()?;
    let text = format!("{}{}", String::from_utf8_lossy(&output.stderr), String::from_utf8_lossy(&output.stdout));
    let line = text.lines().find(|line| line.contains("OpenSSH_")).unwrap_or("").trim();
    Some(line.chars().take(200).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_is_read_from_every_banner_shape() {
        assert_eq!(openssh_version("OpenSSH_10.3p1, LibreSSL 3.3.6"), Some((10, 3)));
        assert_eq!(openssh_version("OpenSSH_8.2p1 Ubuntu-4ubuntu0.5, OpenSSL 1.1.1f  31 Mar 2020"), Some((8, 2)));
        assert_eq!(openssh_version("unknown option -- V\nOpenSSH_8.0p1, OpenSSL 1.1.1k  FIPS 25 Mar 2021"), Some((8, 0)));
        assert_eq!(openssh_version("OpenSSH_9.6"), Some((9, 6)));
        assert_eq!(openssh_version("Sun_SSH_1.1"), None);
        assert_eq!(openssh_version(""), None);
        assert_eq!(openssh_version("OpenSSH_x.y"), None);
    }

    /// 8.4 is the floor, compared as numbers: 10.0 is newer than 8.4, and
    /// 8.10 would be too.
    #[test]
    fn passkeys_need_openssh_8_4() {
        assert!(!takes_passkeys("OpenSSH_8.3p1"));
        assert!(!takes_passkeys("OpenSSH_8.2p1 Ubuntu-4ubuntu0.5"));
        assert!(!takes_passkeys("OpenSSH_7.9p1"));
        assert!(takes_passkeys("OpenSSH_8.4p1"));
        assert!(takes_passkeys("OpenSSH_8.10p1"));
        assert!(takes_passkeys("OpenSSH_10.0p2"));
        assert!(!takes_passkeys("not an sshd"));
    }

    /// `client.enroll` writes a passkey's line only while `passkey_keys` is
    /// offered, so a client that didn't check gets a refusal, not a key that
    /// never logs in. One test, because the capability is process-wide.
    #[tokio::test]
    async fn a_passkey_enrolls_only_while_passkeys_are_offered() {
        use farcooler_core::DomainError;
        use farcooler_protocol::v1::{ClientEnroll, Scope};
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let ssh = dir.path().join("home").join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        std::fs::set_permissions(&ssh, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();
        let keys = ssh.join("authorized_keys");
        let svc = crate::service::Service::open_in(state).await.unwrap().enrolling_into(keys.clone());
        let enroll = ClientEnroll {
            // A passkey for this build's app host, whichever channel it is.
            public_key: match farcooler_fence::app_host() {
                "app.farcooler.com" => "sk-ecdsa-sha2-nistp256@openssh.com AAAAInNrLWVjZHNhLXNoYTItbmlzdHAyNTZAb3BlbnNzaC5jb20AAAAIbmlzdHAyNTYAAABBBGsX0fLhLEJH+Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT+NC4v4af5uO5+tKfA+eFivOM1drMV7Oy7ZAaDe/UfUAAAARYXBwLmZhcmNvb2xlci5jb20=",
                _ => "sk-ecdsa-sha2-nistp256@openssh.com AAAAInNrLWVjZHNhLXNoYTItbmlzdHAyNTZAb3BlbnNzaC5jb20AAAAIbmlzdHAyNTYAAABBBGsX0fLhLEJH+Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT+NC4v4af5uO5+tKfA+eFivOM1drMV7Oy7ZAaDe/UfUAAAAYYXBwLmNhbmFyeS5mYXJjb29sZXIuY29t",
            }
            .into(),
            label: "Browser".into(),
            client_id: "farcooler-0123456789ab".into(),
            scope: Scope::Control as i32,
            shell_access: false,
            node_key: String::new(),
            for_teammate: false,
            step_up_key: None,
            teammate: None,
        };

        capability::withhold(PASSKEY_KEYS);
        let refused = crate::enrollment::enroll(&svc, &enroll).await;
        assert!(
            matches!(refused, Err(DomainError::InvalidArgument { what: "public_key algorithm" })),
            "{refused:?}"
        );
        let written = std::fs::read_to_string(&keys).unwrap_or_default();
        assert!(!written.contains("sk-ecdsa"), "a refused passkey was written: {written}");

        capability::offer(PASSKEY_KEYS);
        let enrolled = crate::enrollment::enroll(&svc, &enroll).await;
        capability::withhold(PASSKEY_KEYS);
        enrolled.expect("a passkey enrolls on an sshd that takes it");
        assert!(std::fs::read_to_string(&keys).unwrap().contains(" sk-ecdsa-sha2-nistp256@openssh.com AAAA"));
    }
}
