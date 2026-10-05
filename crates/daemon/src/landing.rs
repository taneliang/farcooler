//! Which way a repository can land work, from what GitHub says (ov-313,
//! ov-305 section 6.2): the pure half. What the runner read is parsed here,
//! and `decide` turns the facts into a suggestion with reasons. Reading them
//! (`landing_read`) is a second file, so every decision is a test on recorded
//! answers with no `gh` in it.
//!
//! **Detection suggests and never switches.** The mode a board lands by is a
//! setting (`farcooler_store::landing`) that only `workspace.set_settings`
//! writes. What this decides is a suggestion, and one fact: whether
//! `direct` can work at all.
//!
//! **An unread fact is not a false one.** Every fact is an `Option`. A branch
//! whose rules could not be read is not an unprotected one, so `Direct` is
//! suggested only when the rules and the branch's protection were both read
//! and neither asks for pull requests; with less than that the suggestion is
//! nothing at all, and the reason says what could not be read.
//!
//! | Facts | Direct possible | Suggested |
//! |---|---|---|
//! | a `pull_request`, `merge_queue` or `update` rule, or a login that can only read | no | pull requests |
//! | required checks (rules or classic protection), `required_deployments`, `workflows`, `code_scanning`, a rule of a kind this build doesn't know, or a protected branch whose details can't be read | yes, unless the owner can bypass | pull requests |
//! | rules, protection and what this login may do all read, none asking for anything | yes | direct |
//! | any of those three unread, and nothing else found | unknown | nothing |
//!
//! A rule of a kind not on the known-harmless list (`creation`, `deletion`,
//! `non_fast_forward`, `required_linear_history`, `required_signatures`, and
//! the pattern and file rules) is read as one that might refuse a push: a
//! direct suggestion must never be wrong in that direction.

use farcooler_store::landing::LandingMode;

/// What the base branch's rules ask for, as `GET /repos/{o}/{r}/rules/branches/{base}` lists them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules {
    pub pull_request: bool,
    /// `required_approving_review_count` of the pull request rule.
    pub approvals: u32,
    /// The merge methods the pull request rule allows, when it limits them.
    pub allowed_methods: Option<Vec<String>>,
    pub merge_queue: bool,
    /// The merge queue's own method (`squash`, `rebase`, `merge`): the queue
    /// merges by it, whatever else is allowed.
    pub queue_method: Option<String>,
    pub required_checks: Vec<String>,
    /// Rules that restrict who may update the branch: direct can't work.
    pub restricts_updates: bool,
    /// Rule types that a direct push can't satisfy (`required_deployments`,
    /// `workflows`, `code_scanning`), by name.
    pub leaning: Vec<String>,
    /// Rule types this build doesn't know, by name.
    pub unknown: Vec<String>,
}

/// What `gh repo view` says of the repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoView {
    pub default_branch: Option<String>,
    pub squash: Option<bool>,
    pub rebase: Option<bool>,
    pub merge_commit: Option<bool>,
    /// As GitHub says it: `ADMIN`, `MAINTAIN`, `WRITE`, `TRIAGE`, `READ`.
    pub viewer_permission: Option<String>,
}

/// Everything read about one base branch; `None` is a read that failed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub base: String,
    pub rules: Option<Rules>,
    pub protected: Option<bool>,
    /// Required checks of classic branch protection, which anyone with read
    /// access can list.
    pub protection_checks: Vec<String>,
    pub repo: Option<RepoView>,
    pub codeowners: Option<bool>,
    pub merge_group_workflow: Option<bool>,
}

/// What the facts add up to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Decision {
    pub suggested: Option<LandingMode>,
    pub direct_impossible: bool,
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
    /// `squash`, `rebase` or `merge`.
    pub merge_method: Option<&'static str>,
}

/// Rule types that can't refuse a direct push on their own.
const HARMLESS: [&str; 12] = [
    "creation",
    "deletion",
    "non_fast_forward",
    "required_linear_history",
    "required_signatures",
    "commit_message_pattern",
    "commit_author_email_pattern",
    "committer_email_pattern",
    "branch_name_pattern",
    "tag_name_pattern",
    "file_path_restriction",
    "file_extension_restriction",
];

/// Other rules about the shape of a push (size limits).
fn harmless(kind: &str) -> bool {
    HARMLESS.contains(&kind) || matches!(kind, "max_file_path_length" | "max_file_size")
}

/// The rules list, from `gh api --paginate --slurp .../rules/branches/{base}`:
/// one array per page, or a single flat array.
pub fn parse_rules(bytes: &[u8]) -> Option<Rules> {
    // One array per page, as `--paginate` prints them back to back or
    // `--slurp` wraps them, or a single flat array.
    let pages: Vec<serde_json::Value> =
        serde_json::Deserializer::from_slice(bytes).into_iter::<serde_json::Value>().collect::<Result<_, _>>().ok()?;
    if pages.is_empty() {
        return None;
    }
    // Pages may be nested (`--slurp` wraps them): every array is flattened.
    fn flatten<'a>(v: &'a serde_json::Value, into: &mut Vec<&'a serde_json::Value>) {
        match v.as_array() {
            Some(items) => items.iter().for_each(|i| flatten(i, into)),
            None => into.push(v),
        }
    }
    let mut list: Vec<&serde_json::Value> = Vec::new();
    pages.iter().for_each(|p| flatten(p, &mut list));
    // An error object is neither a rule nor an empty list.
    if list.iter().any(|r| r.get("type").is_none()) {
        return None;
    }
    let mut rules = Rules::default();
    for rule in list {
        let params = rule.get("parameters");
        let kind = rule.get("type")?.as_str()?;
        match kind {
            "pull_request" => {
                rules.pull_request = true;
                let n = params.and_then(|p| p.get("required_approving_review_count")).and_then(|n| n.as_u64());
                rules.approvals = rules.approvals.max(n.unwrap_or(0) as u32);
                let methods = params.and_then(|p| p.get("allowed_merge_methods")).and_then(|m| m.as_array());
                if let Some(methods) = methods {
                    let names: Vec<String> = methods.iter().filter_map(|m| m.as_str().map(str::to_string)).collect();
                    // Several rulesets apply together: a method must be allowed by all.
                    rules.allowed_methods = Some(match rules.allowed_methods.take() {
                        Some(before) => before.into_iter().filter(|m| names.contains(m)).collect(),
                        None => names,
                    });
                }
            }
            "merge_queue" => {
                rules.merge_queue = true;
                let method = params.and_then(|p| p.get("merge_method")).and_then(|m| m.as_str());
                if let Some(method) = method {
                    rules.queue_method = Some(method.to_lowercase());
                }
            }
            "required_status_checks" => {
                let checks = params.and_then(|p| p.get("required_status_checks")).and_then(|c| c.as_array());
                for check in checks.into_iter().flatten() {
                    if let Some(name) = check.get("context").and_then(|c| c.as_str())
                        && !rules.required_checks.iter().any(|c| c == name)
                    {
                        rules.required_checks.push(name.to_string());
                    }
                }
            }
            "update" => rules.restricts_updates = true,
            "required_deployments" | "workflows" | "code_scanning" => {
                if !rules.leaning.iter().any(|k| k == kind) {
                    rules.leaning.push(kind.to_string());
                }
            }
            other if harmless(other) => {}
            other => {
                if !rules.unknown.iter().any(|k| k == other) {
                    rules.unknown.push(other.to_string());
                }
            }
        }
    }
    Some(rules)
}

/// The required checks of classic protection, from the same answer
/// `parse_protected` reads: `protection.required_status_checks`' `contexts`
/// and `checks[].context`, when protection is on.
pub fn parse_protection_checks(bytes: &[u8]) -> Vec<String> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) else { return vec![] };
    let Some(checks) = v.get("protection").filter(|p| p.get("enabled").and_then(|e| e.as_bool()) != Some(false)).and_then(|p| p.get("required_status_checks")) else {
        return vec![];
    };
    let mut names: Vec<String> = Vec::new();
    let contexts = checks.get("contexts").and_then(|c| c.as_array()).into_iter().flatten().filter_map(|c| c.as_str());
    let structured = checks.get("checks").and_then(|c| c.as_array()).into_iter().flatten().filter_map(|c| c.get("context").and_then(|c| c.as_str()));
    for name in contexts.chain(structured) {
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    names
}

/// `protected`, from `gh api repos/{owner}/{repo}/branches/{base}`.
pub fn parse_protected(bytes: &[u8]) -> Option<bool> {
    serde_json::from_slice::<serde_json::Value>(bytes).ok()?.get("protected")?.as_bool()
}

/// `gh repo view --json defaultBranchRef,squashMergeAllowed,rebaseMergeAllowed,mergeCommitAllowed,viewerPermission`.
pub fn parse_repo_view(bytes: &[u8]) -> Option<RepoView> {
    let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let text = |p: Option<&serde_json::Value>| {
        p.and_then(|s| s.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    };
    Some(RepoView {
        default_branch: text(v.get("defaultBranchRef").and_then(|r| r.get("name"))),
        squash: v.get("squashMergeAllowed").and_then(|b| b.as_bool()),
        rebase: v.get("rebaseMergeAllowed").and_then(|b| b.as_bool()),
        merge_commit: v.get("mergeCommitAllowed").and_then(|b| b.as_bool()),
        viewer_permission: text(v.get("viewerPermission")),
    })
}

/// A line's YAML with its comment cut off.
fn code_of(line: &str) -> &str {
    line.split('#').next().unwrap_or("")
}

fn has_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|w| w == word)
}

/// Whether a workflow file's text runs on `merge_group`: the word, in a
/// comment-free line of the top-level `on:` key (`on: merge_group`,
/// `on: [push, merge_group]`, a `merge_group:` key or a `- merge_group` item
/// under it). The same word in an `if:` expression, a step's name or a script
/// isn't a trigger, so it doesn't count. A YAML parser would be exact; this
/// reads only the one block, and a flow list continued over indented lines.
pub fn runs_on_merge_group(workflow: &str) -> bool {
    let workflow = workflow.strip_prefix('\u{feff}').unwrap_or(workflow);
    let mut in_on = false;
    for line in workflow.lines() {
        let code = code_of(line);
        if code.trim().is_empty() {
            continue;
        }
        let top_level = !code.starts_with([' ', '\t']);
        // `on:` then `- push` at the key's own indentation is a list under it.
        if in_on && code.starts_with('-') {
            if has_word(code, "merge_group") {
                return true;
            }
        } else if top_level {
            let key = code.split(':').next().unwrap_or("").trim().trim_matches(['"', '\'']);
            in_on = matches!(key, "on" | "true");
            if in_on && code.contains(':') && has_word(code.split_once(':').map_or("", |(_, rest)| rest), "merge_group") {
                return true;
            }
        } else if in_on && has_word(code, "merge_group") {
            return true;
        }
    }
    false
}

/// How many approvals, in a sentence's words.
fn approvals_words(n: u32) -> String {
    match n {
        0 => String::new(),
        1 => ", with one approving review".into(),
        n => format!(", with {n} approving reviews"),
    }
}

/// The merge method a pull request should use: squash, then rebase, then a
/// merge commit, among the ones the repository allows and the rules permit.
fn merge_method(facts: &Facts) -> Option<&'static str> {
    // The merge queue merges by its own method.
    if let Some(queued) = facts.rules.as_ref().and_then(|r| r.queue_method.as_deref()) {
        return ["squash", "rebase", "merge"].into_iter().find(|m| *m == queued);
    }
    let repo = facts.repo.as_ref()?;
    let ruled = |name: &str| {
        facts.rules.as_ref().and_then(|r| r.allowed_methods.as_ref()).is_none_or(|m| m.iter().any(|x| x == name))
    };
    [("squash", repo.squash), ("rebase", repo.rebase), ("merge", repo.merge_commit)]
        .into_iter()
        .find(|(name, allowed)| *allowed == Some(true) && ruled(name))
        .map(|(name, _)| name)
}

/// Whether every merge method was read and none is allowed by both the
/// repository and the rules: a fact, unlike a method that was not read.
fn no_method_is_allowed(facts: &Facts) -> bool {
    let Some(repo) = &facts.repo else { return false };
    let all_read = [repo.squash, repo.rebase, repo.merge_commit].iter().all(Option::is_some);
    all_read || facts.rules.as_ref().and_then(|r| r.allowed_methods.as_ref()).is_some_and(Vec::is_empty)
}

/// What `facts` add up to. See the module's table.
pub fn decide(facts: &Facts) -> Decision {
    let base = &facts.base;
    let mut reasons = Vec::new();
    let mut impossible = false;
    let mut leans_pull_requests = false;

    let mut required: Vec<String> = Vec::new();
    if let Some(rules) = &facts.rules {
        if rules.pull_request {
            impossible = true;
            reasons.push(format!("{base} requires pull requests{}.", approvals_words(rules.approvals)));
        }
        if rules.merge_queue {
            impossible = true;
            reasons.push(format!("{base} uses a merge queue, which only pull requests can enter."));
        }
        if rules.restricts_updates {
            impossible = true;
            reasons.push(format!("{base} restricts who can update it, so a direct push is refused."));
        }
        required.extend(rules.required_checks.iter().cloned());
        for kind in &rules.leaning {
            leans_pull_requests = true;
            reasons.push(format!("{base} has a {kind} rule, which a direct push can't satisfy."));
        }
        for kind in &rules.unknown {
            leans_pull_requests = true;
            reasons.push(format!("{base} has a rule of a kind Far Cooler doesn't know ({kind}), so it can't confirm a direct push would work."));
        }
    }
    for name in &facts.protection_checks {
        if !required.contains(name) {
            required.push(name.clone());
        }
    }
    if !required.is_empty() {
        leans_pull_requests = true;
        let n = required.len();
        let checks = if n == 1 { "1 required check".to_string() } else { format!("{n} required checks") };
        reasons.push(format!("{base} needs {checks} to pass before a change lands, which a direct push can't wait for."));
    }
    let permission = facts.repo.as_ref().and_then(|r| r.viewer_permission.as_deref());
    if let Some(permission) = permission.filter(|p| matches!(*p, "READ" | "TRIAGE")) {
        impossible = true;
        reasons.push(format!(
            "This GitHub login can't push to the repository: it has {} access.",
            permission.to_lowercase()
        ));
    }
    let rules_ask_nothing = facts.rules.as_ref().is_some_and(|r| !r.pull_request && !r.merge_queue);
    if facts.protected == Some(true) && !leans_pull_requests && !impossible {
        leans_pull_requests = true;
        let why = if permission == Some("ADMIN") {
            "its classic protection isn't something Far Cooler reads".to_string()
        } else if rules_ask_nothing {
            "GitHub only shows the details of its protection to an admin".to_string()
        } else {
            "its rules could not be read".to_string()
        };
        reasons.push(format!("{base} is protected and {why}, so pull requests are the safe way to land."));
    }

    let can_push = matches!(permission, Some("ADMIN" | "MAINTAIN" | "WRITE"));
    let suggested = if impossible || leans_pull_requests {
        Some(LandingMode::PullRequests)
    } else if facts.rules.is_some() && facts.protected == Some(false) && can_push {
        reasons.push(format!("Nothing on {base} asks for pull requests."));
        Some(LandingMode::Direct)
    } else {
        let mut missing = Vec::new();
        if facts.rules.is_none() {
            missing.push("its rules");
        }
        if facts.protected.is_none() {
            missing.push("whether it is protected");
        }
        if permission.is_none() {
            missing.push("what this login may do");
        }
        reasons.push(format!("Far Cooler couldn't read {} on {base}, so it suggests nothing.", missing.join(" or ")));
        None
    };

    let mut warnings = Vec::new();
    let method = merge_method(facts);
    if method.is_none() && no_method_is_allowed(facts) {
        warnings.push("No merge method is allowed by both the repository and its rules, so a pull request can't be merged.".to_string());
    }
    if facts.rules.as_ref().is_some_and(|r| r.merge_queue) {
        match facts.merge_group_workflow {
            Some(false) => warnings.push(
                "The merge queue will wait forever: no workflow runs on `merge_group`.".to_string(),
            ),
            None => warnings.push(
                "Far Cooler couldn't check whether a workflow runs on `merge_group`, which a merge queue needs.".to_string(),
            ),
            Some(true) => {}
        }
    }
    Decision { suggested, direct_impossible: impossible, reasons, warnings, merge_method: method }
}

#[cfg(test)]
#[path = "landing_tests.rs"]
mod tests;
