//! A browser's passkey as Key A (ov-765): which `sk-ecdsa` keys the fence
//! writes a line for.
//!
//! A WebAuthn passkey reaches OpenSSH as an `sk-ecdsa-sha2-nistp256@openssh.com`
//! key whose application string is the passkey's RP ID (PROTOCOL.u2f). sshd
//! checks every signature against SHA-256 of that string, which is the
//! authenticator's rpIdHash. So the application decides which web pages can
//! ask this key to sign, and that is what the fence checks.
//!
//! **Only the exact app hosts** (owner, R-88). Never `farcooler.com`: a passkey
//! scoped to the parent domain could be asked to sign by any page on any
//! `*.farcooler.com` host, including www, which carries a third-party beacon
//! script. Not `ssh:` either, a hardware key's default: nothing in Far Cooler
//! makes one, so admitting it is a decision for the day something does.
//!
//! **Key A only.** A passkey is a browser's key, and a browser never gets a
//! shell: a plain line (`Grant::Shell`) stays ed25519.
//!
//! sshd needs OpenSSH 8.4 or newer to accept the `webauthn-sk` signatures a
//! browser makes. The fence does not ask: a line for a passkey on an older
//! runner admits nobody, and the daemon offers `passkey_keys` only on 8.4+, so
//! a client never asks it to write one there.

use ssh_key::{Algorithm, PublicKey};

use crate::Grant;

/// The RP IDs a passkey's line may carry: the stable app's host and canary's.
pub const APPLICATIONS: &[&str] = &["app.farcooler.com", "app.canary.farcooler.com"];

/// Whether `render` and `with_node_key` may write a line for `key`.
///
/// Ed25519 as before, on either kind of line. A passkey only on Key A, and
/// only for an app host.
pub(crate) fn admits(key: &PublicKey, grant: Grant) -> bool {
    match key.algorithm() {
        Algorithm::Ed25519 => true,
        Algorithm::SkEcdsaSha2NistP256 => {
            grant == Grant::FarCooler
                && key.key_data().sk_ecdsa_p256().is_some_and(|sk| APPLICATIONS.contains(&sk.application()))
        }
        _ => false,
    }
}

/// Whether the fence would write a Key A line for `key`: what a phone may
/// certify as another device's Key A (`farcooler_client::device_keys`).
pub fn admits_as_key_a(key: &PublicKey) -> bool {
    admits(key, Grant::FarCooler)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{entry_from_line, render, with_node_key, Rejected};
    use farcooler_protocol::v1::Scope;

    /// The base64 of an sk-ecdsa key for `application`, its point the P-256
    /// generator: a real point, so it parses as a key and not as nothing.
    fn sk(application: &str) -> String {
        let blob = match application {
            "app.canary.farcooler.com" => "AAAAInNrLWVjZHNhLXNoYTItbmlzdHAyNTZAb3BlbnNzaC5jb20AAAAIbmlzdHAyNTYAAABBBGsX0fLhLEJH+Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT+NC4v4af5uO5+tKfA+eFivOM1drMV7Oy7ZAaDe/UfUAAAAYYXBwLmNhbmFyeS5mYXJjb29sZXIuY29t",
            "app.farcooler.com" => "AAAAInNrLWVjZHNhLXNoYTItbmlzdHAyNTZAb3BlbnNzaC5jb20AAAAIbmlzdHAyNTYAAABBBGsX0fLhLEJH+Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT+NC4v4af5uO5+tKfA+eFivOM1drMV7Oy7ZAaDe/UfUAAAARYXBwLmZhcmNvb2xlci5jb20=",
            "farcooler.com" => "AAAAInNrLWVjZHNhLXNoYTItbmlzdHAyNTZAb3BlbnNzaC5jb20AAAAIbmlzdHAyNTYAAABBBGsX0fLhLEJH+Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT+NC4v4af5uO5+tKfA+eFivOM1drMV7Oy7ZAaDe/UfUAAAANZmFyY29vbGVyLmNvbQ==",
            "ssh:" => "AAAAInNrLWVjZHNhLXNoYTItbmlzdHAyNTZAb3BlbnNzaC5jb20AAAAIbmlzdHAyNTYAAABBBGsX0fLhLEJH+Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT+NC4v4af5uO5+tKfA+eFivOM1drMV7Oy7ZAaDe/UfUAAAAEc3NoOg==",
            other => panic!("no fixture for {other}"),
        };
        format!("sk-ecdsa-sha2-nistp256@openssh.com {blob} webauthn@browser")
    }

    /// The same point as a plain `ecdsa-sha2-nistp256` key, which no passkey is.
    const PLAIN_ECDSA: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBGsX0fLhLEJH+Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT+NC4v4af5uO5+tKfA+eFivOM1drMV7Oy7ZAaDe/UfU= x";

    /// A real, tiny RSA key, as in the crate's own tests.
    const RSA: &str = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAAAIH+rq6urq6urq6urq6urq6urq6urq6urq6urq6urq6sB x";

    fn key_a(received: &str) -> Result<String, Rejected> {
        render(received, "Browser", "farcooler-0123456789ab", Scope::Control, Grant::FarCooler, None)
    }

    /// An sk line parses at all. This is the test that needs `ssh-key`'s
    /// `ecdsa` feature on the fence's own dependency: without it the crate
    /// decodes no sk key, and `-p farcooler-fence` alone is that build.
    #[test]
    fn an_sk_line_parses_as_a_passkey_for_its_host() {
        let key = PublicKey::from_openssh(&sk("app.canary.farcooler.com")).expect("an sk-ecdsa line parses");
        assert_eq!(key.algorithm(), Algorithm::SkEcdsaSha2NistP256);
        assert!(admits(&key, Grant::FarCooler));
    }

    /// A passkey for either app host renders as Key A, and the line is still
    /// one this crate built: options of ours, the key re-encoded, our comment.
    #[test]
    fn a_passkey_for_an_app_host_renders_as_key_a() {
        for host in APPLICATIONS {
            let line = key_a(&sk(host)).unwrap_or_else(|r| panic!("{host}: {r:?}"));
            assert!(line.starts_with("restrict,command=\""), "{line}");
            assert!(line.contains(" sk-ecdsa-sha2-nistp256@openssh.com AAAA"), "{line}");
            assert!(!line.contains("webauthn@browser"), "the received comment survived: {line}");
            // `restrict` without `no-touch-required`: sshd demands the
            // authenticator's user-presence flag on every signature.
            assert!(!line.contains("no-touch-required"), "{line}");
        }
    }

    /// The line reads back as ours, with its id and scope, so `client.list`
    /// and revocation see a passkey device like any other.
    #[test]
    fn a_passkey_line_reads_back_as_ours() {
        let line = key_a(&sk("app.farcooler.com")).expect("render");
        let entry = entry_from_line(&line);
        assert_eq!(entry.client_id, "farcooler-0123456789ab");
        assert_eq!(entry.scope, Scope::Control);
        assert!(!entry.shell_access);
    }

    /// The RP ID is the exact app host: the parent domain, which every
    /// `*.farcooler.com` page could make sign, and a hardware key's `ssh:`
    /// are each refused.
    #[test]
    fn a_passkey_for_any_other_host_is_refused() {
        for application in ["farcooler.com", "ssh:"] {
            assert_eq!(key_a(&sk(application)), Err(Rejected::Algorithm), "{application}");
        }
    }

    /// A browser never gets a shell, even at host_admin.
    #[test]
    fn a_passkey_never_gets_a_plain_line() {
        let shell = render(
            &sk("app.canary.farcooler.com"),
            "Browser",
            "farcooler-0123456789ab",
            Scope::HostAdmin,
            Grant::Shell,
            None,
        );
        assert_eq!(shell, Err(Rejected::Algorithm));
    }

    /// Nothing else that is not ed25519 came in with passkeys.
    #[test]
    fn plain_ecdsa_and_rsa_are_still_refused() {
        assert_eq!(key_a(PLAIN_ECDSA), Err(Rejected::Algorithm));
        assert_eq!(key_a(RSA), Err(Rejected::Algorithm));
    }

    /// A passkey device that is already enrolled can register a node key,
    /// as an ed25519 one can; the line keeps its key, id and scope.
    #[test]
    fn a_passkey_line_takes_a_node_key() {
        let line = key_a(&sk("app.canary.farcooler.com")).expect("render");
        let entry = entry_from_line(&line);
        let node = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let moved = with_node_key(&entry, node).expect("a passkey line takes a node key");
        let back = entry_from_line(&moved);
        assert_eq!(back.fingerprint, entry.fingerprint);
        assert_eq!(back.client_id, entry.client_id);
        assert_eq!(back.node_key, node);
    }

    /// A line whose passkey is for another host is not rewritten, even though
    /// something put it in the fence: `with_node_key` makes the same check.
    #[test]
    fn a_line_for_another_host_takes_no_node_key() {
        let good = key_a(&sk("app.farcooler.com")).expect("render");
        let (good_blob, bad_blob) = (blob(&sk("app.farcooler.com")), blob(&sk("farcooler.com")));
        let entry = entry_from_line(&good.replace(&good_blob, &bad_blob));
        let node = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        assert_eq!(with_node_key(&entry, node), Err(Rejected::Algorithm));
    }

    fn blob(line: &str) -> String {
        line.split_whitespace().nth(1).expect("a key has a blob").to_string()
    }

}
