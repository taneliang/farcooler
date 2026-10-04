//! The board's routes: `task.*` but `task.move`, a workspace write, and
//! `workspace.mark_read`, whose read state is the board's.
//!
//! Out of `rpc.rs` so the dispatch table there stays inside its size budget.
//! `Rpc::dispatch` names every route here in one arm, so its own coverage
//! test (`every_method_is_dispatched_and_every_dispatched_route_is_a_method`)
//! still reads each name out of that file; this match is the other half.

use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{Request, request, result};

use crate::service::Service;
use crate::watch::Watcher;

/// One board route, as `Rpc::dispatch` hands it over.
pub(crate) async fn dispatch(svc: &Service, watcher: &Watcher, req: Request) -> Result<result::Value> {
    match req.method.as_str() {
        "task.list" => {
            let Some(request::Payload::TaskList(p)) = req.payload else {
                return Err(DomainError::InvalidArgument { what: "payload" });
            };
            Ok(result::Value::TaskList(crate::task_ops::list(svc, &p)?))
        }

        "task.get" => {
            let Some(request::Payload::TaskGet(p)) = req.payload else {
                return Err(DomainError::InvalidArgument { what: "payload" });
            };
            Ok(result::Value::TaskDetail(crate::task_ops::get(svc, &p)?))
        }

        "task.get_by_key" => {
            let Some(request::Payload::TaskGetByKey(p)) = req.payload else {
                return Err(DomainError::InvalidArgument { what: "payload" });
            };
            Ok(result::Value::TaskList(crate::task_ops::get_by_key(svc, &p)?))
        }

        "task.search" => {
            let Some(request::Payload::TaskSearch(p)) = req.payload else {
                return Err(DomainError::InvalidArgument { what: "payload" });
            };
            Ok(result::Value::TaskNoteHitList(crate::task_ops::search(svc, &p)?))
        }

        "task.create" => {
            let Some(request::Payload::TaskCreate(p)) = req.payload else {
                return Err(DomainError::InvalidArgument { what: "payload" });
            };
            Ok(result::Value::Task(crate::task_ops::create(svc, watcher, &p)?))
        }

        "task.update" => {
            let Some(request::Payload::TaskUpdate(p)) = req.payload else {
                return Err(DomainError::InvalidArgument { what: "payload" });
            };
            Ok(result::Value::Task(crate::task_ops::update(svc, watcher, &p)?))
        }

        "task.set_status" => {
            let Some(request::Payload::TaskSetStatus(p)) = req.payload else {
                return Err(DomainError::InvalidArgument { what: "payload" });
            };
            Ok(result::Value::Task(crate::task_ops::set_status(svc, watcher, &p)?))
        }

        "task.note" => {
            let Some(request::Payload::TaskNoteAppend(p)) = req.payload else {
                return Err(DomainError::InvalidArgument { what: "payload" });
            };
            Ok(result::Value::TaskNote(crate::task_ops::note(svc, watcher, &p)?))
        }

        "task.block" => {
            let Some(request::Payload::TaskBlockSet(p)) = req.payload else {
                return Err(DomainError::InvalidArgument { what: "payload" });
            };
            Ok(result::Value::TaskBlockList(crate::task_ops::block(
                svc,
                watcher,
                &p,
            )?))
        }

        "task.set_wait" => {
            let Some(request::Payload::TaskSetWait(p)) = req.payload else {
                return Err(DomainError::InvalidArgument { what: "payload" });
            };
            Ok(result::Value::Task(crate::task_starts::set_wait(svc, watcher, &p)?))
        }

        "task.set_line" => {
            let Some(request::Payload::TaskSetLine(p)) = req.payload else {
                return Err(DomainError::InvalidArgument { what: "payload" });
            };
            Ok(result::Value::TaskList(crate::task_starts::set_line(svc, watcher, &p)?))
        }

        "task.worker" => {
            let Some(request::Payload::TaskWorkerSet(p)) = req.payload else {
                return Err(DomainError::InvalidArgument { what: "payload" });
            };
            let pane = crate::task_starts::orchestrator_pane(svc, &p).await;
            Ok(result::Value::Task(crate::task_starts::worker(svc, watcher, &p, pane)?))
        }
        "workspace.mark_read" => {
            let Some(request::Payload::WorkspaceMarkRead(p)) = req.payload else {
                return Err(DomainError::InvalidArgument { what: "payload" });
            };
            Ok(result::Value::BoardReads(crate::board_reads_ops::mark_read(svc, watcher, &p)?))
        }
        other => {
            tracing::error!(method = %other, "a board route with no handler");
            Err(DomainError::NotFound)
        }
    }
}

#[cfg(test)]
mod tests {
    use farcooler_protocol::method::Method;

    /// Every board route `Rpc::dispatch` hands here has an arm here: a route
    /// named there and missing here would answer NOT_FOUND on every runner,
    /// and the compiler can't see it.
    #[test]
    fn every_board_route_has_an_arm() {
        let source = include_str!("rpc_board.rs");
        let routes = Method::ALL
            .iter()
            .map(|m| m.name())
            .filter(|m| (m.starts_with("task.") && *m != "task.move") || *m == "workspace.mark_read");
        let mut seen = 0;
        for route in routes {
            assert!(source.contains(&format!("\"{route}\" => {{")), "no arm for {route}");
            seen += 1;
        }
        assert_eq!(seen, 13, "the board's routes");
    }
}
