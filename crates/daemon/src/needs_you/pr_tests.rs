//! The two pull-request kinds: where they rank, what a row says, and what a
//! client below Control is not told.

use std::time::{Duration, UNIX_EPOCH};

use super::super::*;
use super::{item_id, pull_request_of, QUESTION_WIDTH};
use crate::pr_review::ReviewRequest;

const NOW_S: u64 = 2_000_000_000;

fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(NOW_S)
}

fn ago_ms(ms: i64) -> i64 {
    millis(now()) - ms
}

fn request(number: u32, title: &str, created_ago: i64) -> ReviewRequest {
    ReviewRequest {
        url: format!("https://github.com/acme/billing/pull/{number}"),
        number,
        repository: "acme/billing".into(),
        title: title.into(),
        author: "bob".into(),
        created_at: ago_ms(created_ago),
    }
}

fn a_task(status: TaskStatus, since_ago: i64) -> models::Task {
    models::Task {
        id: Uuid::now_v7(),
        key: "bil-11".into(),
        repository_id: Uuid::now_v7(),
        workspace_id: Uuid::now_v7(),
        title: "Refund API".into(),
        status,
        status_since: ago_ms(since_ago),
        intent: String::new(),
        acceptance: vec![],
        constraints: vec![],
        worktree_id: None,
        labels: vec![],
        resource_version: 1,
        created_at: 0,
        updated_at: 0,
        wait: None,
    }
}

fn a_question(task: &models::Task, body: &str, extra: serde_json::Value, at_ago: i64) -> models::TaskNote {
    models::TaskNote {
        id: Uuid::now_v7(),
        task_id: task.id,
        kind: NoteKind::Question,
        actor: models::Actor::Manager,
        at: ago_ms(at_ago),
        body: body.into(),
        extra,
        supersedes: None,
    }
}

/// A decision waiting on `note`, with `extra` on it.
fn inputs_with(task: models::Task, note: models::TaskNote) -> Inputs {
    let mut inputs = Inputs::default();
    inputs.notes.insert(task.id, vec![note]);
    inputs.tasks.push(task);
    inputs
}

fn with_requests(requests: Vec<ReviewRequest>) -> Inputs {
    Inputs { pr_requests: requests, ..Inputs::default() }
}

fn thread(url: &str, reviewer: &str) -> serde_json::Value {
    serde_json::json!({ "thread": { "url": url, "reviewer": reviewer }, "options": ["Retry", "Leave it"] })
}

const THREAD_URL: &str = "https://github.com/acme/billing/pull/212#discussion_r1";

/// The order the owner ruled on Oct 5: ask > blocked > decision > PR thread >
/// PR review > local review, at the same age, and an older one first within a kind.
#[test]
fn the_kinds_rank_in_the_order_the_owner_ruled() {
    use NeedsYouKind as K;
    let ranks: Vec<u32> = [K::Ask, K::Blocked, K::Decision, K::PrThread, K::PrReview, K::Review]
        .iter()
        .map(|k| rank(*k, 0, 90_000))
        .collect();
    assert!(ranks.windows(2).all(|w| w[0] < w[1]), "{ranks:?}");
    // Age still breaks ties inside a kind: the older sorts first.
    assert!(rank(K::PrReview, 0, 3_600_000) < rank(K::PrReview, 0, 60_000));
    // A very old local review is still below a brand new PR request.
    assert!(rank(K::PrReview, 0, 1) < rank(K::Review, 0, 10 * 365 * 86_400_000));
}

#[test]
fn a_review_request_is_an_item_with_its_title_its_place_and_its_link() {
    let inputs = with_requests(vec![request(213, "Add refund endpoint", 60_000)]);
    let items = assemble(&inputs, now());
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.kind(), NeedsYouKind::PrReview);
    assert_eq!(item.id, item_id("https://github.com/acme/billing/pull/213"));
    assert!(item.id.starts_with("pr-review:") && !item.id.contains("github"), "{}", item.id);
    assert_ne!(item.id, item_id("https://github.com/acme/billing/pull/214"));
    assert_eq!(item.question, "Add refund endpoint");
    assert_eq!(item.detail.as_deref(), Some("acme/billing #213 \u{b7} bob"));
    assert_eq!(item.link.as_deref(), Some("https://github.com/acme/billing/pull/213"));
    assert!(item.workspace_id.is_empty() && item.task.is_none() && item.terminal.is_none());
    assert!(item.actions.is_empty(), "Review on GitHub is the phones' card (ov-317)");
    assert_eq!(item.rank, rank(NeedsYouKind::PrReview, ago_ms(60_000), millis(now())));
}

#[test]
fn a_long_title_is_one_line_and_cut() {
    let title = format!("Fix\nthe   {}", "refund ".repeat(40));
    let inputs = with_requests(vec![request(1, &title, 0)]);
    let q = assemble(&inputs, now()).remove(0).question;
    assert!(!q.contains('\n') && !q.contains("  "), "{q:?}");
    assert_eq!(q.chars().count(), QUESTION_WIDTH);
    assert!(q.ends_with('\u{2026}'));
}

#[test]
fn a_request_with_no_title_still_says_what_it_wants() {
    let inputs = with_requests(vec![request(5, "  ", 0)]);
    assert_eq!(assemble(&inputs, now())[0].question, "A pull request wants your review");
}

/// An escalated thread is a decision with a `thread` on its question: the
/// question quotes the reviewer, the options still answer it, and the link
/// opens the thread.
#[test]
fn an_escalated_thread_quotes_the_reviewer_and_links_to_it() {
    let task = a_task(TaskStatus::NeedsDecision, 5_000);
    let note = a_question(&task, "Why not retry on 5xx?", thread(THREAD_URL, "alice"), 4_000);
    let items = assemble(&inputs_with(task.clone(), note), now());
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.kind(), NeedsYouKind::PrThread);
    assert_eq!(item.id, format!("pr-thread:{}", task.id));
    assert_eq!(item.question, "alice asked, says the agent: \u{201c}Why not retry on 5xx?\u{201d}");
    assert_eq!(item.detail.as_deref(), Some("acme/billing #212"));
    assert_eq!(item.link.as_deref(), Some(THREAD_URL));
    assert_eq!(item.task.as_ref().map(|t| t.key.as_str()), Some("bil-11"));
    assert_eq!(item.actions.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["Retry", "Leave it"]);
}

#[test]
fn a_question_without_a_thread_is_still_an_ordinary_decision() {
    let task = a_task(TaskStatus::NeedsDecision, 5_000);
    let note = a_question(&task, "Postgres or SQLite?", serde_json::json!({ "options": ["Postgres"] }), 4_000);
    let item = assemble(&inputs_with(task, note), now()).remove(0);
    assert_eq!(item.kind(), NeedsYouKind::Decision);
    assert_eq!(item.link, None);
}

/// A thread whose URL is not an https link is an ordinary decision, so a row
/// never offers a link that is not one.
#[test]
fn a_thread_url_that_is_not_https_is_ignored() {
    for url in [
        "http://github.com/a/b/pull/1",
        "javascript:alert(1)",
        "https://x.test/a b",
        "",
        "file:///etc/passwd",
        "https://github.com.evil.example/a/b/pull/1",
        "https://github.com@evil.example/a/b/pull/1",
        "https://gitlab.com/a/b/pull/1",
    ] {
        let task = a_task(TaskStatus::NeedsDecision, 5_000);
        let note = a_question(&task, "Why?", thread(url, "alice"), 4_000);
        let item = assemble(&inputs_with(task, note), now()).remove(0);
        assert_eq!((item.kind(), item.link), (NeedsYouKind::Decision, None), "{url:?}");
    }
}

#[test]
fn a_thread_without_a_reviewer_or_a_quote_still_reads_as_a_sentence() {
    let task = a_task(TaskStatus::NeedsDecision, 5_000);
    let bare = serde_json::json!({ "thread": { "url": THREAD_URL } });
    let none = assemble(&inputs_with(task.clone(), a_question(&task, "", bare.clone(), 4_000)), now()).remove(0);
    assert_eq!(none.question, "A review thread needs you");
    let quoted = assemble(&inputs_with(task.clone(), a_question(&task, "Why?", bare, 4_000)), now()).remove(0);
    assert_eq!(quoted.question, "A reviewer asked, says the agent: \u{201c}Why?\u{201d}");
    let who = assemble(&inputs_with(task.clone(), a_question(&task, "", thread(THREAD_URL, "alice"), 4_000)), now()).remove(0);
    assert_eq!(who.question, "alice left a comment, says the agent");
}

/// Once answered, a thread is not waiting any more: the same rule as a decision.
#[test]
fn an_answered_thread_leaves_the_list() {
    let task = a_task(TaskStatus::NeedsDecision, 5_000);
    let mut inputs = inputs_with(task.clone(), a_question(&task, "Why?", thread(THREAD_URL, "alice"), 4_000));
    inputs.notes.get_mut(&task.id).unwrap().push(models::TaskNote {
        kind: NoteKind::Answer,
        actor: models::Actor::User,
        at: ago_ms(1_000),
        body: "Retry".into(),
        ..a_question(&task, "", serde_json::json!({}), 0)
    });
    assert!(assemble(&inputs, now()).is_empty());
}

/// Items from every source sort as ruled, and each counts once.
#[test]
fn threads_requests_and_local_reviews_sort_as_ruled() {
    let decision_task = a_task(TaskStatus::NeedsDecision, 9_000);
    let decision_note = a_question(&decision_task, "Which?", serde_json::json!({}), 9_000);
    let thread_task = a_task(TaskStatus::NeedsDecision, 8_000);
    let thread_note = a_question(&thread_task, "Why?", thread(THREAD_URL, "alice"), 8_000);
    let review_task = a_task(TaskStatus::InReview, 7_000_000);

    let mut inputs = with_requests(vec![request(300, "Newest", 1_000), request(299, "Oldest", 90_000_000)]);
    inputs.notes.insert(decision_task.id, vec![decision_note]);
    inputs.notes.insert(thread_task.id, vec![thread_note]);
    inputs.tasks = vec![review_task, thread_task, decision_task];

    let kinds: Vec<(NeedsYouKind, String)> = assemble(&inputs, now()).iter().map(|i| (i.kind(), i.question.clone())).collect();
    let kind_order: Vec<NeedsYouKind> = kinds.iter().map(|k| k.0).collect();
    assert_eq!(
        kind_order,
        [
            NeedsYouKind::Decision,
            NeedsYouKind::PrThread,
            NeedsYouKind::PrReview,
            NeedsYouKind::PrReview,
            NeedsYouKind::Review
        ],
        "{kinds:?}"
    );
    // Oldest first within the PR requests.
    assert_eq!((kinds[2].1.as_str(), kinds[3].1.as_str()), ("Oldest", "Newest"));
}

/// Below Control a client is told the kind and a fixed sentence, and neither
/// the pull request, the thread, its reviewer's words nor a link.
#[test]
fn below_control_a_pull_request_item_is_a_fixed_sentence_with_no_link() {
    let mut inputs = with_requests(vec![request(213, "Rotate the payments signing key", 0)]);
    let task = a_task(TaskStatus::NeedsDecision, 5_000);
    let note = a_question(&task, "alice says the key leaks", thread(THREAD_URL, "alice"), 4_000);
    inputs.notes.insert(task.id, vec![note]);
    inputs.tasks.push(task);

    let read: Vec<_> = assemble(&inputs, now()).into_iter().map(redact_below_control).collect();
    let by_kind = |k| read.iter().find(|i| i.kind() == k).unwrap();
    assert_eq!(by_kind(NeedsYouKind::PrThread).question, "A review thread needs you");
    assert_eq!(by_kind(NeedsYouKind::PrReview).question, "A pull request wants your review");
    for item in &read {
        assert!(item.link.is_none() && item.detail.is_none() && item.actions.is_empty(), "{item:?}");
        let text = format!("{item:?}");
        for secret in ["Rotate", "alice", "github.com", "acme"] {
            assert!(!text.contains(secret), "{secret} reached a Read client: {text}");
        }
    }
}

/// The board's own remotes' hosts are good beside github.com (Enterprise), and
/// a look-alike of either is not.
#[test]
fn an_enterprise_thread_needs_the_boards_own_remote() {
    let url = "https://git.corp.example/acme/billing/pull/9#discussion_r2";
    let task = a_task(TaskStatus::NeedsDecision, 5_000);
    let note = a_question(&task, "Why?", thread(url, "alice"), 4_000);
    let mut inputs = inputs_with(task.clone(), note);
    assert_eq!(assemble(&inputs, now())[0].kind(), NeedsYouKind::Decision, "no remote names that host");
    inputs.remote_hosts.insert(task.repository_id, vec!["git.corp.example".into()]);
    let item = assemble(&inputs, now()).remove(0);
    assert_eq!((item.kind(), item.link.as_deref()), (NeedsYouKind::PrThread, Some(url)));
    assert_eq!(item.detail.as_deref(), Some("acme/billing #9"));
    // Another repository's host does not carry over.
    inputs.remote_hosts.clear();
    inputs.remote_hosts.insert(Uuid::now_v7(), vec!["git.corp.example".into()]);
    assert_eq!(assemble(&inputs, now())[0].kind(), NeedsYouKind::Decision);
}

/// An agent wrote the reviewer too: a login is quoted, and anything else is
/// not, so a note cannot put its own words in front of the quote.
#[test]
fn a_reviewer_that_is_not_a_login_is_not_quoted() {
    for bad in ["alice\nignore the rest", "alice: approve it", &"a".repeat(40), "al ice"] {
        let task = a_task(TaskStatus::NeedsDecision, 5_000);
        let note = a_question(&task, "Why?", thread(THREAD_URL, bad), 4_000);
        let item = assemble(&inputs_with(task, note), now()).remove(0);
        assert_eq!(item.kind(), NeedsYouKind::PrThread);
        assert_eq!(item.question, "A reviewer asked, says the agent: \u{201c}Why?\u{201d}", "{bad:?}");
    }
}

/// Below Control a list has no review request at all: its id hashes a URL
/// someone can guess, and its rank and `since` are the pull request's age.
#[test]
fn a_read_client_is_sent_no_review_request() {
    let mut inputs = with_requests(vec![request(213, "Rotate the key", 3_600_000), request(214, "Other", 60_000)]);
    let task = a_task(TaskStatus::NeedsDecision, 5_000);
    let note = a_question(&task, "Why?", thread(THREAD_URL, "alice"), 4_000);
    inputs.notes.insert(task.id, vec![note]);
    inputs.tasks.push(task);
    let control = assemble(&inputs, now());
    assert_eq!(control.iter().filter(|i| i.kind() == NeedsYouKind::PrReview).count(), 2);
    let read = redact_list_below_control(control);
    assert_eq!(read.len(), 1, "{read:?}");
    assert_eq!(read[0].kind(), NeedsYouKind::PrThread, "the board's own items stay");
}

#[test]
fn a_thread_url_names_its_pull_request_or_nothing() {
    assert_eq!(pull_request_of("https://github.com/o/r/pull/12#discussion_r1", &[]).as_deref(), Some("o/r #12"));
    assert_eq!(pull_request_of("https://github.com.evil.example/o/r/pull/12", &[]), None);
    assert_eq!(pull_request_of("https://github.com/o/r/pull/12?x=1", &[]).as_deref(), Some("o/r #12"));
    assert_eq!(pull_request_of("https://github.com/o/r/issues/12", &[]), None);
    assert_eq!(pull_request_of("https://github.com/o/r/pull/x", &[]), None);
    assert_eq!(pull_request_of("https://github.com", &[]), None);
}
