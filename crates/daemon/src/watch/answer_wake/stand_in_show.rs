//! `StandIn::show`, which returns once tmux has drawn the screen.
//!
//! The stand-in logs `MODE x` when it writes the screen to its pty. tmux parses
//! those bytes on its own time, and on a loaded runner a capture right after
//! can still read the screen before (ov-695, ov-696). So the stand-in follows
//! each screen with a pane title, `stand-in N` (N counts the screens it has
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
        // The screens the stand-in has sent are its `MODE` lines, and the title
        // counts them the same way: this one is drawn once the title has
        // reached the count, whatever earlier screen was still on its way.
        let sent = self.log().lines().filter(|l| l.starts_with("MODE ")).count() as u64;
        for _ in 0..750 {
            if self.screens().await >= sent {
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

    /// How many screens tmux has drawn of the stand-in's, by its pane title;
    /// 0 before the first, or when tmux didn't answer.
    async fn screens(&self) -> u64 {
        self.title().await.strip_prefix("stand-in ").and_then(|n| n.trim().parse().ok()).unwrap_or(0)
    }

    async fn title(&self) -> String {
        let Some((svc, terminal)) = &self.pane else { return String::new() };
        let Some(pane) = svc.inventory_snapshot().claimants(*terminal).into_iter().next().map(|p| p.pane_id.clone()) else { return String::new() };
        let out = svc.runtime().tmux.run(&["display-message", "-p", "-t", &pane, "#{pane_title}"]).await;
        out.map(|o| o.stdout.trim().to_string()).unwrap_or_default()
    }
}
