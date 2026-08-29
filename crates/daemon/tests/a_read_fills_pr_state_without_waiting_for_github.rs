//! `stack.get` fills an empty PR cache without putting GitHub on the read path.
//!
//! Three things are proved here that no unit test can, because all three are
//! about what happens BETWEEN a request and a subprocess:
//!
//!   1. the read answers at local speed while a `gh` that takes seconds is
//!      still running,
//!   2. the answer that arrives late arrives as a `stack_changed` event, on the
//!      same connection, without the client asking again,
//!   3. "GitHub answered and there is no PR" and "GitHub could not be asked"
//!      are distinguishable on the wire.
//!
//! (3) is the one that decides whether an app may offer to create a pull
//! request. Both cases produce an absent `pr` on every link, so an app reading
//! only the links would offer to create a PR that already exists whenever `gh`
//! happens to be logged out.
//!
//! A shim `gh` on `PATH` rather than a mock inside the daemon: the thing under
//! test is a subprocess with a network timeout, and replacing it with a Rust
//! function would remove exactly the property being asserted. It sleeps for
//! `SHIM_DELAY`, which is far longer than every local git command on this path
//! put together — so a read that awaited it could not possibly come back inside
//! `READ_BUDGET`.

use std::time::{Duration, Instant};

use farcooler_protocol::v1::{event, request, result};
use farcooler_transport::{Client, request as request_for};
use tokio::process::{ChildStdin, ChildStdout};

mod common;

/// How long the shim `gh` takes to answer. Longer than any plausible sum of the
/// local git commands `stack.get` legitimately runs.
const SHIM_DELAY: Duration = Duration::from_secs(3);

/// What `stack.get` may take. Generous — this is not a benchmark, it is the
/// difference between "answered from memory" and "waited on a network".
const READ_BUDGET: Duration = Duration::from_secs(1);

/// Long enough for the shim to finish and the fill to announce, on a machine
/// running the rest of the suite at the same time.
const FILL_BUDGET: Duration = Duration::from_secs(60);

/// A directory holding a `gh` that answers however this test needs it to.
///
/// `gh repo view` is answered too, and must be: the fill warms the repository's
/// default branch and web URL through the same binary, and a shim that only
/// knew `pr list` would leave `repo_url` empty and make the assertion about it
/// look like a bug in the daemon.
fn gh_shim(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let gh = bin.join("gh");
    std::fs::write(&gh, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

/// A `gh` that answers, slowly, with no pull requests at all.
fn a_gh_that_answers_with_no_prs(dir: &std::path::Path) -> std::path::PathBuf {
    gh_shim(
        dir,
        &format!(
            r#"#!/bin/sh
sleep {secs}
case "$1 $2" in
  "pr list") echo '[]' ;;
  "repo view") echo '{{"defaultBranchRef":{{"name":"main"}},"url":"https://github.example/o/r"}}' ;;
  *) exit 1 ;;
esac
"#,
            secs = SHIM_DELAY.as_secs()
        ),
    )
}

/// A `gh` that is installed and cannot answer — logged out, rate limited,
/// offline. Every one of those looks like this.
fn a_gh_that_cannot_answer(dir: &std::path::Path) -> std::path::PathBuf {
    gh_shim(
        dir,
        &format!(
            r#"#!/bin/sh
sleep {secs}
echo 'gh: To get started with GitHub CLI, please run: gh auth login' >&2
exit 4
"#,
            secs = SHIM_DELAY.as_secs()
        ),
    )
}

type StdioClient = Client<ChildStdout, ChildStdin>;

/// A registered repository with one commit and a workspace on a branch off it.
async fn a_repository_with_a_branch(
    client: &mut StdioClient,
    root: &std::path::Path,
) -> bytes::Bytes {
    let repo = root.join("demo");
    std::fs::create_dir_all(&repo).unwrap();
    for args in [
        vec!["init", "-q", "."],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["commit", "-q", "--allow-empty", "-m", "base"],
    ] {
        std::process::Command::new("git")
            .args(&args)
            .current_dir(&repo)
            .status()
            .unwrap();
    }

    let mut add = request_for("repository_root.add");
    add.payload = Some(request::Payload::RepositoryRootAdd(
        farcooler_protocol::v1::RepositoryRootAdd {
            absolute_path: root.to_string_lossy().into_owned(),
            typed_confirmation: String::new(),
        },
    ));
    client.call(add).await.expect("repository_root.add");

    let mut register = request_for("repository.register");
    register.payload = Some(request::Payload::RepositoryRegister(
        farcooler_protocol::v1::RepositoryRegister {
            relative_path: repo.to_string_lossy().into_owned(),
        },
    ));
    let Some(result::Value::Repository(repository)) =
        client.call(register).await.expect("repository.register").value
    else {
        panic!("wrong result")
    };

    let mut create = request_for("workspace.create");
    create.target_resource_id = Some(repository.id.clone());
    create.payload = Some(request::Payload::WorkspaceCreate(
        farcooler_protocol::v1::WorkspaceCreate {
            task_name: "pr status".into(),
            branch: "feat/pr-status".into(),
            base_revision: "HEAD".into(),
            terminal_preset: String::new(),
            adopt_existing: false,
        },
    ));
    client.call(create).await.expect("workspace.create");

    repository.id
}

async fn stack_get(
    client: &mut StdioClient,
    repository: &bytes::Bytes,
) -> farcooler_protocol::v1::StackLinkList {
    let mut req = request_for("stack.get");
    req.target_resource_id = Some(repository.clone());
    req.payload = Some(request::Payload::StackGet(farcooler_protocol::v1::StackGet {
        repository_id: repository.clone(),
        branch: "feat/pr-status".into(),
    }));
    let Some(result::Value::StackLinkList(list)) = client.call(req).await.expect("stack.get").value
    else {
        panic!("wrong result")
    };
    list
}

/// The next `stack_changed`, ignoring the fleet traffic a live daemon makes.
async fn next_stack_changed(client: &mut StdioClient) -> farcooler_protocol::v1::StackLinkList {
    tokio::time::timeout(FILL_BUDGET, async {
        loop {
            let event = client.next_event().await.expect("the connection stayed up");
            if let Some(event::Payload::StackChanged(list)) = event.payload {
                return list;
            }
        }
    })
    .await
    .expect("the fill never announced; nothing would ever re-read the cache it filled")
}

#[tokio::test]
async fn a_read_answers_from_memory_and_the_fill_arrives_as_an_event() {
    let dir = tempfile::tempdir().unwrap();
    let bin = a_gh_that_answers_with_no_prs(dir.path());
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());

    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let (_daemon, mut client) = common::spawn_with_env(&home, &[("PATH", &path)]).await;

    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let repository = a_repository_with_a_branch(&mut client, &work).await;

    // The read itself. Nothing has ever asked GitHub about this repository, so
    // this is the call that starts the fill.
    let started = Instant::now();
    let first = stack_get(&mut client, &repository).await;
    let took = started.elapsed();

    assert!(
        took < READ_BUDGET,
        "stack.get took {took:?}, which is GitHub's latency and not this runner's — \
         the fetch is being awaited inside a Scope::Read call"
    );
    assert!(
        !first.pr_known,
        "nothing has answered yet, so the daemon must not claim GitHub said anything"
    );
    assert!(!first.items.is_empty(), "the branch chain is local and was available immediately");

    // And a moment later, the answer, unasked for.
    let filled = next_stack_changed(&mut client).await;
    assert_eq!(filled.repository_id, repository, "the event names the repository it is about");
    assert!(
        filled.pr_known,
        "gh answered with an empty list: that is 'there is no PR', not 'we could not ask'"
    );
    assert!(
        filled.items.iter().all(|l| l.pr.is_none()),
        "an empty list means no PR on any branch"
    );
    assert_eq!(
        filled.repo_url, "https://github.example/o/r",
        "the compare link's other half comes from the same cached `gh repo view`"
    );
}

#[tokio::test]
async fn a_repository_github_cannot_answer_for_never_claims_it_did() {
    let dir = tempfile::tempdir().unwrap();
    let bin = a_gh_that_cannot_answer(dir.path());
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());

    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let (_daemon, mut client) = common::spawn_with_env(&home, &[("PATH", &path)]).await;

    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let repository = a_repository_with_a_branch(&mut client, &work).await;

    let first = stack_get(&mut client, &repository).await;
    assert!(!first.pr_known);

    // The fill still announces when it fails. It has to: a client holding a row
    // it could not fill is otherwise waiting for an event that will never come,
    // with no way to tell that from an answer still in flight.
    let filled = next_stack_changed(&mut client).await;
    assert!(
        !filled.pr_known,
        "gh exited non-zero. Reporting this as 'there is no PR' is how an app comes \
         to offer Create Pull Request over a PR that already exists"
    );
    assert!(filled.items.iter().all(|l| l.pr.is_none()), "and there is still no PR state");
}
