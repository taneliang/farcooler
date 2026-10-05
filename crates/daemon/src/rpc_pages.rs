//! The routes for orchestrator pages (ov-269): `page.*`, behind `board_pages`.
//!
//! Experimental, and beside the board: nothing here touches `task_ops` or the
//! plan layer's writes, and nothing in either calls in. Each write that
//! changes a page announces `pages_changed` once; identical bytes announce
//! nothing, because the store reports them as unchanged.
//!
//! # What the runner checks that the CLI can't
//!
//! The document is validated again here (`farcooler_core::page_doc`), and this
//! is the authority. Then its references are read against the board: a card
//! key must be on this workspace's board, and a lane or a theme must be in its
//! plan, so a typo fails when it's made and not when the owner taps it. The
//! lane and theme check is the one place this file reads the plan layer, and
//! it degrades: with the plan layer removed there is nothing to check against,
//! and the references are accepted and drawn as plain text by every client.
//!
//! Not announced in bursts: the store refuses more than thirty changing writes
//! to one slot an hour and more than 120 changes and removals to a board's
//! pages an hour, whatever the slots are called, so the events are bounded at
//! 120 an hour a board without a debounce.

use std::collections::HashSet;

use farcooler_core::page_doc::{self, Caps, Page, Target};
use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{self as pb, Request, request, result};
use farcooler_store::pages::{Anchor, PageWrite, SetOutcome, SlotStats, StoredPage};
use uuid::Uuid;

use crate::service::Service;
use crate::task_ops::{actor_from_wire, required_id};
use crate::watch::Watcher;
use crate::wire::id_bytes;

fn payload_missing() -> DomainError {
    DomainError::InvalidArgument { what: "payload" }
}

fn refused(said: impl Into<String>) -> DomainError {
    DomainError::PageRefused { said: said.into() }
}

/// One page route, as `Rpc::dispatch` hands it over.
pub(crate) async fn dispatch(svc: &Service, watcher: &Watcher, req: Request) -> Result<result::Value> {
    match req.method.as_str() {
        "page.list" => {
            let Some(request::Payload::PageList(p)) = req.payload else { return Err(payload_missing()) };
            let workspace = required_id(&p.workspace_id)?;
            let pages = svc.store.list_pages(workspace)?;
            Ok(result::Value::BoardPageList(pb::BoardPageList {
                pages: pages.iter().map(|page| pb_page(page, p.with_docs)).collect(),
            }))
        }
        "page.get" => {
            let Some(request::Payload::PageGet(p)) = req.payload else { return Err(payload_missing()) };
            let workspace = required_id(&p.workspace_id)?;
            Ok(result::Value::BoardPage(pb_page(&svc.store.get_page(workspace, &p.slot)?, true)))
        }
        "page.set" => {
            let Some(request::Payload::PageSet(p)) = req.payload else { return Err(payload_missing()) };
            let workspace = required_id(&p.workspace_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let page = page_doc::parse(&p.doc_json, &Caps::default()).map_err(|e| refused(e.to_string()))?;
            svc.store.get_workspace(workspace)?;
            check_references(svc, workspace, &page)?;
            let anchor = match &p.anchor_theme_id {
                None => Anchor::Keep,
                Some(id) if id.is_empty() => Anchor::Clear,
                Some(id) => Anchor::Theme(check_theme(svc, workspace, required_id(id)?)?.to_string()),
            };
            let write = PageWrite { slot: &p.slot, page: &page, anchor, ordinal: p.ordinal, if_revision: p.if_revision };
            let outcome = svc.store.set_page(workspace, &write, actor)?;
            let changed = matches!(outcome, SetOutcome::Written(_));
            let stored = outcome.page();
            if changed {
                watcher.announce_pages_changed(workspace, &stored.slot, stored.revision, actor, false);
                // A CI reference (ov-306) is read now, not at the watch's next turn.
                if page.references().iter().any(|at| at.reference.target.ci_subject().is_some()) {
                    crate::ci_watch::kick();
                }
            }
            Ok(result::Value::PageSetResult(pb::PageSetResult { page: Some(pb_page(stored, true)), changed }))
        }
        "page.remove" => {
            let Some(request::Payload::PageRemove(p)) = req.payload else { return Err(payload_missing()) };
            let workspace = required_id(&p.workspace_id)?;
            let actor = actor_from_wire(&p.actor)?;
            let gone = svc.store.remove_page(workspace, &p.slot, actor)?;
            watcher.announce_pages_changed(workspace, &gone.slot, gone.revision, actor, true);
            Ok(result::Value::BoardPage(pb_page(&gone, true)))
        }
        "page.stats" => {
            let Some(request::Payload::PageStats(p)) = req.payload else { return Err(payload_missing()) };
            let workspace = required_id(&p.workspace_id)?;
            let stats = svc.store.page_stats(workspace, p.since_ms)?;
            Ok(result::Value::PageStatsList(pb::PageStatsList { slots: stats.iter().map(pb_stats).collect() }))
        }
        _ => Err(DomainError::NotFound),
    }
}

/// What the plan layer knows about a board's names, or `None` when there is no
/// plan layer to ask. The one place pages read it.
struct PlanNames {
    lanes: HashSet<String>,
    themes: Vec<(Uuid, String)>,
}

fn plan_names(svc: &Service, workspace: Uuid) -> Option<PlanNames> {
    let plan = svc.store.plan(workspace, i64::MIN).ok()?;
    Some(PlanNames {
        lanes: plan.lanes.iter().map(|l| l.lane.name.to_lowercase()).collect(),
        themes: plan
            .themes
            .iter()
            .filter(|t| t.theme.state != farcooler_store::plan::ThemeState::Dropped)
            .map(|t| (t.theme.id, t.theme.name.to_lowercase()))
            .collect(),
    })
}

/// A theme a page may be anchored to: live, on this board, and there to ask.
fn check_theme(svc: &Service, workspace: Uuid, theme: Uuid) -> Result<Uuid> {
    let Some(names) = plan_names(svc, workspace) else {
        return Err(refused("This runner has no plan, so a page can't be drawn in a theme. Publish it without one."));
    };
    if names.themes.iter().any(|(id, _)| *id == theme) {
        Ok(theme)
    } else {
        Err(refused("There's no theme with that id on this board."))
    }
}

/// A name from the page, cut short enough to sit in a sentence.
fn shown(name: &str) -> String {
    let mut chars = name.chars();
    let head: String = chars.by_ref().take(40).collect();
    if chars.next().is_some() { format!("{head}...") } else { head }
}

/// Every card, lane and theme a page names is on this board.
///
/// Page, worktree, terminal and link references aren't checked: a page may
/// name another that isn't published yet, and a worktree or a terminal comes
/// and goes with the work. An app draws one it can't find as plain text.
fn check_references(svc: &Service, workspace: Uuid, page: &Page) -> Result<()> {
    let references = page.references();
    if references.is_empty() {
        return Ok(());
    }
    let cards = svc.store.page_card_keys(workspace)?;
    let plan = plan_names(svc, workspace);
    for at in references {
        let missing = |what: &str, name: &str| refused(format!("{}: there's no {what} {} on this board.", at.path, shown(name)));
        match &at.reference.target {
            Target::Task(key) | Target::Ask(key) if !cards.contains(&key.to_ascii_lowercase()) => {
                return Err(missing("card", key));
            }
            Target::Lane(name) if plan.as_ref().is_some_and(|p| !p.lanes.contains(&name.to_lowercase())) => {
                return Err(missing("lane", name));
            }
            Target::Theme(name) if plan.as_ref().is_some_and(|p| !p.themes.iter().any(|(_, n)| *n == name.to_lowercase())) => {
                return Err(missing("theme", name));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Every board's CI subjects its pages' references name (ov-306), for the CI
/// watch (`ci_watch.rs`) to read while they're named.
pub(crate) fn ci_subjects(svc: &Service) -> Vec<(Uuid, String)> {
    let mut out = Vec::new();
    for ws in svc.store.list_workspaces(None).unwrap_or_default() {
        for page in svc.store.list_pages(ws.id).unwrap_or_default() {
            let Ok(doc) = page_doc::parse(&page.doc_json, &Caps::default()) else { continue };
            out.extend(doc.references().iter().filter_map(|at| at.reference.target.ci_subject()).map(|s| (ws.id, s)));
        }
    }
    out
}

fn pb_page(p: &StoredPage, with_doc: bool) -> pb::BoardPage {
    pb::BoardPage {
        id: id_bytes(p.id),
        slot: p.slot.clone(),
        title: p.title.clone(),
        summary: p.summary.clone(),
        anchor_kind: p.anchor_kind.clone(),
        anchor: p.anchor.clone(),
        doc_json: if with_doc { p.doc_json.clone() } else { String::new() },
        revision: p.revision,
        ordinal: p.ordinal,
        actor: p.actor.clone(),
        updated_at_ms: p.updated_at,
    }
}

fn pb_stats(s: &SlotStats) -> pb::PageSlotStats {
    pb::PageSlotStats {
        slot: s.slot.clone(),
        sets: s.sets,
        removes: s.removes,
        first_at_ms: s.first_at,
        last_at_ms: s.last_at,
        shape: s.shape.iter().map(|(kind, count)| pb::PageShapeCount { kind: kind.clone(), count: *count }).collect(),
    }
}
