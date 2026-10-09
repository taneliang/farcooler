//! `Terminal.running_agent` as the watcher decides it (ov-443). Its own file to
//! keep `watch.rs` inside its size budget.

use super::*;

/// A shell whose screen shows claude's banner and footer, as one does after
/// claude quits or while a transcript is `cat`ed, runs no agent: the watcher says
/// `""` (looked, found none), never `claude`. Goes red on the screen fallback.
#[tokio::test]
async fn a_shell_showing_claudes_banner_is_not_running_claude() {
    if farcooler_core::programs::find("tmux").is_none() {
        assert!(std::env::var_os("CI").is_none(), "CI installs tmux");
        eprintln!("SKIP: tmux is not installed here");
        return;
    }
    let (_dir, svc, repo) = crate::test_support::fixture().await;
    let watcher = Watcher::new(svc.clone());
    let main = svc.store.list_worktrees_for_repository(repo).unwrap().into_iter().find(|w| w.is_main_checkout).unwrap();
    let pane = svc.create_terminal(main.id, "banner", "shell").await.expect("a shell pane");
    let id = pane.id;
    svc.send_input(id, "clear; printf 'Claude Code v2.1.220\\n? for shortcuts\\nesc to interrupt\\n'; sleep 0.1\r")
        .await
        .expect("typed");
    let registry = svc.registry();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let (screen, _, _) = svc.screen(id).await.expect("a screen");
        // The banner is drawn, the shell is back at its prompt, and the screen
        // alone would name claude: the precondition that makes this test mean it.
        if screen.contains("esc to interrupt") && !screen.contains("sleep 0.1\n") && registry.identify("zsh", &screen).is_some() {
            watcher.sample().await;
            if watcher.running_agent(id).await.is_some() {
                break;
            }
        }
        assert!(std::time::Instant::now() < deadline, "the banner never drew:\n{screen}");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    assert_eq!(watcher.running_agent(id).await.as_deref(), Some(""), "a shell is not a claude");
}
