//! One retained owner, started by ordinary exclusive bootstrap, joined on stop.
use crate::{service::Service, workflow_runtime::{Runtime, CommandHandle, Error, Result}};
use tokio::{sync::watch, task::JoinHandle};

pub(crate) struct Owner {
    handle: CommandHandle,
    stop: watch::Sender<bool>,
    task: JoinHandle<Result<()>>,
}
impl Service {
    pub fn start_workflows(&self) -> Result<()> {
        let mut owner = self.workflow_owner.lock().map_err(|_| Error::Refused("workflow owner lock poisoned"))?;
        if owner.is_some() { return Err(Error::Refused("workflow service already started")); }
        let mut runtime = Runtime::open_service(&self.root_dir().join("workflow"), self.store.clone())?;
        let handle = runtime.command_handle(&runtime.controller())?;
        let (stop, signal) = watch::channel(false);
        let task = tokio::spawn(async move { runtime.run(signal).await });
        *owner = Some(Owner { handle,stop,task });
        Ok(())
    }
    pub(crate) fn workflow_handle(&self) -> Option<CommandHandle> {
        self.workflow_owner.lock().ok()?.as_ref().map(|o| o.handle.clone())
    }
    pub async fn stop_workflows(&self) -> Result<()> {
        let owner = self.workflow_owner.lock().map_err(|_| Error::Refused("workflow owner lock poisoned"))?.take();
        if let Some(owner) = owner {
            owner.stop.send_replace(true);
            owner.task.await.map_err(|_| Error::Refused("workflow owner failed to join"))??;
        }
        Ok(())
    }
}
