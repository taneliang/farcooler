//! The CLI's own tests: moved out of `main.rs` (ov-455), which was at its
//! size ceiling.

use super::*;

/// Every verb that acts on a layout takes `--layout`, and it comes back
/// as given: the app names the window it shows, so the main checkout's
/// row never acts on an orchestrator's window tmux calls active.
#[test]
fn every_verb_that_acts_on_a_layout_can_name_it() {
    for line in [
        "farcooler layout split main --layout @3",
        "farcooler layout preset main tiled --layout @3",
        "farcooler layout cycle main --layout @3",
        "farcooler layout focus main --next --layout @3",
        "farcooler layout focus main --pane 2 --layout @3",
        "farcooler layout zoom main --layout @3",
        "farcooler layout zoom main --off --layout @3",
        "farcooler layout break main --layout @3",
        "farcooler layout rename main shells --layout @3",
        "farcooler layout viewport main 100 30 --layout @3",
    ] {
        let cli = Cli::try_parse_from(line.split_whitespace()).unwrap_or_else(|e| panic!("{line}: {e}"));
        let Command::Layout(cmd) = cli.command else { panic!("{line}: not a layout verb") };
        assert_eq!(named_layout(&cmd), Some("@3"), "{line}");
    }
    let cli = Cli::try_parse_from("farcooler layout zoom main".split_whitespace()).expect("parses");
    let Command::Layout(cmd) = cli.command else { panic!("not a layout verb") };
    assert_eq!(named_layout(&cmd), None, "naming none is still tmux's active layout");
}

#[test]
fn a_layout_is_named_by_number_name_or_window_id() {
    let group = |id: &str, name: &str| farcooler_protocol::v1::PaneGroup {
        id: id.into(),
        name: name.into(),
        ..Default::default()
    };
    let existing = [group("@4", "orchestrator"), group("@7", "shells")];
    assert_eq!(find_layout(&existing, "2").as_deref(), Ok("@7"));
    assert_eq!(find_layout(&existing, "Shells").as_deref(), Ok("@7"));
    assert_eq!(find_layout(&existing, "@4").as_deref(), Ok("@4"));
    assert!(find_layout(&existing, "3").is_err());
    assert!(find_layout(&existing, "@9").is_err());

    assert!(is_window_id("@0") && is_window_id("@12"));
    for not in ["@", "@x", "3", "shells", "@1 ", "%3"] {
        assert!(!is_window_id(not), "{not:?}");
    }
}

#[test]
fn a_refusal_carries_its_stable_code_word_under_json() {
    let refused: Box<dyn std::error::Error> = Box::new(farcooler_transport::ClientError::Daemon {
        code: farcooler_protocol::v1::ErrorCode::BranchExists as i32,
        retryable: false,
        message: "branch already exists".into(),
        what: String::new(),
    });
    assert_eq!(error_code_lines(refused.as_ref(), true), ["code: branch-exists"]);
    assert!(error_code_lines(refused.as_ref(), false).is_empty(), "a person has the sentence");
    let other: Box<dyn std::error::Error> = "no such worktree".into();
    assert!(error_code_lines(other.as_ref(), true).is_empty());
}

/// A terminal the CLI can't find among the records is not found, said
/// as the daemon says it: the Mac's Close tells a record already reaped
/// from a runner it couldn't reach by this word alone. An ambiguous
/// prefix is the caller's to fix and carries no word.
#[test]
fn a_resolve_miss_carries_not_found_under_json() {
    let terminals = vec![farcooler_protocol::v1::Terminal {
        id: bytes::Bytes::copy_from_slice(Uuid::from_u128(0xabc).as_bytes()),
        ..Default::default()
    }];
    let missing = resolve(&terminals, "def", |t| &t.id, "terminal").map(|_| ()).unwrap_err();
    assert_eq!(missing.to_string(), "no terminal matching \"def\"", "the sentence is unchanged");
    let boxed: Box<dyn std::error::Error> = Box::new(missing);
    assert_eq!(error_code_lines(boxed.as_ref(), true), ["code: not-found"]);
    assert!(error_code_lines(boxed.as_ref(), false).is_empty());

    let twice = vec![terminals[0].clone(), terminals[0].clone()];
    let ambiguous: Box<dyn std::error::Error> =
        Box::new(resolve(&twice, "abc", |t| &t.id, "terminal").map(|_| ()).unwrap_err());
    assert!(error_code_lines(ambiguous.as_ref(), true).is_empty());
}

/// A runner that answers every call with `answer`, and records them.
struct Answering {
    capabilities: Vec<String>,
    answer: Result<farcooler_protocol::v1::Result, farcooler_transport::ClientError>,
    sent: Vec<farcooler_protocol::v1::Request>,
}

impl tasks::DispatchLink for Answering {
    fn capabilities(&self) -> Vec<String> {
        self.capabilities.clone()
    }
    async fn call(
        &mut self,
        req: farcooler_protocol::v1::Request,
    ) -> Result<farcooler_protocol::v1::Result, farcooler_transport::ClientError> {
        self.sent.push(req);
        match &self.answer {
            Ok(r) => Ok(r.clone()),
            Err(farcooler_transport::ClientError::Daemon { code, retryable, message, what }) => {
                Err(farcooler_transport::ClientError::Daemon {
                    code: *code,
                    retryable: *retryable,
                    message: message.clone(),
                    what: what.clone(),
                })
            }
            Err(_) => Err(farcooler_transport::ClientError::EmptyResult),
        }
    }
    async fn pause(&mut self, _: std::time::Duration) {}
}

fn answering(answer: Result<farcooler_protocol::v1::Result, farcooler_transport::ClientError>) -> Answering {
    Answering { capabilities: farcooler_protocol::capability::ALL.iter().map(|c| c.to_string()).collect(), answer, sent: Vec::new() }
}

fn conflict(what: &str) -> farcooler_transport::ClientError {
    farcooler_transport::ClientError::Daemon {
        code: farcooler_protocol::v1::ErrorCode::ResourceConflict as i32,
        retryable: false,
        message: "Someone already answered this.".into(),
        what: what.into(),
    }
}

fn needs_you_item(kind: farcooler_protocol::v1::NeedsYouKind, rank: u32, question: &str) -> farcooler_protocol::v1::NeedsYouItem {
    farcooler_protocol::v1::NeedsYouItem {
        id: format!("{rank}"),
        kind: kind as i32,
        rank,
        workspace_name: "Billing".into(),
        question: question.into(),
        ..Default::default()
    }
}

#[test]
fn needs_you_prints_one_line_per_item_in_rank_order() {
    use farcooler_protocol::v1 as pb;
    let list = pb::NeedsYouList {
        items: vec![
            pb::NeedsYouItem {
                task: Some(pb::TaskRef { key: "bil-9".into(), ..Default::default() }),
                detail: Some("+18 −40".into()),
                ..needs_you_item(pb::NeedsYouKind::Review, 900, "Ready for review")
            },
            pb::NeedsYouItem {
                terminal: Some(pb::TerminalRef { label: "claude".into(), ..Default::default() }),
                ..needs_you_item(pb::NeedsYouKind::Ask, 5, "Allow touch x")
            },
            pb::NeedsYouItem {
                task: Some(pb::TaskRef { key: "bil-7".into(), ..Default::default() }),
                workspace_name: String::new(),
                ..needs_you_item(pb::NeedsYouKind::Decision, 300, "Postgres or SQLite?")
            },
        ],
    };
    let printed = needs_you_output(&list, false);
    let lines: Vec<&str> = printed.lines().collect();
    assert_eq!(
        lines,
        [
            "ask       Billing · claude          Allow touch x",
            "decision  bil-7                     Postgres or SQLite?",
            "review    Billing · bil-9           Ready for review  +18 −40",
        ],
        "{printed}"
    );
    assert_eq!(needs_you_output(&pb::NeedsYouList::default(), false), "nothing needs you");
}

#[tokio::test]
async fn needs_you_json_is_the_client_cores_shape() {
    use farcooler_protocol::v1 as pb;
    let list = pb::NeedsYouList {
        items: vec![pb::NeedsYouItem {
            ask_id: Some("a-1".into()),
            ..needs_you_item(pb::NeedsYouKind::Ask, 5, "Allow touch x")
        }],
    };
    let mut link = answering(Ok(pb::Result {
        value: Some(pb::result::Value::NeedsYouList(list.clone())),
    }));
    let printed = needs_you_read(&mut link, true).await.expect("read");
    assert_eq!(link.sent[0].method, "needs_you.list");
    let printed: serde_json::Value = serde_json::from_str(&printed).expect("--json prints JSON");
    assert_eq!(printed, farcooler_client::needs_you_json::needs_you_json(&list));
    assert_eq!(printed["items"][0]["ask_id"], "a-1", "and not an empty object that happens to match");
}

#[tokio::test]
async fn needs_you_refused_otherwise_says_so_in_its_own_words() {
    for code in [farcooler_protocol::v1::ErrorCode::NotFound, farcooler_protocol::v1::ErrorCode::ResourceConflict] {
        let mut link = answering(Err(farcooler_transport::ClientError::Daemon {
            code: code as i32,
            retryable: false,
            message: "resource not found".into(),
            what: String::new(),
        }));
        let refused = needs_you_read(&mut link, false).await.expect_err("refused");
        assert_eq!(refused.to_string(), NEEDS_YOU_UNREAD, "not the board's sentence");
        assert_eq!(
            error_code_lines(refused.as_ref(), true),
            [format!("code: {}", farcooler_core::error::word_for(code as i32))],
            "the code still reaches a script"
        );
    }
}

#[tokio::test]
async fn needs_you_on_a_runner_without_the_capability_says_to_update_it() {
    let mut old = answering(Err(conflict("")));
    old.capabilities = vec![farcooler_protocol::capability::TASKS.to_string()];
    let refused = needs_you_read(&mut old, false).await.expect_err("an older runner is refused");
    assert!(refused.to_string().contains("update it"), "{refused}");
    assert_eq!(error_code_lines(refused.as_ref(), true), ["code: capability-unsupported"], "a script can still tell");
    assert!(old.sent.is_empty(), "refused without a round trip");

    let mut ancient = answering(Err(conflict("")));
    ancient.capabilities.clear();
    assert!(needs_you_read(&mut ancient, false).await.is_err(), "a runner too old to name capabilities at all");

    // One that advertises it and refuses anyway says the same.
    let mut refusing = answering(Err(farcooler_transport::ClientError::Daemon {
        code: farcooler_protocol::v1::ErrorCode::CapabilityUnsupported as i32,
        retryable: false,
        message: "capability unsupported".into(),
        what: String::new(),
    }));
    let refused = needs_you_read(&mut refusing, false).await.expect_err("refused");
    assert_eq!(refused.to_string(), NO_NEEDS_YOU, "not the board's sentence");
}

/// The runner's two answer conflicts are said in this CLI's words, not
/// its own capitalized sentence, and keep their `what`; any other failure
/// is left as it was.
#[tokio::test]
async fn a_refused_answer_says_which_conflict_in_this_clis_words() {
    let terminal = Uuid::now_v7();
    let mut link = answering(Err(conflict("not_held")));
    let held = answer_agent(&mut link, terminal, "a-1".into(), "allow".into(), Default::default()).await.expect_err("refused");
    assert_eq!(link.sent[0].method, "terminal.agent_answer");
    assert_eq!(held.to_string(), "someone already answered this");
    assert_eq!(error_code_lines(held.as_ref(), true), ["code: resource-conflict", "what: not_held"]);
    let mut link = answering(Err(conflict("not_delivered")));
    let lost = answer_agent(&mut link, terminal, "a-1".into(), "allow".into(), Default::default()).await.expect_err("refused");
    assert_eq!(lost.to_string(), "the answer didn't reach the agent. try again");

    let other = answer_refused(farcooler_transport::ClientError::Daemon {
        code: farcooler_protocol::v1::ErrorCode::NotFound as i32,
        retryable: false,
        message: "resource not found".into(),
        what: String::new(),
    }, "ab12");
    assert_eq!(other.to_string(), farcooler_transport::ClientError::Daemon {
        code: farcooler_protocol::v1::ErrorCode::NotFound as i32,
        retryable: false,
        message: "resource not found".into(),
        what: String::new(),
    }.to_string());
}

/// `what` reaches the error lines, from a bare refusal and from one this
/// CLI reworded, and only when the runner named one.
#[test]
fn a_refusals_what_reaches_the_error_json() {
    let bare: Box<dyn std::error::Error> = Box::new(conflict("not_held"));
    assert_eq!(error_code_lines(bare.as_ref(), true), ["code: resource-conflict", "what: not_held"]);

    let reworded: Box<dyn std::error::Error> = Box::new(tasks::refusal(conflict("not_delivered"), ""));
    assert_eq!(
        error_code_lines(reworded.as_ref(), true),
        ["code: resource-conflict", "what: not_delivered"]
    );

    let unnamed: Box<dyn std::error::Error> = Box::new(conflict(""));
    assert_eq!(error_code_lines(unnamed.as_ref(), true), ["code: resource-conflict"], "no empty what");
}

/// `fork_only` is a field an older daemon would drop, checking out a
/// branch a remote has as if nothing had been asked.
#[test]
fn a_fork_only_create_names_the_capability_it_needs() {
    let repo = uuid::Uuid::now_v7();
    let create = |fork_only| {
        worktree_create_request(
            repo, "fix-it".into(), "fix-it".into(), "HEAD".into(), String::new(), fork_only, None)
    };
    let forking = create(true);
    assert_eq!(forking.required_capabilities, [farcooler_protocol::capability::WORKTREE_FORK_ONLY]);
    let Some(request::Payload::WorktreeCreate(p)) = forking.payload else { panic!("payload") };
    assert!(p.fork_only && !p.adopt_existing);

    let plain = create(false);
    assert!(plain.required_capabilities.is_empty(), "an older daemon is asked nothing new");
    let Some(request::Payload::WorktreeCreate(p)) = plain.payload else { panic!("payload") };
    assert!(!p.fork_only);
}

#[test]
fn a_terminal_created_with_a_prompt_names_the_capability_it_needs() {
    let ws = uuid::Uuid::now_v7();
    let with_prompt =
        terminal_create_request(ws, "claude".into(), "claude".into(), false, Some("fix it".into()), None);
    assert_eq!(with_prompt.required_capabilities, [farcooler_protocol::capability::LAUNCH_PROMPT]);
    let Some(request::Payload::TerminalCreate(p)) = with_prompt.payload else { panic!("payload") };
    assert_eq!(p.prompt.as_deref(), Some("fix it"));

    // Without one, an older daemon is asked nothing it cannot do.
    for prompt in [None, Some("  ".to_string())] {
        let plain = terminal_create_request(ws, "c".into(), "claude".into(), false, prompt, None);
        assert!(plain.required_capabilities.is_empty());
    }
}

/// A task key is a field an older daemon would drop, opening a pane that
/// knows no task while `task dispatch` moves the task into progress.
#[test]
fn a_terminal_created_for_a_task_names_the_capability_it_needs() {
    let ws = uuid::Uuid::now_v7();
    let for_task =
        terminal_create_request(ws, "w".into(), "claude".into(), false, None, Some(" fc-3 ".into()));
    assert_eq!(for_task.required_capabilities, [farcooler_protocol::capability::TERMINAL_TASK]);
    let Some(request::Payload::TerminalCreate(p)) = for_task.payload else { panic!("payload") };
    assert_eq!(p.task_key.as_deref(), Some("fc-3"));

    let both = terminal_create_request(
        ws, "w".into(), "claude".into(), false, Some("go".into()), Some("fc-3".into()));
    assert_eq!(
        both.required_capabilities,
        [farcooler_protocol::capability::LAUNCH_PROMPT, farcooler_protocol::capability::TERMINAL_TASK]
    );

    let none = terminal_create_request(ws, "w".into(), "claude".into(), false, None, Some("".into()));
    assert!(none.required_capabilities.is_empty(), "an empty key is no key");
}

/// The Mac's ⌘N create, word for word (`DaemonClient.startTask`), reaches
/// the daemon fork-only and naming the capability. A renamed or dropped
/// flag would fail every ⌘N task on a new runner with a clap error and no
/// `code:` line, while each half of the chain tested on its own stayed
/// green.
#[test]
fn the_macs_fork_only_create_parses_and_is_sent_fork_only() {
    let argv = [
        "farcooler", "--json", "worktree", "create", "repo", "fix-it", "--branch", "el/fix-it",
        "--no-terminal", "--fork-only",
    ];
    let cli = Cli::try_parse_from(argv).expect("the Mac's argv parses");
    let Command::Worktree(WorktreeCmd::Create(args)) = cli.command else { panic!("worktree create") };
    assert_eq!(args.repo, "repo");
    // What the command's own arm passes: the parsed struct, whole.
    let req = worktree_create_from_args(uuid::Uuid::now_v7(), args, None);
    assert_eq!(req.required_capabilities, [farcooler_protocol::capability::WORKTREE_FORK_ONLY]);
    let Some(request::Payload::WorktreeCreate(p)) = req.payload else { panic!("payload") };
    assert!(p.fork_only);
    assert_eq!((p.task_name.as_str(), p.branch.as_str()), ("fix-it", "el/fix-it"));
    assert!(p.terminal_preset.is_empty(), "--no-terminal");

    // Each flag reaches its own field: one without the other, both ways.
    // The Mac sends both, so its argv alone can't tell them apart.
    for (flags, fork_only, preset) in [
        (&["--fork-only"][..], true, "shell"),
        (&["--no-terminal"][..], false, ""),
        (&[][..], false, "shell"),
    ] {
        let argv = ["farcooler", "worktree", "create", "repo", "n", "--branch", "b"]
            .into_iter()
            .chain(flags.iter().copied());
        let Command::Worktree(WorktreeCmd::Create(args)) =
            Cli::try_parse_from(argv).expect("parses").command
        else {
            panic!("worktree create")
        };
        let req = worktree_create_from_args(uuid::Uuid::now_v7(), args, None);
        let Some(request::Payload::WorktreeCreate(p)) = req.payload else { panic!("payload") };
        assert_eq!((p.fork_only, p.terminal_preset.as_str()), (fork_only, preset), "{flags:?}");
    }
}

/// The rows of `worktree list --json` ride under `worktrees`, and from a
/// runner without workspaces only the four envelope keys are there: the
/// Mac reads this object by name.
#[test]
fn the_worktree_list_envelope_names_its_rows_worktrees() {
    let row = serde_json::json!({ "id": "w1" });
    let v = worktree_list_envelope(true, 2, "e/".into(), vec![row.clone()], None);
    assert_eq!(v["worktrees"], serde_json::json!([row]), "{v}");
    let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(keys, ["branch_prefix", "live_panes", "runtime_healthy", "worktrees"]);
}

/// The plain listing says who owns a worktree and which signal claimed
/// it, and names every other workspace writing there on a line of its
/// own; an unclaimed worktree says neither.
#[test]
fn the_plain_worktree_list_shows_the_claim_and_who_else_is_writing() {
    use farcooler_protocol::v1 as pb;
    let id = |n: u8| bytes::Bytes::copy_from_slice(&[n; 16]);
    let workspaces = [
        pb::Workspace { id: id(3), repository_id: id(2), name: "Main".into(), ..Default::default() },
        pb::Workspace { id: id(4), repository_id: id(2), name: "Billing".into(), ..Default::default() },
    ];
    let w = Worktree {
        id: id(1),
        repository_id: id(2),
        task_name: "fix-it".into(),
        branch: "fix-it".into(),
        workspace_id: Some(id(3)),
        claim_source: Some("hook".into()),
        foreign_writer_workspace_ids: vec![id(4)],
        ..Default::default()
    };
    let lines = worktree_list_lines(&w, &workspaces);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with(&short_bytes(&id(1))), "{lines:?}");
    assert!(lines[0].ends_with("fix-it  [Main, hook]"), "{lines:?}");
    assert_eq!(lines[1], "    also writing here: Billing");

    let quiet = Worktree { foreign_writer_workspace_ids: vec![], claim_source: None, ..w.clone() };
    let lines = worktree_list_lines(&quiet, &workspaces);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].ends_with("fix-it  [Main]"), "{lines:?}");

    let unclaimed = Worktree { workspace_id: None, claim_source: None, foreign_writer_workspace_ids: vec![], ..w };
    let lines = worktree_list_lines(&unclaimed, &workspaces);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].ends_with("fix-it") && !lines[0].contains('['), "{lines:?}");
}

/// `worktree create` claims for `--workspace`, looked for in the target
/// repository, or else for the pane's own workspace when that is in the
/// target repository; and the request names the capability.
#[test]
fn a_new_worktree_is_claimed_for_the_workspace_asking() {
    use farcooler_protocol::v1 as pb;
    let id = |n: u8| bytes::Bytes::copy_from_slice(&[n; 16]);
    let repositories = [
        pb::Repository { id: id(1), display_name: "api".into(), ..Default::default() },
        pb::Repository { id: id(2), display_name: "web".into(), ..Default::default() },
    ];
    let workspaces = [
        pb::Workspace { id: id(11), repository_id: id(1), name: "Main".into(), task_prefix: "api".into(), ..Default::default() },
        pb::Workspace { id: id(12), repository_id: id(1), name: "Billing".into(), task_prefix: "bil".into(), ..Default::default() },
        pb::Workspace { id: id(21), repository_id: id(2), name: "Billing".into(), task_prefix: "wb".into(), ..Default::default() },
    ];
    let api = uuid_of(&id(1));
    let pick = |named: Option<&str>, pane: Option<u8>| {
        worktree_workspace(&workspaces, &repositories, api, named, pane.map(|n| uuid_of(&id(n))))
    };
    assert_eq!(pick(Some("billing"), None), Ok(Some(uuid_of(&id(12)))), "in the target repository only");
    assert_eq!(pick(None, Some(12)), Ok(Some(uuid_of(&id(12)))));
    assert_eq!(pick(Some("Main"), Some(12)), Ok(Some(uuid_of(&id(11)))), "the flag beats the pane");
    assert_eq!(pick(None, Some(21)), Ok(None), "a pane in another repository claims nothing here");
    assert_eq!(pick(None, None), Ok(None));
    assert!(pick(Some("wb"), None).is_err(), "another repository's workspace is refused");

    let argv = ["farcooler", "worktree", "create", "api", "fix-it", "--branch", "fix-it", "--workspace", "Billing"];
    let Command::Worktree(WorktreeCmd::Create(args)) = Cli::try_parse_from(argv).expect("parses").command else {
        panic!("worktree create")
    };
    assert_eq!(args.workspace.as_deref(), Some("Billing"));
    let req = worktree_create_from_args(api, args, Some(uuid_of(&id(12))));
    assert_eq!(req.required_capabilities, [farcooler_protocol::capability::WORKSTREAMS]);
    let Some(request::Payload::WorktreeCreate(p)) = req.payload else { panic!("payload") };
    assert_eq!(p.workspace_id.as_deref(), Some(id(12).as_ref()));
}

/// `worktree adopt` claims its worktree as `worktree create` does: for
/// `--workspace`, else for the pane's own workspace when it's in the
/// repository, else for nobody. It used to claim nothing at all.
#[tokio::test]
async fn an_adopted_worktree_is_claimed_as_a_created_one_is() {
    use farcooler_protocol::v1 as pb;
    let id = |n: u8| bytes::Bytes::copy_from_slice(&[n; 16]);
    struct Fake {
        workspaces: Vec<pb::Workspace>,
        sent: Vec<pb::Request>,
    }
    impl tasks::DispatchLink for Fake {
        fn capabilities(&self) -> Vec<String> {
            farcooler_protocol::capability::ALL.iter().map(|c| c.to_string()).collect()
        }
        async fn pause(&mut self, _: std::time::Duration) {}
        async fn call(&mut self, req: pb::Request) -> Result<pb::Result, farcooler_transport::ClientError> {
            let value = match req.method.as_str() {
                "workspace.list" => result::Value::WorkspaceList(pb::WorkspaceList {
                    items: self
                        .workspaces
                        .iter()
                        .filter(|w| req.target_resource_id.as_ref() == Some(&w.repository_id))
                        .cloned()
                        .collect(),
                }),
                "worktree.create" => result::Value::Worktree(pb::Worktree::default()),
                other => panic!("adopt sent {other}"),
            };
            self.sent.push(req);
            Ok(pb::Result { value: Some(value) })
        }
    }
    let repositories = [
        pb::Repository { id: id(1), display_name: "api".into(), ..Default::default() },
        pb::Repository { id: id(2), display_name: "web".into(), ..Default::default() },
    ];
    let workspaces = vec![
        pb::Workspace { id: id(11), repository_id: id(1), name: "Main".into(), task_prefix: "api".into(), is_main: true, ..Default::default() },
        pb::Workspace { id: id(12), repository_id: id(1), name: "Billing".into(), task_prefix: "bil".into(), ..Default::default() },
        pb::Workspace { id: id(21), repository_id: id(2), name: "Web".into(), task_prefix: "web".into(), ..Default::default() },
    ];
    let adopt = |named: Option<&'static str>, pane: Option<u8>| {
        let (repositories, workspaces) = (repositories.clone(), workspaces.clone());
        async move {
            let mut link = Fake { workspaces, sent: Vec::new() };
            let pane = pane.map(|n| uuid_of(&id(n)));
            adopt_worktree(&mut link, &repositories, "api", "feat/x".into(), named, pane, &mut |_| {})
                .await
                .expect("adopted");
            let create = link.sent.pop().expect("a create");
            assert_eq!(create.method, "worktree.create");
            let Some(request::Payload::WorktreeCreate(p)) = create.payload else { panic!("payload") };
            assert!(p.adopt_existing && p.branch == "feat/x");
            let asked_for_workstreams = create
                .required_capabilities
                .iter()
                .any(|c| c == farcooler_protocol::capability::WORKSTREAMS);
            assert_eq!(asked_for_workstreams, p.workspace_id.is_some(), "an older daemon would drop it");
            p.workspace_id
        }
    };
    assert_eq!(adopt(None, Some(12)).await.as_deref(), Some(id(12).as_ref()), "the pane's own");
    assert_eq!(adopt(Some("Main"), Some(12)).await.as_deref(), Some(id(11).as_ref()), "the flag beats the pane");
    assert_eq!(adopt(None, Some(21)).await, None, "a pane in another repository claims nothing here");
    assert_eq!(adopt(None, None).await, None);

    let argv = ["farcooler", "worktree", "adopt", "api", "feat/x", "--workspace", "Billing"];
    let Command::Worktree(WorktreeCmd::Adopt { workspace, .. }) = Cli::try_parse_from(argv).expect("parses").command
    else {
        panic!("worktree adopt")
    };
    assert_eq!(workspace.as_deref(), Some("Billing"));
}

/// Every command that manages a worktree is under `worktree`, with the
/// flags it had under `workspace`. The Mac app, the manager skill and
/// `task dispatch`'s own advice all spell these out, so a verb left behind
/// fails in front of somebody as a clap error.
#[test]
fn worktree_commands_parse_where_workspace_commands_used_to() {
    for line in [
        "farcooler --json worktree create repo fix-it --branch fix-it --no-terminal --fork-only",
        "farcooler worktree create repo fix-it --branch fix-it --base main --terminal claude",
        "farcooler --json worktree list",
        "farcooler worktree adopt repo feature",
        "farcooler --json worktree branches repo",
        "farcooler worktree reorder a b",
        "farcooler worktree hide a",
        "farcooler worktree unhide a",
        "farcooler worktree remove a",
        "farcooler worktree remove a --confirm a",
        "farcooler --json worktree file-search a query --limit 5",
    ] {
        Cli::try_parse_from(line.split_whitespace()).unwrap_or_else(|e| panic!("{line}: {e}"));
    }
    // The old spellings are gone rather than kept as aliases: `workspace`
    // is back meaning a workstream. A stale `workspace create` parses, so
    // that it can be refused by name (`the_old_create_spelling_points_at_
    // worktree`); a stale verb fails here.
    for line in ["farcooler worktree remove-worktree a", "farcooler workspace hide a"] {
        assert!(Cli::try_parse_from(line.split_whitespace()).is_err(), "{line} still parses");
    }
    Cli::try_parse_from("farcooler workspace list".split_whitespace()).expect("a workstream list");
}

/// `farcooler workspace create <repo> <name> --branch <b>` made a
/// worktree until the rename. Read now, it would make a workspace named
/// after a branch, so it is refused before anything connects, pointing
/// at the command it meant — with its own words filled in.
#[test]
fn the_old_create_spelling_points_at_worktree() {
    let refused = |line: &str| {
        Cli::try_parse_from(line.split_whitespace())
            .map_err(|e| e.to_string())
            .and_then(run_parse_only)
            .err()
            .unwrap_or_else(|| panic!("{line} was taken as a workspace"))
    };
    let err = refused("farcooler workspace create repo fix-it --branch fix-it");
    assert!(err.contains("farcooler worktree create repo fix-it --branch fix-it"), "{err}");
    // Every flag the old command took is the old command.
    for line in [
        "farcooler --json workspace create repo fix-it --branch el/fix-it --no-terminal --fork-only",
        "farcooler workspace create repo fix-it --branch b --base main --terminal claude",
        "farcooler workspace create repo --name x --prefix x --fork-only",
    ] {
        assert!(refused(line).contains("farcooler worktree create"), "{line}");
    }
    // And the new spelling is not refused.
    let ok = Cli::try_parse_from("farcooler workspace create --name Billing --prefix bil".split_whitespace())
        .map_err(|e| e.to_string())
        .and_then(run_parse_only);
    assert!(ok.is_ok());
}

#[test]
fn workspace_commands_parse() {
    for line in [
        "farcooler workspace create repo --name Billing --prefix bil",
        "farcooler workspace create --name Billing --prefix bil",
        "farcooler --json workspace list",
        "farcooler workspace list repo",
        "farcooler --json workspace show Billing --repo repo",
        "farcooler workspace rename Billing Payments",
        "farcooler workspace set-prefix Billing pay",
        "farcooler workspace delete Billing",
        "farcooler workspace start-orchestrator Billing --harness codex --replace",
        "farcooler workspace start-orchestrator Billing --harness claude:opus",
        "farcooler workspace start-orchestrator Billing --harness codex --read bil-3 --repo repo",
        "farcooler worktree assign fix-it --to Billing",
        "farcooler terminal set-role abc orchestrator",
        "farcooler terminal set-role abc agent",
        "farcooler terminal set-role abc shell",
        "farcooler task move ov-3 ov-4 --to Billing",
        "farcooler task move -3 --to Billing",
        "farcooler task list --workspace Billing",
        "farcooler task create --title t --workspace Billing",
    ] {
        Cli::try_parse_from(line.split_whitespace()).unwrap_or_else(|e| panic!("{line}: {e}"));
    }
    for line in ["farcooler terminal set-role abc manager", "farcooler task move --to Billing"] {
        assert!(Cli::try_parse_from(line.split_whitespace()).is_err(), "{line} parses");
    }
}

#[test]
fn a_prompt_that_starts_with_a_dash_is_a_prompt_and_not_a_flag() {
    let cli = Cli::try_parse_from([
        "farcooler", "terminal", "create", "ws", "--preset", "claude", "--prompt", "--help me",
    ])
    .expect("parses");
    let Command::Terminal(TerminalCmd::Create { prompt, .. }) = cli.command else { panic!("create") };
    assert_eq!(prompt.as_deref(), Some("--help me"));
}

/// Every resource with a reader on the other end gets a line.
///
/// The guard over the DISPATCH, not the shape — `task_event_json` below
/// covers the shape, and it stayed green through the exact bug this
/// catches. `events` used to sweep the board arm into `_ => continue`,
/// so the daemon emitted the change, this command silently dropped it,
/// and the Mac board rendered once and never moved. Nothing noticed,
/// because the arm lived inside an infinite loop over a live link. Now it
/// lives in a function, and this asks that function.
#[test]
fn every_resource_a_client_reads_gets_a_line_rather_than_being_swept_up() {
    use farcooler_protocol::v1::event::Payload;
    let kinds = [
        (Payload::TerminalChanged(Default::default()), "terminal"),
        (Payload::WorktreeChanged(Default::default()), "worktree"),
        (Payload::LayoutChanged(Default::default()), "layout"),
        (Payload::FleetChanged(farcooler_protocol::v1::Empty {}), "fleet"),
        (Payload::ChangeSetChanged(Default::default()), "change_set"),
        (Payload::StackChanged(Default::default()), "stack"),
        (Payload::TaskChanged(Default::default()), "task"),
        // Not a resource, but the one line that says every other line
        // may have been lost — and it fell into `_` exactly the way the
        // board arm once did, so a Mac that fell behind never re-read.
        (Payload::EventsMissed(farcooler_protocol::v1::Empty {}), "events_missed"),
        (Payload::NeedsYouChanged(farcooler_protocol::v1::Empty {}), "needs_you"),
        (Payload::Notice(Default::default()), "notice"),
        (Payload::BoardReadsChanged(Default::default()), "reads"),
        (Payload::PlanChanged(Default::default()), "plan"),
        (Payload::PagesChanged(Default::default()), "pages"),
    ];
    for (payload, kind) in kinds {
        let line = event_json(payload)
            .unwrap_or_else(|| panic!("a {kind} change reached a client as nothing at all"));
        assert_eq!(line["kind"], kind, "the wrong resource was named on the line");
    }
}

/// **A task notice's line carries what the Mac posts it from** (ov-94):
/// `NoticeEvent` in apps/macos/Sources/FarCooler/EventStream.swift reads
/// these keys, and a key renamed here is a notice the Mac never posts.
#[test]
fn a_notice_line_carries_what_the_mac_posts() {
    let line = event_lines::notice_json(&farcooler_protocol::v1::Notice {
        notice_id: "t:r-1:ov-90".into(),
        event: "decision".into(),
        level: "time-sensitive".into(),
        title: "ov-90 Wake the agent".into(),
        body: "Needs your decision · Which?".into(),
        task_key: "ov-90".into(),
        runner_id: "r-1".into(),
        options: vec!["pdfkit".into()],
        repository_id: bytes::Bytes::copy_from_slice(uuid::Uuid::from_u128(7).as_bytes()),
        ..Default::default()
    });
    assert_eq!(line["repository"], uuid::Uuid::from_u128(7).to_string());
    assert_eq!(line["kind"], "notice");
    assert_eq!(line["notice_id"], "t:r-1:ov-90");
    assert_eq!(line["event"], "decision");
    assert_eq!(line["level"], "time-sensitive");
    assert_eq!(line["title"], "ov-90 Wake the agent");
    assert_eq!(line["body"], "Needs your decision · Which?");
    assert_eq!(line["task"], "ov-90");
    assert_eq!(line["runner"], "r-1");
    assert_eq!(line["options"], serde_json::json!(["pdfkit"]));
    // And `status --json` says whether the relay will push it instead.
    let counts = StatusCounts { roots: 0, repositories: 0, worktrees: 0, terminals: 0 };
    let host = farcooler_protocol::v1::Host { push_paired: true, ..Default::default() };
    assert_eq!(status_json(&host, &[], counts)["pushPaired"], true);
    // And which runner it is, by the id the notice names it by (ov-106).
    let counts = || StatusCounts { roots: 0, repositories: 0, worktrees: 0, terminals: 0 };
    let host = farcooler_protocol::v1::Host { runner_id: "r-1".into(), ..Default::default() };
    assert_eq!(status_json(&host, &[], counts())["runnerId"], "r-1");
    let json = status_json(&farcooler_protocol::v1::Host::default(), &[], counts());
    assert!(json.get("runnerId").is_some_and(|v| v.is_null()), "{json}");
    // And which agents it can start (ov-205), but only from a runner that
    // says it reports them: an older one's empty list is no answer.
    let host = farcooler_protocol::v1::Host { agents_found: vec!["codex".into()], ..Default::default() };
    let says = ["agents_found".to_string()];
    assert_eq!(status_json(&host, &says, counts())["agentsFound"], serde_json::json!(["codex"]));
    assert!(status_json(&host, &[], counts())["agentsFound"].is_null());
}

#[test]
fn status_says_the_read_only_folders_only_when_the_runner_does() {
    let counts = || StatusCounts { roots: 0, repositories: 0, worktrees: 0, terminals: 0 };
    let host = farcooler_protocol::v1::Host {
        read_only_folders: vec![farcooler_protocol::v1::ReadOnlyFolder { name: "logs".into(), path: "/var/log".into() }],
        ..Default::default()
    };
    let says = vec![farcooler_protocol::capability::READ_ONLY_FOLDERS.to_string()];
    assert_eq!(
        status_json(&host, &says, counts())["readOnlyFolders"],
        serde_json::json!([{"name": "logs", "path": "/var/log"}])
    );
    assert!(status_json(&host, &[], counts())["readOnlyFolders"].is_null());
}

/// **Which way two builds differ** (ov-143). The Mac read only
/// `buildsMatch`, so a runner a newer Mac installed looked "behind" to an
/// older one, which offered an "Update" that was a downgrade.
#[test]
fn status_says_when_the_runner_is_newer_than_this_build() {
    let counts = || StatusCounts { roots: 0, repositories: 0, worktrees: 0, terminals: 0 };
    let here = farcooler_store::DatabaseSchema::here();
    let at = |schema_version| farcooler_protocol::v1::Host { schema_version, ..Default::default() };
    assert_eq!(status_json(&at(here + 1), &[], counts())["runnerIsNewer"], true);
    assert_eq!(status_json(&at(here), &[], counts())["runnerIsNewer"], false);
    assert_eq!(status_json(&at(here - 1), &[], counts())["runnerIsNewer"], false);
    // A runner too old to say is not newer.
    assert_eq!(status_json(&at(0), &[], counts())["runnerIsNewer"], false);
}

/// **A runner whose agents run a stand-in says so, in `status`** (ov-49).
/// The phones have said it since `Host.stand_in_agent` existed; the CLI
/// dropped the field, so the Mac, which reads `status --json`, and anybody
/// at a terminal could not see it.
#[test]
fn status_says_when_a_runners_agents_run_a_stand_in() {
    let counts = || StatusCounts { roots: 0, repositories: 0, worktrees: 0, terminals: 0 };
    let host = farcooler_protocol::v1::Host {
        stand_in_agent: "/bin/sleep".into(),
        ..Default::default()
    };
    assert_eq!(status_json(&host, &[], counts())["standInAgent"], "/bin/sleep");
    let line = stand_in_line(&host).expect("plain status said nothing about the stand-in");
    assert!(line.starts_with("agents        "), "out of step with the other rows: {line}");
    assert!(line.contains("/bin/sleep"), "the line doesn't name the program: {line}");

    // And a runner whose agents are themselves — or one too old to say —
    // claims nothing: a null, not an empty string, and no line.
    let host = farcooler_protocol::v1::Host::default();
    let json = status_json(&host, &[], counts());
    assert!(json.get("standInAgent").is_some_and(|v| v.is_null()), "{json}");
    assert_eq!(stand_in_line(&host), None);
}

/// And a payload with no reader is still dropped.
///
/// The pair matters. Turning `_ => None` into "everything gets a line" is
/// one careless edit away, and it would put a JSON object on stdout for
/// every chunk of terminal output a busy runner produces.
#[test]
fn a_payload_with_no_reader_is_still_dropped() {
    use farcooler_protocol::v1::event::Payload;
    assert!(event_json(Payload::TerminalFrame(Default::default())).is_none());
    assert!(event_json(Payload::HostChanged(Default::default())).is_none());
}

/// Every kind of event the proto can carry is decided on, and none by
/// omission.
///
/// The list in the test above is kept by hand, which is how
/// `events_missed` stayed off it: the proto gained a kind, `_ => None`
/// swallowed it, and that test had nothing to say because nobody had
/// added a row. So this one walks the proto's own oneof — every tag an
/// `Event` decodes a payload from — and asks `event_json` about each. A
/// kind added to the proto fails here until somebody either gives it a
/// line (and a row above) or writes its tag into `NO_READER` on purpose.
#[test]
fn every_kind_of_event_the_proto_carries_is_decided_on() {
    use farcooler_protocol::v1::Event;
    use prost::Message;
    // Dropped on purpose: `host_changed`, `repository_root_changed`,
    // `repository_changed`, `operation_changed`, `agent_events` and
    // `terminal_frame`. See `a_payload_with_no_reader_is_still_dropped`.
    const NO_READER: &[u32] = &[10, 11, 12, 15, 17, 20];
    let mut kinds = 0;
    // From 3: tags 1 and 2 are `event_id` and `sequence`, not payloads.
    for tag in 3..256 {
        // The tag, then a zero-length body: the default of whichever
        // message the oneof holds at that tag.
        let mut bytes = Vec::new();
        prost::encoding::encode_key(tag, prost::encoding::WireType::LengthDelimited, &mut bytes);
        bytes.push(0);
        // An unknown tag decodes to no payload, and is skipped. A known
        // one that refuses a zero-length body isn't a message at all —
        // a scalar in the oneof — and this walk can't build it, so it
        // says so rather than stepping over a kind it never asked about.
        let event = Event::decode(bytes.as_slice()).unwrap_or_else(|e| {
            panic!("event tag {tag} can't be built as an empty message, so this walk can't ask about it: {e}")
        });
        let Some(payload) = event.payload else { continue };
        kinds += 1;
        assert_eq!(
            event_json(payload).is_none(),
            NO_READER.contains(&tag),
            "event tag {tag} is new to `event_json`: give it a line, or say in NO_READER that nothing reads it",
        );
    }
    // So a walk that decoded nothing can't pass by asking nothing.
    assert!(kinds >= 14, "the walk found {kinds} kinds of event, fewer than the proto has");
}

/// The board event carries what a client needs to act on it.
///
/// The bug this guards is the one this whole surface's history is about: a
/// board that renders once and never moves. The daemon emits
/// `TaskChanged` on every board write, the CLI is the Mac app's transport,
/// and for a while this match arm fell through to `_ => continue` — so the
/// event existed, the daemon sent it, every test passed, and the board on
/// screen sat still.
#[test]
fn a_board_change_reaches_a_client_with_the_repository_and_the_actor() {
    let task = uuid::Uuid::now_v7();
    let repository = uuid::Uuid::now_v7();
    let json = event_lines::task_event_json(&farcooler_protocol::v1::TaskChanged {
        task_id: bytes::Bytes::copy_from_slice(task.as_bytes()),
        repository_id: bytes::Bytes::copy_from_slice(repository.as_bytes()),
        actor: "agent:0198f2c0-0000-7000-8000-000000000001".into(),
        ..Default::default()
    });
    assert_eq!(json["kind"], "task");
    // The repository is the read a client makes, so it is the field a
    // board cannot be refreshed without.
    assert_eq!(json["repository"], repository.to_string());
    assert_eq!(json["task"], task.to_string());
    assert_eq!(json["short"], short_bytes(&bytes::Bytes::copy_from_slice(task.as_bytes())));
    assert_eq!(
        json["actor"], "agent:0198f2c0-0000-7000-8000-000000000001",
        "the actor was dropped, and a client can no longer tell its own write apart"
    );
    // A write that didn't move the task names its board and no other.
    assert_eq!(json["workspace"], serde_json::json!(null), "no workspace was sent");
    assert_eq!(json["from_workspace"], serde_json::json!(null));
}

/// A board event names its board, and a move names both boards, so the
/// board the task left reads itself again too. Spelled as the FFI's line
/// spells them.
#[test]
fn a_board_change_names_its_board_and_a_move_names_both() {
    let (billing, main) = (uuid::Uuid::now_v7(), uuid::Uuid::now_v7());
    let bytes = |u: uuid::Uuid| bytes::Bytes::copy_from_slice(u.as_bytes());
    let json = event_lines::task_event_json(&farcooler_protocol::v1::TaskChanged {
        workspace_id: Some(bytes(billing)),
        from_workspace_id: Some(bytes(main)),
        ..Default::default()
    });
    assert_eq!(json["workspace"], billing.to_string());
    assert_eq!(json["from_workspace"], main.to_string());
    let nil = event_lines::task_event_json(&farcooler_protocol::v1::TaskChanged {
        workspace_id: Some(bytes(uuid::Uuid::nil())),
        ..Default::default()
    });
    assert_eq!(nil["workspace"], serde_json::json!(null), "never the nil uuid");
}

/// Every actor word crosses whole, including the two that have no id in
/// them.
#[test]
fn every_actor_word_crosses_the_cli_unchanged() {
    for word in ["user", "manager", "agent:0198f2c0-0000-7000-8000-000000000001"] {
        let json = event_lines::task_event_json(&farcooler_protocol::v1::TaskChanged {
            task_id: bytes::Bytes::new(),
            repository_id: bytes::Bytes::new(),
            actor: word.to_string(),
            ..Default::default()
        });
        assert_eq!(json["actor"], word, "the actor was rewritten on the way out");
    }
}

#[test]
fn a_terminal_changed_event_carries_chat_capable() {
    // The regression this test exists to catch: `events` used to build
    // its own JSON object field by field and simply left `chatCapable`
    // out, so a codex pane that relabeled itself live from this exact
    // event never told the client it could be switched to chat.
    let t = farcooler_protocol::v1::Terminal { chat_capable: true, ..Default::default() };
    let json = terminal_event_json(&t);
    assert_eq!(json["chatCapable"], serde_json::json!(true));
}

#[test]
fn a_chat_incapable_terminal_says_so_rather_than_omitting_the_key() {
    let t = farcooler_protocol::v1::Terminal { chat_capable: false, ..Default::default() };
    let json = terminal_event_json(&t);
    assert_eq!(json["chatCapable"], serde_json::json!(false));
}

#[test]
fn a_terminal_changed_event_carries_the_exit_status() {
    // The same regression as `chatCapable`, one field later: a live push
    // that moves a terminal to `exited` is exactly the moment a client
    // needs to tell a clean exit from a failed one apart, and this
    // function had already left one field out of this same object before.
    let t = farcooler_protocol::v1::Terminal {
        exit_status: Some(farcooler_protocol::v1::ExitStatus { code: Some(101), signal: None }),
        ..Default::default()
    };
    let json = terminal_event_json(&t);
    assert_eq!(json["exitCode"], serde_json::json!(101));
    assert_eq!(json["exitSignal"], serde_json::json!(null));
}

#[test]
fn a_terminal_changed_event_carries_the_turn_clock_and_the_question() {
    // The regression this exists to catch: a row that just went Blocked
    // over this exact event is the one moment "Needs you" and the
    // question under it both need to be true at once, and this function
    // had already left two other fields out of this same object twice
    // before.
    let t = farcooler_protocol::v1::Terminal {
        turn_started_at: Some(prost_types::Timestamp { seconds: 1_700_000_000, nanos: 0 }),
        blocked_question: Some("Overwrite config.toml?".to_string()),
        ..Default::default()
    };
    let json = terminal_event_json(&t);
    assert_eq!(json["turnStartedAt"], serde_json::json!(1_700_000_000_000_i64));
    assert_eq!(json["blockedQuestion"], serde_json::json!("Overwrite config.toml?"));
}

/// The other terminal-to-JSON function in this file, and the one the Mac
/// app's `refresh()` actually calls (`worktree list --json`) — see the
/// function's own doc comment for why that distinction matters. Every
/// field this stage of the branch added is checked here, because this is
/// the function where three of them turned out to be missing at once.
#[test]
fn the_full_list_carries_every_field_this_branch_added() {
    let t = farcooler_protocol::v1::Terminal {
        exit_status: Some(farcooler_protocol::v1::ExitStatus { code: Some(101), signal: None }),
        turn_started_at: Some(prost_types::Timestamp { seconds: 1_700_000_000, nanos: 0 }),
        blocked_question: Some("Overwrite config.toml?".to_string()),
        chat_capable: true,
        feed: vec!["Written to haiku.txt.".to_string(), "Both tests pass.".to_string()],
        subagents: vec!["Auditing the redaction rules".to_string()],
        ..Default::default()
    };
    let json = worktree_list_terminal_json(&t);
    assert_eq!(json["exitCode"], serde_json::json!(101));
    assert_eq!(json["exitSignal"], serde_json::json!(null));
    assert_eq!(json["turnStartedAt"], serde_json::json!(1_700_000_000_000_i64));
    assert_eq!(json["blockedQuestion"], serde_json::json!("Overwrite config.toml?"));
    // Not new this round, but this is the function `chatCapable` was
    // already known to be present in — kept as a canary so a future
    // refactor of this function trips a test rather than a support ticket.
    assert_eq!(json["chatCapable"], serde_json::json!(true));
    assert_eq!(json["feed"], serde_json::json!(["Written to haiku.txt.", "Both tests pass."]));
    assert_eq!(json["subagents"], serde_json::json!(["Auditing the redaction rules"]));
}

/// The feed, on the push path.
///
/// A step is news for as long as the agent is on it. A feed that only
/// arrived on a full refresh would always be describing the previous
/// minute, which for the one field whose whole job is "what is it doing
/// RIGHT NOW" is the same as not sending it.
#[test]
fn a_terminal_changed_event_carries_the_feed_and_the_subagents() {
    let t = farcooler_protocol::v1::Terminal {
        feed: vec!["Written to haiku.txt.".to_string(), "Both tests pass.".to_string()],
        subagents: vec!["Auditing the redaction rules".to_string()],
        ..Default::default()
    };
    let json = terminal_event_json(&t);
    assert_eq!(json["feed"], serde_json::json!(["Written to haiku.txt.", "Both tests pass."]));
    assert_eq!(json["subagents"], serde_json::json!(["Auditing the redaction rules"]));
}

/// A plain shell has nothing to report, and says so with an empty list
/// rather than by omitting the key — a client that had to tell "no feed"
/// from "feed missing" would be guessing.
#[test]
fn a_terminal_with_nothing_to_report_sends_an_empty_feed() {
    let t = farcooler_protocol::v1::Terminal::default();
    assert_eq!(terminal_event_json(&t)["feed"], serde_json::json!([]));
    assert_eq!(worktree_list_terminal_json(&t)["feed"], serde_json::json!([]));
    assert_eq!(terminal_event_json(&t)["subagents"], serde_json::json!([]));
    assert_eq!(worktree_list_terminal_json(&t)["subagents"], serde_json::json!([]));
}

/// Keys the EVENT projection carries that the list one has no business
/// carrying: an event has to say what kind of thing changed and which
/// worktree it is in, because it arrives on its own with no surrounding
/// document. A list entry is already inside both.
const EVENT_ONLY: &[&str] = &["kind", "worktree"];

/// Keys the LIST projection carries that the event one deliberately does
/// not. `epoch` is write-conflict bookkeeping for a client about to send a
/// command, not something a row renders, and it was left off the event path
/// on purpose.
const LIST_ONLY: &[&str] = &["epoch"];

/// The two projections of a `Terminal` must not drift apart again.
///
/// This is the test that would have caught this branch's own worst bug, and
/// the two before it. Three times a field was added to one of these
/// functions and not the other — `chatCapable`, then
/// `exitCode`/`exitSignal`, then `turnStartedAt`/`blockedQuestion`, that
/// last pair missing from BOTH — and every suite stayed green while every
/// headline feature of this branch was invisible on the shipped Mac app.
///
/// Field-by-field assertions cannot catch that: they test the fields
/// somebody remembered. This walks the KEY SETS, so a field added to one
/// projection and forgotten in the other fails here with no test change at
/// all. The only way past it is to name the new key in `EVENT_ONLY` or
/// `LIST_ONLY` above, which is a deliberate act with a comment attached.
#[test]
fn the_two_terminal_projections_agree_on_every_field() {
    // Fully populated, because a projection that reads an absent optional
    // still emits its key — but a reader comparing the two by hand would
    // rather see real values in the failure message.
    let t = farcooler_protocol::v1::Terminal {
        exit_status: Some(farcooler_protocol::v1::ExitStatus { code: Some(101), signal: None }),
        turn_started_at: Some(prost_types::Timestamp { seconds: 1_700_000_000, nanos: 0 }),
        blocked_question: Some("Overwrite config.toml?".to_string()),
        chat_capable: true,
        feed: vec!["Written to haiku.txt.".to_string(), "Both tests pass.".to_string()],
        subagents: vec!["Auditing the redaction rules".to_string()],
        glyph: "?".to_string(),
        headline: "claude needs you".to_string(),
        line: "Overwrite config.toml?".to_string(),
        rank: 1,
        // A blocked agent still has a position in its list, and `line`
        // above is the question rather than `3/7` — which is exactly why
        // these travel as numbers.
        plan_done: Some(3),
        plan_total: Some(7),
        turn_failed: true,
        ..Default::default()
    };

    let keys = |value: &serde_json::Value, except: &[&str]| -> std::collections::BTreeSet<String> {
        value
            .as_object()
            .expect("a terminal projects to an object")
            .keys()
            .filter(|k| !except.contains(&k.as_str()))
            .cloned()
            .collect()
    };
    let event = keys(&terminal_event_json(&t), EVENT_ONLY);
    let list = keys(&worktree_list_terminal_json(&t), LIST_ONLY);

    assert_eq!(
        event, list,
        "the two terminal projections disagree.\n\
         only in the event JSON: {:?}\n\
         only in `worktree list --json`: {:?}\n\
         Add the field to both, or name it in EVENT_ONLY/LIST_ONLY with a reason.",
        event.difference(&list).collect::<Vec<_>>(),
        list.difference(&event).collect::<Vec<_>>(),
    );

    // Not vacuous by construction either: an empty set equals an empty set,
    // so the branch's own fields are named here to prove the sets are real.
    for field in [
        "exitCode",
        "exitSignal",
        "turnStartedAt",
        "blockedQuestion",
        "chatCapable",
        "feed",
        "said",
        "subagents",
        "glyph",
        "headline",
        "line",
        "rank",
        "planDone",
        "planTotal",
        "turnFailed",
        "agentFailure",
        "taskId",
        "workspace",
        "role",
        "splitOf",
        "splitOfOrchestrator",
        "ports",
        "draftHold",
    ] {
        assert!(event.contains(field), "{field} is in neither projection");
    }
}

/// A draft held behind a dialog crosses both projections in the shape the
/// apps read (ov-385), and is null when there's none.
#[test]
fn a_held_draft_crosses_both_projections() {
    let id = Uuid::now_v7();
    let t = farcooler_protocol::v1::Terminal {
        draft_hold: Some(farcooler_protocol::v1::DraftHold {
            id: bytes::Bytes::copy_from_slice(id.as_bytes()),
            state: farcooler_protocol::v1::DraftHoldState::Sent as i32,
            held_ms: 5,
            expires_ms: 1_800_005,
            ended_ms: 9,
        }),
        ..Default::default()
    };
    let want = serde_json::json!({
        "id": id.to_string(), "state": "sent", "heldMs": 5, "expiresMs": 1_800_005, "endedMs": 9,
    });
    assert_eq!(worktree_list_terminal_json(&t)["draftHold"], want);
    assert_eq!(terminal_event_json(&t)["draftHold"], want);
    let none = farcooler_protocol::v1::Terminal::default();
    assert_eq!(terminal_event_json(&none)["draftHold"], serde_json::Value::Null);
}

/// The ports a terminal serves cross both projections, as numbers, so the
/// Mac shows `:5173` without reading the label.
#[test]
fn a_terminals_ports_cross_both_projections() {
    let t = farcooler_protocol::v1::Terminal { ports: vec![5173, 9229], ..Default::default() };
    assert_eq!(worktree_list_terminal_json(&t)["ports"], serde_json::json!([5173, 9229]));
    assert_eq!(terminal_event_json(&t)["ports"], serde_json::json!([5173, 9229]));
    let none = farcooler_protocol::v1::Terminal::default();
    assert_eq!(worktree_list_terminal_json(&none)["ports"], serde_json::json!([]));
}

/// The board task a pane was opened for crosses both projections.
///
/// The daemon has stored `terminals.task_id` and put it on the wire since
/// `terminal_task`, and both of these dropped it, so no app could go from
/// a card on the board to the agent working it. The Mac reads it from
/// `worktree list`: a pane's task is set when it is created, and a new
/// pane's first event makes the Mac re-read the list. The event carries
/// it too so the two projections stay one shape — which
/// `the_two_terminal_projections_agree_on_every_field` enforces — for any
/// client that applies events without re-reading.
#[test]
fn a_dispatched_terminal_names_its_task_in_both_projections() {
    let task = uuid::Uuid::now_v7();
    let t = farcooler_protocol::v1::Terminal {
        task_id: Some(bytes::Bytes::copy_from_slice(task.as_bytes())),
        ..Default::default()
    };
    assert_eq!(worktree_list_terminal_json(&t)["taskId"], task.to_string());
    assert_eq!(terminal_event_json(&t)["taskId"], task.to_string());
}

/// A split names the pane it was split from in both projections, and
/// anything else names none, never the nil uuid: the Mac reads this to
/// tell a pane somebody split beside the orchestrator from one that
/// landed in its window another way (ov-73).
#[test]
fn a_split_names_the_pane_it_was_split_from_in_both_projections() {
    let from = uuid::Uuid::now_v7();
    let t = farcooler_protocol::v1::Terminal {
        split_of: Some(bytes::Bytes::copy_from_slice(from.as_bytes())),
        ..Default::default()
    };
    assert_eq!(worktree_list_terminal_json(&t)["splitOf"], from.to_string());
    assert_eq!(terminal_event_json(&t)["splitOf"], from.to_string());
    for split_of in [None, Some(bytes::Bytes::new()), Some(bytes::Bytes::from_static(b"nope"))] {
        let t = farcooler_protocol::v1::Terminal { split_of, ..Default::default() };
        assert_eq!(worktree_list_terminal_json(&t)["splitOf"], serde_json::json!(null));
        assert_eq!(terminal_event_json(&t)["splitOf"], serde_json::json!(null));
    }
}

/// Whether a split was made from the orchestrator crosses both
/// projections, either way, and only beside a `splitOf` that reads: the
/// Mac keeps a pane split beside a terminal later made the orchestrator
/// in its Move to Its Own Window notice (ov-76).
#[test]
fn a_split_says_whether_it_was_made_from_the_orchestrator_in_both_projections() {
    let from = Some(bytes::Bytes::copy_from_slice(uuid::Uuid::now_v7().as_bytes()));
    for said in [true, false] {
        let t = farcooler_protocol::v1::Terminal {
            split_of: from.clone(),
            split_of_orchestrator: Some(said),
            ..Default::default()
        };
        assert_eq!(worktree_list_terminal_json(&t)["splitOfOrchestrator"], said);
        assert_eq!(terminal_event_json(&t)["splitOfOrchestrator"], said);
    }
    let unknown = [
        farcooler_protocol::v1::Terminal { split_of: from.clone(), ..Default::default() },
        farcooler_protocol::v1::Terminal { split_of_orchestrator: Some(true), ..Default::default() },
    ];
    for t in unknown {
        assert_eq!(worktree_list_terminal_json(&t)["splitOfOrchestrator"], serde_json::json!(null));
        assert_eq!(terminal_event_json(&t)["splitOfOrchestrator"], serde_json::json!(null));
    }
}

/// And a pane nobody dispatched names none, rather than the nil uuid.
///
/// `uuid_of` answers malformed bytes with `Uuid::nil()`, and a client
/// handed `00000000-…` would hold a task id that matches no task, which
/// reads as a link rather than as its absence.
#[test]
fn a_terminal_with_no_task_or_a_malformed_one_names_none() {
    for task_id in [None, Some(bytes::Bytes::new()), Some(bytes::Bytes::from_static(b"nope"))] {
        let t = farcooler_protocol::v1::Terminal { task_id, ..Default::default() };
        assert_eq!(worktree_list_terminal_json(&t)["taskId"], serde_json::json!(null));
        assert_eq!(terminal_event_json(&t)["taskId"], serde_json::json!(null));
    }
}

#[test]
fn a_terminal_with_no_turn_or_question_sends_neither_as_present() {
    // The ordinary case — idle, or working outside a permission prompt —
    // must not invent a clock or a question that is not there.
    let t = farcooler_protocol::v1::Terminal::default();
    assert_eq!(worktree_list_terminal_json(&t)["turnStartedAt"], serde_json::json!(null));
    assert_eq!(worktree_list_terminal_json(&t)["blockedQuestion"], serde_json::json!(null));
}
