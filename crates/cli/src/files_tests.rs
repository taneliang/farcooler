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
    let mut link = runner(&["worktree_files", "read_only_folders"]);
    files_over(&mut link, FilesCmd::FolderLs { folder: "logs".into(), path: "nginx".into() }, true).await.expect("listed");
    let Some(request::Payload::WorktreeDir(p)) = &link.sent[0].payload else { panic!("{:?}", link.sent[0]) };
    assert_eq!((p.folder.as_str(), p.path.as_str()), ("logs", "nginx"));
    assert!(p.worktree_id.is_empty(), "the runner refuses both");
}

#[tokio::test]
async fn a_folder_read_names_the_folder_and_no_worktree() {
    let mut link = runner(&["worktree_files", "read_only_folders"]);
    files_over(&mut link, FilesCmd::FolderCat { folder: "logs".into(), path: "a.log".into() }, true).await.expect("read");
    let Some(request::Payload::WorktreeFile(p)) = &link.sent[0].payload else { panic!("{:?}", link.sent[0]) };
    assert_eq!((p.folder.as_str(), p.path.as_str()), ("logs", "a.log"));
    assert!(p.worktree_id.is_empty());
}

#[tokio::test]
async fn a_runner_without_the_capability_is_refused_before_anything_is_sent() {
    let mut link = runner(&["worktree_files"]);
    let err = files_over(&mut link, FilesCmd::FolderLs { folder: "logs".into(), path: String::new() }, true)
        .await
        .expect_err("refused");
    assert!(err.to_string().contains("needs an update"), "{err}");
    assert!(link.sent.is_empty());
}
