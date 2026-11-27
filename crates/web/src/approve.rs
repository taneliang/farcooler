//! Approving and removing devices from this browser, and the runners the
//! account has that this browser doesn't open (ov-1490).
//!
//! Every rule is Rust's: what may be approved is `farcooler-ceremony`'s and the
//! runner's own (`client.enroll` refuses a browser's approval of anything but
//! another browser at Control or below), and the page only draws what these
//! answer.

use std::collections::BTreeSet;

use farcooler_ceremony::{self as ceremony, CeremonyError, Offer, RunnerEntry};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::enroll::Granted;

/// What another browser's code says, for the person to check before they
/// approve it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Asking {
    pub name: String,
    /// Its passkey's fingerprint, as the new browser shows it.
    pub fingerprint: String,
}

/// The code a new browser shows, if this account may approve it: a browser's,
/// for this account and channel, and not this browser's own.
pub fn asking(code: &str, account: &str, own_key_line: &str) -> Result<(Offer, Asking), CeremonyError> {
    let offer = ceremony::accept_browser_offer(code, account)?;
    let fingerprint = ceremony::fingerprint(&offer.key_a).unwrap_or_default();
    if ceremony::fingerprint(own_key_line).as_deref() == Some(fingerprint.as_str()) {
        return Err(CeremonyError::WrongTarget);
    }
    let asking = Asking { name: offer.name.clone(), fingerprint };
    Ok((offer, asking))
}

/// What `client.enroll` is asked for `offer`: another browser, at Control,
/// admitted to the tunnel by its node key.
pub fn enroll_args(offer: &Offer) -> Value {
    json!({
        "publicKey": offer.key_a,
        "label": offer.name,
        "clientId": ceremony::client_id(&offer.key_a).unwrap_or_default(),
        "scope": "control",
        "nodeKey": offer.node_key,
    })
}

/// The sealed reply that grants `runners` to the browser `offer` names, and
/// the code both screens show. A runner with no tunnel token is refused: the
/// new browser could never open it.
pub fn seal(offer: &Offer, runners: Vec<RunnerEntry>) -> Result<(String, String), CeremonyError> {
    if runners.is_empty() || runners.iter().any(|r| matches!(&r.reach, ceremony::Reach::Tailcat { token, .. } if token.is_empty())) {
        return Err(CeremonyError::NoTunnel);
    }
    ceremony::seal_reply(offer, &ceremony::manifest(offer, runners))
}

/// One runner of the account's directory, as `directory/teams/runners` lists it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listed {
    id: String,
    name: String,
    #[serde(default)]
    last_seen_at: Option<f64>,
    #[serde(default)]
    access: Vec<Access>,
}

#[derive(Debug, Deserialize)]
struct Access {
    user: String,
}

#[derive(Debug, Deserialize)]
struct Team {
    #[serde(default)]
    runners: Vec<Listed>,
}

/// A runner the account has that this browser can't open yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unenrolled {
    pub name: String,
    pub last_seen_at: Option<i64>,
}

/// The runners `account` may reach, from each team's list, that this browser
/// holds no usable grant for.
///
/// The directory doesn't keep what a grant names, so the two are matched by
/// name, and each grant accounts for one directory runner at most: two runners
/// called "studio" and one grant leave one "studio" listed. A runner is listed
/// when in doubt, never hidden. A grant the browser can't dial (`why`) still
/// takes its runner's place here, and is listed from the grant itself.
pub fn unenrolled(teams: &str, account: &str, grants: &[Granted]) -> Result<Vec<Unenrolled>, serde_json::Error> {
    let teams: Vec<Team> = serde_json::from_str(teams)?;
    let mut seen = BTreeSet::new();
    let mut listed: Vec<Listed> = teams
        .into_iter()
        .flat_map(|t| t.runners)
        .filter(|r| r.access.iter().any(|a| a.user == account) && seen.insert(r.id.clone()))
        .collect();
    let mut out: Vec<Unenrolled> = Vec::new();
    for grant in grants {
        let found = listed.iter().position(|r| r.name.trim() == grant.label.trim()).map(|at| listed.remove(at));
        if !grant.why.is_empty() {
            out.push(Unenrolled { name: grant.label.clone(), last_seen_at: found.and_then(|r| r.last_seen_at).map(|at| at as i64) });
        }
    }
    out.extend(listed.into_iter().map(|r| Unenrolled { name: r.name, last_seen_at: r.last_seen_at.map(|at| at as i64) }));
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// One device that may log in to a runner, as the Devices list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRow {
    pub client_id: String,
    pub label: String,
    /// This browser's own row, which Sign Out takes away, not Remove.
    pub here: bool,
}

#[derive(Deserialize)]
struct Clients {
    clients: Vec<Client>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Client {
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    foreign: bool,
}

/// The devices in `client.list`'s answer, one row each (a Mac has two lines
/// under one id). Keys Far Cooler didn't write aren't devices it can remove,
/// so they aren't listed here.
pub fn devices(list: &str, own_key_line: &str) -> Result<Vec<DeviceRow>, serde_json::Error> {
    let list: Clients = serde_json::from_str(list)?;
    let own = ceremony::client_id(own_key_line).unwrap_or_default();
    let mut seen = BTreeSet::new();
    Ok(list
        .clients
        .into_iter()
        .filter(|c| !c.foreign && !c.client_id.is_empty() && seen.insert(c.client_id.clone()))
        .map(|c| DeviceRow { here: c.client_id == own, label: shown(&c.label), client_id: c.client_id })
        .collect())
}

/// The name a line's comment was made from: `farcooler-<name>-<fingerprint>`.
fn shown(label: &str) -> String {
    let inner = label.strip_prefix("farcooler-").and_then(|rest| rest.rsplit_once('-')).map_or(label, |(name, _)| name);
    inner.replace('-', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(label: &str, why: &str) -> Granted {
        Granted { id: label.into(), label: label.into(), user: "u".into(), host_key: String::new(), token: String::new(), why: why.into(), cause: String::new() }
    }

    fn teams(runners: &[(&str, &str, &str)]) -> String {
        let runners: Vec<_> = runners.iter().map(|(id, name, user)| json!({"id": id, "name": name, "lastSeenAt": 5.0, "access": [{"user": user}]})).collect();
        json!([{ "runners": runners }]).to_string()
    }

    #[test]
    fn an_account_runner_with_no_grant_here_is_listed_never_hidden() {
        let listed = teams(&[("1", "studio", "me"), ("2", "studio", "me"), ("3", "build box", "me"), ("4", "theirs", "someone")]);
        let names = |grants: &[Granted]| unenrolled(&listed, "me", grants).unwrap().into_iter().map(|r| r.name).collect::<Vec<_>>();
        assert_eq!(names(&[]), ["build box", "studio", "studio"], "another person's runner isn't the account's");
        // One grant accounts for one of two runners of that name.
        assert_eq!(names(&[grant("studio", "")]), ["build box", "studio"]);
        // A grant the browser can't dial is still not enrolled.
        assert_eq!(names(&[grant("studio", ""), grant("studio", "pending"), grant("build box", "")]), ["studio"]);
        // One the directory never heard of, from a grant alone.
        assert_eq!(unenrolled("[]", "me", &[grant("laptop", "address_only")]).unwrap().len(), 1);
    }

    #[test]
    fn a_device_is_one_row_with_its_name_and_this_browser_marked() {
        const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFJhEUXBcOjzsos5aFyw6hfDrERWHei9akIudYJGG2gR browser";
        let own = ceremony::client_id(KEY).unwrap();
        let list = json!({"clients": [
            {"clientId": own, "label": "farcooler-Chrome-on-Mac-abc123"},
            {"clientId": "farcooler-1", "label": "farcooler-iPhone-abc"},
            {"clientId": "farcooler-1", "label": "farcooler-iPhone-def"},
            {"clientId": "", "label": "mine", "foreign": true},
        ]})
        .to_string();
        let rows = devices(&list, KEY).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].label.as_str(), rows[0].here), ("Chrome on Mac", true));
        assert_eq!((rows[1].label.as_str(), rows[1].here), ("iPhone", false));
    }
}
