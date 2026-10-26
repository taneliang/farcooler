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
    /// How long a compose waits out a turn claude began between its paste
    /// and its Enter (`compose::TURN_BEGUN_WAIT`).
    pub turn_begun_wait: Duration,
    /// How far apart a waiting message's two reads between turns are before
    /// its plain Enter (`compose::pending::IDLE_TWICE`).
    pub idle_twice: Duration,
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
    /// (`compose::held::TORN_FOR`).
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
        idle_twice: super::answer_wake::compose::IDLE_TWICE,
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
                turn_begun_wait: s(30),
                idle_twice: ms(300),
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

    /// A watcher the daemon builds runs with the production waits.
    #[tokio::test]
    async fn a_new_watcher_runs_with_the_production_waits() {
        let (_dir, svc, _) = crate::test_support::fixture().await;
        assert_eq!(super::super::Watcher::new(svc).timing(), Timing::PRODUCTION);
    }
}
