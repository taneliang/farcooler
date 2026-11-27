//! `farcooler join <code>`: add this runner to a browser and its account, from
//! this runner's own terminal (ov-1490).
//!
//! The browser's Add Runner screen shows the command, with its own offer and a
//! relay ticket inside the code. This is also how a person with no enrolled
//! device left gets back in: being at a terminal on the runner, as its Unix
//! user, is what it asks for, and the runner refuses it from a pane, where an
//! agent could type it.
//!
//! The order: the person compares the browser's passkey and types yes here;
//! the runner enrolls it over its own socket; the sealed reply goes to the
//! relay under the ticket; the browser opens it and shows a code, which must
//! match the one printed here; only once the person approves it there does the
//! relay take this runner into the account.

use std::io::{BufRead, Write};
use std::time::Duration;

use farcooler_ceremony::{self as ceremony, JoinCode};
use farcooler_daemon::join_relay;
use farcooler_daemon::runner_key::{Action, RunnerKey};
use farcooler_daemon_contracts::paths;
use farcooler_protocol::v1::Scope;
use farcooler_transport::Peer;
use serde_json::{Value, json};

use crate::Fallible;

#[derive(clap::Args)]
pub struct Args {
    /// The code from the browser's Add Runner screen.
    code: String,
    /// How the runner appears in the browser. Default: this computer's name.
    #[arg(long)]
    label: Option<String>,
    /// The SSH login a browser signs in as. Default: the user running this.
    #[arg(long)]
    user: Option<String>,
    /// This computer's SSH host public key file.
    #[arg(long, default_value = "/etc/ssh/ssh_host_ed25519_key.pub")]
    host_key: std::path::PathBuf,
    /// The relay that issued the code. Default: this build's own relay.
    #[arg(long)]
    relay: Option<String>,
}

/// How often, and how many times, the relay is asked whether the person has
/// approved this runner in the browser: two minutes in all.
const POLL: Duration = Duration::from_secs(3);
const POLLS: u32 = 40;

const EXPIRED: &str = "This command has expired or was already used. Copy the new one from your browser and run it.";

pub async fn join(remote: Option<&str>, args: Args) -> Fallible {
    if remote.is_some() {
        return Err("Run this command in a terminal on the runner itself, without --runner.".into());
    }
    let (code, offer) = JoinCode::decode(&args.code)?;
    let relay = join_relay::relay_origin(&args.relay.unwrap_or_else(farcooler_daemon::push::default_relay))?;
    let host_key = std::fs::read_to_string(&args.host_key).ok().and_then(|line| ceremony::fingerprint(line.trim())).ok_or_else(|| {
        format!("Far Cooler couldn't read an SSH host key at {}. Turn on SSH on this computer, or name the key with --host-key.", args.host_key.display())
    })?;
    let user = match args.user.or_else(|| std::env::var("USER").ok()).filter(|u| !u.trim().is_empty()) {
        Some(user) => user,
        None => return Err("Name the SSH login with --user.".into()),
    };
    let label = args.label.filter(|l| !l.trim().is_empty()).unwrap_or_else(computer_name);

    // Asked before anything is written here, so an expired command adds nothing.
    let (status, _) = join_relay::post(&relay, "check", &json!({ "ticket": code.ticket })).await?;
    if status != 200 {
        return Err(EXPIRED.into());
    }

    eprintln!("This lets {} open this runner ({label}) with Control: its terminals, and the work in them.", offer.name.escape_default());
    eprintln!("The browser's passkey: {}", ceremony::fingerprint(&offer.key_a).unwrap_or_default());
    eprint!("Check that it matches the browser. Type yes to continue: ");
    std::io::stderr().flush()?;
    if std::io::stdin().lock().lines().next().transpose()?.as_deref().map(str::trim) != Some("yes") {
        return Err("Nothing was added.".into());
    }

    let (install, runner) = crate::browser::grant(&offer, &label, &user, &host_key).await?;
    let (sealed, compare) = ceremony::seal_reply(&offer, &ceremony::manifest(&offer, vec![runner]))?;
    let (status, answer) = join_relay::post(&relay, "reply", &json!({ "ticket": code.ticket, "sealed": sealed })).await?;
    let claim = match (status, answer.get("claim").and_then(Value::as_str)) {
        (200, Some(claim)) => claim.to_string(),
        _ => return Err(EXPIRED.into()),
    };
    eprintln!("\nYour browser now shows a code. Check that it's {compare}, then select They Match there.");
    eprintln!("Waiting for you to approve this runner in the browser…");

    let dir = paths::ensure_runtime_dir()?;
    let key = RunnerKey::load_or_create_in(&dir)?;
    let local = Peer { client_id: None, scope: Scope::HostAdmin };
    for _ in 0..POLLS {
        tokio::time::sleep(POLL).await;
        // Signed afresh each time: the relay takes a statement for a few minutes only.
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64;
        let proof = key.sign(&local, Action::Register, &offer.account, &code.team, &install, &relay, now)?;
        let body = json!({ "claim": claim, "install": install, "name": label, "runnerKey": key.public_line(), "issuedAt": now, "proof": proof });
        let Ok((status, answer)) = join_relay::post(&relay, "register", &body).await else { continue };
        match (status, answer.get("error").and_then(Value::as_str), answer.get("token").and_then(Value::as_str)) {
            (200, _, Some(token)) => {
                farcooler_daemon::push::Pairing { relay, token: token.to_string() }.save_in(&dir)?;
                println!("Added. {label} is in your account, and your browser can open it now.");
                return Ok(());
            }
            (409, Some("waiting"), _) => {}
            (409, Some("registered" | "elsewhere"), _) => {
                return Err("Your browser can open this runner, but another account or team already lists it. To move it here, run `farcooler runner claim` and see docs/runners.md.".into());
            }
            (429, ..) | (500.., ..) => {}
            _ => break,
        }
    }
    Err(format!(
        "The browser didn't approve this runner. If its code didn't match {compare}, remove the browser: `farcooler client revoke {}`. Otherwise copy a new command and run it again.",
        ceremony::client_id(&offer.key_a).unwrap_or_default()
    )
    .into())
}

/// This computer's name as its owner would say it.
fn computer_name() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|name| name.trim().trim_end_matches(".local").to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Runner".to_string())
}
