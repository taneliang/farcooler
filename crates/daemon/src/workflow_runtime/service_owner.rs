//! Ordinary daemon ownership and immutable per-item adapter resolution.
use super::*;
use super::coding::PinnedDefinition;
use farcooler_protocol::workflow_config::AdapterConfiguration;

impl Runtime {
    pub(super) async fn validate_wire_publication(&self, workspace: &str, principal: &str,
        action: &farcooler_protocol::workflow_commands::Action) -> Result<()> {
        use farcooler_protocol::workflow_commands::Action;
        let Action::Publish { mutation, definition, adapter: Some(adapter), .. } = action else { return Ok(()) };
        let AdapterConfiguration::Coding(config) = adapter.as_ref();
        let payload = serde_json::to_string(&(principal, action))?;
        let id = format!("wire:{principal}:{}", mutation.id);
        if self.store.workflow_wire_receipt(workspace, &id, &payload)?.is_some() { return Ok(()); }
        let state = self.state(workspace)?.unwrap_or_else(|| State::empty(workspace.into(), 1));
        if state.epoch != mutation.epoch || state.revision != mutation.revision { return Ok(()); }
        self.validate_workspace_repository(workspace, config)?;
        config.validate_definition(definition)?;
        self.validate_native_configuration(config)?;
        let check = async {
            adapters::git(&config.repository, &["cat-file", "-e", &format!("{}^{{commit}}", config.base)]).await?;
            if adapters::git(&config.repository, &["rev-parse", &config.landing_ref]).await? != config.base {
                return Err(Error::Refused("publication landing ref differs from selected base"));
            }
            if let Some(landing) = &config.landing {
                if adapters::git(&landing.repository, &["rev-parse", &landing.target]).await? != config.base {
                    return Err(Error::Refused("publication remote ref differs from selected base"));
                }
            }
            Ok(())
        };
        tokio::time::timeout(Duration::from_secs(5), check).await
            .map_err(|_| Error::Refused("publication identity check timed out"))?
    }

    /// Called only by the exclusive daemon bootstrap; secondary Service opens
    /// retain no writer. The workflow and legacy ownership transactions share DB.
    pub(crate) fn open_service(root: &Path, store: std::sync::Arc<Store>) -> Result<Self> {
        if root.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) { return Err(Error::Refused("aliased service runtime root")); }
        std::fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
        let receipts = root.join("receipts");
        if !receipts.try_exists()? { std::fs::create_dir(&receipts)?; }
        if receipts.canonicalize()? != receipts { return Err(Error::Refused("aliased receipts")); }
        let writer = OpenOptions::new().read(true).write(true).create(true).truncate(false)
            .mode(0o600).custom_flags(libc::O_NOFOLLOW).open(root.join("writer.lock"))?;
        if !writer.metadata()?.is_file() || writer.metadata()?.nlink() != 1 { return Err(Error::Refused("aliased writer")); }
        rustix::fs::flock(&writer, rustix::fs::FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| Error::Refused("workflow service already owned"))?;
        let (send,inbox) = mpsc::channel(32);
        Ok(Self { exec:Exec { root,store,service_mode:true,resource:String::new(),base:"0".repeat(40),coding:None,native_slot:Default::default(),launched:None },
            notice_wake:None,configuration_locks:Default::default(),shutdown:None,
            instance:Uuid::now_v7(),
            last_occupancy:None,inbox,send,next_lease:None,effect_cap:DEFAULT_EFFECT_CAP,continuous:false,
            #[cfg(test)] sweeps:0, #[cfg(test)] fail_observe:Default::default(), #[cfg(test)] drained:0,
            _writer:writer,_repository_writer:None,_landing_writer:None })
    }
    fn validate_workspace_repository(&self, workspace: &str, config: &CodingConfig) -> Result<()> {
        if !self.service_mode { return Ok(()); }
        let workspace = Uuid::parse_str(workspace).map_err(|_| Error::Refused("invalid workspace"))?;
        let board = self.store.get_workspace(workspace).map_err(|_| Error::Refused("unknown workspace"))?;
        let repository = self.store.get_repository(board.repository_id).map_err(|_| Error::Refused("unknown repository"))?;
        if Path::new(&repository.canonical_git_dir).canonicalize()? != config.repository.join(".git") {
            return Err(Error::Refused("configuration belongs to another repository"));
        }
        Ok(())
    }
    pub(super) fn publication_resource(&self, workspace: &str, adapter: Option<&AdapterConfiguration>) -> String {
        match adapter {
            Some(AdapterConfiguration::Coding(config)) => digest(&("configured-repository/v1", &config.repository)),
            None if self.service_mode => digest(&("workflow-tracker/v1", workspace)),
            None => self.resource.clone(),
        }
    }
    pub(super) fn validate_publication(&self, definition: &Definition, adapter: Option<&AdapterConfiguration>) -> Result<()> {
        match adapter {
            Some(AdapterConfiguration::Coding(config)) => {
                config.validate_definition(definition)?;
                self.validate_native_configuration(config)
            },
            None if self.service_mode => {
                if definition.stages.iter().any(|s| s.kind != StageKind::Tracker) {
                    return Err(Error::Refused("coding definition requires versioned configuration"));
                }
                adapters::validate(definition)
            },
            None => self.validate_definition(definition),
        }
    }
    pub(super) fn select_item_configuration(&mut self, workspace: &str, item_id: &str, item: &Item) -> Result<()> {
        if !self.service_mode { return Ok(()); }
        // The admitted item's policy pin is immutable and also present in every
        // effect/receipt. Never consult the workspace's current selection here.
        let bundle = self.store.workflow_bundle(&item.policy_pin)?.ok_or(Error::Refused("missing item bundle"))?;
        // A group item runs the steps of one of the ticket bundle's groups.
        let expected = match &item.group_of {
            Some(group) => bundle.definition.group_definition(&group.name),
            None => Some(bundle.definition.clone()),
        };
        let group_resource = bundle.adapter.as_ref().and_then(|AdapterConfiguration::Coding(c)| c.landing_resource());
        if expected.as_ref() != Some(&item.definition)
            || (item.resource != Exec::item_resource(workspace, item_id)
                && self.publication_resource(workspace,bundle.adapter.as_ref()) != item.resource
                && !(item.group_of.is_some() && group_resource.as_ref() == Some(&item.resource))) {
            return Err(Error::Refused("item configuration binding changed"));
        }
        // Bundle bytes, workspace/resource and operation pins remain mandatory
        // during recovery; launchable current files and resource locks do not.
        match (&bundle.adapter, &expected) {
            (Some(AdapterConfiguration::Coding(config)), Some(definition)) if item.group_of.is_some() =>
                config.validate_pinned_group(definition)?,
            (Some(AdapterConfiguration::Coding(config)), _) => config.validate_pinned_definition(&bundle.definition)?,
            (None, _) => self.validate_publication(&bundle.definition, None)?,
        }
        self.coding = bundle.adapter.map(|AdapterConfiguration::Coding(config)| config);
        self.resource = item.resource.clone();
        self.base = self.coding.as_ref().map_or_else(|| "0".repeat(40), |c| c.base.clone());
        Ok(())
    }
    pub(super) fn validate_item_launch(&mut self, workspace: &str, item: &Item) -> Result<()> {
        if !self.service_mode { return Ok(()); }
        if let Some(config) = &self.coding { self.validate_workspace_repository(workspace, config)?; }
        if let Some(config) = &self.coding { use super::coding::PinnedDefinition;
            if item.group_of.is_some() { config.validate()?; config.validate_pinned_group(&item.definition)?; } else { self.validate_definition(&item.definition)?; } }
        else { self.validate_definition(&item.definition)?; }
        self.validate_native_host()?;
        if let Some(config) = &self.coding {
            let mut paths = vec![config.repository.join(".git/farcooler-workflow.lock")];
            if let Some(landing) = &config.landing { paths.push(landing.repository.join("farcooler-workflow.lock")); }
            for path in paths {
                if self.configuration_locks.contains_key(&path) { continue; }
                let file = OpenOptions::new().read(true).write(true).create(true).truncate(false)
                    .mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&path)?;
                if !file.metadata()?.is_file() || file.metadata()?.nlink() != 1 { return Err(Error::Refused("aliased configuration lock")); }
                rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
                    .map_err(|_| Error::Refused("configured resource already owned"))?;
                self.configuration_locks.insert(path,file);
            }
        }
        self.validate_native_host()
    }
}
