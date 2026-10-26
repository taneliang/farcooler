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
    /// that teammate. One left off the list loses its key here and is
    /// disconnected, and is nobody's, never the owner's: to give it to
    /// someone else, enroll it again.
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
        /// Their team directory account id, when they were added by QR
        /// (the Mac sets it). Kept when left off; refused if another member
        /// has it, or if this member already has a different one.
        #[arg(long)]
        account: Option<String>,
        /// That account's email, which People shows. Kept when left off.
        #[arg(long)]
        email: Option<String>,
        /// The devices `member list` showed as theirs, comma-separated, and
        /// empty for someone not here yet (ov-786). The add is refused,
        /// changing nothing, if their devices have changed since: read the
        /// list again and add from it, so a device someone else just gave
        /// them isn't taken off.
        #[arg(long, value_name = "DEVICES")]
        if_devices: Option<String>,
    },
    /// Remove a person: their devices are revoked, so they can no longer log
    /// in, a workspace they led goes back to the owner, and their reps stop.
    /// Runner admins only.
    Remove { handle: String },
    /// Which of a member's claude.ai connectors their rep may use (ov-586).
    /// With no flag, says which. The member themself sets their own, from
    /// their own device, and the owner sets anyone's; an agent never can.
    ///
    /// A rep reaches no connector until its member turns one on here, and
    /// never a local MCP server. Connectors work only once the member signs
    /// in to Claude on this runner with their claude.ai account.
    Connectors {
        /// Whose rep: a member's handle (`farcooler member list`).
        handle: String,
        /// A connector, by the name claude.ai shows it ("Slack", "Google
        /// Drive"). Repeat for each; the list replaces the one before.
        #[arg(long = "allow", conflicts_with = "none")]
        allow: Vec<String>,
        /// Turn every connector off for their rep.
        #[arg(long)]
        none: bool,
    },
    /// Start signing a member in to Claude on this runner (ov-584). Prints the
    /// sign-in page to open and, with `--json`, the session to finish. Only
    /// the member themself, from their own device.
    ClaudeSignIn { handle: String },
    /// Give a sign-in the code its page showed. The code is read from
    /// standard input, so it is never on a command line.
    ClaudeSignInFinish { session: String },
    /// End a sign-in that is under way.
    ClaudeSignInCancel { session: String },
    /// Sign a member out of Claude on this runner, which also revokes the
    /// login on Anthropic's side.
    ClaudeSignOut { handle: String },
    /// Whether members must sign in to an agent with their own account on
    /// this runner (ov-652). Runner admins only. Off by default: a member
    /// may use the runner's own login for that agent, which is its owner's
    /// account, quota and history. `farcooler member list` says each.
    OwnLogin {
        /// claude, codex or cursor.
        #[arg(value_parser = ["claude", "codex", "cursor"])]
        harness: String,
        /// on: members sign in with their own account. off: they may share
        /// the runner's.
        #[arg(value_parser = ["on", "off"])]
        state: String,
    },
    /// Give a member a role (ov-757). Only the runner's owner, from the app
    /// or their own terminal, not from a pane on the runner.
    ///
    /// A Maintainer edits every workspace's charter, as the owner and each
    /// workspace's lead do, and renames a workspace, changes its prefix, and
    /// sets its PR size, cost line, nudge hours and wake on answer (ov-784).
    /// A member edits a charter only where they lead. Everyone starts a
    /// member.
    Role {
        /// Whose: a member's handle (`farcooler member list`).
        handle: String,
        #[arg(value_parser = ["member", "maintainer"])]
        role: String,
    },
    /// Who changed what about the people here, newest first: a role given
    /// or taken, a member added or removed, their devices changed, a charter
    /// edited through the runner (ov-757), a workspace renamed or given a new
    /// prefix, and its lead handed on (ov-784).
    Log,
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

const NO_ACCOUNTS: &str =
    "this runner's Far Cooler is too old to record a teammate's directory account, so they weren't added. update it and try again";
const NO_DEVICES_READ: &str =
    "this runner's Far Cooler can't check what you read before changing someone's devices. update it, or leave out --if-devices";
const NO_PEOPLE: &str = "this runner's Far Cooler doesn't know its people yet. update it and try again";
const NO_OWN_LOGINS: &str = "this runner's Far Cooler can't let members share its logins yet. update it and try again";

/// `--if-devices`: a comma-separated list, empty for none.
fn devices_read(list: &str) -> pb::MemberDevicesRead {
    let client_ids = list.split(',').map(str::trim).filter(|d| !d.is_empty()).map(str::to_string).collect();
    pb::MemberDevicesRead { client_ids }
}

/// The agents' names as people read them.
fn agent_name(harness: &str) -> &'static str {
    match harness {
        "codex" => "Codex",
        "cursor" => "Cursor",
        _ => "Claude",
    }
}

/// What `member own-login` did, as people read it.
fn own_login_said(harness: &str, on: bool) -> String {
    let agent = agent_name(harness);
    if on {
        format!("members now sign in to {agent} with their own account on this runner, from each pane's next start")
    } else {
        format!("members may now use this runner's own {agent} login, from each pane's next start: your account, quota and history")
    }
}

/// One line per agent: whether members sign in with their own account.
fn own_logins_lines(own: &pb::MemberOwnLogins) -> Vec<String> {
    [("claude", own.claude), ("codex", own.codex), ("cursor", own.cursor)]
        .into_iter()
        .map(|(h, on)| match on {
            true => format!("{}: members sign in with their own account", agent_name(h)),
            false => format!("{}: members may use this runner's own login", agent_name(h)),
        })
        .collect()
}

/// `farcooler member`, on a connected runner.
pub(crate) async fn member(runner: Option<&str>, cmd: MemberCmd, json: bool) -> Fallible {
    let mut link = connect_to(runner).await?;
    if let MemberCmd::ClaudeSignInFinish { session } = &cmd {
        let code = std::io::read_to_string(std::io::stdin())?;
        let call = crate::claude_sign_in::Call::Finish { session, code: &code };
        println!("{}", crate::claude_sign_in::sign_in_on(&mut link, call, json).await?);
        return Ok(());
    }
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
    match &cmd {
        MemberCmd::Role { handle, role } => return roles::role_on(link, handle, role, json).await,
        MemberCmd::Log => return roles::log_on(link, json).await,
        _ => {}
    }
    if let MemberCmd::Connectors { handle, allow, none } = &cmd {
        let set = (!allow.is_empty() || *none).then_some(allow.as_slice());
        return crate::member_connectors::connectors_on(link, handle, set, json).await;
    }
    // A Claude sign-in's calls (ov-584) answer in their own shapes, and
    // the pasted code is read by `member`, from standard input.
    {
        use crate::claude_sign_in::{Call, sign_in_on};
        match &cmd {
            MemberCmd::ClaudeSignIn { handle } => return sign_in_on(link, Call::Start { handle }, json).await,
            MemberCmd::ClaudeSignInCancel { session } => return sign_in_on(link, Call::Cancel { session }, json).await,
            MemberCmd::ClaudeSignOut { handle } => return sign_in_on(link, Call::SignOut { handle }, json).await,
            MemberCmd::ClaudeSignInFinish { .. } => return Err("the code is read from standard input".into()),
            _ => {}
        }
    }
    let (r, done) = match &cmd {
        MemberCmd::List => (req("member.list"), None),
        MemberCmd::Add { handle, name, devices, account, email, if_devices } => (
            with(
                req("member.set"),
                request::Payload::MemberSet(pb::MemberSet {
                    handle: handle.trim().to_string(),
                    name: name.trim().to_string(),
                    client_ids: devices.clone(),
                    account: account.as_deref().unwrap_or_default().trim().to_string(),
                    email: email.as_deref().unwrap_or_default().trim().to_string(),
                    devices_read: if_devices.as_deref().map(devices_read),
                }),
            ),
            Some(format!("{} is on this runner, as {}", handle.trim(), name.trim())),
        ),
        MemberCmd::OwnLogin { harness, state } => (
            with(
                req("member.set_own_login"),
                request::Payload::MemberOwnLoginSet(pb::MemberOwnLoginSet { harness: harness.clone(), on: state == "on" }),
            ),
            Some(own_login_said(harness, state == "on")),
        ),
        MemberCmd::Connectors { .. }
        | MemberCmd::Role { .. }
        | MemberCmd::Log
        | MemberCmd::ClaudeSignIn { .. }
        | MemberCmd::ClaudeSignInFinish { .. }
        | MemberCmd::ClaudeSignInCancel { .. }
        | MemberCmd::ClaudeSignOut { .. } => unreachable!("answered above"),
        // Said once the answer says what was wiped (`removal_said`).
        MemberCmd::Remove { handle } => (
            with(req("member.remove"), request::Payload::MemberRemove(pb::MemberRemove { handle: handle.trim().to_string() })),
            None,
        ),
    };
    let mut r = r;
    r.required_capabilities.push(capability::REPRESENTATIVES.to_string());
    if let MemberCmd::OwnLogin { .. } = &cmd {
        if !link.capabilities().iter().any(|c| c == capability::MEMBER_OWN_LOGINS) {
            return Err(NO_OWN_LOGINS.into());
        }
        r.required_capabilities.push(capability::MEMBER_OWN_LOGINS.to_string());
    }
    // A teammate's directory account (ov-525): a runner from before it would
    // drop the field and bind them with none, so it's refused here instead.
    if let MemberCmd::Add { account: Some(account), .. } = &cmd
        && !account.trim().is_empty()
    {
        if !link.capabilities().iter().any(|c| c == capability::MEMBER_ACCOUNTS) {
            return Err(NO_ACCOUNTS.into());
        }
        r.required_capabilities.push(capability::MEMBER_ACCOUNTS.to_string());
    }
    // What the caller read (ov-786): a runner from before it would drop the
    // field and replace the list as it stands, so it's refused here instead.
    if let MemberCmd::Add { if_devices: Some(_), .. } = &cmd {
        if !link.capabilities().iter().any(|c| c == capability::MEMBER_DEVICES_READ) {
            return Err(NO_DEVICES_READ.into());
        }
        r.required_capabilities.push(capability::MEMBER_DEVICES_READ.to_string());
    }
    let answer = link.call(r).await.map_err(refused)?;
    let result::Value::MemberList(list) = expect_value(answer.value)? else {
        return Err(crate::daemon_link::UNREADABLE.into());
    };
    let done = match &cmd {
        MemberCmd::Remove { handle } => Some(removal_said(handle.trim(), list.removed.as_ref())),
        _ => done,
    };
    if !json {
        return Ok(list_lines(&list, done.as_deref()));
    }
    let mut answer = farcooler_client::members_json::members_json(&list);
    // Whether this runner keeps a member's directory account (ov-525), so the
    // Mac can refuse a teammate on an older one before it writes any key.
    answer["member_accounts"] = link.capabilities().iter().any(|c| c == capability::MEMBER_ACCOUNTS).into();
    // And whether it signs a member in to Claude (ov-584), which the Mac reads
    // to offer the Claude section: it has no handshake of its own to ask.
    answer["claude_sign_in"] = link.capabilities().iter().any(|c| c == capability::CLAUDE_SIGN_IN).into();
    // And whether `member add --if-devices` can be checked here (ov-786).
    answer["member_devices_read"] = link.capabilities().iter().any(|c| c == capability::MEMBER_DEVICES_READ).into();
    Ok(answer.to_string())
}

/// A Claude sign-in's refusals (ov-584), in this CLI's style (`said_about`).
pub(crate) fn sign_in_said(what: &str) -> Option<&'static str> {
    Some(match what {
        "sign_in_too_old" => "this runner's Claude Code is too old to sign in from here. update it to 2.1.268 or later",
        "sign_in_unreadable" => "this runner's Claude Code didn't give a sign-in page Far Cooler recognizes. update it, then try again",
        "sign_in_code" => "that code isn't complete. copy the whole code and try again",
        "code_refused" => "the sign-in didn't accept that code. open the sign-in page again to get a new one",
        "own_device_only" => "sign in to Claude from your own device, not from a pane on the runner",
        "sign_in_too_often" => "that's several sign-ins in a minute. wait a minute, then try again",
        "claude_sign_in_needed" => "sign in to Claude on this runner first. it asks each member to use their own Claude account",
        "codex_sign_in_needed" => "this runner asks each member to use their own Codex account, which can't be connected yet",
        "codex_server_name" => "codex here has an MCP server Far Cooler can't turn off for you, so it won't start on this runner's login. ask the runner's owner to rename it",
        "cursor_sign_in_needed" => "this runner asks each member to use their own Cursor account, which can't be connected yet",
        "unsupported" => "that agent isn't available to you on this runner",
        _ => return None,
    })
}

/// What `member remove` did, as people read it: their devices, their panes,
/// and their Claude login on this runner (ov-588), with what couldn't be
/// done. `None`, from a runner before ov-588, which stops only reps.
pub(crate) fn removal_said(handle: &str, removed: Option<&pb::MemberRemoval>) -> String {
    let mut lines = vec![format!("removed {handle}. their devices are revoked, so they can no longer log in to this runner")];
    let Some(removed) = removed else {
        lines.push("their reps have stopped".to_string());
        return lines.join("\n");
    };
    let failed = |step: &str| removed.failed.iter().any(|f| f == step);
    match removed.panes_stopped {
        0 => {}
        1 => lines.push("their running pane has stopped".to_string()),
        n => lines.push(format!("their {n} running panes have stopped")),
    }
    if failed("panes") {
        lines.push(format!(
            "one of their panes couldn't be stopped. stop it with `farcooler terminal stop`, then run `farcooler member remove {handle}` again to finish deleting their login"
        ));
    }
    if removed.had_login {
        match (failed("keychain"), failed("login_dir")) {
            (false, false) => lines.push("their Claude login is deleted from this runner".to_string()),
            (true, false) => lines.push("their Claude login's folder is deleted, but its Keychain entry couldn't be".to_string()),
            (false, true) => lines.push("their Claude login's Keychain entry is deleted, but its folder couldn't be".to_string()),
            (true, true) => lines.push("their Claude login couldn't be deleted from this runner".to_string()),
        }
        // Signed out on this runner first, unless that couldn't be confirmed
        // (ov-584): only then is claude.ai their step. Whether Anthropic
        // revoked it isn't known here, so it isn't claimed (review M3).
        if failed("revoke") {
            lines.push("its sign-out couldn't be confirmed, so ask them to also sign out of it on claude.ai".to_string());
        } else {
            lines.push("it was signed out on this runner before it was deleted".to_string());
        }
    }
    lines.join("\n")
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
        let role = if m.role == "maintainer" { "  Maintainer" } else { "" };
        lines.push(format!("{:16} {:24} {rep:7} {devices}{role}{you}", m.handle, crate::truncate(&m.name, 24)));
    }
    lines.push(shared_account(&list.claude_account));
    if let Some(own) = &list.own_logins {
        lines.extend(own_logins_lines(own));
    }
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
        "devices_changed" => "their devices changed since you read them, so nothing was changed. run `farcooler member list` and try again",
        "rep_running" => "their rep is already running on this board",
        "device" => "that isn't a device enrolled on this runner. `farcooler client list` names them",
        "command_preset" => "a rep runs claude, with an optional `:model`",
        "connector" => "a connector is named as claude.ai shows it, in up to 64 characters with a letter or digit",
        "connectors" => "a rep can use up to 32 connectors",
        "not_yours" => "only the member themself, from their own device, or the runner's owner can do that",
        "person_only" => "only a person can change that, not an agent",
        "in_pane" => "change that from the app or your own terminal, not from a pane on the runner",
        _ => return None,
    })
}

/// A refused call, in this CLI's words when the runner named why.
pub(crate) fn refused(e: farcooler_transport::ClientError) -> Box<dyn std::error::Error> {
    if let farcooler_transport::ClientError::Daemon { code, what, .. } = &e {
        let said = match farcooler_core::error::word_for(*code) {
            "scope-denied" => Some("only a runner admin or the owner can do that, or the member themself for their own rep"),
            "not-found" => Some("there's nobody on this runner by that handle"),
            _ => said_about(what).or_else(|| sign_in_said(what)),
        };
        if let Some(said) = said {
            return Box::new(tasks::Refused::naming(said.to_string(), *code, what.clone()));
        }
    }
    Box::new(e)
}

#[path = "member_roles.rs"]
mod roles;

#[cfg(test)]
#[path = "people_tests.rs"]
mod tests;
