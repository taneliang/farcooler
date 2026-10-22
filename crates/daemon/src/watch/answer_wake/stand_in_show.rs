//! `StandIn::show`, which returns once tmux has drawn the screen.
//!
//! The stand-in logs `MODE x` when it writes the screen to its pty. tmux parses
//! those bytes on its own time, and on a loaded runner a capture right after
//! can still read the screen before (ov-695, ov-696). So the stand-in follows
//! each screen with a pane title, `stand-in PID N` (N counts the screens it has
//! sent); tmux sets the title when it parses that sequence, after the screen
//! before it, so a title past the last one seen means the screen is drawn.

use super::*;

impl StandIn {
    pub(super) fn new(control: PathBuf, log: PathBuf) -> Self {
        StandIn { control, log, pane: None }
    }

    pub(super) fn in_pane(mut self, svc: &Arc<Service>, terminal: Uuid) -> Self {
        self.pane = Some((svc.clone(), terminal));
        self
    }

    /// Show `mode`, and wait until the stand-in has taken it and tmux has
    /// drawn it. Panics after 15 seconds.
    pub(super) async fn show(&self, mode: &str) {
        let before = self.log().matches(&format!("MODE {mode}\n")).count();
        std::fs::write(&self.control, mode).unwrap();
        for _ in 0..750 {
            if self.log().matches(&format!("MODE {mode}\n")).count() > before {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        if self.log().matches(&format!("MODE {mode}\n")).count() <= before {
            panic!("the stand-in never showed {mode}: {}", self.log());
        }
        if self.pane.is_none() {
            return;
        }
        // The screens this process has sent are the `MODE` lines since its
        // `START $pid` line (a respawned stand-in logs to the same file), and
        // its title, `stand-in $pid N`, counts them the same way: this one is
        // drawn once the title has reached the count, whatever earlier screen
        // was still on its way.
        let log = self.log();
        let lines: Vec<&str> = log.lines().collect();
        let start = lines.iter().rposition(|l| l.starts_with("START ")).unwrap_or(0);
        let pid = lines.get(start).and_then(|l| l.strip_prefix("START ")).unwrap_or("").to_string();
        let sent = lines[start..].iter().filter(|l| l.starts_with("MODE ")).count() as u64;
        for _ in 0..750 {
            if self.screens(&pid).await >= sent {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("tmux never drew {mode} (title {:?}): {}", self.title().await, self.log());
    }

    /// `show` from a hook the daemon runs in the middle of its own await,
    /// which blocks the test's runtime: the wait runs on a thread with a
    /// runtime of its own.
    pub(super) fn show_in_hook(&self, mode: &str) {
        std::thread::scope(|s| {
            s.spawn(|| {
                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                runtime.block_on(self.show(mode));
            })
            .join()
            .unwrap();
        });
    }

    /// How many screens tmux has drawn of process `pid`'s, by its pane title;
    /// 0 before the first, or when the title is another process's.
    async fn screens(&self, pid: &str) -> u64 {
        let title = self.title().await;
        title.strip_prefix("stand-in ").and_then(|t| t.strip_prefix(pid)).and_then(|n| n.trim().parse().ok()).unwrap_or(0)
    }

    async fn title(&self) -> String {
        let Some((svc, terminal)) = &self.pane else { return String::new() };
        let Some(pane) = svc.inventory_snapshot().claimants(*terminal).into_iter().next().map(|p| p.pane_id.clone()) else { return String::new() };
        let out = svc.runtime().tmux.run(&["display-message", "-p", "-t", &pane, "#{pane_title}"]).await;
        out.map(|o| o.stdout.trim().to_string()).unwrap_or_default()
    }
}

/// A stand-in respawned into the same pane logs to the same file and counts
/// its screens afresh (ov-698): `show` waits for the new process's, not for
/// as many as the log holds in all.
#[tokio::test]
async fn show_waits_on_the_respawned_stand_in_not_the_log_of_both() {
    let b = board().await;
    let agent = b.agent("Agent 2", "claude").await;
    let si = b.stand_in(&agent, "claude", "claude").await;
    si.show("working").await;
    si.show("idle").await;
    b.run_unfollowed(&agent, &super::si_command(&b, &agent)).await;
    b.screen_with(agent.id, "stand-in").await;
    si.show("working").await;
    si.show("idle").await;
}
