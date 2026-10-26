//! `repository.survey` and `workspace.write_charter` (ov-519): the survey
//! read on demand for the first run's Reading and Charter screens, and the
//! charter written from it. Nothing is kept: each call reads afresh, so the
//! charter written is the one the repository says now, with the owner's
//! choices on top.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, request, result};
use farcooler_store::models::Repository;

use crate::charter_draft;
use crate::service::Service;
use crate::survey::{self, Answer, Choices, Gh, Signals, Source};
use crate::survey_read::{self, RunnerGh};

/// Every signal for `repo`, with the runner's own `gh` when it has one.
///
/// One deadline for all of it, set first: looking for the agents and for gh
/// (which can ask the login shell) spends the same budget as the reads.
pub async fn signals(svc: &Service, repo: &Repository) -> Signals {
    let limits = survey_read::LIMITS;
    let deadline = tokio::time::Instant::now() + limits.budget;
    let tree = svc.repository_worktree(repo);
    // On a blocking thread: finding a program can ask the login shell.
    let found = tokio::task::spawn_blocking(|| {
        let stand_in = crate::service::stand_in_agent().is_some() || crate::service::test_stub_agents();
        (crate::wire::agents_found(stand_in, farcooler_core::programs::find), crate::git::gh_launch().is_some())
    });
    let (agents, has_gh) = tokio::time::timeout_at(deadline, found).await.ok().and_then(|r| r.ok()).unwrap_or_default();
    let gh = has_gh.then(|| RunnerGh { svc, tree: tree.clone() });
    survey_read::gather_until(&tree, gh.as_ref(), agents, deadline, limits).await
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn wire_answer(a: &Answer) -> pb::SurveyAnswer {
    pb::SurveyAnswer {
        key: a.key.to_string(),
        heading: a.heading.to_string(),
        value: a.value.clone(),
        text: a.text.clone(),
        signal: match &a.source {
            Source::Signal(why) => why.clone(),
            Source::Default | Source::Chosen => String::new(),
        },
    }
}

/// A survey as the wire carries it, its charter dated `date`.
pub fn to_wire(s: &Signals, date: &str, took_ms: u32, read_at: i64) -> pb::RepositorySurvey {
    let answers = survey::answers(s, &Choices::default());
    let (prefixed, median) = survey::commit_style(&s.subjects);
    let merged = s.merged.clone().unwrap_or_default();
    pb::RepositorySurvey {
        answers: answers.iter().map(wire_answer).collect(),
        languages: survey::languages(&s.files)
            .into_iter()
            .map(|(name, percent)| pb::SurveyLanguage { name, percent })
            .collect(),
        packages: survey::packages(&s.files)
            .into_iter()
            .map(|(path, language)| pb::SurveyPackage { path, language })
            .collect(),
        history: Some(pb::SurveyHistory {
            empty: s.empty,
            shallow: s.shallow,
            commits: s.commits,
            recent_commits: s.subjects.len() as u32,
            first_parent: s.parents.len() as u32,
            merges: s.parents.iter().filter(|p| **p > 1).count() as u32,
            prefixed_percent: prefixed,
            median_subject: median,
        }),
        github: Some(pb::SurveyGitHub {
            installed: s.gh != Gh::Missing,
            answered: s.gh == Gh::Answered,
            merged: merged.count,
            reviewed: merged.reviewed,
        }),
        instructions: survey::instructions(s).into_iter().map(|(path, lines)| pb::SurveyFile { path, lines }).collect(),
        agents: s.agents.clone(),
        charter_draft: charter_draft::render(&answers, date),
        took_ms,
        read_at,
    }
}

/// `repository.survey`: read the repository now.
pub async fn survey(svc: &Service, req: pb::Request) -> Result<result::Value> {
    let id = req.target_resource_id.as_deref().and_then(crate::wire::parse_id).ok_or(DomainError::NotFound)?;
    let repo = svc.store.get_repository(id)?;
    let started = std::time::Instant::now();
    let s = signals(svc, &repo).await;
    let took = started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
    Ok(result::Value::RepositorySurvey(to_wire(&s, &charter_draft::today(), took, now_ms())))
}

/// The owner's choices, as the request carries them.
pub fn choices(p: &pb::WorkspaceWriteCharter) -> Result<Choices> {
    let landing = match p.landing {
        None => None,
        Some(raw) => Some(crate::landing_read::from_wire_mode(raw).ok_or(DomainError::InvalidArgument { what: "landing" })?),
    };
    Ok(Choices { landing, lanes: p.lanes, initiative: p.initiative })
}

/// `workspace.write_charter`: survey the workspace's repository again, take
/// the owner's choices, and write the charter or its draft.
pub async fn write_charter(svc: &Service, req: pb::Request) -> Result<result::Value> {
    let id = req.target_resource_id.as_deref().and_then(crate::wire::parse_id).ok_or(DomainError::NotFound)?;
    let Some(request::Payload::WorkspaceWriteCharter(p)) = req.payload else {
        return Err(DomainError::InvalidArgument { what: "payload" });
    };
    let choices = choices(&p)?;
    if choices.lanes.is_some_and(|n| n > 99) {
        return Err(DomainError::InvalidArgument { what: "lanes" });
    }
    let workspace = svc.store.get_workspace(id)?;
    let repo = svc.store.get_repository(workspace.repository_id)?;
    // Never beside an edit (`charter_write`): one writer at a time.
    let _one = crate::charter_write::EDITS.lock().await;
    // Main adopts a repository's `.farcooler/manager.md` here first, so a
    // charter the repository carries is never written over.
    let home = svc.ensure_workspace_home(&workspace).map_err(|e| {
        tracing::warn!(workspace = %id, error = %e, "could not make the workspace's home");
        DomainError::OperationFailed
    })?;
    let s = signals(svc, &repo).await;
    let text = charter_draft::render(&survey::answers(&s, &choices), &charter_draft::today());
    let written = charter_draft::write(&home, &text, p.draft).map_err(|e| {
        tracing::warn!(workspace = %id, error = %e, "could not write the charter");
        DomainError::OperationFailed
    })?;
    Ok(result::Value::CharterWritten(pb::CharterWritten { wrote: written.wrote, draft: p.draft, text }))
}
