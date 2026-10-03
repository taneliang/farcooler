//! `farcooler report`: the period it sends, the words it prints, and how it
//! is refused.

use farcooler_daemon::report::{
    Acceptance, Decisions, NeedsYou, Notable, NotableTask, NotableWait, Scope, StatusTime, Usage,
};
use farcooler_transport::ClientError;

use super::*;

const NOW: i64 = 1_790_000_000_000;
const REPO: Uuid = Uuid::from_u128(0x22);

fn args(since: Option<&str>, until: Option<&str>) -> ReportArgs {
    ReportArgs { since: since.map(Into::into), until: until.map(Into::into), repo: None, workspace: None }
}

/// A link that answers `report.get` with `report`, and lists one
/// repository, `overnight`.
struct FakeLink {
    capabilities: Vec<String>,
    answer: Result<String, ClientError>,
    sent: Vec<pb::Request>,
}

impl FakeLink {
    fn answering(report: &Report) -> FakeLink {
        FakeLink {
            capabilities: vec![farcooler_protocol::capability::REPORT.into()],
            answer: Ok(serde_json::to_string(report).unwrap()),
            sent: Vec::new(),
        }
    }
}

impl DispatchLink for FakeLink {
    fn capabilities(&self) -> Vec<String> {
        self.capabilities.clone()
    }
    async fn pause(&mut self, _wait: std::time::Duration) {}
    async fn call(&mut self, req: pb::Request) -> Result<pb::Result, ClientError> {
        self.sent.push(req.clone());
        let value = match req.method.as_str() {
            "repository.list" => result::Value::RepositoryList(pb::RepositoryList {
                items: vec![pb::Repository {
                    id: bytes::Bytes::copy_from_slice(REPO.as_bytes()),
                    display_name: "overnight".into(),
                    ..pb::Repository::default()
                }],
            }),
            "report.get" => match &self.answer {
                Ok(json) => result::Value::Report(pb::Report { report_json: json.clone() }),
                Err(ClientError::Daemon { code, retryable, message, what }) => {
                    return Err(ClientError::Daemon {
                        code: *code,
                        retryable: *retryable,
                        message: message.clone(),
                        what: what.clone(),
                    });
                }
                Err(_) => unreachable!(),
            },
            other => panic!("unexpected {other}"),
        };
        Ok(pb::Result { value: Some(value) })
    }
}

fn empty_report(since: i64, until: i64) -> Report {
    Report {
        schema: farcooler_daemon::report::SCHEMA,
        since,
        until,
        generated_at: until,
        scope: Scope::default(),
        totals: Tally::default(),
        by_repository: Vec::new(),
        by_workspace: Vec::new(),
        by_area: Vec::new(),
        by_label: Vec::new(),
        notable: Notable::default(),
    }
}

fn a_week() -> Report {
    let mut r = empty_report(NOW - 7 * DAY, NOW);
    r.totals = Tally {
        created: 5,
        completed: 3,
        canceled: 1,
        reopened: 1,
        fix_rounds: 2,
        time_to_done: Some(Spread { count: 3, median_ms: 3 * HOUR + 10 * MINUTE, p90_ms: 2 * DAY + 5 * HOUR }),
        time_in_status: vec![StatusTime { status: "in_progress".into(), total_ms: 41 * HOUR, tasks: 4, median_ms: 9 * HOUR }],
        decisions: Decisions {
            asked: 2,
            answered: 1,
            answered_by_you: 1,
            unanswered: 1,
            latency: Some(Spread { count: 1, median_ms: 6 * MINUTE, p90_ms: 6 * MINUTE }),
            latency_you: Some(Spread { count: 1, median_ms: 6 * MINUTE, p90_ms: 6 * MINUTE }),
            recorded: 4,
            ..Decisions::default()
        },
        needs_you: NeedsYou { times: 3, decisions: 2, reviews: 1, cleared: 2, waiting: 1, time_to_clear: None },
        acceptance: Acceptance { met: 4, total: 5, tasks_fully_met: 1, tasks_without_lines: 1 },
        usage: None,
    };
    r.notable.slowest = vec![NotableTask {
        key: "ov-178".into(),
        title: "Mac: retire the old fleet sidebar".into(),
        workspace: "Main".into(),
        ms: Some(2 * DAY + 5 * HOUR),
        count: None,
    }];
    r.notable.longest_waits = vec![NotableWait {
        key: "ov-182".into(),
        title: "Apps: relaunch returns to where you were".into(),
        workspace: "Main".into(),
        kind: "question".into(),
        ms: 6 * HOUR,
        open: true,
    }];
    r
}

#[test]
fn the_summary_says_what_happened_in_plain_words() {
    let text = render(&a_week(), Some("Last 7 days"));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "Last 7 days · Every repository on this runner");
    for expected in [
        "3 done, 1 canceled, 5 new",
        "Median time to done 3 h 10 min; 90% within 2 d 5 h",
        "1 task reopened, 2 fix rounds",
        "Acceptance: 4 of 5 lines met; 1 of 2 tasks finished every line",
        "Questions: 2 asked, 1 answered, 1 still open",
        "  You answered 1, in a median of 6 min",
        "Needs you: 3 times (2 for a decision, 1 for review); 1 waiting",
        "Decisions recorded: 4",
        "  In Progress     41 h across 4 tasks",
        "    ov-178    Mac: retire the old fleet sidebar             2 d 5 h",
        "    ov-182    Apps: relaunch returns to where you were      question, 6 h, still waiting",
    ] {
        assert!(lines.contains(&expected), "missing {expected:?} in\n{text}");
    }
    assert!(!text.contains("orchestrator answered"), "a zero is left out:\n{text}");
    assert!(!text.contains("Median wait"), "every answer was yours: the line above says it:\n{text}");
    assert!(!text.contains("tokens"), "no usage, no line:\n{text}");
}

#[test]
fn usage_gets_a_line_once_the_runner_records_it() {
    let mut r = a_week();
    r.totals.usage = Some(Usage { input_tokens: Some(2_000_000), output_tokens: Some(100_000), agent_ms: Some(41 * HOUR), ..Usage::default() });
    assert!(render(&r, None).lines().any(|l| l == "Agent time 41 h · 2.1 M tokens"), "{}", render(&r, None));
}

#[test]
fn an_empty_period_says_so() {
    let text = render(&empty_report(NOW - DAY, NOW), Some("Last day"));
    assert!(text.ends_with("Nothing happened on the board in this period."), "{text}");
    assert!(!text.contains("0 done"), "{text}");
}

#[test]
fn spans_read_as_a_person_would_say_them() {
    assert_eq!(span(30_000), "under a minute");
    assert_eq!(span(6 * MINUTE + 59_000), "6 min");
    assert_eq!(span(3 * HOUR), "3 h");
    assert_eq!(span(3 * HOUR + 10 * MINUTE), "3 h 10 min");
    assert_eq!(span(DAY), "1 d");
    assert_eq!(span(2 * DAY + 5 * HOUR + 59 * MINUTE), "2 d 5 h");
    assert_eq!(hours(41 * HOUR + 59 * MINUTE), "41 h", "a total stays in hours");
    assert_eq!(hours(3 * HOUR + 10 * MINUTE + 30_000), "3 h 10 min");
}

#[test]
fn the_period_defaults_to_the_last_7_days() {
    let p = period(&args(None, None), NOW).unwrap();
    assert_eq!(p, Period { since: NOW - 7 * DAY, until: NOW, label: Some("Last 7 days".into()) });
    let p = period(&args(Some("24h"), None), NOW).unwrap();
    assert_eq!((p.since, p.label.as_deref()), (NOW - 24 * HOUR, Some("Last 24 hours")));
    let p = period(&args(Some("2w"), Some("1w")), NOW).unwrap();
    assert_eq!((p.since, p.until, p.label), (NOW - 14 * DAY, NOW - 7 * DAY, None));
}

#[test]
fn a_date_is_local_midnight() {
    let since = when("2026-09-28", NOW).unwrap();
    let tm = broken_down(since);
    assert_eq!((tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min), (2026, 9, 28, 0, 0));
    let later = when("2026-09-28 14:30", NOW).unwrap();
    assert_eq!(later - since, 14 * HOUR + 30 * MINUTE);
    assert_eq!(when("2026-09-28T14:30", NOW), Ok(later));
    let today = when("today", NOW).unwrap();
    assert_eq!(broken_down(today).tm_hour, 0);
    assert!(today <= NOW && NOW - today < DAY);
    assert!(when("yesterday", NOW).unwrap() < today);
}

#[test]
fn a_period_that_cannot_be_read_is_refused_before_connecting() {
    assert!(period(&args(Some("1d"), Some("2d")), NOW).unwrap_err().contains("ends before it starts"));
    assert!(when("last tuesday", NOW).unwrap_err().contains("isn't a time"));
    assert!(when("2026-13-01", NOW).is_err());
    assert!(when("0d", NOW).is_err());
}

#[tokio::test]
async fn it_asks_for_the_period_and_prints_the_runners_json() {
    let report = a_week();
    let mut link = FakeLink::answering(&report);
    let p = Period { since: NOW - 7 * DAY, until: NOW, label: None };
    let printed = report_read(&mut link, &args(None, None), p, true).await.unwrap();
    assert_eq!(serde_json::from_str::<Report>(&printed).unwrap(), report);
    let Some(request::Payload::ReportRequest(asked)) = &link.sent[0].payload else { panic!("{:?}", link.sent) };
    assert_eq!((asked.since, asked.until), (NOW - 7 * DAY, NOW));
    assert_eq!((asked.repository_id.as_ref(), asked.workspace_id.as_ref()), (None, None), "the whole runner");
}

#[tokio::test]
async fn repo_narrows_to_that_repository() {
    let mut link = FakeLink::answering(&a_week());
    let mut a = args(None, None);
    a.repo = Some("Overnight".into());
    let p = Period { since: 0, until: NOW, label: None };
    report_read(&mut link, &a, p, false).await.unwrap();
    let Some(request::Payload::ReportRequest(asked)) = &link.sent[1].payload else { panic!("{:?}", link.sent) };
    assert_eq!(asked.repository_id.as_deref(), Some(REPO.as_bytes().as_slice()));
}

#[tokio::test]
async fn a_runner_without_reports_is_told_to_update() {
    let mut link = FakeLink::answering(&a_week());
    link.capabilities.clear();
    let p = Period { since: 0, until: NOW, label: None };
    let refused = report_read(&mut link, &args(None, None), p, false).await.unwrap_err();
    assert_eq!(refused.to_string(), NO_REPORT);
    assert!(link.sent.is_empty(), "refused without a round trip");

    let mut link = FakeLink::answering(&a_week());
    link.answer = Err(ClientError::Daemon {
        code: pb::ErrorCode::NotFound as i32,
        retryable: false,
        message: "resource not found".into(),
        what: String::new(),
    });
    let p = Period { since: 0, until: NOW, label: None };
    let refused = report_read(&mut link, &args(None, None), p, false).await.unwrap_err();
    assert_eq!(refused.to_string(), "that repository or workspace isn't on this runner");
}
