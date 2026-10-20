//! A member's Claude login as a Needs You item (ov-587). Its own file to keep
//! `needs_you.rs` inside its size budget.
//!
//! **One list per viewer.** The rest of the rollup is the runner's, the same
//! for every device that reads it. A login is one person's: `Inputs::viewer`
//! says who is reading, and `items` makes
//!
//! - for the member themself, `Your Claude login on <Runner> stopped working.`
//!   with **Sign In Again**, or `Sign in to Claude on <Runner> to start your
//!   rep.` with **Sign In**, both the `sign_in` action the apps answer by
//!   opening their sign-in sheet;
//! - for the owner, `<Name>’s Claude login stopped working` (or `<Name> needs
//!   to sign in to Claude`), with no button: it is theirs to fix. The member's
//!   email is never in it;
//! - for another member and a released device, nothing.
//!
//! A reader with no viewer (`None`: the count the relay is told, the plan
//! glance) gets none, so a login never moves the runner-wide count.

use farcooler_protocol::v1::{self as pb, NeedsYouKind};

use super::{Inputs, rank};
use crate::claude_login_renewal::{LoginNeed, Why};
use crate::people::Person;
use crate::wire::timestamp;

/// The one action a member's own item offers. The apps open the Claude
/// sign-in sheet; the daemon is asked nothing.
pub const SIGN_IN: &str = "sign_in";

/// What the item says, to a member and to the owner.
fn question(need: &LoginNeed, runner: &str, own: bool) -> String {
    match (own, need.why) {
        (true, Why::Stopped) => format!("Your Claude login on {runner} stopped working."),
        (true, Why::Asked) => format!("Sign in to Claude on {runner} to start your rep."),
        (false, Why::Stopped) => format!("{}’s Claude login stopped working", need.name),
        (false, Why::Asked) => format!("{} needs to sign in to Claude", need.name),
    }
}

fn item(need: &LoginNeed, runner: &str, own: bool, now_ms: i64) -> pb::NeedsYouItem {
    let title = match need.why {
        Why::Stopped => "Sign In Again",
        Why::Asked => "Sign In",
    };
    pb::NeedsYouItem {
        id: format!("claude-login:{}", need.handle),
        kind: NeedsYouKind::ClaudeLogin as i32,
        rank: rank(NeedsYouKind::ClaudeLogin, need.since_ms, now_ms),
        since: Some(timestamp(need.since_ms)),
        question: question(need, runner, own),
        actions: if own { vec![pb::NeedsYouAction { id: SIGN_IN.into(), title: title.into(), destructive: false, primary: true }] } else { Vec::new() },
        ..pb::NeedsYouItem::default()
    }
}

/// The login items `inputs.viewer` is to see.
pub(super) fn items(inputs: &Inputs, now_ms: i64) -> Vec<pb::NeedsYouItem> {
    let needs = inputs.logins.iter();
    match &inputs.viewer {
        Some(Person::Owner) => needs.map(|n| item(n, &inputs.runner, false, now_ms)).collect(),
        Some(Person::Member(me)) => needs.filter(|n| &n.handle == me).map(|n| item(n, &inputs.runner, true, now_ms)).collect(),
        Some(Person::Released) | None => Vec::new(),
    }
}

#[cfg(test)]
#[path = "login_tests.rs"]
mod tests;
