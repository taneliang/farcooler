//! `farcooler runner sign-registration` and `farcooler runner claim`: the box
//! signs, for the account the app is signed in as (ov-497).
//!
//! The relay's directory registers a runner only on this runner's own
//! signature (`services/relay/src/directory/runners.ts`), because its install
//! id is in `ps` and proves nothing. A runner has no sign-in, so the app does
//! the talking to the relay and asks the box, over the ssh channel it already
//! has, for the proof. This prints exactly what goes in the relay request
//! beside `team`: `install`, `runnerKey`, `issuedAt` and `proof`.
//!
//! `claim` is the same proof under the other statement, for a runner someone
//! else registered first: whoever has a shell on the box is the one entitled
//! to move it, and the relay takes that signature as the whole argument.
//!
//! The private key never reaches this output. Only the public key and the
//! signature are printed, and the command is run by someone who already holds
//! the shell, which is the scope the signer demands (`runner_key::sign`).

use farcooler_daemon::paths;
use farcooler_daemon::runner_key::{Action, RunnerKey};
use farcooler_protocol::v1::Scope;

use crate::Fallible;

pub fn print(action: Action, account: &str, team: &str) -> Fallible {
    let dir = paths::ensure_runtime_dir()?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis();
    println!("{}", proof_in(&dir, action, account, team, now as u64)?);
    Ok(())
}

/// The request fields for `action`, as one JSON object, signed with the key
/// in `dir` (made if this is the first need for it).
///
/// A process on the box under the runner's own user holds the key file
/// already, so it is what the signer calls a shell.
fn proof_in(
    dir: &std::path::Path,
    action: Action,
    account: &str,
    team: &str,
    issued_at: u64,
) -> Result<String, Box<dyn std::error::Error>> {
    let install = paths::load_or_create_install_id_in(dir)?;
    let key = RunnerKey::load_or_create_in(dir)?;
    let proof = key.sign(Scope::HostAdmin, action, account, team, &install, issued_at)?;
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
