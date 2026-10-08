use super::*;

#[test]
fn http_and_https_pages_are_opened_as_given() {
    for url in [
        "https://github.com",
        "https://github.com/",
        "HTTPS://www.notion.so/acme/Roadmap-1a2b3c?pvs=4#heading",
        "http://localhost:3000/",
        "https://linear.app/acme/issue/ENG-12/a-title",
        "https://user@example.com:8443/path",
        "https://[::1]:8080/",
        "https://例え.jp/パス",
        // Plain http where the Mac loads it (App Transport Security exempts
        // an IP address, a name with no dot and `.local`).
        "http://192.168.1.5:8080/",
        "http://[::1]:3000/",
        "http://devbox/",
        "http://my-mac.local/app",
    ] {
        assert_eq!(checked_url(url).expect(url), url, "returned as given");
    }
}

#[test]
fn everything_else_is_refused() {
    let long = format!("https://example.com/{}", "a".repeat(LONGEST_URL));
    for url in [
        "",
        "github.com",
        "//github.com",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "JavaScript://github.com/%0Aalert(1)",
        "data:text/html,<p>hi</p>",
        "about:blank",
        "linear://issue/ENG-12",
        "ftp://example.com/",
        "https:github.com",
        "https://",
        "https:///path",
        "https://user@/",
        "https://:443/",
        "https://exa mple.com/",
        "https://example.com/\n",
        "https://example.com/\u{7}",
        "https://example.com/\u{a0}",
        long.as_str(),
    ] {
        let refused = checked_url(url).expect_err(url);
        assert!(refused.to_string().contains("http or https"), "{url}: {refused}");
    }
}

/// L5: the Mac refuses plain http to a public name, with the system's own
/// words. An agent hears it from the runner instead, and what to do.
#[test]
fn plain_http_to_a_public_name_is_refused_with_what_to_use() {
    for url in ["http://example.com/", "http://github.com", "HTTP://www.example.com/x", "http://example.com./", "http://user@example.com/"] {
        let refused = checked_url(url).expect_err(url);
        assert!(refused.to_string().contains("use the https address"), "{url}: {refused}");
    }
    assert!(checked_url("https://example.com/").is_ok(), "https is the way");
}

/// L1: the `web` preset is `layout.split`'s, with a page. Asked for any other
/// way it would make a pane with no page that nothing can fill in.
#[test]
fn the_web_preset_is_refused_without_a_page() {
    for preset in ["web", "web:anything"] {
        assert!(refuse_without_page(preset).is_err(), "{preset}");
    }
    for preset in ["shell", "claude", "claude:opus", "changes", "website"] {
        assert!(refuse_without_page(preset).is_ok(), "{preset}");
    }
}

/// L9: an agent that loops on `open-url` stops at the cap, and a closed (or
/// stopped) page gives its place back.
#[tokio::test]
async fn a_worktree_holds_only_so_many_pages() {
    let dir = tempfile::tempdir().unwrap();
    let service = Service::open_in(dir.path().to_path_buf()).await.expect("service");
    let store = &service.store;
    let host = Uuid::now_v7();
    let root = store.create_repository_root(host, "/wt", 1_000).unwrap();
    let repo = store.create_repository(host, root.id, "name", "/gitdir", "origin").unwrap();
    let wt = store.create_worktree(repo.id, "feature/x", "/wt", false).unwrap();

    let mut made = Vec::new();
    for n in 0..MOST_PER_WORKTREE {
        assert!(service.room_for_another_page(wt.id).is_ok(), "page {n} has room");
        let t = store.create_terminal(wt.id, "Web", "web", TerminalIntent::Running, 120, 40).unwrap();
        made.push(store.set_pane_mode(t.id, t.resource_version, models::PaneMode::Web, None, false).unwrap());
    }
    // A shell doesn't count against the pages.
    store.create_terminal(wt.id, "sh", "shell", TerminalIntent::Running, 120, 40).unwrap();
    assert!(service.room_for_another_page(wt.id).is_err(), "the cap is {MOST_PER_WORKTREE}");

    let first = &made[0];
    let update = models::TerminalUpdate {
        title: first.title.clone(),
        command_preset: first.command_preset.clone(),
        intent: TerminalIntent::Stopped,
        runtime_confirmed: first.runtime_confirmed,
        exit_code: first.exit_code,
        exit_signal: first.exit_signal,
        lease_generation: first.lease_generation,
        epoch: first.epoch,
        columns: first.columns,
        rows: first.rows,
    };
    store.update_terminal(first.id, first.resource_version, update).unwrap();
    assert!(service.room_for_another_page(wt.id).is_ok(), "a stopped page gives its place back");
}
