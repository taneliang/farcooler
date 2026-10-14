//! `farcooler runner sign-registration`: the box signs, for the account the app
//! is signed in as (ov-497).
//!
//! The relay's directory registers a runner only on this runner's own
//! signature (`services/relay/src/directory/runners.ts`), because its install
//! id is in `ps` and proves nothing. A runner has no sign-in, so the app does
//! the talking to the relay and asks the box, over the ssh channel it already
//! has, for the proof. This prints exactly what goes in the relay request
//! beside `team`: `install`, `runnerKey`, `issuedAt` and `proof`.
//!
//! `farcooler runner claim` prints the same, signed as a claim: it moves a
//! runner someone else registered to the account and team named. Anyone who
//! runs as the runner's Unix user can run it, agents included, and can read the
//! key file besides. The owner accepted that (ov-497): being that Unix user is
//! the trust boundary, so a claim is instant. `docs/runners.md` says so.
//!
//! Both proofs name the relay they are for, so one printed for staging does
//! nothing at production (ov-497 review N3).
//!
//! The private key never reaches this output. Only the public key and the
//! signature are printed.

use farcooler_daemon::paths;
use farcooler_daemon::runner_key::{Action, RunnerKey};
use farcooler_protocol::v1::Scope;
use farcooler_transport::Peer;

use crate::Fallible;

/// What both commands take.
#[derive(clap::Args)]
pub struct ProofArgs {
    /// The signed-in account the runner is registered for.
    #[arg(long)]
    pub account: String,
    /// The team the runner joins.
    #[arg(long)]
    pub team: String,
    /// The relay the proof is for. Default: this build's own relay.
    #[arg(long)]
    pub relay: Option<String>,
}

/// The command that makes `target`'s own CLI print a proof: a registration, or
/// a claim.
///
/// The app asks the box for the proof over the ssh channel it already has (the
/// box signs, so the key never moves), and this is how `--runner` reaches it:
/// this channel's CLI on that runner, as `push` does, because the one this
/// build uploaded is the one that owns that runner's key.
///
/// A claim forwards too, for the one thing the app does with it: moving a
/// runner its owner already registered to another of their teams, after the
/// relay has said it is theirs (`elsewhere`) and they have confirmed. A shell
/// on the box over ssh is the same root of trust a registration relies on.
///
/// Every value is quoted. None is a secret: an account id, a team id and a
/// relay address, all of which the relay already knows.
pub fn remote_command(cli: &str, claim: bool, args: &ProofArgs) -> String {
    use farcooler_daemon::service::shell_quote;
    let relay = args.relay.as_deref().map(|r| format!(" --relay {}", shell_quote(r))).unwrap_or_default();
    let verb = if claim { "claim" } else { "sign-registration" };
    format!(
        "~/.local/bin/{cli} runner {verb} --account {} --team {}{relay}",
        shell_quote(&args.account),
        shell_quote(&args.team),
    )
}

/// Run `sign-registration` (or `claim`) on `target` and let its JSON through on
/// stdout.
pub async fn forward(target: &str, claim: bool, args: &ProofArgs) -> Fallible {
    crate::runner_install::remote_run(target, &remote_command(crate::runner_install::cli_name(), claim, args)).await
}

/// Refuses `--runner` (`remote`): this signs with THIS machine's key, so a
/// remote target would have it sign for the wrong box without a word.
/// A remote target forwards instead (`forward`), which runs it on the box.
pub fn print(remote: bool, claim: bool, args: ProofArgs) -> Fallible {
    if remote {
        return Err("This signs with the key of the runner it runs on. Run it on that runner, without --runner.".into());
    }
    let dir = paths::ensure_runtime_dir()?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis();
    let relay = args.relay.unwrap_or_else(farcooler_daemon::push::default_relay);
    let action = if claim { Action::Claim } else { Action::Register };
    println!("{}", proof_in(&dir, action, &args.account, &args.team, &relay, now as u64)?);
    Ok(())
}

/// The request fields for `action`, as one JSON object, signed with the key
/// in `dir` (made if this is the first need for it).
///
/// This is a process on the box under the runner's own user, which the signer
/// calls a local `host_admin` caller. A remote caller never gets here.
fn proof_in(
    dir: &std::path::Path,
    action: Action,
    account: &str,
    team: &str,
    relay: &str,
    issued_at: u64,
) -> Result<String, Box<dyn std::error::Error>> {
    let install = paths::load_or_create_install_id_in(dir)?;
    let key = RunnerKey::load_or_create_in(dir)?;
    let local = Peer { client_id: None, scope: Scope::HostAdmin };
    let proof = key.sign(&local, action, account, team, &install, relay, issued_at)?;
    Ok(serde_json::json!({
        "install": install,
        "runnerKey": key.public_line(),
        "relay": relay,
        "issuedAt": issued_at,
        "proof": proof,
    })
    .to_string())
}

#[cfg(test)]
#[path = "runner_proof_tests.rs"]
mod tests;
