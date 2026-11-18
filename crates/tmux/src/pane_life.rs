//! Whether one pane still holds one terminal's program (ov-554).
//!
//! A stream asks this when its pane's pipe ends. Before tmux 3.8 that meant
//! the pane had gone; from 3.8 it also means the program died and the pane
//! is kept dead (`remain-on-exit`), waiting for a restart. One
//! `display-message`, so a stream waiting on a dead pane costs one tmux
//! process per look and nothing else.

use farcooler_core::{DomainError, Result, tags};
use uuid::Uuid;

use crate::server::TmuxServer;

/// What a pane is now, for the terminal a stream was opened on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneLife {
    /// Running a program.
    Live,
    /// Kept after its program ended, and may be respawned.
    Dead,
    /// Not there, or someone else's: tmux numbers panes afresh in every
    /// server, so a pane id outlives nothing but its own server, and the
    /// terminal tag is what says this `%3` is still the one that was watched.
    Gone,
}

impl TmuxServer {
    /// What `pane_id` is now, for terminal `terminal`.
    ///
    /// A failed client is unknown, unless tmux explicitly reports a missing pane
    /// or server. Load or a signaled client must never masquerade as a closed pane
    /// and make a native runner tear down a live builder (ov-1357).
    pub async fn pane_life(&self, pane_id: &str, terminal: Uuid) -> Result<PaneLife> {
        let format = format!("#{{pane_dead}}\t#{{{}}}", tags::TERMINAL_ID);
        let out = self.run(&["display-message", "-p", "-t", pane_id, &format]).await?;
        probe(out, terminal)
    }
}

fn probe(out: crate::server::Output, terminal: Uuid) -> Result<PaneLife> {
    if !out.ok() {
        // Only a normal error exit with an explicit absence proves Gone. A failed
        // client alone is not proof that the program in the pane exited.
        let missing = out.status == Some(1) && (crate::pane_read::silent_socket(&out.stderr).is_some()
            || out.stderr.trim().strip_prefix("can't find pane: ").is_some_and(|pane| !pane.is_empty()));
        if missing { return Ok(PaneLife::Gone); }
        tracing::warn!(%terminal, status = ?out.status, stderr = %out.stderr.chars().take(1000).collect::<String>(),
            "tmux pane probe failed without proving absence");
        return Err(DomainError::TmuxUnavailable);
    }
    Ok(parse(&out.stdout, terminal))
}

fn parse(line: &str, terminal: Uuid) -> PaneLife {
    let Some((dead, tag)) = line.trim_end_matches('\n').split_once('\t') else { return PaneLife::Gone };
    if Uuid::parse_str(tag.trim()).ok() != Some(terminal) {
        return PaneLife::Gone;
    }
    match dead.trim() {
        "1" => PaneLife::Dead,
        "0" => PaneLife::Live,
        _ => PaneLife::Gone,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pane_is_read_for_its_own_terminal_only() {
        let mine = Uuid::now_v7();
        assert_eq!(parse(&format!("0\t{mine}\n"), mine), PaneLife::Live);
        assert_eq!(parse(&format!("1\t{mine}\n"), mine), PaneLife::Dead);
        assert_eq!(parse(&format!("0\t{}\n", Uuid::now_v7()), mine), PaneLife::Gone, "another terminal's pane");
        assert_eq!(parse("0\t\n", mine), PaneLife::Gone, "an untagged pane");
        assert_eq!(parse("", mine), PaneLife::Gone);
        assert_eq!(parse(&format!("?\t{mine}"), mine), PaneLife::Gone);
    }
}

#[cfg(test)]
#[path = "pane_life_tests.rs"]
mod probe_tests;
