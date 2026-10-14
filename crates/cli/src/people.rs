//! `farcooler member` and `farcooler rep` (ov-494): the people on a shared
//! runner, and each person's representative.
//!
//! A member is a person who isn't the runner's owner, with the devices that
//! are theirs. Only a workspace's lead talks to its orchestrator; everyone
//! else has a rep, an agent that carries their asks to it and its questions
//! back, and never answers for them. Every agent here, reps included, works
//! on the one Claude account the runner is signed in to, so adding someone,
//! listing who's here and starting a rep all say whose it is.

use clap::{Args, Subcommand};
use farcooler_protocol::capability;
use farcooler_protocol::v1::{self as pb, request, result};

use crate::tasks::{self, board_for};
use crate::workspaces::WORKSPACE_ENV;
use crate::{Fallible, connect_to, expect_value, req, with};

/// `farcooler member`'s subcommands.
#[derive(Debug, Clone, Subcommand)]
pub(crate) enum MemberCmd {
    /// Everyone on this runner besides its owner, their devices, and the
    /// Claude account they all work on.
    List,
    /// Add a person, or change their name or devices. Runner admins only.
    ///
    /// The devices are the ids `farcooler client list` shows, and each must
    /// be enrolled here. A device never named for anyone is the owner's, so a
    /// teammate's devices must be named here before the runner treats them as
    /// that teammate. One left off the list is nobody's, never the owner's.
    Add {
        /// What commands call them: a lowercase letter, then letters, digits or `-`.
        handle: String,
        /// Their name, as people read it.
        #[arg(long)]
        name: String,
        /// One of their enrolled devices, by client id. Repeat for each; the
        /// list replaces the one before.
        #[arg(long = "device")]
        devices: Vec<String>,
    },
    /// Remove a person: their devices are revoked, so they can no longer log
    /// in, a workspace they led goes back to the owner, and their reps stop.
    /// Runner admins only.
    Remove { handle: String },
}

/// `farcooler rep`'s arguments.
#[derive(Debug, Clone, Args)]
pub(crate) struct RepArgs {
    /// Whose rep: a member's handle (`farcooler member list`).
    handle: String,
    /// Which board: a workspace by name or task prefix. Read from
    /// FARCOOLER_WORKSPACE in a pane, and Main when neither says.
    #[arg(long)]
    workspace: Option<String>,
    /// Which repository's board, when two have the workspace's name.
    #[arg(long)]
    repo: Option<String>,
    /// claude, with an optional `:model`.
    #[arg(long, default_value = "claude")]
    harness: String,
}

/// What every answer about members ends with: whose Claude account this is.
pub(crate) fn shared_account(account: &str) -> String {
    match account.trim() {
        "" => "everyone here works on the Claude account signed in on this runner, so its usage and limits are shared"
            .to_string(),
        email => format!("everyone here works on {email}'s Claude account, so its usage and limits are shared"),
    }
}

const NO_PEOPLE: &str = "this runner's Far Cooler doesn't know its people yet. update it and try again";

/// `farcooler member`, on a connected runner.
pub(crate) async fn member(runner: Option<&str>, cmd: MemberCmd, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    println!("{}", member_on(&mut link, cmd, json).await?);
    Ok(())
}

/// `farcooler member` on `link`, answering what to print.
pub(crate) async fn member_on<L: tasks::DispatchLink>(
    link: &mut L,
    cmd: MemberCmd,
    json: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    if !link.capabilities().iter().any(|c| c == capability::REPRESENTATIVES) {
        return Err(NO_PEOPLE.into());
    }
    let (r, done) = match &cmd {
        MemberCmd::List => (req("member.list"), None),
        MemberCmd::Add { handle, name, devices } => (
            with(
                req("member.set"),
                request::Payload::MemberSet(pb::MemberSet {
                    handle: handle.trim().to_string(),
                    name: name.trim().to_string(),
                    client_ids: devices.clone(),
                }),
            ),
            Some(format!("{} is on this runner, as {}", handle.trim(), name.trim())),
        ),
        MemberCmd::Remove { handle } => (
            with(req("member.remove"), request::Payload::MemberRemove(pb::MemberRemove { handle: handle.trim().to_string() })),
            Some(format!(
                "removed {}. their devices are revoked, so they can no longer log in to this runner, and their reps have stopped",
                handle.trim()
            )),
        ),
    };
    let mut r = r;
    r.required_capabilities.push(capability::REPRESENTATIVES.to_string());
    let answer = link.call(r).await.map_err(refused)?;
    let result::Value::MemberList(list) = expect_value(answer.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    Ok(if json { farcooler_client::members_json::members_json(&list).to_string() } else { list_lines(&list, done.as_deref()) })
}

/// The list as people read it: what was done, then one line per member,
/// then the shared account.
pub(crate) fn list_lines(list: &pb::MemberList, done: Option<&str>) -> String {
    let mut lines: Vec<String> = done.map(str::to_string).into_iter().collect();
    if list.members.is_empty() {
        lines.push("no one but the owner is on this runner".to_string());
    }
    for m in &list.members {
        let devices = match m.client_ids.len() {
            0 => "no devices".to_string(),
            _ => m.client_ids.join(", "),
        };
        let rep = match m.rep_terminal_ids.len() {
            0 => "no rep".to_string(),
            1 => "1 rep".to_string(),
            n => format!("{n} reps"),
        };
        let you = if m.handle == list.you { "  (you)" } else { "" };
        lines.push(format!("{:16} {:24} {rep:7} {devices}{you}", m.handle, crate::truncate(&m.name, 24)));
    }
    lines.push(shared_account(&list.claude_account));
    lines.join("\n")
}

/// `farcooler rep`, on a connected runner.
pub(crate) async fn rep(runner: Option<&str>, args: RepArgs, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    println!("{}", rep_on(&mut link, args, std::env::var(WORKSPACE_ENV).ok(), json).await?);
    Ok(())
}

/// `farcooler rep` on `link`, from a pane whose workspace is `pane`.
pub(crate) async fn rep_on<L: tasks::DispatchLink>(
    link: &mut L,
    args: RepArgs,
    pane: Option<String>,
    json: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    if !link.capabilities().iter().any(|c| c == capability::REPRESENTATIVES) {
        return Err(NO_PEOPLE.into());
    }
    let board = board_for(link, args.repo.as_deref(), args.workspace.as_deref(), pane).await?;
    let workspace_id = crate::message::workspace_of(link, &board).await?;
    let mut r = with(
        req("rep.start"),
        request::Payload::RepStart(pb::RepStart {
            workspace_id,
            handle: args.handle.trim().to_string(),
            harness: args.harness.trim().to_string(),
        }),
    );
    r.required_capabilities.push(capability::REPRESENTATIVES.to_string());
    let answer = link.call(r).await.map_err(refused)?;
    let result::Value::RepStarted(started) = expect_value(answer.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    if json {
        let terminal = uuid::Uuid::from_slice(&started.terminal_id).map(|u| u.to_string()).unwrap_or_default();
        let out = serde_json::json!({ "terminal": terminal, "desk": started.desk_task_key, "claude_account": started.claude_account });
        return Ok(out.to_string());
    }
    Ok(format!(
        "{}'s rep is running. what it carries is kept on {}. {}",
        args.handle.trim(),
        started.desk_task_key,
        shared_account(&started.claude_account)
    ))
}

/// A rep's pane (`FARCOOLER_REP`) writes only as itself (ov-494): `--actor`
/// naming anyone else, the owner or the orchestrator included, is refused
/// here, as the daemon refuses it from the pane's process. Any other pane
/// may still name `manager`: an orchestrator recast from an agent's pane does.
pub(crate) fn rep_speaks_as_itself(given: Option<&str>, rep: Option<&str>, pane: Option<&str>) -> Result<(), String> {
    match (given.map(str::trim), rep.filter(|r| !r.is_empty())) {
        (Some(named), Some(_)) if Some(named) != pane.map(str::trim) => {
            Err("a rep writes as itself: leave out --actor. it never answers or decides for its person".into())
        }
        _ => Ok(()),
    }
}

/// This CLI's line for a refusal the runner named.
pub(crate) fn said_about(what: &str) -> Option<&'static str> {
    Some(match what {
        "handle" => "a handle is a lowercase letter, then up to 31 letters, digits or `-`, and not one the runner keeps",
        "member_name" => "a name is one line of up to 64 characters, with no [ or ]",
        "device_taken" => "one of those devices is someone else's. remove it from them first",
        "rep_running" => "their rep is already running on this board",
        "device" => "that isn't a device enrolled on this runner. `farcooler client list` names them",
        "command_preset" => "a rep runs claude, with an optional `:model`",
        _ => return None,
    })
}

/// A refused call, in this CLI's words when the runner named why.
fn refused(e: farcooler_transport::ClientError) -> Box<dyn std::error::Error> {
    if let farcooler_transport::ClientError::Daemon { code, what, .. } = &e {
        let said = match farcooler_core::error::word_for(*code) {
            "scope-denied" => Some("only a runner admin or the owner can do that, or the member themself for their own rep"),
            "not-found" => Some("there's nobody on this runner by that handle"),
            _ => said_about(what),
        };
        if let Some(said) = said {
            return Box::new(tasks::Refused::naming(said.to_string(), *code, what.clone()));
        }
    }
    Box::new(e)
}

#[cfg(test)]
#[path = "people_tests.rs"]
mod tests;
