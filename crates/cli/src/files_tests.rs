use farcooler_protocol::capability;
use farcooler_transport::ClientError;

use super::*;

/// A runner that records what it was sent and answers an empty directory.
struct Runner {
    capabilities: Vec<String>,
    sent: Vec<pb::Request>,
}

fn runner(capabilities: &[&str]) -> Runner {
    Runner { capabilities: capabilities.iter().map(|c| c.to_string()).collect(), sent: vec![] }
}

impl DispatchLink for Runner {
    fn capabilities(&self) -> Vec<String> {
        self.capabilities.clone()
    }
    async fn call(&mut self, req: pb::Request) -> Result<pb::Result, ClientError> {
        let method = req.method.clone();
        self.sent.push(req);
        Ok(pb::Result {
            value: Some(match method.as_str() {
                "worktree.list_dir" => result::Value::WorktreeDir(pb::WorktreeDir::default()),
                "worktree.read_file" => result::Value::WorktreeFile(pb::WorktreeFile::default()),
                other => panic!("sent {other}"),
            }),
        })
    }
    async fn pause(&mut self, _wait: std::time::Duration) {}
}

#[tokio::test]
async fn a_folder_listing_names_the_folder_and_no_worktree() {
    let mut link = runner(&[capability::WORKTREE_FILES, capability::READ_ONLY_FOLDERS]);
    files_over(&mut link, FilesCmd::FolderLs { folder: "logs".into(), path: "nginx".into() }, true).await.expect("listed");
    let Some(request::Payload::WorktreeDir(p)) = &link.sent[0].payload else { panic!("{:?}", link.sent[0]) };
    assert_eq!((p.folder.as_str(), p.path.as_str()), ("logs", "nginx"));
    assert!(p.worktree_id.is_empty(), "the runner refuses both");
}

#[tokio::test]
async fn a_folder_read_names_the_folder_and_no_worktree() {
    let mut link = runner(&[capability::WORKTREE_FILES, capability::READ_ONLY_FOLDERS]);
    files_over(&mut link, FilesCmd::FolderCat { folder: "logs".into(), path: "a.log".into() }, true).await.expect("read");
    let Some(request::Payload::WorktreeFile(p)) = &link.sent[0].payload else { panic!("{:?}", link.sent[0]) };
    assert_eq!((p.folder.as_str(), p.path.as_str()), ("logs", "a.log"));
    assert!(p.worktree_id.is_empty());
}

#[tokio::test]
async fn a_runner_without_the_capability_is_refused_before_anything_is_sent() {
    let mut link = runner(&[capability::WORKTREE_FILES]);
    let err = files_over(&mut link, FilesCmd::FolderLs { folder: "logs".into(), path: String::new() }, true)
        .await
        .expect_err("refused");
    assert!(err.to_string().contains("needs an update"), "{err}");
    assert!(link.sent.is_empty());
}

/// A file name in a directory like /var/log is chosen by whoever writes
/// there, so one shaped like a flag must still be read as a name.
#[test]
fn a_flag_shaped_name_after_the_double_dash_is_a_name_and_not_a_flag() {
    use clap::Parser;
    for argv in [
        ["farcooler", "files", "folder-cat", "--json", "--", "--runner=x", "--runner=y"],
        ["farcooler", "files", "folder-cat", "--json", "--", "--help", "--runner=y"],
    ] {
        let cli = crate::Cli::try_parse_from(argv).expect("parses");
        assert!(cli.runner.is_none(), "the name retargeted the read: {argv:?}");
        let crate::Command::Files(FilesCmd::FolderCat { folder, path }) = cli.command else { panic!("{argv:?}") };
        assert_eq!((folder.as_str(), path.as_str()), (argv[5], "--runner=y"));
    }
    let cli = crate::Cli::try_parse_from(["farcooler", "files", "ls", "--json", "--", "w1", "--runner=x"]).expect("parses");
    assert!(cli.runner.is_none());
    let crate::Command::Files(FilesCmd::Ls { path, .. }) = cli.command else { panic!() };
    assert_eq!(path, "--runner=x");
}

#[test]
fn a_folder_refusal_names_the_folder_and_a_worktree_one_the_worktree() {
    let gone = || ClientError::Daemon {
        code: pb::ErrorCode::NotFound as i32,
        retryable: false,
        message: String::new(),
        what: String::new(),
    };
    assert!(refusal(gone(), true).to_string().contains("this folder"));
    assert!(refusal(gone(), false).to_string().contains("this worktree"));
}
