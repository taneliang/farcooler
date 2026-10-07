//! The sweep for panes an open started and never finished (ov-176).
//!
//! An open makes its pane first and tags it after. When a tag fails, the open
//! closes what it made (`abandon` in the tmux crate); when the open was cut
//! off before tmux even answered with the window's id, nothing knows what to
//! close, and the pane runs on with no terminal id, where the orphan reaper in
//! `sample`, which reads tagged panes only, can never see it. This takes those.
//!
//! Only panes the open marked as its own (`farcooler_tmux::windows::
//! OPENING_MARK`) and never tagged, and only after `UNFINISHED_GRACE`. A pane
//! a person or an agent added to our session through `TMUX` is untagged
//! forever and never marked, and is never touched.

use std::collections::HashMap;
use std::time::Duration;

use super::Watcher;

/// How long a pane an open started may go without its terminal id before the
/// sweep takes it.
///
/// An open makes the pane first and tags it after, so every pane it makes is
/// unfinished for a moment, and on a loaded machine the moment is longer: each
/// of an open's eight tmux commands (`has-session`, `new-*`, two session tags,
/// three window tags and the pane's) may take `TMUX_LIFECYCLE_TIMEOUT`, ten
/// seconds. Two minutes is past all of them together, so the sweep can only
/// take a pane no open is still working on.
///
/// Only panes carrying `farcooler_tmux::windows::OPENING_MARK`. A pane
/// somebody added to our session by hand is untagged forever and never
/// marked, and the sweep never sees it.
const UNFINISHED_GRACE: Duration = Duration::from_secs(120);

/// Which unfinished opens have been unfinished for `grace`, given the ones
/// `present` now. Updates `seen`, which remembers when each was first seen.
///
/// A pane that is no longer present, or has been tagged since, is forgotten,
/// so one that comes back is timed again from scratch. Keyed by pane id AND
/// pid: a restarted server numbers its panes from `%0` again, and a new `%0`
/// must not inherit the old one's age.
fn unfinished_to_reap(
    seen: &mut HashMap<farcooler_tmux::UnfinishedOpen, std::time::Instant>,
    present: &[farcooler_tmux::UnfinishedOpen],
    now: std::time::Instant,
    grace: Duration,
) -> Vec<farcooler_tmux::UnfinishedOpen> {
    seen.retain(|pane, _| present.contains(pane));
    present
        .iter()
        .filter(|pane| now.duration_since(*seen.entry((*pane).clone()).or_insert(now)) >= grace)
        .cloned()
        .collect()
}

impl Watcher {
    /// Kill the panes whose open never finished, once they are past the grace.
    pub(super) async fn sweep_unfinished_opens(&self) {
        // Only panes an open marked as its own, never one a person added, and
        // only once they have been unfinished longer than any open could take.
        // From the read this tick's refresh made: no `list-panes` of its own.
        if let Some(unfinished) = self.service.inventory.unfinished_opens() {
            let stale = {
                let mut seen = self.unfinished_seen.lock().unwrap_or_else(|e| e.into_inner());
                unfinished_to_reap(&mut seen, &unfinished, std::time::Instant::now(), UNFINISHED_GRACE)
            };
            for pane in stale {
                match self.service.tmux.kill_pane(&pane.pane_id).await {
                    Ok(true) => tracing::warn!(pane = %pane.pane_id, "reaped a pane whose open never finished"),
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(pane = %pane.pane_id, ?error, "an unfinished open's pane could not be reaped")
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    /// The sweep, end to end on a private server.
    ///
    /// - An open that never finished is reaped once the grace has passed.
    /// - An open still in progress is not.
    /// - A window or a split a person or an agent added to our session through
    ///   `TMUX`, untagged as it is, is never touched, however old.
    #[tokio::test]
    async fn the_sweep_takes_only_an_open_that_never_finished() {
        let (_dir, svc, _repo) = crate::test_support::fixture().await;
        if farcooler_core::programs::find("tmux").is_none() {
            assert!(std::env::var_os("CI").is_none(), "CI installs tmux");
            return;
        }
        let pane = |out: farcooler_tmux::server::Output| out.stdout.trim().to_string();
        let run = |args: Vec<String>| {
            let svc = svc.clone();
            async move {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                svc.tmux.run(&args).await.expect("tmux")
            }
        };
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // A person's session start, then their window and their split.
        let hand = pane(
            run(args(&["new-session", "-d", "-s", farcooler_tmux::SESSION_NAME, "-P", "-F", "#{pane_id}", "sleep 60"]))
                .await,
        );
        let hand_window = pane(run(args(&["new-window", "-d", "-t", "farcooler:", "-P", "-F", "#{pane_id}", "sleep 60"])).await);
        let hand_split = pane(run(args(&["split-window", "-d", "-t", &hand, "-P", "-F", "#{pane_id}", "sleep 60"])).await);
        // An open that was cut off before its tags: marked, never tagged.
        let marked = farcooler_tmux::windows::marked("sleep 60", Uuid::now_v7());
        let abandoned = pane(run(args(&["new-window", "-d", "-t", "farcooler:", "-P", "-F", "#{pane_id}", &marked])).await);

        let watcher = Watcher::new(svc.clone());
        watcher.sample().await;
        // As though all of it had been seen a grace ago.
        for first_seen in watcher.unfinished_seen.lock().unwrap().values_mut() {
            *first_seen -= UNFINISHED_GRACE;
        }
        // And an open that is under way right now.
        let marked = farcooler_tmux::windows::marked("sleep 60", Uuid::now_v7());
        let opening = pane(run(args(&["new-window", "-d", "-t", "farcooler:", "-P", "-F", "#{pane_id}", &marked])).await);
        watcher.sample().await;

        let alive: Vec<String> =
            run(args(&["list-panes", "-a", "-F", "#{pane_id}"])).await.stdout.lines().map(str::to_string).collect();
        assert!(!alive.contains(&abandoned), "the unfinished open outlived the grace");
        for (what, pane) in
            [("hand session", &hand), ("hand window", &hand_window), ("hand split", &hand_split), ("open under way", &opening)]
        {
            assert!(alive.contains(pane), "the sweep took the {what} {pane}: {alive:?}");
        }
    }

    fn untagged(pane: &str, pid: u32) -> farcooler_tmux::UnfinishedOpen {
        farcooler_tmux::UnfinishedOpen { pane_id: pane.into(), pid }
    }

    #[test]
    fn an_untagged_pane_is_reaped_only_once_the_grace_has_passed() {
        // An open tags its pane after making it, so a fresh untagged pane is
        // an open in progress, not residue.
        let grace = Duration::from_secs(120);
        let start = std::time::Instant::now();
        let mut seen = HashMap::new();
        let pane = untagged("%4", 100);
        assert!(unfinished_to_reap(&mut seen, std::slice::from_ref(&pane), start, grace).is_empty());
        let nearly = start + grace - Duration::from_secs(1);
        assert!(unfinished_to_reap(&mut seen, std::slice::from_ref(&pane), nearly, grace).is_empty());
        assert_eq!(unfinished_to_reap(&mut seen, std::slice::from_ref(&pane), start + grace, grace), vec![pane]);
    }

    #[test]
    fn a_pane_tagged_in_time_is_forgotten_and_timed_afresh_if_it_returns() {
        let grace = Duration::from_secs(120);
        let start = std::time::Instant::now();
        let mut seen = HashMap::new();
        let pane = untagged("%4", 100);
        unfinished_to_reap(&mut seen, std::slice::from_ref(&pane), start, grace);
        // Tagged by its open: no longer in the list.
        unfinished_to_reap(&mut seen, &[], start + Duration::from_secs(5), grace);
        let back = start + grace;
        assert!(unfinished_to_reap(&mut seen, std::slice::from_ref(&pane), back, grace).is_empty());
    }

    #[test]
    fn a_restarted_servers_pane_does_not_inherit_an_old_ones_age() {
        // A new server numbers from %0 again; the pid tells them apart.
        let grace = Duration::from_secs(120);
        let start = std::time::Instant::now();
        let mut seen = HashMap::new();
        unfinished_to_reap(&mut seen, &[untagged("%0", 100)], start, grace);
        let reaped = unfinished_to_reap(&mut seen, &[untagged("%0", 200)], start + grace, grace);
        assert!(reaped.is_empty(), "{reaped:?}");
    }
}
