//! Recover build-output cleanup for finished groups and idle runs.
use super::*;

impl Exec {
    /// Recover cleanup intents and maintain retained process evidence.
    pub(super) fn prune_trees(&self, workspaces: &[String], recovering: bool) {
        if recovering {
            for workspace in workspaces {
                if let Err(error) = self.recover_disk_cleanups(workspace) {
                    tracing::warn!(%error, "could not retain recovery cleanup intents");
                }
            }
        }
        if let Err(error) = self.prune_process_logs(workspaces) {
            tracing::warn!(%error, "could not prune orphan process logs");
        }
    }
}

impl Exec {
    fn recover_disk_cleanups(&self, workspace: &str) -> Result<()> {
        let records = self.store.workflow_cleanups(workspace)?;
        for record in &records {
            if !record.result.as_ref().is_some_and(|result| result.skipped.iter()
                .any(|note| note.starts_with("cleanup attempt interrupted"))) { continue; }
            if record.effect.directory.is_some() {
                self.store.workflow_cleanup_recover(&record.effect, owned_directories::DIRECTORY_INTERRUPTED)?;
            } else if record.effect.tree.is_none() && !record.effect.uncertain && self.ended_group(&record.effect)? {
                self.store.workflow_cleanup_recover(&record.effect, build_cleanup::OUTPUT_INTERRUPTED)?;
            }
        }
        self.retain_finished_group_cleanups(workspace, &records)?;
        self.retain_idle_run_cleanups(workspace, &records)?;
        Ok(())
    }

    /// Revisit output retained by an older engine in a known finished group.
    fn retain_finished_group_cleanups(&self, workspace: &str, records: &[farcooler_store::workflow_cleanup::CleanupRecord]) -> Result<()> {
        let Some(state) = self.state(workspace)? else { return Ok(()) };
        let ended = state.items.iter().filter(|(_, i)| i.group_of.is_some() && i.tree.is_none()
            && matches!(i.phase, Phase::Done | Phase::Failed | Phase::Cancelled))
            .map(|(id, i)| (id, i.run.clone(), Some(i.definition.build_folders())))
            .chain(state.archive.iter().filter(|(_, a)| a.group.is_some()).map(|(id, _)| (id, String::new(), None)));
        for (id, run, folders) in ended {
            if !self.item_tree(workspace, id).try_exists()? || records.iter().any(|r| r.effect.directory.is_none() && r.effect.item == *id
                && r.result.as_ref().is_none_or(|r| r.retry
                    || r.skipped.iter().any(|s| s.starts_with("cleanup attempt interrupted")))) { continue; }
            let folders = folders.or_else(|| {
                // The old whole-tree intent used an empty list regardless of the published policy.
                let legacy_release = digest(&("group-tree-release/v1", workspace, id));
                records.iter().rev().find(|r| r.effect.directory.is_none() && r.effect.item == *id
                    && r.effect.id != legacy_release).map(|r| r.effect.folders.clone())
            }).unwrap_or_else(|| farcooler_workflow::cleanup::DEFAULT_BUILD_FOLDERS.iter().map(|folder| (*folder).into()).collect());
            self.store.workflow_cleanup_enqueue(&farcooler_workflow::cleanup::CleanupEffect { restart: None,
                id: digest(&("group-build-release/v1", workspace, id)), workspace: workspace.into(),
                revision: state.revision, item: id.clone(), run, tree: None, folders, uncertain: false, directory: None,
            })?;
        }
        Ok(())
    }

    fn retain_idle_run_cleanups(&self, workspace: &str, records: &[farcooler_store::workflow_cleanup::CleanupRecord]) -> Result<()> {
        use farcooler_workflow::cleanup::{CleanupEffect, waiting_at_rest};
        let Some(state) = self.state(workspace)? else { return Ok(()) };
        for (id, item) in &state.items {
            let ended_owned_run = matches!(item.phase, Phase::Failed | Phase::Cancelled)
                && !item.tree.as_ref().is_some_and(|tree| tree.adopted);
            if item.group_of.is_some() || item.definition.is_ticket() || item.follows.is_some()
                || !(waiting_at_rest(&item.phase) || ended_owned_run) {
                continue;
            }
            if records.iter().any(|r| r.effect.directory.is_none() && r.effect.item == *id
                && r.effect.run == item.run && r.result.as_ref().is_none_or(|result| result.retry)) {
                continue;
            }
            let tree = item.tree.as_ref().map_or_else(|| self.item_tree_for(workspace, id, item.restart.as_deref()),
                |tree| PathBuf::from(&tree.path));
            let folders = item.definition.build_folders();
            let adopted = item.tree.as_ref().is_some_and(|tree| tree.adopted);
            if !build_cleanup::build_folders(&folders, adopted).any(|folder| build_cleanup::folder_paths(&tree, folder)
                .any(|path| matches!(path, Ok(Some(_)))))
                && !(build_cleanup::default_build_folders(&folders) && build_cleanup::cargo_cache_folders(&tree).next().is_some()) { continue; }
            // The expanded artifact inventory can release output a previous
            // engine left after settling its recovery attempt for this run.
            let policy = if adopted { "idle-adopted-build-release/v3" } else { "idle-build-release/v1" };
            self.store.workflow_cleanup_enqueue(&CleanupEffect {
                id: digest(&(policy, workspace, id, &item.run, item.fence)),
                workspace: workspace.into(), revision: state.revision, item: id.clone(), run: item.run.clone(),
                restart: item.restart.clone(), tree: item.tree.clone(), folders, uncertain: false, directory: None,
            })?;
        }
        Ok(())
    }

    pub(super) fn ended_group(&self, effect: &farcooler_workflow::cleanup::CleanupEffect) -> Result<bool> {
        let Some(state) = self.state(&effect.workspace)? else { return Ok(false) };
        Ok(state.items.get(&effect.item).is_some_and(|i| i.group_of.is_some()
            && matches!(i.phase, Phase::Done | Phase::Failed | Phase::Cancelled))
            || state.archive.get(&effect.item).is_some_and(|a| a.group.is_some()))
    }
}
