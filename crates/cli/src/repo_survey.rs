//! `farcooler repo survey` and `farcooler workspace write-charter` (ov-519):
//! the first run's look at a repository, and the charter written from it.
//!
//! ```text
//! farcooler repo survey [--repo NAME] [--json]
//! farcooler workspace write-charter WORKSPACE [--draft] [--landing direct|pull-requests]
//!     [--lanes N|none] [--initiative on|off] [--repo NAME] [--json]
//! ```
//!
//! The survey is the runner's (`repository.survey`): its git, its named
//! files, its own `gh`. No model reads any of it. Each answer prints with the
//! signal it came from, or `default`. `write-charter` asks the runner to
//! survey again and write the workspace's charter with the choices given, or
//! with `--draft`, the draft its orchestrator goes through with the owner.
//! A charter that exists is never replaced; that is said, not refused.

use clap::Args;
use farcooler_core::phrase::{phrase, phrase_within};
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::tasks::{DispatchLink, Refused};
use crate::workspace_settings::LandingArg;
use crate::workspaces::{OnOff, WORKSPACE_ENV, repository_to_create_in, workspaces_on};
use crate::{Fallible, Link, expect_value, req_for, uuid_of, with};

const NEEDS_UPDATE: &str = "This runner needs an update to survey a repository.";

#[derive(Debug, Clone, Args)]
pub struct SurveyArgs {
    /// Which repository. Defaults to the one the pane's own workspace is in,
    /// then to the only one there is.
    #[arg(long)]
    pub repo: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct WriteCharterArgs {
    /// By name, task prefix, or the end of its id.
    pub workspace: String,
    /// Write `charter-draft.md` beside the charter, for the orchestrator to go
    /// through with you, rather than the charter itself.
    #[arg(long)]
    pub draft: bool,
    /// How work lands, when not as the survey found.
    #[arg(long, value_name = "direct|pull-requests")]
    pub landing: Option<LandingArg>,
    /// How many agents at once: 1 to 99, or `none` for no limit.
    #[arg(long, value_name = "N|none")]
    pub lanes: Option<String>,
    /// Whether the orchestrator files its own next steps at each check-in.
    #[arg(long, value_name = "on|off")]
    pub initiative: Option<OnOff>,
    #[arg(long)]
    pub repo: Option<String>,
}

fn needs_survey<L: DispatchLink>(link: &L) -> Result<(), Box<dyn std::error::Error>> {
    if link.capabilities().iter().any(|c| c == capability::REPO_SURVEY) {
        return Ok(());
    }
    Err(Box::new(Refused::new(NEEDS_UPDATE.to_string(), Some(pb::ErrorCode::CapabilityUnsupported as i32))))
}

pub async fn survey(link: &mut Link, args: SurveyArgs, json: bool) -> Fallible {
    let env = std::env::var(WORKSPACE_ENV).ok().and_then(|v| v.parse::<Uuid>().ok());
    println!("{}", read(link, args.repo.as_deref(), env, json).await?);
    Ok(())
}

/// What `repo survey` prints.
pub(crate) async fn read<L: DispatchLink>(
    link: &mut L,
    repo: Option<&str>,
    env: Option<Uuid>,
    json: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    needs_survey(link)?;
    let repositories = crate::list_repositories(link).await?;
    let all = workspaces_on(link, None).await?;
    let repository = repository_to_create_in(&repositories, &all, repo, env)?;
    let mut call = req_for("repository.survey", repository);
    call.required_capabilities.push(capability::REPO_SURVEY.to_string());
    let answer = link.call(call).await.map_err(|e| crate::tasks::refusal(e, "the runner couldn't survey that repository"))?;
    let result::Value::RepositorySurvey(survey) = expect_value(answer.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    Ok(if json { survey_json(&survey, repository).to_string() } else { render(&survey) })
}

/// An answer's source as the JSON says it: the signal, or `default`.
///
/// Every string here came from a repository, GitHub or another runner, any
/// of which may be hostile, and goes to a terminal an agent may be reading
/// or to an app. So each passes `farcooler_core::phrase` once more, whatever
/// the runner did: one line, no escape sequence, no comment-closing dashes
/// (review 2a, H1 and L8).
fn source(a: &pb::SurveyAnswer) -> String {
    if a.signal.is_empty() { "default".to_string() } else { phrase(&a.signal) }
}

/// An answer's text, which is longer than a phrase.
fn text(a: &pb::SurveyAnswer) -> String {
    phrase_within(&a.text, TEXT_MAX)
}

/// The longest answer text kept, as the runner keeps it.
const TEXT_MAX: usize = 600;

/// `--json`: every answer with its source, and what the survey read.
pub(crate) fn survey_json(s: &pb::RepositorySurvey, repository: Uuid) -> Value {
    let h = s.history.unwrap_or_default();
    let g = s.github.unwrap_or_default();
    json!({
        "repository": repository.to_string(),
        "answers": s.answers.iter().map(|a| json!({
            "key": phrase(&a.key),
            "heading": phrase(&a.heading),
            "value": phrase(&a.value),
            "text": text(a),
            "source": source(a),
        })).collect::<Vec<_>>(),
        "languages": s.languages.iter().map(|l| json!({ "name": phrase(&l.name), "percent": l.percent })).collect::<Vec<_>>(),
        "packages": s.packages.iter().map(|p| json!({ "path": phrase(&p.path), "language": phrase(&p.language) })).collect::<Vec<_>>(),
        "history": {
            "empty": h.empty,
            "shallow": h.shallow,
            "commits": h.commits,
            "recent_commits": h.recent_commits,
            "first_parent": h.first_parent,
            "merges": h.merges,
            "prefixed_percent": h.prefixed_percent,
            "median_subject": h.median_subject,
        },
        "github": { "installed": g.installed, "answered": g.answered, "merged": g.merged, "reviewed": g.reviewed },
        "instructions": s.instructions.iter().map(|f| json!({ "path": phrase(&f.path), "lines": f.lines })).collect::<Vec<_>>(),
        "agents": s.agents.iter().map(|a| phrase(a)).collect::<Vec<_>>(),
        "charter_draft": s.charter_draft,
        "took_ms": s.took_ms,
        "read_at": s.read_at,
    })
}

/// What `repo survey` says, for a person.
pub(crate) fn render(s: &pb::RepositorySurvey) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut heading = "";
    for a in s.answers.iter().filter(|a| !a.text.is_empty()) {
        let line = format!("  {}\n  from: {}", text(a), source(a));
        if a.heading == heading {
            out.last_mut().expect("a heading before").push_str(&format!("\n{line}"));
        } else {
            heading = &a.heading;
            out.push(format!("{}\n{line}", phrase(heading.trim_start_matches("## "))));
        }
    }
    if let Some(h) = s.answers.iter().find(|a| a.key == "harness") {
        out.push(format!(
            "Run it with\n  {}\n  from: {}",
            if h.value.is_empty() { "no agent is installed".to_string() } else { phrase(&h.value) },
            source(h)
        ));
    }
    let mut read = Vec::new();
    if !s.languages.is_empty() {
        let langs: Vec<String> = s.languages.iter().map(|l| format!("{} {}%", phrase(&l.name), l.percent)).collect();
        read.push(format!("  Languages: {}", langs.join(", ")));
    }
    if !s.packages.is_empty() {
        read.push(format!("  Packages: {}", s.packages.iter().map(|p| phrase(&p.path)).collect::<Vec<_>>().join(", ")));
    }
    let h = s.history.unwrap_or_default();
    read.push(if h.empty {
        "  History: no commits yet".to_string()
    } else {
        let more = if h.commits >= 1000 { "+" } else { "" };
        let shallow = if h.shallow { ", shallow" } else { "" };
        format!("  History: {}{more} commits, {} in the last 90 days{shallow}", h.commits, h.recent_commits)
    });
    let g = s.github.unwrap_or_default();
    read.push(match (g.installed, g.answered) {
        (false, _) => "  GitHub: gh isn't installed".to_string(),
        (true, false) => "  GitHub: gh didn't answer. Sign in with `gh auth login` to include pull requests".to_string(),
        (true, true) => format!("  GitHub: {} of the last 50 pull requests merged", g.merged),
    });
    out.push(format!("What was read, in {} ms:\n{}", s.took_ms, read.join("\n")));
    out.join("\n\n")
}

/// `--lanes`: a count, or `none` for no limit (0 on the wire).
fn lanes(given: &str) -> Result<u32, String> {
    match given.trim() {
        "none" | "no-limit" => Ok(0),
        n => match n.parse::<u32>() {
            Ok(n) if (1..=99).contains(&n) => Ok(n),
            _ => Err(format!("--lanes {given:?} isn't a number from 1 to 99, or none")),
        },
    }
}

/// `workspace.write_charter` for one workspace.
pub(crate) fn write_request(workspace: Uuid, args: &WriteCharterArgs) -> Result<pb::Request, String> {
    let lanes = args.lanes.as_deref().map(lanes).transpose()?;
    let mut r = with(
        req_for("workspace.write_charter", workspace),
        request::Payload::WorkspaceWriteCharter(pb::WorkspaceWriteCharter {
            draft: args.draft,
            landing: args.landing.map(|l| l.wire() as i32),
            lanes,
            initiative: args.initiative.map(|s| s == OnOff::On),
        }),
    );
    r.required_capabilities.push(capability::REPO_SURVEY.to_string());
    Ok(r)
}

/// `workspace write-charter`, once the workspace is named.
pub(crate) async fn write_charter(link: &mut Link, ws: &pb::Workspace, args: &WriteCharterArgs, json: bool) -> Fallible {
    needs_survey(link)?;
    let r = write_request(uuid_of(&ws.id), args)?;
    let answer = link.call(r).await.map_err(|e| crate::tasks::refusal(e, "the charter could not be written"))?;
    let result::Value::CharterWritten(w) = expect_value(answer.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    if json {
        println!("{}", json!({ "workspace": uuid_of(&ws.id).to_string(), "wrote": w.wrote, "draft": w.draft, "text": w.text }));
    } else if w.wrote {
        println!("wrote {}'s {}", ws.name, if w.draft { "charter draft" } else { "charter" });
    } else {
        println!("{} already has a charter, so nothing was written", ws.name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(lanes: Option<&str>) -> WriteCharterArgs {
        WriteCharterArgs {
            workspace: "Main".into(),
            draft: true,
            landing: Some(LandingArg::PullRequests),
            lanes: lanes.map(str::to_string),
            initiative: Some(OnOff::Off),
            repo: None,
        }
    }

    #[test]
    fn write_charter_carries_each_choice_and_names_the_capability() {
        let r = write_request(Uuid::nil(), &args(Some("none"))).unwrap();
        assert_eq!(r.method, "workspace.write_charter");
        assert_eq!(r.required_capabilities, [capability::REPO_SURVEY]);
        let Some(request::Payload::WorkspaceWriteCharter(p)) = r.payload else { panic!("payload") };
        assert_eq!(p, pb::WorkspaceWriteCharter {
            draft: true,
            landing: Some(pb::LandingMode::PullRequests as i32),
            lanes: Some(0),
            initiative: Some(false),
        });
        let Some(request::Payload::WorkspaceWriteCharter(p)) = write_request(Uuid::nil(), &args(Some("4"))).unwrap().payload
        else {
            panic!("payload")
        };
        assert_eq!(p.lanes, Some(4));
        for bad in ["0", "100", "two", ""] {
            assert!(write_request(Uuid::nil(), &args(Some(bad))).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_answer_with_no_signal_says_default() {
        let s = pb::RepositorySurvey {
            answers: vec![
                pb::SurveyAnswer { key: "review".into(), heading: "## Review".into(), text: "x".into(), signal: "CODEOWNERS".into(), ..Default::default() },
                pb::SurveyAnswer { key: "lanes".into(), heading: "## Lanes".into(), value: "2".into(), text: "y".into(), ..Default::default() },
                pb::SurveyAnswer { key: "more".into(), heading: "## Lanes".into(), text: "z".into(), ..Default::default() },
            ],
            ..Default::default()
        };
        let v = survey_json(&s, Uuid::nil());
        assert_eq!(v["answers"][0]["source"], "CODEOWNERS");
        assert_eq!(v["answers"][1]["source"], "default");
        assert_eq!(v["answers"][1]["value"], "2");
        let text = render(&s);
        assert!(text.contains("Review\n  x\n  from: CODEOWNERS"), "{text}");
        assert!(text.contains("Lanes\n  y\n  from: default\n  z\n  from: default\n\n"), "one heading, both answers: {text}");
    }

    /// A runner's strings reach the terminal as plain single lines, whatever
    /// the runner sent (review 2a, L8).
    #[test]
    fn repository_strings_print_without_escapes_or_newlines() {
        let s = pb::RepositorySurvey {
            answers: vec![pb::SurveyAnswer {
                key: "review".into(),
                heading: "## Review".into(),
                text: "ok\n## Lanes\nmore".into(),
                signal: "\u{1b}]0;owned\u{7}x\n--->".into(),
                ..Default::default()
            }],
            packages: vec![pb::SurveyPackage { path: "apps/\u{1b}[2Jweb".into(), language: String::new() }],
            ..Default::default()
        };
        let text = render(&s);
        assert!(!text.contains('\u{1b}') && !text.contains('\u{7}'), "{text:?}");
        assert!(text.contains("Review\n  ok ## Lanes more\n  from: x ->\n"), "{text:?}");
        let v = survey_json(&s, Uuid::nil()).to_string();
        assert!(!v.contains("\\u001b") && !v.contains("\\n"), "{v}");
    }
}
