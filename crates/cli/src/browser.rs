//! Approve a browser from an existing SSH login on the runner.
use std::io::{BufRead, Write};

use farcooler_ceremony::{self as ceremony, Offer, Reach, RunnerEntry};
use farcooler_daemon_contracts::paths;
use farcooler_protocol::v1::{self as pb, request, result};

use crate::{Fallible, connect_to, expect_value, req, with};

#[derive(clap::Args)]
pub struct Args {
    /// Account ID displayed beside the browser's offer.
    #[arg(long)]
    account: String,
    /// Runner's SSH host public key from your trusted SSH configuration.
    #[arg(long)]
    host_key: String,
    /// SSH login on this runner.
    #[arg(long)]
    user: String,
    /// How the runner should appear in the browser.
    #[arg(long)]
    label: String,
}

pub async fn enroll(remote: Option<&str>, args: Args) -> Fallible {
    if remote.is_some() {
        return Err("Run this command on the runner over your existing SSH login.".into());
    }
    let host_key =
        ceremony::fingerprint(&args.host_key).ok_or("Expected a trusted SSH host public key.")?;
    eprintln!("Paste the browser offer on one line, then press Enter.");
    let mut lines = std::io::stdin().lock().lines();
    let encoded = lines
        .next()
        .transpose()?
        .ok_or("No browser offer received.")?;
    let offer = ceremony::accept_offer(encoded.trim(), &args.account)?;
    let request = enrollment(&offer)?;
    eprint!(
        "Grant {} control on {}? Compare {} with the browser first. Type yes to continue: ",
        offer.name.escape_default(),
        args.label,
        ceremony::fingerprint(&offer.key_a).unwrap_or_default()
    );
    std::io::stderr().flush()?;
    if lines.next().transpose()?.as_deref() != Some("yes") {
        return Err("Browser enrollment cancelled.".into());
    }
    drop(lines);

    let install = paths::load_or_create_install_id_in(&paths::ensure_runtime_dir()?)?;
    let mut link = connect_to(None).await?;
    if !link
        .daemon_capabilities()
        .iter()
        .any(|capability| capability == farcooler_protocol::capability::PASSKEY_KEYS)
    {
        return Err("This runner does not accept web passkeys. Check that the runner and OpenSSH are up to date.".into());
    }
    let result::Value::ClientEnroll(grant) = expect_value(link.call(request).await?.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    if grant.conn_blob.is_empty() {
        return Err(
            "This runner returned no tunnel token. Check runner tunnel setup and try again.".into(),
        );
    }
    let runner = RunnerEntry {
        id: farcooler_daemon::service::stable_host_id(&install).to_string(),
        label: args.label.clone(),
        alias: args.label,
        user: args.user,
        host_key,
        reach: Reach::Tailcat {
            token: grant.conn_blob,
            client_key: String::new(),
        },
        alternative: None,
        pending: false,
        pending_why: None,
    };
    let (sealed, code) = ceremony::seal_reply(&offer, &ceremony::manifest(&offer, vec![runner]))?;
    eprintln!("Paste the reply into the browser and compare this code: {code}");
    println!("{sealed}");
    Ok(())
}

fn enrollment(offer: &Offer) -> Result<pb::Request, Box<dyn std::error::Error>> {
    if offer.key_b.is_some()
        || !offer
            .key_a
            .starts_with("sk-ecdsa-sha2-nistp256@openssh.com ")
        || offer.node_key.is_empty()
        || offer.reply_key.is_none()
    {
        return Err("Expected a browser offer with a passkey and a tunnel key.".into());
    }
    let client_id = ceremony::client_id(&offer.key_a).ok_or("Invalid browser passkey.")?;
    Ok(with(
        req("client.enroll"),
        request::Payload::ClientEnroll(pb::ClientEnroll {
            public_key: offer.key_a.clone(),
            label: offer.name.clone(),
            client_id,
            scope: pb::Scope::Control as i32,
            node_key: offer.node_key.clone(),
            ..Default::default()
        }),
    ))
}
