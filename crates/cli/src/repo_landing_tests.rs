//! `repo landing`: what it asks, what it prints for each way of landing, and
//! that it only ever reads.

use farcooler_transport::ClientError;

use super::*;

const REPO: Uuid = Uuid::from_u128(0x11);
const MAIN: Uuid = Uuid::from_u128(0x22);

struct Runner {
    capabilities: Vec<String>,
    landing: pb::RepositoryLanding,
    main: pb::Workspace,
    sent: Vec<String>,
}

fn runner(landing: pb::RepositoryLanding, chosen: Option<pb::LandingMode>) -> Runner {
    Runner {
        capabilities: ["workstreams", capability::LANDING].map(String::from).to_vec(),
        landing,
        main: pb::Workspace {
            id: bytes::Bytes::copy_from_slice(MAIN.as_bytes()),
            repository_id: bytes::Bytes::copy_from_slice(REPO.as_bytes()),
            name: "Main".into(),
            is_main: true,
            landing: chosen.map(|m| m as i32),
            ..Default::default()
        },
        sent: vec![],
    }
}

impl DispatchLink for Runner {
    fn capabilities(&self) -> Vec<String> {
        self.capabilities.clone()
    }
    async fn pause(&mut self, _wait: std::time::Duration) {}
    async fn call(&mut self, req: pb::Request) -> Result<pb::Result, ClientError> {
        self.sent.push(req.method.clone());
        Ok(pb::Result {
            value: Some(match req.method.as_str() {
                "repository.list" => result::Value::RepositoryList(pb::RepositoryList {
                    items: vec![pb::Repository {
                        id: bytes::Bytes::copy_from_slice(REPO.as_bytes()),
                        display_name: "overnight".into(),
                        ..Default::default()
                    }],
                }),
                "workspace.list" => result::Value::WorkspaceList(pb::WorkspaceList { items: vec![self.main.clone()] }),
                "repository.landing" => {
                    assert_eq!(req.required_capabilities, [capability::LANDING]);
                    assert_eq!(req.target_resource_id.as_deref(), Some(REPO.as_bytes().as_slice()));
                    result::Value::RepositoryLanding(self.landing.clone())
                }
                other => panic!("sent {other}"),
            }),
        })
    }
}

fn pull_request_rule() -> pb::RepositoryLanding {
    pb::RepositoryLanding {
        base: "main".into(),
        suggested: pb::LandingMode::PullRequests as i32,
        direct_impossible: true,
        reasons: vec![
            "main requires pull requests, with 2 approving reviews.".into(),
            "main needs 2 required checks to pass before a change lands, which a direct push can't wait for.".into(),
        ],
        warnings: vec!["The merge queue will wait forever: no workflow runs on `merge_group`.".into()],
        facts: Some(pb::LandingFacts {
            pull_request_rule: Some(true),
            required_approvals: Some(2),
            merge_queue: Some(true),
            required_checks: vec!["rust".into(), "swift".into()],
            branch_protected: Some(true),
            merge_method: Some("squash".into()),
            codeowners: Some(false),
            viewer_permission: Some("ADMIN".into()),
            merge_group_workflow: Some(false),
            ..Default::default()
        }),
        read_at: 1,
    }
}

#[tokio::test]
async fn it_reads_with_the_capability_named_and_writes_nothing() {
    let mut r = runner(pull_request_rule(), None);
    let text = read(&mut r, None, None, false).await.unwrap();
    assert!(text.starts_with("Base branch: main\nSuggested: land through pull requests\nLanding directly can't work here."), "{text}");
    assert_eq!(r.sent, ["repository.list", "workspace.list", "repository.landing"], "one read, no setting touched");
}

#[tokio::test]
async fn a_runner_without_the_capability_is_told_before_anything_is_sent() {
    let mut r = runner(pull_request_rule(), None);
    r.capabilities.retain(|c| c != capability::LANDING);
    let said = read(&mut r, None, None, false).await.expect_err("refused").to_string();
    assert_eq!(said, "This runner needs an update to read how it lands work.");
    assert!(r.sent.is_empty(), "{:?}", r.sent);
}

#[test]
fn a_protected_main_prints_its_reasons_warnings_facts_and_the_choice_left_alone() {
    let main = pb::Workspace { name: "Main".into(), landing: Some(pb::LandingMode::Direct as i32), ..Default::default() };
    assert_eq!(
        render(&pull_request_rule(), Some(&main)),
        "Base branch: main\n\
         Suggested: land through pull requests\n\
         Landing directly can't work here.\n\
         \n\
         Why:\n\
         \x20 main requires pull requests, with 2 approving reviews.\n\
         \x20 main needs 2 required checks to pass before a change lands, which a direct push can't wait for.\n\
         \n\
         Warnings:\n\
         \x20 The merge queue will wait forever: no workflow runs on `merge_group`.\n\
         \n\
         What was read:\n\
         \x20 Pull request rule: yes, with 2 approving reviews\n\
         \x20 Merge queue: yes\n\
         \x20 Required checks: rust, swift\n\
         \x20 Branch protection: protected (GitHub shows its details only to an admin)\n\
         \x20 Merge method: squash\n\
         \x20 CODEOWNERS: no\n\
         \x20 This login's permission: ADMIN\n\
         \x20 A workflow runs on merge_group: no\n\
         \n\
         Main is set to land directly, and that can't work here. Nothing was changed. \
         Choose pull requests with `farcooler workspace set Main --landing pull-requests`."
    );
}

#[test]
fn an_unprotected_main_suggests_direct_and_an_unchosen_board_is_told_how_to_choose() {
    let l = pb::RepositoryLanding {
        base: "main".into(),
        suggested: pb::LandingMode::Direct as i32,
        reasons: vec!["Nothing on main asks for pull requests.".into()],
        facts: Some(pb::LandingFacts {
            pull_request_rule: Some(false),
            merge_queue: Some(false),
            branch_protected: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    };
    let main = pb::Workspace { name: "Main".into(), ..Default::default() };
    let text = render(&l, Some(&main));
    assert!(text.contains("Suggested: land directly on the base branch\n\nWhy:"), "{text}");
    assert!(!text.contains("can't work here"), "{text}");
    assert!(text.contains("Required checks: none\n  Branch protection: not protected\n  Merge method: not read"), "{text}");
    assert!(text.ends_with(
        "Main hasn't chosen yet. Choose with `farcooler workspace set Main --landing direct` or `--landing pull-requests`."
    ), "{text}");
}

#[test]
fn what_could_not_be_read_is_not_read_and_never_no() {
    let l = pb::RepositoryLanding {
        base: "main".into(),
        reasons: vec!["Far Cooler couldn't read its rules or its protection on main, so it suggests nothing.".into()],
        facts: Some(pb::LandingFacts::default()),
        ..Default::default()
    };
    let text = render(&l, None);
    assert!(text.contains("Suggested: nothing, because not enough could be read"), "{text}");
    for line in [
        "Pull request rule: not read",
        "Merge queue: not read",
        "Required checks: not read",
        "Branch protection: not read",
        "CODEOWNERS: not read",
        "This login's permission: not read",
        "A workflow runs on merge_group: not read",
    ] {
        assert!(text.contains(line), "{line}\n{text}");
    }
    assert!(text.ends_with("No workspace was found to compare with."), "{text}");
}

#[test]
fn the_json_has_words_for_modes_and_null_for_what_was_not_read() {
    let main = pb::Workspace { landing: Some(pb::LandingMode::PullRequests as i32), ..Default::default() };
    let v = landing_json(&pull_request_rule(), Some(&main));
    assert_eq!(v["suggested"], "pull_requests");
    assert_eq!(v["chosen"], "pull_requests");
    assert_eq!(v["direct_impossible"], true);
    assert_eq!(v["facts"]["required_checks"], serde_json::json!(["rust", "swift"]));
    assert_eq!(v["facts"]["required_approvals"], 2);
    let unread = landing_json(&pb::RepositoryLanding { base: "main".into(), ..Default::default() }, None);
    assert!(unread["suggested"].is_null() && unread["chosen"].is_null(), "{unread}");
    assert!(unread["facts"]["branch_protected"].is_null() && unread["facts"]["merge_group_workflow"].is_null(), "{unread}");
}
