//! One retained owner, started by ordinary exclusive bootstrap, joined on stop.
use crate::{service::Service, workflow_runtime::{Runtime, CommandHandle, Error, Result}};
use tokio::{sync::watch, task::JoinHandle};

pub(crate) struct Owner {
    handle: CommandHandle,
    stop: watch::Sender<bool>,
    task: JoinHandle<Result<()>>,
    notices: JoinHandle<Result<()>>,
}
/// The engine column a card's status puts it in when it is first tracked.
pub(crate) fn column_of(s: farcooler_store::models::TaskStatus) -> &'static str {
    use farcooler_store::models::TaskStatus as S;
    match s { S::Backlog => "backlog", S::Todo => "todo", S::InProgress => "doing", S::InReview => "review",
        S::NeedsDecision => "decision", S::Done | S::Cancelled => "backlog" }
}
impl Service {
    pub fn start_workflows(&self) -> Result<()> {
        let mut owner = self.workflow_owner.lock().map_err(|_| Error::Refused("workflow owner lock poisoned"))?;
        if let Some(current) = owner.as_ref() {
            return Err(Error::Refused(if current.task.is_finished() || current.handle.is_closed() {
                "workflow service stopped; join failed owner before restart"
            } else { "workflow service already started" }));
        }
        let mut runtime = Runtime::open_service(&self.root_dir().join("workflow"), self.store.clone())?;
        let wake=self.workflow_notice_wake.clone();
        self.store.set_workflow_notice_waker(Some(std::sync::Arc::new(move||wake.notify_one())));
        runtime.notice_wake = Some(self.workflow_notice_wake.clone());
        let handle = runtime.command_handle(&runtime.controller())?;
        let (stop, signal) = watch::channel(false);
        let notices = tokio::spawn(crate::workflow_notices::run(self.store.clone(),self.inventory.clone(),self.workflow_notice_wake.clone(),signal.clone()));
        let task = tokio::spawn(async move { runtime.run(signal).await });
        *owner = Some(Owner { handle,stop,task,notices });
        Ok(())
    }
    /// Bring every open card no run owns under the engine (ov-951): all of them at start, or one just made.
    pub async fn track_tickets(&self, only: Option<uuid::Uuid>) {
        let Some(handle) = self.workflow_handle() else { return };
        let Ok(found) = self.store.untracked_tickets(only) else { return };
        let tickets: Vec<_> = found.into_iter().map(|(w, k, s)| (w.to_string(), k, column_of(s).to_string())).collect();
        if tickets.is_empty() { return; }
        if let Err(error) = handle.track(tickets).await { tracing::warn!(%error, "could not track cards"); }
    }
    pub(crate) fn workflow_handle(&self) -> Option<CommandHandle> {
        self.workflow_owner.lock().ok()?.as_ref()
            .filter(|o| !o.task.is_finished() && !o.handle.is_closed()).map(|o| o.handle.clone())
    }
    pub async fn stop_workflows(&self) -> Result<()> {
        let owner = self.workflow_owner.lock().map_err(|_| Error::Refused("workflow owner lock poisoned"))?.take();
        if let Some(owner) = owner {
            owner.stop.send_replace(true);
            let runtime = owner.task.await;
            let notices = owner.notices.await;
            runtime.map_err(|_| Error::Refused("workflow owner failed to join"))??;
            notices.map_err(|_| Error::Refused("workflow notice owner failed to join"))??;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn workflow_service_availability_tracks_failed_owner_and_join_before_restart() {
        let dir = tempfile::tempdir().unwrap();
        let service = Service::open_in(dir.path().join("state")).await.unwrap();
        service.start_workflows().unwrap();
        let retained = service.workflow_handle().unwrap();
        service.workflow_owner.lock().unwrap().as_ref().unwrap().task.abort();
        tokio::task::yield_now().await;
        assert!(service.workflow_handle().is_none());
        assert!(retained.state("any").await.is_err());
        assert!(matches!(service.start_workflows(),Err(Error::Refused("workflow service stopped; join failed owner before restart"))));
        assert!(service.stop_workflows().await.is_err(),"joining must report failure");
        service.start_workflows().unwrap();
        assert!(service.workflow_handle().unwrap().state("any").await.unwrap().is_none());
        service.stop_workflows().await.unwrap();
        assert!(service.workflow_handle().is_none());
    }
    #[tokio::test]
    async fn every_open_card_becomes_an_engine_item_in_its_own_column_and_a_new_one_at_once() {
        use farcooler_store::models::{Actor, TaskStatus};
        let (_dir, service, repo) = crate::test_support::fixture().await;
        let board = service.store.ensure_main_workspace(repo).unwrap();
        let backlog = service.store.create_task(board.id, "plain", Actor::User).unwrap();
        let doing = service.store.create_task(board.id, "under way", Actor::User).unwrap();
        service.store.set_task_status(doing.id, TaskStatus::InProgress, Actor::User).unwrap();
        // A board with nothing published keeps its cards as they were.
        service.start_workflows().unwrap();
        service.track_tickets(None).await;
        assert!(service.store.untracked_tickets(None).unwrap().is_empty(), "no publication, nothing to track");
        assert!(service.store.set_task_status(backlog.id, TaskStatus::Todo, Actor::User).is_ok(), "an untracked card takes a hand move");
        service.store.set_task_status(backlog.id, TaskStatus::Backlog, Actor::User).unwrap();
        service.stop_workflows().await.unwrap();
        crate::test_support::publish_for_test(&service, board.id);
        let closed = service.store.create_task(board.id, "finished", Actor::User).unwrap();
        service.store.set_task_status(closed.id, TaskStatus::Done, Actor::User).unwrap();
        service.start_workflows().unwrap();
        service.track_tickets(None).await;
        let state = service.workflow_handle().unwrap().state(&board.id.to_string()).await.unwrap().unwrap();
        let column = |key: &str| { let i = &state.items[key]; i.definition.stages[i.stage].id.clone() };
        assert_eq!(column(&backlog.key), "backlog");
        assert_eq!(column(&doing.key), "doing", "a card keeps the column it is in");
        assert!(!state.items.contains_key(&closed.key), "a finished card is no run");
        assert_eq!(service.store.get_task(doing.id).unwrap().status, TaskStatus::InProgress, "tracking moves nothing");
        assert!(service.store.untracked_tickets(None).unwrap().is_empty());
        // A card made later is tracked when it is made, and a hand move is then refused.
        let fresh = service.store.create_task(board.id, "later", Actor::User).unwrap();
        service.track_tickets(Some(fresh.id)).await;
        assert!(service.store.set_task_status(fresh.id, TaskStatus::Todo, Actor::User).is_err());
        service.stop_workflows().await.unwrap();
    }
}
