//! The watcher's waits, as one value (ov-536).
//!
//! Each is a production constant that keeps its own name and its own reason
//! where it is used; `Timing::PRODUCTION` gathers them, and every `Watcher`
//! the daemon builds takes exactly that. A test that would otherwise wait one
//! out in real time sets a shorter one on its watcher instead
//! (`Watcher::set_timing`). Not a paused tokio clock: these waits run beside
//! real tmux, perl and git, which a paused clock reads as idle, so it would
//! jump straight past every deadline.
//!
//! Only a wait a test spends in full belongs here: a window the negative case
//! waits out (nothing confirmed it), a hold after a failure, a spacing or a
//! retry. A test that leaves one alone keeps the production value, which is
//! also the bound its positive case returns early from.

use std::time::Duration;

/// How long each of the watcher's waits lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Timing {
    /// How long after the Enter a composed message has to be confirmed by
    /// claude's hook or transcript, or codex's rollout (`compose`'s
    /// `CONFIRM_SETTLES`).
    pub compose_confirm: Duration,
    /// How long a compose whose turn claude began between its paste and its
    /// Enter reads the pane every `wait_poll`, before it parks
    /// (`compose::TURN_BEGUN_WAIT`, ov-827).
    pub turn_begun_wait: Duration,
    /// How often a parked message reads the pane when the watcher hasn't
    /// woken it first (`compose::pending::PARKED_POLL`, ov-827).
    pub parked_poll: Duration,
    /// The longest a message waits in claude's box for its Enter, parked
    /// included, before it's given up (`compose::pending::LEFT_KEPT`, ov-827).
    pub left_kept: Duration,
    /// How far apart a waiting message's two reads between turns are before
    /// its plain Enter (`compose::pending::IDLE_TWICE`).
    pub idle_twice: Duration,
    /// How long a waiting message reads the pane every `PASTE_POLL` before
    /// it reads it every `wait_poll` (`compose::pending::SLOWS_AFTER`).
    pub wait_slows_after: Duration,
    /// How often a waiting message reads the pane once it has slowed
    /// (`compose::pending::SLOW_POLL`).
    pub wait_poll: Duration,
    /// How long a waiting message trusts the background agents its gate saw
    /// (`compose::pending::AGENTS_TRUSTED_FOR`).
    pub carry_agents_for: Duration,
    /// How long after a key claude has to say it took it (`interrupt`'s
    /// `CONFIRM_SETTLES`).
    pub key_confirm: Duration,
    /// How long after a mid-turn Enter the queue has to show the message
    /// (`mid_turn::QUEUE_SETTLES`).
    pub queue_settles: Duration,
    /// The longest a mid-turn Enter's `tmux send-keys` may take
    /// (`mid_turn::ENTER_DEADLINE`).
    pub enter_deadline: Duration,
    /// How long the fence stays held after a send that failed or overran
    /// (`mid_turn::LATE_KEY`).
    pub late_key: Duration,
    /// The longest a key or a clear waits for another send to let go of the
    /// box (`interrupt::TYPING_WAIT`; a clear, `bring`, waits the same).
    pub typing_wait: Duration,
    /// The least time between two keys pressed in one terminal
    /// (`interrupt::LOCKOUT`).
    pub key_lockout: Duration,
    /// How long a pasted answer or message has to show in the box before
    /// it's judged not to have (`answer_wake::PASTE_SETTLES`).
    pub paste_settles: Duration,
    /// How long after telling a terminal one thing the next may be told
    /// (`answer_wake::TOLD_SPACING_MS`).
    pub told_spacing: Duration,
    /// How often a pane's oldest held message tries the gate again
    /// (`compose::held::RETRY`).
    pub held_retry: Duration,
    /// How long a held message waits out a registry that reads torn
    /// (`compose::held::TORN_FOR`), and a waiting message a run of reads that
    /// failed (`compose::pending`, "A bad read").
    pub torn_cap: Duration,
    /// The count notice's window (`COUNT_NOTICE_EVERY`).
    pub count_notice_every: Duration,
    /// How long a message codex's queue took is watched for in its rollout
    /// (`answer_wake::queued::WATCH_FOR`).
    pub queue_watch: Duration,
}

impl Timing {
    /// What the daemon runs with: each constant as its module defines it.
    pub(crate) const PRODUCTION: Timing = Timing {
        compose_confirm: super::answer_wake::compose::CONFIRM_SETTLES,
        turn_begun_wait: super::answer_wake::compose::TURN_BEGUN_WAIT,
        parked_poll: super::answer_wake::compose::PARKED_POLL,
        left_kept: super::answer_wake::compose::LEFT_KEPT,
        idle_twice: super::answer_wake::compose::IDLE_TWICE,
        wait_slows_after: super::answer_wake::compose::SLOWS_AFTER,
        wait_poll: super::answer_wake::compose::SLOW_POLL,
        carry_agents_for: super::answer_wake::compose::AGENTS_TRUSTED_FOR,
        key_confirm: super::answer_wake::interrupt::CONFIRM_SETTLES,
        queue_settles: super::answer_wake::mid_turn::QUEUE_SETTLES,
        enter_deadline: super::answer_wake::mid_turn::ENTER_DEADLINE,
        late_key: super::answer_wake::mid_turn::LATE_KEY,
        typing_wait: super::answer_wake::interrupt::TYPING_WAIT,
        key_lockout: super::answer_wake::interrupt::LOCKOUT,
        paste_settles: super::answer_wake::PASTE_SETTLES,
        told_spacing: Duration::from_millis(super::answer_wake::TOLD_SPACING_MS as u64),
        held_retry: super::answer_wake::compose::held::RETRY,
        torn_cap: super::answer_wake::compose::held::TORN_FOR,
        count_notice_every: super::COUNT_NOTICE_EVERY,
        queue_watch: super::answer_wake::queued::WATCH_FOR,
    };
}

#[cfg(test)]
impl Timing {
    /// A wait a test spends in full because nothing will end it early: a
    /// window no confirmation comes in, a spacing, a retry. Short, since the
    /// test only needs it to end, never to hold something back.
    pub(crate) const SHORT: Duration = Duration::from_millis(300);
}

impl super::Watcher {
    /// The waits this watcher runs with.
    pub(crate) fn timing(&self) -> Timing {
        *self.timing.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run with `timing` from now on: a test's shorter waits.
    #[cfg(test)]
    pub(crate) fn set_timing(&self, timing: Timing) {
        *self.timing.lock().unwrap_or_else(|e| e.into_inner()) = timing;
    }

    /// Change some of this watcher's waits, keeping the rest.
    #[cfg(test)]
    pub(crate) fn with_timing(&self, change: impl FnOnce(&mut Timing)) {
        let mut timing = self.timing();
        change(&mut timing);
        self.set_timing(timing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The production waits, pinned: injecting them changed no value the
    /// daemon runs with, and a change to one is a change to this test.
    #[test]
    fn production_keeps_every_wait_it_had() {
        let s = Duration::from_secs;
        let ms = Duration::from_millis;
        assert_eq!(
            Timing::PRODUCTION,
            Timing {
                compose_confirm: s(5),
                turn_begun_wait: s(10 * 60),
                parked_poll: s(10),
                left_kept: s(4 * 60 * 60),
                idle_twice: ms(300),
                wait_slows_after: s(30),
                wait_poll: s(1),
                carry_agents_for: s(30),
                key_confirm: s(3),
                queue_settles: s(3),
                enter_deadline: s(2),
                late_key: s(5),
                typing_wait: s(10),
                key_lockout: ms(1_500),
                paste_settles: s(2),
                told_spacing: s(2),
                held_retry: ms(500),
                torn_cap: s(10),
                count_notice_every: s(5),
                queue_watch: s(4 * 60 * 60),
            }
        );
    }

    /// Why `t`'s waits don't fit together, if they don't: the ways the
    /// production values work against each other, which pinning each value
    /// (`production_keeps_every_wait_it_had`) can't say.
    fn misfit(t: &Timing) -> Option<&'static str> {
        let poll = crate::watch::answer_wake::PASTE_POLL;
        if t.wait_poll <= t.idle_twice {
            Some("a slow poll no longer than idle_twice: two reads between turns would not be two polls")
        } else if t.wait_poll < poll {
            Some("a slow poll faster than the fast one")
        } else if t.wait_slows_after >= t.turn_begun_wait {
            Some("a wait that parks before it slows")
        } else if t.parked_poll < t.wait_poll {
            Some("a parked poll faster than the slow one")
        } else if t.left_kept <= t.turn_begun_wait {
            Some("a message given up before it parks")
        } else if t.carry_agents_for > t.wait_slows_after {
            Some("the gate's agents trusted past the slowdown, after the fast reads that watch for them")
        } else if t.torn_cap < t.wait_poll * 3 {
            Some("fewer than three slow reads before a run of bad ones gives up")
        } else if t.torn_cap >= t.turn_begun_wait / 10 {
            Some("a run of bad reads allowed a tenth of the wait or more")
        } else if t.turn_begun_wait < t.wait_slows_after + t.idle_twice * 2 + t.wait_poll * 2 {
            Some("too little wait after the slowdown for two idle reads and an Enter")
        } else {
            None
        }
    }

    /// How the production waits fit together (ov-816 review, N6), and that
    /// `misfit` can fail: each rule is broken in turn on a copy.
    #[test]
    fn the_production_waits_fit_together() {
        assert_eq!(misfit(&Timing::PRODUCTION), None);
        type Break = (fn(&mut Timing), &'static str);
        let broken: [Break; 9] = [
            (|t| t.wait_poll = t.idle_twice, "no longer than idle_twice"),
            (|t| (t.wait_poll, t.idle_twice) = (Duration::from_millis(50), Duration::from_millis(10)), "faster than the fast one"),
            (|t| t.wait_slows_after = t.turn_begun_wait, "parks before it slows"),
            (|t| t.parked_poll = t.wait_poll / 2, "parked poll faster than the slow one"),
            (|t| t.left_kept = t.turn_begun_wait, "given up before it parks"),
            (|t| t.carry_agents_for = t.wait_slows_after + Duration::from_millis(1), "trusted past the slowdown"),
            (|t| t.torn_cap = t.wait_poll, "fewer than three slow reads"),
            (|t| t.torn_cap = t.turn_begun_wait, "a tenth of the wait"),
            (|t| (t.turn_begun_wait, t.torn_cap) = (t.wait_slows_after + Duration::from_millis(900), Duration::from_secs(3)), "too little wait after the slowdown"),
        ];
        for (change, said) in broken {
            let mut t = Timing::PRODUCTION;
            change(&mut t);
            let why = misfit(&t).unwrap_or_else(|| panic!("{said} didn't fail"));
            assert!(why.contains(said), "{said}: {why}");
        }
    }

    /// A watcher the daemon builds runs with the production waits.
    #[tokio::test]
    async fn a_new_watcher_runs_with_the_production_waits() {
        let (_dir, svc, _) = crate::test_support::fixture().await;
        assert_eq!(super::super::Watcher::new(svc).timing(), Timing::PRODUCTION);
    }
}
