use super::*;

const THIS_REPO: &str = "# Overnight\n\n## Workflow\n\nDirect.\n\n## Landing\n\n<!-- how work lands -->\n- Route: push\n- Route `services/site/**`: ask\n- Gate: `scripts/landing-gate.sh`\n- Gate: true\n\n## Anything else\n\n- Gate: not this one\n";

/// This repository's section: push, except the marketing site, which asks.
#[test]
fn this_repositorys_section_reads() {
    let rules = parse(THIS_REPO).unwrap().expect("a section");
    assert_eq!(rules.default_route, Route::Push);
    assert_eq!(rules.rules, vec![("services/site/**".to_string(), Route::Ask)]);
    assert_eq!(rules.gate, vec!["scripts/landing-gate.sh".to_string(), "true".to_string()], "the next heading ends it");
    assert_eq!((rules.base.as_str(), rules.remote.as_str(), rules.ci), ("main", "origin", true));
}

/// A ticket takes its most careful file's route, and says which rule.
#[test]
fn a_site_file_makes_the_ticket_ask() {
    let rules = parse(THIS_REPO).unwrap().unwrap();
    let files = |f: &[&str]| f.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(rules.route_of(&files(&["crates/daemon/src/lib.rs"])), (Route::Push, None));
    let mixed = files(&["crates/daemon/src/lib.rs", "services/site/public/index.html"]);
    assert_eq!(rules.route_of(&mixed), (Route::Ask, Some("services/site/**")));
    assert_eq!(rules.route_of(&files(&["services/sitemap.txt"])), (Route::Push, None), "a glob matches whole folders");
    assert_eq!(rules.route_of(&[]), (Route::Push, None));
}

#[test]
fn a_work_repository_opens_pull_requests() {
    let rules = parse("## Landing\n- Route: pull request\n- CI: none\n").unwrap().unwrap();
    assert_eq!(rules.default_route, Route::PullRequest);
    assert!(rules.gate.is_empty(), "nothing pushes, so no gate is needed");
    assert!(!rules.ci);
}

#[test]
fn no_section_says_nothing() {
    assert_eq!(parse("## Workflow\n\nPush to main.\n").unwrap(), None);
    assert_eq!(parse("### Landing\n- Route: ask\n").unwrap(), None, "only a second-level heading");
}

/// Anything that doesn't read refuses the charter, naming the line.
#[test]
fn a_section_that_does_not_read_names_its_line() {
    let refused = |text: &str| parse(text).expect_err(text);
    assert_eq!(refused("## Landing\n\nWe push to main.\n").line, 3);
    assert!(refused("## Landing\n- Route: yolo\n").why.contains("push, pull request or ask"));
    assert!(refused("## Landing\n- Route: push\n- Route: ask\n").why.contains("once"));
    assert!(refused("## Landing\n- Gate: true\n- Branch: main\n").why.contains("keys"));
    assert!(refused("## Landing\n- Route `services/**: ask\n").why.contains("backtick"));
    assert!(refused("## Landing\n- Route `/abs/**`: ask\n- Gate: none\n").why.contains("top of the repository"));
    assert!(refused("## Landing\n- Gate: true\n- Base: ../etc\n").why.contains("branch"));
    assert!(refused("## Landing\n- Gate: true\n- CI: travis\n").why.contains("github or none"));
    assert!(refused("## Landing\n- Route: push\n").why.contains("names its gate"), "a push needs a gate");
    assert!(refused("## Landing\n- Gate: none\n- Gate: make\n").why.contains("both"));
    assert!(parse("## Landing\n- Gate: none\n").is_ok(), "`Gate: none` is said on purpose");
}

#[test]
fn globs_match_whole_paths() {
    assert!(glob_matches("services/site/**", "services/site/public/a.html"));
    assert!(!glob_matches("services/site/**", "services/sites/a"));
    assert!(glob_matches("*.md", "README.md"));
    assert!(!glob_matches("*.md", "docs/a.md"), "`*` stays in one folder");
    assert!(glob_matches("**/*.md", "docs/a.md"));
    assert!(glob_matches("**/*.md", "a.md"), "`**/` matches no folder too");
    assert!(glob_matches("docs/?.md", "docs/a.md"));
    assert!(!glob_matches("docs/?.md", "docs/ab.md"));
}

/// An explicit file rule overrides the default; only actual files vote.
#[test]
fn a_push_rule_can_override_the_default_ask_route() {
    let rules = parse("## Landing\n- Route: ask\n- Route `docs/**`: push\n- Gate: none\n").unwrap().unwrap();
    assert_eq!(rules.route_of(&["docs/readme.md".into()]), (Route::Push, Some("docs/**")));
    assert_eq!(rules.route_of(&["docs/readme.md".into(), "src/main.rs".into()]), (Route::Ask, None));
}
