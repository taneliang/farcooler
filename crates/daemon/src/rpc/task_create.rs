use farcooler_core::{DomainError, Result};
use farcooler_protocol::v1::{Request, request, result};

use crate::service::Service;
use crate::watch::Watcher;

pub(super) async fn dispatch(svc: &Service, watcher: &Watcher, req: Request) -> Result<result::Value> {
    let Some(request::Payload::TaskCreate(create)) = req.payload else {
        return Err(DomainError::InvalidArgument { what: "payload" });
    };
    let task = match &create.theme_id {
        Some(theme) => crate::rpc_plan::create_themed_task(svc, watcher, &create, theme)?,
        None => crate::task_ops::create(svc, watcher, &create)?,
    };
    // A new card is an engine item at once, in its column, with no run (ov-951).
    if let Ok(id) = crate::task_ops::required_id(&task.id) {
        svc.track_tickets(Some(id)).await;
    }
    Ok(result::Value::Task(task))
}
