//! `farcooler settings set-projector on|off` (ov-372): the runner's agent
//! row projector, which the Mac's native agent view reads, turned on or off
//! through `settings.set_projector`. It writes `[agents] projector` to the
//! runner's config.toml and takes effect at once.

use farcooler_protocol::v1::request;

use super::{req, with};
use crate::daemon_link::{Link, expect_value};

#[derive(Clone, Copy, clap::ValueEnum)]
pub(crate) enum OnOff {
    On,
    Off,
}

/// Ask the runner `link` reaches to turn its projector `state`, and say so.
pub(crate) async fn set(link: &mut Link, state: OnOff, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let on = matches!(state, OnOff::On);
    let mut ask = with(
        req("settings.set_projector"),
        request::Payload::HostSettings(farcooler_protocol::v1::HostSettings { branch_prefix: String::new(), projector: on }),
    );
    ask.required_capabilities.push(farcooler_protocol::capability::PROJECTOR_SETTING.to_string());
    expect_value(link.call(ask).await?.value)?;
    if json {
        println!("{}", serde_json::json!({ "projector": on }));
    } else {
        println!("projector is now {}", if on { "on" } else { "off" });
    }
    Ok(())
}
