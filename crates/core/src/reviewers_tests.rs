//! Whom to ask (ov-580): CODEOWNERS matching, the three sources in order, and
//! the author never, however many places name them.

use super::*;

fn paths(p: &[&str]) -> Vec<String> {
    p.iter().map(|s| s.to_string()).collect()
}

fn names(s: &[Suggestion]) -> Vec<&str> {
    s.iter().map(|s| s.reviewer.as_str()).collect()
}

#[test]
fn patterns_match_the_way_codeowners_reads_them() {
    for (pattern, path, yes) in [
        ("*", "a/b.rs", true),
        ("*.rs", "crates/x/lib.rs", true),
        ("*.rs", "crates/x/lib.rsx", false),
        ("/docs/", "docs/a.md", true),
        ("/docs/", "x/docs/a.md", false),
        ("docs/", "x/docs/a.md", true),
        ("docs/", "docs", false),
        ("docs", "docs", true),
        ("docs", "docs/a.md", true),
        ("/crates/store", "crates/store/src/a.rs", true),
        ("crates/store", "x/crates/store/a.rs", false),
        ("crates/*/src", "crates/store/src/a.rs", true),
        ("crates/*/src", "crates/store/a/src/a.rs", false),
        ("apps/**/Sources", "apps/macos/x/Sources/a.swift", true),
        ("apps/**/Sources", "apps/Sources/a.swift", true),
        ("docs/*", "docs/a.md", true),
        ("docs/*", "docs/build/a.md", false),
        ("/docs/*", "docs/build/a.md", false),
        ("docs/**", "docs/build/a.md", true),
        ("docs/*/", "docs/build/a.md", true),
        ("a?.md", "ab.md", true),
        ("a?.md", "abc.md", false),
        ("Cargo.lock", "crates/x/Cargo.lock", true),
        ("/Cargo.lock", "crates/x/Cargo.lock", false),
    ] {
        assert_eq!(matches(pattern, path), yes, "{pattern} against {path}");
    }
}

#[test]
fn the_last_matching_rule_wins_and_a_rule_with_no_owner_clears_them() {
    let rules = parse("# who\n* @alice\ncrates/ @bob @acme/core # inline\ncrates/store/generated/\n/docs bob@example.com\n");
    assert_eq!(owners_of(&rules, "README.md"), ["alice"]);
    assert_eq!(owners_of(&rules, "crates/x/a.rs"), ["bob", "acme/core"]);
    assert_eq!(owners_of(&rules, "crates/store/generated/a.rs"), &[] as &[String], "cleared");
    assert_eq!(owners_of(&rules, "docs/a.md"), &[] as &[String], "an email is nobody a login can be asked of");
}

#[test]
fn codeowners_come_first_ranked_by_the_files_each_owns() {
    let owners = "crates/store/ @bob\ncrates/cli/ @alice\n*.md @carol\n";
    let touched = paths(&["crates/cli/a.rs", "crates/cli/b.rs", "crates/store/s.rs", "README.md"]);
    let got = suggest(&Evidence { authors: &["zed".to_string()], paths: &touched, codeowners: Some(owners), ..Default::default() });
    assert_eq!(names(&got), ["alice", "bob", "carol"]);
    assert!(got.iter().all(|s| s.source == Source::Codeowners));
}

#[test]
fn history_fills_in_after_codeowners_and_the_map_after_history() {
    let owners = "crates/cli/ @alice\n";
    let touched = paths(&["crates/cli/a.rs", "docs/x.md"]);
    let history = [("dave".to_string(), 9), ("erin".to_string(), 12)];
    let got = suggest(&Evidence { authors: &["zed".to_string()], paths: &touched, codeowners: Some(owners), history: &history, map: Some("docs/ @frank\n") });
    assert_eq!(names(&got), ["alice", "erin", "dave"], "three is the most, and history outranks the map");
    assert_eq!(got.iter().map(|s| s.source).collect::<Vec<_>>(), [Source::Codeowners, Source::History, Source::History]);
    // With nothing from the first two, the map speaks.
    let only_map = suggest(&Evidence { authors: &["zed".to_string()], paths: &touched, map: Some("docs/ @frank\n"), ..Default::default() });
    assert_eq!((names(&only_map), only_map[0].source), (vec!["frank"], Source::Map));
}

#[test]
fn the_owner_who_is_the_author_is_never_suggested_and_the_next_source_speaks() {
    let touched = paths(&["crates/cli/a.rs"]);
    // Every source names the author, in other cases than the author's own.
    let history = [("E-Liang".to_string(), 30), ("dave".to_string(), 2)];
    let got = suggest(&Evidence {
        authors: &["e-liang".to_string(), "Hal".to_string()],
        paths: &touched,
        codeowners: Some("* @E-LIANG\n"),
        history: &history,
        map: Some("* @e-Liang @gina\n"),
    });
    assert_eq!(names(&got), ["dave", "gina"]);
    // And anyone who wrote the branch's commits is out too.
    let hal = suggest(&Evidence { authors: &["e-liang".to_string(), "Hal".to_string()], paths: &touched, codeowners: Some("* @hal @ida\n"), ..Default::default() });
    assert_eq!(names(&hal), ["ida"]);
    // Nobody else anywhere: no suggestion, which is not the author.
    let alone = suggest(&Evidence { authors: &["e-liang".to_string(), "Hal".to_string()], paths: &touched, codeowners: Some("* @e-liang\n"), ..Default::default() });
    assert!(alone.is_empty());
    assert_eq!(line(&alone), None);
}

#[test]
fn nobody_is_named_twice_across_sources() {
    let touched = paths(&["a.rs"]);
    let history = [("Alice".to_string(), 3)];
    let got = suggest(&Evidence { authors: &["z".to_string()], paths: &touched, codeowners: Some("* @alice\n"), history: &history, map: Some("* @ALICE\n") });
    assert_eq!(names(&got), ["alice"]);
}

#[test]
fn only_a_noreply_commit_email_is_a_login() {
    assert_eq!(login_from_email("12345+alice@users.noreply.github.com"), Some("alice".into()));
    assert_eq!(login_from_email("alice@users.noreply.github.com"), Some("alice".into()));
    for no in [
        "alice@example.com",
        "49699333+dependabot[bot]@users.noreply.github.com",
        "noreply@github.com",
        "-bad@users.noreply.github.com",
        "a b@users.noreply.github.com",
        "",
    ] {
        assert_eq!(login_from_email(no), None, "{no}");
    }
    let counted = history_of(["1+Alice@users.noreply.github.com", "alice@users.noreply.github.com", "x@example.com", "2+bob@users.noreply.github.com"]);
    assert_eq!(counted, [("Alice".to_string(), 2), ("bob".to_string(), 1)]);
}

#[test]
fn the_line_names_each_in_a_code_span_with_where_it_came_from() {
    let s = [Suggestion { reviewer: "alice".into(), source: Source::History }, Suggestion { reviewer: "acme/core".into(), source: Source::Map }];
    assert_eq!(line(&s).as_deref(), Some("Suggested reviewers: `@alice` (recent commits), `@acme/core` (the reviewer map)"));
    assert_eq!(plain(&s), "alice (recent commits), acme/core (the reviewer map)");
}
