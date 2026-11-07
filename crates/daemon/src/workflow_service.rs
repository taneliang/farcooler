//! One retained owner, started by ordinary exclusive bootstrap, joined on stop.
use crate::{service::Service, workflow_runtime::{Runtime, CommandHandle, Error, Result}};
use tokio::{sync::watch, task::JoinHandle};

pub(crate) struct Owner {
    handle: CommandHandle,
    stop: watch::Sender<bool>,
    task: JoinHandle<Result<()>>,
    notices: JoinHandle<Result<()>>,
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
}
