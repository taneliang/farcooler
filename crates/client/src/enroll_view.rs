//! What the browser's Add Runner, approval and Devices screens say (ov-1490,
//! R-112): the page names which sentence it needs and the runner data in it,
//! and draws what this answers.

use serde::Deserialize;

/// One sentence or label, with the data it is about.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "say", rename_all = "snake_case")]
pub enum Say {
    // Add Runner, and this browser's own code.
    AddRunner,
    GettingCommand,
    CommandExpired,
    CommandUnavailable,
    CopyCommand,
    CopyByHand,
    CodeExpired,
    OwnPasskey { fingerprint: String },
    /// The runner took this browser, and the account never heard of it.
    NotInAccount,
    AddedTitle { runner: String },
    AddedDetail,
    /// When the directory last heard from a runner; both are in milliseconds.
    LastHeard { at: Option<i64>, now: i64 },
    SetUp,
    SetUpRunner { name: String },

    // Devices.
    DevicesTitle { runner: Option<String> },
    DevicesDetail { runner: Option<String> },
    NoRunnerToList,
    ThisBrowserRow { label: String },
    RemoveButton { confirming: bool },
    RemoveLabel { confirming: bool, device: String },
    Removing { device: String, runner: String },
    /// What a removal did: the runners it left, why any other refused, and
    /// the ones this browser isn't connected to, which still let it in.
    Removed { device: String, done: Vec<String>, failed: Vec<String>, offline: Vec<String> },

    // Approving another browser.
    CodeRefused { word: String },
    NoCamera,
    ConnectFirst,
    Asking { name: String },
    NotConnected { runners: Vec<String> },
    ChooseRunner,
    Adding { runner: String },
    ReplyNotSent,
    CheckCode { failed: Vec<String> },
}

fn named(runner: Option<String>) -> String {
    runner.unwrap_or_else(|| "this runner".into())
}

pub fn words(say: Say) -> String {
    match say {
        Say::AddRunner => "Add Runner".into(),
        Say::GettingCommand => "Getting your command…".into(),
        Say::CommandExpired => "That command expired, so here’s a new one. Copy it again.".into(),
        Say::CommandUnavailable => "Far Cooler couldn’t make a command just now. Reload the page to try again.".into(),
        Say::CopyCommand => "Copy Command".into(),
        Say::CopyByHand => "Select It and Copy".into(),
        Say::CodeExpired => "That code expired, so here’s a new one. Scan or copy it again.".into(),
        Say::OwnPasskey { fingerprint } => format!("This browser’s passkey: {fingerprint}"),
        Say::NotInAccount => "You can open this runner here, but Far Cooler couldn’t add it to your account. Run a new command on it to finish.".into(),
        Say::AddedTitle { runner } => format!("{runner} is added"),
        Say::AddedDetail => "Touch your passkey to open its terminals and workspaces.".into(),
        Say::LastHeard { at, now } => last_heard(at, now),
        Say::SetUp => "Set Up".into(),
        Say::SetUpRunner { name } => format!("Set Up {name}"),
        Say::DevicesTitle { runner } => format!("Devices on {}", named(runner)),
        Say::DevicesDetail { runner } => {
            format!("These can open {}. Remove one you’ve lost or no longer use: it stops opening it at once.", named(runner))
        }
        Say::NoRunnerToList => "Connect to a runner to see its devices.".into(),
        Say::ThisBrowserRow { label } => format!("{label} (this browser)"),
        Say::RemoveButton { confirming } => remove_button(confirming).into(),
        Say::RemoveLabel { confirming, device } => format!("{} {device}", remove_button(confirming)),
        Say::Removing { device, runner } => format!("Removing {device} from {runner}. Verify with your passkey."),
        Say::Removed { device, done, failed, offline } => removed(&device, &done, failed, &offline),
        Say::CodeRefused { word } => code_refused(&word).into(),
        Say::NoCamera => "This browser couldn’t use a camera. Paste the code instead.".into(),
        Say::ConnectFirst => "Connect to a runner first.".into(),
        Say::Asking { name } => format!("{name} wants to open your runners. Check that it shows this passkey:"),
        Say::NotConnected { runners } => format!("Not connected now: {}. To include one, open it first.", runners.join(", ")),
        Say::ChooseRunner => "Choose at least one runner.".into(),
        Say::Adding { runner } => format!("Adding it to {runner}. Verify with your passkey."),
        Say::ReplyNotSent => "The runners added it, but Far Cooler couldn’t send it the answer. Make a new code on the new device and approve it again.".into(),
        Say::CheckCode { failed } => {
            let check = "Check that the new device shows this code, then select They Match there. If it shows another, remove it above.";
            std::iter::once(check.to_string()).chain(failed).collect::<Vec<_>>().join(" ")
        }
    }
}

fn remove_button(confirming: bool) -> &'static str {
    if confirming { "Confirm Remove" } else { "Remove" }
}

fn last_heard(at: Option<i64>, now: i64) -> String {
    let Some(at) = at else { return "Not heard from yet".into() };
    let minutes = (now - at).div_euclid(60_000);
    if minutes < 5 {
        return "Online now".into();
    }
    let ago = |n: i64, unit: &str| format!("Last heard from {n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match minutes / 60 {
        0 => ago(minutes, "minute"),
        hours @ 1..48 => ago(hours, "hour"),
        hours => ago(hours / 24, "day"),
    }
}

/// A removal is said only for the runners it really left. With none, nothing
/// claims the device lost anything, and no other runner is named.
fn removed(device: &str, done: &[String], failed: Vec<String>, offline: &[String]) -> String {
    let mut out = Vec::new();
    if !done.is_empty() {
        out.push(format!("{device} can no longer open {}.", done.join(", ")));
    }
    out.extend(failed);
    if !done.is_empty() && !offline.is_empty() {
        out.push(format!(
            "This browser isn’t connected to {}. If {device} was added there, connect and remove it too.",
            offline.join(", ")
        ));
    }
    out.join(" ")
}

/// Why a pasted or scanned code can't be approved, by the ceremony's word.
fn code_refused(word: &str) -> &'static str {
    match word {
        "wrong_account" => "That code is for another account. Sign in to this account on the new device, then use its new code.",
        "not_a_browser" => "That isn’t a new browser’s code. On the new device, open Far Cooler, sign in and choose Approve From Another Device.",
        "wrong_target" => "That’s this browser’s own code. Use the one on the new device.",
        "channel" => "That code is from a different Far Cooler release. Open the same one on both devices.",
        "version" => "That code is from a newer Far Cooler. Reload this page, then try again.",
        _ => "That code couldn’t be read. Copy all of it, or scan it again.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(request: &str) -> String {
        words(serde_json::from_str(request).unwrap())
    }

    #[test]
    fn a_runners_age_is_said_in_the_largest_unit_that_fits() {
        const MIN: i64 = 60_000;
        let at = |ago: i64| words(Say::LastHeard { at: Some(10_000 * MIN - ago * MIN), now: 10_000 * MIN });
        assert_eq!(said(r#"{"say":"last_heard","at":null,"now":0}"#), "Not heard from yet");
        assert_eq!(at(4), "Online now");
        assert_eq!(at(5), "Last heard from 5 minutes ago");
        assert_eq!(at(60), "Last heard from 1 hour ago");
        assert_eq!(at(47 * 60), "Last heard from 47 hours ago");
        assert_eq!(at(48 * 60), "Last heard from 2 days ago");
    }

    #[test]
    fn a_removal_claims_only_the_runners_it_left() {
        let list = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        let say = |done: &[&str], failed: &[&str], offline: &[&str]| {
            words(Say::Removed { device: "iPhone".into(), done: list(done), failed: list(failed), offline: list(offline) })
        };
        assert_eq!(say(&["studio"], &[], &[]), "iPhone can no longer open studio.");
        assert_eq!(
            say(&["studio"], &["mini refused."], &["box"]),
            "iPhone can no longer open studio. mini refused. This browser isn’t connected to box. If iPhone was added there, connect and remove it too."
        );
        assert_eq!(say(&[], &["studio refused."], &["box"]), "studio refused.");
    }

    #[test]
    fn each_refused_code_says_what_to_do_instead() {
        assert!(said(r#"{"say":"code_refused","word":"wrong_account"}"#).starts_with("That code is for another account."));
        assert!(said(r#"{"say":"code_refused","word":"anything else"}"#).starts_with("That code couldn’t be read."));
        assert_eq!(said(r#"{"say":"devices_title","runner":null}"#), "Devices on this runner");
    }
}
