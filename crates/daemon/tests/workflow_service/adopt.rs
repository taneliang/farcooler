use super::*;
use farcooler_protocol::workflow_commands::NamedTree;

/// ov-1074: Main's first adopt ended Uncertain at Provision. The worktree held nothing but the hooks files Far Cooler
/// writes into every worktree it opens a pane in, untracked, and the step's clean-tree check read them as changes.
/// A tree with only those carries on and lands; one with a person's change in it fails with a sentence that says
/// what to do, never Uncertain, and the change is left as it is.
#[tokio::test]
async fn workflow_service_an_adopted_tree_with_only_our_hooks_files_lands_and_one_with_changes_fails_with_a_sentence() {
    if person_terminal("adopt::workflow_service_an_adopted_tree_with_only_our_hooks_files_lands_and_one_with_changes_fails_with_a_sentence").await { return; }
    let (dir,config)=configured(false,"pass").await;
    let host=Host::open(dir.path()).await; let workspace=host.workspace; let mut link=host.connect().await;
    let definition=config.definition().unwrap();
    let adapter=Some(AdapterConfiguration::Coding(config.clone()));
    let pin=farcooler_store::workflow_publication::configuration_pin(&definition,adapter.as_ref());
    accepted(&call(&mut link,workspace,Action::Publish { notifications:None,mutation:mutation("p",0,1),expected_pin:None,definition,adapter:adapter.map(Box::new)}).await);
    let repository=host.service.store.get_workspace(workspace).unwrap().repository_id;
    let mut sequence=1;
    let mut tickets=Vec::new();
    for (name,theirs) in [("hooks-only",false),("changed",true)] {
        let ticket=host.service.store.create_task(workspace,name,farcooler_store::models::Actor::User).unwrap();
        host.service.track_tickets(Some(ticket.id)).await;
        // Earlier work on a branch of its own, in a worktree the app lists, as a pane left it.
        let tree=dir.path().join("lanes").join(name); let branch=format!("e-liang/{name}");
        git(&config.repository,&["worktree","add","--quiet","-b",&branch,tree.to_str().unwrap(),&config.base]).await.unwrap();
        std::fs::write(tree.join("earlier.txt"),"work from before").unwrap();
        git(&tree,&["add","earlier.txt"]).await.unwrap();
        git(&tree,&["-c","user.name=Test","-c","user.email=test@localhost","-c","commit.gpgsign=false","commit","--quiet","-m","earlier work"]).await.unwrap();
        host.service.store.create_worktree(repository,&branch,tree.to_str().unwrap(),false).unwrap();
        // What the app writes into a worktree it opens a pane in (the live run's git log showed exactly these two).
        for file in farcooler_daemon::hook_install::PROJECT_HOOK_FILES {
            std::fs::create_dir_all(tree.join(file).parent().unwrap()).unwrap();
            std::fs::write(tree.join(file),farcooler_daemon::hook_install::merge_codex("{}",&dir.path().join("h.sock"))).unwrap();
        }
        if theirs { std::fs::write(tree.join("mine.txt"),"someone's work").unwrap(); }
        let snapshot=get(&mut link,workspace).await; let item=&snapshot["state"]["items"][&ticket.key];
        sequence+=1;
        accepted(&call(&mut link,workspace,Action::Start { mutation:mutation(name,snapshot["state"]["revision"].as_u64().unwrap(),sequence),
            item:ticket.key.clone(),run:item["run"].as_str().unwrap().into(),pin:item["pin"].as_str().unwrap().into(),start_pin:pin.clone(),
            models:Default::default(),worktree:Some(NamedTree { name:name.into(),branch,adopt:true }),with:vec![] }).await);
        tickets.push((ticket,tree));
    }
    let settled=|phase:&Phase| matches!(phase,Phase::Done|Phase::Failed|Phase::Uncertain);
    tokio::time::timeout(std::time::Duration::from_secs(30),async { loop {
        let state=host.service.store.workflow_state(&workspace.to_string()).unwrap().unwrap();
        if tickets.iter().all(|(t,_)| state.items.get(&t.key).is_some_and(|i| settled(&i.phase))) { break; }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }}).await.expect("both runs settle");
    let state=host.service.store.workflow_state(&workspace.to_string()).unwrap().unwrap();
    let ((clean,_),(changed,tree))=(&tickets[0],&tickets[1]);
    assert_eq!(state.items[&clean.key].phase,Phase::Done,"only our hooks files: the run carries on and lands: {:?}",state.items[&clean.key]);
    let item=&state.items[&changed.key];
    assert_eq!((&item.phase,item.stage),(&Phase::Failed,0),"a person's change fails the Provision, never Uncertain: {item:?}");
    let said=item.failure.as_deref().unwrap_or_default();
    assert!(said.contains("changes") && said.contains("Commit them") && said.contains("discard them"),"said as a sentence with what to do: {said}");
    assert_eq!(std::fs::read_to_string(tree.join("mine.txt")).unwrap(),"someone's work","the change is left as it is");
    drop(link);host.close().await;
}
