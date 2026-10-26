//! A teammate's device, scanned by a runner admin (ov-525, ruling R-65).
//!
//! [`super::accept_offer`] grants your fleet to your own devices and nobody
//! else's, and it stays exactly that. This is the one other door: an offer
//! from another account is acceptable when the team directory says that
//! account shares a team with the person scanning, and the person scanning
//! then confirms, face to face, the name, the email and the device's key
//! fingerprint, which the device shows under its code.
//!
//! ## Who may be lying
//!
//! - **The scanned code.** Anyone can hold up a QR naming any account id. The
//!   account in an offer is a claim, so this module only ever uses it as a key
//!   to look up what the DIRECTORY says about that account; the email the
//!   admin is shown comes from the directory, never from the code. What stops
//!   a stranger holding up a code that names Bob is the admin looking at the
//!   person in front of them, and at the key fingerprint under their code:
//!   the same in-person check the same-account ceremony rests on.
//! - **The directory.** It may be unreachable, or answer wrongly. Unreachable,
//!   or an answer that is incomplete or about someone else, is a refusal
//!   ([`CeremonyError::DirectoryUnreachable`]), never a pass: a check that
//!   waves people through when it can't run is not a check. A directory that
//!   lies about membership can name a stranger a teammate, which is why the
//!   in-person confirmation is still required on top of it.
//! - **The app.** None of this reads anything a screen holds. The app hands
//!   over the directory's own answers as it received them, and every rule
//!   about them (which teams count, which roles, whose account is whose) is
//!   here, so the three apps cannot disagree about it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{CeremonyError, Offer, decode_offer, fingerprint};

/// A person the directory says shares a team with the one scanning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Teammate {
    /// Their directory account id: what the offer named, and what the runner
    /// stores beside their member row.
    pub account: String,
    /// Their email, as the directory has it. The one thing on the
    /// confirmation that the code couldn't have chosen.
    pub email: String,
    /// The name of the first team the two share, for the sentence.
    pub team: String,
    /// The Key A fingerprints of their trusted and certified devices, when
    /// the directory says (ov-549): it tells a team's admins, in each
    /// member's `keys`, and nobody else. `None` is "not told"; `Some` of an
    /// empty list is "none registered", and refuses every code (R-65).
    ///
    /// A key a roster holds out of `keys` while it waits out a trust reset's
    /// hold (R-76) counts here too, as that roster's word: it is in
    /// [`Teammate::held`] as well, and a code showing it is accepted only
    /// with the hold named, for the admin to confirm in person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<String>>,
    /// Their keys each roster holds (ov-565, R-76): a device trusted by a
    /// reset of their account, or certified by one, that the team's admins
    /// aren't sent as theirs until its hold ends or one of them confirms it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub held: Vec<HeldKey>,
}

/// A key one team's roster holds while it waits out a trust reset's hold
/// (ov-565, R-76).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldKey {
    /// The Key A fingerprint, as `keys` spells one.
    pub key: String,
    /// The id of the team whose roster holds it: whose admin may confirm it.
    pub team: String,
    /// When the hold ends, in milliseconds since the epoch, by the relay's
    /// clock. Shown, never judged: this device's clock decides nothing.
    pub until: i64,
}

/// An offer from a teammate, and who the directory says they are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeammateOffer {
    pub offer: Offer,
    pub teammate: Teammate,
    /// Each team whose roster holds this code's Key A (R-76). Empty for a key
    /// every roster lists. Otherwise the app says the device is waiting out a
    /// hold, and confirms it with the relay for each of these teams, in
    /// person, before it adds the device anywhere.
    pub held: Vec<HeldKey>,
}

/// The roles that put a person in a team for this purpose. `billing` grants
/// nothing (ov-480 §2), so it neither vouches for anyone nor is vouched for.
const ROLES: &[&str] = &["admin", "member"];

/// The teammates of `own_account`, from the directory's own answers.
///
/// `directory` is `{"me": <directory/me>, "rosters": [<teams/members>, …]}`,
/// each exactly as the relay answered. Refused as `directory_unreachable`
/// when it isn't that, when `me` is about another account, or when a team the
/// two could share has no roster: an incomplete answer can't say who isn't a
/// teammate, so it can't be used to say who is.
pub fn teammates(directory: &Value, own_account: &str) -> Result<Vec<Teammate>, CeremonyError> {
    let unusable = Err(CeremonyError::DirectoryUnreachable);
    let me = &directory["me"];
    if own_account.is_empty() || me["user"]["id"].as_str() != Some(own_account) {
        return unusable;
    }
    let Some(teams) = me["teams"].as_array() else { return unusable };
    let Some(rosters) = directory["rosters"].as_array() else { return unusable };

    let mut found: Vec<Teammate> = Vec::new();
    for team in teams {
        let role = team["role"].as_str().unwrap_or_default();
        // A personal team holds only its owner, so it vouches for nobody.
        if team["personal"].as_bool() != Some(false) || !ROLES.contains(&role) {
            continue;
        }
        let Some(id) = team["id"].as_str().filter(|id| !id.is_empty()) else { return unusable };
        let Some(roster) = rosters.iter().find(|r| r["team"]["id"].as_str() == Some(id)) else {
            return unusable;
        };
        let Some(members) = roster["members"].as_array() else { return unusable };
        let name = team["name"].as_str().unwrap_or_default();
        for member in members {
            let (Some(account), Some(role)) = (member["user"].as_str(), member["role"].as_str()) else {
                continue;
            };
            if account.is_empty() || account == own_account || !ROLES.contains(&role) {
                continue;
            }
            // A field that is there but isn't a list of strings is an answer
            // that can't decide whose key is whose, not one that says nothing.
            let mut keys: Option<Vec<String>> = match &member["keys"] {
                Value::Null => None,
                Value::Array(keys) => {
                    let Some(keys) = keys.iter().map(|k| k.as_str().map(str::to_string)).collect() else {
                        return unusable;
                    };
                    Some(keys)
                }
                _ => return unusable,
            };
            // The keys this roster holds (R-76) are its word too, so they're
            // keys it admits, named as held. Held keys from a roster that
            // didn't tell `keys`, or in any other shape, can't decide.
            let held: Vec<HeldKey> = match (&member["held"], keys.as_mut()) {
                (Value::Null, _) => Vec::new(),
                (Value::Array(entries), Some(listed)) => {
                    let mut held = Vec::new();
                    for entry in entries {
                        let (Some(key), Some(until)) = (entry["key"].as_str(), entry["until"].as_i64()) else {
                            return unusable;
                        };
                        if !listed.iter().any(|k| k == key) {
                            listed.push(key.to_string());
                        }
                        held.push(HeldKey { key: key.to_string(), team: id.to_string(), until });
                    }
                    held
                }
                _ => return unusable,
            };
            // Someone in two shared teams is one teammate. If any roster told
            // their keys, the check holds: a second team whose roster didn't
            // (one where the scanner isn't an admin) can't switch it off. Two
            // that told different keys are intersected, never united: a key
            // passes only if every roster that spoke lists it (review L1).
            if let Some(known) = found.iter_mut().find(|t| t.account == account) {
                known.held.extend(held);
                if let Some(more) = keys {
                    match known.keys.as_mut() {
                        Some(list) => list.retain(|k| more.contains(k)),
                        None => known.keys = Some(more),
                    }
                }
                continue;
            }
            let email = member["email"].as_str().unwrap_or_default().trim();
            found.push(Teammate {
                account: account.to_string(),
                email: email.to_string(),
                team: name.to_string(),
                keys,
                held,
            });
        }
    }
    Ok(found)
}

/// Read a scanned first code from another account, refusing it unless the
/// directory says that account is a teammate.
///
/// `directory` is `None` when the directory couldn't be asked, and that is a
/// refusal ([`CeremonyError::DirectoryUnreachable`]). In order, after the
/// version and channel every code is held to:
///
/// 1. the directory's answer must be usable ([`teammates`]);
/// 2. the offer must name an account, and not the caller's own: your own
///    devices go through [`super::accept_offer`], which this never replaces;
/// 3. that account must be one of the teammates (`not_a_teammate`);
///    and the directory must give them an email (`directory_unreachable`
///    otherwise);
/// 4. the offer must ask for no shell key (`teammate_shell`): a teammate is
///    granted Control or Read, never a shell (R-65);
/// 5. when the directory lists their registered keys, which it does to a
///    team's admins, this Key A must be one of them (`unregistered_key`). An
///    empty list refuses every code: they have registered no device that
///    anyone vouched for. Absent, the directory didn't say (the scanner isn't
///    an admin of any team the two share), and only the in-person check
///    stands. A key a roster holds after a trust reset (R-76) passes this
///    step only when `confirms_holds` says the caller will confirm it with
///    each holding team, in person, before adding it anywhere; it then comes
///    back in [`TeammateOffer::held`]. Otherwise it is refused as `held_key`
///    (ov-565 fix round 1, Low 1), so an app that doesn't know about holds
///    can't wave one through.
pub fn accept_teammate_offer(
    encoded: &str,
    own_account: &str,
    directory: Option<&Value>,
    confirms_holds: bool,
) -> Result<TeammateOffer, CeremonyError> {
    let offer = decode_offer(encoded)?;
    let directory = directory.ok_or(CeremonyError::DirectoryUnreachable)?;
    let teammates = teammates(directory, own_account)?;
    if offer.account.is_empty() || offer.account == own_account {
        return Err(CeremonyError::NotATeammate);
    }
    let teammate = teammates
        .into_iter()
        .find(|t| t.account == offer.account)
        .ok_or(CeremonyError::NotATeammate)?;
    // The email is the one thing on the confirmation the code couldn't have
    // chosen, so a teammate the directory gives none for is an answer that
    // can't decide (review F4).
    if teammate.email.is_empty() || !teammate.email.contains('@') {
        return Err(CeremonyError::DirectoryUnreachable);
    }
    if offer.key_b.is_some() {
        return Err(CeremonyError::TeammateShell);
    }
    let mut held = Vec::new();
    if let Some(keys) = &teammate.keys {
        let mine = fingerprint(&offer.key_a).ok_or(CeremonyError::UnregisteredKey)?;
        if !keys.contains(&mine) {
            return Err(CeremonyError::UnregisteredKey);
        }
        held = teammate.held.iter().filter(|h| h.key == mine).cloned().collect::<Vec<_>>();
        if !held.is_empty() && !confirms_holds {
            return Err(CeremonyError::HeldKey);
        }
    }
    Ok(TeammateOffer { offer, teammate, held })
}

#[cfg(test)]
#[path = "teammate_tests.rs"]
mod tests;
