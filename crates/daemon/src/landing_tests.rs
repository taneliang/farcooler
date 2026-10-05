//! Each way of landing, decided from what `gh` printed for it
//! (`test/fixtures/landing/`: the unprotected answers recorded from this
//! repository, the rest in the shapes GitHub documents for rulesets).

use super::*;

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/landing").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The facts of a repository whose base branch answered with these files.
fn facts(rules: &str, branch: &str, repo: &str) -> Facts {
    Facts {
        base: "main".into(),
        rules: parse_rules(&fixture(rules)),
        protected: parse_protected(&fixture(branch)),
        protection_checks: parse_protection_checks(&fixture(branch)),
        repo: parse_repo_view(&fixture(repo)),
        codeowners: Some(false),
        merge_group_workflow: Some(false),
    }
}

fn unprotected() -> Facts {
    facts("rules-unprotected.json", "branch-unprotected.json", "repo-view-admin.json")
}

#[test]
fn an_unprotected_base_suggests_direct_and_says_why() {
    let d = decide(&unprotected());
    assert_eq!(d.suggested, Some(LandingMode::Direct));
    assert!(!d.direct_impossible);
    assert_eq!(d.reasons, ["Nothing on main asks for pull requests."]);
    assert!(d.warnings.is_empty());
}

#[test]
fn a_pull_request_rule_refuses_direct_and_names_the_reviews() {
    let f = facts("rules-pull-request.json", "branch-protected.json", "repo-view-admin.json");
    let d = decide(&f);
    assert_eq!(d.suggested, Some(LandingMode::PullRequests));
    assert!(d.direct_impossible, "a base that requires pull requests refuses direct");
    assert_eq!(d.reasons[0], "main requires pull requests, with 2 approving reviews.");
    assert!(d.reasons[1].starts_with("main needs 2 required checks"), "{:?}", d.reasons);
    let rules = f.rules.unwrap();
    assert_eq!(rules.required_checks, ["rust", "swift"]);
    assert_eq!(rules.allowed_methods, Some(vec!["squash".to_string(), "rebase".to_string()]));
}

#[test]
fn a_merge_queue_alone_refuses_direct() {
    let d = decide(&facts("rules-merge-queue.json", "branch-protected.json", "repo-view-admin.json"));
    assert_eq!(d.suggested, Some(LandingMode::PullRequests));
    assert!(d.direct_impossible);
    assert_eq!(d.reasons, ["main uses a merge queue, which only pull requests can enter."]);
}

#[test]
fn a_merge_queue_no_workflow_answers_warns_it_will_wait_forever() {
    let mut f = facts("rules-merge-queue.json", "branch-protected.json", "repo-view-admin.json");
    f.merge_group_workflow = Some(false);
    assert_eq!(decide(&f).warnings, ["The merge queue will wait forever: no workflow runs on `merge_group`."]);
    f.merge_group_workflow = Some(true);
    assert!(decide(&f).warnings.is_empty(), "a workflow that runs on merge_group answers the queue");
    f.merge_group_workflow = None;
    assert_eq!(decide(&f).warnings.len(), 1, "a workflow list that couldn't be read is said, not assumed fine");
    assert!(decide(&f).warnings[0].starts_with("Far Cooler couldn't check"));
}

#[test]
fn no_merge_queue_means_no_merge_group_warning() {
    let mut f = unprotected();
    f.merge_group_workflow = Some(false);
    assert!(decide(&f).warnings.is_empty(), "a workflow without merge_group is only a problem under a queue");
}

#[test]
fn a_protected_base_whose_rules_are_admin_only_suggests_pull_requests_but_does_not_forbid_direct() {
    // Classic protection: `protected` is public, its details are not.
    let d = decide(&facts("rules-unprotected.json", "branch-protected.json", "repo-view-admin.json"));
    assert_eq!(d.suggested, Some(LandingMode::PullRequests));
    assert!(!d.direct_impossible, "unknown is unknown: it isn't a refusal");
    assert!(d.reasons[0].starts_with("main is protected and its classic protection isn't something Far Cooler reads"), "{:?}", d.reasons);
}

#[test]
fn the_admin_only_sentence_is_for_those_who_are_not_admins() {
    let mut f = facts("rules-unprotected.json", "branch-protected.json", "repo-view-admin.json");
    f.repo.as_mut().unwrap().viewer_permission = Some("WRITE".into());
    let d = decide(&f);
    assert_eq!(d.suggested, Some(LandingMode::PullRequests));
    assert!(d.reasons[0].contains("only shows the details of its protection to an admin"), "{:?}", d.reasons);
}

#[test]
fn required_checks_alone_suggest_pull_requests_without_forbidding_direct() {
    let d = decide(&facts("rules-required-checks.json", "branch-unprotected.json", "repo-view-admin.json"));
    assert_eq!(d.suggested, Some(LandingMode::PullRequests));
    assert!(!d.direct_impossible);
    assert!(d.reasons[0].starts_with("main needs 1 required check to pass"), "{:?}", d.reasons);
}

#[test]
fn a_login_that_can_only_read_cannot_push_even_to_an_unprotected_base() {
    let d = decide(&facts("rules-unprotected.json", "branch-unprotected.json", "repo-view-reader.json"));
    assert_eq!(d.suggested, Some(LandingMode::PullRequests));
    assert!(d.direct_impossible);
    assert_eq!(d.reasons, ["This GitHub login can't push to the repository: it has read access."]);
}

#[test]
fn something_unread_is_not_something_absent() {
    // Nothing read at all.
    let none = Facts { base: "main".into(), ..Default::default() };
    let d = decide(&none);
    assert_eq!((d.suggested, d.direct_impossible), (None, false));
    assert_eq!(d.reasons, ["Far Cooler couldn't read its rules or whether it is protected or what this login may do on main, so it suggests nothing."]);

    // The rules read and empty, the branch's protection not: not "unprotected".
    let mut f = unprotected();
    f.protected = None;
    assert_eq!(decide(&f).suggested, None);
    assert_eq!(decide(&f).reasons, ["Far Cooler couldn't read whether it is protected on main, so it suggests nothing."]);

    // What this login may do, unread: not "may push".
    let f = facts("rules-unprotected.json", "branch-unprotected.json", "repo-view-no-permission.json");
    let d = decide(&f);
    assert_eq!((d.suggested, d.direct_impossible), (None, false));
    assert_eq!(d.reasons, ["Far Cooler couldn't read what this login may do on main, so it suggests nothing."]);

    // The reverse.
    let mut f = unprotected();
    f.rules = None;
    assert_eq!(decide(&f).suggested, None);
}

#[test]
fn the_merge_method_is_squash_then_rebase_then_merge_among_the_allowed() {
    let mut f = unprotected();
    assert_eq!(decide(&f).merge_method, Some("squash"), "recorded: only squash is allowed here");
    f.repo = parse_repo_view(&fixture("repo-view-reader.json"));
    assert_eq!(decide(&f).merge_method, Some("squash"), "all three allowed: squash first");
    f.repo.as_mut().unwrap().squash = Some(false);
    assert_eq!(decide(&f).merge_method, Some("rebase"));
    f.repo.as_mut().unwrap().rebase = Some(false);
    assert_eq!(decide(&f).merge_method, Some("merge"));
    f.repo.as_mut().unwrap().merge_commit = Some(false);
    assert_eq!(decide(&f).merge_method, None);
    f.repo = None;
    assert_eq!(decide(&f).merge_method, None, "unread is not allowed");
}

#[test]
fn a_rule_that_limits_the_methods_limits_the_choice() {
    // The repository allows all three; the ruleset allows squash and rebase only.
    let mut f = facts("rules-pull-request.json", "branch-protected.json", "repo-view-reader.json");
    f.repo.as_mut().unwrap().squash = Some(false);
    assert_eq!(decide(&f).merge_method, Some("rebase"), "squash is off, and merge isn't the ruleset's to offer");
    f.repo.as_mut().unwrap().rebase = Some(false);
    assert_eq!(decide(&f).merge_method, None, "a merge commit is allowed by the repository and refused by the rules");
}

#[test]
fn a_workflow_runs_on_merge_group_only_in_its_code() {
    let with_list = "name: CI\non:\n  push:\n    branches: [main]\n  merge_group:\n    types: [checks_requested]\njobs: {}\n";
    let inline = "on: [push, merge_group]\n";
    let scalar = "on: merge_group\n";
    let without = "name: CI\non:\n  push:\n  pull_request:\njobs:\n  build:\n    runs-on: ubuntu-latest\n";
    let comment = "on:\n  push:\n  # merge_group: later, when the queue is on\n  pull_request:\n";
    let lookalike = "on:\n  push:\n    branches: [merge_group_fix]\n";
    assert!(runs_on_merge_group(with_list));
    assert!(runs_on_merge_group(inline));
    assert!(runs_on_merge_group(scalar));
    assert!(!runs_on_merge_group(without));
    assert!(!runs_on_merge_group(comment), "a comment isn't a trigger");
    assert!(!runs_on_merge_group(lookalike), "a branch with the word in its name isn't a trigger");
}

#[test]
fn an_answer_that_is_not_json_reads_as_nothing() {
    assert_eq!(parse_rules(b"gh: HTTP 404"), None);
    assert_eq!(parse_rules(b"{\"message\":\"Not Found\"}"), None, "an error object isn't an empty rule list");
    assert_eq!(parse_protected(b"[]"), None);
    assert_eq!(parse_protected(b"{\"name\":\"main\"}"), None);
    assert_eq!(parse_repo_view(b""), None);
}

#[test]
fn the_recorded_repo_view_reads_whole() {
    let v = parse_repo_view(&fixture("repo-view-admin.json")).unwrap();
    assert_eq!(v.default_branch.as_deref(), Some("main"));
    assert_eq!((v.squash, v.rebase, v.merge_commit), (Some(true), Some(false), Some(false)));
    assert_eq!(v.viewer_permission.as_deref(), Some("ADMIN"));
}

#[test]
fn a_triage_login_cannot_push_either() {
    let d = decide(&facts("rules-unprotected.json", "branch-unprotected.json", "repo-view-triage.json"));
    assert!(d.direct_impossible);
    assert_eq!(d.reasons, ["This GitHub login can't push to the repository: it has triage access."]);
}

#[test]
fn a_rule_restricting_updates_refuses_direct() {
    let d = decide(&facts("rules-update.json", "branch-unprotected.json", "repo-view-admin.json"));
    assert_eq!(d.suggested, Some(LandingMode::PullRequests));
    assert!(d.direct_impossible);
    assert_eq!(d.reasons, ["main restricts who can update it, so a direct push is refused."]);
}

#[test]
fn deployment_workflow_and_code_scanning_rules_lean_to_pull_requests() {
    for (file, kind) in [
        ("rules-required-deployments.json", "required_deployments"),
        ("rules-workflows.json", "workflows"),
        ("rules-code-scanning.json", "code_scanning"),
    ] {
        let d = decide(&facts(file, "branch-unprotected.json", "repo-view-admin.json"));
        assert_eq!(d.suggested, Some(LandingMode::PullRequests), "{file}");
        assert!(!d.direct_impossible, "{file}: it can't be confirmed either way");
        assert_eq!(d.reasons, [format!("main has a {kind} rule, which a direct push can't satisfy.")], "{file}");
    }
}

#[test]
fn a_rule_of_a_kind_nobody_knows_is_never_read_as_harmless() {
    let d = decide(&facts("rules-unknown-kind.json", "branch-unprotected.json", "repo-view-admin.json"));
    assert_eq!(d.suggested, Some(LandingMode::PullRequests));
    assert!(d.reasons[0].contains("(some_future_rule)"), "{:?}", d.reasons);
}

#[test]
fn rules_about_the_shape_of_a_push_do_not_stop_a_direct_one() {
    let d = decide(&facts("rules-harmless.json", "branch-unprotected.json", "repo-view-admin.json"));
    assert_eq!(d.suggested, Some(LandingMode::Direct), "{:?}", d.reasons);
}

#[test]
fn classic_protection_s_required_checks_are_used_when_readable() {
    let f = facts("rules-unprotected.json", "branch-protected-checks.json", "repo-view-admin.json");
    assert_eq!(f.protection_checks, ["rust", "swift"]);
    let d = decide(&f);
    assert_eq!(d.suggested, Some(LandingMode::PullRequests));
    assert_eq!(d.reasons, ["main needs 2 required checks to pass before a change lands, which a direct push can't wait for."]);
    assert!(parse_protection_checks(&fixture("branch-unprotected.json")).is_empty());
    assert!(parse_protection_checks(b"nonsense").is_empty());
}

#[test]
fn the_merge_queue_s_own_method_is_honored() {
    let d = decide(&facts("rules-merge-queue-rebase.json", "branch-protected.json", "repo-view-all-methods.json"));
    assert_eq!(d.merge_method, Some("rebase"), "squash is allowed, but the queue merges by rebase");
    let d = decide(&facts("rules-merge-queue.json", "branch-protected.json", "repo-view-all-methods.json"));
    assert_eq!(d.merge_method, Some("squash"));
}

/// Two rulesets apply together, so a method must be allowed by both: the first
/// allows only rebase, the second all three. Taking the last one's list would
/// say squash.
#[test]
fn methods_are_combined_across_rulesets() {
    let f = facts("rules-two-rulesets.json", "branch-protected.json", "repo-view-all-methods.json");
    assert_eq!(f.rules.as_ref().unwrap().allowed_methods, Some(vec!["rebase".to_string()]));
    assert_eq!(decide(&f).merge_method, Some("rebase"));
}

#[test]
fn every_page_of_the_rules_is_read() {
    let r = parse_rules(&fixture("rules-paged.json")).unwrap();
    assert!(r.restricts_updates, "a rule on the second page counts");
    assert_eq!(parse_rules(&fixture("rules-update.json")).unwrap(), r_of_update());
}

fn r_of_update() -> Rules {
    Rules { restricts_updates: true, ..Default::default() }
}

#[test]
fn merge_group_counts_only_as_a_trigger() {
    let if_expr = "on:\n  push:\njobs:\n  a:\n    if: github.event_name == 'merge_group'\n    steps:\n      - run: echo merge_group\n";
    let step_name = "on: [push]\njobs:\n  a:\n    steps:\n      - name: merge_group smoke\n";
    let quoted_on = "\"on\":\n  merge_group:\n";
    let multiline_flow = "on:\n  [push,\n   merge_group]\n";
    let after_other_keys = "name: CI\non:\n  push:\n    branches: [main]\n  merge_group:\njobs: {}\n";
    let flow_map = "on: {merge_group: {types: [checks_requested]}}\n";
    assert!(!runs_on_merge_group(if_expr), "an if: expression and a script aren't triggers");
    assert!(!runs_on_merge_group(step_name));
    assert!(runs_on_merge_group(quoted_on));
    assert!(runs_on_merge_group(multiline_flow));
    assert!(runs_on_merge_group(after_other_keys));
    assert!(runs_on_merge_group(flow_map));
}

#[test]
fn an_indentless_list_under_on_and_a_bom_are_read() {
    assert!(runs_on_merge_group("on:\n- push\n- merge_group\njobs: {}\n"));
    assert!(!runs_on_merge_group("on:\n- push\njobs:\n- merge_group\n"), "a list under another key isn't the trigger");
    assert!(runs_on_merge_group("\u{feff}on:\n  merge_group:\n"));
    assert!(runs_on_merge_group("\u{feff}on: [push, merge_group]\n"));
}

#[test]
fn rulesets_that_allow_no_method_together_say_so_as_a_warning() {
    let mut f = facts("rules-two-rulesets.json", "branch-protected.json", "repo-view-all-methods.json");
    f.rules.as_mut().unwrap().allowed_methods = Some(vec![]);
    let d = decide(&f);
    assert_eq!(d.merge_method, None);
    assert_eq!(d.warnings, ["No merge method is allowed by both the repository and its rules, so a pull request can't be merged."]);
    // The repository allows none either.
    let mut g = unprotected();
    let repo = g.repo.as_mut().unwrap();
    (repo.squash, repo.rebase, repo.merge_commit) = (Some(false), Some(false), Some(false));
    assert_eq!(decide(&g).warnings.len(), 1);
    // Not read is not "none allowed": no warning.
    g.repo.as_mut().unwrap().squash = None;
    assert!(decide(&g).warnings.is_empty());
    g.repo = None;
    assert!(decide(&g).warnings.is_empty());
}

#[test]
fn pages_printed_back_to_back_by_an_older_gh_are_read_too() {
    let both = [fixture("rules-update.json"), fixture("rules-harmless.json")].concat();
    let r = parse_rules(&both).unwrap();
    assert!(r.restricts_updates && r.unknown.is_empty());
    assert_eq!(parse_rules(b""), None);
    assert_eq!(parse_rules(b"[][oops"), None);
}
