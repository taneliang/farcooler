//! A browser's passkey as Key A (ov-765): which `sk-ecdsa` keys the fence
//! writes a line for.
//!
//! A WebAuthn passkey reaches OpenSSH as an `sk-ecdsa-sha2-nistp256@openssh.com`
//! key whose application string is the passkey's RP ID (PROTOCOL.u2f). sshd
//! checks every signature against SHA-256 of that string, which is the
//! authenticator's rpIdHash. So the application decides which web pages can
//! ask this key to sign, and that is what the fence checks.
//!
//! **Only this channel's app host** (owner, R-88; the channel tie is the
//! coordinator's reversible ruling, fix round 1). A canary build admits
//! `app.canary.farcooler.com` alone, and stable and preview `app.farcooler.com`
//! alone, the host each channel's web client serves from, which is also what
//! the relay checks (slice 3). So script running on the canary origin, the
//! faster and less-held deploy, can't make a passkey sign for a stable runner.
//! A local build is a developer's, who works against canary, so it takes
//! canary's. Never `farcooler.com`: a passkey
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

use farcooler_protocol::Channel;

use crate::Grant;

/// The RP ID a passkey's line may carry on a build of `channel`: the host
/// that channel's web client is served from (`Channel::app_host`, the
/// spelling the browser's client signs with too).
pub fn app_host_for(channel: Channel) -> &'static str {
    channel.app_host()
}

/// This build's: `app_host_for(farcooler_protocol::CHANNEL)`.
pub fn app_host() -> &'static str {
    app_host_for(farcooler_protocol::CHANNEL)
}

/// Whether `render` and `with_node_key` may write a line for `key`.
///
/// Ed25519 as before, on either kind of line. A passkey only on Key A, and
/// only for this channel's app host.
pub(crate) fn admits(key: &PublicKey, grant: Grant) -> bool {
    admits_on(key, grant, farcooler_protocol::CHANNEL)
}

/// `admits`, as a build of `channel` would answer, so tests can ask both.
fn admits_on(key: &PublicKey, grant: Grant, channel: Channel) -> bool {
    match key.algorithm() {
        Algorithm::Ed25519 => true,
        Algorithm::SkEcdsaSha2NistP256 => {
            grant == Grant::FarCooler
                && key.key_data().sk_ecdsa_p256().is_some_and(|sk| sk.application() == app_host_for(channel))
        }
        _ => false,
    }
}

/// Whether a bare line holding `key` can be one of our Key B lines: only
/// Ed25519, since `render` writes no other kind there. A bare ECDSA or sk line
/// carrying our shell comment is somebody's hand-added key, and stays
/// foreign. Before the fence's `ecdsa` feature (ov-765) the shipped daemon
/// couldn't decode those lines at all, so they were foreign by accident.
pub(crate) fn can_be_key_b(key: &PublicKey) -> bool {
    key.algorithm() == Algorithm::Ed25519
}

/// Whether `entry`'s line holds an Ed25519 key, read back out of the line
/// itself. `client.rekey` moves only such a line (fix round 1): a passkey's
/// line, which needs a touch on every login and can't be exported, must
/// never be swapped for a software key that needs neither.
pub fn holds_ed25519(entry: &crate::Entry) -> bool {
    let key = match crate::split_options(&entry.line) {
        Some((_, rest)) => rest,
        None => entry.line.as_str(),
    };
    PublicKey::from_openssh(key).is_ok_and(|key| key.algorithm() == Algorithm::Ed25519)
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
        let key = PublicKey::from_openssh(&sk(app_host())).expect("an sk-ecdsa line parses");
        assert_eq!(key.algorithm(), Algorithm::SkEcdsaSha2NistP256);
        assert!(admits(&key, Grant::FarCooler));
    }

    /// A passkey for this channel's app host renders as Key A, and the line is
    /// still one this crate built: options of ours, the key re-encoded, our
    /// comment.
    #[test]
    fn a_passkey_for_this_channels_host_renders_as_key_a() {
        let host = app_host();
        let line = key_a(&sk(host)).unwrap_or_else(|r| panic!("{host}: {r:?}"));
        assert!(line.starts_with("restrict,command=\""), "{line}");
        assert!(line.contains(" sk-ecdsa-sha2-nistp256@openssh.com AAAA"), "{line}");
        assert!(!line.contains("webauthn@browser"), "the received comment survived: {line}");
        // `restrict` without `no-touch-required`: sshd demands the
        // authenticator's user-presence flag on every signature.
        assert!(!line.contains("no-touch-required"), "{line}");
    }

    /// The line reads back as ours, with its id and scope, so `client.list`
    /// and revocation see a passkey device like any other.
    #[test]
    fn a_passkey_line_reads_back_as_ours() {
        let line = key_a(&sk(app_host())).expect("render");
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

    /// Each channel admits its own app host and refuses the other's: a canary
    /// passkey never signs for a stable runner, nor the reverse.
    #[test]
    fn each_channel_admits_its_own_app_host_alone() {
        let canary = PublicKey::from_openssh(&sk("app.canary.farcooler.com")).unwrap();
        let stable = PublicKey::from_openssh(&sk("app.farcooler.com")).unwrap();
        for (channel, own, other) in [
            (Channel::Canary, &canary, &stable),
            (Channel::Local, &canary, &stable),
            (Channel::Stable, &stable, &canary),
            (Channel::Preview, &stable, &canary),
        ] {
            assert!(admits_on(own, Grant::FarCooler, channel), "{channel:?} refused its own host");
            assert!(!admits_on(other, Grant::FarCooler, channel), "{channel:?} took the other channel's host");
        }
        // And this build, through `render`: the other host is refused.
        let other = if app_host() == "app.farcooler.com" { "app.canary.farcooler.com" } else { "app.farcooler.com" };
        assert_eq!(key_a(&sk(other)), Err(Rejected::Algorithm));
    }

    /// A bare line carrying our shell comment is Key B only when its key is
    /// Ed25519: a bare sk or ECDSA line with that comment is somebody's own
    /// key, and is read as foreign, not adopted as a device of ours.
    #[test]
    fn only_an_ed25519_bare_line_is_read_as_key_b() {
        let comment = crate::shell_comment("MacBook", "SHA256:t7xq9vd8", "mac-1");
        let ed25519 = format!("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA {comment}");
        let ours = entry_from_line(&ed25519);
        assert!(ours.shell_access && ours.client_id == "mac-1", "{ours:?}");
        let sk_line = sk(app_host()).replace("webauthn@browser", &comment);
        for line in [sk_line, format!("{} {comment}", PLAIN_ECDSA.trim_end_matches(" x"))] {
            let entry = entry_from_line(&line);
            assert!(!entry.shell_access, "adopted as Key B: {line}");
            assert!(entry.client_id.is_empty(), "read as a device of ours: {line}");
            assert!(entry.fingerprint.starts_with("SHA256:"), "still reported by fingerprint: {line}");
        }
    }

    /// Read back off the line: an Ed25519 Key A line is, a passkey's isn't,
    /// and neither is a line that doesn't parse.
    #[test]
    fn holds_ed25519_reads_the_lines_own_key() {
        let ed25519 = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        assert!(crate::holds_ed25519(&entry_from_line(&key_a(ed25519).unwrap())));
        assert!(!crate::holds_ed25519(&entry_from_line(&key_a(&sk(app_host())).unwrap())));
        assert!(!crate::holds_ed25519(&entry_from_line("restrict,command=\"x\" garbage")));
    }

    /// A browser never gets a shell, even at host_admin.
    #[test]
    fn a_passkey_never_gets_a_plain_line() {
        let shell = render(
            &sk(app_host()),
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
        let line = key_a(&sk(app_host())).expect("render");
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
        let good = key_a(&sk(app_host())).expect("render");
        let (good_blob, bad_blob) = (blob(&sk(app_host())), blob(&sk("farcooler.com")));
        let entry = entry_from_line(&good.replace(&good_blob, &bad_blob));
        let node = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        assert_eq!(with_node_key(&entry, node), Err(Rejected::Algorithm));
    }

    fn blob(line: &str) -> String {
        line.split_whitespace().nth(1).expect("a key has a blob").to_string()
    }

}
