//! `terminal.compose` into codex (ov-416): a message typed into codex's own
//! box and sent, between turns only, and answered Sent once codex's rollout
//! records it.
//!
//! Measured on codex-cli 0.153.4 in a sandbox (`composer::codex` has the
//! box's rules, `session_log::codex_prompts` the record). After the gate
//! claude's sends pass (`compose_locked`: the agent proven, its box
//! recognized and empty, nobody typing, bracketed paste on):
//!
//! - **Between turns only.** codex takes an Enter mid-turn as a steer into
//!   the running turn, and raises approval dialogs no hook tells of unless a
//!   person has trusted Far Cooler's hooks, so nothing fences the Enter off a
//!   dialog: a turn running, as the screen or the rollout says, is `busy`.
//! - **Refused before anything is typed:** any slash command (`handoff`):
//!   each opens a popup or a panel, and which ones run as a prompt wasn't
//!   measured; a last word starting `@` or `$`, which opens a picker that
//!   takes the Enter (`picker`); a text taller than the box can show, which
//!   can't be read back (`too_tall`).
//! - **The pastes**, as claude's: each image's path alone, read back as
//!   `[Image #N]`, then the text once, read back as typed, or as `[Pasted
//!   Content N chars]` past 1,000 characters.
//! - **The Enter** goes in on its own, after the read-back, and only if a
//!   fresh capture still shows the box as pasted, nobody has typed since the
//!   paste began, and the rollout doesn't say a turn began meanwhile. An
//!   Enter after a bracketed paste is a send, never codex's paste-burst line
//!   break, which takes only an Enter typed in the same write as text.
//! - **The confirmation**: the rollout's `UserMessage` for the text and its
//!   images, written after the Enter, within `CONFIRM_SETTLES`; otherwise
//!   `unconfirmed`. A codex with no rollout yet (none is opened before the
//!   first prompt) or after `/new` is found again by what it holds open.

use std::path::{Path, PathBuf};

use farcooler_core::composer::{self, codex as drawn, drawn::Expected};
use farcooler_core::session_log::codex_prompts;
use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::AgentActivity;
use farcooler_store::models::Terminal;

use super::super::codex_turn::rollout_now;
use super::super::{Proven, Turn, codex_turn, registry_turn};
use super::{CONFIRM_SETTLES, Composition, Unpasted, quoted};
use crate::runtime::{Runtime, last_input};
use crate::watch::{Watcher, now_millis};

/// How often, while confirming, the pane's rollout is looked up afresh by
/// what its process holds open: a `ps` and an `lsof`, so not each poll.
const REJOIN_EVERY: u32 = 5;

impl Watcher {
    /// Type `composed` into the codex `proven` in `to` and send it, between
    /// turns: Sent once the rollout records it. See this module's docs.
    pub(super) async fn compose_codex(
        &self,
        to: &Terminal,
        composed: &Composition,
        proven: &Proven,
        paths: &[PathBuf],
        unpasted: &mut Unpasted,
    ) -> Result<Turn> {
        let refuse = |what| Err(DomainError::Conflict { what });
        if composed.command.is_some() {
            return refuse("handoff");
        }
        if drawn::opens_picker(&composed.text) {
            return refuse("picker");
        }
        if proven.turn == Turn::During {
            return refuse("busy");
        }
        let (_, columns, rows) = self.service.screen(to.id).await?;
        if !drawn::fits(&composed.text, paths.len(), columns, rows) {
            return refuse("too_tall");
        }
        let pid = proven.pid;
        let before = rollout_now(pid).await;
        let from = before.as_deref().and_then(|p| std::fs::metadata(p).ok()).map_or(0, |m| m.len());
        let preset = proven.preset;
        let runtime = Runtime { marks: None, ..self.service.runtime() };
        let started = now_millis();
        let mut expected = Expected::default();
        let mut held = composer::Composer::Empty;
        let mut first = true;
        for path in paths {
            self.paste_checked(to, &runtime, &proven.tty, preset, std::mem::replace(&mut first, false), &quoted(path)).await?;
            unpasted.0.clear();
            expected = expected.then_image();
            held = self.shown(to, preset, started, &expected, None).await.left(&proven.tty, preset).await?;
        }
        let placeholders = composer::drawn::images(&held);
        if !composed.text.is_empty() {
            let text = if paths.is_empty() { composed.text.clone() } else { format!(" {}", composed.text) };
            self.paste_checked(to, &runtime, &proven.tty, preset, first, &text).await?;
            expected = expected.then_codex_paste(&text);
            self.shown(to, preset, started, &expected, None).await.left(&proven.tty, preset).await?;
        }
        #[cfg(test)]
        if let Some(run) = self.before_enter.lock().unwrap_or_else(|e| e.into_inner()).take() {
            run();
        }
        self.codex_still_between(to, pid, started, &expected, before.is_some()).await?;
        let entered = now_millis();
        runtime.send_bytes_hex(to.id, "0d").await?;
        self.mark_told(to.id);
        let sent = Sent { text: composed.submitted(&placeholders), images: paths.len(), before, from, entered };
        if confirmed(pid, &sent).await { Ok(Turn::Between) } else { refuse("unconfirmed") }
    }

    /// The last check before codex's Enter: nobody typed since the paste
    /// began, the rollout doesn't say a turn runs, and then (last, so the
    /// screen is the newest thing read) a fresh capture reads idle with the
    /// box showing `expected`. `held`: the gate found a rollout, so a lookup
    /// that finds none now is a miss, not a first prompt, and holds too.
    /// Anything else leaves the text in the box: `paste_left`.
    async fn codex_still_between(&self, to: &Terminal, pid: i32, started: i64, expected: &Expected, held: bool) -> Result<()> {
        let left = DomainError::Conflict { what: "paste_left" };
        if last_input(self.service.root_dir(), to.id).is_some_and(|at| at >= started) {
            return Err(left);
        }
        if (held && rollout_now(pid).await.is_none()) || codex_turn::said_of(pid).await == registry_turn::Said::NotIdle {
            return Err(left);
        }
        #[cfg(test)]
        if let Some(run) = self.after_rollout.lock().unwrap_or_else(|e| e.into_inner()).take() {
            run();
        }
        let (screen, _, _) = self.service.screen(to.id).await?;
        if self.service.registry().classify("codex", &screen) != AgentActivity::Idle || !expected.shown_by(&composer::read("codex", &screen)) {
            return Err(left);
        }
        Ok(())
    }
}

/// A message sent, and where its record will be: the rollout held before
/// the Enter, from its length then.
struct Sent {
    text: String,
    images: usize,
    before: Option<PathBuf>,
    from: u64,
    /// When the Enter went (milliseconds since the epoch): a record dated
    /// before it is an earlier message's, whatever its text.
    entered: i64,
}

/// Whether codex recorded `sent` within `CONFIRM_SETTLES`: in the rollout it
/// held before the Enter, past where it ended; or in one it holds now and
/// didn't then, from its start (its first prompt, or the first since `/new`).
async fn confirmed(pid: i32, sent: &Sent) -> bool {
    let deadline = tokio::time::Instant::now() + CONFIRM_SETTLES;
    let mut now: Option<PathBuf> = None;
    let mut polls = 0;
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(super::super::PASTE_POLL).await;
        if sent.before.as_deref().is_some_and(|p| records(p, sent.from, sent)) {
            return true;
        }
        if polls % REJOIN_EVERY == 0 {
            now = rollout_now(pid).await;
        }
        polls += 1;
        if let Some(path) = now.as_deref().filter(|p| Some(*p) != sent.before.as_deref())
            && records(path, 0, sent)
        {
            return true;
        }
    }
    false
}

/// Whether the rollout at `path` records `sent` past byte `from`: its text,
/// whitespace aside (codex puts a space after each image's placeholder), as
/// many images, and dated at or after the Enter (review 1, L1: a rollout read
/// from its start holds earlier messages, maybe the same words).
fn records(path: &Path, from: u64, sent: &Sent) -> bool {
    let squeeze = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let want = squeeze(&sent.text);
    codex_prompts::prompts_from(path, from).iter().any(|p| p.images == sent.images && p.at_ms.is_some_and(|at| at >= sent.entered) && squeeze(&p.text) == want)
}


#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
