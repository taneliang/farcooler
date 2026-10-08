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
use farcooler_protocol::v1::{SplitSide, TerminalIntent};
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

/// The most web panes one worktree holds. An agent that loops on `open-url`
/// would otherwise tile the owner's screen with pages, each a live WebKit
/// process on the Mac.
pub const MOST_PER_WORKTREE: usize = 8;

/// A web pane is made only by `layout.split` with a page, which checks the
/// page. Any other way of asking for the `web` preset (`terminal.create`)
/// would make a pane with no page that nothing can fill in, so it's refused.
pub fn refuse_without_page(preset: &str) -> Result<()> {
    if preset.split(':').next() == Some(WEB_PRESET) {
        return Err(DomainError::InvalidArgument {
            what: "a web pane is opened with an address, by layout open-url",
        });
    }
    Ok(())
}

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
    if scheme.eq_ignore_ascii_case("http") && !plain_http_loads(host) {
        return Err(DomainError::InvalidArgument {
            what: "a web pane opens http only on this network; use the https address",
        });
    }
    Ok(raw)
}

/// Whether the Mac loads plain http from `host`. App Transport Security
/// refuses it to a public name and exempts an IP address, a name with no dot
/// (`localhost`, `devbox`) and `.local`. A pane opened on any other http
/// address could only show the system's error, so it's refused here, in words
/// an agent can act on, rather than there.
fn plain_http_loads(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.parse::<std::net::IpAddr>().is_ok()
        || !bare.contains('.')
        || bare.to_ascii_lowercase().ends_with(".local")
}

impl Service {
    /// Whether `worktree` can hold another web pane (L9, ov-435).
    pub(crate) fn room_for_another_page(&self, worktree: Uuid) -> Result<()> {
        let held = self
            .store
            .list_terminals_for_worktree(worktree)?
            .iter()
            .filter(|t| t.pane_mode == models::PaneMode::Web && t.intent == TerminalIntent::Running)
            .count();
        if held >= MOST_PER_WORKTREE {
            return Err(DomainError::InvalidArgument {
                what: "this worktree already has as many web panes as it can hold; close one first",
            });
        }
        Ok(())
    }

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
        self.room_for_another_page(worktree)?;
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
