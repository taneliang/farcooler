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
//! There is deliberately NO `runner claim` here yet. A claim proof moves a
//! runner to a new owner, and anything that runs as the runner's Unix user,
//! every agent and every control grantee, could print one and could read the
//! key file besides (ov-497 review B1). It waits on an owner decision. Nothing
//! in this crate produces a claim proof.
//!
//! The private key never reaches this output. Only the public key and the
//! signature are printed.

use farcooler_daemon::paths;
use farcooler_daemon::runner_key::{Action, RunnerKey};
use farcooler_protocol::v1::Scope;
use farcooler_transport::Peer;

use crate::Fallible;

pub fn print(account: &str, team: &str) -> Fallible {
    let dir = paths::ensure_runtime_dir()?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis();
    println!("{}", proof_in(&dir, Action::Register, account, team, now as u64)?);
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
    issued_at: u64,
) -> Result<String, Box<dyn std::error::Error>> {
    let install = paths::load_or_create_install_id_in(dir)?;
    let key = RunnerKey::load_or_create_in(dir)?;
    let local = Peer { client_id: None, scope: Scope::HostAdmin };
    let proof = key.sign(&local, action, account, team, &install, issued_at)?;
    Ok(serde_json::json!({
        "install": install,
        "runnerKey": key.public_line(),
        "issuedAt": issued_at,
        "proof": proof,
    })
    .to_string())
}

#[cfg(test)]
#[path = "runner_proof_tests.rs"]
mod tests;
