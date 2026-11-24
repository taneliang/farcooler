use super::*;

#[derive(clap::Args)]
pub struct Reorder {
    /// The ticket to move.
    key: String,
    /// Place it just above this ticket.
    #[arg(long, required_unless_present = "after", conflicts_with = "after")]
    before: Option<String>,
    /// Place it just below this ticket.
    #[arg(long)]
    after: Option<String>,
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    actor: Option<String>,
}

pub(super) async fn reorder(link: &mut crate::Link, args: Reorder, json: bool) -> Fallible {
    let Reorder { key, before, after, repo, actor } = args;
    let actor = actor_for(actor.as_deref())?;
    let (anchor, after) = match (before, after) {
        (Some(anchor), None) => (anchor, false),
        (None, Some(anchor)) => (anchor, true),
        _ => return Err("choose one ticket with --before or --after".into()),
    };
    let task = find_task(link, repo.as_deref(), &key).await?;
    let anchor = find_task(link, repo.as_deref(), &anchor).await?;
    let r = link.call(farcooler_client::session::task_reorder_request(
        uuid_of(&task.id), uuid_of(&anchor.id), after, actor.to_string(),
    )).await.map_err(|e| refused(e, "the ticket could not be reordered"))?;
    let result::Value::Empty(_) = expect_value(r.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    if json {
        println!("{}", serde_json::json!({ "key": task.key, "anchor": anchor.key, "after": after }));
    } else {
        println!("Placed {} {} {}.", task.key, if after { "after" } else { "before" }, anchor.key);
    }
    Ok(())
}

/// `task.move` of `tasks` to `workspace`, as `actor`.
pub(super) fn move_request(tasks: &[pb::Task], workspace: Uuid, actor: Actor) -> pb::Request {
    needs_workstreams(with(
        req("task.move"),
        request::Payload::TaskMove(pb::TaskMove {
            task_ids: tasks.iter().map(|t| t.id.clone()).collect(),
            workspace_id: id_bytes(workspace),
            actor: actor.to_string(),
        }),
    ))
}
