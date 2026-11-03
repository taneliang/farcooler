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
use std::time::Duration;

use farcooler_core::composer::{self, codex as drawn, drawn::Expected};
use farcooler_core::session_log::codex_prompts;
use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::AgentActivity;
use farcooler_store::models::Terminal;

use super::super::codex_turn::rollout_now;
use super::super::{Proven, Turn, codex_turn, registry_turn};
use super::{Composition, Unpasted, quoted};
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
            held = self.shown(to, proven, started, &expected, None).await.left(&proven.tty, preset).await?;
        }
        let placeholders = composer::drawn::images(&held);
        if !composed.text.is_empty() {
            let text = if paths.is_empty() { composed.text.clone() } else { format!(" {}", composed.text) };
            self.paste_checked(to, &runtime, &proven.tty, preset, first, &text).await?;
            expected = expected.then_codex_paste(&text);
            self.shown(to, proven, started, &expected, None).await.left(&proven.tty, preset).await?;
        }
        #[cfg(test)]
        if let Some(run) = self.before_enter.lock().unwrap_or_else(|e| e.into_inner()).take() {
            run();
        }
        self.codex_still_between(to, pid, started, &expected, before.is_some() || proven.held).await?;
        let entered = now_millis();
        runtime.send_bytes_hex(to.id, "0d").await?;
        self.mark_told(to.id);
        let sent = Sent { text: composed.submitted(&placeholders), images: paths.len(), before, from, entered };
        if confirmed(pid, &sent, self.timing().compose_confirm).await { Ok(Turn::Between) } else { refuse("unconfirmed") }
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
        if codex_turn::said_held(pid, held).await.0 == registry_turn::Said::NotIdle {
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
pub(in crate::watch::answer_wake) struct Sent {
    pub(in crate::watch::answer_wake) text: String,
    pub(in crate::watch::answer_wake) images: usize,
    pub(in crate::watch::answer_wake) before: Option<PathBuf>,
    pub(in crate::watch::answer_wake) from: u64,
    /// When the Enter went (milliseconds since the epoch): a record dated
    /// before it is an earlier message's, whatever its text.
    pub(in crate::watch::answer_wake) entered: i64,
}

/// Whether codex recorded `sent` within `within` (`CONFIRM_SETTLES`): in the rollout it
/// held before the Enter, past where it ended; or in one it holds now and
/// didn't then, from its start (its first prompt, or the first since `/new`).
async fn confirmed(pid: i32, sent: &Sent, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
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
pub(in crate::watch::answer_wake) fn records(path: &Path, from: u64, sent: &Sent) -> bool {
    let squeeze = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let want = squeeze(&sent.text);
    codex_prompts::prompts_from(path, from).iter().any(|p| p.images == sent.images && p.at_ms.is_some_and(|at| at >= sent.entered) && squeeze(&p.text) == want)
}


impl Watcher {
    /// A scheduled runner notice has its own durable claim and final Enter
    /// admission; it never enters the ordinary wake or harness queues.
    #[cfg(unix)]
    pub(crate) async fn send_check_in(&self, pending: &farcooler_store::PendingCheckIn, daemon: uuid::Uuid) -> Result<()> {
        use std::future::Future;
        // This operation is owned by the joined scheduler. The gate, child
        // and durable attempt identity outlive the panic-catching body.
        let Ok(_cleanup_gate) = self.check_in_cleanup_gate.try_lock() else { return Ok(()) };
        if self.check_in_arm.load(std::sync::atomic::Ordering::SeqCst) != 1 { return Ok(()) }
        let mut child: Option<farcooler_tmux::server::OwnedEnter> = None;
        let mut tracked: Option<(uuid::Uuid, uuid::Uuid)> = None;
        let helpers = std::sync::Arc::new(farcooler_tmux::server::ScheduledHelpers::default());
        let mut body = Box::pin(helpers.clone().scope(self.send_check_in_body(pending, daemon, &mut child, &mut tracked)));
        let outcome = std::future::poll_fn(|cx| {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| body.as_mut().poll(cx))) {
                Ok(std::task::Poll::Ready(value)) => std::task::Poll::Ready(Ok(value)),
                Ok(std::task::Poll::Pending) => std::task::Poll::Pending,
                Err(_) => std::task::Poll::Ready(Err(())),
            }
        }).await;
        drop(body);
        let helpers_reaped = helpers.stop_and_reap().await;
        if !helpers_reaped {
            self.check_in_arm.store(2, std::sync::atomic::Ordering::SeqCst);
            if let Some(child) = child.as_mut() { child.stop_and_reap().await; }
            if let Some((occurrence, attempt)) = tracked {
                let _ = self.service.store.fail_admitted_check_in(occurrence, attempt);
                let _ = self.service.store.fail_claimed_check_in(occurrence, attempt);
            }
            return Err(DomainError::Conflict { what: "check_in_helper_unreaped" });
        }
        match outcome {
            Ok(result) => result,
            Err(()) => {
                self.check_in_arm.store(2, std::sync::atomic::Ordering::SeqCst);
                if let Some((occurrence, attempt)) = tracked {
                    let _ = self.service.store.fail_admitted_check_in(occurrence, attempt);
                    let _ = self.service.store.fail_claimed_check_in(occurrence, attempt);
                }
                if let Some(child) = child.as_mut() { child.stop_and_reap().await; }
                tracing::error!("scheduled check-in panicked; scheduler unavailable");
                Err(DomainError::Conflict { what: "check_in_owner_panicked" })
            }
        }
    }

    #[cfg(unix)]
    async fn send_check_in_body(
        &self, pending: &farcooler_store::PendingCheckIn, daemon: uuid::Uuid,
        child: &mut Option<farcooler_tmux::server::OwnedEnter>,
        tracked: &mut Option<(uuid::Uuid, uuid::Uuid)>,
    ) -> Result<()> {
        use std::os::unix::fs::MetadataExt;
        use farcooler_core::composer::drawn::Expected;
        use crate::claude_registry::{Kernel, Processes};
        let Some(to) = self.service.live_orchestrator(pending.workspace)? else { return Ok(()) };
        if to.pane_mode != farcooler_store::models::PaneMode::Terminal
            || !super::super::may_be_typed_to(&to.command_preset, to.role)
            || !self.service.is_running(&to)
        { return Ok(()) }
        let Some(thread) = to.agent_session_id.as_deref() else { return Ok(()) };
        let Some(_typing) = self.try_typing(to.id) else { return Ok(()) };
        if !matches!(self.ready(&to).await, Ok(Turn::Between)) { return Ok(()) }
        let Ok(proven) = self.proven_tui(&to).await else { return Ok(()) };
        if proven.preset != "codex" || proven.turn != Turn::Between { return Ok(()) }
        let Some(process_started) = Kernel.started(proven.pid) else { return Ok(()) };
        if codex_turn::said_held(proven.pid, true).await.0 != registry_turn::Said::Idle { return Ok(()) }
        let Some(path) = rollout_now(proven.pid).await else { return Ok(()) };
        if crate::log_join::codex_thread_of(&path).is_none_or(|(id, _)| id != thread) { return Ok(()) }
        let mut file = match std::fs::File::open(&path) { Ok(file) => file, Err(_) => return Ok(()) };
        if farcooler_core::session_log::codex_prompts::main_thread_in(&mut file).as_deref() != Some(thread) { return Ok(()) }
        let meta = match file.metadata() { Ok(meta) => meta, Err(_) => return Ok(()) };
        let (Ok(device), Ok(inode), Ok(offset)) = (i64::try_from(meta.dev()), i64::try_from(meta.ino()), i64::try_from(meta.len())) else { return Ok(()) };
        let Some(path_text) = path.to_str() else { return Ok(()) };
        let task = self.service.store.get_task(pending.task)?;
        let text = farcooler_store::fixed_check_in_message(&task.key, pending.id);
        let (_, columns, rows) = self.service.screen(to.id).await?;
        if drawn::opens_picker(&text) || !drawn::fits(&text, 0, columns, rows) { return Ok(()) }
        let runtime = Runtime { marks: None, ..self.service.runtime() };
        let pane = runtime.pane_id_of(to.id)?;
        let attempt = uuid::Uuid::now_v7();
        if !self.service.store.claim_check_in(pending.id, attempt, daemon, to.id, thread, &pane, proven.pid, process_started, now_millis())? { return Ok(()) }
        *tracked = Some((pending.id, attempt));
        let prepared = async {
            if !self.service.store.pin_check_in_rollout(pending.id, attempt, path_text, device, inode, offset)? {
                return Err(DomainError::Conflict { what: "unconfirmed" });
            }
            let Some(stored) = self.service.store.claimed_check_in_message(pending.id, attempt)? else {
                return Err(DomainError::Conflict { what: "unconfirmed" });
            };
            if stored != text { return Err(DomainError::Conflict { what: "unconfirmed" }) }
            let started = now_millis();
            self.paste_checked(&to, &runtime, &proven.tty, "codex", true, &text).await?;
            let expected = Expected::default().then_codex_paste(&text);
            self.shown(&to, &proven, started, &expected, None).await.left(&proven.tty, "codex").await?;
            let (preset, _, pid) = self.proven_agent(&to).await.map_err(|_| DomainError::Conflict { what: "unconfirmed" })?;
            if preset != "codex" || pid != proven.pid || Kernel.started(pid) != Some(process_started) || runtime.pane_id_of(to.id)? != pane {
                return Err(DomainError::Conflict { what: "unconfirmed" });
            }
            if rollout_now(pid).await.as_deref() != Some(path.as_path())
                || codex_turn::said_held(pid, true).await.0 != registry_turn::Said::Idle
            {
                return Err(DomainError::Conflict { what: "unconfirmed" });
            }
            let now_meta = std::fs::metadata(&path).map_err(|_| DomainError::Conflict { what: "unconfirmed" })?;
            if now_meta.dev() != meta.dev() || now_meta.ino() != meta.ino() { return Err(DomainError::Conflict { what: "unconfirmed" }) }
            self.codex_still_between(&to, pid, started, &expected, true).await?;
            Ok(())
        }.await;
        if let Err(error) = prepared {
            self.service.store.fail_claimed_check_in(pending.id, attempt)?;
            return Err(error);
        }
        if !self.service.store.admit_check_in_enter(pending.id, attempt, daemon, to.id, thread, &pane, proven.pid, process_started, now_millis())? {
            self.service.store.fail_claimed_check_in(pending.id, attempt)?;
            return Ok(());
        }
        // The fence stays durable if this future is cancelled or the client
        // fails: no timeout is proof that tmux did not accept the key.
        let enter = match runtime.tmux.spawn_enter_once(&pane).await {
            Ok(enter) => enter,
            Err(error) => { self.service.store.fail_admitted_check_in(pending.id, attempt)?; return Err(error); }
        };
        *child = Some(enter);
        #[cfg(test)]
        if self.panic_after_check_in_spawn.swap(false, std::sync::atomic::Ordering::SeqCst) {
            panic!("injected scheduled sender panic after spawn");
        }
        match child.as_mut().expect("owned Enter was just installed").wait().await {
            Ok(()) => {
                if !self.service.store.settle_check_in_enter(pending.id, attempt, now_millis())? {
                    return Err(DomainError::Conflict { what: "unconfirmed" });
                }
                self.mark_told(to.id);
                Ok(())
            }
            Err(error) => {
                self.check_in_arm.store(2, std::sync::atomic::Ordering::SeqCst);
                self.service.store.fail_admitted_check_in(pending.id, attempt)?;
                Err(error)
            }
        }
    }
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
