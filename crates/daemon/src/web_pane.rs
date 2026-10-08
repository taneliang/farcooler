//! Web pages as panes (ov-435).
//!
//! A web pane is a real tmux pane made with the `web` preset, exactly as a
//! changes pane is made with `changes`: `farcooler pane-host --kind web` holds
//! the rectangle and the Mac draws a `WKWebView` into it. So it splits, drags,
//! zooms, breaks out and closes like every other pane, and none of that is
//! code here. What is here: the URL check, and the one verb that opens one.
//!
//! **Ruling R-41: an agent may open a page for the owner to look at, and
//! nothing reads or acts on page contents.** Nothing on this side ever sees
//! one. The runner stores the URL the pane opened on and nothing after it:
//! where the owner navigated stays on the Mac.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::SplitSide;
use farcooler_store::models;
use uuid::Uuid;

use crate::service::Service;

/// The preset whose pane is a web page. A word rather than a flag, for the
/// reason `CHANGES_PRESET` is one: every path that opens a pane carries a
/// preset already.
pub const WEB_PRESET: &str = "web";

/// The longest URL a web pane takes, in bytes. Long enough for any real
/// Notion, Linear or GitHub link (a few hundred), short enough that a URL is
/// never a way to carry a document.
pub const LONGEST_URL: usize = 8 * 1024;

/// `raw`, if a web pane may open it: an absolute http or https URL with a
/// host, no whitespace or control characters, at most [`LONGEST_URL`].
///
/// Everything else is refused before anything is made: `file:` would show a
/// local file an agent named, `javascript:` and `data:` run or render what an
/// agent wrote rather than a page that exists, and an app scheme hands the
/// URL to another program. Returned as given, not normalized: the page is
/// the one asked for, and WebKit parses it.
pub fn checked_url(raw: &str) -> Result<&str> {
    const REFUSED: DomainError = DomainError::InvalidArgument {
        what: "a web pane opens only an http or https address",
    };
    if raw.is_empty() || raw.len() > LONGEST_URL || raw.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(REFUSED);
    }
    let (scheme, rest) = raw.split_once(':').ok_or(REFUSED)?;
    if !(scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http")) {
        return Err(REFUSED);
    }
    let authority = rest.strip_prefix("//").ok_or(REFUSED)?;
    let authority = authority.split(['/', '?', '#']).next().unwrap_or_default();
    // The host: after any `user@`, before any `:port`. An IPv6 literal keeps
    // its brackets, which is all a non-empty check needs.
    let host = authority.rsplit('@').next().unwrap_or_default();
    let host = if host.starts_with('[') { host } else { host.split(':').next().unwrap_or_default() };
    if host.is_empty() {
        return Err(REFUSED);
    }
    Ok(raw)
}

impl Service {
    /// `layout.split`, for any preset: a new pane beside `target`, or beside
    /// the focused pane of the layout `group` names, or of the active one.
    ///
    /// The `web` preset differs in two ways. It needs `url`, checked before
    /// anything is made. And with no layout to split, it opens in a window of
    /// its own, as `terminal.create` does, so an agent can open a page in a
    /// worktree nobody has a terminal in without asking first. Every other
    /// preset refuses a URL, rather than dropping it, and needs a pane to
    /// split, as it always has.
    #[allow(clippy::too_many_arguments)]
    pub async fn split_for_wire(
        &self,
        worktree: Uuid,
        target: Option<Uuid>,
        group: Option<&str>,
        side: SplitSide,
        title: &str,
        preset: &str,
        url: Option<&str>,
    ) -> Result<models::Terminal> {
        if preset != WEB_PRESET {
            if url.is_some() {
                return Err(DomainError::InvalidArgument { what: "only a web pane opens an address" });
            }
            let anchor = match target {
                Some(id) => id,
                None => self.focused_pane(worktree, group).await?,
            };
            return self.split_terminal(worktree, anchor, side, title, preset).await;
        }

        let url = checked_url(url.unwrap_or_default())?;
        let anchor = match (target, group) {
            (Some(id), _) => Some(id),
            // A layout named is a layout meant: not finding it is an error,
            // never a quiet new window.
            (None, Some(group)) if !group.is_empty() => Some(self.focused_pane(worktree, Some(group)).await?),
            (None, _) => self.active_layout(worktree).await?.and_then(|view| {
                view.focused().or(view.panes.first()).map(|pane| pane.terminal_id)
            }),
        };
        let term = match anchor {
            Some(anchor) => self.split_terminal(worktree, anchor, side, title, WEB_PRESET).await?,
            None => self.create_terminal(worktree, title, WEB_PRESET).await?,
        };
        self.store.set_web_url(term.id, url)
    }
}

#[cfg(test)]
#[path = "web_pane_tests.rs"]
mod tests;
