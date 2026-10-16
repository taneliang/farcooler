use super::*;

fn hosts() -> Vec<String> {
    vec!["git.corp.example".to_string()]
}

fn target(url: &str) -> Option<(String, String, u32, Option<u64>)> {
    resolve(url, &hosts()).map(|t| (t.host.clone(), t.repository(), t.number, t.comment))
}

#[test]
fn a_link_is_read_to_a_comment_or_to_the_pull_request() {
    assert_eq!(
        target("https://github.com/acme/billing/pull/213#discussion_r987"),
        Some(("github.com".into(), "acme/billing".into(), 213, Some(987)))
    );
    // Left on the pull request: a conversation comment, a review, or no fragment.
    for link in [
        "https://github.com/acme/billing/pull/213",
        "https://github.com/acme/billing/pull/213/",
        "https://github.com/acme/billing/pull/213#issuecomment-55",
        "https://github.com/acme/billing/pull/213#pullrequestreview-9",
        "https://GitHub.com/acme/billing/pull/213?notification_referrer_id=x",
    ] {
        assert_eq!(target(link).map(|t| (t.1, t.2, t.3)), Some(("acme/billing".into(), 213, None)), "{link}");
    }
    // The board's own git host, as the card offers it.
    assert_eq!(
        target("https://git.corp.example/team/app.v2/pull/4#discussion_r1").map(|t| (t.0, t.1)),
        Some(("git.corp.example".into(), "team/app.v2".into()))
    );
}

#[test]
fn a_link_that_is_not_one_place_is_not_read() {
    for bad in [
        "https://github.com/acme/billing/issues/5",
        "https://github.com/acme/billing/pull/213/files",
        "https://github.com/acme/billing/pull/213/files#discussion_r1",
        "https://github.com/acme/billing/pull/abc",
        "https://github.com/acme/billing/pull/0",
        "https://github.com/acme/billing/pull/-1",
        "https://github.com/acme/billing/pull/213#r987",
        "https://github.com/acme/billing/pull/213#discussion_r",
        "https://github.com/acme/billing/pull/213#discussion_rx1",
        "https://github.com/acme/billing/pull/213#discussion_r1/../x",
        "https://github.com/acme/billing/pull/213#discussion_r99999999999999999999999",
        "https://github.com/acme/../pull/213",
        "https://github.com/./billing/pull/213",
        "https://github.com//billing/pull/213",
        "https://github.com/acme/bill%2Fing/pull/213",
        "https://github.com/acme/billing/pull",
        "https://github.com.evil.example/acme/billing/pull/213#discussion_r1",
        "https://github.com@evil.example/acme/billing/pull/213",
        "http://github.com/acme/billing/pull/213",
        "https://other.example/acme/billing/pull/213",
        "",
    ] {
        assert_eq!(target(bad), None, "{bad}");
    }
}

#[test]
fn the_api_path_and_the_comment_url_are_built_from_checked_parts_only() {
    let t = resolve("https://github.com/acme/billing/pull/213?x=1#discussion_r987", &[]).expect("a thread");
    assert_eq!(t.pull_url(), "https://github.com/acme/billing/pull/213");
    assert!(!t.pull_url().contains(['?', '#', ' ']));
}

#[test]
fn a_reply_is_trimmed_and_capped_in_characters() {
    assert_eq!(body_ok("  Thanks, done.\n").unwrap(), "Thanks, done.");
    assert!(body_ok("").is_err() && body_ok(" \n\t").is_err() && body_ok("a\0b").is_err());
    assert!(body_ok(&"x".repeat(REPLY_MAX)).is_ok());
    assert!(body_ok(&"x".repeat(REPLY_MAX + 1)).is_err());
    // Characters, not bytes: 2,000 of a two-byte one is 4,000 bytes.
    assert!(body_ok(&"\u{e9}".repeat(REPLY_MAX)).is_ok());
}
