//! `farcooler client` — which devices may log in to this runner.
//!
//! The Mac's half of device onboarding. A phone reaches `client.list`,
//! `client.enroll` and `client.revoke` through the FFI in
//! `crates/client/src/ffi.rs`; a Mac reaches them through here, and the split is
//! not duplication for its own sake. The CLI runs real `ssh`, so it inherits the
//! agent, the passphrase prompt, `ProxyJump` and everything else a person has
//! already set up — and a phone has no `ssh` at all.
//!
//! **This module owns no rule about what may be written into `authorized_keys`.**
//! The daemon owns all of them, over `crates/daemon/src/fence.rs`, and that is
//! why `--shell-access --scope control` is sent rather than refused here: a rule
//! copied into a second place is a rule that drifts from the file that decides.
//! The one judgement made here is the scope WORD, and even that is made with the
//! daemon's own `fence::scope_from_word` rather than a second table.

use clap::Subcommand;
use farcooler_protocol::v1::{self as pb, request, result};

use crate::{Fallible, connect_to, expect_value, req, truncate, with};

#[derive(Subcommand)]
pub enum ClientCmd {
    /// Every key in this runner's `authorized_keys`, Far Cooler's and otherwise.
    List,
    /// Add a device's key to this runner's `authorized_keys`.
    ///
    /// One call writes ONE line. A Mac is TWO calls under one client id — the
    /// restricted line Far Cooler drives, then `--shell-access` for the plain one
    /// Zed, git and Terminal need — and they must be sequential, because the
    /// daemon reads the file outside its writer's lock and two at once lose a key.
    Enroll {
        /// One OpenSSH public key line, as the device generated it. Nothing of it
        /// survives into the file except the key material, re-encoded.
        #[arg(long)]
        key: String,
        /// A name for people. The comment written to the file is derived from it
        /// and suffixed with the fingerprint, never used as it arrived.
        #[arg(long)]
        label: String,
        /// The device this line names. A Mac sends the SAME id for both its keys,
        /// which is what lets one `client revoke` remove both.
        #[arg(long)]
        client_id: String,
        /// read, control, or host_admin.
        ///
        /// No default, on purpose. A key with no scope at all already means
        /// host_admin to sshd, so a default here would turn a flag somebody
        /// forgot into the whole runner.
        #[arg(long)]
        scope: String,
        /// Ask for the PLAIN line: an ordinary SSH key with a shell behind it.
        ///
        /// This is the key Zed, git and Terminal use, and the restricted line
        /// cannot be it — a forced command means sshd runs that program and only
        /// that program. A flag with no value so that its ABSENCE is the
        /// restricted line, which is what every caller written before this asked
        /// for. The daemon refuses it beside any scope but host_admin.
        #[arg(long)]
        shell_access: bool,
    },
    /// Remove a device's keys and close the sessions they were holding.
    ///
    /// EVERY line under that client id, which for a Mac is both of its keys, in
    /// one write — so this takes that Mac's ssh, git and Zed access to this
    /// runner away too, not only its Far Cooler access.
    Revoke {
        /// The device, as `client list` reports it.
        client_id: String,
    },
}

pub async fn client(runner: Option<&str>, cmd: ClientCmd, json: bool) -> Fallible {
    // The whole request assembled BEFORE the connection, the shape `layout` uses:
    // one call, and every judgement this module makes happens where it costs
    // nothing. A scope word this build does not have is then refused without an
    // ssh round trip, and the answer to a typo is these three words rather than
    // whatever the far end says about an enum it was handed.
    let outbound = match &cmd {
        ClientCmd::List => req("client.list"),
        ClientCmd::Enroll { key, label, client_id, scope, shell_access } => with(
            req("client.enroll"),
            request::Payload::ClientEnroll(pb::ClientEnroll {
                public_key: key.clone(),
                label: label.clone(),
                client_id: client_id.clone(),
                scope: parse_scope(scope)? as i32,
                shell_access: *shell_access,
            }),
        ),
        ClientCmd::Revoke { client_id } => with(
            req("client.revoke"),
            request::Payload::ClientRevoke(pb::ClientRevoke { client_id: client_id.clone() }),
        ),
    };

    let mut link = connect_to(runner).await?;
    // Asked before the call rather than after it fails. A runner too old to serve
    // any of the three would answer "unknown method", which is true and tells a
    // person nothing about what to do — and by then a ceremony has already had
    // somebody scan a code. The list arrives in the handshake, so asking is free.
    if !link.daemon_capabilities().iter().any(|c| c == farcooler_protocol::capability::ENROLLMENT) {
        return Err("that runner's Far Cooler is too old to manage device keys; update it".into());
    }
    let r = link.call(outbound).await?;

    match cmd {
        ClientCmd::List => {
            let result::Value::ClientList(list) = expect_value(r.value, "clients")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            if json {
                println!("{}", clients_json(&list.items));
                return Ok(());
            }
            if list.items.is_empty() {
                println!("no keys in this runner's authorized_keys");
                return Ok(());
            }
            for c in &list.items {
                print_client(c);
            }
        }

        ClientCmd::Enroll { .. } => {
            let result::Value::ClientEnroll(outcome) = expect_value(r.value, "enrollment")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "client": outcome.client.as_ref().map(client_json),
                        // Its own field rather than an error, in the same shape
                        // the phones' FFI answers with — see `enroll_client` in
                        // `crates/client/src/session.rs`. One decoder, two
                        // transports.
                        "alreadyEnrolled": outcome.already_enrolled,
                    })
                );
                return Ok(());
            }
            // Whatever the file says, which when `already_enrolled` is set is the
            // grant the device HAS rather than the one that was asked for.
            let what = outcome.client.as_ref().map(describe).unwrap_or_else(|| "this key".into());
            if outcome.already_enrolled {
                // Exit 0, and this is the whole point of saying it out loud: it
                // is the ordinary outcome of enrolling a Mac on itself, whose own
                // shell key is usually already in that file, and of a ceremony
                // offered a runner the device can already reach. Treating it as a
                // failure would send somebody looking for a problem that is not
                // there.
                println!("already enrolled · {what} · nothing was written");
            } else {
                println!("enrolled {what}");
            }
        }

        ClientCmd::Revoke { client_id } => {
            // What is LEFT, read back out of the file by the daemon after writing
            // it: what `authorized_keys` now says is the only claim worth making
            // about who may log in.
            let result::Value::ClientList(list) = expect_value(r.value, "clients")? else {
                return Err("the daemon returned the wrong resource".into());
            };
            if json {
                println!("{}", clients_json(&list.items));
                return Ok(());
            }
            println!(
                "revoked {client_id} · {} key{} left",
                list.items.len(),
                if list.items.len() == 1 { "" } else { "s" }
            );
            // Said every time, because it is the part a person acts on and the
            // one thing a revocation cannot do. sshd reads `authorized_keys` at
            // authentication and never again: the line is gone for the next login
            // and does nothing to a session the device already holds — and a shell
            // that is still open can put both lines back.
            println!("Any shell that device already had open stays open. Audit the runner.");
        }
    }
    Ok(())
}

/// A scope word, refused rather than resolved.
///
/// `fence::scope_from_word` is the daemon's own parser and the one that will read
/// the line back out of the file, so this cannot come to accept a word the runner
/// does not — and cannot round a misspelling UP to host_admin, which is what an
/// unscoped line already means to sshd and therefore what a default here would
/// hand out by accident.
fn parse_scope(word: &str) -> Result<pb::Scope, Box<dyn std::error::Error>> {
    farcooler_daemon::fence::scope_from_word(word).ok_or_else(|| {
        format!("unknown scope \"{word}\"; a device is enrolled at read, control or host_admin")
            .into()
    })
}

/// Enrolled devices, in the shape three apps already decode.
///
/// The same keys `enrolled_json` produces for the phones in
/// `crates/client/src/session.rs`, deliberately: the Mac drives runners through
/// this CLI rather than through that FFI, and two spellings of one answer is one
/// of them being wrong the day a field is added.
fn clients_json(items: &[pb::EnrolledClient]) -> serde_json::Value {
    serde_json::json!({ "clients": items.iter().map(client_json).collect::<Vec<_>>() })
}

fn client_json(c: &pb::EnrolledClient) -> serde_json::Value {
    serde_json::json!({
        "clientId": c.client_id,
        "fingerprint": c.fingerprint,
        "label": c.label,
        "scope": scope_label(c.scope),
        // Which local account's `authorized_keys` this line was read from.
        // Nothing in the line names it — the file's location does — so this is
        // the daemon's answer and cannot be derived here.
        "account": c.account,
        // 0 for unknown, which is the ordinary answer: `authorized_keys` records
        // no time, so only the reply to an enrollment that just happened has one.
        "enrolledAt": c.enrolled_at,
        // Far Cooler did not write this line. Reported so a person can see it is
        // there, and never touched.
        "foreign": c.foreign,
        // The device's PLAIN line, and the ONLY thing that tells the two rows of
        // a Mac apart: both carry the same client id and both are ours. Without
        // it a client draws one of them twice, and the removal copy's promise —
        // that this takes the Mac's ssh, git and Zed access away too — lands on
        // whichever row happened to sort first.
        "shellAccess": c.shell_access,
    })
}

/// One device, over two lines.
///
/// Two rather than one because everything identifying is long: a client id is a
/// UUID and a fingerprint is fifty characters, and a single row carrying both
/// wraps in any terminal anybody actually has. The handles come first — the
/// client id is what `client revoke` takes — and the fingerprint sits under them,
/// which is the only identity a foreign line has at all.
fn print_client(c: &pb::EnrolledClient) {
    println!(
        "{:22}  {:10}  {:11}  {}",
        truncate(if c.label.is_empty() { "(no name)" } else { &c.label }, 22),
        grant_label(c),
        scope_label(c.scope),
        if c.client_id.is_empty() { "-" } else { &c.client_id },
    );
    println!("  {}  {}", c.fingerprint, c.account);
}

/// What kind of line this is, which is the fact a person needs first.
fn grant_label(c: &pb::EnrolledClient) -> &'static str {
    if c.foreign {
        // Named rather than left as a blank column. "Somebody added this by hand"
        // is the most important thing about a row in this file, and Far Cooler
        // will not touch it — including to revoke it.
        "FOREIGN"
    } else if c.shell_access {
        "shell"
    } else {
        "farcooler"
    }
}

/// One enrolled line, in a sentence.
fn describe(c: &pb::EnrolledClient) -> String {
    let label = if c.label.is_empty() { "this key" } else { &c.label };
    if c.shell_access {
        // Spelled out rather than called "shell": what this line grants is a
        // login, and the three things somebody wanted it for are the reason they
        // are reading this.
        format!("{label} · shell access (ssh, git and Zed)")
    } else {
        format!("{label} · {} · Far Cooler only", scope_label(c.scope))
    }
}

/// A scope as `authorized_keys` spells it.
///
/// `unspecified` is reported as itself rather than rounded to anything: it is what
/// a foreign line grants, and what a line of ours whose scope word this build does
/// not have grants — which is nothing, because the daemon serving it refuses the
/// word too. Deliberately NOT `fence::scope_word`, which rounds an absent scope UP
/// to host_admin. That asymmetry is right in the writer, where an unrestricted line
/// is what sshd already sees, and wrong here, where it would tell a person a device
/// has access it does not have. `scope_label` in `crates/client/src/session.rs`
/// makes the same choice for the phones.
fn scope_label(scope: i32) -> &'static str {
    match pb::Scope::try_from(scope) {
        Ok(pb::Scope::Read) => "read",
        Ok(pb::Scope::Control) => "control",
        Ok(pb::Scope::HostAdmin) => "host_admin",
        _ => "unspecified",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(client_id: &str, shell_access: bool, foreign: bool, scope: pb::Scope) -> pb::EnrolledClient {
        pb::EnrolledClient {
            client_id: client_id.to_string(),
            fingerprint: "SHA256:1YyJhFq0y2s9v1s0kQ4c1kFq0y2s9v1s0kQ4c1kFq0y".to_string(),
            label: "MacBook Air".to_string(),
            scope: scope as i32,
            account: "you".to_string(),
            enrolled_at: 0,
            foreign,
            shell_access,
        }
    }

    /// The keys the Mac app decodes, on the shape `client list` and
    /// `client revoke` both answer with.
    #[test]
    fn the_json_carries_every_field_of_the_wire_message() {
        let json = clients_json(&[line("device-1", false, false, pb::Scope::Control)]);
        let one = &json["clients"][0];
        for key in [
            "clientId",
            "fingerprint",
            "label",
            "scope",
            "account",
            "enrolledAt",
            "foreign",
            "shellAccess",
        ] {
            assert!(!one[key].is_null(), "{key} is missing from a listed client");
        }
        assert_eq!(one["scope"], "control", "the scope is a word, not a number");
    }

    /// **Both of a Mac's rows are distinguishable.** They carry the same client
    /// id and the same label, so `shellAccess` is the only thing that tells them
    /// apart — and a client that could not would draw one of them twice and
    /// attach the removal warning to whichever sorted first.
    #[test]
    fn a_macs_two_lines_are_told_apart() {
        // The plain line carries no scope, which is what the daemon reports for
        // it: it hands out no Far Cooler grant at all.
        let plain = line("device-1", true, false, pb::Scope::Unspecified);
        let restricted = line("device-1", false, false, pb::Scope::HostAdmin);

        let json = clients_json(&[plain, restricted]);
        let a = &json["clients"][0];
        let b = &json["clients"][1];
        assert_eq!(a["clientId"], b["clientId"], "a Mac's two lines are one device");
        assert_ne!(a, b, "the two rows of a Mac are the same JSON object");
        assert_eq!(a["shellAccess"], true);
        assert_eq!(b["shellAccess"], false);
        assert_eq!(a["scope"], "unspecified", "a plain line grants no Far Cooler scope");
    }

    /// A foreign line reports what it is, and is never something to revoke.
    #[test]
    fn a_foreign_line_is_reported_as_one() {
        let mut stranger = line("", false, true, pb::Scope::Unspecified);
        stranger.label = "ada@laptop".to_string();
        let json = client_json(&stranger);
        assert_eq!(json["foreign"], true);
        assert_eq!(json["clientId"], "", "a foreign line names no device");
        assert_eq!(json["scope"], "unspecified");
        assert_eq!(grant_label(&stranger), "FOREIGN");
    }

    /// An enrollment's reply is the shape the FFI answers with, `already_enrolled`
    /// included — and it is a field rather than an error.
    #[test]
    fn an_enrollment_reply_names_already_enrolled() {
        let outcome = pb::ClientEnrollResult {
            client: Some(line("device-1", false, false, pb::Scope::Control)),
            already_enrolled: true,
        };
        let json = serde_json::json!({
            "client": outcome.client.as_ref().map(client_json),
            "alreadyEnrolled": outcome.already_enrolled,
        });
        assert_eq!(json["alreadyEnrolled"], true);
        assert_eq!(json["client"]["clientId"], "device-1");
        // Round-tripped, because what this has to survive is the Mac app's
        // decoder rather than this assertion.
        let text = json.to_string();
        assert!(serde_json::from_str::<serde_json::Value>(&text).is_ok(), "not parseable: {text}");
    }

    /// The three words, and nothing rounded up.
    #[test]
    fn only_the_three_scope_words_are_accepted() {
        for word in ["read", "control", "host_admin"] {
            let scope = parse_scope(word).expect("a scope word this build has");
            assert_eq!(scope_label(scope as i32), word, "the two directions disagree about {word}");
        }
        // Not "close enough to host_admin". Rounding a typo up is privilege
        // escalation by misspelling.
        for word in ["hostadmin", "host-admin", "admin", "unspecified", ""] {
            assert!(parse_scope(word).is_err(), "{word:?} was accepted as a scope");
        }
    }
}
