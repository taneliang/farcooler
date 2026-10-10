//! R-42: a pane has true color whatever the runner inherited.

use super::*;

/// A pane's line gives its program `COLORTERM=truecolor` even when the process
/// that runs the line has none, and a recipe's own value still wins. This is
/// the check that goes red without R-42: tmux 3.7 happens to set the variable
/// itself, so a pane test alone can't tell (an older tmux doesn't).
#[test]
fn a_line_run_with_no_colorterm_still_has_it() {
    let workspace = PaneWorkspace { id: Uuid::now_v7(), charter: None, env: Vec::new() };
    let recipe = PaneWorkspace { env: vec![("COLORTERM".into(), "24bit".into())], ..workspace };
    let run = |line: &str| {
        let out = std::process::Command::new("env")
            .args(["-i", "PATH=/usr/bin:/bin", "sh", "-c", line])
            .output()
            .expect("run");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let plain = with_pane_env(Uuid::now_v7(), "shell", None, None, "printenv COLORTERM".into());
    let won = with_pane_env(Uuid::now_v7(), "claude", None, Some(&recipe), "printenv COLORTERM".into());
    assert_eq!(run(&plain), "truecolor");
    assert_eq!(run(&won), "24bit");
}

/// The same through a real scratch tmux pane started by a client with no
/// `COLORTERM`.
#[tokio::test]
async fn a_pane_has_true_color_when_the_runner_has_none() {
    // One server per pane: a new-session on a socket whose server is still
    // exiting from the last kill-server can land in that dying server, and
    // the pane's output is never seen (CI run 38032155398).
    let tmux = |sock: &str, args: &[&str]| {
        std::process::Command::new("tmux")
            .env_remove("COLORTERM")
            .args(["-L", sock])
            .args(args)
            .output()
            .expect("tmux")
    };
    let workspace = PaneWorkspace { id: Uuid::now_v7(), charter: None, env: Vec::new() };
    let recipe = PaneWorkspace { env: vec![("COLORTERM".into(), "24bit".into())], ..workspace.clone() };
    let plain = with_pane_env(Uuid::now_v7(), "shell", None, None, "printenv COLORTERM".into());
    let won = with_pane_env(Uuid::now_v7(), "claude", None, Some(&recipe), "printenv COLORTERM".into());
    let mut seen = Vec::new();
    for (i, line) in [&plain, &won].into_iter().enumerate() {
        let sock = format!("fc-truecolor-{}-{i}", std::process::id());
        let out = tmux(&sock, &["new-session", "-d", "-x", "80", "-y", "24", "-P", "-F", "#{pane_id}", &format!("{line}; sleep 60")]);
        let pane = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert!(!pane.is_empty(), "tmux started no pane: {}", String::from_utf8_lossy(&out.stderr));
        let mut text = String::new();
        // A loaded CI runner can take seconds to start a server and run the
        // pane's command, so wait up to 30 s (the pane lives for 60).
        for _ in 0..300 {
            text = String::from_utf8_lossy(&tmux(&sock, &["capture-pane", "-p", "-t", &pane]).stdout).trim().to_string();
            if !text.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        seen.push(text);
        tmux(&sock, &["kill-server"]);
    }
    assert_eq!(seen, ["truecolor", "24bit"]);
}
