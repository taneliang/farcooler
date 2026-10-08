//! `farcooler layout open-url` (ov-435): a web page as a pane.
//!
//! A split with the `web` preset and the page's URL, so it takes `layout
//! split`'s arguments and means the same by them. With no layout in the
//! worktree, the runner opens the page in a window of its own. Ruling R-41:
//! an agent may open a page for the owner to look at, and nothing reads or
//! acts on page contents. This command can't do either: it sends a URL and
//! prints the layout.

use clap::Args;
use farcooler_protocol::v1::{LayoutUpdate, Request};

/// Open a web page in a pane, beside the focused one. http and https only.
#[derive(Args, Debug)]
pub(crate) struct OpenUrl {
    pub worktree: String,
    /// The page: an http or https address.
    pub url: String,
    /// The pane to open it beside. Defaults to the focused one.
    pub terminal: Option<String>,
    /// left, right, top, or bottom.
    #[arg(long, default_value = "right")]
    pub side: String,
    /// The layout whose focused pane it opens beside when no pane is named.
    #[arg(long)]
    pub layout: Option<String>,
}

/// `update`, as a split that opens `open.url`. Refused here, before the
/// runner is asked, with the runner's own rule (`checked_url`).
pub(crate) fn fill(
    open: &OpenUrl,
    update: &mut LayoutUpdate,
    pick: impl Fn(&str) -> Result<bytes::Bytes, String>,
) -> Result<(), String> {
    let url = farcooler_daemon::web_pane::checked_url(&open.url).map_err(|e| e.to_string())?;
    update.side = crate::parse_side(&open.side)? as i32;
    update.command_preset = farcooler_daemon::web_pane::WEB_PRESET.into();
    update.name = "Web".into();
    update.url = Some(url.to_string());
    if let Some(given) = &open.terminal {
        update.target = Some(pick(given)?);
    }
    Ok(())
}

/// `request`, refused by a runner that would drop the URL. An older runner
/// reads an unknown field as absent and would launch a program called `web`.
pub(crate) fn required(opens_a_page: bool, mut request: Request) -> Request {
    if opens_a_page {
        request.required_capabilities.push(farcooler_protocol::capability::WEB_PANE.into());
    }
    request
}

#[cfg(test)]
#[path = "web_pane_tests.rs"]
mod tests;
