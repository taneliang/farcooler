//! `report.get` on the wire: the request read, the report computed, and the
//! answer written as JSON.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1 as pb;
use farcooler_store::Store;
use uuid::Uuid;

use super::{Narrowing, Period, compute, gather};

/// The report `req` asks for, as of `now`.
///
/// An empty or backward period, or a request naming both a repository and
/// a workspace, is refused rather than guessed at. A repository or
/// workspace that isn't on this runner is `NotFound`.
pub fn serve(store: &Store, req: &pb::ReportRequest, now: i64) -> Result<pb::Report> {
    if req.until <= req.since {
        return Err(DomainError::InvalidArgument { what: "until" });
    }
    let narrowing = match (id(req.repository_id.as_deref(), "repository_id")?, id(req.workspace_id.as_deref(), "workspace_id")?) {
        (None, None) => Narrowing::Runner,
        (Some(repository), None) => Narrowing::Repository(repository),
        (None, Some(workspace)) => Narrowing::Workspace(workspace),
        (Some(_), Some(_)) => return Err(DomainError::InvalidArgument { what: "workspace_id" }),
    };
    let period = Period { since: req.since, until: req.until };
    let report = compute(&gather(store, narrowing, period, req.utc_offset_minutes)?, period, now);
    let report_json = serde_json::to_string(&report).map_err(|_| DomainError::OperationFailed)?;
    Ok(pb::Report { report_json })
}

fn id(bytes: Option<&[u8]>, what: &'static str) -> Result<Option<Uuid>> {
    bytes.map(|raw| crate::wire::parse_id(raw).ok_or(DomainError::InvalidArgument { what })).transpose()
}
