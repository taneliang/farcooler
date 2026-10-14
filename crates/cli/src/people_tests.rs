//! `farcooler member` and `farcooler rep` (ov-494), against a fake runner
//! that answers as the real one does and records what it was sent.

use super::*;

/// A runner with people on it, or one from before them.
struct Runner {
    sent: Vec<pb::Request>,
    people: bool,
    accounts: bool,
    refuse: Option<(pb::ErrorCode, &'static str)>,
}

const WS: uuid::Uuid = uuid::Uuid::from_u128(0x0303);

fn a_list() -> pb::MemberList {
    pb::MemberList {
        members: vec![
            pb::Member { handle: "alice".into(), name: "Alice Chen".into(), client_ids: vec!["phone-a".into(), "mac-a".into()], rep_terminal_ids: vec![vec![1u8; 16].into()], rep_workspace_ids: vec![crate::id_bytes(WS)], ..Default::default() },
            pb::Member { handle: "bob".into(), name: "Bob".into(), ..Default::default() },
            pb::Member { handle: "carol".into(), name: "Carol".into(), client_ids: vec!["phone-c".into()], rep_terminal_ids: vec![vec![3u8; 16].into(), vec![4u8; 16].into()], rep_workspace_ids: Vec::new(), account: "user_carol".into(), email: "carol@example.com".into() },
        ],
        claude_account: "team@example.com".into(),
        you: "bob".into(),
    }
}

impl tasks::DispatchLink for Runner {
    fn capabilities(&self) -> Vec<String> {
        let mut have = vec!["workstreams".to_string(), "tasks".to_string()];
        if self.people {
            have.push(capability::REPRESENTATIVES.to_string());
        }
        if self.accounts {
            have.push(capability::MEMBER_ACCOUNTS.to_string());
        }
        have
    }
    async fn call(&mut self, req: pb::Request) -> Result<pb::Result, farcooler_transport::ClientError> {
        if let Some((code, what)) = self.refuse.filter(|_| !req.method.ends_with(".list") || req.method == "member.list") {
            return Err(farcooler_transport::ClientError::Daemon { code: code as i32, message: String::new(), what: what.into(), retryable: false });
        }
        let value = match req.method.as_str() {
            "workspace.list" => result::Value::WorkspaceList(pb::WorkspaceList {
                items: vec![pb::Workspace { id: crate::id_bytes(WS), repository_id: crate::id_bytes(WS), is_main: true, name: "Main".into(), ..Default::default() }],
            }),
            "repository.list" => result::Value::RepositoryList(pb::RepositoryList {
                items: vec![pb::Repository { id: crate::id_bytes(WS), display_name: "overnight".into(), ..Default::default() }],
            }),
            "member.list" | "member.set" | "member.remove" => result::Value::MemberList(a_list()),
            "rep.start" => result::Value::RepStarted(pb::RepStarted {
                terminal_id: vec![2u8; 16].into(),
                desk_task_key: "ov-12".into(),
                claude_account: "team@example.com".into(),
            }),
            other => panic!("sent {other}, which this fake doesn't expect"),
        };
        self.sent.push(req);
        Ok(pb::Result { value: Some(value) })
    }
    async fn pause(&mut self, _wait: std::time::Duration) {}
}

fn runner() -> Runner {
    Runner { sent: Vec::new(), people: true, accounts: true, refuse: None }
}

fn parsed(line: &str) -> crate::Command {
    use clap::Parser;
    crate::Cli::try_parse_from(line.split_whitespace()).unwrap().command
}

fn member_cmd(line: &str) -> MemberCmd {
    let crate::Command::Member(cmd) = parsed(line) else { panic!("not member") };
    cmd
}

fn rep_args(line: &str) -> RepArgs {
    let crate::Command::Rep(args) = parsed(line) else { panic!("not rep") };
    args
}

/// `member add` sends the handle, the name and every device, names the
/// capability, says what it did, lists who's here, and ends on the account
/// everyone shares.
#[tokio::test]
async fn adding_someone_says_whose_account_they_join() {
    let mut link = runner();
    let cmd = member_cmd("farcooler member add alice --name Alice --device phone-a --device mac-a");
    let said = member_on(&mut link, cmd, false).await.unwrap();
    let req = &link.sent[0];
    assert_eq!(req.required_capabilities, [capability::REPRESENTATIVES]);
    let Some(request::Payload::MemberSet(set)) = &req.payload else { panic!("{req:?}") };
    assert_eq!((set.handle.as_str(), set.name.as_str(), set.client_ids.as_slice()), ("alice", "Alice", ["phone-a".to_string(), "mac-a".to_string()].as_slice()));
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(lines[0], "alice is on this runner, as Alice");
    assert!(lines[1].starts_with("alice ") && lines[1].contains("Alice Chen") && lines[1].contains("1 rep") && lines[1].ends_with("phone-a, mac-a"), "{said}");
    assert!(lines[2].contains("no rep") && lines[2].ends_with("no devices  (you)"), "{said}");
    assert!(lines[3].contains("2 reps"), "{said}");
    assert_eq!(lines[4], "everyone here works on team@example.com's Claude account, so its usage and limits are shared");
}

/// The Mac binds a teammate it added by QR with their directory account and
/// email (ov-525); left off, neither is sent, so the runner keeps what it has.
#[tokio::test]
async fn adding_a_teammate_sends_their_directory_account() {
    let mut link = runner();
    let cmd = member_cmd("farcooler member add bob --name Bob --device phone-b --account user_bob --email bob@example.com");
    member_on(&mut link, cmd, false).await.unwrap();
    let Some(request::Payload::MemberSet(set)) = &link.sent[0].payload else { panic!() };
    assert_eq!((set.account.as_str(), set.email.as_str()), ("user_bob", "bob@example.com"));

    assert!(link.sent[0].required_capabilities.iter().any(|c| c == capability::MEMBER_ACCOUNTS));

    // A runner from before accounts would drop it: refused, with nothing sent.
    let mut old = Runner { accounts: false, ..runner() };
    let cmd = member_cmd("farcooler member add bob --name Bob --device phone-b --account user_bob");
    let refused = member_on(&mut old, cmd, false).await.expect_err("refused").to_string();
    assert!(refused.contains("too old to record a teammate's directory account"), "{refused}");
    assert!(old.sent.is_empty(), "something was sent: {:?}", old.sent);
    // Without an account, an older runner adds people as it always did.
    let mut old = Runner { accounts: false, ..runner() };
    assert!(member_on(&mut old, member_cmd("farcooler member add bob --name Bob"), false).await.is_ok());

    let mut link = runner();
    member_on(&mut link, member_cmd("farcooler member add bob --name Bob"), false).await.unwrap();
    let Some(request::Payload::MemberSet(set)) = &link.sent[0].payload else { panic!() };
    assert_eq!((set.account.as_str(), set.email.as_str()), ("", ""));
}

/// The account line says the runner's, when the runner couldn't read whose.
#[test]
fn the_shared_account_is_said_even_when_unknown() {
    assert_eq!(
        shared_account(""),
        "everyone here works on the Claude account signed in on this runner, so its usage and limits are shared"
    );
    let empty = list_lines(&pb::MemberList::default(), None);
    assert!(empty.starts_with("no one but the owner is on this runner\n"), "{empty}");
}

/// `--json` is the list's shape, with the account and who's asking.
#[tokio::test]
async fn the_list_as_json() {
    let mut link = runner();
    let out = member_on(&mut link, member_cmd("farcooler member list"), true).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["claude_account"], "team@example.com");
    assert_eq!(v["you"], "bob");
    assert_eq!(v["members"][0]["devices"], serde_json::json!(["phone-a", "mac-a"]));
    assert_eq!(v["members"][0]["reps"], 1);
    // Each rep's pane and board, which the Mac app names it by (ov-508); a
    // runner from before boards were sent says none.
    let pane = uuid::Uuid::from_bytes([1u8; 16]).to_string();
    assert_eq!(v["members"][0]["rep_panes"], serde_json::json!([{ "terminal": pane, "workspace": WS.to_string() }]));
    assert_eq!(v["members"][2]["rep_panes"][1]["workspace"], "");
    assert_eq!(v["members"][1]["rep_panes"], serde_json::json!([]));
    // Who a teammate is in the team directory, for the Mac's People (ov-525).
    assert_eq!((&v["members"][2]["account"], &v["members"][2]["email"]), (&serde_json::json!("user_carol"), &serde_json::json!("carol@example.com")));
    assert_eq!(v["members"][1]["email"], "");
    // Whether this runner keeps a teammate's account, which the Mac reads
    // before it writes any key (ov-525).
    assert_eq!(v["member_accounts"], true);
    let mut old = Runner { accounts: false, ..runner() };
    let out = member_on(&mut old, member_cmd("farcooler member list"), true).await.unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&out).unwrap()["member_accounts"], false);
}

/// `rep` starts one on the board the pane or flag names, and says where its
/// messages are kept and whose account it runs on.
#[tokio::test]
async fn a_rep_is_started_on_the_named_board() {
    let mut link = runner();
    let said = rep_on(&mut link, rep_args("farcooler rep alice --workspace Main"), None, false).await.unwrap();
    let start = link.sent.iter().find(|r| r.method == "rep.start").expect("started");
    let Some(request::Payload::RepStart(p)) = &start.payload else { panic!() };
    assert_eq!((p.workspace_id.as_ref(), p.handle.as_str(), p.harness.as_str()), (WS.as_bytes().as_slice(), "alice", "claude"));
    assert_eq!(start.required_capabilities, [capability::REPRESENTATIVES]);
    assert_eq!(
        said,
        "alice's rep is running. what it carries is kept on ov-12. everyone here works on team@example.com's Claude account, so its usage and limits are shared"
    );
}

/// A runner from before people is told to update, sending nothing; each
/// refusal the runner names has its own line.
#[tokio::test]
async fn refusals_are_said_in_words() {
    let mut old = Runner { people: false, ..runner() };
    let refused = member_on(&mut old, member_cmd("farcooler member list"), false).await.unwrap_err();
    assert_eq!(refused.to_string(), NO_PEOPLE);
    assert!(old.sent.is_empty());
    let cases = [
        (pb::ErrorCode::ScopeDenied, "", "only a runner admin or the owner can do that, or the member themself for their own rep"),
        (pb::ErrorCode::InvalidArgument, "device", "that isn't a device enrolled on this runner. `farcooler client list` names them"),
        (pb::ErrorCode::NotFound, "", "there's nobody on this runner by that handle"),
        (pb::ErrorCode::ResourceConflict, "device_taken", "one of those devices is someone else's. remove it from them first"),
        (pb::ErrorCode::InvalidArgument, "member_name", "a name is one line of up to 64 characters, with no [ or ]"),
    ];
    for (code, what, line) in cases {
        let mut link = Runner { refuse: Some((code, what)), ..runner() };
        let refused = member_on(&mut link, member_cmd("farcooler member add bob --name Bob"), false).await.unwrap_err();
        assert_eq!(refused.to_string(), line, "{code:?} {what}");
    }
    let mut link = Runner { refuse: Some((pb::ErrorCode::ResourceConflict, "rep_running")), ..runner() };
    let refused = rep_on(&mut link, rep_args("farcooler rep alice"), Some(WS.to_string()), false).await.unwrap_err();
    assert_eq!(refused.to_string(), "their rep is already running on this board");
    for what in ["handle", "member_name", "device_taken", "rep_running", "command_preset", "device"] {
        let said = said_about(what).unwrap_or_else(|| panic!("no line for {what}"));
        assert!(!said.ends_with('.') && said.chars().next().is_some_and(char::is_lowercase), "{said}");
    }
}

/// `workspace set --lead` asks for the capability, says who leads now, and
/// has its own words for the two ways handing it on is refused.
#[test]
fn handing_on_the_lead() {
    let crate::Command::Workspace(crate::workspaces::WorkspaceCmd::Set { settings, .. }) =
        parsed("farcooler workspace set Main --lead alice")
    else {
        panic!("not workspace set")
    };
    let req = settings.request(WS, 3);
    let Some(request::Payload::WorkspaceSetSettings(p)) = &req.payload else { panic!() };
    assert_eq!(p.lead.as_deref(), Some("alice"));
    assert!(req.required_capabilities.iter().any(|c| c == capability::REPRESENTATIVES), "{:?}", req.required_capabilities);
    let led = |lead: &str, name: &str| pb::Workspace { name: "Main".into(), lead: Some(lead.into()), lead_name: Some(name.into()), ..Default::default() };
    assert_eq!(
        settings.said(&led("alice", "Alice Chen")),
        "Alice Chen leads Main now: only they talk to its orchestrator, and everyone else through their rep"
    );
    assert_eq!(settings.said(&led("", "")), "the runner's owner leads Main now: only they talk to its orchestrator");
    let refusal = |code: pb::ErrorCode, what: &str| {
        let e = farcooler_transport::ClientError::Daemon { code: code as i32, message: String::new(), what: what.into(), retryable: false };
        crate::workspace_settings::refused(e, true).to_string()
    };
    assert_eq!(refusal(pb::ErrorCode::ScopeDenied, ""), "only a runner admin can hand on a workspace's lead");
    assert_eq!(
        refusal(pb::ErrorCode::InvalidArgument, "lead"),
        "there's nobody on this runner by that handle. `farcooler member list` names them"
    );
}

/// In a rep's pane, `--actor` naming anyone but the pane is refused before
/// anything is sent; its own name, or none, goes through. Outside a rep's
/// pane, an agent's pane may still write as the manager.
#[test]
fn a_rep_writes_only_as_itself() {
    let pane = Some("agent:01a0dad0-afd0-7bd1-9d62-3d125ede9ae2");
    for other in ["user", "manager", "agent:01a0dad0-afd0-7bd1-9d62-000000000000"] {
        let refused = rep_speaks_as_itself(Some(other), Some("alice"), pane).unwrap_err();
        assert_eq!(refused, "a rep writes as itself: leave out --actor. it never answers or decides for its person", "{other}");
    }
    assert!(rep_speaks_as_itself(pane, Some("alice"), pane).is_ok());
    assert!(rep_speaks_as_itself(None, Some("alice"), pane).is_ok());
    assert!(rep_speaks_as_itself(Some("manager"), None, pane).is_ok(), "an orchestrator recast from an agent's pane");
}
