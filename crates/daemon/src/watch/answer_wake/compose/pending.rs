//! A composed message pasted into claude's box whose Enter waits out a turn
//! claude began after the paste, with no fence to be had yet (ov-808), for
//! as long as the owner's orchestrator's turns run (`TURN_BEGUN_WAIT`,
//! ov-816).
//!
//! **Off the lock.** The send is answered Queued at once and the Enter left
//! to a task of its own, which takes the pane's typing lock only for each
//! try (`try_typing`, never waiting on it) and lets it go between tries. So
//! Stop, Bring Here and a phone's keys, which queue behind a pane's call on
//! its connection and behind the typing lock on the runner, never wait on a
//! turn (ov-808 review, S1). The pane's rows carry the message as Queued
//! while it waits, as a held message's do (`held`).
//!
//! **Each try**, every `PASTE_POLL` for the first `SLOWS_AFTER` and every
//! `SLOW_POLL` after, until `turn_begun_wait`: the read-back of
//! `still_between` again (`between_or_why`), claude's config directory read
//! from its process once for the whole wait, and then
//! - a turn still running: the box must still show the message (cleared or
//!   changed in the terminal, it's given up at once, not at the turn's end);
//!   the fenced Enter once the hooks give a witness;
//! - a dialog drawn: waited out, as a turn is. Neither Enter can take it: the
//!   plain one needs two reads between turns, the fenced one is never tried
//!   on it;
//! - a key typed in Far Cooler's terminal (an input mark): not the end of
//!   the wait, since it may be a dialog's answer, from the Mac or the phone,
//!   drawn and gone between two slow reads. Typing in the box shows on the
//!   next read, which gives it up. Either Enter waits until nobody has typed
//!   for `TYPED_WITHIN_MS`, as a send's gate does;
//! - between turns twice running, `IDLE_TWICE` apart, with nothing read
//!   between but that: the plain Enter. A turn has been seen, and one stray
//!   frame read Idle mid-turn must not send a key with no fence (S2);
//! - anything else (the box changed, the agent gone): left.
//!
//! **The gate's agents** are trusted for `AGENTS_TRUSTED_FOR` only. Beside
//! them claude's registry `busy` reads as between turns, and a tall box on
//! a short pane can hide their rows (ov-808), so only the spinner would say
//! a turn runs. Within half a minute the agents seen at the gate still run;
//! over ten, they finish, and claude starts a turn to report each. Past it,
//! only rows on the screen say agents run, and a `busy` beside none waits
//! for the registry's `idle` or `shell`: a later Enter, never one into a
//! turn (ov-808 re-review, 3c).
//!
//! **A bad read** is no verdict. A capture tmux didn't answer, a registry
//! file caught mid-write is tried again on the next tick, and only a run of them lasting
//! `Timing::torn_cap` (10 s, as a held message's, `held::TORN_FOR`) ends the
//! wait: a write is over in milliseconds and tmux answers in a second, so ten
//! seconds is many consecutive bad reads, and still a small part of ten
//! minutes. A good read ends the run. A shell, or another process, in front
//! of the pane is no bad read: the box is gone from the screen (a screen
//! reads as claude's by the pane's command, so a shell's reads between turns
//! with no box) and claude from the pane, said as `not_running`, not as a box
//! changed (`pended`).
//!
//! **Given up.** A Stop pressed since the message was queued leaves it, as
//! the person stopped the turn it waited on (Send Now is refused while the
//! box holds anything, `interrupt::pressable`, so it never reaches here).
//! So does a turn claude's transcript says the person stopped some other
//! way: a permission dialog answered No or Esc, which claude 2.1.309 ends
//! as `[Request interrupted by user for tool use]`, or Esc typed straight
//! in the terminal (ov-816 review, S1). So do the wait running out (said
//! as the agents' when the gate saw agents and only the registry stayed
//! busy, `stayed`), the box changing, and the agent going. Each is logged as a paste left in the
//! box, with its check, and the pane's rows say which, in words of its own (`held::why_not_sent`). A box
//! that emptied, or a key typed, may be the message sent by hand, from the
//! Mac or straight in tmux: claude's transcript is read for it first, and if
//! it's there the Queued row is taken back for claude's own.
//!
//! **Kept.** The wait is a row in the store (`Store::keep_compose_wait`)
//! from before it begins until it ends, so a runner restarted during it
//! picks it up where it was (`resume_compose_waits`): sent meanwhile, its
//! row is taken back; the same claude still there (its pid and its session), the wait goes on, its
//! deadline counted from when it was queued, or given up at once if that
//! ran out while the runner was down; another claude, or none, says so. A
//! wait begun since this start, before its first pump, is running already
//! and isn't started twice.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use farcooler_core::activity::Background;
use farcooler_core::composer::{self, Composer, drawn::Expected};
use farcooler_store::models::Terminal;
use uuid::Uuid;

use super::super::{PASTE_POLL, Proven, Turn, interrupt, mid_turn, registry_turn};
use super::{Confirm, Left, held, left_in_box};
use crate::runtime::{Runtime, last_input};
use crate::watch::{Watcher, now_millis};

/// How far apart two reads between turns must be, with only such reads
/// between them, before a waiting message's plain Enter (S2;
/// `Timing::idle_twice`).
pub(crate) const IDLE_TWICE: Duration = Duration::from_millis(300);

/// How long a waiting message reads the pane every `PASTE_POLL`: a short
/// turn's worth. After it, every `SLOW_POLL`, so a ten-minute wait takes
/// about 870 reads, not 6,000 (ov-808 re-review, 3b).
pub(crate) const SLOWS_AFTER: Duration = Duration::from_secs(30);

/// How often a waiting message reads the pane past `SLOWS_AFTER`. Above
/// `IDLE_TWICE`, so two reads between turns are two polls.
pub(crate) const SLOW_POLL: Duration = Duration::from_secs(1);

/// How long a waiting message trusts the background agents its gate saw
/// (this module's docs, "The gate's agents").
pub(crate) const AGENTS_TRUSTED_FOR: Duration = Duration::from_secs(30);

/// One message waiting for its Enter, in `Tellers::pending` by a token of
/// its own, so one wait ending never clears another's.
pub(super) struct Waiting {
    pub(super) terminal: Uuid,
    /// Stop was pressed in its pane since it was queued.
    stopped: bool,
    /// Its task, for a test to end as a runner stopping would.
    #[cfg(test)]
    task: Option<tokio::task::AbortHandle>,
}

/// A message in claude's box, its Enter waiting out a turn.
pub(super) struct Pending {
    pub(super) to: Terminal,
    pub(super) proven: Proven,
    /// When the paste began (`last_input`'s clock).
    pub(super) started: i64,
    pub(super) expected: Expected,
    pub(super) confirm: Confirm,
    /// The message as the pane's rows show it.
    pub(super) echo: String,
    /// The check that found the turn.
    pub(super) left: Left,
    /// claude's config directory, read from its process once.
    pub(super) config: PathBuf,
    /// When it was answered Queued (Unix ms): the wait's deadline, its
    /// slowing and its trust in the gate's agents count from here.
    pub(super) queued_at: i64,
    /// What was pasted, to draw `expected` again after a restart.
    pub(super) drawn: Drawn,
}

/// What was pasted into the box, in order: the images, then each paste.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Drawn {
    pub(super) images: usize,
    pub(super) pastes: Vec<String>,
}

impl Drawn {
    /// What the box shows for these pastes, as `compose_locked` built it.
    fn expected(&self) -> Expected {
        let images = (0..self.images).fold(Expected::default(), |e, _| e.then_image());
        self.pastes.iter().fold(images, |e, paste| e.then_paste(paste))
    }
}

/// A wait as the store keeps it (`Store::keep_compose_wait`).
#[derive(serde::Serialize, serde::Deserialize)]
struct Kept {
    pid: i32,
    /// The gate's background work: `None`, `Shells` or `Agents`.
    background: String,
    started: i64,
    queued_at: i64,
    drawn: Drawn,
    echo: String,
    session: String,
    transcript: PathBuf,
    from: u64,
    queued_before: bool,
    submitted: String,
}

impl Kept {
    fn of(p: &Pending) -> Kept {
        Kept {
            pid: p.proven.pid,
            background: format!("{:?}", p.proven.background),
            started: p.started,
            queued_at: p.queued_at,
            drawn: p.drawn.clone(),
            echo: p.echo.clone(),
            session: p.confirm.session.clone(),
            transcript: p.confirm.transcript.clone(),
            from: p.confirm.from,
            queued_before: p.confirm.queued_before,
            submitted: p.confirm.submitted.clone(),
        }
    }

    fn background(&self) -> Background {
        match self.background.as_str() {
            "Agents" => Background::Agents,
            "Shells" => Background::Shells,
            _ => Background::None,
        }
    }
}

/// The message's words as a view's echo draws them: each image as
/// `[Image]`, then the text (`held`'s echo).
pub(super) fn echo(images: usize, raw: &str) -> String {
    let mut parts = vec!["[Image]".to_string(); images];
    if !raw.trim().is_empty() {
        parts.push(raw.trim().to_string());
    }
    parts.join(" ")
}

impl Watcher {
    /// Keep `pending`, show it on its pane's rows as Queued, and wait out
    /// its turn on a task of its own. The row, the store's and the pane's
    /// place in `Tellers::pending` go when the wait ends, however it ends
    /// (`Settle`).
    pub(super) fn pend_enter(&self, pending: Pending) {
        let token = Uuid::now_v7();
        let kept = serde_json::to_string(&Kept::of(&pending)).unwrap_or_default();
        if let Err(e) = self.service.store.keep_compose_wait(token, pending.to.id, &kept) {
            tracing::warn!(terminal = %pending.to.id, error = %e, "a waiting message couldn't be kept across a restart");
        }
        let (terminal, echo, preset) = (pending.to.id, pending.echo.clone(), pending.to.command_preset.clone());
        self.wait_out(token, terminal, &echo, pending.queued_at, &preset, move |me| async move {
            let entered = me.enter_when_it_can(&pending, token).await;
            me.pended(&pending, &row_of(token), entered).await
        });
    }

    /// Once, after a start: every wait a restart cut short, picked up again
    /// (this module's docs, "Kept").
    pub(super) fn resume_compose_waits(&self) {
        for wait in self.service.store.compose_waits().unwrap_or_default() {
            // Pended since this start, before its first pump: running already.
            if self.composed_held.lock().unwrap_or_else(|e| e.into_inner()).pending.contains_key(&wait.id) {
                continue;
            }
            let (Ok(kept), Ok(to)) = (serde_json::from_str::<Kept>(&wait.wait), self.service.store.get_terminal(wait.terminal)) else {
                let _ = self.service.store.drop_compose_wait(wait.id);
                continue;
            };
            let (token, terminal, echo, preset) = (wait.id, to.id, kept.echo.clone(), to.command_preset.clone());
            self.wait_out(token, terminal, &echo, kept.queued_at, &preset, move |me| async move {
                match me.wait_resumed(to, kept).await {
                    Ok(pending) => {
                        let entered = me.enter_when_it_can(&pending, token).await;
                        me.pended(&pending, &row_of(token), entered).await
                    }
                    // Sent meanwhile: claude's own rows show it.
                    Err(None) => {
                        crate::session_projectors::global().runner_hold(terminal, |r| r.runner_told(&row_of(token)));
                        None
                    }
                    Err(Some(why)) => Some(why),
                }
            });
        }
    }

    /// Show the wait `token` on `terminal`'s rows as Queued, note it in
    /// `Tellers::pending`, and run `wait` on a task of its own: what the
    /// rows say once it ends, `None` for claude's own (`Settle`).
    fn wait_out<F, Fut>(&self, token: Uuid, terminal: Uuid, echo: &str, queued_at: i64, preset: &str, wait: F)
    where
        F: FnOnce(Arc<Watcher>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Option<String>> + Send + 'static,
    {
        let Some(me) = self.me.upgrade() else { return };
        let row = row_of(token);
        crate::session_projectors::global().runner_hold(terminal, |p| p.runner_held(&row, echo, queued_at));
        let waiting = Waiting {
            terminal,
            stopped: false,
            #[cfg(test)]
            task: None,
        };
        self.composed_held.lock().unwrap_or_else(|e| e.into_inner()).pending.insert(token, waiting);
        let why = held::why_not_sent(held::agent_name(preset), echo, "enter_failed");
        let mut settle = Settle { me: Arc::clone(&me), token, terminal, row, why: Some(why), ended: false };
        let _task = tokio::spawn(async move {
            let why = wait(Arc::clone(&settle.me)).await;
            settle.says(why);
        });
        #[cfg(test)]
        if let Some(waiting) = self.composed_held.lock().unwrap_or_else(|e| e.into_inner()).pending.get_mut(&token) {
            waiting.task = Some(_task.abort_handle());
        }
    }

    /// A wait kept across a restart, ready to go on: `Err(None)` when claude
    /// took the message meanwhile, `Err(Some(why))` when it can't go on.
    async fn wait_resumed(&self, to: Terminal, kept: Kept) -> Result<Pending, Option<String>> {
        if mid_turn::recorded_as(&kept.transcript, kept.from, &kept.submitted, kept.queued_before).is_some() {
            return Err(None);
        }
        let agent = held::agent_name(&to.command_preset);
        let gone = |what| Some(held::why_not_sent(agent, &kept.echo, what));
        let Ok((preset, tty, pid)) = self.proven_agent(&to).await else { return Err(gone("not_running")) };
        if pid != kept.pid {
            return Err(gone("agent_restarted"));
        }
        // Its whole wait ran out while the runner was down: nothing was
        // tried, and claude was never read.
        if now_millis() - kept.queued_at >= i64::try_from(self.timing().turn_begun_wait.as_millis()).unwrap_or(i64::MAX) {
            return Err(gone("runner_down"));
        }
        let Some(config) = registry_turn::config_of(pid).await else { return Err(gone("not_running")) };
        // The pid can come round again; the session it's running can too
        // change (a /clear, a resume). Either way the transcript kept is
        // another conversation's (ov-823, N4).
        match mid_turn::transcript_in(&config, pid) {
            Some((session, _)) if session == kept.session => {}
            Some(_) => return Err(gone("agent_restarted")),
            None => return Err(gone("not_running")),
        }
        let proven = Proven { preset, tty, pid, turn: Turn::During, held: false, background: kept.background() };
        let confirm = Confirm {
            session: kept.session,
            transcript: kept.transcript,
            from: kept.from,
            queued_before: kept.queued_before,
            since: Instant::now(),
            submitted: kept.submitted,
        };
        let left = Left { check: "resumed", screen: None, turn: true, dialog: false };
        let expected = kept.drawn.expected();
        let (started, queued_at, echo, drawn) = (kept.started, kept.queued_at, kept.echo, kept.drawn);
        Ok(Pending { to, proven, started, expected, confirm, echo, left, config, queued_at, drawn })
    }

    /// Whether a message pasted into `terminal` waits for its Enter: a send
    /// meanwhile is held behind it (`compose_or_hold`).
    pub(super) fn enter_pending(&self, terminal: Uuid) -> bool {
        self.composed_held.lock().unwrap_or_else(|e| e.into_inner()).pending.values().any(|w| w.terminal == terminal)
    }

    /// Stop was pressed in `terminal`: every message waiting there gives up
    /// on its next try, unentered (`interrupt::press`).
    pub(in crate::watch::answer_wake) fn stopped_while_waiting(&self, terminal: Uuid) {
        for waiting in self.composed_held.lock().unwrap_or_else(|e| e.into_inner()).pending.values_mut() {
            if waiting.terminal == terminal {
                waiting.stopped = true;
            }
        }
    }

    fn stopped_since_queued(&self, token: Uuid) -> bool {
        self.composed_held.lock().unwrap_or_else(|e| e.into_inner()).pending.get(&token).is_some_and(|w| w.stopped)
    }

    /// Whether `p`'s claude is no longer the pane's agent: a shell in front,
    /// or another process.
    async fn agent_gone(&self, p: &Pending) -> bool {
        self.proven_agent(&p.to).await.map_or(true, |(_, _, pid)| pid != p.proven.pid)
    }

    /// End every wait's task as a runner stopping does, its row kept in the
    /// store, and have the next pump pick them up (`resume_compose_waits`).
    #[cfg(test)]
    pub(crate) fn stop_waits_for_tests(&self) {
        let tasks: Vec<_> = {
            let mut tellers = self.composed_held.lock().unwrap_or_else(|e| e.into_inner());
            tellers.forget_resumed_for_tests();
            tellers.pending.values_mut().filter_map(|w| w.task.take()).collect()
        };
        for task in tasks {
            task.abort();
        }
    }

    /// Try `p`'s Enter until it goes or must be given up (this module's
    /// docs): the word it's given up with (`held::why_not_sent`).
    async fn enter_when_it_can(&self, p: &Pending, token: Uuid) -> Result<(), &'static str> {
        let registry = self.service.registry();
        let waited = || Duration::from_millis(u64::try_from(now_millis() - p.queued_at).unwrap_or(0));
        // Past `carry_agents_for`, only the screen says agents run.
        let uncarried = Proven { tty: p.proven.tty.clone(), background: Background::None, ..p.proven };
        let mut idle_since: Option<Instant> = None;
        // Keys typed since the paste, as far as they've been read: each
        // holds the Enter back until nobody has typed for `TYPED_WITHIN_MS`.
        let mut typed_since = p.started;
        let mut typed = false;
        // When the run of bad reads now going began.
        let mut unread_since: Option<Instant> = None;
        let mut last = Left { check: p.left.check, screen: p.left.screen.clone(), turn: true, dialog: p.left.dialog };
        loop {
            let timing = self.timing();
            tokio::time::sleep(if waited() < timing.wait_slows_after { PASTE_POLL } else { timing.wait_poll }).await;
            #[cfg(test)]
            tried(p.to.id);
            if self.stopped_since_queued(token) {
                left_in_box(&registry, &p.to, "stopped", p.proven.background, None);
                return Err("stopped");
            }
            if waited() >= timing.turn_begun_wait {
                last.logged(&registry, &p.to, &p.proven);
                return Err(stayed(&last, &p.proven));
            }
            // Never wait on the box: Stop and Bring Here take it.
            let Some(_typing) = self.try_typing(p.to.id) else {
                idle_since = None;
                continue;
            };
            let proven = if waited() < timing.carry_agents_for { &p.proven } else { &uncarried };
            #[cfg(test)]
            let read = if fault(p.to.id) {
                Err(farcooler_core::DomainError::TmuxTimedOut)
            } else {
                self.between_or_why(&p.to, proven, typed_since, &p.expected, false, Some(&p.config)).await
            };
            #[cfg(not(test))]
            let read = self.between_or_why(&p.to, proven, typed_since, &p.expected, false, Some(&p.config)).await;
            // A read that failed, or found nothing to judge, is tried again
            // on the next tick, and only a run of them lasting `torn_cap`
            // ends the wait (this module's docs, "A bad read").
            match &read {
                Err(e) => {
                    let first = *unread_since.get_or_insert_with(Instant::now);
                    if first.elapsed() >= timing.torn_cap {
                        tracing::warn!(terminal = %p.to.id, error = %e, "a waiting message gave up: the pane stayed unreadable");
                        return Err("not_running");
                    }
                    idle_since = None;
                    continue;
                }
                Ok(Some(left)) if left.check == "registry_torn" => {
                    let first = *unread_since.get_or_insert_with(Instant::now);
                    if first.elapsed() >= timing.torn_cap {
                        left.logged(&registry, &p.to, &p.proven);
                        return Err(given_up_for(left, p.proven.preset, typed));
                    }
                    idle_since = None;
                    continue;
                }
                _ => unread_since = None,
            }
            let quiet = last_input(self.service.root_dir(), p.to.id).is_none_or(|at| now_millis() - at >= interrupt::TYPED_WITHIN_MS);
            match read {
                Err(_) => return Err("not_running"),
                // A key typed: a dialog's answer, one that changed nothing,
                // or typing in the box, which the next read sees.
                Ok(Some(left)) if left.check == "typed" => {
                    typed_since = last_input(self.service.root_dir(), p.to.id).map_or(typed_since, |at| at + 1);
                    typed = true;
                    idle_since = None;
                }
                Ok(None) if !quiet => idle_since = None,
                // The turn it waited on ended as the person stopped it: a
                // dialog answered No or Esc, or Esc typed straight in the
                // terminal. As with Stop, it's theirs to send (ov-816 S1).
                Ok(None) if mid_turn::interrupted_since(&p.confirm.transcript, p.confirm.from) => {
                    left_in_box(&registry, &p.to, "interrupted", p.proven.background, None);
                    return Err("interrupted");
                }
                Ok(None) => {
                    let first = *idle_since.get_or_insert_with(Instant::now);
                    if first.elapsed() >= timing.idle_twice {
                        let runtime = Runtime { marks: None, ..self.service.runtime() };
                        return runtime.send_bytes_hex(p.to.id, "0d").await.map_err(|_| "enter_failed");
                    }
                }
                Ok(Some(left)) if left.dialog => {
                    idle_since = None;
                    last = left;
                }
                Ok(Some(left)) if !left.turn => {
                    left.logged(&registry, &p.to, &p.proven);
                    return Err(given_up_for(&left, p.proven.preset, typed));
                }
                Ok(Some(left)) => {
                    idle_since = None;
                    // The box is read mid-turn too: cleared or changed in the
                    // terminal, it's said at once, not at the turn's end.
                    if left.screen.as_deref().is_some_and(|screen| !p.expected.shown_on(p.proven.preset, screen)) {
                        let gone = Left { check: "not_shown", turn: false, ..left };
                        gone.logged(&registry, &p.to, &p.proven);
                        return Err(given_up_for(&gone, p.proven.preset, typed));
                    }
                    if !quiet {
                        last = left;
                        continue;
                    }
                    if let Some(witness) = self.witness_in(&p.config, proven, &p.to, &p.confirm.submitted) {
                        match self.enter_fenced_for(&p.to, proven, typed_since, &witness, &p.expected).await {
                            Ok(()) => return Ok(()),
                            // Nothing was pressed: a call or a gate the hooks
                            // say a dialog may follow, waited out as a turn is.
                            Err("dialog") => {}
                            Err("paste_left") => return Err("box_changed"),
                            Err(_) => return Err("enter_failed"),
                        }
                    }
                    last = left;
                }
            }
        }
    }

    /// What became of `p`'s Enter: `None` once claude says it took it, and
    /// its own rows show it; else what the pane's rows say instead. A key
    /// typed or a box changed may be the message sent by hand: if claude's
    /// transcript has it within `compose_confirm`, the row goes to claude's.
    async fn pended(&self, p: &Pending, row: &str, entered: Result<(), &'static str>) -> Option<String> {
        let agent = held::agent_name(&p.to.command_preset);
        match entered {
            Ok(()) => {
                self.mark_told(p.to.id);
                let confirm = Confirm { since: Instant::now(), ..p.confirm.clone() };
                let took = self.took(&confirm).await;
                crate::session_projectors::global().runner_hold(p.to.id, |r| r.runner_told(row));
                took.is_none().then(|| held::why_not_sent(agent, &p.echo, "unconfirmed"))
            }
            Err(what @ ("typed" | "box_changed" | "box_cleared")) => {
                if self.recorded_within(&p.confirm).await {
                    crate::session_projectors::global().runner_hold(p.to.id, |r| r.runner_told(row));
                    return None;
                }
                // A box gone from the screen may be claude gone from the pane.
                let what = if matches!(what, "box_changed" | "box_cleared") && self.agent_gone(p).await { "not_running" } else { what };
                Some(held::why_not_sent(agent, &p.echo, what))
            }
            Err(what) => Some(held::why_not_sent(agent, &p.echo, what)),
        }
    }

    /// Whether claude's transcript records `c`'s message, queued or as a
    /// prompt, within `compose_confirm`.
    async fn recorded_within(&self, c: &Confirm) -> bool {
        let deadline = tokio::time::Instant::now() + self.timing().compose_confirm;
        loop {
            if mid_turn::recorded_as(&c.transcript, c.from, &c.submitted, c.queued_before).is_some() {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(PASTE_POLL).await;
        }
    }
}

/// The wait `token`'s Queued row.
fn row_of(token: Uuid) -> String {
    format!("pending:{token}")
}

/// The word a wait that ran out is given up with, by the last thing read.
/// A dialog still up is `stayed_blocked`. Past the gate's agents, a main loop
/// the screen shows between turns while the registry stays `busy` is
/// background work's, and if the gate saw agents it's theirs: `stayed_agents`
/// says so, not that claude stayed busy (ov-816 review, N3). A spinner
/// drawn, or no agents ever seen, is a turn: `stayed_busy`.
fn stayed(last: &Left, gate: &Proven) -> &'static str {
    if last.dialog {
        "stayed_blocked"
    } else if gate.background == Background::Agents && matches!(last.check, "registry_busy" | "registry_busy_no_background") {
        "stayed_agents"
    } else {
        "stayed_busy"
    }
}

/// The word a wait is given up with for `still_between`'s check `left`,
/// `typed` when a key was typed in Far Cooler's terminal since the paste.
fn given_up_for(left: &Left, preset: &str, typed: bool) -> &'static str {
    match left.check {
        // Not between turns, working or a dialog: a screen not claude's.
        "not_idle" => "unreadable",
        "registry_none" => "not_running",
        "registry_torn" => "unchecked",
        "not_shown" if typed => "typed",
        "not_shown" if left.screen.as_deref().is_some_and(|screen| composer::read(preset, screen) == Composer::Empty) => "box_cleared",
        _ => "box_changed",
    }
}

/// Reads a test has the waits of a pane fail, as a tmux that timed out
/// does, counted down by each try (`fault`).
#[cfg(test)]
static FAULTS: std::sync::Mutex<Option<std::collections::HashMap<Uuid, usize>>> = std::sync::Mutex::new(None);

/// Have the next `reads` tries of `terminal`'s waits fail to read the pane.
#[cfg(test)]
pub(crate) fn fail_reads(terminal: Uuid, reads: usize) {
    FAULTS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).insert(terminal, reads);
}

/// Whether this try's read is to fail (`fail_reads`).
#[cfg(test)]
fn fault(terminal: Uuid) -> bool {
    let mut faults = FAULTS.lock().unwrap_or_else(|e| e.into_inner());
    match faults.as_mut().and_then(|m| m.get_mut(&terminal)) {
        Some(left) if *left > 0 => {
            *left -= 1;
            true
        }
        _ => false,
    }
}

/// How many times each pane's wait has read it, for a test that the wait
/// slows (ov-816).
#[cfg(test)]
static TRIES: std::sync::Mutex<Option<std::collections::HashMap<Uuid, usize>>> = std::sync::Mutex::new(None);

#[cfg(test)]
fn tried(terminal: Uuid) {
    *TRIES.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).entry(terminal).or_default() += 1;
}

/// How many times a wait has read `terminal` (`TRIES`).
#[cfg(test)]
pub(crate) fn tries(terminal: Uuid) -> usize {
    TRIES.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&terminal).copied()).unwrap_or(0)
}

/// A wait's end. Ended (`says`), or by a panic: its place in
/// `Tellers::pending` and its row in the store go, and its Queued row says
/// `why`, or is left to claude's own rows when `why` is `None`. Cut short,
/// as a runner stopping drops its tasks: only its place in memory goes, and
/// the next start picks it up from the store (`resume_compose_waits`).
struct Settle {
    me: Arc<Watcher>,
    token: Uuid,
    terminal: Uuid,
    row: String,
    why: Option<String>,
    ended: bool,
}

impl Settle {
    /// What the row says when the wait ends: `None` for claude's own rows.
    fn says(&mut self, why: Option<String>) {
        self.why = why;
        self.ended = true;
    }
}

impl Drop for Settle {
    fn drop(&mut self) {
        self.me.composed_held.lock().unwrap_or_else(|e| e.into_inner()).pending.remove(&self.token);
        if !self.ended && !std::thread::panicking() {
            return;
        }
        let _ = self.me.service.store.drop_compose_wait(self.token);
        if let Some(why) = &self.why {
            let row = &self.row;
            crate::session_projectors::global().runner_hold(self.terminal, |r| r.runner_dropped(row, why, now_millis()));
        }
    }
}
